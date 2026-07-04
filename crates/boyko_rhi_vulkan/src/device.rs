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

use core::cell::{OnceCell, RefCell};
use core::ffi::{CStr, c_char, c_void};
use core::mem;
use core::ptr;
use core::sync::atomic::{AtomicPtr, Ordering};

use crate::debug::{self, DebugMessengerState};
use crate::ffi::*;
use crate::memory::{DeviceLocalBlock, HostVisibleBlock};
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

/// Minimal physical-device capabilities queried ONCE at device-create (Render P1b),
/// alongside the `dynamicRendering` fail-fast.
///
/// A small POD recorded on the [`VulkanContext`] and exposed read-only via
/// [`VulkanContext::device_caps`]. P1b records `bindless_capable` for a FUTURE bindless
/// path (it is NOT consumed yet — declaring an unused capability is intentional
/// forward wiring, not dead code); `gbuffer_storage_format_ok` is asserted at boot, so
/// a context that exists always has it `true` (the fail-fast rejects a GPU without it).
#[derive(Clone, Copy, Debug)]
pub struct DeviceCaps {
    /// Whether the GPU advertises the Vulkan 1.2 `descriptorIndexing` +
    /// `runtimeDescriptorArray` features (the bindless prerequisite). RECORDED ONLY in
    /// P1b — a future bindless G-buffer path reads it; nothing consumes it yet.
    pub bindless_capable: bool,
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
}

impl DeviceCaps {
    /// The SDF brick-atlas image format chosen from [`Self::atlas_linear_filter_ok`]
    /// (SDF brick-atlas campaign M2): `R8_SNORM` when the GPU supports linear filtering on
    /// it (the dense quantized path), else the `R16_SFLOAT` D8 fallback (half-float, no
    /// quantization — the `EPSILON_Q` store bias is harmless there). Both the CPU baker and
    /// the GPU decode handle either format. Returned as the agnostic [`Format`] the
    /// `create_texture` path maps to a `VkFormat`.
    #[inline]
    pub const fn atlas_format(&self) -> boyko_rhi::Format {
        if self.atlas_linear_filter_ok {
            boyko_rhi::Format::R8Snorm
        } else {
            boyko_rhi::Format::R16Sfloat
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
    pub cmd_pipeline_barrier: PfnVkCmdPipelineBarrier,
    /// `vkCmdCopyBuffer` — the Phase-5 staging upload + readback transfer
    /// (Vulkan 1.0 core, always present).
    pub cmd_copy_buffer: PfnVkCmdCopyBuffer,
    /// `vkCmdFillBuffer` — the Lighting-L1 cull's per-frame reset of the `LightIndexAlloc`
    /// counter to 0 before the cull dispatch (Vulkan 1.0 core, always present).
    pub cmd_fill_buffer: PfnVkCmdFillBuffer,
    /// `vkCmdClearColorImage` — the SDFDDGI I1 boot-clear of the probe IRRADIANCE + DEPTH
    /// color atlases to defined values (Vulkan 1.0 core, always present).
    pub cmd_clear_color_image: PfnVkCmdClearColorImage,
    pub create_fence: PfnVkCreateFence,
    pub destroy_fence: PfnVkDestroyFence,
    pub wait_for_fences: PfnVkWaitForFences,
    pub queue_submit: PfnVkQueueSubmit,
    pub device_wait_idle: PfnVkDeviceWaitIdle,
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
    /// The single shared host-visible+coherent block every
    /// [`RhiDevice::create_buffer`](boyko_rhi::RhiDevice::create_buffer) sub-allocates
    /// from (plan Q1), created lazily on first use.
    ///
    /// The block caches a raw `*const DeviceFns` into the boxed `device_fns`
    /// (plan A1): the box gives the fn-table a stable heap address, so the cached
    /// pointer survives any move of this context — no false `'static` lifetime is
    /// claimed. The block is torn down in `Drop` BEFORE `vkDestroyDevice` + before
    /// the boxed fn-table is freed, so the pointer is live for every block use.
    /// The `RefCell` provides the `&mut` the sub-allocator needs from `&self`
    /// calls (single-threaded, `!Sync`).
    host_block: OnceCell<RefCell<HostVisibleBlock>>,
    /// The single shared device-local (VRAM) block every
    /// [`RhiDevice::create_buffer`](boyko_rhi::RhiDevice::create_buffer) with
    /// [`MemoryLocation::DeviceLocal`](boyko_rhi::MemoryLocation::DeviceLocal)
    /// sub-allocates from (the Phase-5 `GpuColumn` seam), created lazily on first
    /// use. Never mapped (plan D3/MF-8). Caches the same plan-A1 `*const DeviceFns`
    /// and is torn down in `Drop` BEFORE `vkDestroyDevice` + the boxed fn-table.
    device_block: OnceCell<RefCell<DeviceLocalBlock>>,
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
        let config = InstanceConfig {
            enable_validation: validation_requested(&config),
            ..config
        };

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

        // --- 5. Find a graphics+compute queue family. ---
        let queue_family_index = match find_queue_family(&instance_fns, physical_device) {
            Ok(q) => q,
            Err(e) => fail!(e),
        };

        // --- 5b. Query the minimal device caps ONCE (Render P1b), alongside the
        // `dynamicRendering` fail-fast in `create_device`. `bindless_capable` is
        // recorded only; `gbuffer_storage_format_ok` is fail-fast here so a context
        // that exists always has it (a marcher storage-image store can never fault on
        // an unsupported format). Core-guaranteed on the RTX 3060.
        let device_caps = query_device_caps(&instance_fns, physical_device);
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

        // --- 6. Create the logical device + retrieve the queue. ---
        let device = match create_device(
            &instance_fns,
            physical_device,
            queue_family_index,
            config.windowed,
        ) {
            Ok(d) => d,
            Err(e) => fail!(e),
        };

        let device_fns = match load_device_fns(
            instance_fns.get_device_proc_addr,
            device,
            config.windowed,
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
            host_block: OnceCell::new(),
            device_block: OnceCell::new(),
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
    /// context always has `gbuffer_storage_format_ok == true` (the boot fail-fast
    /// rejects a GPU lacking it); `bindless_capable` is recorded for a future bindless
    /// path.
    #[inline]
    pub fn device_caps(&self) -> DeviceCaps {
        self.device_caps
    }

    /// The resolved device command table.
    #[inline]
    pub fn device_fns(&self) -> &DeviceFns {
        &self.device_fns
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

    /// The single shared host-visible+coherent block, created on first use and
    /// cached for the device's lifetime (plan Q1). Every
    /// [`RhiDevice::create_buffer`](boyko_rhi::RhiDevice::create_buffer) sub-allocates
    /// from it. Returns a [`VulkanError`](crate::error::VulkanError) if the block
    /// allocation fails.
    pub(crate) fn host_block(
        &self,
    ) -> Result<&RefCell<HostVisibleBlock>, crate::error::VulkanError> {
        if let Some(block) = self.host_block.get() {
            return Ok(block);
        }
        // Plan A1: the block caches a raw `*const DeviceFns` pointing into the
        // boxed `device_fns` — a stable heap address. NO `'static` lifetime is
        // fabricated; `HostVisibleBlock::new` captures the borrow as a raw pointer
        // internally. The invariant that makes this sound: the boxed fn-table
        // address does not move when the context moves, and the block is dropped
        // in this context's `Drop` (via `host_block.take()`) BEFORE the boxed
        // fn-table is freed and before `vkDestroyDevice`, so the pointee outlives
        // every block use. The context is `!Send + !Sync`, so it never crosses a
        // thread.
        let block = HostVisibleBlock::new(
            self.device(),
            self.device_fns(),
            self.memory_properties(),
            SHARED_HOST_BLOCK_CAPACITY,
        )?;
        // Race-free: `&self` is single-threaded; the cell is empty here.
        let _ = self.host_block.set(RefCell::new(block));
        Ok(self
            .host_block
            .get()
            .expect("invariant: host_block was just set"))
    }

    /// The single shared device-local (VRAM) block, created on first use and
    /// cached for the device's lifetime (plan D3/MF-8). Every
    /// [`RhiDevice::create_buffer`](boyko_rhi::RhiDevice::create_buffer) with
    /// [`MemoryLocation::DeviceLocal`](boyko_rhi::MemoryLocation::DeviceLocal)
    /// sub-allocates from it. The block is never mapped. Returns a
    /// [`VulkanError`](crate::error::VulkanError) if the block allocation fails.
    pub(crate) fn device_block(
        &self,
    ) -> Result<&RefCell<DeviceLocalBlock>, crate::error::VulkanError> {
        if let Some(block) = self.device_block.get() {
            return Ok(block);
        }
        // Plan A1 (identical to `host_block`): the block caches a raw
        // `*const DeviceFns` into the boxed `device_fns` — a stable heap address.
        // The block is dropped in this context's `Drop` (via `device_block.take()`)
        // BEFORE the boxed fn-table is freed and before `vkDestroyDevice`, so the
        // pointee outlives every block use. The context is `!Send + !Sync`.
        let block = DeviceLocalBlock::new(
            self.device(),
            self.device_fns(),
            self.memory_properties(),
            SHARED_DEVICE_BLOCK_CAPACITY,
        )?;
        // Race-free: `&self` is single-threaded; the cell is empty here.
        let _ = self.device_block.set(RefCell::new(block));
        Ok(self
            .device_block
            .get()
            .expect("invariant: device_block was just set"))
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
        // The shared host-visible block (if ever created) is torn down FIRST: its
        // own `Drop` calls `vkUnmapMemory` + `vkFreeMemory` through the raw
        // `*const DeviceFns` it cached, which targets the still-live boxed
        // `device_fns` (the box is a field of `self`, dropped implicitly AFTER this
        // `drop` body runs — plan A1), and it must precede `vkDestroyDevice`. Any
        // buffers sub-allocated from it were already destroyed via
        // `RhiDevice::destroy_buffer` / the registry's `destroy_all` before the
        // context dropped.
        if let Some(block) = self.host_block.take() {
            drop(block);
        }
        // The shared device-local block (if ever created) is torn down next, also
        // BEFORE `vkDestroyDevice`. Its `Drop` calls only `vkFreeMemory` (it was
        // never mapped) through the same plan-A1 raw `*const DeviceFns` into the
        // still-live boxed `device_fns`. Any device-local buffers sub-allocated
        // from it were already destroyed via `RhiDevice::destroy_buffer` / the
        // registry's `destroy_all` before the context dropped.
        if let Some(block) = self.device_block.take() {
            drop(block);
        }
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
            cmd_pipeline_barrier: load_device_command(gdpa, device, c"vkCmdPipelineBarrier")?,
            cmd_copy_buffer: load_device_command(gdpa, device, c"vkCmdCopyBuffer")?,
            cmd_fill_buffer: load_device_command(gdpa, device, c"vkCmdFillBuffer")?,
            cmd_clear_color_image: load_device_command(gdpa, device, c"vkCmdClearColorImage")?,
            create_fence: load_device_command(gdpa, device, c"vkCreateFence")?,
            destroy_fence: load_device_command(gdpa, device, c"vkDestroyFence")?,
            wait_for_fences: load_device_command(gdpa, device, c"vkWaitForFences")?,
            queue_submit: load_device_command(gdpa, device, c"vkQueueSubmit")?,
            device_wait_idle: load_device_command(gdpa, device, c"vkDeviceWaitIdle")?,
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

/// Finds a queue family that supports both graphics and compute.
fn find_queue_family(fns: &InstanceFns, device: VkPhysicalDevice) -> Result<u32, BootError> {
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
            return Ok(idx as u32);
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

/// Queries the minimal Render P1b [`DeviceCaps`]: whether the GPU advertises the
/// bindless prerequisite (Vulkan 1.2 `descriptorIndexing` + `runtimeDescriptorArray`,
/// chained into `vkGetPhysicalDeviceFeatures2`), whether `R8G8B8A8_UNORM` supports
/// `STORAGE_IMAGE` under OPTIMAL tiling (`vkGetPhysicalDeviceFormatProperties`), and
/// (Lighting L0b / W2) whether `R32_SFLOAT` supports `STORAGE_IMAGE` for the `gViewT`
/// lane.
///
/// `bindless_capable` is recorded only (a future bindless path reads it); the caller
/// fail-fasts on `!gbuffer_storage_format_ok` and `!viewt_storage_format_ok` so the
/// marcher's G-buffer / `gViewT` stores can never fault on an unsupported format. P1b
/// enables NEITHER feature at device
/// creation — the shader declares explicit storage-image formats (so
/// `shaderStorageImageWriteWithoutFormat` is not needed) and bindless is unused.
fn query_device_caps(fns: &InstanceFns, physical_device: VkPhysicalDevice) -> DeviceCaps {
    // --- bindless_capable: read the Vulkan 1.2 core feature bools via features2. ---
    // SAFETY: `VkPhysicalDeviceVulkan12Features` is `#[repr(C)]` with only an `s_type`
    // enum, a pointer, and `VkBool32`s — all-zero is a valid initial bit pattern (a
    // null `p_next` + `FALSE` bools); the driver overwrites every bool it owns through
    // the `p_next` chain below. `s_type`/`p_next` are then set explicitly.
    let mut features12: VkPhysicalDeviceVulkan12Features = unsafe { mem::zeroed() };
    features12.s_type = VkStructureType::PhysicalDeviceVulkan12Features;
    features12.p_next = ptr::null_mut();
    let mut features2 = VkPhysicalDeviceFeatures2 {
        s_type: VkStructureType::PhysicalDeviceFeatures2,
        p_next: (&mut features12 as *mut VkPhysicalDeviceVulkan12Features).cast(),
        features: [VK_FALSE; 55],
    };
    // SAFETY: `physical_device` is a valid enumerated GPU; `features2` is a
    // fully-initialized `#[repr(C)]` struct whose `p_next` chains the live `features12`
    // local (both outlive the call). The driver writes the supported feature bools
    // through the out-pointer + the chained struct.
    unsafe { (fns.get_physical_device_features2)(physical_device, &mut features2) };
    let bindless_capable = features12.descriptor_indexing == VK_TRUE
        && features12.runtime_descriptor_array == VK_TRUE;

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

    DeviceCaps {
        bindless_capable,
        gbuffer_storage_format_ok,
        viewt_storage_format_ok,
        gbuffer_color_attachment_format_ok,
        r8_unorm_storage_ok,
        atlas_linear_filter_ok,
    }
}

/// Creates a logical device with one queue from `queue_family_index`.
///
/// The Vulkan 1.3 `dynamicRendering` feature is ALWAYS requested through a
/// `VkPhysicalDeviceVulkan13Features` chained into `p_next` — including the
/// **headless** path (Correction #1): every S0 acceptance path records
/// `cmd_begin_rendering`, which faults without the feature enabled. Support is
/// verified up front by [`supports_dynamic_rendering`] (Correction #2). When
/// `windowed`, the `VK_KHR_swapchain` device extension is additionally enabled.
fn create_device(
    fns: &InstanceFns,
    physical_device: VkPhysicalDevice,
    queue_family_index: u32,
    windowed: bool,
) -> Result<VkDevice, BootError> {
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
    let swapchain_ext: [*const c_char; 1] = [VK_KHR_SWAPCHAIN_EXTENSION_NAME.as_ptr()];
    let mut features13 = zeroed_features13();
    features13.dynamic_rendering = VK_TRUE;
    let p_next: *const c_void = (&features13 as *const VkPhysicalDeviceVulkan13Features).cast();
    let (ext_count, pp_exts): (u32, *const *const c_char) = if windowed {
        (1, swapchain_ext.as_ptr())
    } else {
        (0, ptr::null())
    };

    let create_info = VkDeviceCreateInfo {
        s_type: VkStructureType::DeviceCreateInfo,
        p_next,
        flags: 0,
        queue_create_info_count: 1,
        p_queue_create_infos: &queue_info,
        enabled_layer_count: 0,
        pp_enabled_layer_names: ptr::null(),
        enabled_extension_count: ext_count,
        pp_enabled_extension_names: pp_exts,
        p_enabled_features: ptr::null(),
    };

    let mut device = VkDevice::NULL;
    // SAFETY: `physical_device` is valid; `create_info` is a fully-initialized
    // `#[repr(C)]` struct whose `p_queue_create_infos`/`p_queue_priorities`
    // pointers (`&queue_info`, `&priority`), the always-present `p_next`
    // `dynamicRendering` feature chain (`&features13`), and the windowed-only
    // extension array (`swapchain_ext`) all outlive the call (locals of this
    // frame); `&mut device` is a valid out-pointer; NULL allocator picks the
    // default. The feature is verified supported above (Correction #2).
    let raw =
        unsafe { (fns.create_device)(physical_device, &create_info, ptr::null(), &mut device) };
    let result = VkResult::from_raw(raw);
    if !result.is_success() {
        return Err(BootError::VkError("vkCreateDevice", result));
    }
    Ok(device)
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::{InstanceConfig, VulkanContext, validation_requested};
    use crate::error::VulkanError;

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
}
