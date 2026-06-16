//! Hand-declared raw Vulkan FFI surface — the MINIMUM needed to boot a device
//! and round-trip a host-visible buffer (Slice 0, no SDK).
//!
//! # Discipline
//!
//! This module mirrors the `boyko_ecs::ecs::memory::vm.rs` FFI idiom verbatim:
//! hand-declared `unsafe extern "system"` blocks with a per-block `// SAFETY:`
//! ABI comment, `#[cfg(windows)]` OS gating, no third-party crates. The Vulkan
//! command functions themselves are NOT linked at build time — they are
//! resolved at runtime through `vkGetInstanceProcAddr` / `vkGetDeviceProcAddr`
//! (the loader's three-tier dispatch, §4 of the plan) — so this module only
//! `extern`-declares the *bootstrap* OS calls (`LoadLibraryA`,
//! `GetProcAddress`, `FreeLibrary`) and otherwise defines function-pointer
//! typedefs the loader fills in.
//!
//! # ABI assumptions (x86_64 only, per the plan)
//!
//! - Dispatchable handles (`VkInstance`/`VkPhysicalDevice`/`VkDevice`/
//!   `VkQueue`) are opaque pointers → `#[repr(transparent)]` over a raw
//!   pointer.
//! - Non-dispatchable handles (`VkDeviceMemory`/`VkBuffer`) are 64-bit on every
//!   platform per the Vulkan spec, AND identical to a pointer width on the
//!   x86_64 target → `#[repr(transparent)]` over `u64`.
//! - Vulkan uses the platform default ("system") calling convention for its
//!   commands, matching the loader's exported symbols.

#![allow(non_snake_case)]
#![allow(non_camel_case_types)]

use core::ffi::{c_char, c_void};

// ---------------------------------------------------------------------------
// OS loader surface (Windows) — twin of `vm.rs::win`.
// ---------------------------------------------------------------------------

#[cfg(windows)]
pub mod os {
    use core::ffi::{c_char, c_void};

    // SAFETY: signatures match the Win64 kernel32 ABI exactly. `LoadLibraryA`
    // takes an ANSI C string (LPCSTR -> *const c_char) and returns an HMODULE
    // (an opaque handle, modelled as *mut c_void; NULL on failure).
    // `GetProcAddress` takes that HMODULE plus an ANSI symbol name and returns
    // a FARPROC (a function pointer, modelled as *mut c_void; NULL on failure).
    // `FreeLibrary` takes the HMODULE and returns a BOOL (i32, non-zero on
    // success). kernel32 is linked transitively by std.
    unsafe extern "system" {
        pub fn LoadLibraryA(lpLibFileName: *const c_char) -> *mut c_void;
        pub fn GetProcAddress(hModule: *mut c_void, lpProcName: *const c_char) -> *mut c_void;
        pub fn FreeLibrary(hModule: *mut c_void) -> i32;
    }
}

// ---------------------------------------------------------------------------
// Core scalar types.
// ---------------------------------------------------------------------------

/// `VkBool32` — Vulkan's 32-bit boolean (`VK_TRUE` == 1, `VK_FALSE` == 0).
pub type VkBool32 = u32;
/// `VkFlags` / `Vk*FlagBits` underlying type.
pub type VkFlags = u32;
/// `VkDeviceSize` — byte sizes / offsets in device memory (always 64-bit).
pub type VkDeviceSize = u64;

pub const VK_FALSE: VkBool32 = 0;

// ---------------------------------------------------------------------------
// Handles.
// ---------------------------------------------------------------------------

/// Dispatchable Vulkan handle — an opaque pointer to a loader-internal object.
///
/// `#[repr(transparent)]` over a raw pointer so the newtype is ABI-identical to
/// the C `typedef struct VkInstance_T* VkInstance;` form. The pointer is never
/// dereferenced on the Rust side; it is only handed back to Vulkan commands.
macro_rules! dispatchable_handle {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[repr(transparent)]
        #[derive(Clone, Copy, PartialEq, Eq)]
        pub struct $name(pub *mut c_void);

        impl $name {
            /// The Vulkan null handle (`VK_NULL_HANDLE`).
            pub const NULL: Self = Self(core::ptr::null_mut());

            /// Whether this is the null handle.
            #[inline]
            pub fn is_null(self) -> bool {
                self.0.is_null()
            }
        }

        // SAFETY: the handle is an opaque token (a raw pointer never dereferenced
        // in Rust), so moving the token value between threads cannot race Rust
        // memory — `Send` is sound. `Sync` is deliberately NOT implemented: a
        // shared `&handle` across threads would invite concurrent Vulkan calls on
        // an externally-synchronized object (a Vulkan-level data race the type
        // must not silently bless). Cross-thread access is governed later by the
        // dispatcher-only `NonSendResource` model (plan §5.3), not a blanket `Sync`.
        unsafe impl Send for $name {}
    };
}

/// Non-dispatchable Vulkan handle — a 64-bit opaque token (object handle).
///
/// `#[repr(transparent)]` over `u64`, matching the spec's guarantee that
/// non-dispatchable handles are 64 bits wide on every platform.
macro_rules! non_dispatchable_handle {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[repr(transparent)]
        #[derive(Clone, Copy, PartialEq, Eq)]
        pub struct $name(pub u64);

        impl $name {
            /// The Vulkan null handle (`VK_NULL_HANDLE` == 0).
            pub const NULL: Self = Self(0);

            /// Whether this is the null handle.
            #[inline]
            pub fn is_null(self) -> bool {
                self.0 == 0
            }
        }
    };
}

dispatchable_handle!(
    /// `VkInstance` — the per-application Vulkan connection.
    VkInstance
);
dispatchable_handle!(
    /// `VkPhysicalDevice` — a GPU enumerated from an instance.
    VkPhysicalDevice
);
dispatchable_handle!(
    /// `VkDevice` — a logical device created from a physical device.
    VkDevice
);
dispatchable_handle!(
    /// `VkQueue` — a queue retrieved from a logical device.
    VkQueue
);

non_dispatchable_handle!(
    /// `VkDeviceMemory` — a device memory allocation.
    VkDeviceMemory
);
non_dispatchable_handle!(
    /// `VkBuffer` — a linear buffer resource.
    VkBuffer
);

// ---------------------------------------------------------------------------
// VkResult.
// ---------------------------------------------------------------------------

/// `VkResult` — Vulkan command status. `VK_SUCCESS == 0`; positive values are
/// non-error statuses; negative values are errors (the spec's convention).
///
/// Modelled as a `#[repr(transparent)]` newtype over `i32` (the C enum's
/// underlying type on this ABI) rather than a Rust `enum`, so that ANY code a
/// driver returns is preserved verbatim with zero risk of an unmodelled value
/// becoming UB — the idiomatic raw-FFI pattern (cf. `ash`'s `vk::Result`). The
/// codes Slice 0 can observe are exposed as associated `const`s.
#[repr(transparent)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct VkResult(pub i32);

impl VkResult {
    pub const SUCCESS: Self = Self(0);
    pub const NOT_READY: Self = Self(1);
    pub const INCOMPLETE: Self = Self(5);
    pub const ERROR_OUT_OF_HOST_MEMORY: Self = Self(-1);
    pub const ERROR_OUT_OF_DEVICE_MEMORY: Self = Self(-2);
    pub const ERROR_INITIALIZATION_FAILED: Self = Self(-3);
    pub const ERROR_LAYER_NOT_PRESENT: Self = Self(-6);
    pub const ERROR_EXTENSION_NOT_PRESENT: Self = Self(-7);
    pub const ERROR_FEATURE_NOT_PRESENT: Self = Self(-8);
    pub const ERROR_INCOMPATIBLE_DRIVER: Self = Self(-9);
    pub const ERROR_TOO_MANY_OBJECTS: Self = Self(-10);

    /// Reconstructs a `VkResult` from the raw `i32` an FFI command returned.
    #[inline]
    pub fn from_raw(raw: i32) -> Self {
        Self(raw)
    }

    /// The raw `i32` status code.
    #[inline]
    pub fn as_raw(self) -> i32 {
        self.0
    }

    /// Whether the command succeeded (`VK_SUCCESS`).
    #[inline]
    pub fn is_success(self) -> bool {
        self.0 == 0
    }
}

impl core::fmt::Debug for VkResult {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let name = match *self {
            Self::SUCCESS => "VK_SUCCESS",
            Self::NOT_READY => "VK_NOT_READY",
            Self::INCOMPLETE => "VK_INCOMPLETE",
            Self::ERROR_OUT_OF_HOST_MEMORY => "VK_ERROR_OUT_OF_HOST_MEMORY",
            Self::ERROR_OUT_OF_DEVICE_MEMORY => "VK_ERROR_OUT_OF_DEVICE_MEMORY",
            Self::ERROR_INITIALIZATION_FAILED => "VK_ERROR_INITIALIZATION_FAILED",
            Self::ERROR_LAYER_NOT_PRESENT => "VK_ERROR_LAYER_NOT_PRESENT",
            Self::ERROR_EXTENSION_NOT_PRESENT => "VK_ERROR_EXTENSION_NOT_PRESENT",
            Self::ERROR_FEATURE_NOT_PRESENT => "VK_ERROR_FEATURE_NOT_PRESENT",
            Self::ERROR_INCOMPATIBLE_DRIVER => "VK_ERROR_INCOMPATIBLE_DRIVER",
            Self::ERROR_TOO_MANY_OBJECTS => "VK_ERROR_TOO_MANY_OBJECTS",
            _ => return write!(f, "VkResult({})", self.0),
        };
        f.write_str(name)
    }
}

// ---------------------------------------------------------------------------
// VkStructureType (only the sType tags we set).
// ---------------------------------------------------------------------------

/// `VkStructureType` — the `sType` discriminant heading every `*CreateInfo`.
/// `#[repr(i32)]` matches the C enum ABI; only the tags Slice 0 sets are named.
#[repr(i32)]
#[derive(Clone, Copy)]
pub enum VkStructureType {
    ApplicationInfo = 0,
    InstanceCreateInfo = 1,
    DeviceQueueCreateInfo = 2,
    DeviceCreateInfo = 3,
    MemoryAllocateInfo = 5,
    BufferCreateInfo = 12,
}

// ---------------------------------------------------------------------------
// Enums / flag constants used in the *CreateInfo structs.
// ---------------------------------------------------------------------------

/// `VkPhysicalDeviceType` discriminants (subset).
pub const VK_PHYSICAL_DEVICE_TYPE_DISCRETE_GPU: i32 = 2;

/// `VkQueueFlagBits`.
pub const VK_QUEUE_GRAPHICS_BIT: VkFlags = 0x0000_0001;
pub const VK_QUEUE_COMPUTE_BIT: VkFlags = 0x0000_0002;

/// `VkMemoryPropertyFlagBits`.
pub const VK_MEMORY_PROPERTY_DEVICE_LOCAL_BIT: VkFlags = 0x0000_0001;
pub const VK_MEMORY_PROPERTY_HOST_VISIBLE_BIT: VkFlags = 0x0000_0002;
pub const VK_MEMORY_PROPERTY_HOST_COHERENT_BIT: VkFlags = 0x0000_0004;

/// `VkBufferUsageFlagBits` (subset; the round-trip uses a transfer/storage
/// buffer — the exact bits are immaterial to a host-visible map round-trip but
/// must be a valid usage).
pub const VK_BUFFER_USAGE_TRANSFER_SRC_BIT: VkFlags = 0x0000_0001;
pub const VK_BUFFER_USAGE_TRANSFER_DST_BIT: VkFlags = 0x0000_0002;
pub const VK_BUFFER_USAGE_STORAGE_BUFFER_BIT: VkFlags = 0x0000_0020;

/// `VkSharingMode::VK_SHARING_MODE_EXCLUSIVE`.
pub const VK_SHARING_MODE_EXCLUSIVE: i32 = 0;

/// `VK_API_VERSION_1_3` packed `(major << 22) | (minor << 12) | patch`.
pub const VK_API_VERSION_1_3: u32 = (1 << 22) | (3 << 12);

/// `VK_WHOLE_SIZE` sentinel for `vkMapMemory` range length.
pub const VK_WHOLE_SIZE: VkDeviceSize = u64::MAX;

/// Bound on the physical-device / memory-type / queue-family arrays the spec
/// caps. `VK_MAX_MEMORY_TYPES`.
pub const VK_MAX_MEMORY_TYPES: usize = 32;
/// `VK_MAX_MEMORY_HEAPS`.
pub const VK_MAX_MEMORY_HEAPS: usize = 16;

// ---------------------------------------------------------------------------
// #[repr(C)] structs — declare only fields we read or write.
// ---------------------------------------------------------------------------

/// `VkApplicationInfo`.
#[repr(C)]
pub struct VkApplicationInfo {
    pub s_type: VkStructureType,
    pub p_next: *const c_void,
    pub p_application_name: *const c_char,
    pub application_version: u32,
    pub p_engine_name: *const c_char,
    pub engine_version: u32,
    pub api_version: u32,
}

/// `VkInstanceCreateInfo`.
#[repr(C)]
pub struct VkInstanceCreateInfo {
    pub s_type: VkStructureType,
    pub p_next: *const c_void,
    pub flags: VkFlags,
    pub p_application_info: *const VkApplicationInfo,
    pub enabled_layer_count: u32,
    pub pp_enabled_layer_names: *const *const c_char,
    pub enabled_extension_count: u32,
    pub pp_enabled_extension_names: *const *const c_char,
}

/// `VkDeviceQueueCreateInfo`.
#[repr(C)]
pub struct VkDeviceQueueCreateInfo {
    pub s_type: VkStructureType,
    pub p_next: *const c_void,
    pub flags: VkFlags,
    pub queue_family_index: u32,
    pub queue_count: u32,
    pub p_queue_priorities: *const f32,
}

/// `VkDeviceCreateInfo`.
#[repr(C)]
pub struct VkDeviceCreateInfo {
    pub s_type: VkStructureType,
    pub p_next: *const c_void,
    pub flags: VkFlags,
    pub queue_create_info_count: u32,
    pub p_queue_create_infos: *const VkDeviceQueueCreateInfo,
    pub enabled_layer_count: u32,
    pub pp_enabled_layer_names: *const *const c_char,
    pub enabled_extension_count: u32,
    pub pp_enabled_extension_names: *const *const c_char,
    /// `const VkPhysicalDeviceFeatures*` — left null (no features requested).
    pub p_enabled_features: *const c_void,
}

/// `VkQueueFamilyProperties`.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct VkQueueFamilyProperties {
    pub queue_flags: VkFlags,
    pub queue_count: u32,
    pub timestamp_valid_bits: u32,
    /// `VkExtent3D minImageTransferGranularity` flattened to three `u32`s
    /// (its only fields), so this struct stays a faithful `#[repr(C)]` mirror.
    pub min_image_transfer_granularity_width: u32,
    pub min_image_transfer_granularity_height: u32,
    pub min_image_transfer_granularity_depth: u32,
}

/// `VkPhysicalDeviceLimits` reserved as opaque bytes (504), but declared
/// `#[repr(C, align(8))]` because the real C struct contains `VkDeviceSize`
/// (`u64`) members and is therefore 8-aligned. **The alignment is load-bearing:**
/// it forces `limits` to the C ABI offset (296, after 4 bytes of padding past
/// `pipelineCacheUUID`) and makes the parent struct 824 bytes / align 8 — exactly
/// what `vkGetPhysicalDeviceProperties` writes through the out-pointer. A bare
/// `[u8; 504]` (align 1) collapses that padding to an 816-byte/align-4 struct, so
/// the driver overruns the out-buffer by 8 bytes (a latent stack overflow that
/// happens to be benign only on some drivers/stack layouts). See the layout
/// guards below.
#[repr(C, align(8))]
pub struct VkPhysicalDeviceLimitsBlob(pub [u8; 504]);

/// `VkPhysicalDeviceProperties` — declared up to and including `deviceName`
/// (the only fields Slice 0 reads). `limits`/`sparseProperties` are reserved as
/// opaque, ABI-exact footprints (`VkPhysicalDeviceLimitsBlob` carries the
/// 8-alignment) so the struct's size/layout match the C ABI for the
/// `vkGetPhysicalDeviceProperties` out-pointer.
#[repr(C)]
pub struct VkPhysicalDeviceProperties {
    pub api_version: u32,
    pub driver_version: u32,
    pub vendor_id: u32,
    pub device_id: u32,
    /// `VkPhysicalDeviceType`.
    pub device_type: i32,
    /// `char deviceName[VK_MAX_PHYSICAL_DEVICE_NAME_SIZE]` (256 bytes,
    /// NUL-terminated UTF-8).
    pub device_name: [c_char; 256],
    /// `uint8_t pipelineCacheUUID[VK_UUID_SIZE]`.
    pub pipeline_cache_uuid: [u8; 16],
    /// `VkPhysicalDeviceLimits` — opaque, 8-aligned (see `VkPhysicalDeviceLimitsBlob`).
    pub limits: VkPhysicalDeviceLimitsBlob,
    /// `VkPhysicalDeviceSparseProperties` — 5 `VkBool32`s = 20 bytes (align 4);
    /// the parent's 8-alignment supplies the trailing pad to 824.
    pub sparse_properties: [u8; 20],
}

// FFI layout guards: these structs are written BY the driver through an
// out-pointer, so the Rust type's size/alignment MUST equal the C ABI or the
// driver writes out of bounds (latent UB). They break the build on any drift.
const _: () = assert!(core::mem::size_of::<VkPhysicalDeviceProperties>() == 824);
const _: () = assert!(core::mem::align_of::<VkPhysicalDeviceProperties>() == 8);
const _: () = assert!(core::mem::size_of::<VkPhysicalDeviceMemoryProperties>() == 520);
const _: () = assert!(core::mem::size_of::<VkMemoryRequirements>() == 24);
const _: () = assert!(core::mem::size_of::<VkQueueFamilyProperties>() == 24);

/// `VkMemoryType`.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct VkMemoryType {
    pub property_flags: VkFlags,
    pub heap_index: u32,
}

/// `VkMemoryHeap`.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct VkMemoryHeap {
    pub size: VkDeviceSize,
    pub flags: VkFlags,
}

/// `VkPhysicalDeviceMemoryProperties`.
#[repr(C)]
pub struct VkPhysicalDeviceMemoryProperties {
    pub memory_type_count: u32,
    pub memory_types: [VkMemoryType; VK_MAX_MEMORY_TYPES],
    pub memory_heap_count: u32,
    pub memory_heaps: [VkMemoryHeap; VK_MAX_MEMORY_HEAPS],
}

/// `VkMemoryRequirements`.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct VkMemoryRequirements {
    pub size: VkDeviceSize,
    pub alignment: VkDeviceSize,
    /// Bitmask of memory-type indices acceptable for this resource.
    pub memory_type_bits: u32,
}

/// `VkMemoryAllocateInfo`.
#[repr(C)]
pub struct VkMemoryAllocateInfo {
    pub s_type: VkStructureType,
    pub p_next: *const c_void,
    pub allocation_size: VkDeviceSize,
    pub memory_type_index: u32,
}

/// `VkBufferCreateInfo`.
#[repr(C)]
pub struct VkBufferCreateInfo {
    pub s_type: VkStructureType,
    pub p_next: *const c_void,
    pub flags: VkFlags,
    pub size: VkDeviceSize,
    pub usage: VkFlags,
    /// `VkSharingMode`.
    pub sharing_mode: i32,
    pub queue_family_index_count: u32,
    pub p_queue_family_indices: *const u32,
}

// ---------------------------------------------------------------------------
// Function-pointer typedefs — the loader fills these in at runtime.
//
// Every pointer uses `extern "system"` (Vulkan's calling convention) and is
// declared `unsafe` (calling through it is unconditionally unsafe FFI). The
// proc-loader transmutes a raw `*mut c_void` from `vkGetInstanceProcAddr` /
// `vkGetDeviceProcAddr` into the matching typedef — see `device.rs`.
// ---------------------------------------------------------------------------

/// `PFN_vkVoidFunction` — the untyped function pointer the proc-addr getters
/// return. `Option<...>` so a NULL return is representable as `None` (the
/// null-function-pointer optimization makes this ABI-identical to the raw
/// pointer).
pub type PfnVkVoidFunction = Option<unsafe extern "system" fn()>;

/// `PFN_vkGetInstanceProcAddr`.
pub type PfnVkGetInstanceProcAddr =
    unsafe extern "system" fn(instance: VkInstance, p_name: *const c_char) -> PfnVkVoidFunction;

/// `PFN_vkGetDeviceProcAddr`.
pub type PfnVkGetDeviceProcAddr =
    unsafe extern "system" fn(device: VkDevice, p_name: *const c_char) -> PfnVkVoidFunction;

/// `PFN_vkCreateInstance`.
pub type PfnVkCreateInstance = unsafe extern "system" fn(
    p_create_info: *const VkInstanceCreateInfo,
    p_allocator: *const c_void,
    p_instance: *mut VkInstance,
) -> i32;

/// `PFN_vkDestroyInstance`.
pub type PfnVkDestroyInstance =
    unsafe extern "system" fn(instance: VkInstance, p_allocator: *const c_void);

/// `PFN_vkEnumeratePhysicalDevices`.
pub type PfnVkEnumeratePhysicalDevices = unsafe extern "system" fn(
    instance: VkInstance,
    p_count: *mut u32,
    p_devices: *mut VkPhysicalDevice,
) -> i32;

/// `PFN_vkGetPhysicalDeviceProperties`.
pub type PfnVkGetPhysicalDeviceProperties = unsafe extern "system" fn(
    physical_device: VkPhysicalDevice,
    p_properties: *mut VkPhysicalDeviceProperties,
);

/// `PFN_vkGetPhysicalDeviceMemoryProperties`.
pub type PfnVkGetPhysicalDeviceMemoryProperties = unsafe extern "system" fn(
    physical_device: VkPhysicalDevice,
    p_properties: *mut VkPhysicalDeviceMemoryProperties,
);

/// `PFN_vkGetPhysicalDeviceQueueFamilyProperties`.
pub type PfnVkGetPhysicalDeviceQueueFamilyProperties = unsafe extern "system" fn(
    physical_device: VkPhysicalDevice,
    p_count: *mut u32,
    p_properties: *mut VkQueueFamilyProperties,
);

/// `PFN_vkCreateDevice`.
pub type PfnVkCreateDevice = unsafe extern "system" fn(
    physical_device: VkPhysicalDevice,
    p_create_info: *const VkDeviceCreateInfo,
    p_allocator: *const c_void,
    p_device: *mut VkDevice,
) -> i32;

/// `PFN_vkDestroyDevice`.
pub type PfnVkDestroyDevice =
    unsafe extern "system" fn(device: VkDevice, p_allocator: *const c_void);

/// `PFN_vkGetDeviceQueue`.
pub type PfnVkGetDeviceQueue = unsafe extern "system" fn(
    device: VkDevice,
    queue_family_index: u32,
    queue_index: u32,
    p_queue: *mut VkQueue,
);

/// `PFN_vkCreateBuffer`.
pub type PfnVkCreateBuffer = unsafe extern "system" fn(
    device: VkDevice,
    p_create_info: *const VkBufferCreateInfo,
    p_allocator: *const c_void,
    p_buffer: *mut VkBuffer,
) -> i32;

/// `PFN_vkDestroyBuffer`.
pub type PfnVkDestroyBuffer =
    unsafe extern "system" fn(device: VkDevice, buffer: VkBuffer, p_allocator: *const c_void);

/// `PFN_vkGetBufferMemoryRequirements`.
pub type PfnVkGetBufferMemoryRequirements = unsafe extern "system" fn(
    device: VkDevice,
    buffer: VkBuffer,
    p_requirements: *mut VkMemoryRequirements,
);

/// `PFN_vkAllocateMemory`.
pub type PfnVkAllocateMemory = unsafe extern "system" fn(
    device: VkDevice,
    p_allocate_info: *const VkMemoryAllocateInfo,
    p_allocator: *const c_void,
    p_memory: *mut VkDeviceMemory,
) -> i32;

/// `PFN_vkFreeMemory`.
pub type PfnVkFreeMemory =
    unsafe extern "system" fn(device: VkDevice, memory: VkDeviceMemory, p_allocator: *const c_void);

/// `PFN_vkBindBufferMemory`.
pub type PfnVkBindBufferMemory = unsafe extern "system" fn(
    device: VkDevice,
    buffer: VkBuffer,
    memory: VkDeviceMemory,
    memory_offset: VkDeviceSize,
) -> i32;

/// `PFN_vkMapMemory`.
pub type PfnVkMapMemory = unsafe extern "system" fn(
    device: VkDevice,
    memory: VkDeviceMemory,
    offset: VkDeviceSize,
    size: VkDeviceSize,
    flags: VkFlags,
    pp_data: *mut *mut c_void,
) -> i32;

/// `PFN_vkUnmapMemory`.
pub type PfnVkUnmapMemory = unsafe extern "system" fn(device: VkDevice, memory: VkDeviceMemory);
