//! HW-RT rung R2a-1 — the raw-FFI acceleration-structure Vulkan surface.
//!
//! # Discipline (mirrors [`crate::ffi`] verbatim)
//!
//! Hand-declared `#[repr(C)]` structs + PFN typedefs for `VK_KHR_acceleration_structure`
//! + `VK_KHR_ray_query` + the Vulkan 1.2-core `bufferDeviceAddress` feature/props, with
//! per-struct `size_of`/`align_of`/`offset_of!` const-asserts (see [`crate::abi_guard`]).
//! No third-party crates; the commands resolve at runtime through
//! `vkGetDeviceProcAddr` / `vkGetInstanceProcAddr` (this module only defines typedefs +
//! POD layouts, never links a `vk*` symbol). Sourced from the plan's
//! "Research-confirmed constants" (cross-validated vs Khronos + nvpro/Vulkan-Samples).
//!
//! # Gating
//!
//! The whole module is `#[cfg(feature = "hwrt")]` (the `pub mod` in `lib.rs`): with
//! `hwrt` OFF it compiles to nothing, so the default/golden build carries zero RT FFI
//! and is textually the pre-R2a code.
//!
//! # R2a-1 scope
//!
//! This rung builds NO acceleration structure and traces nothing — it only DECLARES the
//! FFI. The unions `VkDeviceOrHostAddressConstKHR` / `VkDeviceOrHostAddressKHR` are
//! modelled as a bare `u64` (the DEVICE-ADDRESS variant only — a GPU build never uses the
//! host-address arm), matching how the rest of the crate models Vulkan surfaces minimally.

#![allow(non_snake_case)]
#![allow(non_camel_case_types)]

use core::ffi::c_void;
use core::mem::{align_of, offset_of, size_of};

use crate::ffi::{VkBool32, VkBuffer, VkCommandBuffer, VkDevice, VkDeviceSize, VkFlags};

// ---------------------------------------------------------------------------
// Handle.
// ---------------------------------------------------------------------------

/// `VkAccelerationStructureKHR` — a non-dispatchable handle (a 64-bit opaque token).
///
/// `#[repr(transparent)]` over `u64` per the spec's non-dispatchable-handle guarantee,
/// mirroring the [`crate::ffi`] `non_dispatchable_handle!` macro shape (kept local so the
/// gated module owns its own type without touching the ungated macro export).
#[repr(transparent)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct VkAccelerationStructureKHR(pub u64);

impl VkAccelerationStructureKHR {
    /// The Vulkan null handle (`VK_NULL_HANDLE` == 0).
    pub const NULL: Self = Self(0);

    /// Whether this is the null handle.
    #[inline]
    pub fn is_null(self) -> bool {
        self.0 == 0
    }
}

// ---------------------------------------------------------------------------
// sType / enum constants (numeric values confirmed by the plan's RT researcher).
// ---------------------------------------------------------------------------

// The RT sTypes are NON-core `VkStructureType` values not named by the ungated
// `crate::ffi::VkStructureType` enum. Rather than perturb that enum (which must stay
// textually R1 for byte-identity), the accel structs type their `s_type` as a plain
// `i32` and set it from these constants — the same `i32`-discriminant discipline the
// crate already uses for `VkFormat`/`VkImageLayout`.

/// `VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_BUFFER_DEVICE_ADDRESS_FEATURES` — Vulkan 1.2 core.
pub const ST_PHYSICAL_DEVICE_BUFFER_DEVICE_ADDRESS_FEATURES: i32 = 1_000_257_000;
/// `VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_ACCELERATION_STRUCTURE_FEATURES_KHR` (vulkan_core.h:695).
pub const ST_PHYSICAL_DEVICE_ACCELERATION_STRUCTURE_FEATURES_KHR: i32 = 1_000_150_013;
/// `VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_RAY_QUERY_FEATURES_KHR`.
pub const ST_PHYSICAL_DEVICE_RAY_QUERY_FEATURES_KHR: i32 = 1_000_348_013;
/// `VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_ACCELERATION_STRUCTURE_PROPERTIES_KHR` (vulkan_core.h:696).
pub const ST_PHYSICAL_DEVICE_ACCELERATION_STRUCTURE_PROPERTIES_KHR: i32 = 1_000_150_014;
/// `VK_STRUCTURE_TYPE_ACCELERATION_STRUCTURE_GEOMETRY_KHR`.
pub const ST_ACCELERATION_STRUCTURE_GEOMETRY_KHR: i32 = 1_000_150_006;
/// `VK_STRUCTURE_TYPE_ACCELERATION_STRUCTURE_GEOMETRY_TRIANGLES_DATA_KHR`.
pub const ST_ACCELERATION_STRUCTURE_GEOMETRY_TRIANGLES_DATA_KHR: i32 = 1_000_150_005;
/// `VK_STRUCTURE_TYPE_ACCELERATION_STRUCTURE_GEOMETRY_INSTANCES_DATA_KHR`.
pub const ST_ACCELERATION_STRUCTURE_GEOMETRY_INSTANCES_DATA_KHR: i32 = 1_000_150_004;
/// `VK_STRUCTURE_TYPE_ACCELERATION_STRUCTURE_BUILD_GEOMETRY_INFO_KHR` (vulkan_core.h:685).
/// (Was 1000150007 pre-fix — which is `WRITE_DESCRIPTOR_SET_ACCELERATION_STRUCTURE_KHR`, a live
/// collision the driver would mis-parse; caught by the R2a-1 FFI review.)
pub const ST_ACCELERATION_STRUCTURE_BUILD_GEOMETRY_INFO_KHR: i32 = 1_000_150_000;
/// `VK_STRUCTURE_TYPE_ACCELERATION_STRUCTURE_CREATE_INFO_KHR` (vulkan_core.h:697).
pub const ST_ACCELERATION_STRUCTURE_CREATE_INFO_KHR: i32 = 1_000_150_017;
/// `VK_STRUCTURE_TYPE_ACCELERATION_STRUCTURE_BUILD_SIZES_INFO_KHR`.
pub const ST_ACCELERATION_STRUCTURE_BUILD_SIZES_INFO_KHR: i32 = 1_000_150_020;
/// `VK_STRUCTURE_TYPE_ACCELERATION_STRUCTURE_DEVICE_ADDRESS_INFO_KHR`.
pub const ST_ACCELERATION_STRUCTURE_DEVICE_ADDRESS_INFO_KHR: i32 = 1_000_150_002;
/// `VK_STRUCTURE_TYPE_BUFFER_DEVICE_ADDRESS_INFO` — Vulkan 1.2 core.
pub const ST_BUFFER_DEVICE_ADDRESS_INFO: i32 = 1_000_244_001;

/// `VkAccelerationStructureTypeKHR::VK_ACCELERATION_STRUCTURE_TYPE_TOP_LEVEL_KHR`.
pub const VK_ACCELERATION_STRUCTURE_TYPE_TOP_LEVEL_KHR: i32 = 0;
/// `VkAccelerationStructureTypeKHR::VK_ACCELERATION_STRUCTURE_TYPE_BOTTOM_LEVEL_KHR`.
pub const VK_ACCELERATION_STRUCTURE_TYPE_BOTTOM_LEVEL_KHR: i32 = 1;

/// `VkGeometryTypeKHR::VK_GEOMETRY_TYPE_TRIANGLES_KHR`.
pub const VK_GEOMETRY_TYPE_TRIANGLES_KHR: i32 = 0;
/// `VkGeometryTypeKHR::VK_GEOMETRY_TYPE_INSTANCES_KHR` (vulkan_core.h — value `2`; `1` is
/// `VK_GEOMETRY_TYPE_AABBS_KHR`, so the pre-fix `1` would make the TLAS mis-read its instance
/// array as AABB data. Caught by the R2a-1 FFI review).
pub const VK_GEOMETRY_TYPE_INSTANCES_KHR: i32 = 2;

// Value guards for the RT `sType`/enum magic numbers `abi_guard` cannot check (it pins layout,
// not values). A wrong value is a SILENT device-lost on a no-validation box, so pin each to its
// `vulkan_core.h` literal — a future transcription slip on a `const` above fails THIS build,
// not the GPU (the R2a-1 review caught 4 sTypes + this enum wrong pre-fix).
const _: () = assert!(ST_PHYSICAL_DEVICE_ACCELERATION_STRUCTURE_FEATURES_KHR == 1_000_150_013);
const _: () = assert!(ST_PHYSICAL_DEVICE_ACCELERATION_STRUCTURE_PROPERTIES_KHR == 1_000_150_014);
const _: () = assert!(ST_PHYSICAL_DEVICE_RAY_QUERY_FEATURES_KHR == 1_000_348_013);
const _: () = assert!(ST_ACCELERATION_STRUCTURE_GEOMETRY_KHR == 1_000_150_006);
const _: () = assert!(ST_ACCELERATION_STRUCTURE_GEOMETRY_TRIANGLES_DATA_KHR == 1_000_150_005);
const _: () = assert!(ST_ACCELERATION_STRUCTURE_GEOMETRY_INSTANCES_DATA_KHR == 1_000_150_004);
const _: () = assert!(ST_ACCELERATION_STRUCTURE_BUILD_GEOMETRY_INFO_KHR == 1_000_150_000);
const _: () = assert!(ST_ACCELERATION_STRUCTURE_CREATE_INFO_KHR == 1_000_150_017);
const _: () = assert!(ST_ACCELERATION_STRUCTURE_BUILD_SIZES_INFO_KHR == 1_000_150_020);
const _: () = assert!(ST_ACCELERATION_STRUCTURE_DEVICE_ADDRESS_INFO_KHR == 1_000_150_002);
const _: () = assert!(VK_GEOMETRY_TYPE_TRIANGLES_KHR == 0 && VK_GEOMETRY_TYPE_INSTANCES_KHR == 2);
const _: () = assert!(
    VK_ACCELERATION_STRUCTURE_TYPE_TOP_LEVEL_KHR == 0
        && VK_ACCELERATION_STRUCTURE_TYPE_BOTTOM_LEVEL_KHR == 1
);

/// `VkGeometryFlagBitsKHR::VK_GEOMETRY_OPAQUE_BIT_KHR` — the geometry never invokes an
/// any-hit shader (all triangles opaque); the `rayQuery` shadow at R2a-4 traces with
/// `FORCE_OPAQUE`, so this is the geometry-side match.
pub const VK_GEOMETRY_OPAQUE_BIT_KHR: VkFlags = 0x0000_0001;

/// `VkBuildAccelerationStructureModeKHR::VK_BUILD_ACCELERATION_STRUCTURE_MODE_BUILD_KHR`
/// — a from-scratch build (R2a rebuilds every frame; the `UPDATE` mode is R6).
pub const VK_BUILD_ACCELERATION_STRUCTURE_MODE_BUILD_KHR: i32 = 0;

/// `VkBuildAccelerationStructureFlagBitsKHR::VK_BUILD_ACCELERATION_STRUCTURE_PREFER_FAST_TRACE_BIT_KHR`
/// — prioritize trace performance over build time (mesh RT shadows: static-ish BLAS +
/// per-frame TLAS both favor fast trace).
pub const VK_BUILD_ACCELERATION_STRUCTURE_PREFER_FAST_TRACE_BIT_KHR: VkFlags = 0x0000_0004;

/// `VkAccelerationStructureBuildTypeKHR::VK_ACCELERATION_STRUCTURE_BUILD_TYPE_DEVICE_KHR`
/// — a device (GPU) build/size-query, not a host build.
pub const VK_ACCELERATION_STRUCTURE_BUILD_TYPE_DEVICE_KHR: i32 = 1;

/// `VkIndexType::VK_INDEX_TYPE_UINT16` (BLAS triangle index type for a `Uint16` mesh, R2a-3;
/// value matches `crate::ffi::VK_INDEX_TYPE_UINT16`). The BLAS reads the mesh's real-width
/// index buffer, so a `Uint16` mesh needs no duplicate `u32` buffer.
pub const VK_INDEX_TYPE_UINT16: i32 = 0;

/// `VkIndexType::VK_INDEX_TYPE_UINT32` (BLAS triangle index type; re-declared here for the
/// gated module's self-containment — value matches `crate::ffi::VK_INDEX_TYPE_UINT32`).
pub const VK_INDEX_TYPE_UINT32: i32 = 1;

/// `VkFormat::VK_FORMAT_R32G32B32_SFLOAT` (BLAS triangle vertex position format; value
/// matches `crate::ffi::VK_FORMAT_R32G32B32_SFLOAT`).
pub const VK_FORMAT_R32G32B32_SFLOAT: i32 = 106;

// ---------------------------------------------------------------------------
// Device-or-host address unions (DEVICE-ADDRESS variant only — a GPU build).
// ---------------------------------------------------------------------------

/// `VkDeviceOrHostAddressConstKHR` — a C union of `{ VkDeviceAddress deviceAddress;
/// const void* hostAddress; }`. On x86_64 both arms are 8 bytes, so the union is a bare
/// `u64`; a GPU build uses ONLY the device-address arm. Modelled as a `#[repr(transparent)]`
/// `u64` (NOT a Rust `union`) because the whole crate only ever sets the device address.
pub type VkDeviceOrHostAddressConstKHR = u64;
/// `VkDeviceOrHostAddressKHR` — the non-const twin (scratch address). Same modelling.
pub type VkDeviceOrHostAddressKHR = u64;

// ---------------------------------------------------------------------------
// Feature / property query structs (driver-written through the p_next chain).
// ---------------------------------------------------------------------------

/// `VkPhysicalDeviceBufferDeviceAddressFeatures` (Vulkan 1.2 core). Chained into
/// `VkPhysicalDeviceFeatures2` to READ / ENABLE `bufferDeviceAddress` (the standalone
/// struct; the equivalent bit also lives in `VkPhysicalDeviceVulkan12Features`).
#[repr(C)]
pub struct VkPhysicalDeviceBufferDeviceAddressFeatures {
    pub s_type: i32,
    pub p_next: *mut c_void,
    pub buffer_device_address: VkBool32,
    pub buffer_device_address_capture_replay: VkBool32,
    pub buffer_device_address_multi_device: VkBool32,
}

/// `VkPhysicalDeviceAccelerationStructureFeaturesKHR`. Chained to READ / ENABLE
/// `accelerationStructure` (the other bools stay `FALSE`).
#[repr(C)]
pub struct VkPhysicalDeviceAccelerationStructureFeaturesKHR {
    pub s_type: i32,
    pub p_next: *mut c_void,
    pub acceleration_structure: VkBool32,
    pub acceleration_structure_capture_replay: VkBool32,
    pub acceleration_structure_indirect_build: VkBool32,
    pub acceleration_structure_host_commands: VkBool32,
    pub descriptor_binding_acceleration_structure_update_after_bind: VkBool32,
}

/// `VkPhysicalDeviceRayQueryFeaturesKHR`. Chained to READ / ENABLE `rayQuery`.
#[repr(C)]
pub struct VkPhysicalDeviceRayQueryFeaturesKHR {
    pub s_type: i32,
    pub p_next: *mut c_void,
    pub ray_query: VkBool32,
}

/// `VkPhysicalDeviceAccelerationStructurePropertiesKHR` — driver-written through
/// `vkGetPhysicalDeviceProperties2`. The R2a-1 caps query reads only
/// `min_acceleration_structure_scratch_offset_alignment` (=128 on Ampere/RTX 3060); the
/// leading `VkDeviceSize`s + trailing `u32`s are declared field-exact so the driver writes
/// every member it owns without reading past our footprint (a size/align guard pins the ABI).
#[repr(C)]
pub struct VkPhysicalDeviceAccelerationStructurePropertiesKHR {
    pub s_type: i32,
    pub p_next: *mut c_void,
    pub max_geometry_count: u64,
    pub max_instance_count: u64,
    pub max_primitive_count: u64,
    pub max_per_stage_descriptor_acceleration_structures: u32,
    pub max_per_stage_descriptor_update_after_bind_acceleration_structures: u32,
    pub max_descriptor_set_acceleration_structures: u32,
    pub max_descriptor_set_update_after_bind_acceleration_structures: u32,
    pub min_acceleration_structure_scratch_offset_alignment: u32,
}

// ---------------------------------------------------------------------------
// Geometry / build / create / size / address structs.
// ---------------------------------------------------------------------------

/// `VkAccelerationStructureGeometryTrianglesDataKHR` — one BLAS triangle-geometry input.
#[repr(C)]
pub struct VkAccelerationStructureGeometryTrianglesDataKHR {
    pub s_type: i32,
    pub p_next: *const c_void,
    /// `VkFormat` of a vertex position (`R32G32B32_SFLOAT`).
    pub vertex_format: i32,
    /// Device address of the vertex buffer.
    pub vertex_data: VkDeviceOrHostAddressConstKHR,
    pub vertex_stride: VkDeviceSize,
    /// The highest vertex index (`vertexCount - 1`).
    pub max_vertex: u32,
    /// `VkIndexType` (`UINT32`).
    pub index_type: i32,
    /// Device address of the index buffer.
    pub index_data: VkDeviceOrHostAddressConstKHR,
    /// Device address of an optional per-triangle transform (`0` = none).
    pub transform_data: VkDeviceOrHostAddressConstKHR,
}

/// `VkAccelerationStructureGeometryInstancesDataKHR` — a TLAS instance-array input.
#[repr(C)]
pub struct VkAccelerationStructureGeometryInstancesDataKHR {
    pub s_type: i32,
    pub p_next: *const c_void,
    /// `VkBool32` — whether `data` is an array of pointers to instances (`FALSE` = a
    /// packed `VkAccelerationStructureInstanceKHR[]`, which is what R2a-3 builds).
    pub array_of_pointers: VkBool32,
    /// Device address of the `VkAccelerationStructureInstanceKHR[]`.
    pub data: VkDeviceOrHostAddressConstKHR,
}

/// `VkAccelerationStructureGeometryDataKHR` — the C union
/// `{ triangles; aabbs; instances; }`. R2a uses only two arms: triangles (BLAS) +
/// instances (TLAS). Modelled as a true `#[repr(C)]` `union` so the enclosing
/// `VkAccelerationStructureGeometryKHR` has the exact C size (the union is as large as its
/// largest arm — the triangles struct); the backend writes exactly ONE arm per build,
/// selected by the geometry's `geometry_type` tag. `ManuallyDrop` wraps each arm because a
/// `union` field must not carry a `Drop` glue (both arms are trivially droppable POD, so
/// this is a formality the compiler requires).
#[repr(C)]
pub union VkAccelerationStructureGeometryDataKHR {
    pub triangles: core::mem::ManuallyDrop<VkAccelerationStructureGeometryTrianglesDataKHR>,
    pub instances: core::mem::ManuallyDrop<VkAccelerationStructureGeometryInstancesDataKHR>,
}

/// `VkAccelerationStructureGeometryKHR` — one geometry (triangles OR instances) + flags.
#[repr(C)]
pub struct VkAccelerationStructureGeometryKHR {
    pub s_type: i32,
    pub p_next: *const c_void,
    /// `VkGeometryTypeKHR` (`TRIANGLES` / `INSTANCES`).
    pub geometry_type: i32,
    pub geometry: VkAccelerationStructureGeometryDataKHR,
    /// `VkGeometryFlagsKHR` (`OPAQUE_BIT`).
    pub flags: VkFlags,
}

/// `VkAccelerationStructureBuildGeometryInfoKHR` — the top-level build descriptor.
#[repr(C)]
pub struct VkAccelerationStructureBuildGeometryInfoKHR {
    pub s_type: i32,
    pub p_next: *const c_void,
    /// `VkAccelerationStructureTypeKHR` (`TOP_LEVEL` / `BOTTOM_LEVEL`).
    pub ty: i32,
    /// `VkBuildAccelerationStructureFlagsKHR` (`PREFER_FAST_TRACE_BIT`).
    pub flags: VkFlags,
    /// `VkBuildAccelerationStructureModeKHR` (`BUILD`).
    pub mode: i32,
    /// Source AS for an update (`NULL` for a from-scratch build).
    pub src_acceleration_structure: VkAccelerationStructureKHR,
    /// Destination AS the build writes into.
    pub dst_acceleration_structure: VkAccelerationStructureKHR,
    pub geometry_count: u32,
    /// `const VkAccelerationStructureGeometryKHR*` — the geometry array.
    pub p_geometries: *const VkAccelerationStructureGeometryKHR,
    /// `const VkAccelerationStructureGeometryKHR* const*` — an array-of-pointers
    /// alternative to `p_geometries` (`NULL` when `p_geometries` is used).
    pub pp_geometries: *const *const VkAccelerationStructureGeometryKHR,
    /// Device address of the scratch buffer (aligned to the scratch-offset alignment).
    pub scratch_data: VkDeviceOrHostAddressKHR,
}

/// `VkAccelerationStructureBuildRangeInfoKHR` — the per-geometry primitive range.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct VkAccelerationStructureBuildRangeInfoKHR {
    /// Triangles (`indexCount / 3`, BLAS) or instances (TLAS).
    pub primitive_count: u32,
    pub primitive_offset: u32,
    pub first_vertex: u32,
    pub transform_offset: u32,
}

/// `VkAccelerationStructureBuildSizesInfoKHR` — driver-written by the size query.
/// `s_type`@0 + `_pad`@4 + `p_next`@8 + the three `VkDeviceSize`s @16/24/32 = 40 B (the
/// `_pad` makes the natural 8-byte alignment of `p_next` explicit for a `Default` init).
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct VkAccelerationStructureBuildSizesInfoKHR {
    pub s_type: i32,
    pub _pad: i32,
    pub p_next: u64,
    pub acceleration_structure_size: VkDeviceSize,
    pub update_scratch_size: VkDeviceSize,
    pub build_scratch_size: VkDeviceSize,
}

/// `VkAccelerationStructureCreateInfoKHR` — the AS creation descriptor over a backing
/// buffer region.
#[repr(C)]
pub struct VkAccelerationStructureCreateInfoKHR {
    pub s_type: i32,
    pub p_next: *const c_void,
    /// `VkAccelerationStructureCreateFlagsKHR` (`0` in R2a).
    pub create_flags: VkFlags,
    /// The backing buffer (usage `ACCELERATION_STRUCTURE_STORAGE_KHR | SHADER_DEVICE_ADDRESS`).
    pub buffer: VkBuffer,
    /// Byte offset within `buffer`.
    pub offset: VkDeviceSize,
    /// Byte size of the AS within `buffer` (`AsBuildSizes::as_size`).
    pub size: VkDeviceSize,
    /// `VkAccelerationStructureTypeKHR`.
    pub ty: i32,
    /// Device address for capture-replay (`0` in R2a).
    pub device_address: VkDeviceSize,
}

/// `VkAccelerationStructureDeviceAddressInfoKHR` — the device-address query input.
#[repr(C)]
pub struct VkAccelerationStructureDeviceAddressInfoKHR {
    pub s_type: i32,
    pub p_next: *const c_void,
    pub acceleration_structure: VkAccelerationStructureKHR,
}

/// `VkBufferDeviceAddressInfo` (Vulkan 1.2 core) — the buffer device-address query input.
#[repr(C)]
pub struct VkBufferDeviceAddressInfo {
    pub s_type: i32,
    pub p_next: *const c_void,
    pub buffer: VkBuffer,
}

/// `VkMemoryBarrier` (Vulkan 1.0 core) — a GLOBAL memory barrier (no buffer/image handle).
/// Used for the AS build→read dependency. Declared in the gated module (typed `s_type: i32`
/// with [`ST_MEMORY_BARRIER`]) so the ungated `crate::ffi::VkStructureType` enum stays R1.
#[repr(C)]
pub struct VkMemoryBarrier {
    pub s_type: i32,
    pub _pad: i32,
    pub p_next: *const c_void,
    /// `VkAccessFlags` source access mask.
    pub src_access_mask: VkFlags,
    /// `VkAccessFlags` destination access mask.
    pub dst_access_mask: VkFlags,
}

/// `VK_STRUCTURE_TYPE_MEMORY_BARRIER` — Vulkan 1.0 core (value 46).
pub const ST_MEMORY_BARRIER: i32 = 46;

const _: () = assert!(size_of::<VkMemoryBarrier>() == 24);
const _: () = assert!(align_of::<VkMemoryBarrier>() == 8);

/// The ABI-critical **`VkAccelerationStructureInstanceKHR`** — one TLAS instance
/// (64 B packed, align 8).
///
/// - `transform`@0 (48 B) — a `float[3][4]` ROW-MAJOR 3×4 affine (translation in
///   column 3 = `m[r][3]`). Byte-identical to `boyko_render::InstanceModelCol::rows`
///   (`[[f32;4];3]`, 48 B row-major) → the R2a-3 TLAS fill is a direct 48-byte memcpy,
///   NO transpose (the 48-B bridge is asserted in `abi_guard.rs`).
/// - `instance_custom_index_and_mask`@48 (`u32`) — packed `customIndex:24`(LSB) |
///   `mask:8`(MSB). Rust has no C bitfields: pack raw as
///   `(custom & 0x00FF_FFFF) | (mask << 24)` (see [`Self::pack_index_mask`]).
/// - `instance_sbt_offset_and_flags`@52 (`u32`) — packed `sbtOffset:24`(LSB) |
///   `flags:8`(MSB), `(sbt & 0x00FF_FFFF) | (flags << 24)`.
/// - `acceleration_structure_reference`@56 (`u64`) — the BLAS **device address** (NOT
///   the handle), from `get_acceleration_structure_device_address`.
///
/// `#[repr(C)]` gives the exact C layout; there is no natural padding (48 + 4 + 4 + 8 = 64,
/// align 8 from the trailing `u64`) — the `abi_guard` offset/size asserts pin it.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct VkAccelerationStructureInstanceKHR {
    /// The 3×4 ROW-MAJOR world affine (48 B) — a direct copy of `InstanceModelCol.rows`.
    pub transform: [[f32; 4]; 3],
    /// Packed `customIndex:24 | mask:8` (see [`Self::pack_index_mask`]).
    pub instance_custom_index_and_mask: u32,
    /// Packed `sbtOffset:24 | flags:8` (see [`Self::pack_sbt_offset_flags`]).
    pub instance_sbt_offset_and_flags: u32,
    /// The referenced BLAS **device address** (NOT the handle).
    pub acceleration_structure_reference: u64,
}

impl VkAccelerationStructureInstanceKHR {
    /// Packs `customIndex:24` (LSB) | `mask:8` (MSB) into the raw `u32` the spec's C
    /// bitfield occupies. `custom_index` is truncated to 24 bits; `mask` fills the top 8.
    #[inline]
    pub const fn pack_index_mask(custom_index: u32, mask: u8) -> u32 {
        (custom_index & 0x00FF_FFFF) | ((mask as u32) << 24)
    }

    /// Unpacks `pack_index_mask` back to `(custom_index_24, mask_8)`.
    #[inline]
    pub const fn unpack_index_mask(word: u32) -> (u32, u8) {
        (word & 0x00FF_FFFF, (word >> 24) as u8)
    }

    /// Packs `sbtOffset:24` (LSB) | `flags:8` (MSB) into the raw `u32`. `flags` is the
    /// `VkGeometryInstanceFlagsKHR` low byte (e.g. `TRIANGLE_FACING_CULL_DISABLE`).
    #[inline]
    pub const fn pack_sbt_offset_flags(sbt_offset: u32, flags: u8) -> u32 {
        (sbt_offset & 0x00FF_FFFF) | ((flags as u32) << 24)
    }

    /// Unpacks `pack_sbt_offset_flags` back to `(sbt_offset_24, flags_8)`.
    #[inline]
    pub const fn unpack_sbt_offset_flags(word: u32) -> (u32, u8) {
        (word & 0x00FF_FFFF, (word >> 24) as u8)
    }
}


// ---------------------------------------------------------------------------
// PFN typedefs (all device-scope except the two Feature/Property instance queries).
// ---------------------------------------------------------------------------

/// `PFN_vkGetAccelerationStructureBuildSizesKHR`.
pub type PfnVkGetAccelerationStructureBuildSizesKHR = unsafe extern "system" fn(
    device: VkDevice,
    // `build_type`: `VkAccelerationStructureBuildTypeKHR` (`DEVICE`).
    build_type: i32,
    p_build_info: *const VkAccelerationStructureBuildGeometryInfoKHR,
    p_max_primitive_counts: *const u32,
    p_size_info: *mut VkAccelerationStructureBuildSizesInfoKHR,
);

/// `PFN_vkCreateAccelerationStructureKHR`.
pub type PfnVkCreateAccelerationStructureKHR = unsafe extern "system" fn(
    device: VkDevice,
    p_create_info: *const VkAccelerationStructureCreateInfoKHR,
    p_allocator: *const c_void,
    p_acceleration_structure: *mut VkAccelerationStructureKHR,
) -> i32;

/// `PFN_vkDestroyAccelerationStructureKHR`.
pub type PfnVkDestroyAccelerationStructureKHR = unsafe extern "system" fn(
    device: VkDevice,
    acceleration_structure: VkAccelerationStructureKHR,
    p_allocator: *const c_void,
);

/// `PFN_vkCmdBuildAccelerationStructuresKHR`.
pub type PfnVkCmdBuildAccelerationStructuresKHR = unsafe extern "system" fn(
    command_buffer: VkCommandBuffer,
    info_count: u32,
    p_infos: *const VkAccelerationStructureBuildGeometryInfoKHR,
    pp_build_range_infos: *const *const VkAccelerationStructureBuildRangeInfoKHR,
);

/// `PFN_vkGetAccelerationStructureDeviceAddressKHR`.
pub type PfnVkGetAccelerationStructureDeviceAddressKHR = unsafe extern "system" fn(
    device: VkDevice,
    p_info: *const VkAccelerationStructureDeviceAddressInfoKHR,
) -> VkDeviceSize;

/// `PFN_vkGetBufferDeviceAddressKHR` (identical ABI to the 1.2-core `vkGetBufferDeviceAddress`).
pub type PfnVkGetBufferDeviceAddressKHR = unsafe extern "system" fn(
    device: VkDevice,
    p_info: *const VkBufferDeviceAddressInfo,
) -> VkDeviceSize;

// ---------------------------------------------------------------------------
// Instance-scope queries the `supports_ray_query` presence+feature+props path needs.
// ---------------------------------------------------------------------------

/// `VkPhysicalDeviceProperties2` — the head for `vkGetPhysicalDeviceProperties2` (Vulkan 1.1
/// core). The `properties` member is the 824-byte `VkPhysicalDeviceProperties` block,
/// reserved as an opaque ABI-exact footprint; the R2a-1 caps query reads only the chained
/// `VkPhysicalDeviceAccelerationStructurePropertiesKHR.min…ScratchOffsetAlignment` via `p_next`.
#[repr(C)]
pub struct VkPhysicalDeviceProperties2 {
    pub s_type: i32,
    pub _pad: i32,
    pub p_next: *mut c_void,
    /// `VkPhysicalDeviceProperties properties` — opaque, driver-written (824 bytes).
    pub properties: crate::ffi::VkPhysicalDeviceProperties,
}

/// `VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_PROPERTIES_2` — Vulkan 1.1 core.
pub const ST_PHYSICAL_DEVICE_PROPERTIES_2: i32 = 1_000_059_001;
/// `VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_FEATURES_2` — Vulkan 1.1 core (matches
/// `crate::ffi::VkStructureType::PhysicalDeviceFeatures2`, re-declared for self-containment).
pub const ST_PHYSICAL_DEVICE_FEATURES_2: i32 = 1_000_059_000;

/// `PFN_vkGetPhysicalDeviceProperties2` (Vulkan 1.1 core; the AS scratch-align props query).
pub type PfnVkGetPhysicalDeviceProperties2 = unsafe extern "system" fn(
    physical_device: crate::ffi::VkPhysicalDevice,
    p_properties: *mut VkPhysicalDeviceProperties2,
);

/// `PFN_vkEnumerateDeviceExtensionProperties` — the RT device-extension presence query
/// (Vulkan 1.0 core; `p_layer_name` null to query the device's own extensions).
pub type PfnVkEnumerateDeviceExtensionProperties = unsafe extern "system" fn(
    physical_device: crate::ffi::VkPhysicalDevice,
    p_layer_name: *const core::ffi::c_char,
    p_count: *mut u32,
    p_properties: *mut crate::ffi::VkExtensionProperties,
) -> i32;

const _: () = assert!(size_of::<VkPhysicalDeviceProperties2>() == 840);
const _: () = assert!(align_of::<VkPhysicalDeviceProperties2>() == 8);

// ---------------------------------------------------------------------------
// ABI guards (belt-and-suspenders, mirroring the crate's other Vk-struct pins).
// The struct byte layouts + the ABI-critical InstanceKHR pack sit in `abi_guard.rs`;
// the ones below are the self-contained module-local invariants (offsets a reader can
// verify against vulkan_core.h without cross-crate context).
// ---------------------------------------------------------------------------

// The instance struct is the load-bearing ABI (a driver reads a `[Self]` array during the
// TLAS build): pin its 64-B size, 8-B align, and all four field offsets HERE too (the
// primary assert lives in `abi_guard.rs`).
const _: () = assert!(size_of::<VkAccelerationStructureInstanceKHR>() == 64);
const _: () = assert!(align_of::<VkAccelerationStructureInstanceKHR>() == 8);
const _: () = assert!(offset_of!(VkAccelerationStructureInstanceKHR, transform) == 0);
const _: () =
    assert!(offset_of!(VkAccelerationStructureInstanceKHR, instance_custom_index_and_mask) == 48);
const _: () =
    assert!(offset_of!(VkAccelerationStructureInstanceKHR, instance_sbt_offset_and_flags) == 52);
const _: () =
    assert!(offset_of!(VkAccelerationStructureInstanceKHR, acceleration_structure_reference) == 56);

// The `transform` field IS the 48-B row-major bridge target (asserted cross-crate in
// abi_guard against `InstanceModelCol.rows`).
const _: () = assert!(size_of::<[[f32; 4]; 3]>() == 48);

#[cfg(test)]
mod tests {
    use super::VkAccelerationStructureInstanceKHR as Inst;

    /// The `customIndex:24 | mask:8` / `sbtOffset:24 | flags:8` bitfield pack MUST
    /// round-trip through the raw `u32` (the spec's C-bitfield layout Rust has no native
    /// support for). Pins the LSB=field / MSB=mask|flags split + the 24-bit truncation.
    #[test]
    fn instance_bitfield_pack_unpack_round_trip() {
        // customIndex fits 24 bits; mask is the full high byte.
        let custom = 0x00AB_CDEF; // 24-bit value (bit 24+ must be dropped).
        let mask: u8 = 0xF0;
        let w0 = Inst::pack_index_mask(custom, mask);
        assert_eq!(w0, (custom & 0x00FF_FFFF) | ((mask as u32) << 24));
        let (custom_out, mask_out) = Inst::unpack_index_mask(w0);
        assert_eq!(custom_out, custom & 0x00FF_FFFF);
        assert_eq!(mask_out, mask);

        // Over-24-bit customIndex is truncated to the low 24 bits (no bleed into the mask).
        let over = 0xFF00_0001; // bits 24..31 set → must NOT corrupt the mask byte.
        let w0b = Inst::pack_index_mask(over, 0x00);
        let (custom_b, mask_b) = Inst::unpack_index_mask(w0b);
        assert_eq!(custom_b, 0x0000_0001);
        assert_eq!(mask_b, 0x00);

        // sbtOffset:24 | flags:8 round-trip.
        let sbt = 0x0012_3456;
        let flags: u8 = 0x01; // e.g. TRIANGLE_FACING_CULL_DISABLE.
        let w1 = Inst::pack_sbt_offset_flags(sbt, flags);
        assert_eq!(w1, (sbt & 0x00FF_FFFF) | ((flags as u32) << 24));
        let (sbt_out, flags_out) = Inst::unpack_sbt_offset_flags(w1);
        assert_eq!(sbt_out, sbt & 0x00FF_FFFF);
        assert_eq!(flags_out, flags);

        // A full instance packs the two words + the BLAS device-address reference.
        let inst = Inst {
            transform: [
                [1.0, 0.0, 0.0, 10.0],
                [0.0, 1.0, 0.0, 20.0],
                [0.0, 0.0, 1.0, 30.0],
            ],
            instance_custom_index_and_mask: Inst::pack_index_mask(7, 0xFF),
            instance_sbt_offset_and_flags: Inst::pack_sbt_offset_flags(0, 0),
            acceleration_structure_reference: 0xDEAD_BEEF_0000_1000,
        };
        assert_eq!(Inst::unpack_index_mask(inst.instance_custom_index_and_mask), (7, 0xFF));
        assert_eq!(Inst::unpack_sbt_offset_flags(inst.instance_sbt_offset_and_flags), (0, 0));
        assert_eq!(inst.acceleration_structure_reference, 0xDEAD_BEEF_0000_1000);
        // The translation lives in column 3 (row-major m[r][3]).
        assert_eq!(inst.transform[0][3], 10.0);
        assert_eq!(inst.transform[2][3], 30.0);
    }
}
