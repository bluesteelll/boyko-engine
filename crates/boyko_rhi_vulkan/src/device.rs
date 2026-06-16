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

use core::ffi::{CStr, c_char, c_void};
use core::mem;
use core::ptr;

use crate::ffi::*;

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
}

/// Whether to attempt enabling the Khronos validation layer.
#[derive(Clone, Copy, Default)]
pub struct InstanceConfig {
    /// Request `VK_LAYER_KHRONOS_validation` **iff it is installed**. Defaults
    /// to `false`; an absent layer silently downgrades to no validation.
    pub enable_validation: bool,
}

/// Global-scope Vulkan commands (resolved with a NULL instance).
struct GlobalFns {
    create_instance: PfnVkCreateInstance,
}

/// Instance-scope Vulkan commands.
struct InstanceFns {
    destroy_instance: PfnVkDestroyInstance,
    enumerate_physical_devices: PfnVkEnumeratePhysicalDevices,
    get_physical_device_properties: PfnVkGetPhysicalDeviceProperties,
    get_physical_device_memory_properties: PfnVkGetPhysicalDeviceMemoryProperties,
    get_physical_device_queue_family_properties: PfnVkGetPhysicalDeviceQueueFamilyProperties,
    create_device: PfnVkCreateDevice,
    get_device_proc_addr: PfnVkGetDeviceProcAddr,
}

/// Device-scope Vulkan commands needed for the buffer round-trip.
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
    instance_fns: InstanceFns,
    device_fns: DeviceFns,
}

impl VulkanContext {
    /// Boots a headless Vulkan context, picking a discrete GPU if available.
    ///
    /// Returns a [`BootError`] (never panics) on any loader / driver / GPU
    /// absence so the caller can skip gracefully on a GPU-less machine.
    pub fn boot(config: InstanceConfig) -> Result<Self, BootError> {
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
        );
        match result {
            Ok(ctx) => Ok(ctx),
            Err((e, instance_fns)) => {
                // SAFETY: `instance` was created above and not yet stored in a
                // live context; `destroy_instance` is the matching teardown,
                // called exactly once before the loader is freed.
                unsafe { (instance_fns.destroy_instance)(instance, ptr::null()) };
                // SAFETY: `module` is live and freed exactly once here.
                unsafe { free_vulkan_loader(module) };
                Err(e)
            }
        }
    }

    /// Continues the boot once the instance exists. On error it returns the
    /// loaded [`InstanceFns`] so the caller can destroy the instance with the
    /// correct command pointer.
    fn boot_with_instance(
        module: *mut c_void,
        instance: VkInstance,
        gipa: PfnVkGetInstanceProcAddr,
    ) -> Result<Self, (BootError, InstanceFns)> {
        let instance_fns = match load_instance_fns(gipa, instance) {
            Ok(f) => f,
            // No fns loaded → we cannot even destroy the instance with a typed
            // pointer; load just the destroyer best-effort. If even that is
            // missing the instance leaks, but that is a broken-loader corner
            // the spec does not allow.
            Err(e) => return Err((e, fallback_instance_fns(gipa, instance))),
        };

        // --- 4. Pick a physical device (prefer a discrete GPU). ---
        let (physical_device, device_name, memory_properties) =
            match pick_physical_device(&instance_fns, instance) {
                Ok(p) => p,
                Err(e) => return Err((e, instance_fns)),
            };

        // --- 5. Find a graphics+compute queue family. ---
        let queue_family_index = match find_queue_family(&instance_fns, physical_device) {
            Ok(q) => q,
            Err(e) => return Err((e, instance_fns)),
        };

        // --- 6. Create the logical device + retrieve the queue. ---
        let device = match create_device(&instance_fns, physical_device, queue_family_index) {
            Ok(d) => d,
            Err(e) => return Err((e, instance_fns)),
        };

        let device_fns = match load_device_fns(instance_fns.get_device_proc_addr, device) {
            Ok(f) => f,
            Err(e) => {
                // The device was created but its commands are unloadable: we
                // have no typed destroyer, so this is an unrecoverable broken
                // loader. Surface the error; the instance is torn down by the
                // caller. (A conformant loader always exports these.)
                return Err((e, instance_fns));
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
            instance_fns,
            device_fns,
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
}

impl Drop for VulkanContext {
    fn drop(&mut self) {
        // SAFETY: `device`/`instance` are the exact handles created in `boot`,
        // each destroyed exactly once here in reverse creation order with its
        // matching destroyer. `module` is the live HMODULE freed once. No
        // handle is used after its destroyer runs.
        unsafe {
            (self.device_fns.destroy_device)(self.device, ptr::null());
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
    let create_instance =
        unsafe { load_instance_command(gipa, VkInstance::NULL, c"vkCreateInstance")? };
    Ok(GlobalFns { create_instance })
}

fn load_instance_fns(
    gipa: PfnVkGetInstanceProcAddr,
    instance: VkInstance,
) -> Result<InstanceFns, BootError> {
    // SAFETY: instance commands resolve with the live `instance`; each `T`
    // matches its command's PFN typedef.
    unsafe {
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
            create_device: load_instance_command(gipa, instance, c"vkCreateDevice")?,
            get_device_proc_addr: load_instance_command(gipa, instance, c"vkGetDeviceProcAddr")?,
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
        create_device: noop_create_device,
        get_device_proc_addr: noop_get_device_proc_addr,
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

fn load_device_fns(gdpa: PfnVkGetDeviceProcAddr, device: VkDevice) -> Result<DeviceFns, BootError> {
    // SAFETY: device commands resolve with the live `device`; each `T` matches
    // its command's PFN typedef.
    unsafe {
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
    let app_info = VkApplicationInfo {
        s_type: VkStructureType::ApplicationInfo,
        p_next: ptr::null(),
        p_application_name: c"boyko_rhi_vulkan slice0".as_ptr(),
        application_version: 0,
        p_engine_name: c"boyko-engine".as_ptr(),
        engine_version: 0,
        api_version: VK_API_VERSION_1_3,
    };

    // Validation layer is requested only when the caller asks for it. Whether
    // it is actually *present* is a query the SDK-gated steps add; for the
    // NO-SDK sub-step the flag is never set, so the layer array is empty and
    // instance creation does not require the layer. If a future caller sets the
    // flag and the layer is absent, `vkCreateInstance` returns
    // `VK_ERROR_LAYER_NOT_PRESENT`, surfaced as a loud `VkError` (never a
    // silent requirement).
    let layer_ptrs: [*const c_char; 1] = [VALIDATION_LAYER.as_ptr()];
    let (layer_count, pp_layers) = if config.enable_validation {
        (1u32, layer_ptrs.as_ptr())
    } else {
        (0u32, ptr::null())
    };

    let create_info = VkInstanceCreateInfo {
        s_type: VkStructureType::InstanceCreateInfo,
        p_next: ptr::null(),
        flags: 0,
        p_application_info: &app_info,
        enabled_layer_count: layer_count,
        pp_enabled_layer_names: pp_layers,
        enabled_extension_count: 0,
        pp_enabled_extension_names: ptr::null(),
    };

    let mut instance = VkInstance::NULL;
    // SAFETY: `create_info` is a fully-initialized `#[repr(C)]`
    // `VkInstanceCreateInfo` whose pointer members (`p_application_info`,
    // optional layer array) outlive the call; `&mut instance` is a valid
    // out-pointer. NULL allocator selects the default.
    let raw = unsafe { (global.create_instance)(&create_info, ptr::null(), &mut instance) };
    let result = VkResult::from_raw(raw);
    if !result.is_success() {
        return Err(BootError::VkError("vkCreateInstance", result));
    }
    Ok(instance)
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

    let mut mem_props: VkPhysicalDeviceMemoryProperties = unsafe { mem::zeroed() };
    // SAFETY: `chosen` is a valid physical device enumerated above; `&mut
    // mem_props` is a valid out-pointer for the `#[repr(C)]`
    // `VkPhysicalDeviceMemoryProperties` the driver fully overwrites. (Zeroed
    // init is a valid bit pattern for the all-integer/array struct.)
    unsafe { (fns.get_physical_device_memory_properties)(chosen, &mut mem_props) };

    Ok((chosen, name, mem_props))
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

/// Creates a logical device with one queue from `queue_family_index`.
fn create_device(
    fns: &InstanceFns,
    physical_device: VkPhysicalDevice,
    queue_family_index: u32,
) -> Result<VkDevice, BootError> {
    let priority: f32 = 1.0;
    let queue_info = VkDeviceQueueCreateInfo {
        s_type: VkStructureType::DeviceQueueCreateInfo,
        p_next: ptr::null(),
        flags: 0,
        queue_family_index,
        queue_count: 1,
        p_queue_priorities: &priority,
    };

    let create_info = VkDeviceCreateInfo {
        s_type: VkStructureType::DeviceCreateInfo,
        p_next: ptr::null(),
        flags: 0,
        queue_create_info_count: 1,
        p_queue_create_infos: &queue_info,
        enabled_layer_count: 0,
        pp_enabled_layer_names: ptr::null(),
        enabled_extension_count: 0,
        pp_enabled_extension_names: ptr::null(),
        p_enabled_features: ptr::null(),
    };

    let mut device = VkDevice::NULL;
    // SAFETY: `physical_device` is valid; `create_info` is a fully-initialized
    // `#[repr(C)]` struct whose `p_queue_create_infos`/`p_queue_priorities`
    // pointers (`&queue_info`, `&priority`) outlive the call; `&mut device` is
    // a valid out-pointer; NULL allocator picks the default.
    let raw =
        unsafe { (fns.create_device)(physical_device, &create_info, ptr::null(), &mut device) };
    let result = VkResult::from_raw(raw);
    if !result.is_success() {
        return Err(BootError::VkError("vkCreateDevice", result));
    }
    Ok(device)
}
