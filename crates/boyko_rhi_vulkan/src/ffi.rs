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

    // --- I6 / I6b: Win32 INPUT FFI from the OFFICIAL windows-sys bindings. ---
    //
    // The window-handle accessors (`SetWindowLongPtrW`/`GetWindowLongPtrW`), the
    // Raw-Input calls (`RegisterRawInputDevices`/`GetRawInputData`), the
    // `RAWINPUT*` structs, and the WM_* / GWLP_USERDATA / RID_* / HID_* / mouse
    // input constants are re-exported here from `windows-sys` so the window's
    // call sites keep their `os::…` prefix. These are MS-maintained bindings: the
    // hand-rolled `#[repr(C)]` structs + ABI-guard const-asserts they replace are
    // deleted (the layouts are now guaranteed by the official crate). The Vulkan
    // FFI below stays 100% in-house. `windows-sys` is target-gated to
    // `cfg(windows)`, so non-Windows builds pull nothing.
    pub use windows_sys::Win32::Devices::HumanInterfaceDevice::{
        HID_USAGE_GENERIC_MOUSE, HID_USAGE_PAGE_GENERIC,
    };
    pub use windows_sys::Win32::UI::Input::{
        GetRawInputData, RAWINPUT, RAWINPUTDEVICE, RAWINPUTHEADER, RAWMOUSE, RID_INPUT,
        RIM_TYPEMOUSE, RegisterRawInputDevices,
    };
    pub use windows_sys::Win32::UI::WindowsAndMessaging::{
        GWLP_USERDATA, GetWindowLongPtrW, SetWindowLongPtrW, WM_INPUT, WM_KEYDOWN, WM_KEYUP,
        WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MBUTTONDOWN, WM_MBUTTONUP, WM_MOUSEHWHEEL, WM_MOUSEMOVE,
        WM_MOUSEWHEEL, WM_RBUTTONDOWN, WM_RBUTTONUP, WM_SYSKEYDOWN, WM_SYSKEYUP, WM_XBUTTONDOWN,
        WM_XBUTTONUP,
    };

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

    // --- Slice-1 raw Win32 windowing (kernel32 + user32). ---

    /// `WPARAM` — a `UINT_PTR` (pointer-width on Win64).
    pub type WPARAM = usize;
    /// `LPARAM` — a `LONG_PTR` (pointer-width on Win64).
    pub type LPARAM = isize;
    /// `LRESULT` — a `LONG_PTR`.
    pub type LRESULT = isize;
    /// `ATOM` — the `RegisterClassExW` return type (a `WORD`, widened to u16).
    pub type ATOM = u16;

    /// `WNDPROC` — the window-procedure callback the OS calls per message.
    /// `extern "system"` matches the Win32 calling convention.
    pub type WndProc = unsafe extern "system" fn(
        hwnd: *mut c_void,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT;

    /// `WNDCLASSEXW` — read BY user32 in `RegisterClassExW`; ABI-exact (its
    /// layout is fixed by the Win32 header). `cbSize` MUST be `size_of::<Self>()`.
    #[repr(C)]
    pub struct WNDCLASSEXW {
        pub cb_size: u32,
        pub style: u32,
        pub lpfn_wnd_proc: Option<WndProc>,
        pub cb_cls_extra: i32,
        pub cb_wnd_extra: i32,
        pub h_instance: *mut c_void,
        pub h_icon: *mut c_void,
        pub h_cursor: *mut c_void,
        pub hbr_background: *mut c_void,
        pub lpsz_menu_name: *const u16,
        pub lpsz_class_name: *const u16,
        pub h_icon_sm: *mut c_void,
    }

    /// `POINT`.
    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    pub struct POINT {
        pub x: i32,
        pub y: i32,
    }

    /// `MSG` — written BY user32 in `PeekMessageW`; ABI-exact.
    #[repr(C)]
    pub struct MSG {
        pub hwnd: *mut c_void,
        pub message: u32,
        pub w_param: WPARAM,
        pub l_param: LPARAM,
        pub time: u32,
        pub pt: POINT,
        /// `lPrivate` (Win64-only trailing DWORD; present in the OS struct).
        pub l_private: u32,
    }

    /// `RECT` — written BY user32 in `AdjustWindowRectEx`/`GetClientRect`.
    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    pub struct RECT {
        pub left: i32,
        pub top: i32,
        pub right: i32,
        pub bottom: i32,
    }

    // SAFETY: signatures match the Win64 user32/kernel32 ABI exactly. HWND /
    // HINSTANCE / HMODULE / HCURSOR are opaque handles (`*mut c_void`); wide
    // strings are `*const u16` (LPCWSTR); BOOLs are `i32`. user32 is linked via
    // the `#[link]` attribute below; kernel32 is linked transitively by std.
    #[link(name = "user32")]
    unsafe extern "system" {
        pub fn RegisterClassExW(unnamed: *const WNDCLASSEXW) -> ATOM;
        pub fn UnregisterClassW(lpClassName: *const u16, hInstance: *mut c_void) -> i32;
        #[allow(clippy::too_many_arguments)]
        pub fn CreateWindowExW(
            dwExStyle: u32,
            lpClassName: *const u16,
            lpWindowName: *const u16,
            dwStyle: u32,
            x: i32,
            y: i32,
            nWidth: i32,
            nHeight: i32,
            hWndParent: *mut c_void,
            hMenu: *mut c_void,
            hInstance: *mut c_void,
            lpParam: *mut c_void,
        ) -> *mut c_void;
        pub fn DestroyWindow(hWnd: *mut c_void) -> i32;
        pub fn ShowWindow(hWnd: *mut c_void, nCmdShow: i32) -> i32;
        pub fn UpdateWindow(hWnd: *mut c_void) -> i32;
        pub fn DefWindowProcW(
            hWnd: *mut c_void,
            msg: u32,
            wParam: WPARAM,
            lParam: LPARAM,
        ) -> LRESULT;
        pub fn PeekMessageW(
            lpMsg: *mut MSG,
            hWnd: *mut c_void,
            wMsgFilterMin: u32,
            wMsgFilterMax: u32,
            wRemoveMsg: u32,
        ) -> i32;
        pub fn TranslateMessage(lpMsg: *const MSG) -> i32;
        pub fn DispatchMessageW(lpMsg: *const MSG) -> LRESULT;
        pub fn PostQuitMessage(nExitCode: i32);
        pub fn LoadCursorW(hInstance: *mut c_void, lpCursorName: *const u16) -> *mut c_void;
        pub fn GetClientRect(hWnd: *mut c_void, lpRect: *mut RECT) -> i32;
        pub fn AdjustWindowRectEx(
            lpRect: *mut RECT,
            dwStyle: u32,
            bMenu: i32,
            dwExStyle: u32,
        ) -> i32;
    }

    // SAFETY: `GetModuleHandleW(null)` returns the HMODULE of the calling
    // process image (an HINSTANCE for windowing); the ABI is a single LPCWSTR
    // argument returning an HMODULE. kernel32 is linked transitively by std.
    unsafe extern "system" {
        pub fn GetModuleHandleW(lpModuleName: *const u16) -> *mut c_void;
    }

    // --- Win32 message / style / show constants used by the window. ---

    /// `WM_DESTROY`.
    pub const WM_DESTROY: u32 = 0x0002;
    /// `WM_CLOSE`.
    pub const WM_CLOSE: u32 = 0x0010;
    /// `WM_QUIT` (posted by `PostQuitMessage`).
    pub const WM_QUIT: u32 = 0x0012;

    /// `PM_REMOVE` for `PeekMessageW` (remove the message from the queue).
    pub const PM_REMOVE: u32 = 0x0001;

    /// `WS_OVERLAPPEDWINDOW` — a standard titled, resizable window.
    pub const WS_OVERLAPPEDWINDOW: u32 = 0x00CF_0000;

    /// `SW_SHOW` for `ShowWindow`.
    pub const SW_SHOW: i32 = 5;

    /// `CW_USEDEFAULT` window-position sentinel.
    pub const CW_USEDEFAULT: i32 = i32::MIN; // 0x80000000 as i32

    /// `IDC_ARROW` — the standard arrow cursor (a `MAKEINTRESOURCE(32512)`).
    pub const IDC_ARROW: u16 = 32512;

    /// `CS_HREDRAW | CS_VREDRAW` — redraw on horizontal/vertical resize.
    pub const CS_HREDRAW: u32 = 0x0002;
    pub const CS_VREDRAW: u32 = 0x0001;

    // FFI layout guards on the OS-written structs (the OS writes `MSG` in
    // `PeekMessageW`; `WNDCLASSEXW` is read by `RegisterClassExW`). A drift here
    // would make the OS read/write out of bounds.
    const _: () = assert!(core::mem::size_of::<MSG>() == 48);
    const _: () = assert!(core::mem::size_of::<WNDCLASSEXW>() == 80);
    const _: () = assert!(core::mem::size_of::<POINT>() == 8);
    const _: () = assert!(core::mem::size_of::<RECT>() == 16);
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
pub const VK_TRUE: VkBool32 = 1;

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

// --- Slice-0 0c/0d compute-pipeline handles (all non-dispatchable / 64-bit). ---

non_dispatchable_handle!(
    /// `VkShaderModule` — a compiled SPIR-V module.
    VkShaderModule
);
non_dispatchable_handle!(
    /// `VkDescriptorSetLayout` — the layout of one descriptor set.
    VkDescriptorSetLayout
);
non_dispatchable_handle!(
    /// `VkPipelineLayout` — descriptor-set-layouts + push-constant ranges.
    VkPipelineLayout
);
non_dispatchable_handle!(
    /// `VkPipeline` — a compute (or graphics) pipeline.
    VkPipeline
);
non_dispatchable_handle!(
    /// `VkDescriptorPool` — allocates `VkDescriptorSet`s.
    VkDescriptorPool
);
non_dispatchable_handle!(
    /// `VkDescriptorSet` — a bound set of descriptors.
    VkDescriptorSet
);
non_dispatchable_handle!(
    /// `VkCommandPool` — allocates command buffers.
    VkCommandPool
);
non_dispatchable_handle!(
    /// `VkFence` — a host-visible GPU-completion sync primitive.
    VkFence
);
non_dispatchable_handle!(
    /// `VkQueryPool` — a pool of GPU queries (HW-RT rung R0: TIMESTAMP queries).
    VkQueryPool
);
non_dispatchable_handle!(
    /// `VkDebugUtilsMessengerEXT` — the validation-message callback registration.
    VkDebugUtilsMessengerEXT
);

dispatchable_handle!(
    /// `VkCommandBuffer` — a recorded command stream (a dispatchable handle).
    VkCommandBuffer
);

// --- Slice-1 surface / swapchain / dynamic-rendering handles (non-dispatchable). ---

non_dispatchable_handle!(
    /// `VkSurfaceKHR` — a platform window surface to present to.
    VkSurfaceKHR
);
non_dispatchable_handle!(
    /// `VkSwapchainKHR` — a swapchain of presentable images over a surface.
    VkSwapchainKHR
);
non_dispatchable_handle!(
    /// `VkImage` — an image resource (here, a swapchain color image).
    VkImage
);
non_dispatchable_handle!(
    /// `VkImageView` — a typed view of one `VkImage` mip/array range.
    VkImageView
);
non_dispatchable_handle!(
    /// `VkSemaphore` — a GPU↔GPU queue-ordering sync primitive.
    VkSemaphore
);

// --- Phase-6 S0 rung-5 sampler handle (non-dispatchable). ---

non_dispatchable_handle!(
    /// `VkSampler` — a texture-sampling state object (filter + address mode).
    VkSampler
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
    /// `VK_SUBOPTIMAL_KHR` — present succeeded but the swapchain no longer matches
    /// the surface optimally (a positive, non-error status; recreate at leisure).
    pub const SUBOPTIMAL_KHR: Self = Self(1_000_001_003);
    /// `VK_ERROR_OUT_OF_DATE_KHR` — the swapchain is incompatible with the surface
    /// (e.g. after a resize) and MUST be recreated before further use.
    pub const ERROR_OUT_OF_DATE_KHR: Self = Self(-1_000_001_004);
    /// `VK_ERROR_SURFACE_LOST_KHR` — the surface is no longer available.
    pub const ERROR_SURFACE_LOST_KHR: Self = Self(-1_000_000_000);

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
            Self::SUBOPTIMAL_KHR => "VK_SUBOPTIMAL_KHR",
            Self::ERROR_OUT_OF_DATE_KHR => "VK_ERROR_OUT_OF_DATE_KHR",
            Self::ERROR_SURFACE_LOST_KHR => "VK_ERROR_SURFACE_LOST_KHR",
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
    SubmitInfo = 4,
    MemoryAllocateInfo = 5,
    FenceCreateInfo = 8,
    /// `VK_STRUCTURE_TYPE_QUERY_POOL_CREATE_INFO` (HW-RT rung R0: the timestamp
    /// query-pool create). Verified against vulkan_core.h: value 11.
    QueryPoolCreateInfo = 11,
    BufferCreateInfo = 12,
    BufferMemoryBarrier = 44,
    ShaderModuleCreateInfo = 16,
    PipelineLayoutCreateInfo = 30,
    ComputePipelineCreateInfo = 29,
    PipelineShaderStageCreateInfo = 18,
    // --- Phase-6 S0 rung-2 graphics-pipeline sub-state sTypes. ---
    GraphicsPipelineCreateInfo = 28,
    PipelineVertexInputStateCreateInfo = 19,
    PipelineInputAssemblyStateCreateInfo = 20,
    PipelineViewportStateCreateInfo = 22,
    PipelineRasterizationStateCreateInfo = 23,
    PipelineMultisampleStateCreateInfo = 24,
    /// `VK_STRUCTURE_TYPE_PIPELINE_DEPTH_STENCIL_STATE_CREATE_INFO` (rung 4).
    PipelineDepthStencilStateCreateInfo = 25,
    PipelineColorBlendStateCreateInfo = 26,
    PipelineDynamicStateCreateInfo = 27,
    /// `VK_STRUCTURE_TYPE_PIPELINE_RENDERING_CREATE_INFO` — the dynamic-rendering
    /// attachment-format chain (no `VkRenderPass`), Vulkan 1.3 core.
    PipelineRenderingCreateInfo = 1_000_044_002,
    /// `VK_STRUCTURE_TYPE_SAMPLER_CREATE_INFO` (Phase-6 S0 rung 5).
    SamplerCreateInfo = 31,
    DescriptorSetLayoutCreateInfo = 32,
    DescriptorPoolCreateInfo = 33,
    DescriptorSetAllocateInfo = 34,
    WriteDescriptorSet = 35,
    CommandPoolCreateInfo = 39,
    CommandBufferAllocateInfo = 40,
    CommandBufferBeginInfo = 42,
    ImageMemoryBarrier = 45,
    SemaphoreCreateInfo = 9,
    ImageViewCreateInfo = 15,
    /// `VK_STRUCTURE_TYPE_IMAGE_CREATE_INFO` (S0 `create_texture`).
    ImageCreateInfo = 14,
    /// `VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_FEATURES_2` (S0 dynamic-rendering query).
    PhysicalDeviceFeatures2 = 1_000_059_000,
    DebugUtilsMessengerCreateInfoExt = 1_000_128_004,
    /// `VK_STRUCTURE_TYPE_VALIDATION_FEATURES_EXT` — chained into the instance
    /// `p_next` to turn on synchronization validation (plan G2).
    ValidationFeaturesExt = 1_000_011_000,
    // --- Slice-1 surface / swapchain / dynamic rendering. ---
    Win32SurfaceCreateInfoKhr = 1_000_009_000,
    SwapchainCreateInfoKhr = 1_000_001_000,
    PresentInfoKhr = 1_000_001_001,
    /// `VkPhysicalDeviceVulkan12Features` — the Vulkan 1.2 aggregate feature struct.
    /// Declared for ABI completeness; NOT used by the T-dev bindless query/enable path
    /// (which reads/writes the GRANULAR `VkPhysicalDeviceDescriptorIndexingFeatures`
    /// instead — see [`VkStructureType::PhysicalDeviceDescriptorIndexingFeatures`] for
    /// why the aggregate is avoided in the `vkCreateDevice` chain).
    /// vulkan_core.h: `VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_VULKAN_1_2_FEATURES = 51`.
    PhysicalDeviceVulkan12Features = 51,
    /// `VkPhysicalDeviceVulkan13Features` — chained to enable dynamic rendering.
    PhysicalDeviceVulkan13Features = 53,
    RenderingInfo = 1_000_044_000,
    RenderingAttachmentInfo = 1_000_044_001,
    /// `VkPhysicalDeviceDescriptorIndexingFeatures` — the GRANULAR bindless feature
    /// struct (T-dev), chained into `VkPhysicalDeviceFeatures2` to READ and into
    /// `VkDeviceCreateInfo` to ENABLE the 5 descriptor-indexing bits `bindless_capable`
    /// gates. Deliberately the granular struct, NOT the `VkPhysicalDeviceVulkan12Features`
    /// aggregate: the aggregate also carries `bufferDeviceAddress`, which would collide
    /// with the hwrt arm's standalone `VkPhysicalDeviceBufferDeviceAddressFeatures` in the
    /// same `pNext` chain (VUID-VkDeviceCreateInfo-pNext-02830 forbids a promoted core
    /// struct's aggregate alongside its own granular sub-struct). vulkan_core.h:
    /// `VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_DESCRIPTOR_INDEXING_FEATURES = 1000161001`.
    PhysicalDeviceDescriptorIndexingFeatures = 1_000_161_001,
    /// `VkPhysicalDeviceHostQueryResetFeatures` — the GRANULAR `hostQueryReset` feature
    /// struct (profiling rung 4), chained into `VkPhysicalDeviceFeatures2` to READ and into
    /// `VkDeviceCreateInfo` to ENABLE. Granular for the same VUID reason as the sibling
    /// above. vulkan_core.h:
    /// `VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_HOST_QUERY_RESET_FEATURES = 1000261000`.
    PhysicalDeviceHostQueryResetFeatures = 1_000_261_000,
    /// `VkDescriptorSetLayoutBindingFlagsCreateInfo` (T4 bindless) — chained into
    /// `VkDescriptorSetLayoutCreateInfo.pNext` to declare the PARTIALLY_BOUND /
    /// UPDATE_AFTER_BIND / VARIABLE_DESCRIPTOR_COUNT flags per binding (the bindless
    /// texture array binding needs all three; the paired immutable-sampler binding
    /// needs none). Same `VK_EXT_descriptor_indexing` extension family as
    /// [`Self::PhysicalDeviceDescriptorIndexingFeatures`] (extension number 161).
    /// vulkan_core.h: `VK_STRUCTURE_TYPE_DESCRIPTOR_SET_LAYOUT_BINDING_FLAGS_CREATE_INFO
    /// = 1000161000`.
    DescriptorSetLayoutBindingFlagsCreateInfo = 1_000_161_000,
    /// `VkDescriptorSetVariableDescriptorCountAllocateInfo` (T4 bindless) — chained
    /// into `VkDescriptorSetAllocateInfo.pNext` to supply the RUNTIME descriptor
    /// count for the layout's VARIABLE_DESCRIPTOR_COUNT binding at allocation time
    /// (the bindless texture array is declared with capacity `N` but allocated with
    /// the actual runtime size — this engine always allocates the full capacity, see
    /// `boyko_rhi_vulkan::bindless`). vulkan_core.h:
    /// `VK_STRUCTURE_TYPE_DESCRIPTOR_SET_VARIABLE_DESCRIPTOR_COUNT_ALLOCATE_INFO =
    /// 1000161003`.
    DescriptorSetVariableDescriptorCountAllocateInfo = 1_000_161_003,
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

/// `VkMemoryHeapFlagBits` (SSAA W2 VRAM probe: `VkMemoryHeap::flags`, distinct field from
/// `VkMemoryType::propertyFlags` above though numerically the same bit per the Vulkan spec).
pub const VK_MEMORY_HEAP_DEVICE_LOCAL_BIT: VkFlags = 0x0000_0001;

/// `VkBufferUsageFlagBits` (subset; the round-trip uses a transfer/storage
/// buffer — the exact bits are immaterial to a host-visible map round-trip but
/// must be a valid usage).
pub const VK_BUFFER_USAGE_TRANSFER_SRC_BIT: VkFlags = 0x0000_0001;
pub const VK_BUFFER_USAGE_TRANSFER_DST_BIT: VkFlags = 0x0000_0002;
/// `VK_BUFFER_USAGE_UNIFORM_BUFFER_BIT`.
pub const VK_BUFFER_USAGE_UNIFORM_BUFFER_BIT: VkFlags = 0x0000_0010;
pub const VK_BUFFER_USAGE_STORAGE_BUFFER_BIT: VkFlags = 0x0000_0020;
/// `VK_BUFFER_USAGE_INDIRECT_BUFFER_BIT`.
pub const VK_BUFFER_USAGE_INDIRECT_BUFFER_BIT: VkFlags = 0x0000_0100;

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

// --- Slice-0 0a (validation) constants. ---

/// `VK_EXT_debug_utils` extension name, as a static NUL-terminated string.
pub const VK_EXT_DEBUG_UTILS_EXTENSION_NAME: &core::ffi::CStr = c"VK_EXT_debug_utils";

/// `VK_EXT_validation_features` extension name. Enabled alongside the validation
/// layer so a `VkValidationFeaturesEXT` chained into `VkInstanceCreateInfo.p_next`
/// (sync-validation, plan G2) is recognized — the loader/layer require the
/// extension to be enabled before they will interpret the chained struct.
pub const VK_EXT_VALIDATION_FEATURES_EXTENSION_NAME: &core::ffi::CStr =
    c"VK_EXT_validation_features";

/// `VkDebugUtilsMessageSeverityFlagBitsEXT`.
pub const VK_DEBUG_UTILS_MESSAGE_SEVERITY_VERBOSE_BIT_EXT: VkFlags = 0x0000_0001;
pub const VK_DEBUG_UTILS_MESSAGE_SEVERITY_INFO_BIT_EXT: VkFlags = 0x0000_0010;
pub const VK_DEBUG_UTILS_MESSAGE_SEVERITY_WARNING_BIT_EXT: VkFlags = 0x0000_0100;
pub const VK_DEBUG_UTILS_MESSAGE_SEVERITY_ERROR_BIT_EXT: VkFlags = 0x0000_1000;

/// `VkDebugUtilsMessageTypeFlagBitsEXT`.
pub const VK_DEBUG_UTILS_MESSAGE_TYPE_GENERAL_BIT_EXT: VkFlags = 0x0000_0001;
pub const VK_DEBUG_UTILS_MESSAGE_TYPE_VALIDATION_BIT_EXT: VkFlags = 0x0000_0002;
pub const VK_DEBUG_UTILS_MESSAGE_TYPE_PERFORMANCE_BIT_EXT: VkFlags = 0x0000_0004;

// --- Slice-0 0c/0d (compute) constants. ---

/// `VkShaderStageFlagBits::VK_SHADER_STAGE_VERTEX_BIT`.
pub const VK_SHADER_STAGE_VERTEX_BIT: VkFlags = 0x0000_0001;
/// `VkShaderStageFlagBits::VK_SHADER_STAGE_FRAGMENT_BIT`.
pub const VK_SHADER_STAGE_FRAGMENT_BIT: VkFlags = 0x0000_0010;
/// `VkShaderStageFlagBits::VK_SHADER_STAGE_COMPUTE_BIT`.
pub const VK_SHADER_STAGE_COMPUTE_BIT: VkFlags = 0x0000_0020;

/// `VkDescriptorType::VK_DESCRIPTOR_TYPE_STORAGE_BUFFER`.
pub const VK_DESCRIPTOR_TYPE_STORAGE_BUFFER: i32 = 7;

/// `VkDescriptorType::VK_DESCRIPTOR_TYPE_COMBINED_IMAGE_SAMPLER` (Phase-6 S0 rung
/// 5: the sampled texture's binding type).
pub const VK_DESCRIPTOR_TYPE_COMBINED_IMAGE_SAMPLER: i32 = 1;

/// `VkDescriptorType::VK_DESCRIPTOR_TYPE_SAMPLED_IMAGE` (Render P1a: a sampled image
/// with the sampler bound separately).
pub const VK_DESCRIPTOR_TYPE_SAMPLED_IMAGE: i32 = 2;

/// `VkDescriptorType::VK_DESCRIPTOR_TYPE_STORAGE_IMAGE` (Render P1a: a compute
/// read/write image — the marcher's output target).
pub const VK_DESCRIPTOR_TYPE_STORAGE_IMAGE: i32 = 3;

/// `VkDescriptorType::VK_DESCRIPTOR_TYPE_UNIFORM_BUFFER` (Render P1a: a read-only
/// constant buffer).
pub const VK_DESCRIPTOR_TYPE_UNIFORM_BUFFER: i32 = 6;

/// `VkDescriptorType::VK_DESCRIPTOR_TYPE_SAMPLER` (T4 bindless: the shared
/// trilinear+anisotropic sampler baked as an IMMUTABLE sampler at the bindless
/// layout's second binding — never written at runtime, so this constant is used
/// only at layout-create time, not in any [`crate::rhi_impl::VulkanBindGroup`]
/// write path).
pub const VK_DESCRIPTOR_TYPE_SAMPLER: i32 = 0;

// --- T4 bindless (`VK_EXT_descriptor_indexing`) binding-flag + create-flag bits.
//     `VkDescriptorBindingFlagBits` values from vulkan_core.h; see
//     `boyko_rhi_vulkan::bindless` for where each is applied. ---

/// `VkDescriptorBindingFlagBits::VK_DESCRIPTOR_BINDING_UPDATE_AFTER_BIND_BIT` — the
/// binding may be updated while a descriptor set using it is bound to a command
/// buffer that is not yet executing, WITHOUT invalidating that command buffer
/// (requires the layout's `UPDATE_AFTER_BIND_POOL` create bit + a pool created with
/// the matching bit).
pub const VK_DESCRIPTOR_BINDING_UPDATE_AFTER_BIND_BIT: VkFlags = 0x0000_0001;
/// `VkDescriptorBindingFlagBits::VK_DESCRIPTOR_BINDING_PARTIALLY_BOUND_BIT` — a
/// descriptor at this binding need not be written for every index the runtime
/// array declares; only slots actually SAMPLED by a shader invocation must hold a
/// valid descriptor. The bindless table still writes the error texture into every
/// slot at init (a stale/unwritten index is a bug-shaped access, not a
/// spec-legal one this bit alone would excuse — see `BindlessTextureTable::new`).
///
/// VALUE PIN (validation-audit fix): the spec's `VkDescriptorBindingFlagBits` are
/// `UPDATE_AFTER_BIND = 0x1`, `UPDATE_UNUSED_WHILE_PENDING = 0x2`,
/// `PARTIALLY_BOUND = 0x4`, `VARIABLE_DESCRIPTOR_COUNT = 0x8`. This constant was
/// mis-pinned at `0x2` — every "partially bound" layout actually requested
/// UPDATE_UNUSED_WHILE_PENDING (whose device feature is never enabled — a
/// validation error) and silently DROPPED partially-bound (making any dynamically
/// unsampled-yet-unwritten slot spec-UB at draw time).
pub const VK_DESCRIPTOR_BINDING_PARTIALLY_BOUND_BIT: VkFlags = 0x0000_0004;
/// `VkDescriptorBindingFlagBits::VK_DESCRIPTOR_BINDING_VARIABLE_DESCRIPTOR_COUNT_BIT`
/// — the LAST binding in the layout may be allocated with a descriptor count `<=`
/// its declared `descriptorCount`, supplied via
/// [`VkDescriptorSetVariableDescriptorCountAllocateInfo`] at allocation time.
pub const VK_DESCRIPTOR_BINDING_VARIABLE_DESCRIPTOR_COUNT_BIT: VkFlags = 0x0000_0008;

/// `VkDescriptorPoolCreateFlagBits::VK_DESCRIPTOR_POOL_CREATE_UPDATE_AFTER_BIND_BIT`
/// — the pool may allocate sets whose layout carries the
/// `UPDATE_AFTER_BIND_POOL` create bit.
pub const VK_DESCRIPTOR_POOL_CREATE_UPDATE_AFTER_BIND_BIT: VkFlags = 0x0000_0002;
/// `VkDescriptorSetLayoutCreateFlagBits::VK_DESCRIPTOR_SET_LAYOUT_CREATE_UPDATE_AFTER_BIND_POOL_BIT`
/// — the layout may be used to allocate a set from an UPDATE_AFTER_BIND pool, and
/// its UPDATE_AFTER_BIND-flagged bindings may be updated after being bound (T4:
/// the whole point of a bindless layout — live incremental per-slot writes with no
/// pipeline/command-buffer rebuild).
pub const VK_DESCRIPTOR_SET_LAYOUT_CREATE_UPDATE_AFTER_BIND_POOL_BIT: VkFlags = 0x0000_0002;

/// `VkPipelineBindPoint::VK_PIPELINE_BIND_POINT_COMPUTE`.
pub const VK_PIPELINE_BIND_POINT_COMPUTE: i32 = 1;

// --- Phase-6 S0 rung-2 graphics-pipeline state constants. ---

/// `VkPipelineBindPoint::VK_PIPELINE_BIND_POINT_GRAPHICS`.
pub const VK_PIPELINE_BIND_POINT_GRAPHICS: i32 = 0;
/// `VkPrimitiveTopology::VK_PRIMITIVE_TOPOLOGY_TRIANGLE_LIST`.
pub const VK_PRIMITIVE_TOPOLOGY_TRIANGLE_LIST: i32 = 3;
/// `VkPolygonMode::VK_POLYGON_MODE_FILL`.
pub const VK_POLYGON_MODE_FILL: i32 = 0;
/// `VkCullModeFlagBits::VK_CULL_MODE_NONE` (rung 2 disables culling so the
/// triangle rasterizes regardless of winding).
pub const VK_CULL_MODE_NONE: VkFlags = 0;
/// `VkCullModeFlagBits::VK_CULL_MODE_FRONT_BIT` (CSM Increment 0: a shadow-map depth
/// pass renders back faces by culling front faces).
pub const VK_CULL_MODE_FRONT_BIT: VkFlags = 0x0000_0001;
/// `VkCullModeFlagBits::VK_CULL_MODE_BACK_BIT` (CSM Increment 0).
pub const VK_CULL_MODE_BACK_BIT: VkFlags = 0x0000_0002;
/// `VkFrontFace::VK_FRONT_FACE_COUNTER_CLOCKWISE`.
pub const VK_FRONT_FACE_COUNTER_CLOCKWISE: i32 = 0;
/// `VkDynamicState::VK_DYNAMIC_STATE_VIEWPORT`.
pub const VK_DYNAMIC_STATE_VIEWPORT: i32 = 0;
/// `VkDynamicState::VK_DYNAMIC_STATE_SCISSOR`.
pub const VK_DYNAMIC_STATE_SCISSOR: i32 = 1;
/// `VkColorComponentFlagBits` — the RGBA write-mask bits (all four = write all
/// channels), so the fragment color reaches every channel of the attachment.
pub const VK_COLOR_COMPONENT_R_BIT: VkFlags = 0x0000_0001;
pub const VK_COLOR_COMPONENT_G_BIT: VkFlags = 0x0000_0002;
pub const VK_COLOR_COMPONENT_B_BIT: VkFlags = 0x0000_0004;
pub const VK_COLOR_COMPONENT_A_BIT: VkFlags = 0x0000_0008;

/// `VkBlendFactor` constants (GUI P5a Decision 3 — the agnostic `BlendFactor`
/// discriminants equal these, asserted in `abi_guard.rs`).
pub const VK_BLEND_FACTOR_ZERO: i32 = 0;
pub const VK_BLEND_FACTOR_ONE: i32 = 1;
pub const VK_BLEND_FACTOR_SRC_ALPHA: i32 = 6;
pub const VK_BLEND_FACTOR_ONE_MINUS_SRC_ALPHA: i32 = 7;
/// `VkBlendOp::VK_BLEND_OP_ADD`.
pub const VK_BLEND_OP_ADD: i32 = 0;

// --- Phase-6 S0 rung-3 vertex-input + index + buffer-usage constants. ---

/// `VkVertexInputRate::VK_VERTEX_INPUT_RATE_VERTEX` — one attribute set per vertex.
pub const VK_VERTEX_INPUT_RATE_VERTEX: i32 = 0;
/// `VkIndexType::VK_INDEX_TYPE_UINT16`.
pub const VK_INDEX_TYPE_UINT16: i32 = 0;
/// `VkIndexType::VK_INDEX_TYPE_UINT32`.
pub const VK_INDEX_TYPE_UINT32: i32 = 1;
/// `VkBufferUsageFlagBits::VK_BUFFER_USAGE_VERTEX_BUFFER_BIT` (rung 3).
pub const VK_BUFFER_USAGE_VERTEX_BUFFER_BIT: VkFlags = 0x0000_0080;
/// `VkBufferUsageFlagBits::VK_BUFFER_USAGE_INDEX_BUFFER_BIT` (rung-3 seam).
pub const VK_BUFFER_USAGE_INDEX_BUFFER_BIT: VkFlags = 0x0000_0040;

/// `VkCommandBufferLevel::VK_COMMAND_BUFFER_LEVEL_PRIMARY`.
pub const VK_COMMAND_BUFFER_LEVEL_PRIMARY: i32 = 0;

/// `VkCommandPoolCreateFlagBits::VK_COMMAND_POOL_CREATE_RESET_COMMAND_BUFFER_BIT`.
pub const VK_COMMAND_POOL_CREATE_RESET_COMMAND_BUFFER_BIT: VkFlags = 0x0000_0002;

/// `VkCommandBufferUsageFlagBits::VK_COMMAND_BUFFER_USAGE_ONE_TIME_SUBMIT_BIT`.
pub const VK_COMMAND_BUFFER_USAGE_ONE_TIME_SUBMIT_BIT: VkFlags = 0x0000_0001;

/// `VkPipelineStageFlagBits::VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT`.
pub const VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT: VkFlags = 0x0000_0800;
/// `VkPipelineStageFlagBits::VK_PIPELINE_STAGE_TRANSFER_BIT`.
pub const VK_PIPELINE_STAGE_TRANSFER_BIT: VkFlags = 0x0000_1000;

/// `VkAccessFlagBits` (subset used by the 0d buffer barrier).
pub const VK_ACCESS_SHADER_READ_BIT: VkFlags = 0x0000_0020;
/// `VK_ACCESS_INDIRECT_COMMAND_READ_BIT` — the only access
/// `VK_PIPELINE_STAGE_DRAW_INDIRECT_BIT` performs. Virtual-geometry rung R1: its absence is
/// why `boyko_render`'s `GpuStage::Indirect` widened to a whole shader/transfer superset.
pub const VK_ACCESS_INDIRECT_COMMAND_READ_BIT: VkFlags = 0x0000_0001;
pub const VK_ACCESS_SHADER_WRITE_BIT: VkFlags = 0x0000_0040;
/// `VkAccessFlagBits::VK_ACCESS_TRANSFER_READ_BIT`.
pub const VK_ACCESS_TRANSFER_READ_BIT: VkFlags = 0x0000_0800;
/// `VkAccessFlagBits::VK_ACCESS_TRANSFER_WRITE_BIT`.
pub const VK_ACCESS_TRANSFER_WRITE_BIT: VkFlags = 0x0000_1000;

/// `VK_QUEUE_FAMILY_IGNORED` — no queue-family-ownership transfer in a barrier.
pub const VK_QUEUE_FAMILY_IGNORED: u32 = u32::MAX;

/// Timeout sentinel for `vkWaitForFences` (wait indefinitely).
pub const VK_TIMEOUT_INFINITE: u64 = u64::MAX;

// --- HW-RT rung R0 — GPU timestamp-query constants. ---

/// `VkQueryType::VK_QUERY_TYPE_TIMESTAMP` — a query that captures the GPU's
/// monotonic timestamp counter at a pipeline stage.
pub const VK_QUERY_TYPE_TIMESTAMP: i32 = 2;

/// `VkQueryResultFlagBits::VK_QUERY_RESULT_64_BIT` — read each result as a 64-bit
/// value (mandatory for timestamps: a 32-bit ~1 ns counter overflows in ~0.43 s).
pub const VK_QUERY_RESULT_64_BIT: VkFlags = 0x0000_0001;
/// `VkQueryResultFlagBits::VK_QUERY_RESULT_WAIT_BIT` — block until the results are
/// available before writing them (paired with the caller's `wait_fence`).
pub const VK_QUERY_RESULT_WAIT_BIT: VkFlags = 0x0000_0002;
/// `VkQueryResultFlagBits::VK_QUERY_RESULT_WITH_AVAILABILITY_BIT` — write an extra
/// availability word after each query's value, non-zero iff that query's result is
/// ready. **This is the bit that makes a non-blocking reader possible**: without it a
/// caller can only learn "ready" by asking the driver to block
/// ([`VK_QUERY_RESULT_WAIT_BIT`]), and a query the recorder never wrote then blocks
/// forever.
///
/// With [`VK_QUERY_RESULT_64_BIT`] also set, the availability word is 64-bit too, so
/// each query occupies **two** `u64` slots — value then availability — and the stride
/// is 16 bytes.
///
/// vulkan_core.h: `VK_QUERY_RESULT_WITH_AVAILABILITY_BIT = 0x00000004`. (`0x10` is
/// `VK_QUERY_RESULT_WITH_STATUS_BIT_KHR`, a different extension's bit, and `0x20` is
/// not defined at all — both are worth naming here because a wrong value does not
/// fail: the driver writes fewer words and the caller reads whatever was in the
/// staging buffer, which is how a stale byte becomes an availability answer.)
pub const VK_QUERY_RESULT_WITH_AVAILABILITY_BIT: VkFlags = 0x0000_0004;

// --- Slice-1 instance / device extension names. ---

/// `VK_KHR_surface` instance-extension name.
pub const VK_KHR_SURFACE_EXTENSION_NAME: &core::ffi::CStr = c"VK_KHR_surface";
/// `VK_KHR_win32_surface` instance-extension name (Windows-only WSI).
pub const VK_KHR_WIN32_SURFACE_EXTENSION_NAME: &core::ffi::CStr = c"VK_KHR_win32_surface";
/// `VK_KHR_swapchain` device-extension name.
pub const VK_KHR_SWAPCHAIN_EXTENSION_NAME: &core::ffi::CStr = c"VK_KHR_swapchain";

// --- HW-RT rung R2a-1 — ray-query device-extension names + AS buffer-usage /
//     build-barrier constants (gated `hwrt`: absent from the default/golden build). ---

/// `VK_KHR_acceleration_structure` device-extension name (HW-RT rung R2a-1).
#[cfg(feature = "hwrt")]
pub const VK_KHR_ACCELERATION_STRUCTURE_EXTENSION_NAME: &core::ffi::CStr =
    c"VK_KHR_acceleration_structure";
/// `VK_KHR_ray_query` device-extension name (inline `rayQuery`; NO ray-tracing-pipeline).
#[cfg(feature = "hwrt")]
pub const VK_KHR_RAY_QUERY_EXTENSION_NAME: &core::ffi::CStr = c"VK_KHR_ray_query";
/// `VK_KHR_deferred_host_operations` device-extension name — a DECLARED dependency of
/// `VK_KHR_acceleration_structure` (must be enabled even though a GPU-build path never
/// calls its API; omitting it fails device create).
#[cfg(feature = "hwrt")]
pub const VK_KHR_DEFERRED_HOST_OPERATIONS_EXTENSION_NAME: &core::ffi::CStr =
    c"VK_KHR_deferred_host_operations";

/// `VkBufferUsageFlagBits::VK_BUFFER_USAGE_SHADER_DEVICE_ADDRESS_BIT` — the buffer can
/// return a device address via `vkGetBufferDeviceAddress` (required for every AS-input /
/// scratch / AS-backing buffer). Consumed at R2a-2.
#[cfg(feature = "hwrt")]
pub const VK_BUFFER_USAGE_SHADER_DEVICE_ADDRESS_BIT: VkFlags = 0x0002_0000;
/// `VkBufferUsageFlagBits::VK_BUFFER_USAGE_ACCELERATION_STRUCTURE_STORAGE_BIT_KHR` — the
/// backing buffer an acceleration structure lives in.
#[cfg(feature = "hwrt")]
pub const VK_BUFFER_USAGE_ACCELERATION_STRUCTURE_STORAGE_BIT_KHR: VkFlags = 0x0010_0000;
/// `VkBufferUsageFlagBits::VK_BUFFER_USAGE_ACCELERATION_STRUCTURE_BUILD_INPUT_READ_ONLY_BIT_KHR`
/// — vertex / index / instance buffers read by an AS build.
#[cfg(feature = "hwrt")]
pub const VK_BUFFER_USAGE_ACCELERATION_STRUCTURE_BUILD_INPUT_READ_ONLY_BIT_KHR: VkFlags =
    0x0008_0000;

/// `VkMemoryAllocateFlagBits::VK_MEMORY_ALLOCATE_DEVICE_ADDRESS_BIT` (HW-RT rung R2a-2) — set
/// in a [`VkMemoryAllocateFlagsInfo`] chained into `VkMemoryAllocateInfo.p_next` so the
/// allocation's buffers can return a device address (the `SHADER_DEVICE_ADDRESS` buffer-usage
/// bit alone is NOT enough; the backing MEMORY must carry this allocation flag too, or the
/// address is garbage — the research-confirmed triple-gate). Consumed by the shared blocks
/// when ray query is enabled.
#[cfg(feature = "hwrt")]
pub const VK_MEMORY_ALLOCATE_DEVICE_ADDRESS_BIT: VkFlags = 0x0000_0002;

/// `VkPipelineStageFlagBits2`-independent 32-bit
/// `VkPipelineStageFlagBits::VK_PIPELINE_STAGE_ACCELERATION_STRUCTURE_BUILD_BIT_KHR` — the
/// stage an AS build runs at (the source stage of the build→read barrier).
#[cfg(feature = "hwrt")]
pub const VK_PIPELINE_STAGE_ACCELERATION_STRUCTURE_BUILD_BIT_KHR: VkFlags = 0x0200_0000;
/// `VkAccessFlagBits::VK_ACCESS_ACCELERATION_STRUCTURE_WRITE_BIT_KHR` — a build's write.
#[cfg(feature = "hwrt")]
pub const VK_ACCESS_ACCELERATION_STRUCTURE_WRITE_BIT_KHR: VkFlags = 0x0040_0000;
/// `VkAccessFlagBits::VK_ACCESS_ACCELERATION_STRUCTURE_READ_BIT_KHR` — a trace's read (the
/// `rayQuery` resolve at R2a-4; the destination access of the build→read barrier).
#[cfg(feature = "hwrt")]
pub const VK_ACCESS_ACCELERATION_STRUCTURE_READ_BIT_KHR: VkFlags = 0x0020_0000;

// --- Slice-1 format / color-space / present-mode / image enums. ---

/// `VkFormat::VK_FORMAT_B8G8R8A8_UNORM` — a universally-supported swapchain format.
pub const VK_FORMAT_B8G8R8A8_UNORM: i32 = 44;
/// `VkFormat::VK_FORMAT_B8G8R8A8_SRGB`.
pub const VK_FORMAT_B8G8R8A8_SRGB: i32 = 50;
/// `VkFormat::VK_FORMAT_R8G8B8A8_UNORM`.
pub const VK_FORMAT_R8G8B8A8_UNORM: i32 = 37;
/// `VkFormat::VK_FORMAT_R8G8B8A8_SRGB`.
pub const VK_FORMAT_R8G8B8A8_SRGB: i32 = 43;
/// `VkFormat::VK_FORMAT_UNDEFINED`.
pub const VK_FORMAT_UNDEFINED: i32 = 0;
/// `VkFormat::VK_FORMAT_R8_SNORM` — a single signed-normalized 8-bit channel (SDF
/// brick-atlas campaign: the quantized narrow-band distance the brick atlas stores).
///
/// BUG-M2-GPU-1: this was 9, which is actually `VK_FORMAT_R8_UNORM`; `VK_FORMAT_R8_SNORM`
/// is 10. The wrong value made the atlas image+view UNORM, so the sampler returned
/// `byte/255` instead of the signed `byte/127`, collapsing the M2 cubic (a value 127
/// decoded to 0.498 instead of 1.0). The `abi_guard` assert passed only because the
/// `Format::R8Snorm` enum discriminant carried the SAME wrong value.
pub const VK_FORMAT_R8_SNORM: i32 = 10;
/// `VkFormat::VK_FORMAT_R8_UNORM` — a single unsigned-normalized 8-bit channel mapping the
/// byte range onto `[0, 1]` (Render P7: the SSAO term `gSsao` — a full-res STORAGE image the
/// resolve loads under the `ssao_mode != 0` gate). `VK_FORMAT_R8_UNORM` is 9.
pub const VK_FORMAT_R8_UNORM: i32 = 9;
/// `VkFormat::VK_FORMAT_R8G8_UNORM` — two unsigned-normalized 8-bit channels (Rung 3a: the
/// RT soft-shadow VISIBILITY target `shadow_vis`, R = mesh visibility, G = validity). The
/// value is 16 (the 8-bit two-component UNORM block) — the M2 lesson: the const is pinned to
/// the ACTUAL enumerant, cross-checked against `Format::R8G8Unorm` in `abi_guard`.
pub const VK_FORMAT_R8G8_UNORM: i32 = 16;
/// `VkFormat::VK_FORMAT_R16_SFLOAT` — a single 16-bit (half) float (SDF brick-atlas
/// campaign M2: the D8 atlas fallback when `R8_SNORM` lacks the linear-filter feature;
/// half-float carries the narrow-band distance with NO quantization, so the `EPSILON_Q`
/// store bias is harmless there).
pub const VK_FORMAT_R16_SFLOAT: i32 = 76;
/// `VkFormat::VK_FORMAT_R16G16_UNORM` — two unsigned-normalized 16-bit channels (Rung 3a: the
/// à-trous ping-pong target `shadow_vis2`; 16-bit avoids the cumulative 8-bit rounding of a
/// multi-level filter). The value is 77 (the 16-bit two-component UNORM block: R16_UNORM=70,
/// R16G16_UNORM=77) — pinned to the ACTUAL enumerant, cross-checked against
/// `Format::R16G16Unorm` in `abi_guard`.
pub const VK_FORMAT_R16G16_UNORM: i32 = 77;
/// `VkFormat::VK_FORMAT_R16_UNORM` — a single unsigned-normalized 16-bit channel (the SSAO
/// à-trous denoise chain's interior ping-pong ring; 16-bit avoids the cumulative 8-bit rounding
/// of a multi-level filter, one channel narrower than [`VK_FORMAT_R16G16_UNORM`]). The value is
/// 70 (the 16-bit single-component UNORM block) — pinned to the ACTUAL enumerant, cross-checked
/// against `Format::R16Unorm` in `abi_guard`.
pub const VK_FORMAT_R16_UNORM: i32 = 70;
/// `VkFormat::VK_FORMAT_R16G16_SFLOAT` — two 16-bit (half) floats (SDFDDGI I1: the probe
/// DEPTH/visibility atlas's two Chebyshev moments `E[d]`/`E[d²]`). The value is 83 (the
/// 16-bit-per-component SFLOAT block: R16=76, R16G16=83) — the M2 lesson: the const is
/// pinned to the ACTUAL enumerant, cross-checked against `Format::R16G16Sfloat` in
/// `abi_guard`.
pub const VK_FORMAT_R16G16_SFLOAT: i32 = 83;
/// `VkFormat::VK_FORMAT_R16G16B16A16_UNORM` — four 16-bit UNORM channels (HW-RT Rung 3b: the
/// temporal shadow-vis history ring — vis / confidence / prev-depth / reserved). The value is 91
/// (the 16-bit four-component UNORM block: R16=70, R16G16=77, R16G16B16A16=91) — pinned to the
/// ACTUAL enumerant, cross-checked against `Format::R16G16B16A16Unorm` in `abi_guard`.
pub const VK_FORMAT_R16G16B16A16_UNORM: i32 = 91;
/// `VkFormat::VK_FORMAT_R16G16B16A16_SFLOAT` — four 16-bit (half) floats (textured-PBR T6a: the
/// `gPbr` deferred-resolve MRT lane — metallic/roughness/AO-modulation/emissive-modulation). The
/// value is 97 (the 16-bit four-component SFLOAT block: R16=76, R16G16=83, R16G16B16A16_SFLOAT=97)
/// — pinned to the ACTUAL enumerant, cross-checked against `Format::R16G16B16A16Sfloat` in
/// `abi_guard`.
pub const VK_FORMAT_R16G16B16A16_SFLOAT: i32 = 97;
/// `VkFormat::VK_FORMAT_R32_SFLOAT` — a single 32-bit float (Lighting L0b: the
/// `gViewT` G-buffer storage-image lane carrying the marcher's surface ray param `t`).
pub const VK_FORMAT_R32_SFLOAT: i32 = 100;
/// `VkFormat::VK_FORMAT_R32G32_UINT` — two 32-bit unsigned integers (Multi-paradigm render-path
/// plan, rung R8: the `vb_id` Visibility-Buffer id channel — `R` = `instance_id`, `G` = raw
/// `SV_PrimitiveID`, Decision 9). The value is 101 (the 32-bit two-component UINT block:
/// R32=100, R32G32=101..103, R32G32_UINT=101) — pinned to the ACTUAL enumerant, cross-checked
/// against `Format::R32G32Uint` in `abi_guard`.
pub const VK_FORMAT_R32G32_UINT: i32 = 101;
/// `VkFormat::VK_FORMAT_R32G32_SFLOAT` — two 32-bit floats (textured-PBR T6c: a vec2
/// vertex UV coordinate). The value is 103 (the 32-bit two-component SFLOAT block:
/// R32=100, R32G32=101..103, R32G32_SFLOAT=103) — pinned to the ACTUAL enumerant,
/// cross-checked against `VertexFormat::Float32x2` in `abi_guard`.
pub const VK_FORMAT_R32G32_SFLOAT: i32 = 103;
/// `VkFormat::VK_FORMAT_R32G32B32_SFLOAT` — three 32-bit floats (a vec3 vertex
/// position, Phase-6 S0 rung 3).
pub const VK_FORMAT_R32G32B32_SFLOAT: i32 = 106;
/// `VkFormat::VK_FORMAT_R32G32B32A32_SFLOAT` — four 32-bit floats (a vec4 vertex
/// color, Phase-6 S0 rung 3).
pub const VK_FORMAT_R32G32B32A32_SFLOAT: i32 = 109;
/// `VkFormat::VK_FORMAT_B10G11R11_UFLOAT_PACK32` — the packed R11G11B10 unsigned-float HDR
/// format (SDFDDGI I1: the probe IRRADIANCE atlas, Decision D6 — `R11G11B10F`-no-gamma). The
/// value is 122 (the packed-32 specials block); there is NO `VK_FORMAT_R11G11B10_*` — this
/// B10G11R11 packing is the only Vulkan format for it (the M2 lesson: validate the ACTUAL
/// enumerant, cross-checked against `Format::B10G11R11UfloatPack32` in `abi_guard`).
pub const VK_FORMAT_B10G11R11_UFLOAT_PACK32: i32 = 122;
/// `VkFormat::VK_FORMAT_D32_SFLOAT` — a 32-bit float depth attachment (Phase-6 S0
/// rung 4). Spec-mandated as a depth attachment on every conformant device.
pub const VK_FORMAT_D32_SFLOAT: i32 = 126;

/// `VkFormatFeatureFlagBits::VK_FORMAT_FEATURE_STORAGE_IMAGE_BIT` — the OPTIMAL-tiling
/// capability the Render P1b G-buffer storage images require (a compute store into an
/// `R8G8B8A8_UNORM` image). Queried via `vkGetPhysicalDeviceFormatProperties` at
/// device-create for the [`crate::device::DeviceCaps`] fail-fast.
// vulkan_core.h: `VK_FORMAT_FEATURE_STORAGE_IMAGE_BIT = 0x00000002`. (`0x8` is
// `VK_FORMAT_FEATURE_UNIFORM_TEXEL_BUFFER_BIT`, a buffer feature never set in an
// image's `optimalTilingFeatures` — the prior wrong value fail-fast'd boot on
// capable GPUs.)
pub const VK_FORMAT_FEATURE_STORAGE_IMAGE_BIT: VkFlags = 0x0000_0002;

// Value guard for the hand-typed P1b format-feature bit. A struct-size assert can
// only catch a bad LAYOUT, not a bad flag VALUE — which is exactly how the wrong
// `0x8` slipped past review. Pin the header value (`0x2`), require a single set bit
// (a power of two, as a format-feature flag must be), and assert it is DISTINCT from
// the `UNIFORM_TEXEL_BUFFER` bit (`0x8`) it was previously confused with.
const _: () = assert!(
    VK_FORMAT_FEATURE_STORAGE_IMAGE_BIT == 0x2,
    "VK_FORMAT_FEATURE_STORAGE_IMAGE_BIT must equal the vulkan_core.h value 0x00000002"
);
const _: () = assert!(
    VK_FORMAT_FEATURE_STORAGE_IMAGE_BIT.is_power_of_two(),
    "a format-feature flag bit must be a single set bit (power of two)"
);
const _: () = assert!(
    // 0x8 = VK_FORMAT_FEATURE_UNIFORM_TEXEL_BUFFER_BIT (vulkan_core.h) — the wrong
    // transcription this guard exists to reject.
    VK_FORMAT_FEATURE_STORAGE_IMAGE_BIT != 0x8,
    "STORAGE_IMAGE bit collides with the UNIFORM_TEXEL_BUFFER bit (0x8)"
);

/// `VkFormatFeatureFlagBits::VK_FORMAT_FEATURE_SAMPLED_IMAGE_FILTER_LINEAR_BIT` — the
/// OPTIMAL-tiling capability the SDF brick-atlas campaign (M2) requires of the chosen
/// atlas format: a SAMPLED image must support `VK_FILTER_LINEAR` so the hardware
/// trilinear fetch of the `R8_SNORM` brick atlas is well-defined. Queried via
/// `vkGetPhysicalDeviceFormatProperties` at device-create for the
/// [`crate::device::DeviceCaps::atlas_linear_filter_ok`] probe. When `R8_SNORM` lacks
/// it the probe falls the atlas back to `R16_SFLOAT` (which supports linear filtering
/// on every conformant GPU per the Vulkan spec's mandatory-format table).
// vulkan_core.h: `VK_FORMAT_FEATURE_SAMPLED_IMAGE_FILTER_LINEAR_BIT = 0x00001000`.
pub const VK_FORMAT_FEATURE_SAMPLED_IMAGE_FILTER_LINEAR_BIT: VkFlags = 0x0000_1000;

// Value guard for the hand-typed M2 format-feature bit (same discipline as the
// STORAGE_IMAGE guard above): pin the header value, require a single set bit, and assert
// it is DISTINCT from the STORAGE_IMAGE bit it sits near.
const _: () = assert!(
    VK_FORMAT_FEATURE_SAMPLED_IMAGE_FILTER_LINEAR_BIT == 0x1000,
    "VK_FORMAT_FEATURE_SAMPLED_IMAGE_FILTER_LINEAR_BIT must equal the vulkan_core.h value 0x00001000"
);
const _: () = assert!(
    VK_FORMAT_FEATURE_SAMPLED_IMAGE_FILTER_LINEAR_BIT.is_power_of_two(),
    "a format-feature flag bit must be a single set bit (power of two)"
);
const _: () = assert!(
    VK_FORMAT_FEATURE_SAMPLED_IMAGE_FILTER_LINEAR_BIT != VK_FORMAT_FEATURE_STORAGE_IMAGE_BIT,
    "SAMPLED_IMAGE_FILTER_LINEAR bit collides with the STORAGE_IMAGE bit"
);

/// `VkFormatFeatureFlagBits::VK_FORMAT_FEATURE_COLOR_ATTACHMENT_BIT` — the OPTIMAL-tiling
/// capability Render P5-r0 requires of the `R8G8B8A8_UNORM` G-buffer images: the mesh
/// raster pass A now writes albedo/normal/material as MRT COLOR attachments (alongside
/// their STORAGE usage), so the format must be color-attachment-renderable. RGBA8_UNORM
/// color-attachment renderability is mandatory in Vulkan, so the boot fail-fast
/// ([`crate::device::DeviceCaps::gbuffer_color_attachment_format_ok`]) passes universally
/// — the explicit gate is the project's fail-fast discipline (no validation oracle on
/// this box, so an unsupported usage must abort at boot with a clear message, not as a
/// device-lost). Queried via `vkGetPhysicalDeviceFormatProperties` at device-create.
// vulkan_core.h: `VK_FORMAT_FEATURE_COLOR_ATTACHMENT_BIT = 0x00000080`.
pub const VK_FORMAT_FEATURE_COLOR_ATTACHMENT_BIT: VkFlags = 0x0000_0080;

// Value guard for the hand-typed P5-r0 format-feature bit (same discipline as the
// STORAGE_IMAGE / SAMPLED_IMAGE_FILTER_LINEAR guards): pin the header value, require a
// single set bit, and assert it is DISTINCT from the two bits it sits among.
const _: () = assert!(
    VK_FORMAT_FEATURE_COLOR_ATTACHMENT_BIT == 0x80,
    "VK_FORMAT_FEATURE_COLOR_ATTACHMENT_BIT must equal the vulkan_core.h value 0x00000080"
);
const _: () = assert!(
    VK_FORMAT_FEATURE_COLOR_ATTACHMENT_BIT.is_power_of_two(),
    "a format-feature flag bit must be a single set bit (power of two)"
);
const _: () = assert!(
    VK_FORMAT_FEATURE_COLOR_ATTACHMENT_BIT != VK_FORMAT_FEATURE_STORAGE_IMAGE_BIT
        && VK_FORMAT_FEATURE_COLOR_ATTACHMENT_BIT != VK_FORMAT_FEATURE_SAMPLED_IMAGE_FILTER_LINEAR_BIT,
    "COLOR_ATTACHMENT bit collides with the STORAGE_IMAGE / SAMPLED_IMAGE_FILTER_LINEAR bit"
);

/// `VkColorSpaceKHR::VK_COLOR_SPACE_SRGB_NONLINEAR_KHR` — the always-present space.
pub const VK_COLOR_SPACE_SRGB_NONLINEAR_KHR: i32 = 0;

/// `VkPresentModeKHR::VK_PRESENT_MODE_IMMEDIATE_KHR` — present as soon as submitted, tearing
/// allowed. **Optional**: profiling rung 8 D12 probes it and falls back to FIFO with a notice.
pub const VK_PRESENT_MODE_IMMEDIATE_KHR: i32 = 0;
/// `VkPresentModeKHR::VK_PRESENT_MODE_MAILBOX_KHR` — one queued image, replaced rather than
/// blocked. Optional; declared for the probe's vocabulary.
pub const VK_PRESENT_MODE_MAILBOX_KHR: i32 = 1;
/// `VkPresentModeKHR::VK_PRESENT_MODE_FIFO_KHR` — the only mode the spec guarantees.
pub const VK_PRESENT_MODE_FIFO_KHR: i32 = 2;

/// `VkImageUsageFlagBits::VK_IMAGE_USAGE_COLOR_ATTACHMENT_BIT`.
pub const VK_IMAGE_USAGE_COLOR_ATTACHMENT_BIT: VkFlags = 0x0000_0010;
/// `VkImageUsageFlagBits::VK_IMAGE_USAGE_TRANSFER_DST_BIT`.
pub const VK_IMAGE_USAGE_TRANSFER_DST_BIT: VkFlags = 0x0000_0002;
/// `VkImageUsageFlagBits::VK_IMAGE_USAGE_TRANSFER_SRC_BIT`.
pub const VK_IMAGE_USAGE_TRANSFER_SRC_BIT: VkFlags = 0x0000_0001;
/// `VkImageUsageFlagBits::VK_IMAGE_USAGE_SAMPLED_BIT`.
pub const VK_IMAGE_USAGE_SAMPLED_BIT: VkFlags = 0x0000_0004;
/// `VkImageUsageFlagBits::VK_IMAGE_USAGE_STORAGE_BIT`.
pub const VK_IMAGE_USAGE_STORAGE_BIT: VkFlags = 0x0000_0008;
/// `VkImageUsageFlagBits::VK_IMAGE_USAGE_DEPTH_STENCIL_ATTACHMENT_BIT` (Phase-6 S0
/// rung 4: the depth buffer).
pub const VK_IMAGE_USAGE_DEPTH_STENCIL_ATTACHMENT_BIT: VkFlags = 0x0000_0020;

/// `VkImageType` discriminants (S0 `create_texture`).
pub const VK_IMAGE_TYPE_2D: i32 = 1;
/// `VkImageType::VK_IMAGE_TYPE_3D` (deferred SDF storage image).
pub const VK_IMAGE_TYPE_3D: i32 = 2;
/// `VkImageViewType::VK_IMAGE_VIEW_TYPE_3D`.
pub const VK_IMAGE_VIEW_TYPE_3D: i32 = 2;

/// `VkImageTiling::VK_IMAGE_TILING_OPTIMAL`.
pub const VK_IMAGE_TILING_OPTIMAL: i32 = 0;

/// `VkImageCreateFlagBits::VK_IMAGE_CREATE_MUTABLE_FORMAT_BIT` (textured-PBR T2
/// Decision D2): the image may be viewed through an image view of a DIFFERENT but
/// compatible format than the image's own — the sRGB-view trick (a mutable
/// `R8G8B8A8_UNORM` image sampled through an `R8G8B8A8_SRGB` view). Set only when
/// [`boyko_rhi::TextureDesc::view_format`] is `Some(f)` with `f != format`; `0`
/// (the byte-identical default) for every pre-T2 texture.
pub const VK_IMAGE_CREATE_MUTABLE_FORMAT_BIT: VkFlags = 0x0000_0008;

/// `VkImageLayout` discriminants used by the S0 transfer/storage transitions
/// (the buffer-path `VK_ACCESS_TRANSFER_*`/stage consts are reused for images).
pub const VK_IMAGE_LAYOUT_GENERAL: i32 = 1;
pub const VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL: i32 = 6;
pub const VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL: i32 = 7;

/// `VkSurfaceTransformFlagBitsKHR::VK_SURFACE_TRANSFORM_IDENTITY_BIT_KHR`.
pub const VK_SURFACE_TRANSFORM_IDENTITY_BIT_KHR: VkFlags = 0x0000_0001;
/// `VkCompositeAlphaFlagBitsKHR::VK_COMPOSITE_ALPHA_OPAQUE_BIT_KHR`.
pub const VK_COMPOSITE_ALPHA_OPAQUE_BIT_KHR: VkFlags = 0x0000_0001;

/// `VkImageViewType::VK_IMAGE_VIEW_TYPE_2D`.
pub const VK_IMAGE_VIEW_TYPE_2D: i32 = 1;
/// `VkImageViewType::VK_IMAGE_VIEW_TYPE_2D_ARRAY` (CSM Increment 0: the array SAMPLE
/// view over a multi-layer depth texture — the resolve samples `float3(uv, layer)`).
pub const VK_IMAGE_VIEW_TYPE_2D_ARRAY: i32 = 5;

/// `VkImageAspectFlagBits::VK_IMAGE_ASPECT_COLOR_BIT`.
pub const VK_IMAGE_ASPECT_COLOR_BIT: VkFlags = 0x0000_0001;
/// `VkImageAspectFlagBits::VK_IMAGE_ASPECT_DEPTH_BIT` (Phase-6 S0 rung 4).
pub const VK_IMAGE_ASPECT_DEPTH_BIT: VkFlags = 0x0000_0002;

/// `VkComponentSwizzle::VK_COMPONENT_SWIZZLE_IDENTITY`.
pub const VK_COMPONENT_SWIZZLE_IDENTITY: i32 = 0;

/// `VkImageLayout` discriminants used by the present barriers.
pub const VK_IMAGE_LAYOUT_UNDEFINED: i32 = 0;
pub const VK_IMAGE_LAYOUT_PRESENT_SRC_KHR: i32 = 1_000_001_002;
/// `VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL`.
pub const VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL: i32 = 2;
/// `VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL` (Phase-6 S0 rung 5: the layout a
/// sampled texture must be in for a COMBINED_IMAGE_SAMPLER read).
pub const VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL: i32 = 5;
/// `VK_IMAGE_LAYOUT_DEPTH_ATTACHMENT_OPTIMAL` (Vulkan 1.2 core, Phase-6 S0 rung 4).
pub const VK_IMAGE_LAYOUT_DEPTH_ATTACHMENT_OPTIMAL: i32 = 1_000_241_000;

/// `VkPipelineStageFlagBits` used by the present barriers / submit wait stage.
pub const VK_PIPELINE_STAGE_TOP_OF_PIPE_BIT: VkFlags = 0x0000_0001;
/// `VK_PIPELINE_STAGE_DRAW_INDIRECT_BIT` — the stage that FETCHES indirect draw/dispatch
/// arguments from a buffer. Virtual-geometry rung R1.
pub const VK_PIPELINE_STAGE_DRAW_INDIRECT_BIT: VkFlags = 0x0000_0002;
/// `VK_PIPELINE_STAGE_VERTEX_SHADER_BIT` (Pillar B B3: the interp draw SSBO is READ by the
/// raster + shadow VERTEX shaders — the destination stage of the COMPUTE→VERTEX RAW barrier
/// the framegraph derives after the interp compute writes the interpolated model columns).
pub const VK_PIPELINE_STAGE_VERTEX_SHADER_BIT: VkFlags = 0x0000_0008;
/// `VK_PIPELINE_STAGE_FRAGMENT_SHADER_BIT` (Phase-6 S0 rung 5: the COLOR → SHADER_READ
/// barrier's destination stage — the sampling draw's fragment stage waits on it).
pub const VK_PIPELINE_STAGE_FRAGMENT_SHADER_BIT: VkFlags = 0x0000_0080;
pub const VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT: VkFlags = 0x0000_0400;
/// `VK_PIPELINE_STAGE_EARLY_FRAGMENT_TESTS_BIT` (Phase-6 S0 rung 4 depth barrier).
pub const VK_PIPELINE_STAGE_EARLY_FRAGMENT_TESTS_BIT: VkFlags = 0x0000_0100;
/// `VK_PIPELINE_STAGE_LATE_FRAGMENT_TESTS_BIT` (Phase-6 S0 rung 4 depth barrier).
pub const VK_PIPELINE_STAGE_LATE_FRAGMENT_TESTS_BIT: VkFlags = 0x0000_0200;
pub const VK_PIPELINE_STAGE_BOTTOM_OF_PIPE_BIT: VkFlags = 0x0000_2000;

/// `VkAccessFlagBits` used by the color-attachment present barriers.
pub const VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT: VkFlags = 0x0000_0100;
/// `VK_ACCESS_COLOR_ATTACHMENT_READ_BIT` — a `loadOp = LOAD` attachment access (the
/// composite→UI same-image barrier in `record_present_sampled`).
pub const VK_ACCESS_COLOR_ATTACHMENT_READ_BIT: VkFlags = 0x0000_0080;
/// `VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_READ_BIT` (Phase-6 S0 rung 4 depth barrier).
pub const VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_READ_BIT: VkFlags = 0x0000_0200;
/// `VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_WRITE_BIT` (Phase-6 S0 rung 4 depth barrier).
pub const VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_WRITE_BIT: VkFlags = 0x0000_0400;

/// `VkCompareOp::VK_COMPARE_OP_LESS` — the rung-4 depth-test compare op (a smaller
/// fragment `z` passes, i.e. nearer geometry wins).
pub const VK_COMPARE_OP_LESS: i32 = 1;
/// `VkCompareOp::VK_COMPARE_OP_LESS_OR_EQUAL` (CSM Increment 0: the comparison
/// sampler's PCF op — `reference <= stored_depth` passes, so a fragment at the stored
/// depth is lit, not self-shadowed).
pub const VK_COMPARE_OP_LESS_OR_EQUAL: i32 = 3;
/// `VkCompareOp::VK_COMPARE_OP_ALWAYS` (CSM Increment 0: pinned in `abi_guard.rs` for
/// the agnostic [`CompareOp::Always`](boyko_rhi::enums::CompareOp) discriminant).
pub const VK_COMPARE_OP_ALWAYS: i32 = 7;
/// `VkCompareOp::VK_COMPARE_OP_GREATER` — multi-paradigm render-path plan, rung R4b-b
/// (Decision 4): the Forward path's reverse-Z depth-test compare op (a LARGER stored
/// depth is nearer under reverse-Z, so the fragment with the greater `z` wins).
pub const VK_COMPARE_OP_GREATER: i32 = 4;
/// `VkCompareOp::VK_COMPARE_OP_EQUAL` — multi-paradigm render-path plan, rung R5
/// (ForwardPlus): the EQUAL-depth zero-overdraw compare op `forward_opaque` tests
/// against under `ForwardPlus` (depth-write OFF), after `depth_prepass` has already
/// written the exact same reverse-Z value with `VK_COMPARE_OP_GREATER` — a fragment
/// survives only if its interpolated depth exactly matches the prepass-written value,
/// so hardware early-Z rejects every occluded fragment before the inline shade runs.
pub const VK_COMPARE_OP_EQUAL: i32 = 2;

/// `VkAttachmentLoadOp` / `VkAttachmentStoreOp` discriminants for dynamic rendering.
pub const VK_ATTACHMENT_LOAD_OP_LOAD: i32 = 0;
pub const VK_ATTACHMENT_LOAD_OP_CLEAR: i32 = 1;
pub const VK_ATTACHMENT_LOAD_OP_DONT_CARE: i32 = 2;
pub const VK_ATTACHMENT_STORE_OP_STORE: i32 = 0;
pub const VK_ATTACHMENT_STORE_OP_DONT_CARE: i32 = 1;

/// `VkSampleCountFlagBits::VK_SAMPLE_COUNT_1_BIT`.
pub const VK_SAMPLE_COUNT_1_BIT: VkFlags = 0x0000_0001;

// --- Phase-6 S0 rung-5 sampler-state constants (`vkCreateSampler`). ---

/// `VkFilter::VK_FILTER_NEAREST` — nearest-texel sampling (rung-5 1:1 sample).
pub const VK_FILTER_NEAREST: i32 = 0;
/// `VkFilter::VK_FILTER_LINEAR` — bilinear interpolation.
pub const VK_FILTER_LINEAR: i32 = 1;
/// `VkSamplerMipmapMode::VK_SAMPLER_MIPMAP_MODE_LINEAR` — interpolated (trilinear
/// when paired with `VK_FILTER_LINEAR` mag/min) mip sampling. T4: the bindless
/// table's shared sampler.
pub const VK_SAMPLER_MIPMAP_MODE_LINEAR: i32 = 1;
/// `VkSamplerMipmapMode::VK_SAMPLER_MIPMAP_MODE_NEAREST` — no mip interpolation
/// (rung-5 textures have a single mip level).
pub const VK_SAMPLER_MIPMAP_MODE_NEAREST: i32 = 0;
/// `VkSamplerAddressMode::VK_SAMPLER_ADDRESS_MODE_REPEAT`.
pub const VK_SAMPLER_ADDRESS_MODE_REPEAT: i32 = 0;
/// `VkSamplerAddressMode::VK_SAMPLER_ADDRESS_MODE_CLAMP_TO_EDGE` (rung 5).
pub const VK_SAMPLER_ADDRESS_MODE_CLAMP_TO_EDGE: i32 = 2;
/// `VkBorderColor::VK_BORDER_COLOR_FLOAT_OPAQUE_BLACK` — unused under
/// CLAMP_TO_EDGE, but a valid required value for the create-info field.
pub const VK_BORDER_COLOR_FLOAT_OPAQUE_BLACK: i32 = 0;
/// `VkCompareOp::VK_COMPARE_OP_NEVER` — the sampler's (disabled) compare op.
pub const VK_COMPARE_OP_NEVER: i32 = 0;

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
    /// `const VkPhysicalDeviceFeatures*` (T-dev: points to a stack-local
    /// [`VkPhysicalDeviceFeatures`] enabling `samplerAnisotropy`). NEVER combined with a
    /// `VkPhysicalDeviceFeatures2` in `pNext` — the two are mutually exclusive
    /// (VUID-VkDeviceCreateInfo-pNext-00373).
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

// Documented byte offsets of the `maxPerStageDescriptor*` fields inside `VkPhysicalDeviceLimits`
// (the SDFDDGI I0 device-limit check reads them out of the opaque blob — the FFI does not expose the
// struct typed). The `VkPhysicalDeviceLimits` field order is spec-fixed: the leading `u32`s run
// `maxImageDimension1D/2D/3D/Cube`, `maxImageArrayLayers`, `maxTexelBufferElements`,
// `maxUniformBufferRange`, `maxStorageBufferRange`, `maxPushConstantsSize`,
// `maxMemoryAllocationCount`, `maxSamplerAllocationCount` (11 × 4 = 44 B), then 4 B pad to the
// 8-aligned `VkDeviceSize bufferImageGranularity` @48 + `sparseAddressSpaceSize` @56, then
// `maxBoundDescriptorSets` @64, and the six per-stage descriptor caps @68..92 in the order below.
/// Offset of `maxImageDimension2D` (`u32`) within `VkPhysicalDeviceLimits` — the SECOND
/// leading `u32` (`maxImageDimension1D` is @0). SSAA W2: the boot device probe reads this
/// to decide whether `native * 2` fits the device's max 2D image extent on both axes
/// before arming the 2× render scale.
pub const LIMITS_OFF_MAX_IMAGE_DIMENSION_2D: usize = 4;
/// Offset of `maxPerStageDescriptorSamplers` (`u32`) within `VkPhysicalDeviceLimits`.
pub const LIMITS_OFF_MAX_PER_STAGE_SAMPLERS: usize = 68;
/// Offset of `maxPerStageDescriptorUniformBuffers` (`u32`).
pub const LIMITS_OFF_MAX_PER_STAGE_UNIFORM_BUFFERS: usize = 72;
/// Offset of `maxPerStageDescriptorStorageBuffers` (`u32`).
pub const LIMITS_OFF_MAX_PER_STAGE_STORAGE_BUFFERS: usize = 76;
/// Offset of `maxPerStageDescriptorSampledImages` (`u32`).
pub const LIMITS_OFF_MAX_PER_STAGE_SAMPLED_IMAGES: usize = 80;
/// Offset of `maxPerStageDescriptorStorageImages` (`u32`).
pub const LIMITS_OFF_MAX_PER_STAGE_STORAGE_IMAGES: usize = 84;
/// Offset of `maxBoundDescriptorSets` (`u32`) within `VkPhysicalDeviceLimits` — the
/// field immediately preceding `maxPerStageDescriptorSamplers` @68 (see the field-order
/// comment above `LIMITS_OFF_MAX_IMAGE_DIMENSION_2D`). Multi-paradigm render-path plan,
/// rung R-VBGEO (Decision 0 / P2-c): `MeshGeometryTable::new` asserts this is `>= 4`
/// (the `VisibilityBuffer` path's Set-3 geometry table needs a 4th bound descriptor set
/// alongside Set 0/1/2 — the Vulkan-guaranteed floor).
pub const LIMITS_OFF_MAX_BOUND_DESCRIPTOR_SETS: usize = 64;

// The read offsets must lie inside the blob (the last field read is a `u32` at 84 → 84..88 <= 504).
const _: () = assert!(LIMITS_OFF_MAX_PER_STAGE_STORAGE_IMAGES + 4 <= 504);
const _: () = assert!(LIMITS_OFF_MAX_IMAGE_DIMENSION_2D + 4 <= 504);
const _: () = assert!(LIMITS_OFF_MAX_BOUND_DESCRIPTOR_SETS + 4 <= 504);

/// Offset of `timestampPeriod` (`float`) within `VkPhysicalDeviceLimits` (HW-RT rung
/// R0). Re-derived from the in-repo anchor `maxPerStageDescriptorStorageImages == 84`
/// by walking the spec-fixed field order forward: the trailing block runs `…,
/// maxSampleMaskWords (u32)`, `timestampComputeAndGraphics (VkBool32) @420`,
/// `timestampPeriod (float) @424` — the last two 4-byte scalars before the
/// sample-count-flags / image-limit tail. A runtime plausibility guard (a period
/// outside `(0, 1000)` ns/tick ⇒ treat as unusable) degrades a WRONG offset to a
/// graceful skip, never to fake timings — so a byte-offset drift can never produce a
/// bogus measurement.
pub const LIMITS_OFF_TIMESTAMP_PERIOD: usize = 424;

/// Offset of `timestampComputeAndGraphics` (`VkBool32`) within `VkPhysicalDeviceLimits` — the
/// field immediately PRECEDING [`LIMITS_OFF_TIMESTAMP_PERIOD`] in the spec-fixed order the
/// comment above already walks (`…, maxSampleMaskWords (u32)`,
/// `timestampComputeAndGraphics (VkBool32) @420`, `timestampPeriod (float) @424`).
///
/// VB-SV0 rung S1.5: read so a timing harness can state whether the GRAPHICS+COMPUTE queue
/// families are all guaranteed to support timestamps (`VK_TRUE`), rather than relying solely on
/// the chosen family's `timestampValidBits`. A `VK_FALSE` device is not a failure — the per-family
/// `timestampValidBits` check already gates usability — but a bench that reports its own
/// resolution should report which of the two guarantees it is standing on. RECORDED ONLY: nothing
/// branches on it (see [`crate::device::DeviceCaps::timestamps_usable`], unchanged).
pub const LIMITS_OFF_TIMESTAMP_COMPUTE_AND_GRAPHICS: usize = 420;

// The `f32` read at 424 must lie inside the blob (424..428 <= 504).
const _: () = assert!(LIMITS_OFF_TIMESTAMP_PERIOD + 4 <= 504);
// The `VkBool32` read at 420 must lie inside the blob, and must sit exactly one 4-byte scalar
// before the period — a drift in either constant breaks this pairing at compile time.
const _: () = assert!(LIMITS_OFF_TIMESTAMP_COMPUTE_AND_GRAPHICS + 4 == LIMITS_OFF_TIMESTAMP_PERIOD);

impl VkPhysicalDeviceLimitsBlob {
    /// Reads the `u32` field at `offset` bytes into the opaque limits blob. The
    /// `LIMITS_OFF_*` constants above name the documented spec offsets.
    ///
    /// 2026-07 audit: this was a SAFE `pub fn` performing an unchecked raw read at a
    /// caller-supplied offset, with only a `debug_assert` in front of it — and a
    /// `debug_assert` is absent from the release build, which is the build that matters.
    /// A safe function must be sound for EVERY input it accepts, so `read_u32(10_000)`
    /// from safe code was an out-of-bounds read. It is now a checked slice index: the
    /// `unsafe` block is gone entirely, and the bounds check is free — the blob is read a
    /// handful of times at device boot, never per frame.
    ///
    /// # Panics
    ///
    /// If `offset + 4` exceeds the 504-byte blob. Every `LIMITS_OFF_*` const-asserts that
    /// it does not, so reaching the panic means a hand-written offset, i.e. a bug.
    #[inline]
    pub fn read_u32(&self, offset: usize) -> u32 {
        // The driver writes native-endian through the `vkGetPhysicalDeviceProperties`
        // out-pointer; this target is little-endian x86_64.
        u32::from_ne_bytes(self.field_bytes(offset))
    }

    /// Reads the `f32` field at `offset` bytes into the opaque limits blob (HW-RT rung
    /// R0: `timestampPeriod` at [`LIMITS_OFF_TIMESTAMP_PERIOD`]). The companion of
    /// [`Self::read_u32`] — see it for why this is a checked read.
    ///
    /// # Panics
    ///
    /// If `offset + 4` exceeds the 504-byte blob.
    #[inline]
    pub fn read_f32(&self, offset: usize) -> f32 {
        f32::from_ne_bytes(self.field_bytes(offset))
    }

    /// The shared checked 4-byte window both readers slice out of the blob.
    ///
    /// `try_into` on a `&[u8]` of the right length is a compile-time-sized copy — the same
    /// codegen the old raw read produced, with the index check the old version omitted.
    #[inline]
    fn field_bytes(&self, offset: usize) -> [u8; 4] {
        self.0[offset..offset + 4]
            .try_into()
            .expect("invariant: a 4-byte window slices to a [u8; 4]")
    }
}

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

/// `VK_STRUCTURE_TYPE_MEMORY_ALLOCATE_FLAGS_INFO` (HW-RT rung R2a-2) — Vulkan 1.1 core. The
/// `sType` heading a [`VkMemoryAllocateFlagsInfo`] chained into `VkMemoryAllocateInfo.p_next`.
///
/// Typed as a plain `i32` (the same discipline `accel_ffi.rs` uses for the RT `ST_*` values)
/// rather than a [`VkStructureType`] variant, so the ungated `VkStructureType` enum stays
/// textually pre-R2a for byte-identity. Value verified against vulkan_core.h.
#[cfg(feature = "hwrt")]
pub const ST_MEMORY_ALLOCATE_FLAGS_INFO: i32 = 1_000_060_000;

// R2a-1 lesson: raw-FFI RT sType/flag VALUES matter (abi_guard only pins layout). The two
// magic numbers below were verified against vulkan_core.h at authoring (see the const docs);
// these asserts are the REGRESSION LOCK — they trip if a later edit changes the `const` without
// updating the pinned literal (they do not, by themselves, prove the original value correct).
#[cfg(feature = "hwrt")]
const _: () = assert!(
    ST_MEMORY_ALLOCATE_FLAGS_INFO == 1_000_060_000,
    "ST_MEMORY_ALLOCATE_FLAGS_INFO must equal VK_STRUCTURE_TYPE_MEMORY_ALLOCATE_FLAGS_INFO"
);
#[cfg(feature = "hwrt")]
const _: () = assert!(
    VK_MEMORY_ALLOCATE_DEVICE_ADDRESS_BIT == 0x0000_0002,
    "VK_MEMORY_ALLOCATE_DEVICE_ADDRESS_BIT must equal 0x2"
);

/// `VkMemoryAllocateFlagsInfo` (HW-RT rung R2a-2) — chained into `VkMemoryAllocateInfo.p_next`
/// with [`VK_MEMORY_ALLOCATE_DEVICE_ADDRESS_BIT`] so buffers bound into the allocation can
/// return a device address (`vkGetBufferDeviceAddress`).
///
/// `#[repr(C)]` matching the C ABI: `sType`\@0 (4 B) + 4 B pad + `pNext`\@8 + `flags`\@16 +
/// `deviceMask`\@20, size 24, align 8 (the offsets pinned in `abi_guard.rs`).
#[cfg(feature = "hwrt")]
#[repr(C)]
pub struct VkMemoryAllocateFlagsInfo {
    /// `VK_STRUCTURE_TYPE_MEMORY_ALLOCATE_FLAGS_INFO`.
    pub s_type: i32,
    /// The next struct in the chain (null — this is the tail).
    pub p_next: *const c_void,
    /// `VkMemoryAllocateFlags` (`VK_MEMORY_ALLOCATE_DEVICE_ADDRESS_BIT`).
    pub flags: VkFlags,
    /// The device mask (`0` for a single-device group).
    pub device_mask: u32,
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

/// `VkLayerProperties` — one entry from `vkEnumerateInstanceLayerProperties`.
/// Written BY the driver, so the layout is ABI-exact: two fixed char arrays
/// (`layerName[256]`, `description[256]`) bracketing two `u32` versions.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct VkLayerProperties {
    pub layer_name: [c_char; 256],
    pub spec_version: u32,
    pub implementation_version: u32,
    pub description: [c_char; 256],
}

/// `VkExtensionProperties` — one entry from the extension enumerators. Written
/// BY the driver: a fixed `extensionName[256]` char array + a `u32` version.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct VkExtensionProperties {
    pub extension_name: [c_char; 256],
    pub spec_version: u32,
}

// FFI layout guards for the driver-written enumeration structs.
const _: () = assert!(core::mem::size_of::<VkLayerProperties>() == 520);
const _: () = assert!(core::mem::size_of::<VkExtensionProperties>() == 260);

// ---------------------------------------------------------------------------
// Slice-0 0a — VK_EXT_debug_utils (validation-message oracle) structs.
// ---------------------------------------------------------------------------

/// `PFN_vkDebugUtilsMessengerCallbackEXT` — the validation callback the loader
/// invokes for each message. Returns a `VkBool32` that must be `VK_FALSE`
/// (returning `VK_TRUE` is reserved for the layer-development case and aborts
/// the triggering call). `extern "system"` matches the loader's call ABI.
pub type PfnVkDebugUtilsMessengerCallbackExt = unsafe extern "system" fn(
    message_severity: VkFlags,
    message_types: VkFlags,
    p_callback_data: *const VkDebugUtilsMessengerCallbackDataExt,
    p_user_data: *mut c_void,
) -> VkBool32;

/// `VkDebugUtilsMessengerCallbackDataEXT` — the per-message payload the driver
/// fills and passes to the callback. Only the fields the callback reads are
/// named; the trailing label/object arrays are reserved as ABI-exact footprints
/// (pointer + count pairs) so the struct's size/layout match the C ABI the
/// driver writes through. `pMessage` is the human-readable validation text.
#[repr(C)]
pub struct VkDebugUtilsMessengerCallbackDataExt {
    pub s_type: VkStructureType,
    pub p_next: *const c_void,
    pub flags: VkFlags,
    pub p_message_id_name: *const c_char,
    pub message_id_number: i32,
    pub p_message: *const c_char,
    pub queue_label_count: u32,
    pub p_queue_labels: *const c_void,
    pub cmd_buf_label_count: u32,
    pub p_cmd_buf_labels: *const c_void,
    pub object_count: u32,
    pub p_objects: *const c_void,
}

/// `VkDebugUtilsMessengerCreateInfoEXT`.
#[repr(C)]
pub struct VkDebugUtilsMessengerCreateInfoExt {
    pub s_type: VkStructureType,
    pub p_next: *const c_void,
    pub flags: VkFlags,
    pub message_severity: VkFlags,
    pub message_type: VkFlags,
    pub pfn_user_callback: PfnVkDebugUtilsMessengerCallbackExt,
    pub p_user_data: *mut c_void,
}

/// `VkValidationFeatureEnableEXT::VK_VALIDATION_FEATURE_ENABLE_SYNCHRONIZATION_VALIDATION_EXT`.
///
/// Turns on the validation layer's **synchronization validation**, which flags a
/// missing / wrong pipeline barrier as a WARNING/ERROR — the oracle that makes the
/// chained-barrier golden test actually prove the barrier (plan G2). It is part of
/// `VkValidationFeatureEnableEXT`, an `i32` C enum.
pub const VK_VALIDATION_FEATURE_ENABLE_SYNCHRONIZATION_VALIDATION_EXT: i32 = 3;

/// `VkValidationFeaturesEXT` — chained into `VkInstanceCreateInfo::p_next` to
/// enable extra validation features (here: synchronization validation, plan G2).
#[repr(C)]
pub struct VkValidationFeaturesExt {
    pub s_type: VkStructureType,
    pub p_next: *const c_void,
    pub enabled_validation_feature_count: u32,
    pub p_enabled_validation_features: *const i32,
    pub disabled_validation_feature_count: u32,
    pub p_disabled_validation_features: *const i32,
}

// ---------------------------------------------------------------------------
// Slice-0 0c/0d — compute pipeline / descriptor / command structs.
// ---------------------------------------------------------------------------

/// `VkShaderModuleCreateInfo`. `p_code` is a `*const u32` to the SPIR-V word
/// stream; `code_size` is in BYTES (the spec is explicit) and must be a
/// multiple of 4.
#[repr(C)]
pub struct VkShaderModuleCreateInfo {
    pub s_type: VkStructureType,
    pub p_next: *const c_void,
    pub flags: VkFlags,
    pub code_size: usize,
    pub p_code: *const u32,
}

/// `VkDescriptorSetLayoutBinding` — one binding within a set layout.
#[repr(C)]
pub struct VkDescriptorSetLayoutBinding {
    pub binding: u32,
    /// `VkDescriptorType`.
    pub descriptor_type: i32,
    pub descriptor_count: u32,
    /// `VkShaderStageFlags`.
    pub stage_flags: VkFlags,
    /// `const VkSampler*` — null for non-sampler descriptors.
    pub p_immutable_samplers: *const c_void,
}

/// `VkDescriptorSetLayoutCreateInfo`.
#[repr(C)]
pub struct VkDescriptorSetLayoutCreateInfo {
    pub s_type: VkStructureType,
    pub p_next: *const c_void,
    pub flags: VkFlags,
    pub binding_count: u32,
    pub p_bindings: *const VkDescriptorSetLayoutBinding,
}

/// `VkPushConstantRange`.
#[repr(C)]
pub struct VkPushConstantRange {
    /// `VkShaderStageFlags`.
    pub stage_flags: VkFlags,
    pub offset: u32,
    pub size: u32,
}

/// `VkPipelineLayoutCreateInfo`.
#[repr(C)]
pub struct VkPipelineLayoutCreateInfo {
    pub s_type: VkStructureType,
    pub p_next: *const c_void,
    pub flags: VkFlags,
    pub set_layout_count: u32,
    pub p_set_layouts: *const VkDescriptorSetLayout,
    pub push_constant_range_count: u32,
    pub p_push_constant_ranges: *const VkPushConstantRange,
}

/// `VkPipelineShaderStageCreateInfo`.
#[repr(C)]
pub struct VkPipelineShaderStageCreateInfo {
    pub s_type: VkStructureType,
    pub p_next: *const c_void,
    pub flags: VkFlags,
    /// `VkShaderStageFlagBits` (a single bit for a stage).
    pub stage: VkFlags,
    pub module: VkShaderModule,
    pub p_name: *const c_char,
    /// `const VkSpecializationInfo*` — null (no specialization constants), or a
    /// pointer to a [`VkSpecializationInfo`] the driver reads and COPIES during the
    /// create call (Rung 1a).
    pub p_specialization_info: *const c_void,
}

/// `VkSpecializationMapEntry` — maps one constant_id to a byte range in the data blob.
#[repr(C)]
pub struct VkSpecializationMapEntry {
    pub constant_id: u32,
    pub offset: u32,
    pub size: usize, // C size_t
}
const _: () = assert!(core::mem::size_of::<VkSpecializationMapEntry>() == 16); // 4+4+8 (x86_64)

/// `VkSpecializationInfo` — the specialization blob handed to a shader stage.
#[repr(C)]
pub struct VkSpecializationInfo {
    pub map_entry_count: u32,
    pub p_map_entries: *const VkSpecializationMapEntry,
    pub data_size: usize, // C size_t
    pub p_data: *const c_void,
}
const _: () = assert!(core::mem::size_of::<VkSpecializationInfo>() == 32); // 4 + pad4 + 8 + 8 + 8

/// `VkComputePipelineCreateInfo`.
#[repr(C)]
pub struct VkComputePipelineCreateInfo {
    pub s_type: VkStructureType,
    pub p_next: *const c_void,
    pub flags: VkFlags,
    pub stage: VkPipelineShaderStageCreateInfo,
    pub layout: VkPipelineLayout,
    /// `VkPipeline basePipelineHandle` — null (no derivative).
    pub base_pipeline_handle: VkPipeline,
    pub base_pipeline_index: i32,
}

// ---------------------------------------------------------------------------
// Phase-6 S0 rung-2 — graphics-pipeline create-info chain. Every struct is read
// BY the driver in `vkCreateGraphicsPipelines`, so the `#[repr(C)]` layout must
// match the C ABI (the layout guards below break the build on any drift).
// ---------------------------------------------------------------------------

/// `VkVertexInputBindingDescription` — one vertex buffer binding's stride + input
/// rate (Phase-6 S0 rung 3).
#[repr(C)]
pub struct VkVertexInputBindingDescription {
    pub binding: u32,
    pub stride: u32,
    /// `VkVertexInputRate`.
    pub input_rate: i32,
}

/// `VkVertexInputAttributeDescription` — one attribute's `(location, binding,
/// format, offset)` within a vertex (Phase-6 S0 rung 3).
#[repr(C)]
pub struct VkVertexInputAttributeDescription {
    pub location: u32,
    pub binding: u32,
    /// `VkFormat`.
    pub format: i32,
    pub offset: u32,
}

/// `VkPipelineVertexInputStateCreateInfo`. Rung 2 binds NO vertex buffer (the
/// vertex shader generates positions from `SV_VertexID`) so both arrays are empty
/// (count `0`, null pointers); rung 3 supplies one binding + one attribute per
/// vertex-layout entry.
#[repr(C)]
pub struct VkPipelineVertexInputStateCreateInfo {
    pub s_type: VkStructureType,
    pub p_next: *const c_void,
    pub flags: VkFlags,
    pub vertex_binding_description_count: u32,
    /// `const VkVertexInputBindingDescription*` — null when count is `0`.
    pub p_vertex_binding_descriptions: *const VkVertexInputBindingDescription,
    pub vertex_attribute_description_count: u32,
    /// `const VkVertexInputAttributeDescription*` — null when count is `0`.
    pub p_vertex_attribute_descriptions: *const VkVertexInputAttributeDescription,
}

/// `VkPipelineInputAssemblyStateCreateInfo`.
#[repr(C)]
pub struct VkPipelineInputAssemblyStateCreateInfo {
    pub s_type: VkStructureType,
    pub p_next: *const c_void,
    pub flags: VkFlags,
    /// `VkPrimitiveTopology`.
    pub topology: i32,
    pub primitive_restart_enable: VkBool32,
}

/// `VkViewport` — a dynamic viewport (set via `vkCmdSetViewport`). Declared so the
/// agnostic `boyko_rhi::Viewport` (same `(x, y, w, h, minDepth, maxDepth)` `f32`
/// layout) is passed straight through (the layout match is asserted in `abi_guard`).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct VkViewport {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub min_depth: f32,
    pub max_depth: f32,
}

/// `VkPipelineViewportStateCreateInfo` — rung 2 uses DYNAMIC viewport + scissor, so
/// the counts are 1 but the pointers are null (the actual rects come from
/// `vkCmdSetViewport`/`vkCmdSetScissor`).
#[repr(C)]
pub struct VkPipelineViewportStateCreateInfo {
    pub s_type: VkStructureType,
    pub p_next: *const c_void,
    pub flags: VkFlags,
    pub viewport_count: u32,
    /// `const VkViewport*` — null (dynamic viewport).
    pub p_viewports: *const VkViewport,
    pub scissor_count: u32,
    /// `const VkRect2D*` — null (dynamic scissor).
    pub p_scissors: *const VkRect2D,
}

/// `VkPipelineRasterizationStateCreateInfo`. `line_width` MUST be `1.0` unless the
/// `wideLines` feature is enabled (rung 2 fills triangles, so it is `1.0`).
#[repr(C)]
pub struct VkPipelineRasterizationStateCreateInfo {
    pub s_type: VkStructureType,
    pub p_next: *const c_void,
    pub flags: VkFlags,
    pub depth_clamp_enable: VkBool32,
    pub rasterizer_discard_enable: VkBool32,
    /// `VkPolygonMode`.
    pub polygon_mode: i32,
    /// `VkCullModeFlags`.
    pub cull_mode: VkFlags,
    /// `VkFrontFace`.
    pub front_face: i32,
    pub depth_bias_enable: VkBool32,
    pub depth_bias_constant_factor: f32,
    pub depth_bias_clamp: f32,
    pub depth_bias_slope_factor: f32,
    pub line_width: f32,
}

/// `VkPipelineMultisampleStateCreateInfo` — rung 2 is single-sampled.
#[repr(C)]
pub struct VkPipelineMultisampleStateCreateInfo {
    pub s_type: VkStructureType,
    pub p_next: *const c_void,
    pub flags: VkFlags,
    /// `VkSampleCountFlagBits`.
    pub rasterization_samples: VkFlags,
    pub sample_shading_enable: VkBool32,
    pub min_sample_shading: f32,
    /// `const VkSampleMask*` — null (no custom sample mask).
    pub p_sample_mask: *const u32,
    pub alpha_to_coverage_enable: VkBool32,
    pub alpha_to_one_enable: VkBool32,
}

/// `VkPipelineColorBlendAttachmentState` — one per color attachment. Rung 2
/// disables blending (opaque write) with an all-channel write mask.
#[repr(C)]
pub struct VkPipelineColorBlendAttachmentState {
    pub blend_enable: VkBool32,
    /// `VkBlendFactor`.
    pub src_color_blend_factor: i32,
    /// `VkBlendFactor`.
    pub dst_color_blend_factor: i32,
    /// `VkBlendOp`.
    pub color_blend_op: i32,
    /// `VkBlendFactor`.
    pub src_alpha_blend_factor: i32,
    /// `VkBlendFactor`.
    pub dst_alpha_blend_factor: i32,
    /// `VkBlendOp`.
    pub alpha_blend_op: i32,
    /// `VkColorComponentFlags`.
    pub color_write_mask: VkFlags,
}

/// `VkPipelineColorBlendStateCreateInfo`.
#[repr(C)]
pub struct VkPipelineColorBlendStateCreateInfo {
    pub s_type: VkStructureType,
    pub p_next: *const c_void,
    pub flags: VkFlags,
    pub logic_op_enable: VkBool32,
    /// `VkLogicOp`.
    pub logic_op: i32,
    pub attachment_count: u32,
    pub p_attachments: *const VkPipelineColorBlendAttachmentState,
    pub blend_constants: [f32; 4],
}

/// `VkStencilOpState` — one face's stencil op set. Rung 4 disables stencil, so all
/// fields are zero/no-op; the struct exists because [`VkPipelineDepthStencilStateCreateInfo`]
/// embeds two of them by value (front + back) and the C ABI lays them out inline.
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct VkStencilOpState {
    /// `VkStencilOp` fail op.
    pub fail_op: i32,
    /// `VkStencilOp` pass op.
    pub pass_op: i32,
    /// `VkStencilOp` depth-fail op.
    pub depth_fail_op: i32,
    /// `VkCompareOp`.
    pub compare_op: i32,
    pub compare_mask: u32,
    pub write_mask: u32,
    pub reference: u32,
}

/// `VkPipelineDepthStencilStateCreateInfo` — the rung-4 depth-test state
/// (`depthTestEnable`/`depthWriteEnable` = TRUE, `depthCompareOp` = LESS, no
/// depth-bounds, no stencil). Pointed to by `VkGraphicsPipelineCreateInfo.
/// p_depth_stencil_state` ONLY when a depth format is declared; the rungs-1..3
/// no-depth path leaves that pointer null.
#[repr(C)]
pub struct VkPipelineDepthStencilStateCreateInfo {
    pub s_type: VkStructureType,
    pub p_next: *const c_void,
    pub flags: VkFlags,
    pub depth_test_enable: VkBool32,
    pub depth_write_enable: VkBool32,
    /// `VkCompareOp`.
    pub depth_compare_op: i32,
    pub depth_bounds_test_enable: VkBool32,
    pub stencil_test_enable: VkBool32,
    pub front: VkStencilOpState,
    pub back: VkStencilOpState,
    pub min_depth_bounds: f32,
    pub max_depth_bounds: f32,
}

/// `VkPipelineDynamicStateCreateInfo` — rung 2 marks viewport + scissor dynamic.
#[repr(C)]
pub struct VkPipelineDynamicStateCreateInfo {
    pub s_type: VkStructureType,
    pub p_next: *const c_void,
    pub flags: VkFlags,
    pub dynamic_state_count: u32,
    /// `const VkDynamicState*`.
    pub p_dynamic_states: *const i32,
}

/// `VkPipelineRenderingCreateInfo` — the dynamic-rendering attachment-format chain
/// (no `VkRenderPass`), chained into `VkGraphicsPipelineCreateInfo.p_next`. The
/// color-attachment format declared here MUST equal the format of every
/// `begin_rendering` scope the pipeline is bound inside (the W2-b SAFETY contract).
#[repr(C)]
pub struct VkPipelineRenderingCreateInfo {
    pub s_type: VkStructureType,
    pub p_next: *const c_void,
    pub view_mask: u32,
    pub color_attachment_count: u32,
    /// `const VkFormat*` (an `i32` per format).
    pub p_color_attachment_formats: *const i32,
    /// `VkFormat` depth attachment — `VK_FORMAT_UNDEFINED` (no depth, rung 2).
    pub depth_attachment_format: i32,
    /// `VkFormat` stencil attachment — `VK_FORMAT_UNDEFINED`.
    pub stencil_attachment_format: i32,
}

/// `VkGraphicsPipelineCreateInfo` — the top-level graphics-pipeline create-info.
/// `p_next` chains the [`VkPipelineRenderingCreateInfo`]; `render_pass` is
/// `VK_NULL_HANDLE` and `subpass` is `0` (dynamic rendering, OQ-6). Tessellation +
/// depth-stencil state are unused (null) for rung 2.
#[repr(C)]
pub struct VkGraphicsPipelineCreateInfo {
    pub s_type: VkStructureType,
    pub p_next: *const c_void,
    pub flags: VkFlags,
    pub stage_count: u32,
    pub p_stages: *const VkPipelineShaderStageCreateInfo,
    pub p_vertex_input_state: *const VkPipelineVertexInputStateCreateInfo,
    pub p_input_assembly_state: *const VkPipelineInputAssemblyStateCreateInfo,
    /// `const VkPipelineTessellationStateCreateInfo*` — null.
    pub p_tessellation_state: *const c_void,
    pub p_viewport_state: *const VkPipelineViewportStateCreateInfo,
    pub p_rasterization_state: *const VkPipelineRasterizationStateCreateInfo,
    pub p_multisample_state: *const VkPipelineMultisampleStateCreateInfo,
    /// `const VkPipelineDepthStencilStateCreateInfo*` — null on the no-depth path
    /// (rungs 2-3); points at a live `VkPipelineDepthStencilStateCreateInfo` when a
    /// depth format is supplied (rung 4).
    pub p_depth_stencil_state: *const c_void,
    pub p_color_blend_state: *const VkPipelineColorBlendStateCreateInfo,
    pub p_dynamic_state: *const VkPipelineDynamicStateCreateInfo,
    pub layout: VkPipelineLayout,
    /// `VkRenderPass` — `VK_NULL_HANDLE` (dynamic rendering, no render pass).
    pub render_pass: u64,
    pub subpass: u32,
    /// `VkPipeline basePipelineHandle` — null (no derivative).
    pub base_pipeline_handle: VkPipeline,
    pub base_pipeline_index: i32,
}

/// `VkDescriptorPoolSize`.
///
/// `#[derive(Clone, Copy)]`: a plain POD with no Drop, so the bind-group create path
/// can build a fixed inline `[VkDescriptorPoolSize; N]` histogram by value-repeat.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct VkDescriptorPoolSize {
    /// `VkDescriptorType`.
    pub descriptor_type: i32,
    pub descriptor_count: u32,
}

/// `VkDescriptorPoolCreateInfo`.
#[repr(C)]
pub struct VkDescriptorPoolCreateInfo {
    pub s_type: VkStructureType,
    pub p_next: *const c_void,
    pub flags: VkFlags,
    pub max_sets: u32,
    pub pool_size_count: u32,
    pub p_pool_sizes: *const VkDescriptorPoolSize,
}

/// `VkDescriptorSetAllocateInfo`.
#[repr(C)]
pub struct VkDescriptorSetAllocateInfo {
    pub s_type: VkStructureType,
    pub p_next: *const c_void,
    pub descriptor_pool: VkDescriptorPool,
    pub descriptor_set_count: u32,
    pub p_set_layouts: *const VkDescriptorSetLayout,
}

/// `VkDescriptorSetLayoutBindingFlagsCreateInfo` (T4 bindless) — chained into
/// [`VkDescriptorSetLayoutCreateInfo::p_next`] to supply one [`VkFlags`] of
/// [`VK_DESCRIPTOR_BINDING_UPDATE_AFTER_BIND_BIT`] /
/// [`VK_DESCRIPTOR_BINDING_PARTIALLY_BOUND_BIT`] /
/// [`VK_DESCRIPTOR_BINDING_VARIABLE_DESCRIPTOR_COUNT_BIT`] per binding, in the SAME
/// order as the layout's `p_bindings` array (`binding_count` MUST equal the
/// layout's `binding_count`, or the driver reads/writes past one of the two
/// arrays).
#[repr(C)]
pub struct VkDescriptorSetLayoutBindingFlagsCreateInfo {
    pub s_type: VkStructureType,
    pub p_next: *const c_void,
    pub binding_count: u32,
    /// `const VkDescriptorBindingFlags*` — one flags word per binding, positionally
    /// paired with the layout's `p_bindings[i]`.
    pub p_binding_flags: *const VkFlags,
}

/// `VkDescriptorSetVariableDescriptorCountAllocateInfo` (T4 bindless) — chained
/// into [`VkDescriptorSetAllocateInfo::p_next`] to supply the RUNTIME descriptor
/// count for each set's VARIABLE_DESCRIPTOR_COUNT-flagged binding (the LAST
/// binding in the layout) at allocation time. `descriptor_set_count` MUST equal
/// the enclosing alloc-info's `descriptor_set_count`.
#[repr(C)]
pub struct VkDescriptorSetVariableDescriptorCountAllocateInfo {
    pub s_type: VkStructureType,
    pub p_next: *const c_void,
    pub descriptor_set_count: u32,
    /// `const uint32_t*` — one runtime count per set being allocated.
    pub p_descriptor_counts: *const u32,
}

/// `VkDescriptorBufferInfo`.
///
/// `#[derive(Clone, Copy)]`: a plain POD with no Drop, so the bind-group create path
/// can build a fixed inline `[VkDescriptorBufferInfo; N]` by value-repeat.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct VkDescriptorBufferInfo {
    pub buffer: VkBuffer,
    pub offset: VkDeviceSize,
    pub range: VkDeviceSize,
}

/// `VkDescriptorImageInfo` — the `(sampler, image view, layout)` triple written
/// into a COMBINED_IMAGE_SAMPLER / SAMPLED_IMAGE / STORAGE_IMAGE descriptor.
///
/// `#[derive(Clone, Copy)]`: a plain POD with no Drop, so the bind-group create path
/// can build a fixed inline `[VkDescriptorImageInfo; N]` by value-repeat.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct VkDescriptorImageInfo {
    pub sampler: VkSampler,
    pub image_view: VkImageView,
    /// `VkImageLayout` the image is in when sampled (SHADER_READ_ONLY_OPTIMAL).
    pub image_layout: i32,
}

/// `VkSamplerCreateInfo` — read BY the driver in `vkCreateSampler` (Phase-6 S0
/// rung 5). The `#[repr(C)]` layout must match the C ABI (the layout guard at the
/// bottom of this module breaks the build on any drift).
#[repr(C)]
pub struct VkSamplerCreateInfo {
    pub s_type: VkStructureType,
    pub p_next: *const c_void,
    pub flags: VkFlags,
    /// `VkFilter`.
    pub mag_filter: i32,
    /// `VkFilter`.
    pub min_filter: i32,
    /// `VkSamplerMipmapMode`.
    pub mipmap_mode: i32,
    /// `VkSamplerAddressMode` (U axis).
    pub address_mode_u: i32,
    /// `VkSamplerAddressMode` (V axis).
    pub address_mode_v: i32,
    /// `VkSamplerAddressMode` (W axis).
    pub address_mode_w: i32,
    pub mip_lod_bias: f32,
    pub anisotropy_enable: VkBool32,
    pub max_anisotropy: f32,
    pub compare_enable: VkBool32,
    /// `VkCompareOp`.
    pub compare_op: i32,
    pub min_lod: f32,
    pub max_lod: f32,
    /// `VkBorderColor`.
    pub border_color: i32,
    pub unnormalized_coordinates: VkBool32,
}

/// `VkWriteDescriptorSet`.
#[repr(C)]
pub struct VkWriteDescriptorSet {
    pub s_type: VkStructureType,
    pub p_next: *const c_void,
    pub dst_set: VkDescriptorSet,
    pub dst_binding: u32,
    pub dst_array_element: u32,
    pub descriptor_count: u32,
    /// `VkDescriptorType`.
    pub descriptor_type: i32,
    /// `const VkDescriptorImageInfo*` — null for a storage buffer.
    pub p_image_info: *const c_void,
    pub p_buffer_info: *const VkDescriptorBufferInfo,
    /// `const VkBufferView*` — null for a storage buffer.
    pub p_texel_buffer_view: *const c_void,
}

/// `VkCommandPoolCreateInfo`.
#[repr(C)]
pub struct VkCommandPoolCreateInfo {
    pub s_type: VkStructureType,
    pub p_next: *const c_void,
    pub flags: VkFlags,
    pub queue_family_index: u32,
}

/// `VkCommandBufferAllocateInfo`.
#[repr(C)]
pub struct VkCommandBufferAllocateInfo {
    pub s_type: VkStructureType,
    pub p_next: *const c_void,
    pub command_pool: VkCommandPool,
    /// `VkCommandBufferLevel`.
    pub level: i32,
    pub command_buffer_count: u32,
}

/// `VkCommandBufferBeginInfo`.
#[repr(C)]
pub struct VkCommandBufferBeginInfo {
    pub s_type: VkStructureType,
    pub p_next: *const c_void,
    pub flags: VkFlags,
    /// `const VkCommandBufferInheritanceInfo*` — null for a primary buffer.
    pub p_inheritance_info: *const c_void,
}

/// `VkBufferMemoryBarrier` — the §5.5 edge→barrier lowering in miniature (0d).
#[repr(C)]
pub struct VkBufferMemoryBarrier {
    pub s_type: VkStructureType,
    pub p_next: *const c_void,
    /// `VkAccessFlags` made available by the source scope.
    pub src_access_mask: VkFlags,
    /// `VkAccessFlags` made visible to the destination scope.
    pub dst_access_mask: VkFlags,
    pub src_queue_family_index: u32,
    pub dst_queue_family_index: u32,
    pub buffer: VkBuffer,
    pub offset: VkDeviceSize,
    pub size: VkDeviceSize,
}

/// `VkBufferCopy` — one buffer-to-buffer copy region for `vkCmdCopyBuffer`
/// (Phase-5 staging upload + readback). Field order + `VkDeviceSize` (u64) types
/// match the agnostic `boyko_rhi::BufferCopy` exactly, so the encoder can pass a
/// `&[BufferCopy]` straight through as a `&[VkBufferCopy]` (layout match asserted
/// below + at the cast site).
#[repr(C)]
pub struct VkBufferCopy {
    pub src_offset: VkDeviceSize,
    pub dst_offset: VkDeviceSize,
    pub size: VkDeviceSize,
}

/// `VkFenceCreateInfo`.
#[repr(C)]
pub struct VkFenceCreateInfo {
    pub s_type: VkStructureType,
    pub p_next: *const c_void,
    pub flags: VkFlags,
}

/// `VkQueryPoolCreateInfo` — the HW-RT rung R0 TIMESTAMP query-pool create struct.
/// `query_type` is a `VkQueryType` (`i32`; set to [`VK_QUERY_TYPE_TIMESTAMP`]);
/// `pipeline_statistics` is `0` for a TIMESTAMP pool.
#[repr(C)]
pub struct VkQueryPoolCreateInfo {
    pub s_type: VkStructureType,
    pub p_next: *const c_void,
    pub flags: VkFlags,
    /// `VkQueryType` (`i32`).
    pub query_type: i32,
    pub query_count: u32,
    /// `VkQueryPipelineStatisticFlags` — `0` for a TIMESTAMP pool.
    pub pipeline_statistics: VkFlags,
}
// ABI pin (belt-and-suspenders, matching the crate's other Vk-struct guards): on the x86_64 ABI
// `s_type`@0(4) + pad(4) + `p_next`@8(8) + `flags`@16(4) + `query_type`@20(4) + `query_count`@24(4)
// + `pipeline_statistics`@28(4) = 32 B, align 8. A field-type slip would change this and fail here.
const _: () = assert!(size_of::<VkQueryPoolCreateInfo>() == 32);
const _: () = assert!(align_of::<VkQueryPoolCreateInfo>() == 8);

/// `VkSubmitInfo`.
#[repr(C)]
pub struct VkSubmitInfo {
    pub s_type: VkStructureType,
    pub p_next: *const c_void,
    pub wait_semaphore_count: u32,
    pub p_wait_semaphores: *const c_void,
    /// `const VkPipelineStageFlags*` — null (no wait stages).
    pub p_wait_dst_stage_mask: *const VkFlags,
    pub command_buffer_count: u32,
    pub p_command_buffers: *const VkCommandBuffer,
    pub signal_semaphore_count: u32,
    pub p_signal_semaphores: *const c_void,
}

// ---------------------------------------------------------------------------
// Slice-1 — surface / swapchain / dynamic-rendering / image-barrier structs.
// ---------------------------------------------------------------------------

/// `VkWin32SurfaceCreateInfoKHR` — the Windows WSI surface-creation struct.
/// `hinstance`/`hwnd` are the Win32 HINSTANCE / HWND (opaque pointers).
#[repr(C)]
pub struct VkWin32SurfaceCreateInfoKhr {
    pub s_type: VkStructureType,
    pub p_next: *const c_void,
    pub flags: VkFlags,
    pub hinstance: *mut c_void,
    pub hwnd: *mut c_void,
}

/// `VkExtent2D` — a width/height pair (used by surface caps + swapchain extent).
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct VkExtent2D {
    pub width: u32,
    pub height: u32,
}

/// `VkSurfaceCapabilitiesKHR` — written BY the driver; ABI-exact (it ends with
/// two `VkExtent2D`s and three flag/usage `u32`s after the count + extent
/// fields). Every field is read to size the swapchain.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct VkSurfaceCapabilitiesKhr {
    pub min_image_count: u32,
    pub max_image_count: u32,
    pub current_extent: VkExtent2D,
    pub min_image_extent: VkExtent2D,
    pub max_image_extent: VkExtent2D,
    pub max_image_array_layers: u32,
    /// `VkSurfaceTransformFlagsKHR`.
    pub supported_transforms: VkFlags,
    /// `VkSurfaceTransformFlagBitsKHR`.
    pub current_transform: VkFlags,
    /// `VkCompositeAlphaFlagsKHR`.
    pub supported_composite_alpha: VkFlags,
    /// `VkImageUsageFlags`.
    pub supported_usage_flags: VkFlags,
}

/// `VkSurfaceFormatKHR` — written BY the driver: a `VkFormat` + `VkColorSpaceKHR`
/// (both `i32` C enums).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct VkSurfaceFormatKhr {
    pub format: i32,
    pub color_space: i32,
}

/// `VkSwapchainCreateInfoKHR`.
#[repr(C)]
pub struct VkSwapchainCreateInfoKhr {
    pub s_type: VkStructureType,
    pub p_next: *const c_void,
    pub flags: VkFlags,
    pub surface: VkSurfaceKHR,
    pub min_image_count: u32,
    /// `VkFormat`.
    pub image_format: i32,
    /// `VkColorSpaceKHR`.
    pub image_color_space: i32,
    pub image_extent: VkExtent2D,
    pub image_array_layers: u32,
    /// `VkImageUsageFlags`.
    pub image_usage: VkFlags,
    /// `VkSharingMode`.
    pub image_sharing_mode: i32,
    pub queue_family_index_count: u32,
    pub p_queue_family_indices: *const u32,
    /// `VkSurfaceTransformFlagBitsKHR`.
    pub pre_transform: VkFlags,
    /// `VkCompositeAlphaFlagBitsKHR`.
    pub composite_alpha: VkFlags,
    /// `VkPresentModeKHR`.
    pub present_mode: i32,
    pub clipped: VkBool32,
    pub old_swapchain: VkSwapchainKHR,
}

/// `VkComponentMapping` — per-channel swizzle for an image view (identity here).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct VkComponentMapping {
    pub r: i32,
    pub g: i32,
    pub b: i32,
    pub a: i32,
}

/// `VkImageSubresourceRange`.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct VkImageSubresourceRange {
    /// `VkImageAspectFlags`.
    pub aspect_mask: VkFlags,
    pub base_mip_level: u32,
    pub level_count: u32,
    pub base_array_layer: u32,
    pub layer_count: u32,
}

/// `VkImageViewCreateInfo`.
#[repr(C)]
pub struct VkImageViewCreateInfo {
    pub s_type: VkStructureType,
    pub p_next: *const c_void,
    pub flags: VkFlags,
    pub image: VkImage,
    /// `VkImageViewType`.
    pub view_type: i32,
    /// `VkFormat`.
    pub format: i32,
    pub components: VkComponentMapping,
    pub subresource_range: VkImageSubresourceRange,
}

/// `VkSemaphoreCreateInfo`.
#[repr(C)]
pub struct VkSemaphoreCreateInfo {
    pub s_type: VkStructureType,
    pub p_next: *const c_void,
    pub flags: VkFlags,
}

/// `VkImageMemoryBarrier` — the layout transition for the dynamic-rendering
/// present path (UNDEFINED→COLOR_ATTACHMENT_OPTIMAL→PRESENT_SRC_KHR).
#[repr(C)]
pub struct VkImageMemoryBarrier {
    pub s_type: VkStructureType,
    pub p_next: *const c_void,
    pub src_access_mask: VkFlags,
    pub dst_access_mask: VkFlags,
    /// `VkImageLayout`.
    pub old_layout: i32,
    /// `VkImageLayout`.
    pub new_layout: i32,
    pub src_queue_family_index: u32,
    pub dst_queue_family_index: u32,
    pub image: VkImage,
    pub subresource_range: VkImageSubresourceRange,
}

/// `VkClearColorValue` — the real Vulkan union `{ float32[4]; int32[4]; uint32[4]; }`. Multi-
/// paradigm render-path plan, rung R8: widened from a bare `float32`-only struct to a proper
/// `union` (both variants are the same 16-byte size/align) so the `vb_id` `R32G32_UINT` color
/// attachment can be cleared to its sentinel `(0xFFFFFFFF, 0)` via `uint32`, alongside every
/// existing `float32` clear (UNORM/SFLOAT color targets), which is unaffected — a union read of
/// `float32` after a `float32` write (the only pattern every existing call site uses) is
/// unchanged.
#[repr(C)]
#[derive(Clone, Copy)]
pub union VkClearColorValue {
    pub float32: [f32; 4],
    pub uint32: [u32; 4],
}

/// `VkClearDepthStencilValue` (the `{ float depth; uint32_t stencil; }` member of
/// the [`VkClearValue`] union — used by the rung-4 depth attachment's `CLEAR`).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct VkClearDepthStencilValue {
    pub depth: f32,
    pub stencil: u32,
}

/// `VkClearValue` — the C union of `VkClearColorValue` (16 B, the largest variant)
/// and `VkClearDepthStencilValue` (8 B). A `#[repr(C)] union` is the exact ABI
/// shape (16-byte size, 4-byte align). A color attachment writes `.color`; a depth
/// attachment writes `.depth_stencil` (Phase-6 S0 rung 4). Reading the union is
/// unsafe (the active field is implied by the attachment's aspect, never read by
/// this crate — only written and handed to the driver).
#[repr(C)]
#[derive(Clone, Copy)]
pub union VkClearValue {
    pub color: VkClearColorValue,
    pub depth_stencil: VkClearDepthStencilValue,
}

/// `VkRenderingAttachmentInfo` — one color attachment for `vkCmdBeginRendering`.
#[repr(C)]
pub struct VkRenderingAttachmentInfo {
    pub s_type: VkStructureType,
    pub p_next: *const c_void,
    pub image_view: VkImageView,
    /// `VkImageLayout` the attachment is in during rendering.
    pub image_layout: i32,
    /// `VkResolveModeFlagBits` — `0` (`VK_RESOLVE_MODE_NONE`), no MSAA resolve.
    pub resolve_mode: VkFlags,
    pub resolve_image_view: VkImageView,
    /// `VkImageLayout` of the (unused) resolve target.
    pub resolve_image_layout: i32,
    /// `VkAttachmentLoadOp`.
    pub load_op: i32,
    /// `VkAttachmentStoreOp`.
    pub store_op: i32,
    pub clear_value: VkClearValue,
}

/// `VkOffset2D`.
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct VkOffset2D {
    pub x: i32,
    pub y: i32,
}

/// `VkRect2D` — the render area.
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct VkRect2D {
    pub offset: VkOffset2D,
    pub extent: VkExtent2D,
}

/// `VkRenderingInfo` — the Vulkan 1.3 dynamic-rendering scope (no render pass).
#[repr(C)]
pub struct VkRenderingInfo {
    pub s_type: VkStructureType,
    pub p_next: *const c_void,
    /// `VkRenderingFlags` — `0`.
    pub flags: VkFlags,
    pub render_area: VkRect2D,
    pub layer_count: u32,
    pub view_mask: u32,
    pub color_attachment_count: u32,
    pub p_color_attachments: *const VkRenderingAttachmentInfo,
    /// `const VkRenderingAttachmentInfo*` depth — null on the no-depth path; points
    /// at a live depth `VkRenderingAttachmentInfo` when a depth attachment is supplied
    /// (rung 4).
    pub p_depth_attachment: *const c_void,
    /// `const VkRenderingAttachmentInfo*` stencil — null.
    pub p_stencil_attachment: *const c_void,
}

/// `VkExtent3D` — a width/height/depth triple (image extent for `create_texture`).
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct VkExtent3D {
    pub width: u32,
    pub height: u32,
    pub depth: u32,
}

/// `VkImageCreateInfo` — the S0 2D/3D image creation parameters
/// (`vkCreateImage`). Declared with the exact C ABI field order.
#[repr(C)]
pub struct VkImageCreateInfo {
    pub s_type: VkStructureType,
    pub p_next: *const c_void,
    pub flags: VkFlags,
    /// `VkImageType`.
    pub image_type: i32,
    /// `VkFormat`.
    pub format: i32,
    pub extent: VkExtent3D,
    pub mip_levels: u32,
    pub array_layers: u32,
    /// `VkSampleCountFlagBits`.
    pub samples: VkFlags,
    /// `VkImageTiling`.
    pub tiling: i32,
    /// `VkImageUsageFlags`.
    pub usage: VkFlags,
    /// `VkSharingMode`.
    pub sharing_mode: i32,
    pub queue_family_index_count: u32,
    pub p_queue_family_indices: *const u32,
    /// `VkImageLayout` — the layout the image is created in (`UNDEFINED`).
    pub initial_layout: i32,
}

/// `VkImageSubresourceLayers` — the mip/layer selector for a `VkBufferImageCopy`.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct VkImageSubresourceLayers {
    /// `VkImageAspectFlags`.
    pub aspect_mask: VkFlags,
    pub mip_level: u32,
    pub base_array_layer: u32,
    pub layer_count: u32,
}

/// `VkBufferImageCopy` — one image↔buffer copy region (the S0 readback uses
/// `vkCmdCopyImageToBuffer` for the offscreen image → host-visible staging path).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct VkBufferImageCopy {
    pub buffer_offset: VkDeviceSize,
    /// `0` = tightly packed (row length = image width).
    pub buffer_row_length: u32,
    /// `0` = tightly packed (image height = extent height).
    pub buffer_image_height: u32,
    pub image_subresource: VkImageSubresourceLayers,
    pub image_offset: VkOffset3D,
    pub image_extent: VkExtent3D,
}

/// `VkOffset3D` — a texel offset for a buffer↔image copy region.
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct VkOffset3D {
    pub x: i32,
    pub y: i32,
    pub z: i32,
}

/// `VkImageBlit` — one mip-to-mip blit region for `vkCmdBlitImage` (textured-PBR
/// T2 Decision D3, the mip-chain-generation blit). `srcOffsets`/`dstOffsets` are the
/// two opposite corners of the (axis-aligned) source/destination box; a mip-chain
/// blit always uses `[(0,0,0), (extent_w, extent_h, 1)]` (the full mip level).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct VkImageBlit {
    pub src_subresource: VkImageSubresourceLayers,
    pub src_offsets: [VkOffset3D; 2],
    pub dst_subresource: VkImageSubresourceLayers,
    pub dst_offsets: [VkOffset3D; 2],
}

/// `VkPhysicalDeviceFeatures2` — the head struct for `vkGetPhysicalDeviceFeatures2`
/// (S0 fail-fast `dynamicRendering` support query). The `features` member is the
/// large `VkPhysicalDeviceFeatures` block (55 `VkBool32`s = 220 bytes), reserved
/// here as an ABI-exact opaque footprint: we only read the chained
/// `VkPhysicalDeviceVulkan13Features.dynamic_rendering` written through `p_next`.
#[repr(C)]
pub struct VkPhysicalDeviceFeatures2 {
    pub s_type: VkStructureType,
    pub p_next: *mut c_void,
    /// `VkPhysicalDeviceFeatures features` — 55 `VkBool32`s (opaque, written by
    /// the driver; we do not read it for the dynamic-rendering query).
    pub features: [VkBool32; 55],
}

/// `VkPhysicalDeviceVulkan13Features` — chained into `VkDeviceCreateInfo` to
/// enable `dynamicRendering` + `synchronization2` (we use only `dynamicRendering`).
/// All other feature bools are zero. The struct is large in the real header; we
/// declare exactly the fields up to `dynamicRendering` and reserve the tail as an
/// ABI-exact opaque footprint (it is written BY us with zeros, but its size must
/// match so the driver does not read past our struct when walking `p_next`).
#[repr(C)]
pub struct VkPhysicalDeviceVulkan13Features {
    pub s_type: VkStructureType,
    pub p_next: *mut c_void,
    pub robust_image_access: VkBool32,
    pub inline_uniform_block: VkBool32,
    pub descriptor_binding_inline_uniform_block_update_after_bind: VkBool32,
    pub pipeline_creation_cache_control: VkBool32,
    pub private_data: VkBool32,
    pub shader_demote_to_helper_invocation: VkBool32,
    pub shader_terminate_invocation: VkBool32,
    pub subgroup_size_control: VkBool32,
    pub compute_full_subgroups: VkBool32,
    pub synchronization2: VkBool32,
    pub texture_compression_astc_hdr: VkBool32,
    pub shader_zero_initialize_workgroup_memory: VkBool32,
    pub dynamic_rendering: VkBool32,
    pub shader_integer_dot_product: VkBool32,
    pub maintenance4: VkBool32,
}

/// `VkFormatProperties` — written BY the driver through
/// `vkGetPhysicalDeviceFormatProperties`'s out-pointer. The three feature masks
/// advertise what a format supports per tiling/buffer; the Render P1b device-caps
/// query reads only `optimal_tiling_features` for `VK_FORMAT_FEATURE_STORAGE_IMAGE_BIT`.
/// `#[repr(C)]` + a size/align guard below: it is driver-written, so its layout must
/// match the C ABI or the driver overruns our out-buffer.
#[repr(C)]
pub struct VkFormatProperties {
    pub linear_tiling_features: VkFlags,
    pub optimal_tiling_features: VkFlags,
    pub buffer_features: VkFlags,
}

/// `VkPhysicalDeviceVulkan12Features` — the Vulkan 1.2 aggregate feature struct.
/// Declared field-exact (ABI completeness / a future reader of other 1.2 bits), but
/// NOT used by the T-dev bindless query/enable path — see
/// [`VkPhysicalDeviceDescriptorIndexingFeatures`] for the granular struct that path
/// reads/writes instead. The struct is declared field-exact so the driver, walking
/// `p_next`, writes every bool it owns without reading past our footprint; the
/// size/align guard below pins the ABI. All fields are `VkBool32`, written BY the driver.
#[repr(C)]
pub struct VkPhysicalDeviceVulkan12Features {
    pub s_type: VkStructureType,
    pub p_next: *mut c_void,
    pub sampler_mirror_clamp_to_edge: VkBool32,
    pub draw_indirect_count: VkBool32,
    pub storage_buffer_8bit_access: VkBool32,
    pub uniform_and_storage_buffer_8bit_access: VkBool32,
    pub storage_push_constant8: VkBool32,
    pub shader_buffer_int64_atomics: VkBool32,
    pub shader_shared_int64_atomics: VkBool32,
    pub shader_float16: VkBool32,
    pub shader_int8: VkBool32,
    pub descriptor_indexing: VkBool32,
    pub shader_input_attachment_array_dynamic_indexing: VkBool32,
    pub shader_uniform_texel_buffer_array_dynamic_indexing: VkBool32,
    pub shader_storage_texel_buffer_array_dynamic_indexing: VkBool32,
    pub shader_uniform_buffer_array_non_uniform_indexing: VkBool32,
    pub shader_sampled_image_array_non_uniform_indexing: VkBool32,
    pub shader_storage_buffer_array_non_uniform_indexing: VkBool32,
    pub shader_storage_image_array_non_uniform_indexing: VkBool32,
    pub shader_input_attachment_array_non_uniform_indexing: VkBool32,
    pub shader_uniform_texel_buffer_array_non_uniform_indexing: VkBool32,
    pub shader_storage_texel_buffer_array_non_uniform_indexing: VkBool32,
    pub descriptor_binding_uniform_buffer_update_after_bind: VkBool32,
    pub descriptor_binding_sampled_image_update_after_bind: VkBool32,
    pub descriptor_binding_storage_image_update_after_bind: VkBool32,
    pub descriptor_binding_storage_buffer_update_after_bind: VkBool32,
    pub descriptor_binding_uniform_texel_buffer_update_after_bind: VkBool32,
    pub descriptor_binding_storage_texel_buffer_update_after_bind: VkBool32,
    pub descriptor_binding_update_unused_while_pending: VkBool32,
    pub descriptor_binding_partially_bound: VkBool32,
    pub descriptor_binding_variable_descriptor_count: VkBool32,
    pub runtime_descriptor_array: VkBool32,
    pub sampler_filter_minmax: VkBool32,
    pub scalar_block_layout: VkBool32,
    pub imageless_framebuffer: VkBool32,
    pub uniform_buffer_standard_layout: VkBool32,
    pub shader_subgroup_extended_types: VkBool32,
    pub separate_depth_stencil_layouts: VkBool32,
    pub host_query_reset: VkBool32,
    pub timeline_semaphore: VkBool32,
    pub buffer_device_address: VkBool32,
    pub buffer_device_address_capture_replay: VkBool32,
    pub buffer_device_address_multi_device: VkBool32,
    pub vulkan_memory_model: VkBool32,
    pub vulkan_memory_model_device_scope: VkBool32,
    pub vulkan_memory_model_availability_visibility_chains: VkBool32,
    pub shader_output_viewport_index: VkBool32,
    pub shader_output_layer: VkBool32,
    pub subgroup_broadcast_dynamic_id: VkBool32,
}

/// `VkPhysicalDeviceFeatures` — the CORE (Vulkan 1.0) feature-enable struct passed via
/// [`VkDeviceCreateInfo::p_enabled_features`] (T-dev: `samplerAnisotropy`). Field-exact,
/// in `vulkan_core.h` declaration order, so a raw pointer cast is ABI-correct.
///
/// Deliberately NOT chained through `VkPhysicalDeviceFeatures2`/`pNext` — the two are
/// mutually exclusive at `vkCreateDevice` (VUID-VkDeviceCreateInfo-pNext-00373: a
/// `VkPhysicalDeviceFeatures2` in `pNext` supersedes `p_enabled_features`, so combining
/// them is invalid). `#[derive(Default)]` gives the all-`VK_FALSE` baseline (every
/// `VkBool32` is `u32`, whose `Default` is `0`); callers flip only the bits they enable.
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct VkPhysicalDeviceFeatures {
    pub robust_buffer_access: VkBool32,
    pub full_draw_index_uint32: VkBool32,
    pub image_cube_array: VkBool32,
    pub independent_blend: VkBool32,
    pub geometry_shader: VkBool32,
    pub tessellation_shader: VkBool32,
    pub sample_rate_shading: VkBool32,
    pub dual_src_blend: VkBool32,
    pub logic_op: VkBool32,
    pub multi_draw_indirect: VkBool32,
    pub draw_indirect_first_instance: VkBool32,
    pub depth_clamp: VkBool32,
    pub depth_bias_clamp: VkBool32,
    pub fill_mode_non_solid: VkBool32,
    pub depth_bounds: VkBool32,
    pub wide_lines: VkBool32,
    pub large_points: VkBool32,
    pub alpha_to_one: VkBool32,
    pub multi_viewport: VkBool32,
    pub sampler_anisotropy: VkBool32,
    pub texture_compression_etc2: VkBool32,
    pub texture_compression_astc_ldr: VkBool32,
    pub texture_compression_bc: VkBool32,
    pub occlusion_query_precise: VkBool32,
    pub pipeline_statistics_query: VkBool32,
    pub vertex_pipeline_stores_and_atomics: VkBool32,
    pub fragment_stores_and_atomics: VkBool32,
    pub shader_tessellation_and_geometry_point_size: VkBool32,
    pub shader_image_gather_extended: VkBool32,
    pub shader_storage_image_extended_formats: VkBool32,
    pub shader_storage_image_multisample: VkBool32,
    pub shader_storage_image_read_without_format: VkBool32,
    pub shader_storage_image_write_without_format: VkBool32,
    pub shader_uniform_buffer_array_dynamic_indexing: VkBool32,
    pub shader_sampled_image_array_dynamic_indexing: VkBool32,
    pub shader_storage_buffer_array_dynamic_indexing: VkBool32,
    pub shader_storage_image_array_dynamic_indexing: VkBool32,
    pub shader_clip_distance: VkBool32,
    pub shader_cull_distance: VkBool32,
    pub shader_float64: VkBool32,
    pub shader_int64: VkBool32,
    pub shader_int16: VkBool32,
    pub shader_resource_residency: VkBool32,
    pub shader_resource_min_lod: VkBool32,
    pub sparse_binding: VkBool32,
    pub sparse_residency_buffer: VkBool32,
    pub sparse_residency_image_2d: VkBool32,
    pub sparse_residency_image_3d: VkBool32,
    pub sparse_residency_2_samples: VkBool32,
    pub sparse_residency_4_samples: VkBool32,
    pub sparse_residency_8_samples: VkBool32,
    pub sparse_residency_16_samples: VkBool32,
    pub sparse_residency_aliased: VkBool32,
    pub variable_multisample_rate: VkBool32,
    pub inherited_queries: VkBool32,
}

/// `VkPhysicalDeviceDescriptorIndexingFeatures` (T-dev) — the GRANULAR bindless
/// feature struct. Chained into `VkPhysicalDeviceFeatures2` to READ (the
/// `bindless_capable` query) and into `VkDeviceCreateInfo` to ENABLE exactly the 5
/// bits the query gates: `shader_sampled_image_array_non_uniform_indexing`,
/// `runtime_descriptor_array`, `descriptor_binding_partially_bound`,
/// `descriptor_binding_variable_descriptor_count`,
/// `descriptor_binding_sampled_image_update_after_bind`. Field-exact (in
/// `vulkan_core.h` declaration order — identical to the tail of
/// [`VkPhysicalDeviceVulkan12Features`] from `shader_input_attachment_array_dynamic_indexing`
/// through `runtime_descriptor_array`), so the driver, walking `p_next`, writes/reads
/// every bool it owns without stepping past our footprint. Deliberately carries NO
/// `buffer_device_address` field (unlike the `Vulkan12Features` aggregate), so it
/// coexists cleanly with the hwrt arm's standalone
/// `VkPhysicalDeviceBufferDeviceAddressFeatures` in the same `pNext` chain.
/// `VkPhysicalDeviceHostQueryResetFeatures` — the GRANULAR `hostQueryReset` feature struct
/// (profiling rung 4). Chained into `VkPhysicalDeviceFeatures2` to READ whether the device
/// advertises host query reset, and into `VkDeviceCreateInfo` to ENABLE it when it does.
///
/// **Granular, not the aggregate, for exactly the reason the sibling above states.**
/// `hostQueryReset` also lives in [`VkPhysicalDeviceVulkan12Features`], but that aggregate
/// carries `descriptorIndexing`'s bits too, and VUID-VkDeviceCreateInfo-pNext-02830 forbids
/// a promoted struct's aggregate alongside its own granular sub-struct — which this chain
/// already has.
///
/// vulkan_core.h: `VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_HOST_QUERY_RESET_FEATURES = 1000261000`.
#[repr(C)]
pub struct VkPhysicalDeviceHostQueryResetFeatures {
    pub s_type: VkStructureType,
    pub p_next: *mut c_void,
    pub host_query_reset: VkBool32,
}

#[repr(C)]
pub struct VkPhysicalDeviceDescriptorIndexingFeatures {
    pub s_type: VkStructureType,
    pub p_next: *mut c_void,
    pub shader_input_attachment_array_dynamic_indexing: VkBool32,
    pub shader_uniform_texel_buffer_array_dynamic_indexing: VkBool32,
    pub shader_storage_texel_buffer_array_dynamic_indexing: VkBool32,
    pub shader_uniform_buffer_array_non_uniform_indexing: VkBool32,
    pub shader_sampled_image_array_non_uniform_indexing: VkBool32,
    pub shader_storage_buffer_array_non_uniform_indexing: VkBool32,
    pub shader_storage_image_array_non_uniform_indexing: VkBool32,
    pub shader_input_attachment_array_non_uniform_indexing: VkBool32,
    pub shader_uniform_texel_buffer_array_non_uniform_indexing: VkBool32,
    pub shader_storage_texel_buffer_array_non_uniform_indexing: VkBool32,
    pub descriptor_binding_uniform_buffer_update_after_bind: VkBool32,
    pub descriptor_binding_sampled_image_update_after_bind: VkBool32,
    pub descriptor_binding_storage_image_update_after_bind: VkBool32,
    pub descriptor_binding_storage_buffer_update_after_bind: VkBool32,
    pub descriptor_binding_uniform_texel_buffer_update_after_bind: VkBool32,
    pub descriptor_binding_storage_texel_buffer_update_after_bind: VkBool32,
    pub descriptor_binding_update_unused_while_pending: VkBool32,
    pub descriptor_binding_partially_bound: VkBool32,
    pub descriptor_binding_variable_descriptor_count: VkBool32,
    pub runtime_descriptor_array: VkBool32,
}

/// `VkPresentInfoKHR`.
#[repr(C)]
pub struct VkPresentInfoKhr {
    pub s_type: VkStructureType,
    pub p_next: *const c_void,
    pub wait_semaphore_count: u32,
    pub p_wait_semaphores: *const VkSemaphore,
    pub swapchain_count: u32,
    pub p_swapchains: *const VkSwapchainKHR,
    pub p_image_indices: *const u32,
    /// `VkResult*` per-swapchain out-results — null (one swapchain, the call
    /// result suffices).
    pub p_results: *mut i32,
}

// FFI layout guards for the Slice-1 driver-written + driver-read structs.
// `VkSurfaceCapabilitiesKHR` / `VkSurfaceFormatKHR` are written BY the driver →
// their size/align MUST match the C ABI or the driver overruns our out-buffer.
const _: () = assert!(core::mem::size_of::<VkSurfaceCapabilitiesKhr>() == 52);
const _: () = assert!(core::mem::size_of::<VkSurfaceFormatKhr>() == 8);
const _: () = assert!(core::mem::size_of::<VkExtent2D>() == 8);
const _: () = assert!(core::mem::size_of::<VkImageMemoryBarrier>() == 72);
const _: () = assert!(core::mem::align_of::<VkImageMemoryBarrier>() == 8);
const _: () = assert!(core::mem::size_of::<VkImageSubresourceRange>() == 20);
const _: () = assert!(core::mem::size_of::<VkComponentMapping>() == 16);
const _: () = assert!(core::mem::size_of::<VkClearValue>() == 16);
const _: () = assert!(core::mem::size_of::<VkRect2D>() == 16);
const _: () = assert!(core::mem::size_of::<VkPhysicalDeviceVulkan13Features>() == 80);
// S0 image / features2 layout guards. `VkImageCreateInfo` is read BY the driver;
// `VkPhysicalDeviceFeatures2` is written BY the driver through its out-pointer, so
// both must match the C ABI exactly.
const _: () = assert!(core::mem::size_of::<VkExtent3D>() == 12);
const _: () = assert!(core::mem::size_of::<VkImageCreateInfo>() == 88);
const _: () = assert!(core::mem::align_of::<VkImageCreateInfo>() == 8);
const _: () = assert!(core::mem::size_of::<VkImageSubresourceLayers>() == 16);
const _: () = assert!(core::mem::size_of::<VkBufferImageCopy>() == 56);
const _: () = assert!(core::mem::align_of::<VkBufferImageCopy>() == 8);
// T2 mip-chain blit: `VkImageBlit` is `VkImageSubresourceLayers` (16 bytes) +
// `[VkOffset3D; 2]` (24 bytes) x2 = 80 bytes, 4-byte aligned (every field is
// `i32`/`u32`, no 8-byte member).
const _: () = assert!(core::mem::size_of::<VkImageBlit>() == 80);
const _: () = assert!(core::mem::align_of::<VkImageBlit>() == 4);
// 16-byte head (sType + 4 pad + pNext) + [VkBool32; 55] = 220 bytes → 236, rounded
// up to the struct's 8-byte alignment = 240.
const _: () = assert!(core::mem::size_of::<VkPhysicalDeviceFeatures2>() == 240);
const _: () = assert!(core::mem::align_of::<VkPhysicalDeviceFeatures2>() == 8);
// Render P1b device-caps query layout guards. `VkFormatProperties` is written BY the
// driver; `VkPhysicalDeviceVulkan12Features` is written BY the driver through the
// `p_next` chain — both must match the C ABI exactly. `VkFormatProperties` is three
// `VkFlags` (12 bytes). `VkPhysicalDeviceVulkan12Features` is the 16-byte head
// (sType + 4 pad + pNext) + 47 `VkBool32`s (188 bytes) = 204, rounded up to the
// 8-byte alignment = 208.
const _: () = assert!(core::mem::size_of::<VkFormatProperties>() == 12);
const _: () = assert!(core::mem::size_of::<VkPhysicalDeviceVulkan12Features>() == 208);
const _: () = assert!(core::mem::align_of::<VkPhysicalDeviceVulkan12Features>() == 8);
// T-dev bindless device-feature layout guards. `VkPhysicalDeviceFeatures` is READ by
// the driver through `p_enabled_features` (55 `VkBool32`s = 220 bytes, 4-byte aligned —
// no pointer member, unlike the sType-headed feature structs above).
// `VkPhysicalDeviceDescriptorIndexingFeatures` is written/read BY the driver through the
// `p_next` chain: the 16-byte head (sType + 4 pad + pNext) + 20 `VkBool32`s (80 bytes) =
// 96, already 8-byte aligned (no rounding needed).
const _: () = assert!(core::mem::size_of::<VkPhysicalDeviceFeatures>() == 220);
const _: () = assert!(core::mem::align_of::<VkPhysicalDeviceFeatures>() == 4);
const _: () = assert!(core::mem::size_of::<VkPhysicalDeviceDescriptorIndexingFeatures>() == 96);
const _: () = assert!(core::mem::align_of::<VkPhysicalDeviceDescriptorIndexingFeatures>() == 8);
// Profiling rung 4: the 16-byte sType+pNext head + one `VkBool32` (4 bytes), rounded up to
// the 8-byte alignment = 24. Written BY the driver on the query pass and READ by it on the
// enable pass, so the layout is pinned exactly like its siblings above.
const _: () = assert!(core::mem::size_of::<VkPhysicalDeviceHostQueryResetFeatures>() == 24);
const _: () = assert!(core::mem::align_of::<VkPhysicalDeviceHostQueryResetFeatures>() == 8);
// T4 bindless: both `p_next`-chained structs are READ by the driver (we author every
// byte), but a wrong field ORDER still misfeeds the driver a garbage count/pointer
// (a real OOB-read hazard, not just a Rust-side type error) — pinned like every other
// chained struct in this file. Both are the 16-byte sType+pNext head + one `u32` count
// (padded to 8) + one pointer = 32 bytes, 8-byte aligned.
const _: () = assert!(core::mem::size_of::<VkDescriptorSetLayoutBindingFlagsCreateInfo>() == 32);
const _: () = assert!(core::mem::align_of::<VkDescriptorSetLayoutBindingFlagsCreateInfo>() == 8);
const _: () =
    assert!(core::mem::size_of::<VkDescriptorSetVariableDescriptorCountAllocateInfo>() == 32);
const _: () =
    assert!(core::mem::align_of::<VkDescriptorSetVariableDescriptorCountAllocateInfo>() == 8);

// FFI layout guards for the new structs. The callback-data struct is written BY
// the driver and read through the callback, so its size/align must match the C
// ABI; the create-infos/barriers are written BY us but their layout still must
// match for the driver to read them. These break the build on any drift.
const _: () = assert!(core::mem::size_of::<VkDebugUtilsMessengerCallbackDataExt>() == 96);
const _: () = assert!(core::mem::align_of::<VkDebugUtilsMessengerCallbackDataExt>() == 8);
const _: () = assert!(core::mem::size_of::<VkBufferMemoryBarrier>() == 56);
const _: () = assert!(core::mem::align_of::<VkBufferMemoryBarrier>() == 8);
const _: () = assert!(core::mem::size_of::<VkBufferCopy>() == 24);
const _: () = assert!(core::mem::align_of::<VkBufferCopy>() == 8);
const _: () = assert!(core::mem::size_of::<VkDescriptorBufferInfo>() == 24);
const _: () = assert!(core::mem::size_of::<VkDescriptorSetLayoutBinding>() == 24);
const _: () = assert!(core::mem::size_of::<VkPushConstantRange>() == 12);
const _: () = assert!(core::mem::size_of::<VkDescriptorPoolSize>() == 8);

// Phase-6 S0 rung-5 sampler + combined-image-sampler descriptor layout guards.
// `VkDescriptorImageInfo` = (VkSampler u64, VkImageView u64, i32 layout + 4 pad) =
// 24 B. `VkSamplerCreateInfo`: 16-byte head (sType + 4 pad + pNext), then flags +
// 5×i32 (mag/min/mipmap/u/v) = 24 B → 40, +i32 w + f32 bias = 48, +VkBool32 +
// f32 + VkBool32 + i32 + f32 + f32 + i32 + VkBool32 = 8×4 = 80 B. No 8-byte
// member after the head, so the tail packs to 80 with the struct's 8-byte align.
const _: () = assert!(core::mem::size_of::<VkDescriptorImageInfo>() == 24);
const _: () = assert!(core::mem::align_of::<VkDescriptorImageInfo>() == 8);
const _: () = assert!(core::mem::size_of::<VkSamplerCreateInfo>() == 80);
const _: () = assert!(core::mem::align_of::<VkSamplerCreateInfo>() == 8);

// Phase-6 S0 rung-2 graphics-pipeline create-info layout guards. Each struct is
// read BY the driver in `vkCreateGraphicsPipelines`, so the Rust `#[repr(C)]`
// layout MUST match the C ABI or the driver reads garbage at a shifted offset.
// (Sizes are the x86_64 / LP64 C ABI footprints with 8-byte pointer alignment.)
const _: () = assert!(core::mem::size_of::<VkViewport>() == 24);
// Phase-6 S0 rung-3 vertex-input descriptions (read BY the driver in
// `vkCreateGraphicsPipelines`): `(u32, u32, i32)` = 12 B; `(u32, u32, i32, u32)` = 16 B.
const _: () = assert!(core::mem::size_of::<VkVertexInputBindingDescription>() == 12);
const _: () = assert!(core::mem::size_of::<VkVertexInputAttributeDescription>() == 16);
const _: () = assert!(core::mem::size_of::<VkPipelineVertexInputStateCreateInfo>() == 48);
const _: () = assert!(core::mem::size_of::<VkPipelineInputAssemblyStateCreateInfo>() == 32);
const _: () = assert!(core::mem::size_of::<VkPipelineViewportStateCreateInfo>() == 48);
const _: () = assert!(core::mem::align_of::<VkPipelineViewportStateCreateInfo>() == 8);
const _: () = assert!(core::mem::size_of::<VkPipelineRasterizationStateCreateInfo>() == 64);
const _: () = assert!(core::mem::size_of::<VkPipelineMultisampleStateCreateInfo>() == 48);
const _: () = assert!(core::mem::align_of::<VkPipelineMultisampleStateCreateInfo>() == 8);
const _: () = assert!(core::mem::size_of::<VkPipelineColorBlendAttachmentState>() == 32);
const _: () = assert!(core::mem::size_of::<VkPipelineColorBlendStateCreateInfo>() == 56);
const _: () = assert!(core::mem::align_of::<VkPipelineColorBlendStateCreateInfo>() == 8);
const _: () = assert!(core::mem::size_of::<VkPipelineDynamicStateCreateInfo>() == 32);
// Phase-6 S0 rung-4 depth-stencil state (read BY the driver in
// `vkCreateGraphicsPipelines` when a depth format is declared). `VkStencilOpState`
// is 7 × u32 = 28 B; the depth-stencil create-info is the standard 104-B LP64
// footprint. `VkClearDepthStencilValue` is `{ f32, u32 }` = 8 B, and the
// `VkClearValue` union is still 16 B / align 4 (the color variant remains largest).
const _: () = assert!(core::mem::size_of::<VkStencilOpState>() == 28);
const _: () = assert!(core::mem::size_of::<VkPipelineDepthStencilStateCreateInfo>() == 104);
const _: () = assert!(core::mem::align_of::<VkPipelineDepthStencilStateCreateInfo>() == 8);
const _: () = assert!(core::mem::size_of::<VkClearDepthStencilValue>() == 8);
// VkClearValue size (== 16) is already asserted in the base layout-guard block
// above; the union conversion newly needs only the align guard.
const _: () = assert!(core::mem::align_of::<VkClearValue>() == 4);
const _: () = assert!(core::mem::size_of::<VkPipelineRenderingCreateInfo>() == 40);
const _: () = assert!(core::mem::align_of::<VkPipelineRenderingCreateInfo>() == 8);
const _: () = assert!(core::mem::size_of::<VkGraphicsPipelineCreateInfo>() == 144);
const _: () = assert!(core::mem::align_of::<VkGraphicsPipelineCreateInfo>() == 8);

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

/// `PFN_vkEnumerateInstanceLayerProperties` — a global command (NULL instance).
pub type PfnVkEnumerateInstanceLayerProperties = unsafe extern "system" fn(
    p_count: *mut u32,
    p_properties: *mut VkLayerProperties,
) -> i32;

/// `PFN_vkEnumerateInstanceExtensionProperties` — a global command. `p_layer_name`
/// is null to query the instance's own (non-layer) extensions.
pub type PfnVkEnumerateInstanceExtensionProperties = unsafe extern "system" fn(
    p_layer_name: *const c_char,
    p_count: *mut u32,
    p_properties: *mut VkExtensionProperties,
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

// ---------------------------------------------------------------------------
// Slice-0 0a — VK_EXT_debug_utils PFNs (instance-scope extension commands).
// ---------------------------------------------------------------------------

/// `PFN_vkCreateDebugUtilsMessengerEXT`.
pub type PfnVkCreateDebugUtilsMessengerExt = unsafe extern "system" fn(
    instance: VkInstance,
    p_create_info: *const VkDebugUtilsMessengerCreateInfoExt,
    p_allocator: *const c_void,
    p_messenger: *mut VkDebugUtilsMessengerEXT,
) -> i32;

/// `PFN_vkDestroyDebugUtilsMessengerEXT`.
pub type PfnVkDestroyDebugUtilsMessengerExt = unsafe extern "system" fn(
    instance: VkInstance,
    messenger: VkDebugUtilsMessengerEXT,
    p_allocator: *const c_void,
);

// ---------------------------------------------------------------------------
// Slice-0 0c/0d — compute / descriptor / command device-scope PFNs.
// ---------------------------------------------------------------------------

/// `PFN_vkCreateShaderModule`.
pub type PfnVkCreateShaderModule = unsafe extern "system" fn(
    device: VkDevice,
    p_create_info: *const VkShaderModuleCreateInfo,
    p_allocator: *const c_void,
    p_shader_module: *mut VkShaderModule,
) -> i32;

/// `PFN_vkDestroyShaderModule`.
pub type PfnVkDestroyShaderModule = unsafe extern "system" fn(
    device: VkDevice,
    shader_module: VkShaderModule,
    p_allocator: *const c_void,
);

/// `PFN_vkCreateSampler` (Phase-6 S0 rung 5).
pub type PfnVkCreateSampler = unsafe extern "system" fn(
    device: VkDevice,
    p_create_info: *const VkSamplerCreateInfo,
    p_allocator: *const c_void,
    p_sampler: *mut VkSampler,
) -> i32;

/// `PFN_vkDestroySampler` (Phase-6 S0 rung 5).
pub type PfnVkDestroySampler =
    unsafe extern "system" fn(device: VkDevice, sampler: VkSampler, p_allocator: *const c_void);

/// `PFN_vkCreateDescriptorSetLayout`.
pub type PfnVkCreateDescriptorSetLayout = unsafe extern "system" fn(
    device: VkDevice,
    p_create_info: *const VkDescriptorSetLayoutCreateInfo,
    p_allocator: *const c_void,
    p_set_layout: *mut VkDescriptorSetLayout,
) -> i32;

/// `PFN_vkDestroyDescriptorSetLayout`.
pub type PfnVkDestroyDescriptorSetLayout = unsafe extern "system" fn(
    device: VkDevice,
    set_layout: VkDescriptorSetLayout,
    p_allocator: *const c_void,
);

/// `PFN_vkCreatePipelineLayout`.
pub type PfnVkCreatePipelineLayout = unsafe extern "system" fn(
    device: VkDevice,
    p_create_info: *const VkPipelineLayoutCreateInfo,
    p_allocator: *const c_void,
    p_pipeline_layout: *mut VkPipelineLayout,
) -> i32;

/// `PFN_vkDestroyPipelineLayout`.
pub type PfnVkDestroyPipelineLayout = unsafe extern "system" fn(
    device: VkDevice,
    pipeline_layout: VkPipelineLayout,
    p_allocator: *const c_void,
);

/// `PFN_vkCreateComputePipelines`. `pipeline_cache` is null for Slice 0;
/// `create_info_count` pipelines are written into `p_pipelines`.
pub type PfnVkCreateComputePipelines = unsafe extern "system" fn(
    device: VkDevice,
    pipeline_cache: u64,
    create_info_count: u32,
    p_create_infos: *const VkComputePipelineCreateInfo,
    p_allocator: *const c_void,
    p_pipelines: *mut VkPipeline,
) -> i32;

/// `PFN_vkCreateGraphicsPipelines` (Phase-6 S0 rung 2). `pipeline_cache` is null;
/// `create_info_count` pipelines are written into `p_pipelines`.
pub type PfnVkCreateGraphicsPipelines = unsafe extern "system" fn(
    device: VkDevice,
    pipeline_cache: u64,
    create_info_count: u32,
    p_create_infos: *const VkGraphicsPipelineCreateInfo,
    p_allocator: *const c_void,
    p_pipelines: *mut VkPipeline,
) -> i32;

/// `PFN_vkDestroyPipeline`.
pub type PfnVkDestroyPipeline =
    unsafe extern "system" fn(device: VkDevice, pipeline: VkPipeline, p_allocator: *const c_void);

/// `PFN_vkCreateDescriptorPool`.
pub type PfnVkCreateDescriptorPool = unsafe extern "system" fn(
    device: VkDevice,
    p_create_info: *const VkDescriptorPoolCreateInfo,
    p_allocator: *const c_void,
    p_descriptor_pool: *mut VkDescriptorPool,
) -> i32;

/// `PFN_vkDestroyDescriptorPool`.
pub type PfnVkDestroyDescriptorPool = unsafe extern "system" fn(
    device: VkDevice,
    descriptor_pool: VkDescriptorPool,
    p_allocator: *const c_void,
);

/// `PFN_vkAllocateDescriptorSets`.
pub type PfnVkAllocateDescriptorSets = unsafe extern "system" fn(
    device: VkDevice,
    p_allocate_info: *const VkDescriptorSetAllocateInfo,
    p_descriptor_sets: *mut VkDescriptorSet,
) -> i32;

/// `PFN_vkUpdateDescriptorSets`.
pub type PfnVkUpdateDescriptorSets = unsafe extern "system" fn(
    device: VkDevice,
    descriptor_write_count: u32,
    p_descriptor_writes: *const VkWriteDescriptorSet,
    descriptor_copy_count: u32,
    p_descriptor_copies: *const c_void,
);

/// `PFN_vkCreateCommandPool`.
pub type PfnVkCreateCommandPool = unsafe extern "system" fn(
    device: VkDevice,
    p_create_info: *const VkCommandPoolCreateInfo,
    p_allocator: *const c_void,
    p_command_pool: *mut VkCommandPool,
) -> i32;

/// `PFN_vkDestroyCommandPool`.
pub type PfnVkDestroyCommandPool = unsafe extern "system" fn(
    device: VkDevice,
    command_pool: VkCommandPool,
    p_allocator: *const c_void,
);

/// `PFN_vkAllocateCommandBuffers`.
pub type PfnVkAllocateCommandBuffers = unsafe extern "system" fn(
    device: VkDevice,
    p_allocate_info: *const VkCommandBufferAllocateInfo,
    p_command_buffers: *mut VkCommandBuffer,
) -> i32;

/// `PFN_vkFreeCommandBuffers`.
pub type PfnVkFreeCommandBuffers = unsafe extern "system" fn(
    device: VkDevice,
    command_pool: VkCommandPool,
    command_buffer_count: u32,
    p_command_buffers: *const VkCommandBuffer,
);

/// `PFN_vkBeginCommandBuffer`.
pub type PfnVkBeginCommandBuffer = unsafe extern "system" fn(
    command_buffer: VkCommandBuffer,
    p_begin_info: *const VkCommandBufferBeginInfo,
) -> i32;

/// `PFN_vkEndCommandBuffer`.
pub type PfnVkEndCommandBuffer =
    unsafe extern "system" fn(command_buffer: VkCommandBuffer) -> i32;

/// `PFN_vkCmdBindPipeline`.
pub type PfnVkCmdBindPipeline = unsafe extern "system" fn(
    command_buffer: VkCommandBuffer,
    pipeline_bind_point: i32,
    pipeline: VkPipeline,
);

/// `PFN_vkCmdBindDescriptorSets`.
pub type PfnVkCmdBindDescriptorSets = unsafe extern "system" fn(
    command_buffer: VkCommandBuffer,
    pipeline_bind_point: i32,
    layout: VkPipelineLayout,
    first_set: u32,
    descriptor_set_count: u32,
    p_descriptor_sets: *const VkDescriptorSet,
    dynamic_offset_count: u32,
    p_dynamic_offsets: *const u32,
);

/// `PFN_vkCmdPushConstants`.
pub type PfnVkCmdPushConstants = unsafe extern "system" fn(
    command_buffer: VkCommandBuffer,
    layout: VkPipelineLayout,
    stage_flags: VkFlags,
    offset: u32,
    size: u32,
    p_values: *const c_void,
);

/// `PFN_vkCmdDispatch`.
pub type PfnVkCmdDispatch = unsafe extern "system" fn(
    command_buffer: VkCommandBuffer,
    group_count_x: u32,
    group_count_y: u32,
    group_count_z: u32,
);

// --- Virtual-geometry rung R1: the indirect seam. Both commands are Vulkan 1.0 CORE and need no
//     feature bit, which is what makes this rung free. Their `Count` variants (`vkCmdDrawIndexed-
//     IndirectCount`) are NOT: those need `drawIndirectCount` in a `VkPhysicalDeviceVulkan12Features`
//     this device never chains, so they belong to a later rung and are deliberately absent here. ---

/// `PFN_vkCmdDispatchIndirect` — a compute dispatch whose `VkDispatchIndirectCommand`
/// (three `u32` group counts) is FETCHED FROM `buffer` at `offset` by the GPU, so the
/// group count can be decided by an earlier pass instead of by the host.
pub type PfnVkCmdDispatchIndirect = unsafe extern "system" fn(
    command_buffer: VkCommandBuffer,
    buffer: VkBuffer,
    offset: VkDeviceSize,
);

/// `PFN_vkCmdDrawIndexedIndirect` — `draw_count` indexed draws whose
/// `VkDrawIndexedIndirectCommand` records are fetched from `buffer` starting at `offset`
/// with `stride` bytes between them. The record count is still host-supplied; only the
/// record CONTENTS are GPU-decided (the fully GPU-decided count needs the `Count` variant
/// and its feature bit — see the note above).
pub type PfnVkCmdDrawIndexedIndirect = unsafe extern "system" fn(
    command_buffer: VkCommandBuffer,
    buffer: VkBuffer,
    offset: VkDeviceSize,
    draw_count: u32,
    stride: u32,
);

// --- Phase-6 S0 rung-2 graphics draw commands (Vulkan 1.0 core). ---

/// `PFN_vkCmdSetViewport` — dynamic viewport state (`first_viewport`/`count` +
/// `p_viewports`).
pub type PfnVkCmdSetViewport = unsafe extern "system" fn(
    command_buffer: VkCommandBuffer,
    first_viewport: u32,
    viewport_count: u32,
    p_viewports: *const VkViewport,
);

/// `PFN_vkCmdSetScissor` — dynamic scissor state.
pub type PfnVkCmdSetScissor = unsafe extern "system" fn(
    command_buffer: VkCommandBuffer,
    first_scissor: u32,
    scissor_count: u32,
    p_scissors: *const VkRect2D,
);

/// `PFN_vkCmdDraw` — a non-indexed draw.
pub type PfnVkCmdDraw = unsafe extern "system" fn(
    command_buffer: VkCommandBuffer,
    vertex_count: u32,
    instance_count: u32,
    first_vertex: u32,
    first_instance: u32,
);

/// `PFN_vkCmdDrawIndexed` — an indexed draw. Records `vkCmdDrawIndexed`; requires a
/// bound index buffer (`vkCmdBindIndexBuffer`). `vertex_offset` is `i32` (added to the
/// fetched index before vertex lookup, per the Vulkan spec).
pub type PfnVkCmdDrawIndexed = unsafe extern "system" fn(
    command_buffer: VkCommandBuffer,
    index_count: u32,
    instance_count: u32,
    first_index: u32,
    vertex_offset: i32,
    first_instance: u32,
);

/// `PFN_vkCmdBindVertexBuffers` — binds `binding_count` vertex buffers starting at
/// `first_binding` (rung 3 binds one).
pub type PfnVkCmdBindVertexBuffers = unsafe extern "system" fn(
    command_buffer: VkCommandBuffer,
    first_binding: u32,
    binding_count: u32,
    p_buffers: *const VkBuffer,
    p_offsets: *const VkDeviceSize,
);

/// `PFN_vkCmdBindIndexBuffer` — binds an index buffer (rung-3 seam; non-indexed
/// rung 3 does not call it).
pub type PfnVkCmdBindIndexBuffer = unsafe extern "system" fn(
    command_buffer: VkCommandBuffer,
    buffer: VkBuffer,
    offset: VkDeviceSize,
    index_type: i32,
);

/// `PFN_vkCmdPipelineBarrier`.
pub type PfnVkCmdPipelineBarrier = unsafe extern "system" fn(
    command_buffer: VkCommandBuffer,
    src_stage_mask: VkFlags,
    dst_stage_mask: VkFlags,
    dependency_flags: VkFlags,
    memory_barrier_count: u32,
    p_memory_barriers: *const c_void,
    buffer_memory_barrier_count: u32,
    p_buffer_memory_barriers: *const VkBufferMemoryBarrier,
    image_memory_barrier_count: u32,
    p_image_memory_barriers: *const c_void,
);

/// `PFN_vkCmdCopyBuffer`.
///
/// SAFETY (ABI): the signature mirrors the Vulkan spec's `vkCmdCopyBuffer`
/// — `(VkCommandBuffer, VkBuffer src, VkBuffer dst, u32 regionCount,
/// const VkBufferCopy* pRegions)`, all parameter types `#[repr(C)]`/transparent
/// — so transmuting the loader-resolved function pointer to this typedef is
/// sound (size-checked by `load_device_command`'s `debug_assert`).
pub type PfnVkCmdCopyBuffer = unsafe extern "system" fn(
    command_buffer: VkCommandBuffer,
    src_buffer: VkBuffer,
    dst_buffer: VkBuffer,
    region_count: u32,
    p_regions: *const VkBufferCopy,
);

/// `PFN_vkCmdFillBuffer` — fills `size` bytes of `dst_buffer` from `dst_offset` with the
/// 4-byte `data` pattern (Vulkan 1.0 core, always present). The Lighting-L1 cull resets its
/// `LightIndexAlloc` counter to 0 with this before each frame's cull dispatch.
pub type PfnVkCmdFillBuffer = unsafe extern "system" fn(
    command_buffer: VkCommandBuffer,
    dst_buffer: VkBuffer,
    dst_offset: VkDeviceSize,
    size: VkDeviceSize,
    data: u32,
);

/// `PFN_vkCmdUpdateBuffer` — writes up to 65536 bytes INLINE in the command buffer.
///
/// Virtual-geometry rung R2a′. Vulkan 1.0 core, no feature bit. It is a TRANSFER-stage operation,
/// which is exactly why it is used here instead of a host-visible buffer: a host write completed
/// before `vkQueueSubmit` needs no barrier at all, so a host-filled indirect buffer would exercise
/// none of the indirect-barrier plumbing this rung exists to de-risk.
///
/// ⚠️ Must be recorded OUTSIDE a render-pass instance (`VUID-vkCmdUpdateBuffer-renderpass`), and
/// both `dst_offset` and `data_size` must be multiples of 4.
pub type PfnVkCmdUpdateBuffer = unsafe extern "system" fn(
    command_buffer: VkCommandBuffer,
    dst_buffer: VkBuffer,
    dst_offset: VkDeviceSize,
    data_size: VkDeviceSize,
    p_data: *const core::ffi::c_void,
);

/// `VkDrawIndexedIndirectCommand` — the 20-byte record `vkCmdDrawIndexedIndirect` fetches.
///
/// Field order and size are ABI, not a choice: the GPU reads this layout directly.
///
/// ⚠️ **`first_instance` MUST be 0 on this device.** `drawIndirectFirstInstance` is left `VK_FALSE`
/// (only `samplerAnisotropy` is enabled), and the validation layers cannot read buffer CONTENTS —
/// only GPU-assisted validation would catch a violation, so a nonzero value here is a silent
/// corruption class. Every producer asserts it host-side.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct VkDrawIndexedIndirectCommand {
    pub index_count: u32,
    pub instance_count: u32,
    pub first_index: u32,
    pub vertex_offset: i32,
    pub first_instance: u32,
}

/// Bytes per [`VkDrawIndexedIndirectCommand`] — the `stride` an indirect draw is given, and the
/// multiplier for a record's byte offset. A multiple of 4, so every record offset satisfies
/// `VUID-vkCmdDrawIndexedIndirect-offset-02710`.
pub const DRAW_INDEXED_INDIRECT_STRIDE: u32 = 20;

const _: () = assert!(
    core::mem::size_of::<VkDrawIndexedIndirectCommand>() == DRAW_INDEXED_INDIRECT_STRIDE as usize
);

/// `PFN_vkCmdClearColorImage` — clears the given subresource ranges of `image` (which must
/// be in `image_layout`, one of `GENERAL`/`TRANSFER_DST_OPTIMAL`) to `p_color` (Vulkan 1.0
/// core, always present). SDFDDGI I1 uses it to boot-clear the probe IRRADIANCE
/// (`B10G11R11_UFLOAT`) + DEPTH (`R16G16_SFLOAT`) atlases to defined values before the first
/// resolve can sample them (the uninitialized-read hazard fix).
pub type PfnVkCmdClearColorImage = unsafe extern "system" fn(
    command_buffer: VkCommandBuffer,
    image: VkImage,
    image_layout: i32,
    p_color: *const VkClearColorValue,
    range_count: u32,
    p_ranges: *const VkImageSubresourceRange,
);

/// `PFN_vkCreateFence`.
pub type PfnVkCreateFence = unsafe extern "system" fn(
    device: VkDevice,
    p_create_info: *const VkFenceCreateInfo,
    p_allocator: *const c_void,
    p_fence: *mut VkFence,
) -> i32;

/// `PFN_vkDestroyFence`.
pub type PfnVkDestroyFence =
    unsafe extern "system" fn(device: VkDevice, fence: VkFence, p_allocator: *const c_void);

/// `PFN_vkWaitForFences`.
pub type PfnVkWaitForFences = unsafe extern "system" fn(
    device: VkDevice,
    fence_count: u32,
    p_fences: *const VkFence,
    wait_all: VkBool32,
    timeout: u64,
) -> i32;

/// `PFN_vkQueueSubmit`.
pub type PfnVkQueueSubmit = unsafe extern "system" fn(
    queue: VkQueue,
    submit_count: u32,
    p_submits: *const VkSubmitInfo,
    fence: VkFence,
) -> i32;

/// `PFN_vkDeviceWaitIdle`.
pub type PfnVkDeviceWaitIdle = unsafe extern "system" fn(device: VkDevice) -> i32;

// ---------------------------------------------------------------------------
// Slice-1 — surface / swapchain / dynamic-rendering / image PFNs.
// ---------------------------------------------------------------------------

/// `PFN_vkCreateWin32SurfaceKHR` (instance-scope; `VK_KHR_win32_surface`).
pub type PfnVkCreateWin32SurfaceKhr = unsafe extern "system" fn(
    instance: VkInstance,
    p_create_info: *const VkWin32SurfaceCreateInfoKhr,
    p_allocator: *const c_void,
    p_surface: *mut VkSurfaceKHR,
) -> i32;

/// `PFN_vkDestroySurfaceKHR` (instance-scope; `VK_KHR_surface`).
pub type PfnVkDestroySurfaceKhr = unsafe extern "system" fn(
    instance: VkInstance,
    surface: VkSurfaceKHR,
    p_allocator: *const c_void,
);

/// `PFN_vkGetPhysicalDeviceSurfaceSupportKHR`.
pub type PfnVkGetPhysicalDeviceSurfaceSupportKhr = unsafe extern "system" fn(
    physical_device: VkPhysicalDevice,
    queue_family_index: u32,
    surface: VkSurfaceKHR,
    p_supported: *mut VkBool32,
) -> i32;

/// `PFN_vkGetPhysicalDeviceSurfaceCapabilitiesKHR`.
pub type PfnVkGetPhysicalDeviceSurfaceCapabilitiesKhr = unsafe extern "system" fn(
    physical_device: VkPhysicalDevice,
    surface: VkSurfaceKHR,
    p_capabilities: *mut VkSurfaceCapabilitiesKhr,
) -> i32;

/// `PFN_vkGetPhysicalDeviceSurfaceFormatsKHR`.
pub type PfnVkGetPhysicalDeviceSurfaceFormatsKhr = unsafe extern "system" fn(
    physical_device: VkPhysicalDevice,
    surface: VkSurfaceKHR,
    p_count: *mut u32,
    p_formats: *mut VkSurfaceFormatKhr,
) -> i32;

/// `PFN_vkGetPhysicalDeviceSurfacePresentModesKHR`.
pub type PfnVkGetPhysicalDeviceSurfacePresentModesKhr = unsafe extern "system" fn(
    physical_device: VkPhysicalDevice,
    surface: VkSurfaceKHR,
    p_count: *mut u32,
    p_present_modes: *mut i32,
) -> i32;

/// `PFN_vkCreateSwapchainKHR` (device-scope; `VK_KHR_swapchain`).
pub type PfnVkCreateSwapchainKhr = unsafe extern "system" fn(
    device: VkDevice,
    p_create_info: *const VkSwapchainCreateInfoKhr,
    p_allocator: *const c_void,
    p_swapchain: *mut VkSwapchainKHR,
) -> i32;

/// `PFN_vkDestroySwapchainKHR`.
pub type PfnVkDestroySwapchainKhr = unsafe extern "system" fn(
    device: VkDevice,
    swapchain: VkSwapchainKHR,
    p_allocator: *const c_void,
);

/// `PFN_vkGetSwapchainImagesKHR`.
pub type PfnVkGetSwapchainImagesKhr = unsafe extern "system" fn(
    device: VkDevice,
    swapchain: VkSwapchainKHR,
    p_count: *mut u32,
    p_images: *mut VkImage,
) -> i32;

/// `PFN_vkAcquireNextImageKHR`.
pub type PfnVkAcquireNextImageKhr = unsafe extern "system" fn(
    device: VkDevice,
    swapchain: VkSwapchainKHR,
    timeout: u64,
    semaphore: VkSemaphore,
    fence: VkFence,
    p_image_index: *mut u32,
) -> i32;

/// `PFN_vkQueuePresentKHR`.
pub type PfnVkQueuePresentKhr =
    unsafe extern "system" fn(queue: VkQueue, p_present_info: *const VkPresentInfoKhr) -> i32;

/// `PFN_vkCreateImageView`.
pub type PfnVkCreateImageView = unsafe extern "system" fn(
    device: VkDevice,
    p_create_info: *const VkImageViewCreateInfo,
    p_allocator: *const c_void,
    p_view: *mut VkImageView,
) -> i32;

/// `PFN_vkDestroyImageView`.
pub type PfnVkDestroyImageView = unsafe extern "system" fn(
    device: VkDevice,
    image_view: VkImageView,
    p_allocator: *const c_void,
);

/// `PFN_vkCreateSemaphore`.
pub type PfnVkCreateSemaphore = unsafe extern "system" fn(
    device: VkDevice,
    p_create_info: *const VkSemaphoreCreateInfo,
    p_allocator: *const c_void,
    p_semaphore: *mut VkSemaphore,
) -> i32;

/// `PFN_vkDestroySemaphore`.
pub type PfnVkDestroySemaphore = unsafe extern "system" fn(
    device: VkDevice,
    semaphore: VkSemaphore,
    p_allocator: *const c_void,
);

/// `PFN_vkResetFences`.
pub type PfnVkResetFences = unsafe extern "system" fn(
    device: VkDevice,
    fence_count: u32,
    p_fences: *const VkFence,
) -> i32;

// --- HW-RT rung R0 — GPU timestamp-query PFNs (Vulkan 1.0 core, always present). ---

/// `PFN_vkCreateQueryPool`.
pub type PfnVkCreateQueryPool = unsafe extern "system" fn(
    device: VkDevice,
    p_create_info: *const VkQueryPoolCreateInfo,
    p_allocator: *const c_void,
    p_query_pool: *mut VkQueryPool,
) -> i32;

/// `PFN_vkDestroyQueryPool`.
pub type PfnVkDestroyQueryPool = unsafe extern "system" fn(
    device: VkDevice,
    query_pool: VkQueryPool,
    p_allocator: *const c_void,
);

/// `PFN_vkCmdResetQueryPool`.
pub type PfnVkCmdResetQueryPool = unsafe extern "system" fn(
    command_buffer: VkCommandBuffer,
    query_pool: VkQueryPool,
    first_query: u32,
    query_count: u32,
);

/// `PFN_vkCmdWriteTimestamp` — `pipeline_stage` is a single `VkPipelineStageFlagBits`
/// (`VkFlags`) naming the stage at which the timestamp is written.
pub type PfnVkCmdWriteTimestamp = unsafe extern "system" fn(
    command_buffer: VkCommandBuffer,
    pipeline_stage: VkFlags,
    query_pool: VkQueryPool,
    query: u32,
);

/// `PFN_vkGetQueryPoolResults` — `stride`/`data_size` are `VkDeviceSize` (`u64`);
/// `flags` is a `VkQueryResultFlags` (`VkFlags`).
pub type PfnVkGetQueryPoolResults = unsafe extern "system" fn(
    device: VkDevice,
    query_pool: VkQueryPool,
    first_query: u32,
    query_count: u32,
    data_size: usize,
    p_data: *mut c_void,
    stride: VkDeviceSize,
    flags: VkFlags,
) -> i32;

/// `PFN_vkResetQueryPool` — resets `query_count` queries from `first_query` **on the
/// host**, with no command buffer and no queue submission.
///
/// Vulkan 1.2 core (promoted from `VK_EXT_host_query_reset`), so it loads on this
/// engine's 1.3 device — but calling it is legal only when the `hostQueryReset`
/// feature was ENABLED at device creation, which is why
/// [`VkPhysicalDeviceHostQueryResetFeatures`] exists beside it and why the capability
/// is recorded rather than assumed.
pub type PfnVkResetQueryPool = unsafe extern "system" fn(
    device: VkDevice,
    query_pool: VkQueryPool,
    first_query: u32,
    query_count: u32,
);

/// `PFN_vkCmdBeginRendering` (Vulkan 1.3 core dynamic rendering).
pub type PfnVkCmdBeginRendering = unsafe extern "system" fn(
    command_buffer: VkCommandBuffer,
    p_rendering_info: *const VkRenderingInfo,
);

/// `PFN_vkCmdEndRendering`.
pub type PfnVkCmdEndRendering = unsafe extern "system" fn(command_buffer: VkCommandBuffer);

// --- Phase-6 S0 image / features2 / image-copy commands. ---

/// `PFN_vkCreateImage`.
pub type PfnVkCreateImage = unsafe extern "system" fn(
    device: VkDevice,
    p_create_info: *const VkImageCreateInfo,
    p_allocator: *const c_void,
    p_image: *mut VkImage,
) -> i32;

/// `PFN_vkDestroyImage`.
pub type PfnVkDestroyImage =
    unsafe extern "system" fn(device: VkDevice, image: VkImage, p_allocator: *const c_void);

/// `PFN_vkGetImageMemoryRequirements`.
pub type PfnVkGetImageMemoryRequirements = unsafe extern "system" fn(
    device: VkDevice,
    image: VkImage,
    p_requirements: *mut VkMemoryRequirements,
);

/// `PFN_vkBindImageMemory`.
pub type PfnVkBindImageMemory = unsafe extern "system" fn(
    device: VkDevice,
    image: VkImage,
    memory: VkDeviceMemory,
    memory_offset: VkDeviceSize,
) -> i32;

/// `PFN_vkCmdCopyImageToBuffer` — the S0 offscreen-image → host-visible-staging
/// readback transfer.
pub type PfnVkCmdCopyImageToBuffer = unsafe extern "system" fn(
    command_buffer: VkCommandBuffer,
    src_image: VkImage,
    // `src_image_layout`: `VkImageLayout` the source image is in (`TRANSFER_SRC_OPTIMAL`).
    src_image_layout: i32,
    dst_buffer: VkBuffer,
    region_count: u32,
    p_regions: *const VkBufferImageCopy,
);

/// `PFN_vkCmdCopyBufferToImage` — the rung-11 buffer → SAMPLED-image upload (the
/// symmetric counterpart of [`PfnVkCmdCopyImageToBuffer`]): copies the compute
/// composite's packed-RGBA pixel region into an `R8G8B8A8_UNORM` texture so a
/// fullscreen-sample pass can present it to the swapchain.
pub type PfnVkCmdCopyBufferToImage = unsafe extern "system" fn(
    command_buffer: VkCommandBuffer,
    src_buffer: VkBuffer,
    dst_image: VkImage,
    // `dst_image_layout`: `VkImageLayout` the destination image is in (`TRANSFER_DST_OPTIMAL`).
    dst_image_layout: i32,
    region_count: u32,
    p_regions: *const VkBufferImageCopy,
);

/// `PFN_vkCmdBlitImage` — the textured-PBR T2 mip-chain-generation blit (Decision
/// D3): a LINEAR-filtered, format-converting copy between two mip levels of an
/// image (here always the SAME image for both `src_image`/`dst_image`).
pub type PfnVkCmdBlitImage = unsafe extern "system" fn(
    command_buffer: VkCommandBuffer,
    src_image: VkImage,
    // `src_image_layout`: `VkImageLayout` the source image is in (`TRANSFER_SRC_OPTIMAL`).
    src_image_layout: i32,
    dst_image: VkImage,
    // `dst_image_layout`: `VkImageLayout` the destination image is in (`TRANSFER_DST_OPTIMAL`).
    dst_image_layout: i32,
    region_count: u32,
    p_regions: *const VkImageBlit,
    // `filter`: `VkFilter` (`VK_FILTER_LINEAR` for mip-chain downsampling).
    filter: i32,
);

/// `PFN_vkGetPhysicalDeviceFeatures2` — the S0 fail-fast `dynamicRendering`
/// support query (Vulkan 1.1 core; the `2` suffix, no `KHR`).
pub type PfnVkGetPhysicalDeviceFeatures2 = unsafe extern "system" fn(
    physical_device: VkPhysicalDevice,
    p_features: *mut VkPhysicalDeviceFeatures2,
);

/// `PFN_vkGetPhysicalDeviceFormatProperties` — the Render P1b device-caps query for
/// the G-buffer storage-image format support (Vulkan 1.0 core, always present).
pub type PfnVkGetPhysicalDeviceFormatProperties = unsafe extern "system" fn(
    physical_device: VkPhysicalDevice,
    // `format`: `VkFormat` (an `i32`).
    format: i32,
    p_format_properties: *mut VkFormatProperties,
);
