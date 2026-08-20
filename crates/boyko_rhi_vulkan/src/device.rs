//! Vulkan loader → instance → physical-device → logical-device bootstrap.
//!
//! Mirrors the `vm.rs` raw-FFI discipline (§4 of the plan): load `vulkan-1.dll`
//! via `LoadLibraryA` + `GetProcAddress`, obtain `vkGetInstanceProcAddr`, load
//! the global commands, create a `VkInstance`, load the instance commands,
//! enumerate physical devices and pick one (preferring a discrete GPU), then
//! create a `VkDevice` + one graphics+compute queue and load the device
//! commands via `vkGetDeviceProcAddr`.
//!
//! # Validation layers (structured-but-never-required)
//!
//! [`InstanceConfig::enable_validation`] threads `VK_LAYER_KHRONOS_validation`
//! into instance creation **only if the layer is present** (it is queried
//! first; an absent layer silently downgrades to no validation rather than
//! failing). Slice 0's NO-SDK sub-step never sets the flag — the SDK that
//! ships the layer is installed separately — but the seam is here so the
//! compute/validation steps (Phase 0c+) can flip it on without reshaping the
//! bootstrap.
//!
//! # Lifetime / teardown
//!
//! [`VulkanContext`] owns the loaded module, the instance and the device. Its
//! `Drop` tears them down in reverse creation order (`vkDestroyDevice` →
//! `vkDestroyInstance` → `FreeLibrary`) so a dropped context leaves no leaked
//! Vulkan objects or DLL references.

// `RefCell` here is the borrow gate on the two lazy sub-allocator blocks below — a
// `!Send + !Sync` device context handing out `&mut` from `&self`. See
// docs/HOT-PATH-EXCEPTIONS.md (class `alloc-guarded`).
#[allow(clippy::disallowed_types)]
use core::cell::{OnceCell, RefCell};
use core::ffi::{CStr, c_char, c_void};
use core::mem;
use core::ptr;
use core::sync::atomic::{AtomicPtr, Ordering};

use boyko_log::codes::{E2101, OnceSite, W2102};

use crate::debug::{self, DebugMessengerState};
use crate::ffi::*;
use crate::memory::{BlockPool, DeviceLocalBlock, HostVisibleBlock};
use crate::rhi_impl::ComputeLayouts;

/// Capacity of the device's shared host-visible block backing
/// [`RhiDevice::create_buffer`](boyko_rhi::RhiDevice::create_buffer) (plan Q1: the
/// foundation routes every buffer through one block). 64 MiB comfortably holds
/// the Slice-0 storage buffers and any near-term compute working set.
const SHARED_HOST_BLOCK_CAPACITY: u64 = 64 * 1024 * 1024;

/// Capacity of the device's shared device-local (VRAM) block backing
/// [`RhiDevice::create_buffer`](boyko_rhi::RhiDevice::create_buffer) with
/// [`MemoryLocation::DeviceLocal`](boyko_rhi::MemoryLocation::DeviceLocal) (the
/// Phase-5 `GpuColumn` seam). 64 MiB matches the host block's working-set budget;
/// the block is created lazily on the first device-local buffer.
const SHARED_DEVICE_BLOCK_CAPACITY: u64 = 64 * 1024 * 1024;

/// Errors that can occur while bootstrapping a Vulkan device.
///
/// All are recoverable from the caller's perspective: a GPU-less or
/// loader-less machine yields [`BootError::LoaderUnavailable`] /
/// [`BootError::NoPhysicalDevice`], which the integration test treats as
/// "skip gracefully" rather than a failure.
#[derive(Debug)]
pub enum BootError {
    /// `vulkan-1.dll` could not be loaded, or `vkGetInstanceProcAddr` was not
    /// exported.
    LoaderUnavailable,
    /// A required global/instance/device command was missing from the loader.
    MissingCommand(&'static str),
    /// A Vulkan command returned a non-success `VkResult`.
    VkError(&'static str, VkResult),
    /// `vkEnumeratePhysicalDevices` reported zero GPUs.
    NoPhysicalDevice,
    /// No queue family on the chosen GPU supports graphics + compute.
    NoSuitableQueueFamily,
    /// Validation was requested but `VK_LAYER_KHRONOS_validation` is not
    /// installed (the SDK is absent). The caller decides whether this is fatal
    /// (the compute tests treat it as "skip gracefully").
    ValidationUnavailable,
    /// A windowed context was requested but a required WSI / dynamic-rendering
    /// extension or feature is not present on this driver (the test treats it as
    /// "skip gracefully").
    WindowingUnavailable,
    /// The GPU does not advertise `STORAGE_IMAGE` on the Render P1b G-buffer color
    /// format (`R8G8B8A8_UNORM`, OPTIMAL tiling) — the marcher cannot store into the
    /// G-buffer color images. Core-guaranteed on the RTX 3060 / any desktop GPU; a
    /// CLEAR fail-fast at device-create beats an opaque storage-image store fault.
    GbufferStorageFormatUnsupported,
    /// The GPU does not advertise `STORAGE_IMAGE` on the Lighting L0b `gViewT` lane
    /// format (`R32_SFLOAT`, OPTIMAL tiling) — the marcher cannot store the surface ray
    /// parameter `t` the deferred resolve reconstructs `P` from. Core-guaranteed on the
    /// RTX 3060 / any desktop GPU; the fail-fast mirrors
    /// [`Self::GbufferStorageFormatUnsupported`] so the new lane can never fault on an
    /// unsupported format.
    ViewtStorageFormatUnsupported,
    /// The GPU does not advertise `COLOR_ATTACHMENT` on the G-buffer color format
    /// (`R8G8B8A8_UNORM`, OPTIMAL tiling) — Render P5-r0's mesh raster pass A cannot write
    /// the albedo/normal/material images as MRT color attachments. RGBA8_UNORM
    /// color-attachment renderability is mandatory in Vulkan, so this is core-guaranteed on
    /// any conformant GPU; the fail-fast mirrors [`Self::GbufferStorageFormatUnsupported`]
    /// so the producer can never fault as a device-lost on an unsupported usage.
    GbufferColorAttachmentFormatUnsupported,
    /// The GPU does not advertise `STORAGE_IMAGE` on the Render P7 SSAO term format
    /// (`R8_UNORM`, OPTIMAL tiling) — the resolve binds `gSsao` (and the SSAO pass stores it)
    /// as a STORAGE image. `R8_UNORM` storage-image support is broadly available (core-
    /// guaranteed on the RTX 3060 / any desktop GPU); the fail-fast mirrors
    /// [`Self::GbufferStorageFormatUnsupported`] so the SSAO image can never fault on an
    /// unsupported format.
    SsaoStorageFormatUnsupported,
    /// T-dev: the chosen GPU does not advertise (or the driver failed to enable) all 5
    /// `VkPhysicalDeviceDescriptorIndexingFeatures` bits the bindless prerequisite needs
    /// (`shaderSampledImageArrayNonUniformIndexing`, `runtimeDescriptorArray`,
    /// `descriptorBindingPartiallyBound`, `descriptorBindingVariableDescriptorCount`,
    /// `descriptorBindingSampledImageUpdateAfterBind`) — the textured-PBR T4 bindless
    /// descriptor path cannot function without them. A CLEAR boot fail-fast beats a
    /// silent bindless-disabled degrade or an opaque shader fault later; mirrors
    /// [`Self::GbufferStorageFormatUnsupported`]'s discipline.
    BindlessUnsupported,
    /// SDFDDGI I0: the chosen GPU's per-stage descriptor limits cannot satisfy the deferred
    /// resolve set's ACTUAL declared per-type descriptor counts (the resolve set grew to 19
    /// bindings with the 3 DDGI bindings + the CSM/atlas ones). A device below the real need is
    /// EXTERNAL INPUT (not an engine invariant), so it is surfaced as a `BootError` through the
    /// device-selection path (`pick_physical_device` → `VulkanContext::boot`) — NOT a release
    /// `assert!`. It projects to `RhiError::BackendError("vulkan boot failed")` in
    /// `From<VulkanError> for RhiError` (the same agnostic category as the format-unsupported device
    /// rejections). Carries `(kind, need, limit)`: the descriptor kind that overflowed (a
    /// `&'static str` name), the resolve set's need, and the device's `maxPerStageDescriptor*` cap
    /// for that kind. The I(-1) deferral (validate the ACTUAL per-type counts, not the aggregate cap
    /// vs a per-type limit) is closed here.
    ResolveDescriptorLimitExceeded {
        /// The overflowing descriptor kind's `maxPerStageDescriptor*` field name.
        kind: &'static str,
        /// The resolve set's declared need for that kind.
        need: u32,
        /// The device's per-stage limit for that kind.
        limit: u32,
    },
}

/// Bootstrap options for the instance.
#[derive(Clone, Copy, Default)]
pub struct InstanceConfig {
    /// Request `VK_LAYER_KHRONOS_validation` + a `VK_EXT_debug_utils` messenger
    /// that records WARNING/ERROR validation messages as the test oracle
    /// (plan §6 / Slice-0 step 0a). Defaults to `false`.
    ///
    /// If `true` but the layer or the `VK_EXT_debug_utils` extension is absent,
    /// boot fails with [`BootError::ValidationUnavailable`] rather than silently
    /// running without the oracle — a missing oracle must never be invisible.
    pub enable_validation: bool,

    /// Build an on-screen-capable context (Slice 1): enable the `VK_KHR_surface`
    /// and `VK_KHR_win32_surface` instance extensions and the `VK_KHR_swapchain`
    /// device extension, request the `dynamicRendering` (Vulkan 1.3) feature, and
    /// load the surface/swapchain command tables.
    ///
    /// Defaults to `false`, so the headless [`VulkanContext::boot`] path is
    /// byte-for-byte unchanged.
    ///
    /// The surface itself is created later from a window via
    /// [`crate::swapchain::Surface::new`]; this flag only wires the extensions
    /// the surface/swapchain need at instance/device-creation time.
    pub windowed: bool,
}

/// HW-RT rung R1: the ray-tracing capability TIER a device resolves to — the
/// dormant seam a later rung's backend routing gates on.
///
/// Derived from [`DeviceCaps`] via [`DeviceCaps::rt_tier`], NOT stored: the tier
/// is a pure function of the recorded RT feature bits. In R1 those bits are
/// hard-wired `false` (no `VK_KHR_ray_query`/`acceleration_structure` extension
/// is requested), so `rt_tier()` returns [`RtTier::Absent`] for EVERY device —
/// the dormancy anchor. R2a (`feature=hwrt`) enables the real presence+enable
/// query and the `Weak`/`Strong` arms come alive.
///
/// `#[repr(u8)]` for a compact, stable discriminant a later config table may
/// index by. Ordered by capability (`Absent < Weak < Strong`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u8)]
pub enum RtTier {
    /// No usable hardware ray tracing (`ray_query == false`). Every device in R1
    /// — the software-only path is the only honest selection.
    Absent = 0,
    /// Hardware ray query WITHOUT shader-execution-reorder (`ray_query == true`,
    /// `ray_reorder == false`). Reserved for R2a; unreachable in R1.
    Weak = 1,
    /// Hardware ray query WITH shader-execution-reorder (`ray_query == true`,
    /// `ray_reorder == true`). Reserved for R2a; unreachable in R1.
    Strong = 2,
}

/// Minimal physical-device capabilities queried ONCE at device-create (Render P1b),
/// alongside the `dynamicRendering` fail-fast.
///
/// A small POD recorded on the [`VulkanContext`] and exposed read-only via
/// [`VulkanContext::device_caps`]. `gbuffer_storage_format_ok` / `bindless_capable` are
/// asserted at boot, so a context that exists always has them `true` (the fail-fast
/// rejects a GPU without them).
#[derive(Clone, Copy, Debug)]
pub struct DeviceCaps {
    /// Whether the GPU advertises (T-dev: AND enables) the 5 `VkPhysicalDeviceDescriptorIndexingFeatures`
    /// bits the bindless path needs: `shaderSampledImageArrayNonUniformIndexing`,
    /// `runtimeDescriptorArray`, `descriptorBindingPartiallyBound`,
    /// `descriptorBindingVariableDescriptorCount`,
    /// `descriptorBindingSampledImageUpdateAfterBind`. Boot fail-fast: a booted context
    /// always has this `true` — [`BootError::BindlessUnsupported`] rejects a GPU
    /// lacking any of the 5.
    pub bindless_capable: bool,
    /// Multi-paradigm render-path plan, Decision 0 / rung R1 (widened at rung R8, code review
    /// P1-2 fix): whether the GPU advertises BOTH
    /// `VkPhysicalDeviceDescriptorIndexingFeatures::shaderStorageBufferArrayNonUniformIndexing`
    /// (indexing `gMeshVerts[]`/`gMeshIndices[]` by a wave-non-uniform `mesh_id`,
    /// `NonUniformResourceIndex`) AND `descriptorBindingStorageBufferUpdateAfterBind` (the VB
    /// geometry table's Set-2 UPDATE_AFTER_BIND pool/layout, `geometry_bindless.rs`) — the
    /// CONJUNCTION of both, not just the first: `create_device` conditionally enables BOTH bits
    /// under the SAME `enable_vb_geometry_table` gate this field ultimately drives (below), so
    /// querying only one while enabling both risked a hard `VK_ERROR_FEATURE_NOT_PRESENT`
    /// `vkCreateDevice` failure on a device advertising the first but not the second (the P1-2
    /// bug this rung's code review caught). RECORDED ONLY (NO boot fail-fast, unlike
    /// [`bindless_capable`](Self::bindless_capable)'s 5-bit group): VisibilityBuffer is
    /// opt-in and near-universal-but-not-guaranteed, so an unsupported device degrades the
    /// path to `Deferred` at boot (`boyko_render::render_path_config::resolve_render_path`'s
    /// `RenderPathDeviceCaps` input — this crate sits BELOW `boyko_render` in the dependency
    /// graph, so it cannot doc-link that type), never a boot failure. Read from the SAME
    /// `descriptor_indexing` features-2 query
    /// [`bindless_capable`](Self::bindless_capable) already runs (`query_device_caps`); `create_device`
    /// enables both bits IFF this cap is `true` (the "query before request" precedent
    /// `enable_ray_query` establishes).
    pub storage_buffer_array_non_uniform_indexing_ok: bool,
    /// Whether `R8G8B8A8_UNORM` supports `STORAGE_IMAGE` under OPTIMAL tiling (the P1b
    /// G-buffer color images are compute-store targets). Always `true` on a booted
    /// context — boot fails with [`BootError::GbufferStorageFormatUnsupported`]
    /// otherwise.
    pub gbuffer_storage_format_ok: bool,
    /// Whether `R32_SFLOAT` supports `STORAGE_IMAGE` under OPTIMAL tiling (the Lighting
    /// L0b `gViewT` lane is a compute-store target). Always `true` on a booted context —
    /// boot fails with [`BootError::ViewtStorageFormatUnsupported`] otherwise (W2).
    pub viewt_storage_format_ok: bool,
    /// Whether `R8G8B8A8_UNORM` supports `COLOR_ATTACHMENT` under OPTIMAL tiling (Render
    /// P5-r0: the mesh raster pass A writes the G-buffer color images as MRT color
    /// attachments). Always `true` on a booted context — boot fails with
    /// [`BootError::GbufferColorAttachmentFormatUnsupported`] otherwise.
    pub gbuffer_color_attachment_format_ok: bool,
    /// Whether `R8_UNORM` supports `STORAGE_IMAGE` under OPTIMAL tiling (Render P7: the SSAO
    /// term `gSsao` is a full-res STORAGE-image the resolve loads + the SSAO pass stores).
    /// Always `true` on a booted context — boot fails with
    /// [`BootError::SsaoStorageFormatUnsupported`] otherwise.
    pub r8_unorm_storage_ok: bool,
    /// Whether `R8_SNORM` supports `SAMPLED_IMAGE_FILTER_LINEAR` under OPTIMAL tiling (SDF
    /// brick-atlas campaign M2): the hardware trilinear fetch of the quantized narrow-band
    /// brick atlas needs `VK_FILTER_LINEAR` on the sampled `R8_SNORM` 3D image. RECORDED
    /// (not a boot fail-fast): when `false`, the M2 atlas falls back to `R16_SFLOAT` (which
    /// supports linear filtering on every conformant GPU), so the engine boots on either
    /// path. Read via [`DeviceCaps::atlas_format`] to pick the brick-atlas image format.
    pub atlas_linear_filter_ok: bool,
    /// Whether `B10G11R11_UFLOAT_PACK32` supports `STORAGE_IMAGE` under OPTIMAL tiling
    /// (SDFDDGI I2: the probe-update pass writes the irradiance atlas via a storage image).
    /// B10G11R11 storage is a device-OPTIONAL format feature (`shaderStorageImageExtendedFormats`),
    /// so unlike [`viewt_storage_format_ok`](Self::viewt_storage_format_ok) this is RECORDED,
    /// NOT a boot fail-fast: DDGI is opt-in, so when `false` the atlas is created WITHOUT storage
    /// and `resolve_ddgi_grid` clamps DDGI permanently disabled — a GI-OFF (or unsupported)
    /// device boots normally (plan §3). Read via [`DeviceCaps::ddgi_storage_ok`].
    pub ddgi_irr_storage_ok: bool,
    /// Whether `R16G16_SFLOAT` supports `STORAGE_IMAGE` under OPTIMAL tiling (SDFDDGI I2: the
    /// probe-update pass writes the two-moment depth atlas via a storage image). RG16F storage
    /// is broadly available, but gated together with the irradiance atlas so a device missing
    /// EITHER degrades DDGI gracefully (RECORDED, not a boot fail-fast — see
    /// [`ddgi_irr_storage_ok`](Self::ddgi_irr_storage_ok)).
    pub ddgi_depth_storage_ok: bool,
    /// Rung 3a: whether `R8G8_UNORM` supports `STORAGE_IMAGE` under OPTIMAL tiling. RECORDED ONLY
    /// (NO boot fail-fast). NO LONGER part of the shadow-denoise gate: both ping-pong rings
    /// (`shadow_vis` + `shadow_vis2`) were unified to `R16G16_UNORM` (the uniform-RG16 design that
    /// lets one `"rg16"` shader pin fit every binding on every parity), so
    /// [`shadow_denoise_storage_ok`](Self::shadow_denoise_storage_ok) reads
    /// [`rg16_unorm_storage_ok`](Self::rg16_unorm_storage_ok) alone. UNCONDITIONAL as of the SV0
    /// dedicated pass: the `sdf_term` ring is an RG8 STORAGE target on every VB boot, so this
    /// probe gates SV0 arming and the ring's STORAGE usage bit (degrade-not-panic — an
    /// unsupported device gets a SAMPLED-only ring and SV0 resolves unarmable).
    pub rg8_unorm_storage_ok: bool,
    /// Rung 3a: whether `R16G16_UNORM` supports `STORAGE_IMAGE` under OPTIMAL tiling (BOTH ping-pong
    /// rings `shadow_vis` + `shadow_vis2` — 16-bit avoids the cumulative 8-bit rounding of a
    /// multi-level filter, and a single format lets one `"rg16"` shader pin fit every binding).
    /// RECORDED ONLY (NO boot fail-fast): this is the SOLE storage precondition read by
    /// [`shadow_denoise_storage_ok`](Self::shadow_denoise_storage_ok), so a device missing it
    /// degrades the shadow denoise gracefully (mirrors the DDGI pair). `#[cfg(feature = "hwrt")]`.
    #[cfg(feature = "hwrt")]
    pub rg16_unorm_storage_ok: bool,
    /// The SSAO à-trous denoise chain: whether `R16_UNORM` supports `STORAGE_IMAGE` under
    /// OPTIMAL tiling (the interior ping-pong ring's format — 16-bit avoids the cumulative
    /// 8-bit rounding of a multi-level filter, mirroring `rg16_unorm_storage_ok`'s
    /// rationale one channel narrower). RECORDED ONLY (NO boot fail-fast): the SSAO à-trous
    /// denoise is software (NOT `hwrt`-gated, unlike the shadow-visibility denoiser) — a device
    /// missing it degrades to the raw (un-denoised) `sdf_ssao` gather, never a boot failure.
    pub r16_unorm_storage_ok: bool,
    /// HW-RT rung R0: `VkPhysicalDeviceLimits::timestampPeriod` — nanoseconds per GPU
    /// timestamp tick (multiply a masked tick delta by this to get ns). RECORDED (not a
    /// boot fail-fast): a `<= 0` or `> 1000` value (an implausible period, or a wrong-offset
    /// read) makes [`Self::timestamps_usable`] `false`, degrading the GPU-timing harness to a
    /// graceful skip.
    pub timestamp_period: f32,
    /// HW-RT rung R0: the chosen graphics+compute queue family's
    /// `timestampValidBits` — the number of MEANINGFUL low bits in a raw timestamp
    /// (bits above this width are hardware garbage and MUST be masked off before
    /// subtracting). `0` means the family does not support timestamps → the harness
    /// skips (see [`Self::timestamps_usable`]).
    pub timestamp_valid_bits: u32,
    /// VB-SV0 rung S1.5: `VkPhysicalDeviceLimits::timestampComputeAndGraphics` — whether ALL
    /// graphics+compute queue families are guaranteed to support timestamps.
    ///
    /// RECORDED ONLY. It deliberately does NOT participate in [`Self::timestamps_usable`]: the
    /// authoritative per-queue answer is [`Self::timestamp_valid_bits`] on the family actually
    /// chosen, and `false` here merely means the guarantee is per-family rather than blanket. It
    /// is read so a timing harness reporting its own resolution can state which guarantee its
    /// numbers rest on instead of implying the stronger one.
    pub timestamp_compute_and_graphics: bool,
    /// Profiling rung 4 (D18): whether `hostQueryReset` was **ENABLED** at device creation —
    /// the contract is "enabled", not "advertised", exactly as [`Self::ray_query`]'s is, so a
    /// caller reading `true` may call `vkResetQueryPool` without a further check.
    ///
    /// RECORDED, never a boot fail-fast. Host reset is an optimisation with a fully specified
    /// fallback (a recorded `vkCmdResetQueryPool` at the frame top), so a device without it
    /// costs one frame of query-pool recycle latency and nothing else.
    pub host_query_reset: bool,
    /// Profiling rung 9 (D14 tier 2): whether `VK_EXT_calibrated_timestamps` was **ENABLED** at
    /// device creation AND the device advertises [`crate::ffi::VK_TIME_DOMAIN_DEVICE_EXT`] among
    /// its calibrateable domains — the same "enabled, not advertised" contract
    /// [`Self::host_query_reset`] carries, so a caller reading `true` may call
    /// `vkGetCalibratedTimestampsEXT` without a second check.
    ///
    /// RECORDED, never a boot fail-fast. Without it the profiler's `cpu_gpu_offset` stays
    /// `UNCORRELATED`, which is a stated status on the data rather than a degraded number — D14's
    /// rule is that an uncalibrated cross-domain offset is a fabrication, not an approximation.
    pub calibrated_timestamps: bool,
    /// HW-RT rung R1: whether hardware ray query is ENABLED on this device (the
    /// `VK_KHR_ray_query` extension requested + its feature turned on). The
    /// field's contract is "ENABLED", not "present": R1 requests NO RT extension
    /// and adds nothing to `VkDeviceCreateInfo`, so the only honest value is
    /// `false` (a presence-query reporting `true` while disabled would let a
    /// consumer arm a null trace path — UB). HARD-WIRED `false` in R1 (the
    /// dormancy anchor); R2a (`feature=hwrt`) runs the real presence+enable query.
    /// [`Self::rt_tier`] gates on this, so `rt_tier() == Absent` for every device.
    pub ray_query: bool,
    /// HW-RT rung R1: whether hardware shader-execution-reorder is ENABLED
    /// (`VK_NV_ray_tracing_invocation_reorder` or equivalent). HARD-WIRED `false`
    /// in R1; distinguishes [`RtTier::Weak`] from [`RtTier::Strong`] once
    /// [`Self::ray_query`] is `true` (R2a). Unread in R1 (`ray_query == false`
    /// short-circuits `rt_tier()` to `Absent` first).
    pub ray_reorder: bool,
    /// HW-RT rung R1: `VkPhysicalDeviceProperties::vendorID` — the PCI vendor ID
    /// (real value, populated at the boot site; the R3 per-GPU calibration-cache
    /// key). RECORDED ONLY: nothing branches on it in R1 (`rt_tier()` gates on
    /// [`Self::ray_query`], which is `false`), so a real ID arms nothing.
    pub vendor_id: u32,
    /// HW-RT rung R1: `VkPhysicalDeviceProperties::deviceID` — the PCI device ID
    /// (real value; part of the R3 calibration-cache key). RECORDED ONLY (see
    /// [`Self::vendor_id`]).
    pub device_id: u32,
    /// HW-RT rung R1: `VkPhysicalDeviceProperties::driverVersion` — the
    /// vendor-encoded driver version (real value; part of the R3
    /// calibration-cache key). RECORDED ONLY (see [`Self::vendor_id`]).
    pub driver_version: u32,
    /// HW-RT rung R2a-1: `VkPhysicalDeviceAccelerationStructurePropertiesKHR
    /// ::minAccelerationStructureScratchOffsetAlignment` — the byte alignment a build's
    /// scratch device address MUST satisfy (=128 on Ampere/RTX 3060). `0` when ray query is
    /// off (`hwrt` compiled out, OR the device lacks the RT extensions): the AS build path
    /// (R2a-2) is then never reached. RECORDED; consumed by the scratch-buffer suballocator
    /// at R2a-2 — do NOT trust the buffer memreq alignment for scratch.
    pub as_scratch_align: u64,
    /// SSAA W2: `VkPhysicalDeviceLimits::maxImageDimension2D` — the device's max 2D image
    /// extent per axis. The boot arming probe requires `native * SSAA_SCALE <=` this on
    /// BOTH axes before committing the 2× `composite_extent`; on failure SSAA degrades to
    /// `Off` (never a panic). RECORDED ONLY here — read via `WindowHost::boot`
    /// (`boyko_app`), which this crate does not depend on.
    pub max_image_dimension_2d: u32,
    /// SSAA W2: the largest `DEVICE_LOCAL` heap size (bytes) reported by
    /// `vkGetPhysicalDeviceMemoryProperties`. The boot arming probe requires the estimated
    /// 2× ring VRAM cost to stay under half of this before committing SSAA; on failure SSAA
    /// degrades to `Off` (never an allocation panic).
    pub device_local_heap_bytes: u64,
    /// Multi-paradigm render-path plan, rung R-VBGEO (Decision 0 / P2-c):
    /// `VkPhysicalDeviceLimits::maxBoundDescriptorSets`. The `VisibilityBuffer` path's
    /// bindless geometry table lives in its own Set 3 (Set 0/1/2 + Set 3 = 4 bound sets),
    /// so `MeshGeometryTable::new` `debug_assert!`s this is `>= 4` at construction — the
    /// Vulkan spec's guaranteed floor is exactly 4, so this always holds on a conformant
    /// device; RECORDED (not a boot fail-fast) since a booted context never needs this
    /// value until a live `MeshGeometryTable` is actually constructed (R8+).
    pub max_bound_descriptor_sets: u32,
}

impl DeviceCaps {
    /// The SDF brick-atlas image format chosen from [`Self::atlas_linear_filter_ok`]
    /// (SDF brick-atlas campaign M2): `R8_SNORM` when the GPU supports linear filtering on
    /// it (the dense quantized path), else the `R16_SFLOAT` D8 fallback (half-float, no
    /// quantization — the `EPSILON_Q` store bias is harmless there). Both the CPU baker and
    /// the GPU decode handle either format. Returned as the agnostic [`Format`](boyko_rhi::Format) the
    /// `create_texture` path maps to a `VkFormat`.
    #[inline]
    pub const fn atlas_format(&self) -> boyko_rhi::Format {
        if self.atlas_linear_filter_ok {
            boyko_rhi::Format::R8Snorm
        } else {
            boyko_rhi::Format::R16Sfloat
        }
    }

    /// Whether BOTH SDFDDGI atlas storage formats (`B10G11R11_UFLOAT` irradiance +
    /// `R16G16_SFLOAT` depth) support `STORAGE_IMAGE` under OPTIMAL tiling — the precondition
    /// for the I2 probe-update compute WRITE. When `false`, the atlas is created without storage
    /// and DDGI is clamped permanently disabled (graceful degradation — DDGI is opt-in, plan
    /// §3): both `DdgiAtlas::create` (usage selection) and `resolve_ddgi_grid` (the disabled
    /// clamp) read this single predicate so they cannot disagree.
    #[inline]
    pub const fn ddgi_storage_ok(&self) -> bool {
        self.ddgi_irr_storage_ok && self.ddgi_depth_storage_ok
    }

    /// Rung 3a: whether the RT soft-shadow denoise storage format `R16G16_UNORM` supports
    /// `STORAGE_IMAGE` under OPTIMAL tiling — the precondition for the VIS/à-trous compute WRITEs.
    /// BOTH ping-pong rings (`shadow_vis` + `shadow_vis2`) are R16G16_UNORM now (the uniform-RG16
    /// design that lets one `"rg16"` shader pin fit every binding on every parity), so this gate is
    /// rg16-only — the former RG8 probe is no longer part of the denoise precondition. When `false`,
    /// the denoise targets are not used and the spatial denoise stays disabled (graceful degradation
    /// — the denoise is opt-in, `feature = "hwrt"` + config `Spatial`), mirroring
    /// [`Self::ddgi_storage_ok`]. Read through this single predicate so the target-allocation and the
    /// (steps 4-7) activation gate cannot disagree.
    #[cfg(feature = "hwrt")]
    #[inline]
    pub const fn shadow_denoise_storage_ok(&self) -> bool {
        self.rg16_unorm_storage_ok
    }

    /// The SSAO à-trous denoise chain: whether `R16_UNORM` supports `STORAGE_IMAGE` under
    /// OPTIMAL tiling — the precondition for the interior ping-pong ring's WRITEs. When `false`,
    /// the ring is not allocated and the resolve reads the raw (un-denoised) `sdf_ssao` gather —
    /// graceful degradation (software, mirrors `Self::shadow_denoise_storage_ok`'s pattern one
    /// channel narrower, but NOT `hwrt`-gated).
    #[inline]
    pub const fn ssao_atrous_storage_ok(&self) -> bool {
        self.r16_unorm_storage_ok
    }

    /// HW-RT rung R0: whether GPU timestamp measurement is USABLE on this device — the
    /// queue family reports at least one valid timestamp bit AND the period is a plausible
    /// ns/tick (`0 < period < 1000`). The plausibility bound also degrades a WRONG
    /// [`crate::ffi::LIMITS_OFF_TIMESTAMP_PERIOD`] offset to a graceful skip (never a fake
    /// timing). The GPU-timing harness prints a skip line + returns when this is `false`; it
    /// NEVER panics.
    #[inline]
    pub const fn timestamps_usable(&self) -> bool {
        self.timestamp_valid_bits > 0 && self.timestamp_period > 0.0 && self.timestamp_period < 1000.0
    }

    /// HW-RT rung R0: the low-bit mask for a raw timestamp (`(1 << valid_bits) - 1`),
    /// guarding the `1u64 << 64` shift UB — a `valid_bits >= 64` family (all bits valid)
    /// yields `u64::MAX`. AND both bracket endpoints with this BEFORE subtracting; high bits
    /// above the valid width are hardware garbage.
    #[inline]
    pub const fn timestamp_mask(&self) -> u64 {
        if self.timestamp_valid_bits >= 64 {
            u64::MAX
        } else {
            (1u64 << self.timestamp_valid_bits) - 1
        }
    }

    /// HW-RT rung R1: the ray-tracing [`RtTier`] this device resolves to — the
    /// dormant seam the later backend routing gates on. Pure function of the
    /// recorded RT feature bits: no hardware ray query ⇒ [`RtTier::Absent`]; ray
    /// query without reorder ⇒ [`RtTier::Weak`]; ray query with reorder ⇒
    /// [`RtTier::Strong`].
    ///
    /// In R1 [`Self::ray_query`] is hard-wired `false` (no RT extension is
    /// requested), so this returns [`RtTier::Absent`] for EVERY device — the
    /// dormancy anchor the byte-identity proof rests on. R2a's real
    /// presence+enable query brings the `Weak`/`Strong` arms alive.
    #[inline]
    pub const fn rt_tier(&self) -> RtTier {
        if !self.ray_query {
            RtTier::Absent
        } else if self.ray_reorder {
            RtTier::Strong
        } else {
            RtTier::Weak
        }
    }
}

/// Global-scope Vulkan commands (resolved with a NULL instance).
struct GlobalFns {
    create_instance: PfnVkCreateInstance,
    enumerate_instance_layer_properties: PfnVkEnumerateInstanceLayerProperties,
    enumerate_instance_extension_properties: PfnVkEnumerateInstanceExtensionProperties,
}

/// Instance-scope Vulkan commands.
struct InstanceFns {
    destroy_instance: PfnVkDestroyInstance,
    enumerate_physical_devices: PfnVkEnumeratePhysicalDevices,
    get_physical_device_properties: PfnVkGetPhysicalDeviceProperties,
    get_physical_device_memory_properties: PfnVkGetPhysicalDeviceMemoryProperties,
    get_physical_device_queue_family_properties: PfnVkGetPhysicalDeviceQueueFamilyProperties,
    /// `vkGetPhysicalDeviceFeatures2` (Vulkan 1.1 core) — the S0 fail-fast
    /// `dynamicRendering` support query (Correction #2). Always present at API 1.3.
    get_physical_device_features2: PfnVkGetPhysicalDeviceFeatures2,
    /// `vkGetPhysicalDeviceFormatProperties` (Vulkan 1.0 core) — the Render P1b
    /// device-caps query for G-buffer storage-image format support. Always present.
    get_physical_device_format_properties: PfnVkGetPhysicalDeviceFormatProperties,
    create_device: PfnVkCreateDevice,
    get_device_proc_addr: PfnVkGetDeviceProcAddr,
    /// `VK_EXT_debug_utils` destroyer — `Some` only when validation is enabled
    /// (the extension command resolves only with the extension enabled).
    destroy_debug_messenger: Option<PfnVkDestroyDebugUtilsMessengerExt>,
    /// `VK_KHR_surface` / `VK_KHR_win32_surface` instance commands — `Some` only
    /// when a windowed context is requested (they resolve only with the surface
    /// extensions enabled).
    surface: Option<SurfaceInstanceFns>,
}

/// `VK_KHR_surface` + `VK_KHR_win32_surface` instance-scope commands (windowed
/// contexts only). `SurfaceFns` (the public swapchain-facing view) is built from
/// these by [`crate::swapchain`].
pub struct SurfaceInstanceFns {
    pub create_win32_surface: PfnVkCreateWin32SurfaceKhr,
    pub destroy_surface: PfnVkDestroySurfaceKhr,
    pub get_surface_support: PfnVkGetPhysicalDeviceSurfaceSupportKhr,
    pub get_surface_capabilities: PfnVkGetPhysicalDeviceSurfaceCapabilitiesKhr,
    pub get_surface_formats: PfnVkGetPhysicalDeviceSurfaceFormatsKhr,
    pub get_surface_present_modes: PfnVkGetPhysicalDeviceSurfacePresentModesKhr,
}

/// `VK_KHR_swapchain` device-scope commands (windowed contexts only).
pub struct SwapchainDeviceFns {
    pub create_swapchain: PfnVkCreateSwapchainKhr,
    pub destroy_swapchain: PfnVkDestroySwapchainKhr,
    pub get_swapchain_images: PfnVkGetSwapchainImagesKhr,
    pub acquire_next_image: PfnVkAcquireNextImageKhr,
    pub queue_present: PfnVkQueuePresentKhr,
}

/// Device-scope Vulkan commands needed for the buffer round-trip and the
/// Slice-0 0c/0d compute dispatch + chained-barrier passes.
pub struct DeviceFns {
    pub destroy_device: PfnVkDestroyDevice,
    pub get_device_queue: PfnVkGetDeviceQueue,
    pub create_buffer: PfnVkCreateBuffer,
    pub destroy_buffer: PfnVkDestroyBuffer,
    pub get_buffer_memory_requirements: PfnVkGetBufferMemoryRequirements,
    pub allocate_memory: PfnVkAllocateMemory,
    pub free_memory: PfnVkFreeMemory,
    pub bind_buffer_memory: PfnVkBindBufferMemory,
    pub map_memory: PfnVkMapMemory,
    pub unmap_memory: PfnVkUnmapMemory,
    // --- 0c/0d compute commands. ---
    pub create_shader_module: PfnVkCreateShaderModule,
    pub destroy_shader_module: PfnVkDestroyShaderModule,
    pub create_descriptor_set_layout: PfnVkCreateDescriptorSetLayout,
    pub destroy_descriptor_set_layout: PfnVkDestroyDescriptorSetLayout,
    pub create_pipeline_layout: PfnVkCreatePipelineLayout,
    pub destroy_pipeline_layout: PfnVkDestroyPipelineLayout,
    pub create_compute_pipelines: PfnVkCreateComputePipelines,
    pub destroy_pipeline: PfnVkDestroyPipeline,
    pub create_descriptor_pool: PfnVkCreateDescriptorPool,
    pub destroy_descriptor_pool: PfnVkDestroyDescriptorPool,
    pub allocate_descriptor_sets: PfnVkAllocateDescriptorSets,
    pub update_descriptor_sets: PfnVkUpdateDescriptorSets,
    pub create_command_pool: PfnVkCreateCommandPool,
    pub destroy_command_pool: PfnVkDestroyCommandPool,
    pub allocate_command_buffers: PfnVkAllocateCommandBuffers,
    pub free_command_buffers: PfnVkFreeCommandBuffers,
    pub begin_command_buffer: PfnVkBeginCommandBuffer,
    pub end_command_buffer: PfnVkEndCommandBuffer,
    pub cmd_bind_pipeline: PfnVkCmdBindPipeline,
    pub cmd_bind_descriptor_sets: PfnVkCmdBindDescriptorSets,
    pub cmd_push_constants: PfnVkCmdPushConstants,
    pub cmd_dispatch: PfnVkCmdDispatch,
    /// `vkCmdDispatchIndirect` — virtual-geometry rung R1's half of the indirect seam on the
    /// compute side (Vulkan 1.0 core, no feature bit, always present).
    pub cmd_dispatch_indirect: PfnVkCmdDispatchIndirect,
    pub cmd_pipeline_barrier: PfnVkCmdPipelineBarrier,
    /// `vkCmdCopyBuffer` — the Phase-5 staging upload + readback transfer
    /// (Vulkan 1.0 core, always present).
    pub cmd_copy_buffer: PfnVkCmdCopyBuffer,
    /// `vkCmdFillBuffer` — the Lighting-L1 cull's per-frame reset of the `LightIndexAlloc`
    /// counter to 0 before the cull dispatch (Vulkan 1.0 core, always present).
    pub cmd_fill_buffer: PfnVkCmdFillBuffer,
    /// `vkCmdUpdateBuffer` — virtual-geometry rung R2a': the inline (<=64 KiB) TRANSFER write that
    /// fills the indirect-draw records. Vulkan 1.0 core, no feature bit, always present.
    pub cmd_update_buffer: PfnVkCmdUpdateBuffer,
    /// `vkCmdClearColorImage` — the SDFDDGI I1 boot-clear of the probe IRRADIANCE + DEPTH
    /// color atlases to defined values (Vulkan 1.0 core, always present).
    pub cmd_clear_color_image: PfnVkCmdClearColorImage,
    pub create_fence: PfnVkCreateFence,
    pub destroy_fence: PfnVkDestroyFence,
    pub wait_for_fences: PfnVkWaitForFences,
    pub queue_submit: PfnVkQueueSubmit,
    pub device_wait_idle: PfnVkDeviceWaitIdle,
    // --- HW-RT rung R0 GPU timestamp-query commands (Vulkan 1.0 core, always present). ---
    pub create_query_pool: PfnVkCreateQueryPool,
    pub destroy_query_pool: PfnVkDestroyQueryPool,
    pub cmd_reset_query_pool: PfnVkCmdResetQueryPool,
    pub cmd_write_timestamp: PfnVkCmdWriteTimestamp,
    pub get_query_pool_results: PfnVkGetQueryPoolResults,
    /// Profiling rung 4: `vkResetQueryPool`, Vulkan 1.2 core, so it LOADS on this engine's
    /// 1.3 device unconditionally. Loading it is not permission to call it — that needs the
    /// `hostQueryReset` feature enabled at device creation, which
    /// [`DeviceCaps::host_query_reset`] records.
    pub reset_query_pool: PfnVkResetQueryPool,
    /// Profiling rung 9: `vkGetCalibratedTimestampsEXT`.
    ///
    /// `Option`, unlike every sibling above, because it is an EXTENSION command: it resolves only
    /// when `VK_EXT_calibrated_timestamps` was enabled at device creation. A `?`-load would turn a
    /// device without the extension — a perfectly ordinary device — into a boot failure. `None`
    /// and [`DeviceCaps::calibrated_timestamps`] `false` are set from the one probe, so they
    /// cannot disagree.
    pub get_calibrated_timestamps: Option<PfnVkGetCalibratedTimestampsExt>,
    // --- Slice-1 core (Vulkan 1.0 / 1.3) commands, always loaded. ---
    pub reset_fences: PfnVkResetFences,
    pub create_image_view: PfnVkCreateImageView,
    pub destroy_image_view: PfnVkDestroyImageView,
    pub create_semaphore: PfnVkCreateSemaphore,
    pub destroy_semaphore: PfnVkDestroySemaphore,
    pub cmd_begin_rendering: PfnVkCmdBeginRendering,
    pub cmd_end_rendering: PfnVkCmdEndRendering,
    // --- Phase-6 S0 image (`create_texture`) + image-copy (readback) commands,
    //     Vulkan 1.0 core, always loaded. ---
    pub create_image: PfnVkCreateImage,
    pub destroy_image: PfnVkDestroyImage,
    pub get_image_memory_requirements: PfnVkGetImageMemoryRequirements,
    pub bind_image_memory: PfnVkBindImageMemory,
    pub cmd_copy_image_to_buffer: PfnVkCmdCopyImageToBuffer,
    /// `vkCmdCopyBufferToImage` — the rung-11 composite-buffer → SAMPLED-texture
    /// upload (the symmetric counterpart of `cmd_copy_image_to_buffer`; Vulkan 1.0
    /// core, always present).
    pub cmd_copy_buffer_to_image: PfnVkCmdCopyBufferToImage,
    /// `vkCmdBlitImage` — the textured-PBR T2 mip-chain-generation blit (Decision
    /// D3; Vulkan 1.0 core, always present).
    pub cmd_blit_image: PfnVkCmdBlitImage,
    // --- Phase-6 S0 rung-2 graphics-pipeline + draw commands, Vulkan 1.0 core,
    //     always loaded. ---
    pub create_graphics_pipelines: PfnVkCreateGraphicsPipelines,
    pub cmd_set_viewport: PfnVkCmdSetViewport,
    pub cmd_set_scissor: PfnVkCmdSetScissor,
    pub cmd_draw: PfnVkCmdDraw,
    /// `vkCmdDrawIndexed` — the indexed-draw counterpart of `cmd_draw` (mesh M0;
    /// requires a bound index buffer via `cmd_bind_index_buffer`; Vulkan 1.0 core,
    /// always present).
    pub cmd_draw_indexed: PfnVkCmdDrawIndexed,
    /// `vkCmdDrawIndexedIndirect` — virtual-geometry rung R1's half of the indirect seam on the
    /// graphics side (Vulkan 1.0 core, no feature bit, always present). The `Count` variant is
    /// deliberately NOT loaded: it needs `drawIndirectCount` in a `VkPhysicalDeviceVulkan12Features`
    /// this device never chains, so loading it here would fail on a conformant 1.0 driver.
    pub cmd_draw_indexed_indirect: PfnVkCmdDrawIndexedIndirect,
    // --- Phase-6 S0 rung-3 vertex/index buffer bind commands, Vulkan 1.0 core,
    //     always loaded. ---
    pub cmd_bind_vertex_buffers: PfnVkCmdBindVertexBuffers,
    pub cmd_bind_index_buffer: PfnVkCmdBindIndexBuffer,
    // --- Phase-6 S0 rung-5 sampler create/destroy, Vulkan 1.0 core, always loaded. ---
    pub create_sampler: PfnVkCreateSampler,
    pub destroy_sampler: PfnVkDestroySampler,
    // --- Slice-1 `VK_KHR_swapchain` device commands — `Some` only when windowed. ---
    pub swapchain: Option<SwapchainDeviceFns>,
}

/// A booted Vulkan context: a loaded loader, an instance, a logical device and
/// one graphics+compute queue, with the device commands resolved.
pub struct VulkanContext {
    /// HMODULE for `vulkan-1.dll`; freed in `Drop`. Opaque pointer.
    module: *mut c_void,
    instance: VkInstance,
    physical_device: VkPhysicalDevice,
    device: VkDevice,
    queue: VkQueue,
    queue_family_index: u32,
    /// Cached physical-device memory properties (for memory-type selection).
    memory_properties: VkPhysicalDeviceMemoryProperties,
    /// Human-readable device name (from `VkPhysicalDeviceProperties`).
    device_name: String,
    /// Minimal physical-device capabilities queried once at boot (Render P1b). A POD
    /// `Copy` cache; `gbuffer_storage_format_ok` is always `true` here (the boot
    /// fail-fast rejected a GPU lacking it).
    device_caps: DeviceCaps,
    /// The validation-message messenger (`NULL` when validation is disabled).
    debug_messenger: VkDebugUtilsMessengerEXT,
    /// Heap-owned callback state pointed-to by the messenger's `p_user_data`.
    /// `None` when validation is disabled. Boxed so its address is stable across
    /// moves of the context (the messenger holds a raw pointer into it); dropped
    /// AFTER the messenger is destroyed in `Drop`.
    debug_state: Option<Box<DebugMessengerState>>,
    instance_fns: InstanceFns,
    /// The resolved device command table, **heap-boxed** so its address is stable
    /// across moves of this (`pub`, by-value-returned) context (plan A1). The host
    /// block, the [`VulkanQueue`](crate::rhi_impl::VulkanQueue) and the
    /// [`VulkanCommandEncoder`](crate::rhi_impl::VulkanCommandEncoder) cache a raw
    /// `*const DeviceFns` pointing into this allocation; a context move relocates
    /// the `Box` handle but NOT the pointee, so those caches survive the move.
    /// Dropped implicitly last (after the host block + compute layouts in `Drop`),
    /// so it outlives every cache that points into it.
    device_fns: Box<DeviceFns>,
    /// The shared compute descriptor-set + pipeline layouts (one STORAGE_BUFFER
    /// @ set0/binding0 + a 4-byte push range), cached on first
    /// `create_compute_pipeline` / `create_command_encoder` (plan Q1/W2).
    ///
    /// Created lazily through [`VulkanContext::compute_layouts`]; `OnceCell` is
    /// the single-threaded once-init primitive — the RHI is touched only by the
    /// dispatcher in the apply-window (`!Send + !Sync`, plan §5.3), so no atomic
    /// `OnceLock` is needed. Torn down in `Drop` BEFORE `vkDestroyDevice`, so the
    /// layouts never outlive their device.
    compute_layouts: OnceCell<ComputeLayouts>,
    /// The **growable pool** of host-visible+coherent blocks every
    /// [`RhiDevice::create_buffer`](boyko_rhi::RhiDevice::create_buffer) sub-allocates
    /// from (plan Q1). Empty until the first allocation; blocks are appended on
    /// demand.
    ///
    /// ⚠️ **This was ONE block of a fixed 64 MiB with no growth path**, which made
    /// 64 MiB a hard ceiling on every host-visible resource in the engine — for
    /// mesh geometry (~44 B/triangle) roughly **1.5 M triangles** total, failing
    /// as a `vkCreateBuffer` panic rather than a recoverable `Err`. VG-R0's
    /// staging rung S1 replaced it with [`BlockPool`]; see that type's docs.
    ///
    /// Each block caches a raw `*const DeviceFns` into the boxed `device_fns`
    /// (plan A1): the box gives the fn-table a stable heap address, so the cached
    /// pointer survives any move of this context — no false `'static` lifetime is
    /// claimed. Blocks are torn down in `Drop` BEFORE `vkDestroyDevice` + before
    /// the boxed fn-table is freed, so the pointer is live for every block use.
    /// The `RefCell` provides the `&mut` the sub-allocator needs from `&self`
    /// calls (single-threaded, `!Sync`).
    #[allow(clippy::disallowed_types)]
    host_pool: RefCell<BlockPool<HostVisibleBlock>>,
    /// The **growable pool** of device-local (VRAM) blocks every
    /// [`RhiDevice::create_buffer`](boyko_rhi::RhiDevice::create_buffer) with
    /// [`MemoryLocation::DeviceLocal`](boyko_rhi::MemoryLocation::DeviceLocal)
    /// sub-allocates from (the Phase-5 `GpuColumn` seam). Never mapped (plan
    /// D3/MF-8). Same plan-A1 `*const DeviceFns` contract and the same `Drop`
    /// ordering as the host pool above; it carried the identical 64 MiB ceiling,
    /// which is why moving mesh data here would only have relocated it.
    #[allow(clippy::disallowed_types)]
    device_pool: RefCell<BlockPool<DeviceLocalBlock>>,
    /// HW-RT rung R2a-1: the resolved `VK_KHR_acceleration_structure` command table,
    /// `Some` ONLY when the RT extensions were enabled at device create (mirroring
    /// `DeviceFns::swapchain: Option<SwapchainDeviceFns>`). `None` when the device lacks
    /// ray query — the AS verbs then return `Unsupported`. Gated `hwrt`: the field itself
    /// is absent from a default build, so `VulkanContext`'s layout is textually R1 there.
    #[cfg(feature = "hwrt")]
    accel_fns: Option<crate::accel::AccelFns>,
    /// Multi-paradigm render-path plan, rung R-VBGEO (Decision 0 / Rev-5 streaming
    /// invariant): whether the boot-committed `ResolvedRenderPath.vb_geometry_table` is
    /// `true` for this run. Set EXACTLY ONCE, by `boyko_app::runner`, right after
    /// `resolve_render_path` — BEFORE `app.finish()` drains any startup system that might
    /// register a mesh, and BEFORE the boot one-shot `upload_mesh_assets` drain (the
    /// Rev-5 "flag reaches the registration site before the first mesh upload" gate).
    /// `OnceCell` (not a plain field) because `VulkanContext` is fully constructed at
    /// `boot()`/`boot_singleton()` time, BEFORE the render-path resolve exists — the SAME
    /// "settable once after construction, read many times, single-threaded" shape
    /// [`Self::compute_layouts`] already uses. Read via [`Self::vb_geometry_table_armed`],
    /// which defaults to `false` if never set — the case for a context booted outside the
    /// `boyko_app::runner` seam (RHI-level tests), NOT for a VB boot: `VB_IMPLEMENTED` is
    /// `true` in `boyko_render::render_path_config`, so a `VisibilityBuffer x Mesh` resolve
    /// on a capable device sets this `true`. `ctx: &VulkanContext` is
    /// already the channel present at EVERY mesh-registration call site
    /// (`build_mesh_gpu`/`register_mesh`/`cube`/`plane`/the streamed `GpuUpload` path), so
    /// this is a zero-signature-change way to thread the flag universally (mirrors
    /// `DeviceCaps::storage_buffer_array_non_uniform_indexing_ok`'s "device/context config
    /// already reaches every call site" channel, one layer up).
    vb_geometry_table_armed: OnceCell<bool>,
}

/// The retained OWNING pointer behind [`VulkanContext::boot_singleton`] /
/// [`VulkanContext::destroy_singleton`] (host plan D2, review-P0 soundness
/// shape): the `&'static` `boot_singleton` hands out is DERIVED from this
/// pointer, and `destroy_singleton` reclaims the allocation through it — never
/// through a shared reference (a shared reference carries no ownership-capable
/// tag, so `Box::from_raw` from one is Stacked/Tree-Borrows UB). Null ⇔ no live
/// singleton; the CAS/swap pair doubles as the second-boot error and the
/// exactly-once destroy tripwire. Private by design: the lifecycle pair is the
/// only code that may touch it.
static SINGLETON: AtomicPtr<VulkanContext> = AtomicPtr::new(ptr::null_mut());

impl VulkanContext {
    /// Boots a headless Vulkan context, picking a discrete GPU if available.
    ///
    /// Returns a [`BootError`] (never panics) on any loader / driver / GPU
    /// absence so the caller can skip gracefully on a GPU-less machine.
    pub fn boot(config: InstanceConfig) -> Result<Self, BootError> {
        // --- 0. Validation escape hatch. ---
        // Normalize `enable_validation` to its EFFECTIVE value ONCE, at the single
        // entry point, before `config` (passed BY VALUE / `Copy`) flows into
        // `create_instance`, `boot_with_instance`, `load_instance_fns` and
        // `create_debug_messenger`. Every downstream read — including the
        // `validation_enabled()` accessor, which reflects whether a messenger was
        // created — therefore sees the effective flag with NO per-site changes.
        //
        // WHY the env gate: on this windows-gnu (MinGW) box the VulkanSDK
        // `VkLayer_khronos_validation.dll` (an MSVC build) crashes the MinGW
        // process (0xc0000005) on LOAD, so `vkCreateInstance` faults whenever the
        // layer is requested-and-present, and boot returns `ValidationUnavailable`
        // when it is absent — either way no GPU pixel golden can run. The render
        // OUTPUT does not depend on validation (it only catches API misuse), so
        // `BOYKO_DISABLE_VALIDATION` lets the goldens boot WITHOUT the layer.
        //
        // DEFAULT (env unset): `validation_requested` returns `config.enable_validation`
        // unchanged — byte-identical to prior behavior; this is a pure opt-in.
        let requested_by_caller = config.enable_validation;
        let config = InstanceConfig {
            enable_validation: validation_requested(&config),
            ..config
        };
        if requested_by_caller && !config.enable_validation {
            report_validation_withheld_by_env();
        }

        // --- 1. Load the loader DLL + vkGetInstanceProcAddr. ---
        let module = load_vulkan_loader().ok_or(BootError::LoaderUnavailable)?;

        // SAFETY: `module` is the live HMODULE just returned by `LoadLibraryA`;
        // `GetProcAddress` with a valid NUL-terminated symbol returns the
        // exported address or NULL. We immediately null-check before any use.
        let gipa_raw = unsafe { os_get_proc(module, c"vkGetInstanceProcAddr") };
        let Some(gipa_fn) = gipa_raw else {
            // SAFETY: `module` is the live HMODULE; freeing it on this early-out
            // path matches the single LoadLibraryA above (no double free — we
            // return before storing it in `self`).
            unsafe { free_vulkan_loader(module) };
            return Err(BootError::LoaderUnavailable);
        };
        // SAFETY: `vkGetInstanceProcAddr` has the `PfnVkGetInstanceProcAddr`
        // ABI by the Vulkan spec; transmuting the loader's exported function
        // pointer (an `extern "system" fn()`) to that signature is the
        // documented bootstrap contract.
        let get_instance_proc_addr: PfnVkGetInstanceProcAddr =
            unsafe { mem::transmute::<unsafe extern "system" fn(), PfnVkGetInstanceProcAddr>(gipa_fn) };

        // --- 2. Global commands (NULL-instance scope). ---
        let global = match load_global_fns(get_instance_proc_addr) {
            Ok(g) => g,
            Err(e) => {
                // SAFETY: see the early-out above — `module` is live and freed
                // exactly once on this path.
                unsafe { free_vulkan_loader(module) };
                return Err(e);
            }
        };

        // --- 3. Create the instance (optional validation layer). ---
        let instance = match create_instance(&global, get_instance_proc_addr, config) {
            Ok(i) => i,
            Err(e) => {
                unsafe { free_vulkan_loader(module) };
                return Err(e);
            }
        };

        // From here on, `instance` must be destroyed on every error path. A
        // small RAII-on-error helper keeps the early returns honest.
        let result = Self::boot_with_instance(
            module,
            instance,
            get_instance_proc_addr,
            config,
        );
        match result {
            Ok(ctx) => Ok(ctx),
            Err((e, instance_fns)) => {
                // SAFETY: `instance` was created above and not yet stored in a
                // live context; `destroy_instance` is the matching teardown,
                // called exactly once before the loader is freed. Any debug
                // messenger created inside `boot_with_instance` is already
                // destroyed there before this error is surfaced, so no messenger
                // outlives its instance.
                unsafe { (instance_fns.destroy_instance)(instance, ptr::null()) };
                // SAFETY: `module` is live and freed exactly once here.
                unsafe { free_vulkan_loader(module) };
                Err(e)
            }
        }
    }

    /// Boots the device and pins it as the process singleton: the returned
    /// `&'static` is the ONE device handle every layer (host, World resources)
    /// shares. Immutable after boot — no `&mut VulkanContext` ever exists
    /// again, so every holder sees one frozen, shared handle. Ended EXACTLY
    /// ONCE by [`destroy_singleton`](Self::destroy_singleton); the `'static`
    /// lifetime is a documented fiction that call ends (host plan D2).
    ///
    /// The OWNING raw pointer is retained in the private [`SINGLETON`] static
    /// (review-P0 soundness shape): the `&'static` handed out is derived FROM
    /// that pointer, so the later reclamation goes through the retained,
    /// ownership-capable raw pointer — never through a shared reference. The
    /// mechanics live HERE, next to the type they manage — the host
    /// (`boyko_app`) only calls this pair.
    ///
    /// # Errors
    ///
    /// - [`VulkanError::Boot`](crate::error::VulkanError::Boot) on any loader /
    ///   driver / GPU absence (never panics), so a GPU-less machine can skip
    ///   gracefully;
    /// - [`VulkanError::SingletonAlreadyBooted`](crate::error::VulkanError::SingletonAlreadyBooted)
    ///   if a live singleton already exists — a second boot is a contract
    ///   violation (the fast path rejects it before creating a second device;
    ///   the CAS-race path destroys the just-booted second device before
    ///   returning).
    pub fn boot_singleton(
        config: InstanceConfig,
    ) -> Result<&'static VulkanContext, crate::error::VulkanError> {
        // Advisory fast-fail so a contract-violating second boot does not
        // create (and immediately destroy) a whole second device. The
        // compare_exchange below remains the authority.
        // Acquire: matches the Release success ordering of the CAS below.
        if !SINGLETON.load(Ordering::Acquire).is_null() {
            return Err(crate::error::VulkanError::SingletonAlreadyBooted);
        }

        let ctx = Self::boot(config)?;
        let raw = Box::into_raw(Box::new(ctx));

        // Publish the owning pointer. Release on success: whoever observes
        // `raw` (the advisory load above, `destroy_singleton`'s swap) also
        // observes the fully-initialized `VulkanContext` behind it. Acquire on
        // failure: observe the racing publisher's state coherently.
        if SINGLETON
            .compare_exchange(ptr::null_mut(), raw, Ordering::Release, Ordering::Acquire)
            .is_err()
        {
            // Another boot published between the advisory load and the CAS —
            // this boot lost.
            // SAFETY: `raw` came from `Box::into_raw` just above, was NEVER
            // published (the CAS failed) and no reference was ever derived
            // from it — reconstructing and dropping the box is the unique
            // owner reclaiming its own fresh allocation; the second device is
            // fully torn down by the normal `Drop`.
            drop(unsafe { Box::from_raw(raw) });
            return Err(crate::error::VulkanError::SingletonAlreadyBooted);
        }

        // SAFETY: `raw` came from `Box::into_raw` (non-null, aligned, valid
        // for reads) and its allocation stays live until `destroy_singleton`
        // reclaims it through the SAME retained pointer in `SINGLETON` — the
        // shared reference minted here is derived FROM the owning pointer and
        // stays below it in the borrow stack, so the eventual `Box::from_raw`
        // does not go through this reference. The referent is write-never
        // after boot (no `&mut` ever exists again), so shared reads are sound
        // for as long as the caller upholds `destroy_singleton`'s contract.
        Ok(unsafe { &*raw })
    }

    /// Ends the singleton's lifecycle: destroys the device, the instance, the
    /// debug messenger, and frees the loader (the normal [`Drop`] path), then
    /// releases the pinned allocation.
    ///
    /// PARAMLESS by soundness necessity (review P0): the owning raw pointer is
    /// retained in the private [`SINGLETON`] static at boot. A `&'static`
    /// downgraded from `Box::leak`'s `&'static mut` loses the
    /// ownership-capable tag, so reconstructing a `Box` from a shared
    /// reference is Stacked/Tree-Borrows UB — and a reference-typed parameter
    /// is PROTECTED for the call duration, making deallocation of its referent
    /// inside the call UB regardless of which pointer performs it. Taking no
    /// parameter and deallocating through the retained raw pointer avoids
    /// both.
    ///
    /// # Panics
    ///
    /// Panics if no live singleton exists (`boot_singleton` never succeeded,
    /// or `destroy_singleton` already ran) — the exactly-once tripwire fires
    /// on the null-swap BEFORE any memory is touched.
    ///
    /// # Safety
    ///
    /// The caller guarantees that:
    /// - a [`boot_singleton`](Self::boot_singleton) succeeded and its
    ///   singleton is still live (the tripwire panics otherwise, before
    ///   touching memory — but exactly-once remains the caller's contract);
    /// - the device is idle (no submitted GPU work still references it);
    /// - NO `&'static VulkanContext` reference obtained from `boot_singleton`
    ///   EXISTS anywhere any more (host structs dropped, World GPU residents
    ///   evicted, no copy stashed in any live structure) — reference validity,
    ///   not merely "no deref": the `'static` is a documented fiction this
    ///   call ends, and any surviving reference would dangle.
    pub unsafe fn destroy_singleton() {
        // AcqRel: the Acquire half matches the Release success ordering of the
        // CAS in `boot_singleton` (this thread observes the fully-initialized
        // context before dropping it); the Release half orders the
        // null-publish after this thread's prior device use.
        let raw = SINGLETON.swap(ptr::null_mut(), Ordering::AcqRel);
        if raw.is_null() {
            panic!(
                "invariant: destroy_singleton with no live device singleton \
                 (boot_singleton never succeeded, or destroy_singleton ran twice)"
            );
        }
        // SAFETY: `raw` is the exact pointer `boot_singleton`'s `Box::into_raw`
        // stashed in `SINGLETON` (correct provenance + layout for
        // `Box::from_raw`); the swap-to-null above guarantees no other call
        // can observe or reclaim it again (exactly once); and per this fn's
        // contract no `&'static VulkanContext` derived from it survives — so
        // re-owning the box and dropping it (the normal `VulkanContext::Drop`
        // teardown: device → messenger → instance → loader) cannot double-free
        // or invalidate a live borrow.
        drop(unsafe { Box::from_raw(raw) });
    }

    /// Continues the boot once the instance exists. On error it returns the
    /// loaded [`InstanceFns`] so the caller can destroy the instance with the
    /// correct command pointer. Any debug messenger this creates is destroyed
    /// in-place on the error paths (before returning), so the caller only ever
    /// has to destroy the instance + free the loader.
    // `InstanceFns` is a command table of function pointers carried back on the
    // COLD error path purely so the caller can destroy the instance with the
    // right destroyer; boxing it would add a heap alloc to a once-per-boot
    // failure path with no benefit (the table is moved, not copied around).
    #[allow(clippy::result_large_err)]
    fn boot_with_instance(
        module: *mut c_void,
        instance: VkInstance,
        gipa: PfnVkGetInstanceProcAddr,
        config: InstanceConfig,
    ) -> Result<Self, (BootError, InstanceFns)> {
        let instance_fns = match load_instance_fns(gipa, instance, config) {
            Ok(f) => f,
            // No fns loaded → we cannot even destroy the instance with a typed
            // pointer; load just the destroyer best-effort. If even that is
            // missing the instance leaks, but that is a broken-loader corner
            // the spec does not allow.
            Err(e) => return Err((e, fallback_instance_fns(gipa, instance))),
        };

        // --- 3b. Create the validation messenger (Slice-0 0a oracle). ---
        // Boxed so the callback's `p_user_data` pointer is address-stable.
        // `debug_messenger`/`debug_state` are NULL/None when validation is off.
        let (debug_messenger, debug_state) =
            match create_debug_messenger(gipa, instance, config.enable_validation) {
                Ok(pair) => pair,
                Err(e) => return Err((e, instance_fns)),
            };

        // After this point every error path MUST destroy the messenger before
        // returning (the instance destroyer alone would leave it dangling).
        macro_rules! fail {
            ($err:expr) => {{
                // SAFETY: `debug_messenger` (if non-null) was just created on
                // `instance` with `instance_fns.destroy_debug_messenger`
                // resolved (it is `Some` exactly when a messenger exists);
                // destroying it here once, before the caller destroys the
                // instance, keeps teardown ordered. `debug_state` is dropped at
                // end of scope AFTER the messenger no longer references it.
                if !debug_messenger.is_null() {
                    if let Some(destroy) = instance_fns.destroy_debug_messenger {
                        unsafe { destroy(instance, debug_messenger, ptr::null()) };
                    }
                }
                drop(debug_state);
                return Err(($err, instance_fns));
            }};
        }

        // --- 4. Pick a physical device (prefer a discrete GPU). ---
        let (physical_device, device_name, memory_properties) =
            match pick_physical_device(&instance_fns, instance) {
                Ok(p) => p,
                Err(e) => fail!(e),
            };

        // --- 5. Find a graphics+compute queue family (+ its timestampValidBits, R0). ---
        let (queue_family_index, timestamp_valid_bits) =
            match find_queue_family(&instance_fns, physical_device) {
                Ok(q) => q,
                Err(e) => fail!(e),
            };

        // --- 5b. Query the minimal device caps ONCE (Render P1b), alongside the
        // `dynamicRendering` fail-fast in `create_device`. `gbuffer_storage_format_ok`
        // and (T-dev) `bindless_capable` are fail-fast here so a context that exists
        // always has them (a marcher storage-image store can never fault on an
        // unsupported format; the bindless descriptor path can never fault on
        // unenabled descriptor-indexing bits). Core-guaranteed on the RTX 3060.
        let mut device_caps = query_device_caps(&instance_fns, physical_device);
        // HW-RT rung R0: populate the two timestamp caps `query_device_caps` left at
        // placeholder zeros — the `timestampPeriod` from the physical-device limits blob +
        // the CHOSEN family's `timestampValidBits` (from `find_queue_family`). RECORDED (not
        // a fail-fast): `timestamps_usable()` degrades an unusable/implausible device to a
        // graceful skip in the GPU-timing harness.
        let device_props = query_device_properties(&instance_fns, physical_device);
        device_caps.timestamp_period = device_props.limits.read_f32(LIMITS_OFF_TIMESTAMP_PERIOD);
        device_caps.timestamp_valid_bits = timestamp_valid_bits;
        // VB-SV0 rung S1.5: `timestampComputeAndGraphics` is a `VkBool32` (0/1) one 4-byte
        // scalar before `timestampPeriod` in the same limits blob. RECORDED ONLY — it does not
        // gate anything (`timestamps_usable()` is unchanged); a timing harness prints it so its
        // resolution claim names the guarantee it rests on.
        device_caps.timestamp_compute_and_graphics =
            device_props.limits.read_u32(LIMITS_OFF_TIMESTAMP_COMPUTE_AND_GRAPHICS) != 0;
        // HW-RT rung R1: copy the real GPU identity from the physical-device properties
        // (`vendor_id`/`device_id`/`driver_version` are typed `u32` at the TOP of
        // `VkPhysicalDeviceProperties`, NOT in the opaque limits blob — plain field copies,
        // no offset math). RECORDED ONLY (the R3 calibration-cache key); `ray_query`/
        // `ray_reorder` stay `false` (no RT extension requested), so `rt_tier() == Absent`
        // for every device and nothing branches on these IDs in R1.
        device_caps.vendor_id = device_props.vendor_id;
        device_caps.device_id = device_props.device_id;
        device_caps.driver_version = device_props.driver_version;
        // SSAA W2: populate the arming-probe caps `query_device_caps` left at placeholder
        // zeros — `maxImageDimension2D` from the limits blob already read above, and the
        // largest `DEVICE_LOCAL` heap from `memory_properties` (already returned by
        // `pick_physical_device`, step 4). RECORDED ONLY: `boyko_app::WindowHost::boot`
        // reads both to decide whether to arm the 2× SSAA composite extent.
        device_caps.max_image_dimension_2d =
            device_props.limits.read_u32(LIMITS_OFF_MAX_IMAGE_DIMENSION_2D);
        device_caps.device_local_heap_bytes = max_device_local_heap_bytes(&memory_properties);
        // Multi-paradigm render-path plan, rung R-VBGEO: populate the placeholder
        // `query_device_caps` left at zero — mirrors `max_image_dimension_2d` immediately
        // above (the same physical-device limits blob, a different offset).
        device_caps.max_bound_descriptor_sets =
            device_props.limits.read_u32(LIMITS_OFF_MAX_BOUND_DESCRIPTOR_SETS);
        if !device_caps.gbuffer_storage_format_ok {
            fail!(BootError::GbufferStorageFormatUnsupported);
        }
        // W2 (Lighting L0b): the `gViewT` R32_SFLOAT lane is a compute-store target —
        // fail-fast here (mirroring the G-buffer check) so the new lane can never fault on
        // an unsupported format. Core-guaranteed on the RTX 3060.
        if !device_caps.viewt_storage_format_ok {
            fail!(BootError::ViewtStorageFormatUnsupported);
        }
        // Render P5-r0: the mesh raster pass A writes the R8G8B8A8_UNORM G-buffer images as
        // MRT COLOR attachments — fail-fast here (mirroring the storage checks) so the
        // producer can never device-lost on an unsupported usage. RGBA8_UNORM
        // color-attachment renderability is mandatory in Vulkan (core-guaranteed on the RTX).
        if !device_caps.gbuffer_color_attachment_format_ok {
            fail!(BootError::GbufferColorAttachmentFormatUnsupported);
        }
        // Render P7: the SSAO term `gSsao` is an R8_UNORM STORAGE image (resolve load + SSAO-
        // pass store) — fail-fast here (mirroring the storage checks) so the SSAO image can
        // never fault on an unsupported format. Core-guaranteed on the RTX 3060.
        if !device_caps.r8_unorm_storage_ok {
            fail!(BootError::SsaoStorageFormatUnsupported);
        }
        // T-dev: the textured-PBR T4 bindless descriptor path needs the 5
        // descriptor-indexing bits `create_device` enables — fail-fast here (mirroring
        // the format checks) so a GPU that cannot get them enabled is rejected at boot,
        // not discovered as an opaque shader fault later.
        if !device_caps.bindless_capable {
            fail!(BootError::BindlessUnsupported);
        }

        // HW-RT rung R2a-1: query ray-query support ONCE (presence + feature + props) BEFORE
        // device create — its result drives BOTH the RT-extension enable in `create_device`
        // AND the `accel_fns` load + caps below (a single query, no double-enumeration). On a
        // non-hwrt build the flag is the hard `false` `RT_ENABLE_DEFAULT` (dormancy anchor).
        #[cfg(feature = "hwrt")]
        let rt_caps = supports_ray_query(&instance_fns, gipa, instance, physical_device);
        #[cfg(feature = "hwrt")]
        let enable_ray_query = rt_caps.ray_query;
        #[cfg(not(feature = "hwrt"))]
        let enable_ray_query = RT_ENABLE_DEFAULT;

        // Multi-paradigm render-path plan, rung R8: enable the VB geometry table's two
        // descriptor-indexing bits (`shaderStorageBufferArrayNonUniformIndexing` +
        // `descriptorBindingStorageBufferUpdateAfterBind`) IFF the device already advertised
        // support (`device_caps.storage_buffer_array_non_uniform_indexing_ok`, queried above,
        // step 5b — the SAME "query before request" precedent `enable_ray_query` establishes).
        // Gated, not unconditional: a device that lacks the bit would otherwise fail
        // `vkCreateDevice` outright (requesting an unsupported feature bit is a hard error, not
        // a silent no-op) — closing R-VBGEO's documented "device-create gap" without risking a
        // boot regression on a device that lacks the bit (VB itself degrades to Deferred at
        // resolve time on such a device, `resolve_render_path`'s `VbDeviceCapMissing` rule; this
        // gate makes that degrade the ONLY behavior change, never a device-create failure).
        let enable_vb_geometry_table = device_caps.storage_buffer_array_non_uniform_indexing_ok;

        // Profiling rung 4 (D18): `hostQueryReset` on the SAME "query before request" precedent.
        // It is an OPTIMISATION and nothing depends on it — the GPU zone recorder's fallback is a
        // recorded `vkCmdResetQueryPool` at the frame top, and with `GPU_RING_DEPTH = 4` against
        // `FRAMES_IN_FLIGHT = 2` there is always a clean slot, so the fallback never stalls. Host
        // reset only removes the one-frame recycle latency. Recorded in the caps rather than
        // assumed, because nothing in this tree establishes that this box's driver advertises it.
        device_caps.host_query_reset = supports_host_query_reset(&instance_fns, physical_device);

        // Profiling rung 9 (D14 tier 2): the SAME "query before request" precedent once more.
        // Requesting an unadvertised extension string is a hard `vkCreateDevice` failure, so the
        // probe runs first and its answer is what both `create_device` and the device-command
        // loader are handed — one query, one answer, no second spelling that could drift.
        device_caps.calibrated_timestamps =
            supports_calibrated_timestamps(gipa, instance, physical_device);

        // --- 6. Create the logical device + retrieve the queue. ---
        let device = match create_device(
            &instance_fns,
            physical_device,
            queue_family_index,
            config.windowed,
            DeviceEnables {
                enable_ray_query,
                enable_vb_geometry_table,
                enable_host_query_reset: device_caps.host_query_reset,
                enable_calibrated_timestamps: device_caps.calibrated_timestamps,
            },
        ) {
            Ok(d) => d,
            Err(e) => fail!(e),
        };

        let device_fns = match load_device_fns(
            instance_fns.get_device_proc_addr,
            device,
            config.windowed,
            device_caps.calibrated_timestamps,
        ) {
            Ok(f) => f,
            Err(e) => {
                // The device was created but its commands are unloadable: a
                // broken loader. We cannot destroy the device with a typed
                // pointer, so the device leaks (a spec-impossible corner), but
                // we still tear down the messenger + instance to minimize leaks.
                fail!(e)
            }
        };

        let mut queue = VkQueue::NULL;
        // SAFETY: `device` is the freshly-created logical device; `family`/0
        // name the single queue requested in `create_device`; `&mut queue` is
        // a valid out-pointer for one `VkQueue`.
        unsafe { (device_fns.get_device_queue)(device, queue_family_index, 0, &mut queue) };

        // HW-RT rung R2a-1: when `feature="hwrt"` AND the device advertised + enabled ray
        // query, resolve the AS command table + populate the RT caps (`ray_query`,
        // `as_scratch_align`). `create_device` only appends the 3 RT extensions when the
        // same `supports_ray_query` presence+feature query returned true, so the table
        // resolves; a non-RT (or `hwrt`-off) device keeps `accel_fns = None` and the R1
        // caps (`ray_query == false`, `as_scratch_align == 0`).
        #[cfg(feature = "hwrt")]
        let accel_fns: Option<crate::accel::AccelFns> = {
            if rt_caps.ray_query {
                // SAFETY: `get_device_proc_addr` is the live device's proc-addr fn; the RT
                // extensions were enabled in `create_device` (same `rt_caps` query), so the
                // commands resolve.
                let fns = unsafe {
                    crate::accel::AccelFns::load(instance_fns.get_device_proc_addr, device)
                };
                if fns.is_some() {
                    device_caps.ray_query = true;
                    device_caps.as_scratch_align = rt_caps.scratch_align;
                }
                fns
            } else {
                None
            }
        };

        // `RefCell`, not a lock: `VulkanContext` is `!Send + !Sync` and every
        // allocation path is single-threaded (plan §5.3), so this is interior
        // mutability for the `&mut` a sub-allocator needs from `&self` calls —
        // NOT hot-path synchronisation. It is also boot-time, once per device.
        // Same exception the pool FIELDS already carry; the `let`s exist only
        // because an attribute cannot sit on a struct-literal field expression.
        #[allow(clippy::disallowed_types)]
        let host_pool = RefCell::new(BlockPool::new(SHARED_HOST_BLOCK_CAPACITY));
        #[allow(clippy::disallowed_types)]
        let device_pool = RefCell::new(BlockPool::new(SHARED_DEVICE_BLOCK_CAPACITY));

        Ok(Self {
            module,
            instance,
            physical_device,
            device,
            queue,
            queue_family_index,
            memory_properties,
            device_name,
            device_caps,
            debug_messenger,
            debug_state,
            instance_fns,
            // Heap-box the fn-table so its address is stable across context moves
            // (plan A1): caches into it (host block / queue / encoder) hold a
            // `*const DeviceFns` that a move must not invalidate.
            device_fns: Box::new(device_fns),
            compute_layouts: OnceCell::new(),
            host_pool,
            device_pool,
            #[cfg(feature = "hwrt")]
            accel_fns,
            vb_geometry_table_armed: OnceCell::new(),
        })
    }

    /// The logical device handle.
    #[inline]
    pub fn device(&self) -> VkDevice {
        self.device
    }

    /// The physical device handle.
    #[inline]
    pub fn physical_device(&self) -> VkPhysicalDevice {
        self.physical_device
    }

    /// The graphics+compute queue handle.
    #[inline]
    pub fn queue(&self) -> VkQueue {
        self.queue
    }

    /// The queue family index the queue belongs to.
    #[inline]
    pub fn queue_family_index(&self) -> u32 {
        self.queue_family_index
    }

    /// The chosen device's human-readable name.
    #[inline]
    pub fn device_name(&self) -> &str {
        &self.device_name
    }

    /// The minimal physical-device capabilities queried at boot (Render P1b). A booted
    /// context always has `gbuffer_storage_format_ok == true` and (T-dev)
    /// `bindless_capable == true` — both are boot fail-fasts rejecting a GPU lacking them.
    #[inline]
    pub fn device_caps(&self) -> DeviceCaps {
        self.device_caps
    }

    /// Multi-paradigm render-path plan, rung R-VBGEO: commits the boot-resolved
    /// `ResolvedRenderPath.vb_geometry_table` flag exactly once (`boyko_app::runner`,
    /// right after `resolve_render_path`, before `app.finish()` / the `upload_mesh_assets`
    /// boot drain). A second call (there is none in the current boot sequence) is a
    /// harmless no-op — `OnceCell::set` on an already-set cell silently keeps the first
    /// value, since every caller in this codebase sets the SAME boot-resolved value.
    #[inline]
    pub fn set_vb_geometry_table_armed(&self, armed: bool) {
        let _ = self.vb_geometry_table_armed.set(armed);
    }

    /// Whether the boot-committed `ResolvedRenderPath.vb_geometry_table` is armed —
    /// `false` until [`Self::set_vb_geometry_table_armed`] runs (every mesh-registration
    /// call site reads this through the `ctx: &VulkanContext` parameter it already takes,
    /// so no new parameter threads the flag — see the field's own doc).
    #[inline]
    pub fn vb_geometry_table_armed(&self) -> bool {
        self.vb_geometry_table_armed.get().copied().unwrap_or(false)
    }

    /// The resolved device command table.
    #[inline]
    pub fn device_fns(&self) -> &DeviceFns {
        &self.device_fns
    }

    /// HW-RT rung R2a-1: the resolved acceleration-structure command table, or `None` when
    /// ray query is not enabled on this device (the AS verbs return `Unsupported`). Gated
    /// `hwrt` — absent from a default build.
    #[cfg(feature = "hwrt")]
    #[inline]
    pub fn accel_fns_opt(&self) -> Option<&crate::accel::AccelFns> {
        self.accel_fns.as_ref()
    }

    /// Whether the shared blocks must carry `VK_MEMORY_ALLOCATE_DEVICE_ADDRESS_BIT` (HW-RT
    /// R2a-2): true only under `hwrt` AND when the device enabled ray query. Always false
    /// otherwise (byte-identical — the alloc flag / `p_next` chain is never added).
    #[inline]
    pub(crate) fn rt_buffer_device_address(&self) -> bool {
        #[cfg(feature = "hwrt")]
        {
            self.device_caps().ray_query
        }
        #[cfg(not(feature = "hwrt"))]
        {
            false
        }
    }

    /// Whether hardware ray query is enabled on this device (HW-RT R2a-2). Exposed for the
    /// render layer's gated mesh buffer-usage bits (`SHADER_DEVICE_ADDRESS | ACCEL_BUILD_INPUT`
    /// on a mesh that will be a BLAS build input). Always false without `hwrt` (byte-identical).
    #[inline]
    pub fn ray_query_enabled(&self) -> bool {
        #[cfg(feature = "hwrt")]
        {
            self.device_caps().ray_query
        }
        #[cfg(not(feature = "hwrt"))]
        {
            false
        }
    }

    /// The cached physical-device memory properties.
    #[inline]
    pub fn memory_properties(&self) -> &VkPhysicalDeviceMemoryProperties {
        &self.memory_properties
    }

    /// The `VkInstance` handle (needed to create / destroy a `VkSurfaceKHR`).
    #[inline]
    pub fn instance(&self) -> VkInstance {
        self.instance
    }

    /// The `VK_KHR_surface` / `VK_KHR_win32_surface` instance command table, or
    /// `None` for a headless context ([`InstanceConfig::windowed`] was `false`).
    #[inline]
    pub fn surface_fns(&self) -> Option<&SurfaceInstanceFns> {
        self.instance_fns.surface.as_ref()
    }

    /// The `VK_KHR_swapchain` device command table, or `None` for a headless
    /// context.
    #[inline]
    pub fn swapchain_fns(&self) -> Option<&SwapchainDeviceFns> {
        self.device_fns.swapchain.as_ref()
    }

    /// The validation-message recorder, present iff validation was enabled.
    ///
    /// Tests assert `state.total() == 0` after a clean GPU run; a non-zero count
    /// means the validation layer flagged a WARNING/ERROR — the soundness oracle
    /// (plan §6). Returns `None` when [`InstanceConfig::enable_validation`] was
    /// `false`.
    #[inline]
    pub fn debug_state(&self) -> Option<&DebugMessengerState> {
        self.debug_state.as_deref()
    }

    /// Whether a validation messenger is active on this context.
    #[inline]
    pub fn validation_enabled(&self) -> bool {
        self.debug_state.is_some()
    }

    /// The shared compute descriptor-set + pipeline layouts, created on first
    /// use and cached for the device's lifetime (plan Q1/W2).
    ///
    /// One STORAGE_BUFFER @ set0/binding0 (COMPUTE) + a 4-byte COMPUTE push
    /// range — the fixed layout every Slice-0 compute pipeline + command encoder
    /// shares. The Phase-6 bind-group seam supersedes it. Returns a
    /// [`VulkanError`](crate::error::VulkanError) if layout creation fails.
    pub(crate) fn compute_layouts(&self) -> Result<&ComputeLayouts, crate::error::VulkanError> {
        // `get_or_init` cannot carry an error, so try-init only when empty and
        // surface the failure to the caller (a transient layout-create failure
        // must not be cached as a poisoned cell).
        if let Some(layouts) = self.compute_layouts.get() {
            return Ok(layouts);
        }
        let created = ComputeLayouts::new(self.device, &self.device_fns)?;
        // Race-free: `&self` here is single-threaded (RHI is `!Sync`), so the
        // cell is empty and this `set` succeeds; the `Err` arm cannot occur.
        let _ = self.compute_layouts.set(created);
        Ok(self
            .compute_layouts
            .get()
            .expect("invariant: compute_layouts was just set"))
    }

    /// Sub-allocates a host-visible+coherent buffer from the growable pool
    /// (plan Q1), appending a block if no existing one has room.
    ///
    /// ⚠️ **The pool is not exposed by reference, deliberately.** It used to be a
    /// `OnceCell` handing out `&RefCell<HostVisibleBlock>`; a pool that grows
    /// stores its blocks in a `Vec`, and a `&` into a `Vec` element is
    /// invalidated by the very push that growth performs. Allocation and freeing
    /// therefore happen behind these methods, so no reference to a block ever
    /// outlives a possible growth.
    ///
    /// Plan A1: each block caches a raw `*const DeviceFns` pointing into the
    /// boxed `device_fns` — a stable heap address. NO `'static` lifetime is
    /// fabricated; `HostVisibleBlock::new` captures the borrow as a raw pointer
    /// internally. The invariant that makes this sound: the boxed fn-table
    /// address does not move when the context moves, and every block is dropped
    /// in this context's `Drop` (via `host_pool.clear()`) BEFORE the boxed
    /// fn-table is freed and before `vkDestroyDevice`, so the pointee outlives
    /// every block use. The context is `!Send + !Sync`, so it never crosses a
    /// thread.
    pub(crate) fn alloc_host_buffer(
        &self,
        size: u64,
        usage: crate::ffi::VkFlags,
    ) -> Result<crate::memory::BoundBuffer, crate::error::VulkanError> {
        Ok(self.host_pool.borrow_mut().alloc(
            self.device(),
            self.device_fns(),
            self.memory_properties(),
            self.rt_buffer_device_address(),
            size,
            usage,
        )?)
    }

    /// Returns a host-visible sub-allocation to the block that minted it.
    ///
    /// # Safety
    ///
    /// `bound` must have come from [`Self::alloc_host_buffer`] on this context
    /// and not already been destroyed; the GPU must no longer be using it.
    pub(crate) unsafe fn free_host_buffer(&self, bound: crate::memory::BoundBuffer) {
        // SAFETY: forwarded from this function's own contract — the pool routes
        // by `bound.block`, which it stamped at allocation.
        unsafe { self.host_pool.borrow_mut().free(bound) }
    }

    /// Sub-allocates a device-local (VRAM) buffer from the growable pool
    /// (plan D3/MF-8), appending a block if no existing one has room. Blocks
    /// here are never mapped. Same reference-safety and plan-A1 contracts as
    /// [`Self::alloc_host_buffer`].
    pub(crate) fn alloc_device_buffer(
        &self,
        size: u64,
        usage: crate::ffi::VkFlags,
    ) -> Result<crate::memory::BoundBuffer, crate::error::VulkanError> {
        Ok(self.device_pool.borrow_mut().alloc(
            self.device(),
            self.device_fns(),
            self.memory_properties(),
            self.rt_buffer_device_address(),
            size,
            usage,
        )?)
    }

    /// Returns a device-local sub-allocation to the block that minted it.
    ///
    /// # Safety
    ///
    /// Identical contract to [`Self::free_host_buffer`].
    pub(crate) unsafe fn free_device_buffer(&self, bound: crate::memory::BoundBuffer) {
        // SAFETY: forwarded from this function's own contract.
        unsafe { self.device_pool.borrow_mut().free(bound) }
    }

    /// How many blocks each pool currently holds, as `(host, device)`.
    ///
    /// Exposed so the growth gate can assert that exceeding one block's capacity
    /// **adds a block** rather than failing — the property S1 exists to create.
    pub fn pool_block_counts(&self) -> (usize, usize) {
        (self.host_pool.borrow().block_count(), self.device_pool.borrow().block_count())
    }

    /// Total bytes each pool has allocated from the driver, as `(host, device)`.
    pub fn pool_total_capacities(&self) -> (u64, u64) {
        (self.host_pool.borrow().total_capacity(), self.device_pool.borrow().total_capacity())
    }
}

impl Drop for VulkanContext {
    fn drop(&mut self) {
        // SAFETY: `device`/`instance` are the exact handles created in `boot`,
        // each destroyed exactly once here in reverse creation order with its
        // matching destroyer. The debug messenger (if any) is destroyed BEFORE
        // the instance — it is an instance child — and its `debug_state` Box is
        // dropped only after this `drop` returns (the field outlives the
        // messenger). `module` is the live HMODULE freed once. No handle is
        // used after its destroyer runs.
        //
        // Every host-visible block is torn down FIRST: each block's own `Drop`
        // calls `vkUnmapMemory` + `vkFreeMemory` through the raw
        // `*const DeviceFns` it cached, which targets the still-live boxed
        // `device_fns` (the box is a field of `self`, dropped implicitly AFTER this
        // `drop` body runs — plan A1), and they must precede `vkDestroyDevice`. Any
        // buffers sub-allocated from them were already destroyed via
        // `RhiDevice::destroy_buffer` / the registry's `destroy_all` before the
        // context dropped. `clear` drops the whole `Vec` of blocks, so growth does
        // not change what this must reach — it changes how many.
        self.host_pool.borrow_mut().clear();
        // Every device-local block is torn down next, also BEFORE
        // `vkDestroyDevice`. Their `Drop` calls only `vkFreeMemory` (they are
        // never mapped) through the same plan-A1 raw `*const DeviceFns` into the
        // still-live boxed `device_fns`. Any device-local buffers sub-allocated
        // from them were already destroyed via `RhiDevice::destroy_buffer` / the
        // registry's `destroy_all` before the context dropped.
        self.device_pool.borrow_mut().clear();
        // The shared compute layouts (if ever created) are destroyed next — they
        // are device children, so they must go before `vkDestroyDevice` (plan
        // Q1/W2). `ComputeLayouts::destroy` consumes them exactly once.
        if let Some(layouts) = self.compute_layouts.take() {
            // SAFETY: `layouts` were created on `self.device` via
            // `ComputeLayouts::new` and are destroyed here exactly once (the
            // `take` removes them from the cell). No compute pipeline or command
            // encoder referencing them is still in flight: the registry's
            // `destroy_all` (and the encoder/pipeline `destroy_*`) run before the
            // context is dropped, and `vkDestroyDevice` below would otherwise
            // wait-idle the device anyway.
            unsafe { layouts.destroy(self.device, &self.device_fns) };
        }
        unsafe {
            (self.device_fns.destroy_device)(self.device, ptr::null());
            if !self.debug_messenger.is_null()
                && let Some(destroy) = self.instance_fns.destroy_debug_messenger
            {
                destroy(self.instance, self.debug_messenger, ptr::null());
            }
            (self.instance_fns.destroy_instance)(self.instance, ptr::null());
            free_vulkan_loader(self.module);
        }
    }
}

// ---------------------------------------------------------------------------
// OS loader helpers (Windows).
// ---------------------------------------------------------------------------

/// Loads `vulkan-1.dll`, returning its HMODULE or `None` if absent.
#[cfg(windows)]
fn load_vulkan_loader() -> Option<*mut c_void> {
    // SAFETY: `c"vulkan-1.dll"` is a static NUL-terminated ANSI string;
    // `LoadLibraryA` returns the module handle or NULL. We null-check before
    // returning, so a NULL never escapes as a live handle.
    let module = unsafe { os::LoadLibraryA(c"vulkan-1.dll".as_ptr()) };
    if module.is_null() { None } else { Some(module) }
}

/// Resolves an exported symbol from the loaded module.
///
/// # Safety
///
/// `module` must be a live HMODULE returned by [`load_vulkan_loader`]; `name`
/// must be a valid NUL-terminated symbol name.
#[cfg(windows)]
unsafe fn os_get_proc(module: *mut c_void, name: &CStr) -> PfnVkVoidFunction {
    // SAFETY: the caller guarantees `module` is live and `name` is a valid
    // NUL-terminated C string; `GetProcAddress` returns the symbol address or
    // NULL. The returned FARPROC is a function pointer; transmuting a non-null
    // one to `extern "system" fn()` matches the Win32 ABI. NULL maps to `None`.
    let raw = unsafe { os::GetProcAddress(module, name.as_ptr()) };
    if raw.is_null() {
        None
    } else {
        // SAFETY: `raw` is a non-null exported function address; reinterpreting
        // it as an opaque `extern "system" fn()` is the canonical FARPROC use.
        Some(unsafe { mem::transmute::<*mut c_void, unsafe extern "system" fn()>(raw) })
    }
}

/// Frees the loaded `vulkan-1.dll` module.
///
/// # Safety
///
/// `module` must be a live HMODULE returned by [`load_vulkan_loader`] and not
/// already freed.
#[cfg(windows)]
unsafe fn free_vulkan_loader(module: *mut c_void) {
    // SAFETY: the caller guarantees `module` is a live, not-yet-freed HMODULE
    // from `LoadLibraryA`; `FreeLibrary` releases the matching reference.
    unsafe {
        os::FreeLibrary(module);
    }
}

// Non-Windows stubs keep the crate compiling cross-platform; the Linux
// `dlopen`/`dlsym` arm is added when first targeted (Slice 0 is Windows-first).
#[cfg(not(windows))]
fn load_vulkan_loader() -> Option<*mut c_void> {
    None
}

#[cfg(not(windows))]
unsafe fn os_get_proc(_module: *mut c_void, _name: &CStr) -> PfnVkVoidFunction {
    None
}

#[cfg(not(windows))]
unsafe fn free_vulkan_loader(_module: *mut c_void) {}

// ---------------------------------------------------------------------------
// Command-table loaders.
// ---------------------------------------------------------------------------

/// Resolves a command through `vkGetInstanceProcAddr` and transmutes it to the
/// requested PFN type, or returns `MissingCommand`.
///
/// # Safety
///
/// `T` must be the exact `Pfn*` function-pointer typedef matching `name`'s
/// Vulkan ABI; `gipa` and `instance` must be valid for the requested scope
/// (NULL instance for global commands).
unsafe fn load_instance_command<T: Copy>(
    gipa: PfnVkGetInstanceProcAddr,
    instance: VkInstance,
    name: &'static CStr,
) -> Result<T, BootError> {
    debug_assert_eq!(
        mem::size_of::<T>(),
        mem::size_of::<PfnVkVoidFunction>(),
        "PFN typedef must be pointer-sized"
    );
    // SAFETY: `gipa` is the validated `vkGetInstanceProcAddr`; calling it with
    // `instance` (NULL for global commands) and a NUL-terminated name returns
    // the command address or NULL (mapped to `None`).
    let pfn = unsafe { gipa(instance, name.as_ptr()) };
    match pfn {
        Some(f) => {
            // SAFETY: the caller's `T` bound guarantees `T` is the matching
            // pointer-sized PFN typedef; transmuting the non-null function
            // pointer to it is the documented proc-addr contract (size checked
            // by the debug_assert above).
            Ok(unsafe { mem::transmute_copy::<unsafe extern "system" fn(), T>(&f) })
        }
        None => Err(BootError::MissingCommand(leak_name(name))),
    }
}

/// Resolves a command through `vkGetDeviceProcAddr`.
///
/// # Safety
///
/// Same contract as [`load_instance_command`] but in device scope.
unsafe fn load_device_command<T: Copy>(
    gdpa: PfnVkGetDeviceProcAddr,
    device: VkDevice,
    name: &'static CStr,
) -> Result<T, BootError> {
    debug_assert_eq!(
        mem::size_of::<T>(),
        mem::size_of::<PfnVkVoidFunction>(),
        "PFN typedef must be pointer-sized"
    );
    // SAFETY: `gdpa` is the validated `vkGetDeviceProcAddr`; calling it with a
    // live `device` and a NUL-terminated name returns the command or NULL.
    let pfn = unsafe { gdpa(device, name.as_ptr()) };
    match pfn {
        // SAFETY: as in `load_instance_command` — `T` is the matching PFN
        // typedef per the caller's bound; size checked above.
        Some(f) => Ok(unsafe { mem::transmute_copy::<unsafe extern "system" fn(), T>(&f) }),
        None => Err(BootError::MissingCommand(leak_name(name))),
    }
}

/// Returns a `'static` str for a known command name used in errors. The name
/// set is closed (every caller passes a `c"..."` literal), and the `&'static CStr`
/// parameter makes that staticness **type-enforced** — no lifetime laundering.
fn leak_name(name: &'static CStr) -> &'static str {
    // The names are ASCII Vulkan command identifiers → valid UTF-8; the fallback
    // covers the impossible non-UTF-8 case without panicking on an error path.
    name.to_str().unwrap_or("vk<non-utf8-command-name>")
}

fn load_global_fns(gipa: PfnVkGetInstanceProcAddr) -> Result<GlobalFns, BootError> {
    // SAFETY: global commands resolve with a NULL instance; each `T` matches
    // its command's PFN typedef.
    unsafe {
        Ok(GlobalFns {
            create_instance: load_instance_command(gipa, VkInstance::NULL, c"vkCreateInstance")?,
            enumerate_instance_layer_properties: load_instance_command(
                gipa,
                VkInstance::NULL,
                c"vkEnumerateInstanceLayerProperties",
            )?,
            enumerate_instance_extension_properties: load_instance_command(
                gipa,
                VkInstance::NULL,
                c"vkEnumerateInstanceExtensionProperties",
            )?,
        })
    }
}

fn load_instance_fns(
    gipa: PfnVkGetInstanceProcAddr,
    instance: VkInstance,
    config: InstanceConfig,
) -> Result<InstanceFns, BootError> {
    // SAFETY: instance commands resolve with the live `instance`; each `T`
    // matches its command's PFN typedef.
    unsafe {
        // The debug-utils destroyer is an extension command; it only resolves
        // when `VK_EXT_debug_utils` is enabled on the instance (which we do iff
        // validation is requested). Resolve it eagerly so the messenger can be
        // destroyed even on later error paths.
        let destroy_debug_messenger: Option<PfnVkDestroyDebugUtilsMessengerExt> =
            if config.enable_validation {
                Some(load_instance_command(
                    gipa,
                    instance,
                    c"vkDestroyDebugUtilsMessengerEXT",
                )?)
            } else {
                None
            };

        // The surface commands resolve only when the surface extensions are
        // enabled (which we do iff a windowed context is requested).
        let surface: Option<SurfaceInstanceFns> = if config.windowed {
            Some(SurfaceInstanceFns {
                create_win32_surface: load_instance_command(
                    gipa,
                    instance,
                    c"vkCreateWin32SurfaceKHR",
                )?,
                destroy_surface: load_instance_command(gipa, instance, c"vkDestroySurfaceKHR")?,
                get_surface_support: load_instance_command(
                    gipa,
                    instance,
                    c"vkGetPhysicalDeviceSurfaceSupportKHR",
                )?,
                get_surface_capabilities: load_instance_command(
                    gipa,
                    instance,
                    c"vkGetPhysicalDeviceSurfaceCapabilitiesKHR",
                )?,
                get_surface_formats: load_instance_command(
                    gipa,
                    instance,
                    c"vkGetPhysicalDeviceSurfaceFormatsKHR",
                )?,
                get_surface_present_modes: load_instance_command(
                    gipa,
                    instance,
                    c"vkGetPhysicalDeviceSurfacePresentModesKHR",
                )?,
            })
        } else {
            None
        };

        Ok(InstanceFns {
            destroy_instance: load_instance_command(gipa, instance, c"vkDestroyInstance")?,
            enumerate_physical_devices: load_instance_command(
                gipa,
                instance,
                c"vkEnumeratePhysicalDevices",
            )?,
            get_physical_device_properties: load_instance_command(
                gipa,
                instance,
                c"vkGetPhysicalDeviceProperties",
            )?,
            get_physical_device_memory_properties: load_instance_command(
                gipa,
                instance,
                c"vkGetPhysicalDeviceMemoryProperties",
            )?,
            get_physical_device_queue_family_properties: load_instance_command(
                gipa,
                instance,
                c"vkGetPhysicalDeviceQueueFamilyProperties",
            )?,
            // Vulkan 1.1 core (the `2` suffix, no `KHR`) — always present on a
            // 1.3 instance. The S0 fail-fast dynamic-rendering query (Correction #2).
            get_physical_device_features2: load_instance_command(
                gipa,
                instance,
                c"vkGetPhysicalDeviceFeatures2",
            )?,
            // Vulkan 1.0 core — always present. The Render P1b G-buffer storage-image
            // format-support query.
            get_physical_device_format_properties: load_instance_command(
                gipa,
                instance,
                c"vkGetPhysicalDeviceFormatProperties",
            )?,
            create_device: load_instance_command(gipa, instance, c"vkCreateDevice")?,
            get_device_proc_addr: load_instance_command(gipa, instance, c"vkGetDeviceProcAddr")?,
            destroy_debug_messenger,
            surface,
        })
    }
}

/// Best-effort instance-fns table with only `vkDestroyInstance` populated, for
/// the rare path where the full table failed to load but the instance exists.
fn fallback_instance_fns(gipa: PfnVkGetInstanceProcAddr, instance: VkInstance) -> InstanceFns {
    // SAFETY: `vkDestroyInstance` is resolved (or a no-op fn substituted) so
    // the caller can still call `destroy_instance` exactly once. The remaining
    // fields are never invoked on this error path.
    let destroy_instance: PfnVkDestroyInstance = unsafe {
        load_instance_command(gipa, instance, c"vkDestroyInstance")
            .unwrap_or(noop_destroy_instance)
    };
    // The other fields are never called on the fallback path; populate them
    // with the same destroyer-shaped no-ops where the type allows, else a
    // resolved pointer is unnecessary. We only ever read `destroy_instance`.
    InstanceFns {
        destroy_instance,
        enumerate_physical_devices: noop_enumerate,
        get_physical_device_properties: noop_get_props,
        get_physical_device_memory_properties: noop_get_mem_props,
        get_physical_device_queue_family_properties: noop_get_qf_props,
        get_physical_device_features2: noop_get_features2,
        get_physical_device_format_properties: noop_get_format_props,
        create_device: noop_create_device,
        get_device_proc_addr: noop_get_device_proc_addr,
        // No messenger is ever created on the fallback path.
        destroy_debug_messenger: None,
        // No surface table on the fallback path.
        surface: None,
    }
}

// ---------------------------------------------------------------------------
// No-op command stubs for the unreachable fallback table (never invoked).
// ---------------------------------------------------------------------------

unsafe extern "system" fn noop_destroy_instance(_: VkInstance, _: *const c_void) {}
unsafe extern "system" fn noop_enumerate(_: VkInstance, _: *mut u32, _: *mut VkPhysicalDevice) -> i32 {
    VkResult::ERROR_INITIALIZATION_FAILED.as_raw()
}
unsafe extern "system" fn noop_get_props(_: VkPhysicalDevice, _: *mut VkPhysicalDeviceProperties) {}
unsafe extern "system" fn noop_get_mem_props(
    _: VkPhysicalDevice,
    _: *mut VkPhysicalDeviceMemoryProperties,
) {
}
unsafe extern "system" fn noop_get_qf_props(
    _: VkPhysicalDevice,
    _: *mut u32,
    _: *mut VkQueueFamilyProperties,
) {
}
unsafe extern "system" fn noop_get_features2(
    _: VkPhysicalDevice,
    _: *mut VkPhysicalDeviceFeatures2,
) {
}
unsafe extern "system" fn noop_get_format_props(
    _: VkPhysicalDevice,
    _: i32,
    _: *mut VkFormatProperties,
) {
}
unsafe extern "system" fn noop_create_device(
    _: VkPhysicalDevice,
    _: *const VkDeviceCreateInfo,
    _: *const c_void,
    _: *mut VkDevice,
) -> i32 {
    VkResult::ERROR_INITIALIZATION_FAILED.as_raw()
}
unsafe extern "system" fn noop_get_device_proc_addr(
    _: VkDevice,
    _: *const c_char,
) -> PfnVkVoidFunction {
    None
}

fn load_device_fns(
    gdpa: PfnVkGetDeviceProcAddr,
    device: VkDevice,
    windowed: bool,
    calibrated_timestamps: bool,
) -> Result<DeviceFns, BootError> {
    // SAFETY: device commands resolve with the live `device`; each `T` matches
    // its command's PFN typedef.
    unsafe {
        // `VK_KHR_swapchain` device commands resolve only with the extension
        // enabled (which we do iff windowed). The core dynamic-rendering /
        // image-view / semaphore / reset-fences commands are Vulkan 1.0 / 1.3
        // core and always present, so they load unconditionally below.
        let swapchain: Option<SwapchainDeviceFns> = if windowed {
            Some(SwapchainDeviceFns {
                create_swapchain: load_device_command(gdpa, device, c"vkCreateSwapchainKHR")?,
                destroy_swapchain: load_device_command(gdpa, device, c"vkDestroySwapchainKHR")?,
                get_swapchain_images: load_device_command(
                    gdpa,
                    device,
                    c"vkGetSwapchainImagesKHR",
                )?,
                acquire_next_image: load_device_command(gdpa, device, c"vkAcquireNextImageKHR")?,
                queue_present: load_device_command(gdpa, device, c"vkQueuePresentKHR")?,
            })
        } else {
            None
        };

        Ok(DeviceFns {
            destroy_device: load_device_command(gdpa, device, c"vkDestroyDevice")?,
            get_device_queue: load_device_command(gdpa, device, c"vkGetDeviceQueue")?,
            create_buffer: load_device_command(gdpa, device, c"vkCreateBuffer")?,
            destroy_buffer: load_device_command(gdpa, device, c"vkDestroyBuffer")?,
            get_buffer_memory_requirements: load_device_command(
                gdpa,
                device,
                c"vkGetBufferMemoryRequirements",
            )?,
            allocate_memory: load_device_command(gdpa, device, c"vkAllocateMemory")?,
            free_memory: load_device_command(gdpa, device, c"vkFreeMemory")?,
            bind_buffer_memory: load_device_command(gdpa, device, c"vkBindBufferMemory")?,
            map_memory: load_device_command(gdpa, device, c"vkMapMemory")?,
            unmap_memory: load_device_command(gdpa, device, c"vkUnmapMemory")?,
            // --- 0c/0d compute commands. ---
            create_shader_module: load_device_command(gdpa, device, c"vkCreateShaderModule")?,
            destroy_shader_module: load_device_command(gdpa, device, c"vkDestroyShaderModule")?,
            create_descriptor_set_layout: load_device_command(
                gdpa,
                device,
                c"vkCreateDescriptorSetLayout",
            )?,
            destroy_descriptor_set_layout: load_device_command(
                gdpa,
                device,
                c"vkDestroyDescriptorSetLayout",
            )?,
            create_pipeline_layout: load_device_command(gdpa, device, c"vkCreatePipelineLayout")?,
            destroy_pipeline_layout: load_device_command(
                gdpa,
                device,
                c"vkDestroyPipelineLayout",
            )?,
            create_compute_pipelines: load_device_command(
                gdpa,
                device,
                c"vkCreateComputePipelines",
            )?,
            destroy_pipeline: load_device_command(gdpa, device, c"vkDestroyPipeline")?,
            create_descriptor_pool: load_device_command(gdpa, device, c"vkCreateDescriptorPool")?,
            destroy_descriptor_pool: load_device_command(
                gdpa,
                device,
                c"vkDestroyDescriptorPool",
            )?,
            allocate_descriptor_sets: load_device_command(
                gdpa,
                device,
                c"vkAllocateDescriptorSets",
            )?,
            update_descriptor_sets: load_device_command(gdpa, device, c"vkUpdateDescriptorSets")?,
            create_command_pool: load_device_command(gdpa, device, c"vkCreateCommandPool")?,
            destroy_command_pool: load_device_command(gdpa, device, c"vkDestroyCommandPool")?,
            allocate_command_buffers: load_device_command(
                gdpa,
                device,
                c"vkAllocateCommandBuffers",
            )?,
            free_command_buffers: load_device_command(gdpa, device, c"vkFreeCommandBuffers")?,
            begin_command_buffer: load_device_command(gdpa, device, c"vkBeginCommandBuffer")?,
            end_command_buffer: load_device_command(gdpa, device, c"vkEndCommandBuffer")?,
            cmd_bind_pipeline: load_device_command(gdpa, device, c"vkCmdBindPipeline")?,
            cmd_bind_descriptor_sets: load_device_command(
                gdpa,
                device,
                c"vkCmdBindDescriptorSets",
            )?,
            cmd_push_constants: load_device_command(gdpa, device, c"vkCmdPushConstants")?,
            cmd_dispatch: load_device_command(gdpa, device, c"vkCmdDispatch")?,
            cmd_dispatch_indirect: load_device_command(gdpa, device, c"vkCmdDispatchIndirect")?,
            cmd_pipeline_barrier: load_device_command(gdpa, device, c"vkCmdPipelineBarrier")?,
            cmd_copy_buffer: load_device_command(gdpa, device, c"vkCmdCopyBuffer")?,
            cmd_fill_buffer: load_device_command(gdpa, device, c"vkCmdFillBuffer")?,
            cmd_update_buffer: load_device_command(gdpa, device, c"vkCmdUpdateBuffer")?,
            cmd_clear_color_image: load_device_command(gdpa, device, c"vkCmdClearColorImage")?,
            create_fence: load_device_command(gdpa, device, c"vkCreateFence")?,
            destroy_fence: load_device_command(gdpa, device, c"vkDestroyFence")?,
            wait_for_fences: load_device_command(gdpa, device, c"vkWaitForFences")?,
            queue_submit: load_device_command(gdpa, device, c"vkQueueSubmit")?,
            device_wait_idle: load_device_command(gdpa, device, c"vkDeviceWaitIdle")?,
            // --- HW-RT rung R0 GPU timestamp-query commands (Vulkan 1.0 core ⇒ `?` safe). ---
            create_query_pool: load_device_command(gdpa, device, c"vkCreateQueryPool")?,
            destroy_query_pool: load_device_command(gdpa, device, c"vkDestroyQueryPool")?,
            cmd_reset_query_pool: load_device_command(gdpa, device, c"vkCmdResetQueryPool")?,
            cmd_write_timestamp: load_device_command(gdpa, device, c"vkCmdWriteTimestamp")?,
            get_query_pool_results: load_device_command(gdpa, device, c"vkGetQueryPoolResults")?,
            // Profiling rung 4. Vulkan 1.2 core on a 1.3 device ⇒ `?` is safe here for the same
            // reason it is safe for its five siblings above.
            reset_query_pool: load_device_command(gdpa, device, c"vkResetQueryPool")?,
            // Profiling rung 9. `?`-free on purpose: the caller passed the SAME probe result that
            // decided whether `create_device` appended the extension string, so an unresolvable
            // pointer here would mean the loader contradicted the driver — `.ok()` records that as
            // "no correlation" rather than failing a boot over it.
            get_calibrated_timestamps: if calibrated_timestamps {
                load_device_command(gdpa, device, c"vkGetCalibratedTimestampsEXT").ok()
            } else {
                None
            },
            // --- Slice-1 core (Vulkan 1.0 / 1.3) commands. ---
            reset_fences: load_device_command(gdpa, device, c"vkResetFences")?,
            create_image_view: load_device_command(gdpa, device, c"vkCreateImageView")?,
            destroy_image_view: load_device_command(gdpa, device, c"vkDestroyImageView")?,
            create_semaphore: load_device_command(gdpa, device, c"vkCreateSemaphore")?,
            destroy_semaphore: load_device_command(gdpa, device, c"vkDestroySemaphore")?,
            // `vkCmdBeginRendering` / `vkCmdEndRendering` are Vulkan 1.3 core
            // (no `KHR` suffix) — promoted from `VK_KHR_dynamic_rendering`.
            cmd_begin_rendering: load_device_command(gdpa, device, c"vkCmdBeginRendering")?,
            cmd_end_rendering: load_device_command(gdpa, device, c"vkCmdEndRendering")?,
            // Phase-6 S0 image + image-copy commands (Vulkan 1.0 core).
            create_image: load_device_command(gdpa, device, c"vkCreateImage")?,
            destroy_image: load_device_command(gdpa, device, c"vkDestroyImage")?,
            get_image_memory_requirements: load_device_command(
                gdpa,
                device,
                c"vkGetImageMemoryRequirements",
            )?,
            bind_image_memory: load_device_command(gdpa, device, c"vkBindImageMemory")?,
            cmd_copy_image_to_buffer: load_device_command(
                gdpa,
                device,
                c"vkCmdCopyImageToBuffer",
            )?,
            cmd_copy_buffer_to_image: load_device_command(
                gdpa,
                device,
                c"vkCmdCopyBufferToImage",
            )?,
            cmd_blit_image: load_device_command(gdpa, device, c"vkCmdBlitImage")?,
            // Phase-6 S0 rung-2 graphics-pipeline + draw commands (Vulkan 1.0 core).
            create_graphics_pipelines: load_device_command(
                gdpa,
                device,
                c"vkCreateGraphicsPipelines",
            )?,
            cmd_set_viewport: load_device_command(gdpa, device, c"vkCmdSetViewport")?,
            cmd_set_scissor: load_device_command(gdpa, device, c"vkCmdSetScissor")?,
            cmd_draw: load_device_command(gdpa, device, c"vkCmdDraw")?,
            cmd_draw_indexed: load_device_command(gdpa, device, c"vkCmdDrawIndexed")?,
            cmd_draw_indexed_indirect: load_device_command(gdpa, device, c"vkCmdDrawIndexedIndirect")?,
            // Phase-6 S0 rung-3 vertex/index buffer bind commands (Vulkan 1.0 core).
            cmd_bind_vertex_buffers: load_device_command(
                gdpa,
                device,
                c"vkCmdBindVertexBuffers",
            )?,
            cmd_bind_index_buffer: load_device_command(gdpa, device, c"vkCmdBindIndexBuffer")?,
            // Phase-6 S0 rung-5 sampler commands (Vulkan 1.0 core).
            create_sampler: load_device_command(gdpa, device, c"vkCreateSampler")?,
            destroy_sampler: load_device_command(gdpa, device, c"vkDestroySampler")?,
            swapchain,
        })
    }
}

// ---------------------------------------------------------------------------
// Instance / device creation.
// ---------------------------------------------------------------------------

/// `VK_LAYER_KHRONOS_validation`, as a static NUL-terminated name.
const VALIDATION_LAYER: &CStr = c"VK_LAYER_KHRONOS_validation";

/// `boyko-E2101`, arm 1 — the `BOYKO_DISABLE_VALIDATION` escape hatch took what the caller asked
/// for.
///
/// **Both arms of this code say one thing to a reader: this run's validation is WEAKER than the
/// caller requested, so a clean run is not a proof.** That is the condition this repository has
/// been burned by twice; every golden leg sets the hatch, and until L7 nothing said so.
///
/// `RatePolicy::Once`, honoured by this site's own latch: the answer is a property of the process,
/// not of the boot, so a host that boots several contexts needs it once.
#[cold]
#[inline(never)]
fn report_validation_withheld_by_env() {
    static FIRED: OnceSite = OnceSite::new();
    if FIRED.claim() {
        boyko_log::error!(
            boyko_log::RhiVulkan,
            E2101,
            "validation was requested but BOYKO_DISABLE_VALIDATION withheld it; no messenger is \
             created and no validation message can be produced -- a clean run proves nothing"
        );
    }
}

/// `boyko-E2101`, arm 2 — the layer is on but `VK_EXT_validation_features` is absent, so the
/// chained `VkValidationFeaturesEXT` (synchronization validation) is not recognised.
///
/// **What this cannot claim, and it is why the code is an `error!` about the INSTRUMENT rather
/// than about barriers**: the extension being present does not make the layer sensitive. This
/// crate's own `tests/compute.rs::negative_chained_barrier_hazard` documents, in the tree, that
/// sync-validation is enabled here and still does not flag a compute→compute RAW hazard. Presence
/// and sensitivity are two questions; only the first is observable from inside the engine.
#[cold]
#[inline(never)]
fn report_sync_validation_absent() {
    static FIRED: OnceSite = OnceSite::new();
    if FIRED.claim() {
        boyko_log::error!(
            boyko_log::RhiVulkan,
            E2101,
            "validation is on but VK_EXT_validation_features is absent, so synchronization \
             validation is NOT enabled; this run cannot flag a missing or wrong barrier"
        );
    }
}

fn create_instance(
    global: &GlobalFns,
    _gipa: PfnVkGetInstanceProcAddr,
    config: InstanceConfig,
) -> Result<VkInstance, BootError> {
    // When validation is requested, BOTH the layer and the `VK_EXT_debug_utils`
    // extension must be present, else the messenger (the oracle) cannot be
    // created — a missing oracle must never be invisible (plan §6). Query
    // presence up front and fail loud with `ValidationUnavailable` so the
    // caller's tests treat an SDK-less host as "skip", not "pass blind".
    if config.enable_validation {
        if !is_validation_layer_present(global)? {
            return Err(BootError::ValidationUnavailable);
        }
        if !is_debug_utils_extension_present(global)? {
            return Err(BootError::ValidationUnavailable);
        }
    }
    // A windowed context needs the WSI extensions advertised by the instance.
    // Their absence means no on-screen path on this host → skip gracefully.
    if config.windowed
        && (!is_instance_extension_present(global, VK_KHR_SURFACE_EXTENSION_NAME)?
            || !is_instance_extension_present(global, VK_KHR_WIN32_SURFACE_EXTENSION_NAME)?)
    {
        return Err(BootError::WindowingUnavailable);
    }

    let app_info = VkApplicationInfo {
        s_type: VkStructureType::ApplicationInfo,
        p_next: ptr::null(),
        p_application_name: c"boyko_rhi_vulkan slice0".as_ptr(),
        application_version: 0,
        p_engine_name: c"boyko-engine".as_ptr(),
        engine_version: 0,
        api_version: VK_API_VERSION_1_3,
    };

    // Validation layer + `VK_EXT_debug_utils` extension are enabled together,
    // only when the caller asks for it (both verified present above). The
    // extension is what makes `vkCreateDebugUtilsMessengerEXT` resolvable and
    // the messenger functional; enabling the layer alone would emit no messages
    // to our callback. When validation is on we additionally enable
    // `VK_EXT_validation_features` (plan G2) so the chained `VkValidationFeaturesEXT`
    // (sync-validation) is recognized — chaining the struct WITHOUT enabling the
    // extension is what crashed the loader/layer. A windowed context additionally
    // enables the always-present WSI extensions `VK_KHR_surface` +
    // `VK_KHR_win32_surface` (they are NOT validation-gated). The extension pointer
    // array is sized for the maximum (debug-utils + validation-features + 2
    // surface) and a running count selects the live prefix.
    let layer_ptrs: [*const c_char; 1] = [VALIDATION_LAYER.as_ptr()];
    let (layer_count, pp_layers) = if config.enable_validation {
        (1u32, layer_ptrs.as_ptr())
    } else {
        (0u32, ptr::null())
    };

    // Whether `VK_EXT_validation_features` (sync-validation, plan G2) can be
    // enabled — only if it is present on this host. Its absence downgrades to plain
    // validation rather than crashing on an unrecognized chained struct.
    let sync_validation_available =
        config.enable_validation && is_instance_extension_present(global, VK_EXT_VALIDATION_FEATURES_EXTENSION_NAME)?;
    if config.enable_validation && !sync_validation_available {
        report_sync_validation_absent();
    }

    let mut ext_ptrs: [*const c_char; 4] = [ptr::null(); 4];
    let mut ext_count: u32 = 0;
    if config.enable_validation {
        ext_ptrs[ext_count as usize] = VK_EXT_DEBUG_UTILS_EXTENSION_NAME.as_ptr();
        ext_count += 1;
    }
    if sync_validation_available {
        ext_ptrs[ext_count as usize] = VK_EXT_VALIDATION_FEATURES_EXTENSION_NAME.as_ptr();
        ext_count += 1;
    }
    if config.windowed {
        ext_ptrs[ext_count as usize] = VK_KHR_SURFACE_EXTENSION_NAME.as_ptr();
        ext_count += 1;
        ext_ptrs[ext_count as usize] = VK_KHR_WIN32_SURFACE_EXTENSION_NAME.as_ptr();
        ext_count += 1;
    }
    let pp_exts: *const *const c_char = if ext_count == 0 {
        ptr::null()
    } else {
        ext_ptrs.as_ptr()
    };

    // A create-time messenger threaded through `p_next` captures validation
    // messages emitted DURING `vkCreateInstance` / `vkDestroyInstance` — the
    // window the persistent messenger cannot cover (it does not exist before the
    // instance, and is destroyed before it). Its `p_user_data` is null because
    // the heap state Box does not exist yet at instance-creation time; the
    // callback no-ops on a null user-data pointer but still logs the message
    // text, so a create/destroy-time error is still surfaced to the log. The
    // create-info lives on this stack frame and is only read during the call.
    // Plan G2: enable SYNCHRONIZATION validation via `VkValidationFeaturesEXT`,
    // chained into the instance `p_next`. Sync-validation is what flags a missing
    // / wrong pipeline barrier (a WARNING/ERROR), so the chained-barrier golden
    // test genuinely proves the barrier's correctness — not just that it lowers
    // without crashing. The enable array + this struct are stack locals read only
    // during the `vkCreateInstance` call below. The validation-features node is the
    // head of the chain; the create-time messenger follows it (canonical order:
    // the messenger must see messages emitted while the layer initializes its
    // validation features).
    let sync_validation: [i32; 1] = [VK_VALIDATION_FEATURE_ENABLE_SYNCHRONIZATION_VALIDATION_EXT];
    let mut validation_features = VkValidationFeaturesExt {
        s_type: VkStructureType::ValidationFeaturesExt,
        p_next: ptr::null(),
        enabled_validation_feature_count: sync_validation.len() as u32,
        p_enabled_validation_features: sync_validation.as_ptr(),
        disabled_validation_feature_count: 0,
        p_disabled_validation_features: ptr::null(),
    };

    // A create-time messenger threaded through `p_next` captures validation
    // messages emitted DURING `vkCreateInstance` / `vkDestroyInstance` — the
    // window the persistent messenger cannot cover (it does not exist before the
    // instance, and is destroyed before it). Its `p_user_data` is null because
    // the heap state Box does not exist yet at instance-creation time; the
    // callback no-ops on a null user-data pointer but still logs the message
    // text, so a create/destroy-time error is still surfaced to the log. The
    // create-info lives on this stack frame and is only read during the call. It is
    // chained as the SECOND node, behind the validation-features struct.
    let ci_messenger = VkDebugUtilsMessengerCreateInfoExt {
        s_type: VkStructureType::DebugUtilsMessengerCreateInfoExt,
        p_next: ptr::null(),
        flags: 0,
        message_severity: debug::MESSENGER_SEVERITY,
        message_type: debug::MESSENGER_TYPE,
        pfn_user_callback: debug::debug_callback,
        p_user_data: ptr::null_mut(),
    };
    validation_features.p_next =
        (&ci_messenger as *const VkDebugUtilsMessengerCreateInfoExt).cast();

    // Chain head selection: with sync-validation available, the validation-features
    // node leads (its `p_next` already points at the messenger). With validation on
    // but the extension absent, fall back to just the create-time messenger
    // (original behavior). Off → no chain.
    let p_next: *const c_void = if sync_validation_available {
        (&validation_features as *const VkValidationFeaturesExt).cast()
    } else if config.enable_validation {
        (&ci_messenger as *const VkDebugUtilsMessengerCreateInfoExt).cast()
    } else {
        ptr::null()
    };

    let create_info = VkInstanceCreateInfo {
        s_type: VkStructureType::InstanceCreateInfo,
        p_next,
        flags: 0,
        p_application_info: &app_info,
        enabled_layer_count: layer_count,
        pp_enabled_layer_names: pp_layers,
        enabled_extension_count: ext_count,
        pp_enabled_extension_names: pp_exts,
    };

    let mut instance = VkInstance::NULL;
    // SAFETY: `create_info` is a fully-initialized `#[repr(C)]`
    // `VkInstanceCreateInfo` whose pointer members (`p_application_info`, the
    // optional layer/extension arrays, the optional `p_next` chain
    // [`VkValidationFeaturesEXT` → create-time messenger], including the
    // `sync_validation` enable array) all outlive the call (they are locals of this
    // frame). `&mut instance` is a valid out-pointer. NULL allocator selects the
    // default.
    let raw = unsafe { (global.create_instance)(&create_info, ptr::null(), &mut instance) };
    let result = VkResult::from_raw(raw);
    if !result.is_success() {
        return Err(BootError::VkError("vkCreateInstance", result));
    }
    Ok(instance)
}

/// Whether `VK_LAYER_KHRONOS_validation` is installed (queried via the global
/// `vkEnumerateInstanceLayerProperties`).
fn is_validation_layer_present(global: &GlobalFns) -> Result<bool, BootError> {
    let mut count: u32 = 0;
    // SAFETY: count-query call with a null array; `&mut count` is a valid
    // out-pointer.
    let raw = unsafe { (global.enumerate_instance_layer_properties)(&mut count, ptr::null_mut()) };
    let result = VkResult::from_raw(raw);
    if !result.is_success() && result != VkResult::INCOMPLETE {
        return Err(BootError::VkError(
            "vkEnumerateInstanceLayerProperties(count)",
            result,
        ));
    }
    if count == 0 {
        return Ok(false);
    }

    let mut layers = vec![
        VkLayerProperties {
            layer_name: [0; 256],
            spec_version: 0,
            implementation_version: 0,
            description: [0; 256],
        };
        count as usize
    ];
    // SAFETY: `layers` has exactly `count` slots; the array pointer is valid for
    // `count` writes of the driver-written `#[repr(C)]` `VkLayerProperties`.
    let raw =
        unsafe { (global.enumerate_instance_layer_properties)(&mut count, layers.as_mut_ptr()) };
    let result = VkResult::from_raw(raw);
    if !result.is_success() && result != VkResult::INCOMPLETE {
        return Err(BootError::VkError(
            "vkEnumerateInstanceLayerProperties(fill)",
            result,
        ));
    }
    layers.truncate(count as usize);

    Ok(layers
        .iter()
        .any(|l| cstr_array_eq(&l.layer_name, VALIDATION_LAYER)))
}

/// Whether the `VK_EXT_debug_utils` instance extension is advertised.
fn is_debug_utils_extension_present(global: &GlobalFns) -> Result<bool, BootError> {
    is_instance_extension_present(global, VK_EXT_DEBUG_UTILS_EXTENSION_NAME)
}

/// Whether the named instance extension is advertised (queried via the global
/// `vkEnumerateInstanceExtensionProperties` with a null layer).
fn is_instance_extension_present(
    global: &GlobalFns,
    want: &core::ffi::CStr,
) -> Result<bool, BootError> {
    let mut count: u32 = 0;
    // SAFETY: null `p_layer_name` queries the instance's own extensions; the
    // count-query call passes a null array; `&mut count` is a valid out-pointer.
    let raw = unsafe {
        (global.enumerate_instance_extension_properties)(ptr::null(), &mut count, ptr::null_mut())
    };
    let result = VkResult::from_raw(raw);
    if !result.is_success() && result != VkResult::INCOMPLETE {
        return Err(BootError::VkError(
            "vkEnumerateInstanceExtensionProperties(count)",
            result,
        ));
    }
    if count == 0 {
        return Ok(false);
    }

    let mut exts = vec![
        VkExtensionProperties {
            extension_name: [0; 256],
            spec_version: 0,
        };
        count as usize
    ];
    // SAFETY: `exts` has exactly `count` slots; the array pointer is valid for
    // `count` writes of the driver-written `#[repr(C)]` `VkExtensionProperties`.
    let raw = unsafe {
        (global.enumerate_instance_extension_properties)(
            ptr::null(),
            &mut count,
            exts.as_mut_ptr(),
        )
    };
    let result = VkResult::from_raw(raw);
    if !result.is_success() && result != VkResult::INCOMPLETE {
        return Err(BootError::VkError(
            "vkEnumerateInstanceExtensionProperties(fill)",
            result,
        ));
    }
    exts.truncate(count as usize);

    Ok(exts.iter().any(|e| cstr_array_eq(&e.extension_name, want)))
}

/// Compares a fixed-size NUL-terminated `c_char` name array against a `&CStr`
/// without allocating: byte-for-byte up to and including the NUL.
fn cstr_array_eq(name: &[c_char; 256], want: &CStr) -> bool {
    let want_bytes = want.to_bytes_with_nul();
    if want_bytes.len() > name.len() {
        return false;
    }
    name.iter()
        .zip(want_bytes.iter())
        .all(|(&a, &b)| a as u8 == b)
}

/// The EFFECTIVE validation flag: `config.enable_validation` AND the
/// `BOYKO_DISABLE_VALIDATION` environment variable being UNSET.
///
/// The env variable is an opt-in escape hatch: on a host whose
/// `VK_LAYER_KHRONOS_validation` DLL is incompatible with the process (the
/// windows-gnu / MinGW build crashes on the MSVC-built layer's load), requesting
/// the layer either faults `vkCreateInstance` or makes boot return
/// [`BootError::ValidationUnavailable`] — so no GPU pixel golden can run. Since
/// the render OUTPUT is independent of validation (it only catches API misuse),
/// setting `BOYKO_DISABLE_VALIDATION` lets the goldens boot without the layer.
///
/// With the variable UNSET this is exactly `config.enable_validation`
/// (`x && true`), so the default path is byte-identical to prior behavior.
#[inline]
fn validation_requested(config: &InstanceConfig) -> bool {
    config.enable_validation && std::env::var_os("BOYKO_DISABLE_VALIDATION").is_none()
}

/// Creates the `VK_EXT_debug_utils` validation messenger (Slice-0 0a oracle).
///
/// Returns `(NULL, None)` when `enable_validation` is `false`. Otherwise it
/// boxes a fresh [`DebugMessengerState`] (so the callback's `p_user_data`
/// pointer is address-stable), resolves `vkCreateDebugUtilsMessengerEXT` via
/// `gipa`, and creates a messenger wired to [`debug::debug_callback`] that
/// records WARNING/ERROR messages into the boxed state. The caller stores the
/// returned messenger + Box on the context and destroys the messenger BEFORE
/// the instance in `Drop` (and drops the Box only after).
fn create_debug_messenger(
    gipa: PfnVkGetInstanceProcAddr,
    instance: VkInstance,
    enable_validation: bool,
) -> Result<(VkDebugUtilsMessengerEXT, Option<Box<DebugMessengerState>>), BootError> {
    if !enable_validation {
        return Ok((VkDebugUtilsMessengerEXT::NULL, None));
    }

    // SAFETY: the create command is an instance-scope extension command,
    // resolvable because `VK_EXT_debug_utils` is enabled on this instance (we
    // verified its presence and enabled it in `create_instance`); `T` is the
    // matching PFN typedef.
    let create: PfnVkCreateDebugUtilsMessengerExt =
        unsafe { load_instance_command(gipa, instance, c"vkCreateDebugUtilsMessengerEXT")? };

    // Box the state FIRST so its heap address is fixed before the messenger
    // captures a raw pointer into it.
    let state = Box::new(DebugMessengerState::new());
    let p_user_data = (&*state as *const DebugMessengerState) as *mut c_void;

    let create_info = VkDebugUtilsMessengerCreateInfoExt {
        s_type: VkStructureType::DebugUtilsMessengerCreateInfoExt,
        p_next: ptr::null(),
        flags: 0,
        message_severity: debug::MESSENGER_SEVERITY,
        message_type: debug::MESSENGER_TYPE,
        pfn_user_callback: debug::debug_callback,
        p_user_data,
    };

    let mut messenger = VkDebugUtilsMessengerEXT::NULL;
    // SAFETY: `instance` is live with `VK_EXT_debug_utils` enabled; `create` is
    // the resolved command; `create_info` is a fully-initialized `#[repr(C)]`
    // struct whose `p_user_data` points to the live boxed state (which the
    // caller keeps alive until after the messenger is destroyed); `&mut
    // messenger` is a valid out-pointer; NULL allocator selects the default.
    let raw = unsafe { create(instance, &create_info, ptr::null(), &mut messenger) };
    let result = VkResult::from_raw(raw);
    if !result.is_success() {
        // The Box drops here on the error path (no messenger references it).
        return Err(BootError::VkError("vkCreateDebugUtilsMessengerEXT", result));
    }

    Ok((messenger, Some(state)))
}

/// Picks a physical device (prefer a discrete GPU, else the first), returning
/// its handle, name and memory properties.
fn pick_physical_device(
    fns: &InstanceFns,
    instance: VkInstance,
) -> Result<(VkPhysicalDevice, String, VkPhysicalDeviceMemoryProperties), BootError> {
    let mut count: u32 = 0;
    // SAFETY: first call with a null array queries the count; `&mut count` is a
    // valid out-pointer.
    let raw = unsafe { (fns.enumerate_physical_devices)(instance, &mut count, ptr::null_mut()) };
    let result = VkResult::from_raw(raw);
    if !result.is_success() {
        return Err(BootError::VkError("vkEnumeratePhysicalDevices(count)", result));
    }
    if count == 0 {
        return Err(BootError::NoPhysicalDevice);
    }

    let mut devices = vec![VkPhysicalDevice::NULL; count as usize];
    // SAFETY: `devices` has exactly `count` slots; `count` is passed by
    // pointer (Vulkan may write back a smaller count); the array pointer is
    // valid for `count` writes.
    let raw =
        unsafe { (fns.enumerate_physical_devices)(instance, &mut count, devices.as_mut_ptr()) };
    let result = VkResult::from_raw(raw);
    if !result.is_success() && result != VkResult::INCOMPLETE {
        return Err(BootError::VkError("vkEnumeratePhysicalDevices(fill)", result));
    }
    devices.truncate(count as usize);
    if devices.is_empty() {
        return Err(BootError::NoPhysicalDevice);
    }

    // Prefer the first discrete GPU; fall back to the first device.
    let mut chosen = devices[0];
    let mut chosen_is_discrete = false;
    for &dev in &devices {
        let props = query_device_properties(fns, dev);
        if props.device_type == VK_PHYSICAL_DEVICE_TYPE_DISCRETE_GPU {
            chosen = dev;
            chosen_is_discrete = true;
            break;
        }
    }
    if !chosen_is_discrete {
        chosen = devices[0];
    }

    let props = query_device_properties(fns, chosen);
    let name = device_name_from_props(&props);

    // SDFDDGI I0: validate the deferred resolve set's ACTUAL declared per-type descriptor counts
    // against the chosen device's `maxPerStageDescriptor*` limits (the I(-1) deferral — done
    // CORRECTLY per-type, NOT the aggregate-cap-vs-per-type-limit bug). A device below the real need
    // is external input, so a violation returns a `BootError` (mapped to `RhiError::BackendError` in
    // `From<VulkanError> for RhiError`) through this device-selection path — NOT a release `assert!`.
    check_resolve_descriptor_limits(&props.limits)?;

    let mut mem_props: VkPhysicalDeviceMemoryProperties = unsafe { mem::zeroed() };
    // SAFETY: `chosen` is a valid physical device enumerated above; `&mut
    // mem_props` is a valid out-pointer for the `#[repr(C)]`
    // `VkPhysicalDeviceMemoryProperties` the driver fully overwrites. (Zeroed
    // init is a valid bit pattern for the all-integer/array struct.)
    unsafe { (fns.get_physical_device_memory_properties)(chosen, &mut mem_props) };

    Ok((chosen, name, mem_props))
}

// ── SDFDDGI I0: the deferred resolve set's ACTUAL per-type descriptor need. ──
//
// The exact per-kind counts the resolve bind-group layout declares (19 bindings total — see
// `boyko_app::gpu_scene`'s `resolve_entries` / `present::targets`'s resolve set), one row per kind,
// stated once:
//   CombinedImageSampler: @12, @14, @16, @17          → 4
//   StorageImage:         @0, @1, @2, @3, @7, @11      → 6
//   StorageBuffer:        @4, @6, @8, @9, @10          → 5
//   UniformBuffer:        @5, @13, @15, @18            → 4
// Sum = 4 + 6 + 5 + 4 = 19. (@10 is the SDF edit-list StorageBuffer, not a combined image; the four
// combined-image-samplers are the CSM @12, punctual atlas @14, and the two DDGI atlases @16/@17.)
/// The resolve set's per-stage COMBINED_IMAGE_SAMPLER need (@12 CSM, @14 atlas, @16/@17 DDGI).
const RESOLVE_NEED_COMBINED_IMAGE_SAMPLERS: u32 = 4;
/// The resolve set's per-stage STORAGE_IMAGE need (@0/@1/@2/@3 gbuffer + @7 gViewT + @11 gSsao).
const RESOLVE_NEED_STORAGE_IMAGES: u32 = 6;
/// The resolve set's per-stage STORAGE_BUFFER need (@4 material, @6 light, @8/@9 cluster, @10 edits).
const RESOLVE_NEED_STORAGE_BUFFERS: u32 = 5;
/// The resolve set's per-stage UNIFORM_BUFFER need (@5 camera, @13 CSM, @15 atlas, @18 DDGI).
const RESOLVE_NEED_UNIFORM_BUFFERS: u32 = 4;

/// Validates the deferred resolve set's per-type descriptor need against the device's per-stage
/// `maxPerStageDescriptor*` limits (SDFDDGI I0 — the I(-1) deferral, done per-type not per-aggregate).
///
/// Each combined-image-sampler consumes BOTH a `maxPerStageDescriptorSamplers` slot AND a
/// `maxPerStageDescriptorSampledImages` slot (Vulkan §Limits), so the resolve's combined-image count
/// is checked against BOTH. The storage-image / storage-buffer / uniform-buffer needs are checked
/// against their dedicated limits. This is the CORRECT per-type validation — NOT the aggregate cap
/// (19) vs a single per-type limit (the I(-1) bug this closes).
///
/// # Spec-minimum vs the targeted device class (NOT "all needs ≤ 16 / spec-min-safe")
///
/// The gate is against the ACTUAL per-type device limit, so it is correct regardless of the Vulkan
/// guaranteed minimums. Note two needs EXCEED the Vulkan §Limits guaranteed minimum of 4:
/// STORAGE_IMAGE = 6 and STORAGE_BUFFER = 5. So a hypothetical device that advertises only the
/// spec minimum is INTENTIONALLY rejected here (correctly — the resolve genuinely needs 6/5). The
/// targeted class — desktop GPUs, 2080Ti and up (every NV / AMD / Intel desktop driver) — clears
/// these by orders of magnitude, so the rejection can only fire on a device far below the engine's
/// baseline. This is NOT a regression this rung introduced: the storage-image(6) / storage-buffer(5)
/// counts are PRE-EXISTING; SDFDDGI added only the 2 combined-image-samplers @16/@17 + the 1 uniform
/// buffer @18 (combined-image 2→4, uniform 3→4 — both still well under any real limit).
///
/// A device below any per-type need returns [`BootError::ResolveDescriptorLimitExceeded`] — external
/// input, so a boot error through the device-selection path, not an invariant `assert!`.
fn check_resolve_descriptor_limits(
    limits: &VkPhysicalDeviceLimitsBlob,
) -> Result<(), BootError> {
    // (need, device-limit, field-name) — the per-type checks. A combined-image-sampler counts
    // against BOTH the sampler and the sampled-image limit, so it appears in two rows.
    let checks: [(u32, u32, &'static str); 5] = [
        (
            RESOLVE_NEED_COMBINED_IMAGE_SAMPLERS,
            limits.read_u32(LIMITS_OFF_MAX_PER_STAGE_SAMPLERS),
            "maxPerStageDescriptorSamplers",
        ),
        (
            RESOLVE_NEED_COMBINED_IMAGE_SAMPLERS,
            limits.read_u32(LIMITS_OFF_MAX_PER_STAGE_SAMPLED_IMAGES),
            "maxPerStageDescriptorSampledImages",
        ),
        (
            RESOLVE_NEED_STORAGE_IMAGES,
            limits.read_u32(LIMITS_OFF_MAX_PER_STAGE_STORAGE_IMAGES),
            "maxPerStageDescriptorStorageImages",
        ),
        (
            RESOLVE_NEED_STORAGE_BUFFERS,
            limits.read_u32(LIMITS_OFF_MAX_PER_STAGE_STORAGE_BUFFERS),
            "maxPerStageDescriptorStorageBuffers",
        ),
        (
            RESOLVE_NEED_UNIFORM_BUFFERS,
            limits.read_u32(LIMITS_OFF_MAX_PER_STAGE_UNIFORM_BUFFERS),
            "maxPerStageDescriptorUniformBuffers",
        ),
    ];
    for (need, limit, kind) in checks {
        if need > limit {
            return Err(BootError::ResolveDescriptorLimitExceeded { kind, need, limit });
        }
    }
    Ok(())
}

/// Queries `VkPhysicalDeviceProperties` for one device.
fn query_device_properties(
    fns: &InstanceFns,
    device: VkPhysicalDevice,
) -> VkPhysicalDeviceProperties {
    // SAFETY: a fully-zeroed `VkPhysicalDeviceProperties` is a valid initial bit
    // pattern (all fields are integers / byte arrays); the driver overwrites the
    // fields it owns. `&mut props` is a valid out-pointer.
    let mut props: VkPhysicalDeviceProperties = unsafe { mem::zeroed() };
    // SAFETY: `device` is a valid enumerated physical device; the out-pointer
    // is a live, correctly-sized `#[repr(C)]` struct.
    unsafe { (fns.get_physical_device_properties)(device, &mut props) };
    props
}

/// Extracts the NUL-terminated `deviceName` as an owned `String`.
fn device_name_from_props(props: &VkPhysicalDeviceProperties) -> String {
    // `device_name` is `[c_char; 256]`, NUL-terminated UTF-8. `c_char` is `i8`
    // on this target; reinterpret as bytes up to the first NUL.
    let bytes: &[u8] = unsafe {
        // SAFETY: `device_name` is 256 contiguous bytes; reinterpreting the
        // `[i8; 256]` as `[u8; 256]` is a same-size, same-align cast (both are
        // 1-byte). The slice borrows `props`, which outlives this view.
        core::slice::from_raw_parts(props.device_name.as_ptr() as *const u8, props.device_name.len())
    };
    let nul = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[..nul]).into_owned()
}

/// Finds a queue family that supports both graphics and compute, returning its index
/// and its `timestampValidBits` (HW-RT rung R0: the number of meaningful low bits in a
/// raw timestamp written on that family's queue — `0` means no timestamp support).
fn find_queue_family(
    fns: &InstanceFns,
    device: VkPhysicalDevice,
) -> Result<(u32, u32), BootError> {
    let mut count: u32 = 0;
    // SAFETY: count-query call with a null array; `&mut count` valid.
    unsafe { (fns.get_physical_device_queue_family_properties)(device, &mut count, ptr::null_mut()) };
    if count == 0 {
        return Err(BootError::NoSuitableQueueFamily);
    }

    let mut families = vec![
        VkQueueFamilyProperties {
            queue_flags: 0,
            queue_count: 0,
            timestamp_valid_bits: 0,
            min_image_transfer_granularity_width: 0,
            min_image_transfer_granularity_height: 0,
            min_image_transfer_granularity_depth: 0,
        };
        count as usize
    ];
    // SAFETY: `families` has exactly `count` slots; the array pointer is valid
    // for `count` writes of the `#[repr(C)]` `VkQueueFamilyProperties`.
    unsafe {
        (fns.get_physical_device_queue_family_properties)(
            device,
            &mut count,
            families.as_mut_ptr(),
        )
    };

    let required = VK_QUEUE_GRAPHICS_BIT | VK_QUEUE_COMPUTE_BIT;
    for (idx, fam) in families.iter().take(count as usize).enumerate() {
        if fam.queue_count > 0 && (fam.queue_flags & required) == required {
            // Return the CHOSEN family's `timestampValidBits` (HW-RT rung R0): the mask
            // width for timestamps written on this family's queue. Previously discarded.
            return Ok((idx as u32, fam.timestamp_valid_bits));
        }
    }
    Err(BootError::NoSuitableQueueFamily)
}

/// A zeroed [`VkPhysicalDeviceVulkan13Features`] except for `s_type` — the
/// shared template the support query (Correction #2) and device creation
/// (Correction #1) both build on.
fn zeroed_features13() -> VkPhysicalDeviceVulkan13Features {
    VkPhysicalDeviceVulkan13Features {
        s_type: VkStructureType::PhysicalDeviceVulkan13Features,
        p_next: ptr::null_mut(),
        robust_image_access: VK_FALSE,
        inline_uniform_block: VK_FALSE,
        descriptor_binding_inline_uniform_block_update_after_bind: VK_FALSE,
        pipeline_creation_cache_control: VK_FALSE,
        private_data: VK_FALSE,
        shader_demote_to_helper_invocation: VK_FALSE,
        shader_terminate_invocation: VK_FALSE,
        subgroup_size_control: VK_FALSE,
        compute_full_subgroups: VK_FALSE,
        synchronization2: VK_FALSE,
        texture_compression_astc_hdr: VK_FALSE,
        shader_zero_initialize_workgroup_memory: VK_FALSE,
        dynamic_rendering: VK_FALSE,
        shader_integer_dot_product: VK_FALSE,
        maintenance4: VK_FALSE,
    }
}

/// A zeroed [`VkPhysicalDeviceDescriptorIndexingFeatures`] except for `s_type` (T-dev) —
/// the shared template BOTH the `bindless_capable` query ([`query_device_caps`]) and
/// device creation ([`create_device`]) build on, mirroring [`zeroed_features13`].
fn zeroed_descriptor_indexing_features() -> VkPhysicalDeviceDescriptorIndexingFeatures {
    VkPhysicalDeviceDescriptorIndexingFeatures {
        s_type: VkStructureType::PhysicalDeviceDescriptorIndexingFeatures,
        p_next: ptr::null_mut(),
        shader_input_attachment_array_dynamic_indexing: VK_FALSE,
        shader_uniform_texel_buffer_array_dynamic_indexing: VK_FALSE,
        shader_storage_texel_buffer_array_dynamic_indexing: VK_FALSE,
        shader_uniform_buffer_array_non_uniform_indexing: VK_FALSE,
        shader_sampled_image_array_non_uniform_indexing: VK_FALSE,
        shader_storage_buffer_array_non_uniform_indexing: VK_FALSE,
        shader_storage_image_array_non_uniform_indexing: VK_FALSE,
        shader_input_attachment_array_non_uniform_indexing: VK_FALSE,
        shader_uniform_texel_buffer_array_non_uniform_indexing: VK_FALSE,
        shader_storage_texel_buffer_array_non_uniform_indexing: VK_FALSE,
        descriptor_binding_uniform_buffer_update_after_bind: VK_FALSE,
        descriptor_binding_sampled_image_update_after_bind: VK_FALSE,
        descriptor_binding_storage_image_update_after_bind: VK_FALSE,
        descriptor_binding_storage_buffer_update_after_bind: VK_FALSE,
        descriptor_binding_uniform_texel_buffer_update_after_bind: VK_FALSE,
        descriptor_binding_storage_texel_buffer_update_after_bind: VK_FALSE,
        descriptor_binding_update_unused_while_pending: VK_FALSE,
        descriptor_binding_partially_bound: VK_FALSE,
        descriptor_binding_variable_descriptor_count: VK_FALSE,
        runtime_descriptor_array: VK_FALSE,
    }
}

/// Whether the GPU supports the Vulkan 1.3 `dynamicRendering` feature
/// (Correction #2 / OQ-6 fail-fast). Queries `vkGetPhysicalDeviceFeatures2` with a
/// chained [`VkPhysicalDeviceVulkan13Features`] and reads back `dynamic_rendering`.
///
/// Both the headless and windowed device-creation paths request
/// `dynamicRendering` (Correction #1), so this check must pass on either path or
/// the first `cmd_begin_rendering` faults — a CLEAR error here beats an opaque
/// `vkCreateDevice` failure.
fn supports_dynamic_rendering(fns: &InstanceFns, physical_device: VkPhysicalDevice) -> bool {
    let mut features13 = zeroed_features13();
    let mut features2 = VkPhysicalDeviceFeatures2 {
        s_type: VkStructureType::PhysicalDeviceFeatures2,
        p_next: (&mut features13 as *mut VkPhysicalDeviceVulkan13Features).cast(),
        features: [VK_FALSE; 55],
    };
    // SAFETY: `physical_device` is a valid enumerated GPU; `features2` is a
    // fully-initialized `#[repr(C)]` struct whose `p_next` chains the live
    // `features13` local (both outlive the call). The driver writes the supported
    // feature bools through the out-pointer + the chained struct.
    unsafe { (fns.get_physical_device_features2)(physical_device, &mut features2) };
    features13.dynamic_rendering == VK_TRUE
}

/// A zeroed [`VkPhysicalDeviceHostQueryResetFeatures`] except for `s_type` (profiling rung 4) —
/// the shared template BOTH [`supports_host_query_reset`] and [`create_device`] build on, for the
/// reason [`zeroed_descriptor_indexing_features`] exists: a query and an enable that spell the
/// struct twice are two spellings that can drift.
fn zeroed_host_query_reset_features() -> VkPhysicalDeviceHostQueryResetFeatures {
    VkPhysicalDeviceHostQueryResetFeatures {
        s_type: VkStructureType::PhysicalDeviceHostQueryResetFeatures,
        p_next: ptr::null_mut(),
        host_query_reset: VK_FALSE,
    }
}

/// Whether the GPU advertises `hostQueryReset` (profiling rung 4 / D18).
///
/// Mirrors [`supports_dynamic_rendering`] exactly, except that the answer never fails a boot:
/// the caller records it and passes it to [`create_device`], which requests the bit only when
/// this returned `true` — the "query before request" precedent, because requesting an
/// unsupported feature bit is a hard `vkCreateDevice` error rather than a silent no-op.
fn supports_host_query_reset(fns: &InstanceFns, physical_device: VkPhysicalDevice) -> bool {
    let mut host_reset = zeroed_host_query_reset_features();
    let mut features2 = VkPhysicalDeviceFeatures2 {
        s_type: VkStructureType::PhysicalDeviceFeatures2,
        p_next: (&mut host_reset as *mut VkPhysicalDeviceHostQueryResetFeatures).cast(),
        features: [VK_FALSE; 55],
    };
    // SAFETY: `physical_device` is a valid enumerated GPU; `features2` is a fully-initialized
    // `#[repr(C)]` struct whose `p_next` chains the live `host_reset` local (both outlive the
    // call). The driver writes the supported feature bool through the chained struct.
    unsafe { (fns.get_physical_device_features2)(physical_device, &mut features2) };
    host_reset.host_query_reset == VK_TRUE
}

/// Whether this device can sample its GPU timestamp counter on demand from the host
/// (profiling rung 9 / D14 tier 2).
///
/// Two conditions, and BOTH are load-bearing:
///
/// 1. `VK_EXT_calibrated_timestamps` is advertised as a device extension. Unlike `hostQueryReset`
///    this one was never promoted to core, so the string must be enabled at device creation and
///    the entry point resolved — the same shape as the `hwrt` extensions, not the `pNext`-bit
///    shape.
/// 2. [`VK_TIME_DOMAIN_DEVICE_EXT`] is among the domains
///    `vkGetPhysicalDeviceCalibrateableTimeDomainsEXT` reports. **Presence of the extension does
///    not imply presence of that domain** — the extension is defined over a *set* of domains, and
///    a driver that advertised only host domains would satisfy condition 1 while making the one
///    call this engine wants (`timestampCount = 1`, `VK_TIME_DOMAIN_DEVICE_EXT`) invalid usage.
///
/// Never fails a boot: `false` leaves the profiler's `cpu_gpu_offset` at `UNCORRELATED`.
fn supports_calibrated_timestamps(
    gipa: PfnVkGetInstanceProcAddr,
    instance: VkInstance,
    physical_device: VkPhysicalDevice,
) -> bool {
    // Resolve the two instance-scope queries ad hoc, exactly as `supports_ray_query` does — the
    // standing `InstanceFns` table carries neither, and both are needed once, before the logical
    // device exists.
    //
    // SAFETY: `gipa` is the live instance's `vkGetInstanceProcAddr`.
    // `vkEnumerateDeviceExtensionProperties` is Vulkan 1.0 core and always resolves;
    // `vkGetPhysicalDeviceCalibrateableTimeDomainsEXT` is an EXTENSION command and resolves only
    // when the loader can see the extension — which is why its `None` arm is a normal answer here
    // rather than a `BootError`. Each is reinterpreted as its ABI-matched PFN typedef.
    let (enum_ext, get_domains): (
        crate::ffi::PfnVkEnumerateDeviceExtensionProperties,
        crate::ffi::PfnVkGetPhysicalDeviceCalibrateableTimeDomainsExt,
    ) = unsafe {
        let e = (gipa)(instance, c"vkEnumerateDeviceExtensionProperties".as_ptr());
        let d = (gipa)(
            instance,
            c"vkGetPhysicalDeviceCalibrateableTimeDomainsEXT".as_ptr(),
        );
        match (e, d) {
            (Some(e), Some(d)) => (
                mem::transmute::<PfnVkVoidFunction, crate::ffi::PfnVkEnumerateDeviceExtensionProperties>(
                    Some(e),
                ),
                mem::transmute::<
                    PfnVkVoidFunction,
                    crate::ffi::PfnVkGetPhysicalDeviceCalibrateableTimeDomainsExt,
                >(Some(d)),
            ),
            _ => return false,
        }
    };

    if !is_device_extension_present(
        enum_ext,
        physical_device,
        VK_EXT_CALIBRATED_TIMESTAMPS_EXTENSION_NAME,
    ) {
        return false;
    }

    // Condition 2. The two-call idiom: count, then fill. A stack array rather than a `Vec` —
    // `VkTimeDomainEXT` has exactly four values in the base extension, so eight slots cannot be
    // outgrown by a conformant driver, and the count is clamped rather than trusted.
    let mut count: u32 = 0;
    // SAFETY: a null `p_time_domains` is the spec's count query; `&mut count` is a valid
    // out-pointer for one `u32`.
    let raw = unsafe { (get_domains)(physical_device, &mut count, ptr::null_mut()) };
    let result = VkResult::from_raw(raw);
    if (!result.is_success() && result != VkResult::INCOMPLETE) || count == 0 {
        return false;
    }
    let mut domains = [0i32; 8];
    // Clamped, not asserted: a driver reporting more domains than the extension defines is not a
    // reason to fail a boot, and a truncated read still answers the only question asked — is the
    // DEVICE domain in there. `INCOMPLETE` is the driver's own word for that truncation.
    let mut fill = count.min(domains.len() as u32);
    // SAFETY: `domains` has `fill <= 8` slots and `fill` is what the driver is told it may write;
    // both pointers are valid for the call's duration.
    let raw = unsafe { (get_domains)(physical_device, &mut fill, domains.as_mut_ptr()) };
    let result = VkResult::from_raw(raw);
    if !result.is_success() && result != VkResult::INCOMPLETE {
        return false;
    }
    domains[..fill as usize].contains(&VK_TIME_DOMAIN_DEVICE_EXT)
}

/// HW-RT rung R2a-1: the ray-query capability + scratch alignment of a device.
#[cfg(feature = "hwrt")]
pub(crate) struct RtCaps {
    /// Whether the 3 RT extensions are present AND `accelerationStructure` + `rayQuery` +
    /// `bufferDeviceAddress` are all advertised (the enable precondition).
    pub ray_query: bool,
    /// `minAccelerationStructureScratchOffsetAlignment` (=128 on Ampere); `0` when absent.
    pub scratch_align: u64,
}

/// HW-RT rung R2a-1: whether hardware ray query is available on `physical_device` — the 3
/// non-core extension strings (`VK_KHR_acceleration_structure` + `VK_KHR_ray_query` +
/// `VK_KHR_deferred_host_operations`) all present AND the feature bools
/// (`accelerationStructure` / `rayQuery` / `bufferDeviceAddress`) all advertised via a
/// `vkGetPhysicalDeviceFeatures2` p_next chain (mirroring [`supports_dynamic_rendering`]).
/// Also reads `minAccelerationStructureScratchOffsetAlignment` from a
/// `vkGetPhysicalDeviceProperties2` chain. Absent ⇒ `ray_query == false` (NEVER a boot
/// fail: a non-RT GPU boots on the software path). Gated `hwrt`.
#[cfg(feature = "hwrt")]
fn supports_ray_query(
    fns: &InstanceFns,
    gipa: PfnVkGetInstanceProcAddr,
    instance: VkInstance,
    physical_device: VkPhysicalDevice,
) -> RtCaps {
    use crate::accel_ffi::{
        PfnVkEnumerateDeviceExtensionProperties, PfnVkGetPhysicalDeviceProperties2,
        ST_PHYSICAL_DEVICE_ACCELERATION_STRUCTURE_FEATURES_KHR,
        ST_PHYSICAL_DEVICE_ACCELERATION_STRUCTURE_PROPERTIES_KHR,
        ST_PHYSICAL_DEVICE_BUFFER_DEVICE_ADDRESS_FEATURES, ST_PHYSICAL_DEVICE_FEATURES_2,
        ST_PHYSICAL_DEVICE_PROPERTIES_2, ST_PHYSICAL_DEVICE_RAY_QUERY_FEATURES_KHR,
        VkPhysicalDeviceAccelerationStructureFeaturesKHR,
        VkPhysicalDeviceAccelerationStructurePropertiesKHR,
        VkPhysicalDeviceBufferDeviceAddressFeatures, VkPhysicalDeviceProperties2,
        VkPhysicalDeviceRayQueryFeaturesKHR,
    };

    let absent = RtCaps { ray_query: false, scratch_align: 0 };

    // Resolve the two instance-scope queries this path needs (the standing `InstanceFns`
    // table does not carry them). `vkGetPhysicalDeviceProperties2` is Vulkan 1.1 core;
    // `vkEnumerateDeviceExtensionProperties` is 1.0 core — both always resolvable.
    // SAFETY: `gipa` is the live instance's `vkGetInstanceProcAddr`; the two names are core
    // commands that always resolve; each is reinterpreted as its PFN typedef (ABI-matched).
    let (enum_ext, get_props2): (
        PfnVkEnumerateDeviceExtensionProperties,
        PfnVkGetPhysicalDeviceProperties2,
    ) = unsafe {
        let e = (gipa)(instance, c"vkEnumerateDeviceExtensionProperties".as_ptr());
        let p = (gipa)(instance, c"vkGetPhysicalDeviceProperties2".as_ptr());
        match (e, p) {
            (Some(e), Some(p)) => (
                mem::transmute::<PfnVkVoidFunction, PfnVkEnumerateDeviceExtensionProperties>(
                    Some(e),
                ),
                mem::transmute::<PfnVkVoidFunction, PfnVkGetPhysicalDeviceProperties2>(Some(p)),
            ),
            _ => return absent,
        }
    };

    // 1. All 3 non-core RT extension strings present?
    let have_exts = [
        VK_KHR_ACCELERATION_STRUCTURE_EXTENSION_NAME,
        VK_KHR_RAY_QUERY_EXTENSION_NAME,
        VK_KHR_DEFERRED_HOST_OPERATIONS_EXTENSION_NAME,
    ]
    .iter()
    .all(|want| is_device_extension_present(enum_ext, physical_device, want));
    if !have_exts {
        return absent;
    }

    // 2. Feature chain: bufferDeviceAddress → accelerationStructure → rayQuery.
    let mut ray_query_feat = VkPhysicalDeviceRayQueryFeaturesKHR {
        s_type: ST_PHYSICAL_DEVICE_RAY_QUERY_FEATURES_KHR,
        p_next: ptr::null_mut(),
        ray_query: VK_FALSE,
    };
    let mut accel_feat = VkPhysicalDeviceAccelerationStructureFeaturesKHR {
        s_type: ST_PHYSICAL_DEVICE_ACCELERATION_STRUCTURE_FEATURES_KHR,
        p_next: (&mut ray_query_feat as *mut VkPhysicalDeviceRayQueryFeaturesKHR).cast(),
        acceleration_structure: VK_FALSE,
        acceleration_structure_capture_replay: VK_FALSE,
        acceleration_structure_indirect_build: VK_FALSE,
        acceleration_structure_host_commands: VK_FALSE,
        descriptor_binding_acceleration_structure_update_after_bind: VK_FALSE,
    };
    let mut bda_feat = VkPhysicalDeviceBufferDeviceAddressFeatures {
        s_type: ST_PHYSICAL_DEVICE_BUFFER_DEVICE_ADDRESS_FEATURES,
        p_next: (&mut accel_feat as *mut VkPhysicalDeviceAccelerationStructureFeaturesKHR).cast(),
        buffer_device_address: VK_FALSE,
        buffer_device_address_capture_replay: VK_FALSE,
        buffer_device_address_multi_device: VK_FALSE,
    };
    let mut features2 = VkPhysicalDeviceFeatures2 {
        s_type: VkStructureType::PhysicalDeviceFeatures2,
        p_next: (&mut bda_feat as *mut VkPhysicalDeviceBufferDeviceAddressFeatures).cast(),
        features: [VK_FALSE; 55],
    };
    debug_assert_eq!(features2.s_type as i32, ST_PHYSICAL_DEVICE_FEATURES_2);
    // SAFETY: `physical_device` is valid; `features2` is fully initialized and its `p_next`
    // chains the three live RT feature locals (all outlive the call); the driver writes each
    // advertised bool through the chain.
    unsafe { (fns.get_physical_device_features2)(physical_device, &mut features2) };
    let features_ok = bda_feat.buffer_device_address == VK_TRUE
        && accel_feat.acceleration_structure == VK_TRUE
        && ray_query_feat.ray_query == VK_TRUE;
    if !features_ok {
        return absent;
    }

    // 3. Read the scratch-offset alignment from the AS properties chain.
    let mut as_props = VkPhysicalDeviceAccelerationStructurePropertiesKHR {
        s_type: ST_PHYSICAL_DEVICE_ACCELERATION_STRUCTURE_PROPERTIES_KHR,
        p_next: ptr::null_mut(),
        max_geometry_count: 0,
        max_instance_count: 0,
        max_primitive_count: 0,
        max_per_stage_descriptor_acceleration_structures: 0,
        max_per_stage_descriptor_update_after_bind_acceleration_structures: 0,
        max_descriptor_set_acceleration_structures: 0,
        max_descriptor_set_update_after_bind_acceleration_structures: 0,
        min_acceleration_structure_scratch_offset_alignment: 0,
    };
    let mut props2 = VkPhysicalDeviceProperties2 {
        s_type: ST_PHYSICAL_DEVICE_PROPERTIES_2,
        _pad: 0,
        p_next: (&mut as_props as *mut VkPhysicalDeviceAccelerationStructurePropertiesKHR).cast(),
        properties: unsafe { mem::zeroed() },
    };
    // SAFETY: `physical_device` is valid; `props2` is fully initialized (the opaque
    // `properties` block is zeroed and driver-overwritten) and its `p_next` chains the live
    // `as_props` local (outlives the call); the driver writes the AS properties through it.
    unsafe { (get_props2)(physical_device, &mut props2) };

    RtCaps {
        ray_query: true,
        scratch_align: as_props.min_acceleration_structure_scratch_offset_alignment as u64,
    }
}

/// Whether the named DEVICE extension is advertised (queried via
/// `vkEnumerateDeviceExtensionProperties` with a null layer). Alloc-light: a count query
/// then a fill.
///
/// HW-RT rung R2a-1 wrote it and was its only caller, so it was `hwrt`-gated. **Profiling rung 9
/// un-gated it**: `VK_EXT_calibrated_timestamps` is probed in every build, and a second copy of
/// the same enumerate-and-compare would be two things obliged to agree.
fn is_device_extension_present(
    enum_ext: crate::ffi::PfnVkEnumerateDeviceExtensionProperties,
    physical_device: VkPhysicalDevice,
    want: &CStr,
) -> bool {
    let mut count: u32 = 0;
    // SAFETY: null `p_layer_name` queries the device's own extensions; the count query
    // passes a null array; `&mut count` is a valid out-pointer.
    let raw = unsafe {
        (enum_ext)(physical_device, ptr::null(), &mut count, ptr::null_mut())
    };
    let result = VkResult::from_raw(raw);
    if (!result.is_success() && result != VkResult::INCOMPLETE) || count == 0 {
        return false;
    }
    let mut exts = vec![
        VkExtensionProperties {
            extension_name: [0; 256],
            spec_version: 0,
        };
        count as usize
    ];
    // SAFETY: `exts` has exactly `count` slots; the array pointer is valid for `count`
    // writes of the driver-written `#[repr(C)]` `VkExtensionProperties`.
    let raw = unsafe {
        (enum_ext)(physical_device, ptr::null(), &mut count, exts.as_mut_ptr())
    };
    let result = VkResult::from_raw(raw);
    if !result.is_success() && result != VkResult::INCOMPLETE {
        return false;
    }
    exts.truncate(count as usize);
    exts.iter().any(|e| cstr_array_eq(&e.extension_name, want))
}

// ── `boyko-W2102` — the three device-capability degradations ────────────────────────────────────
//
// **Three functions, not one with an argument, and that is the whole point of this code.** `W2102`
// is the case `logging/emission-path`'s F11 was raised for: one code covers three independent
// degradations, so a code-scoped `Once` would report whichever fired first and lose the other two
// silently -- and `Once` deliberately does not count its suppressions, so the loss would not even
// appear as a number. The latch is therefore per SITE: each reporter below owns its own `OnceSite`,
// and a device missing all three formats produces three lines.
//
// **They are no longer `#[cfg(debug_assertions)]`, which is the behaviour change.** Each of these
// three was a debug-only `eprintln!`, so the shipping build degraded a render feature to disabled
// and said nothing at all. That is the state `boyko_app/src/host.rs`'s own comment argues against
// in writing -- "Emitted UNCONDITIONALLY (not `#[cfg(debug_assertions)]`): a RELEASE-build
// degrade-to-Off must be observable" -- and this rung settles the two-doctrine conflict its way.
// The cost of doing so is one `Relaxed` load from a private line, once per boot, off the hot path.

/// `boyko-W2102`, site 1 — the SDFDDGI probe atlases have no storage-image support.
#[cold]
#[inline(never)]
fn report_ddgi_storage_unsupported(irr_ok: bool, depth_ok: bool) {
    static FIRED: OnceSite = OnceSite::new();
    if FIRED.claim() {
        boyko_log::warn!(
            boyko_log::RhiVulkan,
            W2102,
            "DDGI disabled: B10G11R11/RG16F storage unsupported (irr_ok={}, depth_ok={})",
            irr_ok,
            depth_ok
        );
    }
}

/// `boyko-W2102`, site 2 — the RT soft-shadow à-trous denoise has no `R16G16_UNORM` storage.
///
/// `rg8_ok` is carried for context only: both ping-pong rings are `R16G16_UNORM` since the
/// uniform-RG16 design, so `rg16_ok` is the sole precondition and `rg8_ok` merely says whether the
/// narrower format would have worked.
#[cfg(feature = "hwrt")]
#[cold]
#[inline(never)]
fn report_shadow_denoise_storage_unsupported(rg16_ok: bool, rg8_ok: bool) {
    static FIRED: OnceSite = OnceSite::new();
    if FIRED.claim() {
        boyko_log::warn!(
            boyko_log::RhiVulkan,
            W2102.number(),
            "shadow denoise disabled: RG16 UNORM storage unsupported (rg16_ok={}, rg8_ok={})",
            rg16_ok,
            rg8_ok
        );
    }
}

/// `boyko-W2102`, site 3 — the SSAO à-trous denoise has no `R16_UNORM` storage.
#[cold]
#[inline(never)]
fn report_ssao_denoise_storage_unsupported() {
    static FIRED: OnceSite = OnceSite::new();
    if FIRED.claim() {
        boyko_log::warn!(
            boyko_log::RhiVulkan,
            W2102,
            "SSAO a-trous denoise disabled: R16 UNORM storage unsupported"
        );
    }
}

/// Queries the minimal Render P1b [`DeviceCaps`]: whether the GPU advertises (T-dev)
/// the 5 bindless-prerequisite `VkPhysicalDeviceDescriptorIndexingFeatures` bits
/// (chained into `vkGetPhysicalDeviceFeatures2` — the SAME granular struct
/// `create_device` enables), whether `R8G8B8A8_UNORM` supports `STORAGE_IMAGE` under
/// OPTIMAL tiling (`vkGetPhysicalDeviceFormatProperties`), and (Lighting L0b / W2)
/// whether `R32_SFLOAT` supports `STORAGE_IMAGE` for the `gViewT` lane.
///
/// The caller fail-fasts on `!bindless_capable`, `!gbuffer_storage_format_ok`, and
/// `!viewt_storage_format_ok` so the bindless descriptor path / the marcher's G-buffer
/// / `gViewT` stores can never fault on an unsupported or unenabled feature.
fn query_device_caps(fns: &InstanceFns, physical_device: VkPhysicalDevice) -> DeviceCaps {
    // --- bindless_capable: read the 5 granular descriptor-indexing bits via features2.
    // Reusing `zeroed_descriptor_indexing_features()` (the same builder `create_device`
    // uses to ENABLE the struct) keeps the query and the enable chain reading/writing
    // the identical field layout.
    let mut descriptor_indexing = zeroed_descriptor_indexing_features();
    let mut features2 = VkPhysicalDeviceFeatures2 {
        s_type: VkStructureType::PhysicalDeviceFeatures2,
        p_next: (&mut descriptor_indexing as *mut VkPhysicalDeviceDescriptorIndexingFeatures)
            .cast(),
        features: [VK_FALSE; 55],
    };
    // SAFETY: `physical_device` is a valid enumerated GPU; `features2` is a
    // fully-initialized `#[repr(C)]` struct whose `p_next` chains the live
    // `descriptor_indexing` local (both outlive the call). The driver writes the
    // supported feature bools through the out-pointer + the chained struct.
    unsafe { (fns.get_physical_device_features2)(physical_device, &mut features2) };
    let bindless_capable = descriptor_indexing.shader_sampled_image_array_non_uniform_indexing
        == VK_TRUE
        && descriptor_indexing.runtime_descriptor_array == VK_TRUE
        && descriptor_indexing.descriptor_binding_partially_bound == VK_TRUE
        && descriptor_indexing.descriptor_binding_variable_descriptor_count == VK_TRUE
        && descriptor_indexing.descriptor_binding_sampled_image_update_after_bind == VK_TRUE;
    // Multi-paradigm render-path plan, Decision 0 / rung R1 (code review P1-2 fix): read from
    // the SAME `descriptor_indexing` local the 5-bit `bindless_capable` group above already
    // queried — no second `vkGetPhysicalDeviceFeatures2` call needed. BOTH bits `create_device`
    // conditionally enables under `enable_vb_geometry_table` (below) MUST be queried and ANDed
    // into this ONE cap: enabling `descriptor_binding_storage_buffer_update_after_bind` without
    // having queried it (the original bug) risks a hard `VK_ERROR_FEATURE_NOT_PRESENT`
    // `vkCreateDevice` failure — on ANY boot, Deferred included, since `enable_vb_geometry_table`
    // was the sole gate and it read only the FIRST bit.
    let storage_buffer_array_non_uniform_indexing_ok = descriptor_indexing
        .shader_storage_buffer_array_non_uniform_indexing
        == VK_TRUE
        && descriptor_indexing.descriptor_binding_storage_buffer_update_after_bind == VK_TRUE;

    // --- gbuffer_storage_format_ok: STORAGE_IMAGE on R8G8B8A8_UNORM, OPTIMAL tiling. ---
    let mut format_props = VkFormatProperties {
        linear_tiling_features: 0,
        optimal_tiling_features: 0,
        buffer_features: 0,
    };
    // SAFETY: `physical_device` is valid; `R8G8B8A8_UNORM` is a valid `VkFormat`;
    // `&mut format_props` is a valid out-pointer for the `#[repr(C)]`
    // `VkFormatProperties` the driver fully overwrites.
    unsafe {
        (fns.get_physical_device_format_properties)(
            physical_device,
            VK_FORMAT_R8G8B8A8_UNORM,
            &mut format_props,
        )
    };
    let gbuffer_storage_format_ok =
        (format_props.optimal_tiling_features & VK_FORMAT_FEATURE_STORAGE_IMAGE_BIT) != 0;

    // --- viewt_storage_format_ok (W2): STORAGE_IMAGE on R32_SFLOAT, OPTIMAL tiling. ---
    // The Lighting L0b `gViewT` lane stores the marcher's surface ray param `t` as a
    // compute store; mirror the `gbuffer_storage_format_ok` check exactly for the new
    // format so the caller can fail-fast before the `gViewT` image is created.
    let mut viewt_props = VkFormatProperties {
        linear_tiling_features: 0,
        optimal_tiling_features: 0,
        buffer_features: 0,
    };
    // SAFETY: `physical_device` is valid; `R32_SFLOAT` is a valid `VkFormat`;
    // `&mut viewt_props` is a valid out-pointer for the `#[repr(C)]`
    // `VkFormatProperties` the driver fully overwrites.
    unsafe {
        (fns.get_physical_device_format_properties)(
            physical_device,
            VK_FORMAT_R32_SFLOAT,
            &mut viewt_props,
        )
    };
    let viewt_storage_format_ok =
        (viewt_props.optimal_tiling_features & VK_FORMAT_FEATURE_STORAGE_IMAGE_BIT) != 0;

    // --- gbuffer_color_attachment_format_ok (P5-r0): COLOR_ATTACHMENT on R8G8B8A8_UNORM,
    // OPTIMAL tiling. The mesh raster pass A writes the albedo/normal/material G-buffer
    // images as MRT color attachments (alongside their STORAGE usage); mirror the
    // `gbuffer_storage_format_ok` check exactly for the new feature bit so the caller can
    // fail-fast before pass A binds them. (R8G8B8A8_UNORM color-attachment renderability is
    // mandatory in Vulkan, so this passes universally — the gate is the fail-fast discipline.)
    let mut gbuffer_color_props = VkFormatProperties {
        linear_tiling_features: 0,
        optimal_tiling_features: 0,
        buffer_features: 0,
    };
    // SAFETY: `physical_device` is valid; `R8G8B8A8_UNORM` is a valid `VkFormat`;
    // `&mut gbuffer_color_props` is a valid out-pointer for the `#[repr(C)]`
    // `VkFormatProperties` the driver fully overwrites.
    unsafe {
        (fns.get_physical_device_format_properties)(
            physical_device,
            VK_FORMAT_R8G8B8A8_UNORM,
            &mut gbuffer_color_props,
        )
    };
    let gbuffer_color_attachment_format_ok =
        (gbuffer_color_props.optimal_tiling_features & VK_FORMAT_FEATURE_COLOR_ATTACHMENT_BIT) != 0;

    // --- r8_unorm_storage_ok (Render P7): STORAGE_IMAGE on R8_UNORM, OPTIMAL tiling. The
    // SSAO term `gSsao` is a full-res R8_UNORM STORAGE image (resolve load + SSAO-pass store);
    // mirror the `gbuffer_storage_format_ok` check exactly for the new format so the caller can
    // fail-fast before the SSAO image is created. (R8_UNORM storage-image support is broadly
    // available, so this passes universally — the gate is the fail-fast discipline.)
    let mut ssao_props = VkFormatProperties {
        linear_tiling_features: 0,
        optimal_tiling_features: 0,
        buffer_features: 0,
    };
    // SAFETY: `physical_device` is valid; `R8_UNORM` is a valid `VkFormat`;
    // `&mut ssao_props` is a valid out-pointer for the `#[repr(C)]` `VkFormatProperties`
    // the driver fully overwrites.
    unsafe {
        (fns.get_physical_device_format_properties)(
            physical_device,
            VK_FORMAT_R8_UNORM,
            &mut ssao_props,
        )
    };
    let r8_unorm_storage_ok =
        (ssao_props.optimal_tiling_features & VK_FORMAT_FEATURE_STORAGE_IMAGE_BIT) != 0;

    // --- atlas_linear_filter_ok (M2): SAMPLED_IMAGE_FILTER_LINEAR on R8_SNORM, OPTIMAL
    // tiling. The SDF brick atlas is a SAMPLED `R8_SNORM` 3D image the marcher fetches with
    // a hardware trilinear filter; mirror the storage-format checks for the new feature bit.
    // RECORDED ONLY (no boot fail-fast): on a GPU that lacks it the atlas falls back to
    // `R16_SFLOAT` (always linear-filterable), so the engine boots on either path.
    let mut atlas_props = VkFormatProperties {
        linear_tiling_features: 0,
        optimal_tiling_features: 0,
        buffer_features: 0,
    };
    // SAFETY: `physical_device` is valid; `R8_SNORM` is a valid `VkFormat`;
    // `&mut atlas_props` is a valid out-pointer for the `#[repr(C)]` `VkFormatProperties`
    // the driver fully overwrites.
    unsafe {
        (fns.get_physical_device_format_properties)(
            physical_device,
            VK_FORMAT_R8_SNORM,
            &mut atlas_props,
        )
    };
    let atlas_linear_filter_ok = (atlas_props.optimal_tiling_features
        & VK_FORMAT_FEATURE_SAMPLED_IMAGE_FILTER_LINEAR_BIT)
        != 0;

    // --- ddgi_irr_storage_ok (SDFDDGI I2): STORAGE_IMAGE on B10G11R11_UFLOAT_PACK32, OPTIMAL
    // tiling. The probe-update pass writes the irradiance atlas via a storage image. Mirror the
    // `viewt_storage_format_ok` QUERY shape, but the caller does NOT fail-fast on `false`: DDGI
    // is opt-in, so an unsupported device degrades DDGI to permanently-disabled (plan §3), never
    // a boot failure. B10G11R11 storage is device-OPTIONAL (`shaderStorageImageExtendedFormats`).
    let mut ddgi_irr_props = VkFormatProperties {
        linear_tiling_features: 0,
        optimal_tiling_features: 0,
        buffer_features: 0,
    };
    // SAFETY: `physical_device` is valid; `B10G11R11_UFLOAT_PACK32` is a valid `VkFormat`;
    // `&mut ddgi_irr_props` is a valid out-pointer for the `#[repr(C)]` `VkFormatProperties`
    // the driver fully overwrites.
    unsafe {
        (fns.get_physical_device_format_properties)(
            physical_device,
            VK_FORMAT_B10G11R11_UFLOAT_PACK32,
            &mut ddgi_irr_props,
        )
    };
    let ddgi_irr_storage_ok =
        (ddgi_irr_props.optimal_tiling_features & VK_FORMAT_FEATURE_STORAGE_IMAGE_BIT) != 0;

    // --- ddgi_depth_storage_ok (SDFDDGI I2): STORAGE_IMAGE on R16G16_SFLOAT, OPTIMAL tiling.
    // The probe-update pass writes the two-moment depth atlas via a storage image. Same
    // degrade-not-crash policy as `ddgi_irr_storage_ok` (gated together via `ddgi_storage_ok`).
    let mut ddgi_depth_props = VkFormatProperties {
        linear_tiling_features: 0,
        optimal_tiling_features: 0,
        buffer_features: 0,
    };
    // SAFETY: `physical_device` is valid; `R16G16_SFLOAT` is a valid `VkFormat`;
    // `&mut ddgi_depth_props` is a valid out-pointer for the `#[repr(C)]` `VkFormatProperties`
    // the driver fully overwrites.
    unsafe {
        (fns.get_physical_device_format_properties)(
            physical_device,
            VK_FORMAT_R16G16_SFLOAT,
            &mut ddgi_depth_props,
        )
    };
    let ddgi_depth_storage_ok =
        (ddgi_depth_props.optimal_tiling_features & VK_FORMAT_FEATURE_STORAGE_IMAGE_BIT) != 0;

    // `boyko-W2102` when a supported feature is missing (NO boot fail-fast — DDGI is opt-in; the
    // resolve clamp + the no-storage atlas fallback handle it, plan §3). L7b: this was
    // `#[cfg(debug_assertions)]`, so a shipping build turned DDGI off in silence.
    if !(ddgi_irr_storage_ok && ddgi_depth_storage_ok) {
        report_ddgi_storage_unsupported(ddgi_irr_storage_ok, ddgi_depth_storage_ok);
    }

    // --- rg8_unorm_storage_ok: STORAGE_IMAGE on R8G8_UNORM, OPTIMAL tiling. UNCONDITIONAL as of
    // the SV0 dedicated pass: the `sdf_term` ring is an RG8 STORAGE target on every VB boot, so
    // this probe now gates a SHIPPING feature (SV0 arming + the ring's STORAGE usage bit), not
    // just the hwrt denoise ladder that first added it. Same degrade-not-panic contract: an
    // unsupported device creates the ring SAMPLED-only and SV0 resolves unarmable.
    let rg8_unorm_storage_ok = {
        let mut rg8_props = VkFormatProperties {
            linear_tiling_features: 0,
            optimal_tiling_features: 0,
            buffer_features: 0,
        };
        // SAFETY: `physical_device` is valid; `R8G8_UNORM` is a valid `VkFormat`;
        // `&mut rg8_props` is a valid out-pointer for the `#[repr(C)]` `VkFormatProperties`
        // the driver fully overwrites.
        unsafe {
            (fns.get_physical_device_format_properties)(
                physical_device,
                crate::ffi::VK_FORMAT_R8G8_UNORM,
                &mut rg8_props,
            )
        };
        (rg8_props.optimal_tiling_features & VK_FORMAT_FEATURE_STORAGE_IMAGE_BIT) != 0
    };

    #[cfg(feature = "hwrt")]
    let rg16_unorm_storage_ok = {
        let rg8_ok = rg8_unorm_storage_ok;
        let mut rg16_props = VkFormatProperties {
            linear_tiling_features: 0,
            optimal_tiling_features: 0,
            buffer_features: 0,
        };
        // SAFETY: `physical_device` is valid; `R16G16_UNORM` is a valid `VkFormat`;
        // `&mut rg16_props` is a valid out-pointer for the `#[repr(C)]` `VkFormatProperties`
        // the driver fully overwrites.
        unsafe {
            (fns.get_physical_device_format_properties)(
                physical_device,
                crate::ffi::VK_FORMAT_R16G16_UNORM,
                &mut rg16_props,
            )
        };
        let rg16_ok =
            (rg16_props.optimal_tiling_features & VK_FORMAT_FEATURE_STORAGE_IMAGE_BIT) != 0;

        // `boyko-W2102` when the denoise storage format (RG16 — the sole precondition now both
        // rings are R16G16_UNORM) is missing (NO boot fail-fast — the denoise is opt-in; the
        // target-allocation + activation gate handle it). L7b: was `#[cfg(debug_assertions)]`.
        if !rg16_ok {
            report_shadow_denoise_storage_unsupported(rg16_ok, rg8_ok);
        }
        rg16_ok
    };

    // --- r16_unorm_storage_ok: STORAGE_IMAGE on R16_UNORM, OPTIMAL tiling — the SSAO à-trous
    // denoise chain's interior ping-pong ring precondition. Software (NOT `hwrt`-gated), mirrors
    // the `rg16_unorm_storage_ok` QUERY shape one channel narrower; NO boot fail-fast (the
    // denoise is opt-in — a missing feature degrades to the raw un-denoised gather).
    let r16_unorm_storage_ok = {
        let mut r16_props = VkFormatProperties {
            linear_tiling_features: 0,
            optimal_tiling_features: 0,
            buffer_features: 0,
        };
        // SAFETY: `physical_device` is valid; `R16_UNORM` is a valid `VkFormat`; `&mut
        // r16_props` is a valid out-pointer for the `#[repr(C)]` `VkFormatProperties` the
        // driver fully overwrites.
        unsafe {
            (fns.get_physical_device_format_properties)(
                physical_device,
                crate::ffi::VK_FORMAT_R16_UNORM,
                &mut r16_props,
            )
        };
        let ok = (r16_props.optimal_tiling_features & VK_FORMAT_FEATURE_STORAGE_IMAGE_BIT) != 0;
        // L7b: `boyko-W2102`, previously a `#[cfg(debug_assertions)]` `eprintln!`.
        if !ok {
            report_ssao_denoise_storage_unsupported();
        }
        ok
    };

    DeviceCaps {
        bindless_capable,
        storage_buffer_array_non_uniform_indexing_ok,
        gbuffer_storage_format_ok,
        viewt_storage_format_ok,
        gbuffer_color_attachment_format_ok,
        r8_unorm_storage_ok,
        atlas_linear_filter_ok,
        ddgi_irr_storage_ok,
        ddgi_depth_storage_ok,
        // Rung 3a: the RT soft-shadow denoise storage-format probes (recorded, not fail-fast).
        // RG8 is unconditional as of the SV0 dedicated pass (the `sdf_term` STORAGE gate).
        rg8_unorm_storage_ok,
        #[cfg(feature = "hwrt")]
        rg16_unorm_storage_ok,
        r16_unorm_storage_ok,
        // HW-RT rung R0: placeholders — the boot site overwrites these from the physical-
        // device limits (`timestampPeriod`) + the chosen queue family (`timestampValidBits`),
        // the two inputs `query_device_caps` does not itself read.
        timestamp_period: 0.0,
        timestamp_valid_bits: 0,
        // Profiling rung 4: the same placeholder discipline. `query_device_caps` runs BEFORE
        // `vkCreateDevice`, and this field's contract is "ENABLED", not "advertised" — so the
        // only honest value here is `false`, and the boot site overwrites it from
        // `supports_host_query_reset` on the line that feeds `create_device` the same answer.
        host_query_reset: false,
        // Profiling rung 9: same placeholder discipline, same reason — the contract is "ENABLED",
        // and `vkCreateDevice` has not run yet. The boot site overwrites it from
        // `supports_calibrated_timestamps` on the line that feeds `create_device` the same answer.
        calibrated_timestamps: false,
        // VB-SV0 rung S1.5: same placeholder discipline — the boot site reads it from the
        // limits blob alongside `timestampPeriod`.
        timestamp_compute_and_graphics: false,
        // HW-RT rung R1: `ray_query`/`ray_reorder` stay `false` — R1 requests NO RT
        // extension, so there is nothing to enable (the dormancy anchor; `rt_tier()`
        // then returns `Absent` for every device). The `vendor_id`/`device_id`/
        // `driver_version` are placeholder zeros the boot site overwrites with the real
        // `VkPhysicalDeviceProperties` values (`query_device_caps` reads no properties blob).
        ray_query: false,
        ray_reorder: false,
        vendor_id: 0,
        device_id: 0,
        driver_version: 0,
        // HW-RT rung R2a-1: `0` = no scratch-align requirement (ray query off). Under
        // `feature="hwrt"` the boot site overwrites this from the AS-properties query when
        // the RT extensions were enabled; otherwise it stays `0` (the R1 value).
        as_scratch_align: 0,
        // SSAA W2: placeholders — the boot site overwrites these from the physical-device
        // limits blob (`maxImageDimension2D`) + the memory properties (`memory_properties`,
        // already returned by `pick_physical_device`), the two inputs `query_device_caps`
        // does not itself read (mirrors the `timestamp_period`/`vendor_id` placeholder
        // pattern above).
        max_image_dimension_2d: 0,
        device_local_heap_bytes: 0,
        // Multi-paradigm render-path plan, rung R-VBGEO: placeholder — the boot site
        // overwrites this from the physical-device limits blob (`maxBoundDescriptorSets`),
        // the SAME input `query_device_caps` does not itself read (mirrors the
        // `max_image_dimension_2d` placeholder immediately above).
        max_bound_descriptor_sets: 0,
    }
}

/// SSAA W2: the largest `DEVICE_LOCAL` heap size (bytes) among
/// `mem_props.memory_heaps[..memory_heap_count]`. Zero heaps or no `DEVICE_LOCAL` heap
/// (never observed on a real GPU, but the array can be empty on a stub in tests) yields `0`,
/// which makes the SSAA VRAM-budget check fail closed (degrade to `Off`, never a panic).
fn max_device_local_heap_bytes(mem_props: &VkPhysicalDeviceMemoryProperties) -> u64 {
    let count = (mem_props.memory_heap_count as usize).min(VK_MAX_MEMORY_HEAPS);
    mem_props.memory_heaps[..count]
        .iter()
        .filter(|heap| heap.flags & VK_MEMORY_HEAP_DEVICE_LOCAL_BIT != 0)
        .map(|heap| heap.size)
        .max()
        .unwrap_or(0)
}

/// Creates a logical device with one queue from `queue_family_index`.
///
/// The Vulkan 1.3 `dynamicRendering` feature is ALWAYS requested through a
/// `VkPhysicalDeviceVulkan13Features` chained into `p_next` — including the
/// **headless** path (Correction #1): every S0 acceptance path records
/// `cmd_begin_rendering`, which faults without the feature enabled. Support is
/// verified up front by [`supports_dynamic_rendering`] (Correction #2). When
/// `windowed`, the `VK_KHR_swapchain` device extension is additionally enabled.
///
/// T-dev: ALSO enables core `samplerAnisotropy` (via `p_enabled_features`, the T2
/// aniso-sampler prerequisite) and the 5-bit bindless `descriptorIndexing` granular
/// struct (via `p_next`, the T4 bindless prerequisite) on BOTH the default and hwrt
/// builds — device-state only, no pipeline/shader/descriptor change.
/// The optional device capabilities [`create_device`] may request, as one named record.
///
/// A struct rather than four trailing `bool` parameters, and profiling rung 9 is when it became
/// one: four same-typed arguments in a row is precisely where a transposition hides, and this
/// campaign has already paid for exactly that shape once (a slot index passed where a zone id was
/// expected, live for two rungs because both were `u16`). Named fields make the call site say what
/// it is enabling.
///
/// **Every field carries the "query before request" contract.** Requesting an unsupported feature
/// bit or extension string is a hard `vkCreateDevice` failure, not a silent no-op, so each is
/// `true` only after the corresponding `supports_*` probe returned `true`.
struct DeviceEnables {
    /// HW-RT rung R2a-1: appends the 3 RT extension strings and chains the RT feature structs off
    /// `features13.p_next`. HARD `false` on every non-hwrt build (the caller passes
    /// `RT_ENABLE_DEFAULT`), so the RT arm below is dead and gated.
    enable_ray_query: bool,
    /// Multi-paradigm render-path plan, rung R8 (Decision 0 / R-VBGEO's documented device-create
    /// gap, now closed): enables `shaderStorageBufferArrayNonUniformIndexing` +
    /// `descriptorBindingStorageBufferUpdateAfterBind` on the granular descriptor-indexing struct
    /// — the VB geometry table's (`MeshGeometryTable`) two prerequisite bits.
    enable_vb_geometry_table: bool,
    /// Profiling rung 4 (D18): chains `VkPhysicalDeviceHostQueryResetFeatures` with the bit set.
    /// Enabling it records NO commands and changes no frame — it is a `pNext` bit, so the goldens
    /// are unaffected — and it is what makes `vkResetQueryPool` legal to call.
    enable_host_query_reset: bool,
    /// Profiling rung 9 (D14 tier 2): appends the `VK_EXT_calibrated_timestamps` extension string.
    /// It has NO feature struct — the extension is entirely a pair of entry points — so unlike
    /// [`Self::enable_host_query_reset`] this arm touches no `pNext` chain and cannot change the
    /// walk order. Enabling it records no commands and changes no frame; the goldens are
    /// unaffected.
    enable_calibrated_timestamps: bool,
}

fn create_device(
    fns: &InstanceFns,
    physical_device: VkPhysicalDevice,
    queue_family_index: u32,
    windowed: bool,
    enables: DeviceEnables,
) -> Result<VkDevice, BootError> {
    let DeviceEnables {
        enable_ray_query,
        enable_vb_geometry_table,
        enable_host_query_reset,
        enable_calibrated_timestamps,
    } = enables;
    let _ = enable_ray_query; // read only on the hwrt arm below (silences the OFF build).
    // Correction #2 (OQ-6): fail fast with a CLEAR error if the GPU does not
    // support dynamic rendering, rather than letting `vkCreateDevice` fail opaquely
    // (or, worse, succeed and fault at `cmd_begin_rendering`).
    if !supports_dynamic_rendering(fns, physical_device) {
        return Err(BootError::VkError(
            "dynamicRendering (VkPhysicalDeviceVulkan13Features) unsupported",
            VkResult::ERROR_FEATURE_NOT_PRESENT,
        ));
    }

    let priority: f32 = 1.0;
    let queue_info = VkDeviceQueueCreateInfo {
        s_type: VkStructureType::DeviceQueueCreateInfo,
        p_next: ptr::null(),
        flags: 0,
        queue_family_index,
        queue_count: 1,
        p_queue_priorities: &priority,
    };

    // Correction #1: chain `dynamicRendering` on BOTH the headless and windowed
    // paths. The feature struct lives on this stack frame and is only read during
    // the call; all feature bools except `dynamic_rendering` are zero. The
    // `VK_KHR_swapchain` extension stays windowed-only.
    let mut features13 = zeroed_features13();
    features13.dynamic_rendering = VK_TRUE;

    // The extension name pointers this device enables. The base set is the windowed-only
    // `VK_KHR_swapchain`; the hwrt arm appends the 3 RT strings when `enable_ray_query`; profiling
    // rung 9 appends `VK_EXT_calibrated_timestamps` when `enable_calibrated_timestamps`.
    // A fixed-capacity stack array (no heap) sized to the maximum (1 swapchain + 3 RT + 1
    // calibrated timestamps). The capacity is CHECKED by the const-assert below rather than by a
    // comment: there is no bounds check on the appends, so an array outgrown by a new extension is
    // an index panic at boot on exactly the machines that support the most.
    /// 1 swapchain + 3 RT + 1 calibrated timestamps. Every arm below indexes without a bounds
    /// check, so this sum is the load-bearing part: an array outgrown by a new extension is an
    /// index panic at boot on exactly the machines that support the most.
    const MAX_DEVICE_EXTENSIONS: usize = 1 + 3 + 1;
    let mut ext_ptrs: [*const c_char; MAX_DEVICE_EXTENSIONS] =
        [ptr::null(); MAX_DEVICE_EXTENSIONS];
    let mut ext_count: usize = 0;
    if windowed {
        ext_ptrs[ext_count] = VK_KHR_SWAPCHAIN_EXTENSION_NAME.as_ptr();
        ext_count += 1;
    }

    // Profiling rung 9. Appended BEFORE the hwrt arm so the `hwrt`-off and `hwrt`-on builds put
    // this string at the same index — the array is order-insensitive to Vulkan, but a stable
    // index is what lets a boot dump be compared across the two builds.
    if enable_calibrated_timestamps {
        ext_ptrs[ext_count] = VK_EXT_CALIBRATED_TIMESTAMPS_EXTENSION_NAME.as_ptr();
        ext_count += 1;
    }

    // HW-RT rung R2a-1: enable the 3 RT extensions + chain the RT feature structs off
    // `features13.p_next`. Only ever reached when `enable_ray_query` (⇒
    // `feature="hwrt"` AND `supports_ray_query`). The feature locals live on this frame
    // + are read only during the call. Mutating `features13.p_next` HERE — before
    // `descriptor_indexing` (below) takes a pointer to `features13` — means every write
    // to `features13` is complete before any other struct's `p_next` observes its
    // address, so the chain is built tail-first with no read-after-mutate hazard.
    #[cfg(feature = "hwrt")]
    let (_rt_ray_query, _rt_accel, _rt_bda);
    #[cfg(feature = "hwrt")]
    if enable_ray_query {
        use crate::accel_ffi::{
            ST_PHYSICAL_DEVICE_ACCELERATION_STRUCTURE_FEATURES_KHR,
            ST_PHYSICAL_DEVICE_BUFFER_DEVICE_ADDRESS_FEATURES,
            ST_PHYSICAL_DEVICE_RAY_QUERY_FEATURES_KHR,
            VkPhysicalDeviceAccelerationStructureFeaturesKHR,
            VkPhysicalDeviceBufferDeviceAddressFeatures, VkPhysicalDeviceRayQueryFeaturesKHR,
        };
        ext_ptrs[ext_count] = VK_KHR_ACCELERATION_STRUCTURE_EXTENSION_NAME.as_ptr();
        ext_count += 1;
        ext_ptrs[ext_count] = VK_KHR_RAY_QUERY_EXTENSION_NAME.as_ptr();
        ext_count += 1;
        ext_ptrs[ext_count] = VK_KHR_DEFERRED_HOST_OPERATIONS_EXTENSION_NAME.as_ptr();
        ext_count += 1;
        // Build tail-first: bda (tail, p_next null) → accel → rayQuery, then hook the
        // head onto `features13.p_next`. Final walk order: descriptorIndexing →
        // features13 → rayQuery → accel → bda (each ENABLE bit TRUE). The feature-struct
        // `p_next` fields are `*mut c_void`; every struct in the chain is input-only
        // during `vkCreateDevice` (never written back through it), so the
        // `*const → *mut` casts are sound.
        _rt_bda = VkPhysicalDeviceBufferDeviceAddressFeatures {
            s_type: ST_PHYSICAL_DEVICE_BUFFER_DEVICE_ADDRESS_FEATURES,
            p_next: ptr::null_mut(),
            buffer_device_address: VK_TRUE,
            buffer_device_address_capture_replay: VK_FALSE,
            buffer_device_address_multi_device: VK_FALSE,
        };
        _rt_accel = VkPhysicalDeviceAccelerationStructureFeaturesKHR {
            s_type: ST_PHYSICAL_DEVICE_ACCELERATION_STRUCTURE_FEATURES_KHR,
            p_next: (&_rt_bda as *const VkPhysicalDeviceBufferDeviceAddressFeatures)
                .cast::<c_void>() as *mut c_void,
            acceleration_structure: VK_TRUE,
            acceleration_structure_capture_replay: VK_FALSE,
            acceleration_structure_indirect_build: VK_FALSE,
            acceleration_structure_host_commands: VK_FALSE,
            descriptor_binding_acceleration_structure_update_after_bind: VK_FALSE,
        };
        _rt_ray_query = VkPhysicalDeviceRayQueryFeaturesKHR {
            s_type: ST_PHYSICAL_DEVICE_RAY_QUERY_FEATURES_KHR,
            p_next: (&_rt_accel as *const VkPhysicalDeviceAccelerationStructureFeaturesKHR)
                .cast::<c_void>() as *mut c_void,
            ray_query: VK_TRUE,
        };
        features13.p_next = (&_rt_ray_query as *const VkPhysicalDeviceRayQueryFeaturesKHR)
            .cast::<c_void>() as *mut c_void;
    }

    // T-dev: the granular bindless feature struct, present on BOTH the default and hwrt
    // builds (bindless is device-agnostic, unlike the RT structs above). Enables exactly
    // the 5 bits `DeviceCaps::bindless_capable` gates (mirrors
    // `zeroed_descriptor_indexing_features` — the same builder `query_device_caps` uses
    // to READ these bits, so the query and the enable chain never drift). Deliberately
    // carries NO `buffer_device_address` field, so it coexists cleanly with the hwrt
    // arm's standalone `VkPhysicalDeviceBufferDeviceAddressFeatures` above (no
    // VUID-VkDeviceCreateInfo-pNext-02830 collision). Built LAST so its `p_next` observes
    // `features13` fully finalized (including the hwrt arm's mutation above).
    let mut descriptor_indexing = zeroed_descriptor_indexing_features();
    descriptor_indexing.shader_sampled_image_array_non_uniform_indexing = VK_TRUE;
    descriptor_indexing.runtime_descriptor_array = VK_TRUE;
    descriptor_indexing.descriptor_binding_partially_bound = VK_TRUE;
    descriptor_indexing.descriptor_binding_variable_descriptor_count = VK_TRUE;
    descriptor_indexing.descriptor_binding_sampled_image_update_after_bind = VK_TRUE;
    // Multi-paradigm render-path plan, rung R8 (code review P1-2 fix): closes R-VBGEO's
    // documented device-create gap — `enable_vb_geometry_table` is `true` only after the caller
    // queried `DeviceCaps::storage_buffer_array_non_uniform_indexing_ok`, which is now the
    // CONJUNCTION of BOTH bits this arm enables (`query_device_caps`'s doc — the original P1
    // bug queried only the first bit while enabling both, risking a hard
    // `VK_ERROR_FEATURE_NOT_PRESENT` on a device with the first but not the second). The SAME
    // "query before request" precedent `enable_ray_query` establishes above, now for BOTH bits,
    // so requesting either here can never fail `vkCreateDevice` on a device that lacks it.
    if enable_vb_geometry_table {
        descriptor_indexing.shader_storage_buffer_array_non_uniform_indexing = VK_TRUE;
        descriptor_indexing.descriptor_binding_storage_buffer_update_after_bind = VK_TRUE;
    }
    // Profiling rung 4 (D18): the granular `hostQueryReset` struct, spliced between the
    // descriptor-indexing head and `features13` when — and only when — the caller's
    // `supports_host_query_reset` query said yes. Built here, AFTER the hwrt arm has finished
    // mutating `features13.p_next` and BEFORE `descriptor_indexing` takes its address, so the
    // chain is still built tail-first with no read-after-mutate hazard. When the flag is false
    // the local is never chained and the walk order is byte-identical to before this rung.
    let mut host_query_reset = zeroed_host_query_reset_features();
    if enable_host_query_reset {
        host_query_reset.host_query_reset = VK_TRUE;
        host_query_reset.p_next =
            (&features13 as *const VkPhysicalDeviceVulkan13Features).cast::<c_void>()
                as *mut c_void;
    }

    descriptor_indexing.p_next = if enable_host_query_reset {
        (&host_query_reset as *const VkPhysicalDeviceHostQueryResetFeatures).cast::<c_void>()
            as *mut c_void
    } else {
        (&features13 as *const VkPhysicalDeviceVulkan13Features).cast::<c_void>() as *mut c_void
    };

    // The p_next chain head is ALWAYS the bindless descriptor-indexing struct:
    // descriptorIndexing → features13 → (hwrt only) rayQuery → accelerationStructure →
    // bufferDeviceAddress. Unlike the pre-T-dev chain, the head never changes shape
    // between builds — the RT sub-chain hangs off `features13.p_next` instead.
    let p_next: *const c_void =
        (&descriptor_indexing as *const VkPhysicalDeviceDescriptorIndexingFeatures).cast();

    // Core (Vulkan 1.0) features passed via `p_enabled_features`, NOT `pNext` (the two
    // are mutually exclusive — VUID-VkDeviceCreateInfo-pNext-00373). `samplerAnisotropy`
    // is the T2 aniso-sampler prerequisite; every other core bit stays `VK_FALSE`
    // (`Default` on every `VkBool32` field is `0`).
    let enabled_features = VkPhysicalDeviceFeatures {
        sampler_anisotropy: VK_TRUE,
        ..Default::default()
    };

    let create_info = VkDeviceCreateInfo {
        s_type: VkStructureType::DeviceCreateInfo,
        p_next,
        flags: 0,
        queue_create_info_count: 1,
        p_queue_create_infos: &queue_info,
        enabled_layer_count: 0,
        pp_enabled_layer_names: ptr::null(),
        enabled_extension_count: ext_count as u32,
        pp_enabled_extension_names: if ext_count == 0 {
            ptr::null()
        } else {
            ext_ptrs.as_ptr()
        },
        p_enabled_features: (&enabled_features as *const VkPhysicalDeviceFeatures).cast(),
    };

    let mut device = VkDevice::NULL;
    // SAFETY: `physical_device` is valid; `create_info` is a fully-initialized
    // `#[repr(C)]` struct whose `p_queue_create_infos`/`p_queue_priorities` pointers
    // (`&queue_info`, `&priority`), the `p_next` feature chain (`&descriptor_indexing` →
    // `&features13` → the RT feature structs on the hwrt arm — all frame locals that
    // outlive the call), the `p_enabled_features` pointer (`&enabled_features`, also a
    // frame local), and the extension-name array (`ext_ptrs`) all outlive the call;
    // `&mut device` is a valid out-pointer; NULL allocator picks the default. The
    // dynamic-rendering feature is verified supported above (Correction #2); the RT
    // extensions are appended only when `supports_ray_query` returned true (caller).
    let raw =
        unsafe { (fns.create_device)(physical_device, &create_info, ptr::null(), &mut device) };
    let result = VkResult::from_raw(raw);
    if !result.is_success() {
        return Err(BootError::VkError("vkCreateDevice", result));
    }
    Ok(device)
}

/// The `enable_ray_query` argument to [`create_device`] on a non-hwrt build (always `false`
/// — no RT extension is ever requested; the dormancy anchor).
#[cfg(not(feature = "hwrt"))]
const RT_ENABLE_DEFAULT: bool = false;

#[cfg(test)]
mod tests {
    // Test-harness serialization only: the `Mutex` below guards PROCESS-GLOBAL state
    // (`std::env::set_var` vs. a real device boot) between two `#[test]` fns on the harness's
    // own threads. It is not engine state and is compiled out of every shipping build.
    #![allow(clippy::disallowed_types)]

    use std::sync::Mutex;

    use super::{InstanceConfig, VulkanContext, validation_requested};
    use crate::error::VulkanError;
    use crate::log_probe::{arm, drain, observe_lock, observed};

    /// Serializes the two tests that interact through PROCESS-GLOBAL state:
    /// `validation_requested_env_gate` mutates `BOYKO_DISABLE_VALIDATION` via
    /// `std::env::set_var` (unsound if any other thread reads the environment
    /// concurrently), and `boot_singleton_destroy_singleton_round_trip` boots a
    /// real device — whose dynamic-loader open may `getenv` on POSIX (Linux is
    /// a target). Both tests hold this lock for their whole body, so the env
    /// mutation can never interleave with a boot. Poison-tolerant so one
    /// panicking test does not cascade-fail the other.
    static ENV_AND_BOOT_LOCK: Mutex<()> = Mutex::new(());

    /// R2 device-singleton lifecycle round trip + the exactly-once tripwires
    /// (review P0/P1-1): `boot_singleton` pins the ONE `&'static` device
    /// handle; a SECOND boot while it is live returns `SingletonAlreadyBooted`
    /// (the advisory fast path — no second device is ever created, so nothing
    /// leaks); `destroy_singleton` ends the lifecycle (the normal `Drop`
    /// teardown of device / instance / loader); a SECOND destroy panics on the
    /// null-swap tripwire BEFORE touching any memory (safe to catch). Skips
    /// gracefully when no loader / GPU is present, mirroring the integration
    /// tests' `boot_or_skip` convention.
    ///
    /// Headless, validation OFF by EXPLICIT config: `enable_validation: false`
    /// makes `validation_requested`'s `&&` SHORT-CIRCUIT skip the
    /// `BOYKO_DISABLE_VALIDATION` read entirely — the env var is never read
    /// when the flag is false — so the effective flag is `false` regardless of
    /// the environment and no validation message can be recorded by
    /// construction. `ENV_AND_BOOT_LOCK` additionally serializes this test
    /// against the sibling env-mutating test (the dynamic-loader open inside
    /// `boot` may still `getenv` on POSIX).
    #[test]
    fn boot_singleton_destroy_singleton_round_trip() {
        let _guard = ENV_AND_BOOT_LOCK.lock().unwrap_or_else(|e| e.into_inner());

        let config = InstanceConfig {
            enable_validation: false,
            ..InstanceConfig::default()
        };

        // Scope the `&'static` binding so it is out of scope BEFORE
        // `destroy_singleton` runs — the destroy contract is that no reference
        // obtained from `boot_singleton` exists any more.
        {
            let ctx = match VulkanContext::boot_singleton(config) {
                Ok(c) => c,
                Err(e) => {
                    eprintln!(
                        "SKIP boot_singleton_destroy_singleton_round_trip: Vulkan unavailable ({e:?})"
                    );
                    return;
                }
            };

            // Trivial use of the pinned handle: a booted context always has a
            // device name and passed the G-buffer storage-format boot fail-fast.
            assert!(
                !ctx.device_name().is_empty(),
                "a booted singleton reports its physical-device name"
            );
            assert!(
                ctx.device_caps().gbuffer_storage_format_ok,
                "the boot fail-fast guarantees G-buffer storage-format support"
            );

            // A second boot while the singleton is live is a contract
            // violation: the advisory fast path rejects it WITHOUT booting a
            // second device (so nothing leaks in this test).
            assert!(
                matches!(
                    VulkanContext::boot_singleton(config),
                    Err(VulkanError::SingletonAlreadyBooted)
                ),
                "a second boot_singleton while live must return SingletonAlreadyBooted"
            );
        }

        // SAFETY: the `boot_singleton` above succeeded and its singleton is
        // still live; the device is idle (no GPU work was ever submitted); and
        // no `&'static VulkanContext` reference exists any more — the only one
        // this test created went out of scope with the block above.
        unsafe { VulkanContext::destroy_singleton() };

        // The exactly-once tripwire: a second destroy must panic on the
        // null-swap BEFORE touching any memory, so catching the unwind is safe
        // (no partial teardown state exists on that path). The default panic
        // hook is silenced around the call so the EXPECTED panic does not spam
        // the test log.
        let prev_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {}));
        let second = std::panic::catch_unwind(|| {
            // SAFETY: intentionally violates the exactly-once contract to
            // assert the tripwire: the singleton is null, so the swap observes
            // null and panics before `Box::from_raw` — no memory is touched on
            // this path.
            unsafe { VulkanContext::destroy_singleton() };
        });
        std::panic::set_hook(prev_hook);
        assert!(
            second.is_err(),
            "a second destroy_singleton must panic on the null-swap tripwire"
        );
    }

    /// `BOYKO_DISABLE_VALIDATION` forces the effective flag to `false` regardless of
    /// `config.enable_validation`; with the var unset the helper mirrors the config
    /// (the default path is byte-identical to plain `config.enable_validation`).
    ///
    /// Mutates a process-global env var, so the three cases run in ONE test (no
    /// cross-test interleave), the prior value is saved + restored, and the
    /// whole body holds `ENV_AND_BOOT_LOCK` — the singleton boot test may read
    /// the environment inside the dynamic-loader open, and `set_var` must
    /// never interleave with that.
    #[test]
    fn validation_requested_env_gate() {
        let _guard = ENV_AND_BOOT_LOCK.lock().unwrap_or_else(|e| e.into_inner());

        const KEY: &str = "BOYKO_DISABLE_VALIDATION";
        let saved = std::env::var_os(KEY);

        let on = InstanceConfig {
            enable_validation: true,
            ..InstanceConfig::default()
        };
        let off = InstanceConfig {
            enable_validation: false,
            ..InstanceConfig::default()
        };

        // Env UNSET: helper mirrors the config (default-path invariant).
        // SAFETY: `ENV_AND_BOOT_LOCK` (held for this whole test) serializes
        // every env mutation here against the only other env reader in this
        // binary — the singleton boot test, whose loader open may `getenv` —
        // so no thread reads the environment concurrently. The original value
        // is restored at the end of the test.
        unsafe { std::env::remove_var(KEY) };
        assert!(validation_requested(&on), "env unset + config true => requested");
        assert!(
            !validation_requested(&off),
            "env unset + config false => not requested"
        );

        // Env SET: forces `false` even when the config requests validation.
        // SAFETY: as above — serialized by `ENV_AND_BOOT_LOCK`.
        unsafe { std::env::set_var(KEY, "1") };
        assert!(
            !validation_requested(&on),
            "env set overrides config true => not requested"
        );
        assert!(
            !validation_requested(&off),
            "env set + config false => not requested"
        );

        // Restore the prior process env so sibling tests are unaffected.
        // SAFETY: as above — serialized by `ENV_AND_BOOT_LOCK`.
        match saved {
            Some(v) => unsafe { std::env::set_var(KEY, v) },
            None => unsafe { std::env::remove_var(KEY) },
        }
    }

    // ── SDFDDGI I0: the resolve-descriptor per-type limit check (locks the fragile byte offsets). ──

    use super::{
        BootError, RESOLVE_NEED_COMBINED_IMAGE_SAMPLERS, RESOLVE_NEED_STORAGE_BUFFERS,
        RESOLVE_NEED_STORAGE_IMAGES, RESOLVE_NEED_UNIFORM_BUFFERS, check_resolve_descriptor_limits,
    };
    use crate::ffi::{
        LIMITS_OFF_MAX_PER_STAGE_SAMPLED_IMAGES, LIMITS_OFF_MAX_PER_STAGE_SAMPLERS,
        LIMITS_OFF_MAX_PER_STAGE_STORAGE_BUFFERS, LIMITS_OFF_MAX_PER_STAGE_STORAGE_IMAGES,
        LIMITS_OFF_MAX_PER_STAGE_UNIFORM_BUFFERS, VkPhysicalDeviceLimitsBlob,
    };

    /// Fabricates a `VkPhysicalDeviceLimitsBlob` with the five `maxPerStageDescriptor*` fields the
    /// resolve check reads written at their documented offsets (68/72/76/80/84). Every other byte
    /// stays zero — the check never reads them. This is a HAND-BUILT blob (no driver call), so the
    /// test locks the byte offsets against a silent drift (the I(-1)-class fragility) without a GPU.
    fn limits_blob(
        samplers: u32,
        uniform_buffers: u32,
        storage_buffers: u32,
        sampled_images: u32,
        storage_images: u32,
    ) -> VkPhysicalDeviceLimitsBlob {
        let mut bytes = [0u8; 504];
        let put = |b: &mut [u8; 504], off: usize, v: u32| {
            b[off..off + 4].copy_from_slice(&v.to_ne_bytes());
        };
        put(&mut bytes, LIMITS_OFF_MAX_PER_STAGE_SAMPLERS, samplers);
        put(&mut bytes, LIMITS_OFF_MAX_PER_STAGE_UNIFORM_BUFFERS, uniform_buffers);
        put(&mut bytes, LIMITS_OFF_MAX_PER_STAGE_STORAGE_BUFFERS, storage_buffers);
        put(&mut bytes, LIMITS_OFF_MAX_PER_STAGE_SAMPLED_IMAGES, sampled_images);
        put(&mut bytes, LIMITS_OFF_MAX_PER_STAGE_STORAGE_IMAGES, storage_images);
        VkPhysicalDeviceLimitsBlob(bytes)
    }

    #[test]
    fn resolve_descriptor_check_rejects_below_need_and_accepts_generous() {
        // (a) A device whose `maxPerStageDescriptorStorageBuffers` is BELOW the resolve need
        // (4 < RESOLVE_NEED_STORAGE_BUFFERS == 5) is rejected, carrying the exact (kind, need, limit).
        // Every OTHER limit is generous, so the storage-buffer row is the one that fires.
        let starved = limits_blob(
            1_000_000, // samplers
            1_000_000, // uniform buffers
            4,         // storage buffers — BELOW the need of 5
            1_000_000, // sampled images
            1_000_000, // storage images
        );
        match check_resolve_descriptor_limits(&starved) {
            Err(BootError::ResolveDescriptorLimitExceeded { kind, need, limit }) => {
                assert_eq!(kind, "maxPerStageDescriptorStorageBuffers");
                assert_eq!(need, RESOLVE_NEED_STORAGE_BUFFERS);
                assert_eq!(need, 5);
                assert_eq!(limit, 4);
            }
            other => panic!("expected ResolveDescriptorLimitExceeded, got {other:?}"),
        }

        // (b) A device with every per-type limit set generously satisfies all five per-type needs.
        let generous = limits_blob(1_000_000, 1_000_000, 1_000_000, 1_000_000, 1_000_000);
        assert!(check_resolve_descriptor_limits(&generous).is_ok());

        // Sanity: the needs the check enforces are the pinned per-type counts (19 total).
        assert_eq!(RESOLVE_NEED_COMBINED_IMAGE_SAMPLERS, 4);
        assert_eq!(RESOLVE_NEED_STORAGE_IMAGES, 6);
        assert_eq!(RESOLVE_NEED_STORAGE_BUFFERS, 5);
        assert_eq!(RESOLVE_NEED_UNIFORM_BUFFERS, 4);
        assert_eq!(
            RESOLVE_NEED_COMBINED_IMAGE_SAMPLERS
                + RESOLVE_NEED_STORAGE_IMAGES
                + RESOLVE_NEED_STORAGE_BUFFERS
                + RESOLVE_NEED_UNIFORM_BUFFERS,
            19
        );
    }

    // ── HW-RT rung R1: the RtTier truth table (plan §8). ──

    use super::{DeviceCaps, RtTier};

    /// Builds a [`DeviceCaps`] varying ONLY the two RT feature bits — every other
    /// field is a benign placeholder the tier decision never reads. Not a driver
    /// query: this locks the pure `rt_tier()` truth table without a GPU.
    fn rt_caps(ray_query: bool, ray_reorder: bool) -> DeviceCaps {
        DeviceCaps {
            bindless_capable: false,
            storage_buffer_array_non_uniform_indexing_ok: false,
            gbuffer_storage_format_ok: true,
            viewt_storage_format_ok: true,
            gbuffer_color_attachment_format_ok: true,
            r8_unorm_storage_ok: true,
            atlas_linear_filter_ok: true,
            ddgi_irr_storage_ok: true,
            ddgi_depth_storage_ok: true,
            rg8_unorm_storage_ok: true,
            #[cfg(feature = "hwrt")]
            rg16_unorm_storage_ok: true,
            r16_unorm_storage_ok: true,
            timestamp_period: 1.0,
            timestamp_valid_bits: 64,
            timestamp_compute_and_graphics: true,
            host_query_reset: false,
            calibrated_timestamps: false,
            ray_query,
            ray_reorder,
            vendor_id: 0,
            device_id: 0,
            driver_version: 0,
            as_scratch_align: 0,
            max_image_dimension_2d: 0,
            device_local_heap_bytes: 0,
            max_bound_descriptor_sets: 0,
        }
    }

    #[test]
    fn rt_tier_truth_table() {
        // `ray_query == false` ⇒ Absent regardless of `ray_reorder` (the dormancy
        // anchor — the only reachable state in R1).
        assert_eq!(rt_caps(false, false).rt_tier(), RtTier::Absent);
        assert_eq!(rt_caps(false, true).rt_tier(), RtTier::Absent);
        // `ray_query == true` splits on reorder: without ⇒ Weak, with ⇒ Strong
        // (the R2a arms, unreachable in R1 but pinned here).
        assert_eq!(rt_caps(true, false).rt_tier(), RtTier::Weak);
        assert_eq!(rt_caps(true, true).rt_tier(), RtTier::Strong);
    }

    /// **`boyko-W2102`: three sites, one code, and all three must report.**
    ///
    /// This is the test for the claim `logging/emission-path`'s F11 exists to make: `RatePolicy`
    /// is indexed by code, so a code-scoped `Once` would fire for whichever of the three device
    /// degradations happened first and drop the other two -- uncounted, because `Once` deliberately
    /// does not count its suppressions. The latch is per SITE instead, and the way to show that is
    /// to trip all three and count.
    ///
    /// The RED that earned it: give the three reporters one shared `static FIRED` and this asserts
    /// `1 == 3`.
    ///
    /// The exact delta is sound rather than hopeful -- see `crate::log_probe`'s header for the
    /// measurement that no other `RhiVulkan` record can appear in this binary. The lock is held for
    /// the same reason the two tests above hold it: it is this module's serializer against the one
    /// device boot, which is the only other thing in the crate that could reach a reporter.
    #[test]
    fn w2102_reports_every_degradation_not_just_the_first() {
        let _guard = ENV_AND_BOOT_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _observe = observe_lock();
        arm();

        // `hwrt` off compiles the shadow-denoise probe -- and therefore its reporter -- out of the
        // crate entirely, so the expected count is a property of the build, not a magic number.
        #[cfg(feature = "hwrt")]
        const SITES: u64 = 3;
        #[cfg(not(feature = "hwrt"))]
        const SITES: u64 = 2;

        let before = observed();
        super::report_ddgi_storage_unsupported(false, false);
        super::report_ssao_denoise_storage_unsupported();
        #[cfg(feature = "hwrt")]
        super::report_shadow_denoise_storage_unsupported(false, true);
        drain();
        assert_eq!(
            observed() - before,
            SITES,
            "boyko-W2102 must report EVERY degradation; a code-scoped latch reports one and \
             loses the rest in silence"
        );

        // Second round: every site's latch is spent, so the whole round is silent. This is the
        // clause that would catch a `RatePolicy::Every` slipping in and turning a boot-time notice
        // into per-boot noise.
        let after_first = observed();
        super::report_ddgi_storage_unsupported(false, false);
        super::report_ssao_denoise_storage_unsupported();
        #[cfg(feature = "hwrt")]
        super::report_shadow_denoise_storage_unsupported(false, true);
        drain();
        assert_eq!(observed(), after_first, "a spent Once site let a second W2102 through");
    }
}
