//! HW-RT rung R2a-1 — the bound acceleration structure + the Vulkan AS verb impls.
//!
//! Gated `#[cfg(feature = "hwrt")]` (the `pub mod` in `lib.rs`). This module holds:
//! - [`BoundAccelStruct`] — the owned `VkAccelerationStructureKHR` + its backing buffer +
//!   its device address + its level ([`RhiApi::AccelerationStructure`] for the Vulkan
//!   backend under `hwrt`).
//! - [`AccelFns`] — the resolved `VK_KHR_acceleration_structure` device-command table
//!   (`Option<AccelFns>` on the context, mirroring `swapchain: Option<SwapchainDeviceFns>`),
//!   loaded ONLY when the 3 RT extensions were enabled at device create.
//! - the inherent verb helpers the [`RhiDevice`]/[`RhiCommandEncoder`] overrides
//!   (in `rhi_impl/{device,encoder}.rs`) delegate to — the real `vkCreate*` / `vkCmd*` / `vkGet*` FFI calls.
//!
//! R2a-1 builds NO acceleration structure and traces nothing; these verbs are the FFI
//! surface later rungs (R2a-2 BLAS build, R2a-3 TLAS) call.

use core::mem::MaybeUninit;
use core::ptr;

use boyko_rhi::{AsBuildEntry, AsBuildSizes, AsGeometryDesc, AsIndexType, AsKind};

use crate::accel_ffi::{
    PfnVkCmdBuildAccelerationStructuresKHR, PfnVkCreateAccelerationStructureKHR,
    PfnVkDestroyAccelerationStructureKHR, PfnVkGetAccelerationStructureBuildSizesKHR,
    PfnVkGetAccelerationStructureDeviceAddressKHR, PfnVkGetBufferDeviceAddressKHR,
    ST_ACCELERATION_STRUCTURE_BUILD_GEOMETRY_INFO_KHR, ST_ACCELERATION_STRUCTURE_BUILD_SIZES_INFO_KHR,
    ST_ACCELERATION_STRUCTURE_CREATE_INFO_KHR, ST_ACCELERATION_STRUCTURE_DEVICE_ADDRESS_INFO_KHR,
    ST_ACCELERATION_STRUCTURE_GEOMETRY_INSTANCES_DATA_KHR, ST_ACCELERATION_STRUCTURE_GEOMETRY_KHR,
    ST_ACCELERATION_STRUCTURE_GEOMETRY_TRIANGLES_DATA_KHR, ST_BUFFER_DEVICE_ADDRESS_INFO,
    VkAccelerationStructureBuildGeometryInfoKHR, VkAccelerationStructureBuildRangeInfoKHR,
    VkAccelerationStructureBuildSizesInfoKHR, VkAccelerationStructureCreateInfoKHR,
    VkAccelerationStructureDeviceAddressInfoKHR, VkAccelerationStructureGeometryDataKHR,
    VkAccelerationStructureGeometryInstancesDataKHR, VkAccelerationStructureGeometryKHR,
    VkAccelerationStructureGeometryTrianglesDataKHR, VkAccelerationStructureInstanceKHR,
    VkAccelerationStructureKHR, VkBufferDeviceAddressInfo, VkMemoryBarrier, ST_MEMORY_BARRIER,
    VK_ACCELERATION_STRUCTURE_BUILD_TYPE_DEVICE_KHR,
    VK_ACCELERATION_STRUCTURE_TYPE_BOTTOM_LEVEL_KHR, VK_ACCELERATION_STRUCTURE_TYPE_TOP_LEVEL_KHR,
    VK_BUILD_ACCELERATION_STRUCTURE_MODE_BUILD_KHR,
    VK_BUILD_ACCELERATION_STRUCTURE_PREFER_FAST_TRACE_BIT_KHR, VK_FORMAT_R32G32B32_SFLOAT,
    VK_GEOMETRY_OPAQUE_BIT_KHR, VK_GEOMETRY_TYPE_INSTANCES_KHR, VK_GEOMETRY_TYPE_TRIANGLES_KHR,
    VK_INDEX_TYPE_UINT16, VK_INDEX_TYPE_UINT32,
};
use crate::device::{DeviceFns, VulkanContext};
use crate::error::VulkanError;
use crate::ffi::{
    VkBuffer, VkCommandBuffer, VkDevice, VkResult, VK_ACCESS_ACCELERATION_STRUCTURE_READ_BIT_KHR,
    VK_ACCESS_ACCELERATION_STRUCTURE_WRITE_BIT_KHR,
    VK_PIPELINE_STAGE_ACCELERATION_STRUCTURE_BUILD_BIT_KHR, VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
};

/// The `VK_KHR_acceleration_structure` device-command table (HW-RT rung R2a-1).
///
/// Resolved ONLY when the 3 RT extensions were enabled at device create (mirroring
/// `SwapchainDeviceFns`, which loads only for a windowed device). Stored as
/// `Option<AccelFns>` on the [`VulkanContext`]: `None` when `hwrt` is off OR the device
/// lacks ray query (`ray_query == false`), so the AS verbs cannot dereference an absent
/// table. `vkGetBufferDeviceAddress` is loaded here too (its `KHR` alias resolves under
/// `acceleration_structure`).
pub struct AccelFns {
    pub get_build_sizes: PfnVkGetAccelerationStructureBuildSizesKHR,
    pub create: PfnVkCreateAccelerationStructureKHR,
    pub destroy: PfnVkDestroyAccelerationStructureKHR,
    pub cmd_build: PfnVkCmdBuildAccelerationStructuresKHR,
    pub get_device_address: PfnVkGetAccelerationStructureDeviceAddressKHR,
    pub get_buffer_device_address: PfnVkGetBufferDeviceAddressKHR,
}

impl AccelFns {
    /// Resolves the 6 acceleration-structure device commands through `gdpa`
    /// (`vkGetDeviceProcAddr`). Returns `None` if any is absent (a device that enabled the
    /// extensions must expose them, so `None` here signals a driver bug — the caller then
    /// leaves `ray_query == false` and the software path stands).
    ///
    /// # Safety
    /// `gdpa` must be the live device's `vkGetDeviceProcAddr` and `device` a valid handle;
    /// each resolved pointer is transmuted to its command's PFN typedef (ABI-checked by the
    /// signature). The caller enables the RT extensions before calling this.
    pub(crate) unsafe fn load(
        gdpa: crate::ffi::PfnVkGetDeviceProcAddr,
        device: VkDevice,
    ) -> Option<Self> {
        // SAFETY: `load_one` resolves a device command name to its PFN or `None`; each `T`
        // matches the named command's ABI (the `transmute` is the loader idiom the crate's
        // `load_device_command` uses). `gdpa`/`device` are live (caller contract).
        unsafe {
            Some(Self {
                get_build_sizes: load_one(gdpa, device, c"vkGetAccelerationStructureBuildSizesKHR")?,
                create: load_one(gdpa, device, c"vkCreateAccelerationStructureKHR")?,
                destroy: load_one(gdpa, device, c"vkDestroyAccelerationStructureKHR")?,
                cmd_build: load_one(gdpa, device, c"vkCmdBuildAccelerationStructuresKHR")?,
                get_device_address: load_one(
                    gdpa,
                    device,
                    c"vkGetAccelerationStructureDeviceAddressKHR",
                )?,
                // `bufferDeviceAddress` is enabled as the CORE Vulkan 1.2 feature (NOT the
                // `VK_KHR_buffer_device_address` extension), so the device exposes the CORE
                // `vkGetBufferDeviceAddress`; the `KHR`-suffixed alias is present ONLY when
                // that extension string is enabled (which we never enable — the core feature
                // bit suffices). Resolve the core name first, falling back to the `KHR` alias
                // for a driver that somehow exposes only the latter. (A hardware bug this
                // catches: on the RTX 3060 the `KHR` alias returns null → the whole table
                // failed to load → `ray_query` never latched, silently disabling HW-RT.)
                get_buffer_device_address: load_one(gdpa, device, c"vkGetBufferDeviceAddress")
                    .or_else(|| load_one(gdpa, device, c"vkGetBufferDeviceAddressKHR"))?,
            })
        }
    }
}

/// Resolves one device command name to a typed PFN or `None`.
///
/// # Safety
/// `gdpa`/`device` live; `T` is the command's PFN typedef (the caller passes the matching
/// name), so the reinterpret of the returned function pointer is ABI-correct.
unsafe fn load_one<T: Copy>(
    gdpa: crate::ffi::PfnVkGetDeviceProcAddr,
    device: VkDevice,
    name: &core::ffi::CStr,
) -> Option<T> {
    // SAFETY: `gdpa` is the live device's proc-addr fn; a null return means the command is
    // absent → `None`. The non-null pointer is transmuted to `T` (the command's PFN
    // typedef), matching how `crate::device::load_device_command` resolves core commands.
    let f = unsafe { (gdpa)(device, name.as_ptr()) };
    f.map(|p| {
        debug_assert_eq!(
            size_of::<T>(),
            size_of::<crate::ffi::PfnVkVoidFunction>(),
            "invariant: a PFN typedef is a single function pointer"
        );
        // SAFETY: `p` is a valid non-null function pointer the loader returned for `name`;
        // `T` is that command's `unsafe extern "system" fn` typedef (same size/ABI), so the
        // transmute reinterprets the pointer without changing its value.
        unsafe { core::mem::transmute_copy::<crate::ffi::PfnVkVoidFunction, T>(&Some(p)) }
    })
}

/// An owned Vulkan acceleration structure ([`RhiApi::AccelerationStructure`] under `hwrt`,
/// HW-RT rung R2a-1).
///
/// Holds the `VkAccelerationStructureKHR` handle, the backing `VkBuffer` it lives in (kept
/// so the caller can track its lifetime — it MUST outlive the AS), the cached device
/// address (the value a TLAS instance's `accelerationStructureReference` needs; `0` until
/// [`VulkanContext::accel_device_address`] fills it), and the level.
///
/// # `!Send`
/// The raw `VkBuffer`/handle are single-thread-only (the RHI is touched only by the
/// dispatcher, §5.3). The struct is `!Send + !Sync` by carrying no `Send` marker — it holds
/// only `Copy` handle newtypes, so it is auto-`Send`; the `_not_send` `PhantomData` pins it
/// `!Send` to match the "single-thread RHI" contract (mirroring the encoder's `!Send`).
pub struct BoundAccelStruct {
    /// The `VkAccelerationStructureKHR` handle; destroyed by `destroy_acceleration_structure`.
    pub(crate) handle: VkAccelerationStructureKHR,
    /// The backing buffer the AS lives in — it MUST outlive this structure (the caller owns
    /// and frees it; kept here for lifetime bookkeeping, not freed by `destroy`). R2a-1 stores
    /// it for the R2a-2 build path (which pairs each AS with the buffer that must not be
    /// freed before it); no R2a-1 consumer reads it yet.
    #[allow(dead_code)]
    pub(crate) buffer: VkBuffer,
    /// The cached device address (`0` until queried via
    /// [`VulkanContext::accel_device_address`]); a TLAS instance references a BLAS by this.
    pub(crate) device_address: u64,
    /// Whether this is a BLAS or a TLAS.
    pub(crate) kind: AsKind,
    /// Pins the type `!Send + !Sync` (single-thread RHI, §5.3): a raw pointer marker with no
    /// auto-trait impls, mirroring the encoder/queue discipline.
    pub(crate) _not_send: core::marker::PhantomData<*const ()>,
}

impl BoundAccelStruct {
    /// The AS device address (a TLAS instance's `accelerationStructureReference`).
    #[inline]
    pub fn device_address(&self) -> u64 {
        self.device_address
    }

    /// The AS level (BLAS/TLAS).
    #[inline]
    pub fn kind(&self) -> AsKind {
        self.kind
    }
}

/// Maps the agnostic [`AsKind`] to a `VkAccelerationStructureTypeKHR`.
#[inline]
fn vk_as_type(kind: AsKind) -> i32 {
    match kind {
        AsKind::Blas => VK_ACCELERATION_STRUCTURE_TYPE_BOTTOM_LEVEL_KHR,
        AsKind::Tlas => VK_ACCELERATION_STRUCTURE_TYPE_TOP_LEVEL_KHR,
    }
}

/// Fills the `VkAccelerationStructureGeometryKHR` for one build entry from the agnostic
/// [`AsGeometryDesc`] (triangles for a BLAS, instances for a TLAS). Pure — no FFI.
fn fill_geometry(kind: AsKind, g: &AsGeometryDesc) -> VkAccelerationStructureGeometryKHR {
    match kind {
        AsKind::Blas => {
            let triangles = VkAccelerationStructureGeometryTrianglesDataKHR {
                s_type: ST_ACCELERATION_STRUCTURE_GEOMETRY_TRIANGLES_DATA_KHR,
                p_next: ptr::null(),
                vertex_format: VK_FORMAT_R32G32B32_SFLOAT,
                vertex_data: g.vertex_data,
                vertex_stride: g.vertex_stride,
                max_vertex: g.max_vertex,
                // R2a-3: the BLAS reads the mesh's EXISTING index buffer at its real width
                // (chosen by the O3 crossover), so a `Uint16` mesh needs no duplicate u32 buffer.
                index_type: match g.index_type {
                    AsIndexType::Uint16 => VK_INDEX_TYPE_UINT16,
                    AsIndexType::Uint32 => VK_INDEX_TYPE_UINT32,
                },
                index_data: g.index_data,
                transform_data: 0,
            };
            VkAccelerationStructureGeometryKHR {
                s_type: ST_ACCELERATION_STRUCTURE_GEOMETRY_KHR,
                p_next: ptr::null(),
                geometry_type: VK_GEOMETRY_TYPE_TRIANGLES_KHR,
                geometry: VkAccelerationStructureGeometryDataKHR {
                    triangles: core::mem::ManuallyDrop::new(triangles),
                },
                flags: VK_GEOMETRY_OPAQUE_BIT_KHR,
            }
        }
        AsKind::Tlas => {
            let instances = VkAccelerationStructureGeometryInstancesDataKHR {
                s_type: ST_ACCELERATION_STRUCTURE_GEOMETRY_INSTANCES_DATA_KHR,
                p_next: ptr::null(),
                array_of_pointers: 0,
                data: g.vertex_data,
            };
            VkAccelerationStructureGeometryKHR {
                s_type: ST_ACCELERATION_STRUCTURE_GEOMETRY_KHR,
                p_next: ptr::null(),
                geometry_type: VK_GEOMETRY_TYPE_INSTANCES_KHR,
                geometry: VkAccelerationStructureGeometryDataKHR {
                    instances: core::mem::ManuallyDrop::new(instances),
                },
                flags: VK_GEOMETRY_OPAQUE_BIT_KHR,
            }
        }
    }
}

impl VulkanContext {
    /// The resolved AS command table, or an `Unsupported` error when ray query is off (the
    /// device did not enable the RT extensions, or `hwrt` gated the load away). Every AS
    /// verb funnels through this so a non-RT device cannot dereference an absent table.
    fn accel_fns(&self) -> Result<&AccelFns, VulkanError> {
        self.accel_fns_opt()
            .ok_or(VulkanError::Unsupported("acceleration structure (ray query not enabled)"))
    }

    /// The scratch-address alignment (`minAccelerationStructureScratchOffsetAlignment`) the
    /// caller must align a build's scratch buffer to. `0` when ray query is off.
    #[inline]
    pub fn as_scratch_align(&self) -> u64 {
        self.device_caps().as_scratch_align
    }

    /// HW-RT rung R2a-1: `vkGetAccelerationStructureBuildSizesKHR` — the host-side size
    /// query (no GPU work). Fills a one-geometry `VkAccelerationStructureBuildGeometryInfoKHR`
    /// (dst/scratch left null — the size query does not read them) and returns the AS +
    /// scratch sizes.
    pub(crate) fn build_sizes(
        &self,
        kind: AsKind,
        geometry: &AsGeometryDesc,
    ) -> Result<AsBuildSizes, VulkanError> {
        let fns = self.accel_fns()?;
        let geom = fill_geometry(kind, geometry);
        let build_info = VkAccelerationStructureBuildGeometryInfoKHR {
            s_type: ST_ACCELERATION_STRUCTURE_BUILD_GEOMETRY_INFO_KHR,
            p_next: ptr::null(),
            ty: vk_as_type(kind),
            flags: VK_BUILD_ACCELERATION_STRUCTURE_PREFER_FAST_TRACE_BIT_KHR,
            mode: VK_BUILD_ACCELERATION_STRUCTURE_MODE_BUILD_KHR,
            src_acceleration_structure: VkAccelerationStructureKHR::NULL,
            dst_acceleration_structure: VkAccelerationStructureKHR::NULL,
            geometry_count: 1,
            p_geometries: &geom,
            pp_geometries: ptr::null(),
            scratch_data: 0,
        };
        let max_primitive_counts: [u32; 1] = [geometry.primitive_count];
        let mut sizes = VkAccelerationStructureBuildSizesInfoKHR {
            s_type: ST_ACCELERATION_STRUCTURE_BUILD_SIZES_INFO_KHR,
            ..Default::default()
        };
        // SAFETY: `device` is live + ray query enabled (`accel_fns` ok); `build_info` names
        // the live single-element `geom` local (outlives the call), `max_primitive_counts`
        // is a live 1-element array matching `geometry_count`, and `&mut sizes` is a valid
        // out-pointer. A DEVICE build-type size query issues NO GPU work — it only reads the
        // geometry counts + fills `sizes`.
        unsafe {
            (fns.get_build_sizes)(
                self.device(),
                VK_ACCELERATION_STRUCTURE_BUILD_TYPE_DEVICE_KHR,
                &build_info,
                max_primitive_counts.as_ptr(),
                &mut sizes,
            );
        }
        Ok(AsBuildSizes {
            as_size: sizes.acceleration_structure_size,
            build_scratch: sizes.build_scratch_size,
            update_scratch: sizes.update_scratch_size,
        })
    }

    /// HW-RT rung R2a-1: `vkCreateAccelerationStructureKHR` — creates an AS of `size` bytes
    /// over the caller's backing buffer (named by its device address; the create-info takes
    /// the `VkBuffer` handle + a zero offset). The buffer MUST have usage
    /// `ACCELERATION_STRUCTURE_STORAGE_KHR | SHADER_DEVICE_ADDRESS` and outlive the AS.
    pub(crate) fn create_accel(
        &self,
        kind: AsKind,
        buffer: VkBuffer,
        size: u64,
    ) -> Result<BoundAccelStruct, VulkanError> {
        let fns = self.accel_fns()?;
        let create_info = VkAccelerationStructureCreateInfoKHR {
            s_type: ST_ACCELERATION_STRUCTURE_CREATE_INFO_KHR,
            p_next: ptr::null(),
            create_flags: 0,
            buffer,
            offset: 0,
            size,
            ty: vk_as_type(kind),
            device_address: 0,
        };
        let mut handle = VkAccelerationStructureKHR::NULL;
        // SAFETY: `device` is live + ray query enabled; `create_info` is fully initialized
        // over the caller's live `buffer` (correct usage bits are the caller's contract);
        // `&mut handle` is a valid out-pointer; NULL allocator picks the default.
        let raw = unsafe {
            (fns.create)(self.device(), &create_info, ptr::null(), &mut handle)
        };
        let result = VkResult::from_raw(raw);
        if !result.is_success() {
            return Err(VulkanError::Vk("vkCreateAccelerationStructureKHR", result));
        }
        let mut bound = BoundAccelStruct {
            handle,
            buffer,
            device_address: 0,
            kind,
            _not_send: core::marker::PhantomData,
        };
        // The AS device address is valid right after `vkCreateAccelerationStructureKHR`
        // (independent of the build), so populate it here — every `BoundAccelStruct` carries its
        // real address (the R2a-3 `PersistentTlas` + R2a-4 tracing read it via `device_address()`;
        // R2a-2's `build_blas`/`build_tlas` re-query it into their own wrapper field, harmlessly).
        bound.device_address = self.accel_device_address(&bound)?;
        Ok(bound)
    }

    /// HW-RT rung R2a-1: `vkGetAccelerationStructureDeviceAddressKHR` — the AS device
    /// address (a TLAS instance's `accelerationStructureReference`). A zero address signals a
    /// mis-flagged backing buffer; the caller must fail fast on `0`.
    pub(crate) fn accel_device_address(
        &self,
        accel: &BoundAccelStruct,
    ) -> Result<u64, VulkanError> {
        let fns = self.accel_fns()?;
        let info = VkAccelerationStructureDeviceAddressInfoKHR {
            s_type: ST_ACCELERATION_STRUCTURE_DEVICE_ADDRESS_INFO_KHR,
            p_next: ptr::null(),
            acceleration_structure: accel.handle,
        };
        // SAFETY: `device` is live + ray query enabled; `info` names the live `accel.handle`
        // (a built AS the caller owns); the call reads it + returns the address.
        let addr = unsafe { (fns.get_device_address)(self.device(), &info) };
        Ok(addr)
    }

    /// HW-RT rung R2a-1: `vkGetBufferDeviceAddress` — a buffer's device address (to feed
    /// vertex/index/instance/scratch addresses into a build). The buffer MUST have
    /// `SHADER_DEVICE_ADDRESS` usage + device-address-flagged backing memory.
    pub(crate) fn buffer_device_address(&self, buffer: VkBuffer) -> Result<u64, VulkanError> {
        let fns = self.accel_fns()?;
        let info = VkBufferDeviceAddressInfo {
            s_type: ST_BUFFER_DEVICE_ADDRESS_INFO,
            p_next: ptr::null(),
            buffer,
        };
        // SAFETY: `device` is live + ray query enabled; `info` names the live `buffer` (the
        // correct usage + memory alloc flag are the caller's contract); the call reads it +
        // returns the device address.
        let addr = unsafe { (fns.get_buffer_device_address)(self.device(), &info) };
        Ok(addr)
    }

    /// HW-RT rung R2a-1: `vkDestroyAccelerationStructureKHR`.
    ///
    /// # Safety
    /// `accel.handle` was created on this device, no submission building/tracing it is
    /// pending (caller: fence-waited or `wait_idle`'d), and it is destroyed once (by-value).
    /// Its backing buffer must still be live at this call.
    pub(crate) unsafe fn destroy_accel(&self, accel: BoundAccelStruct) {
        // SAFETY: ray query was enabled when the AS was created, so `accel_fns_opt` is `Some`
        // for this context's lifetime; deref is valid. `accel.handle` is destroyed exactly
        // once (by-value move), on the device it was created on.
        if let Some(fns) = self.accel_fns_opt() {
            // SAFETY: caller contract above — no pending GPU use, single destroy, live device.
            unsafe { (fns.destroy)(self.device(), accel.handle, ptr::null()) };
        }
    }
}

/// Records `vkCmdBuildAccelerationStructuresKHR` for `entries` into `command_buffer` (HW-RT
/// rung R2a-1). One `VkAccelerationStructureBuildGeometryInfoKHR` + one range-info per entry;
/// `dest[i]` is the AS entry `i` builds into. Called by the encoder's trait override.
///
/// # Safety
/// `command_buffer` is recording; `fns` is the live device's AS table; every device address
/// in `entries` (vertex/index/instance/scratch) + each `dest[i].handle` is a live,
/// correctly-flagged resource the caller pre-created; the scratch address is aligned to
/// `as_scratch_align` (caller contract). `entries.len() == dest.len()`.
pub(crate) unsafe fn cmd_build_acceleration_structures(
    fns: &AccelFns,
    command_buffer: VkCommandBuffer,
    entries: &[AsBuildEntry],
    dest: &[&BoundAccelStruct],
) {
    debug_assert_eq!(
        entries.len(),
        dest.len(),
        "invariant: one destination AS per build entry"
    );
    let n = entries.len();
    // The per-frame R2a-3 TLAS build (and every R2a-2 caller) records EXACTLY ONE entry, so the
    // common path builds the per-entry structures into FIXED STACK arrays — ZERO heap allocation
    // on the hot per-frame path. A larger batch (never on the per-frame path) falls back to a heap
    // `Vec` via the `#[cold]` slow path.
    if n <= AS_BUILD_INLINE_CAP {
        // An array of `MaybeUninit` needs NO initialization (each element is trivially valid
        // uninitialized memory), so `[const { MaybeUninit::uninit() }; N]` is safe to construct.
        let mut geoms: [MaybeUninit<VkAccelerationStructureGeometryKHR>; AS_BUILD_INLINE_CAP] =
            [const { MaybeUninit::uninit() }; AS_BUILD_INLINE_CAP];
        let mut ranges: [MaybeUninit<VkAccelerationStructureBuildRangeInfoKHR>; AS_BUILD_INLINE_CAP] =
            [const { MaybeUninit::uninit() }; AS_BUILD_INLINE_CAP];
        let mut build_infos: [MaybeUninit<VkAccelerationStructureBuildGeometryInfoKHR>;
            AS_BUILD_INLINE_CAP] = [const { MaybeUninit::uninit() }; AS_BUILD_INLINE_CAP];
        let mut range_ptrs: [MaybeUninit<*const VkAccelerationStructureBuildRangeInfoKHR>;
            AS_BUILD_INLINE_CAP] = [const { MaybeUninit::uninit() }; AS_BUILD_INLINE_CAP];
        // SAFETY: `n <= AS_BUILD_INLINE_CAP`, so the fixed stack arrays hold every entry;
        // `record_build_arrays` populates `[0..n)` before use and only reads `[0..n)`, upholding
        // the `p_geometries` stability + FFI invariants (its own SAFETY). Caller contract: every
        // device address + `dest[i].handle` is a live, correctly-flagged resource.
        unsafe {
            record_build_arrays(
                fns,
                command_buffer,
                entries,
                dest,
                &mut geoms,
                &mut ranges,
                &mut build_infos,
                &mut range_ptrs,
            );
        }
    } else {
        // SAFETY: same contract as the inline path — the `#[cold]` fallback allocates heap arrays
        // sized to `n` and records the identical build.
        unsafe { cmd_build_acceleration_structures_heap(fns, command_buffer, entries, dest) };
    }
}

/// The inline fast-path capacity of [`cmd_build_acceleration_structures`]: builds up to this many
/// AS entries into fixed STACK arrays (zero heap alloc). Every current caller records ONE entry;
/// the small margin keeps a modest batch alloc-free too.
const AS_BUILD_INLINE_CAP: usize = 4;

/// Populates the caller-supplied per-entry arrays (`[0..entries.len())`) and records the build.
/// Factored out so the inline (stack) and `#[cold]` heap paths share the FFI call — the arrays'
/// storage differs, the record is identical.
///
/// # Safety
/// The four arrays have length `>= entries.len()`; `command_buffer` is recording; `fns` is the
/// live AS table; `entries.len() == dest.len()`; every device address in `entries` + each
/// `dest[i].handle` is a live, correctly-flagged resource (caller contract).
#[allow(clippy::too_many_arguments)]
unsafe fn record_build_arrays(
    fns: &AccelFns,
    command_buffer: VkCommandBuffer,
    entries: &[AsBuildEntry],
    dest: &[&BoundAccelStruct],
    geoms: &mut [MaybeUninit<VkAccelerationStructureGeometryKHR>],
    ranges: &mut [MaybeUninit<VkAccelerationStructureBuildRangeInfoKHR>],
    build_infos: &mut [MaybeUninit<VkAccelerationStructureBuildGeometryInfoKHR>],
    range_ptrs: &mut [MaybeUninit<*const VkAccelerationStructureBuildRangeInfoKHR>],
) {
    let n = entries.len();
    // Fill geometry + range first — `geoms[i]` must be initialized before a build-info takes
    // `&geoms[i]`, and both must outlive the `cmd_build` call below (they do — same stack frame).
    for (i, e) in entries.iter().enumerate() {
        geoms[i].write(fill_geometry(e.kind, &e.geometry));
        ranges[i].write(VkAccelerationStructureBuildRangeInfoKHR {
            primitive_count: e.geometry.primitive_count,
            primitive_offset: 0,
            first_vertex: 0,
            transform_offset: 0,
        });
    }
    for (i, e) in entries.iter().enumerate() {
        // SAFETY: `geoms[i]`/`ranges[i]` were written above (`i < n`), so `.assume_init_ref()` /
        // taking `&` of the initialized value is sound; the pointer stays valid for the whole call.
        let geom_ptr = unsafe { geoms[i].assume_init_ref() } as *const _;
        build_infos[i].write(VkAccelerationStructureBuildGeometryInfoKHR {
            s_type: ST_ACCELERATION_STRUCTURE_BUILD_GEOMETRY_INFO_KHR,
            p_next: ptr::null(),
            ty: vk_as_type(e.kind),
            flags: VK_BUILD_ACCELERATION_STRUCTURE_PREFER_FAST_TRACE_BIT_KHR,
            mode: VK_BUILD_ACCELERATION_STRUCTURE_MODE_BUILD_KHR,
            src_acceleration_structure: VkAccelerationStructureKHR::NULL,
            dst_acceleration_structure: dest[i].handle,
            geometry_count: 1,
            p_geometries: geom_ptr,
            pp_geometries: ptr::null(),
            scratch_data: e.scratch_address,
        });
        // SAFETY: `ranges[i]` initialized above; the range pointer stays valid for the call.
        range_ptrs[i].write(unsafe { ranges[i].assume_init_ref() } as *const _);
    }

    // SAFETY: recording is open; `build_infos[0..n]`/`geoms[0..n]`/`ranges[0..n]`/`range_ptrs[0..n]`
    // are initialized live storage (outlive the call); each build-info's `dst_acceleration_structure`
    // is a live AS (`dest[i]`), its `p_geometries` points at the live `geoms[i]`, and `scratch_data`
    // + every geometry device address are correctly-flagged live resources (caller contract).
    // `info_count == n`; the two pointer arrays are read as `[0..n)`.
    unsafe {
        (fns.cmd_build)(
            command_buffer,
            n as u32,
            build_infos.as_ptr().cast::<VkAccelerationStructureBuildGeometryInfoKHR>(),
            range_ptrs.as_ptr().cast::<*const VkAccelerationStructureBuildRangeInfoKHR>(),
        );
    }
}

/// The `#[cold]` heap fallback of [`cmd_build_acceleration_structures`] for a batch larger than
/// [`AS_BUILD_INLINE_CAP`] (never the per-frame path). Allocates `Vec`-backed arrays, then shares
/// the record via [`record_build_arrays`].
///
/// # Safety
/// Identical contract to [`cmd_build_acceleration_structures`].
#[cold]
#[inline(never)]
unsafe fn cmd_build_acceleration_structures_heap(
    fns: &AccelFns,
    command_buffer: VkCommandBuffer,
    entries: &[AsBuildEntry],
    dest: &[&BoundAccelStruct],
) {
    let n = entries.len();
    let mut geoms: Vec<MaybeUninit<VkAccelerationStructureGeometryKHR>> = Vec::with_capacity(n);
    let mut ranges: Vec<MaybeUninit<VkAccelerationStructureBuildRangeInfoKHR>> =
        Vec::with_capacity(n);
    let mut build_infos: Vec<MaybeUninit<VkAccelerationStructureBuildGeometryInfoKHR>> =
        Vec::with_capacity(n);
    let mut range_ptrs: Vec<MaybeUninit<*const VkAccelerationStructureBuildRangeInfoKHR>> =
        Vec::with_capacity(n);
    geoms.resize_with(n, MaybeUninit::uninit);
    ranges.resize_with(n, MaybeUninit::uninit);
    build_infos.resize_with(n, MaybeUninit::uninit);
    range_ptrs.resize_with(n, MaybeUninit::uninit);
    // SAFETY: the four Vecs are length `n == entries.len()`; the caller contract holds.
    unsafe {
        record_build_arrays(
            fns,
            command_buffer,
            entries,
            dest,
            &mut geoms,
            &mut ranges,
            &mut build_infos,
            &mut range_ptrs,
        );
    }
}

/// Records the `ACCELERATION_STRUCTURE_BUILD → *` (write→read) global memory barrier via
/// `vkCmdPipelineBarrier` (HW-RT rung R2a-1). A single `VkMemoryBarrier` with the AS
/// write→read access masks between the build stage and the (compute-shader) trace stage.
///
/// # Safety
/// `command_buffer` is recording; `fns` is the live device's core command table. Issues one
/// pipeline barrier with an AS-write source + AS-read destination — no resource is touched
/// beyond the execution/memory dependency.
pub(crate) unsafe fn cmd_acceleration_structure_barrier(
    fns: &DeviceFns,
    command_buffer: VkCommandBuffer,
) {
    let barrier = VkMemoryBarrier {
        s_type: ST_MEMORY_BARRIER,
        _pad: 0,
        p_next: ptr::null(),
        src_access_mask: VK_ACCESS_ACCELERATION_STRUCTURE_WRITE_BIT_KHR,
        dst_access_mask: VK_ACCESS_ACCELERATION_STRUCTURE_READ_BIT_KHR,
    };
    // SAFETY: recording is open; `&barrier` is a live 1-element `VkMemoryBarrier` local
    // (outlives the call); the buffer/image barrier counts are 0 with null arrays. The
    // AS-build source stage → compute-shader destination stage orders the build before the
    // `rayQuery` trace read (R2a-4).
    unsafe {
        (fns.cmd_pipeline_barrier)(
            command_buffer,
            VK_PIPELINE_STAGE_ACCELERATION_STRUCTURE_BUILD_BIT_KHR,
            VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
            0,
            1,
            (&barrier as *const VkMemoryBarrier).cast(),
            0,
            ptr::null(),
            0,
            ptr::null(),
        );
    }
}

/// Reinterprets an [`InstanceModelCol`](boyko_render)-shaped 3×4 row-major affine + the
/// per-instance lanes into a [`VkAccelerationStructureInstanceKHR`] (HW-RT rung R2a-1). The
/// `transform` is a direct 48-byte copy (row-major ↔ `float[3][4]`, NO transpose — asserted
/// in `abi_guard`). Provided here so the R2a-3 TLAS fill has a single packing point; unused
/// in R2a-1 (no TLAS is built yet).
#[inline]
pub fn pack_instance(
    transform: [[f32; 4]; 3],
    custom_index: u32,
    mask: u8,
    sbt_offset: u32,
    flags: u8,
    blas_device_address: u64,
) -> VkAccelerationStructureInstanceKHR {
    VkAccelerationStructureInstanceKHR {
        transform,
        instance_custom_index_and_mask: VkAccelerationStructureInstanceKHR::pack_index_mask(
            custom_index,
            mask,
        ),
        instance_sbt_offset_and_flags: VkAccelerationStructureInstanceKHR::pack_sbt_offset_flags(
            sbt_offset,
            flags,
        ),
        acceleration_structure_reference: blas_device_address,
    }
}
