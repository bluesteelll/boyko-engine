//! **Rung P1b deliverable 2 — the register / occupancy figure for all three `particle_sim`
//! modules** (`docs/PARTICLES-PLAN.md` P1b item 2).
//!
//! Reports, per module, what the DRIVER says it compiled: register count, spill counts, occupancy
//! and whatever else this vendor's `VK_KHR_pipeline_executable_properties` implementation exposes.
//! It exists to test gate #17 §A1's standing hypothesis — *a larger, higher-register kernel raises
//! achieved bandwidth on an over-subscribed bandwidth-bound sim* — which gate #17 could state but
//! not measure ("no occupancy or register figure was taken — this instrument cannot produce one on
//! this host").
//!
//! # Why this probe creates its OWN instance and device
//!
//! `VK_KHR_pipeline_executable_properties` must be enabled at `vkCreateDevice`, and the engine's
//! device does not enable it — nor should it: the extension exists to let a driver hand back
//! compilation statistics, and `VK_PIPELINE_CREATE_CAPTURE_STATISTICS_BIT_KHR` asks the driver to
//! keep information it would otherwise discard. Paying that on every shipping boot to serve a
//! measurement nobody takes at runtime is the F24 dark tax in another costume.
//!
//! So this file stands up a MINIMAL headless Vulkan: instance, physical-device pick, device with the
//! one extension, one descriptor-set layout, one pipeline layout, three compute pipelines. No
//! surface, no swapchain, no queue submission, nothing rendered. The three `.spv` are the COMMITTED
//! artifacts, read through the same `embed_spirv!` accessors the engine binds, so the modules
//! measured here are byte-for-byte the modules that run.
//!
//! # What it asserts, and what it only REPORTS
//!
//! It asserts the mechanical facts — the extension resolved, three pipelines compiled, at least one
//! executable per pipeline, and that the statistic NAMES agree across the three modules (a driver
//! that reported different statistic sets per module would make the columns incomparable). It does
//! NOT assert a register count: that is a property of this driver version on this part, and pinning
//! one would red on a driver update while saying nothing about the engine.
//!
//! # Usage
//!
//! ```text
//! cargo test -p boyko_rhi_vulkan --test particle_sim_occupancy -- --ignored --nocapture
//! ```
//!
//! `#[ignore]`: needs a real GPU. SKIPS (loudly) when the loader, a suitable device or the
//! extension is absent — and says WHICH, because "no numbers" and "no device" are different
//! findings and a silent skip would make them the same one.
//!
//! # Why `#![cfg(windows)]`, said rather than left to be inferred
//!
//! ONE reason, and it is not the GPU: the loader is opened through `boyko_rhi_vulkan::ffi::os`'s
//! `LoadLibraryA`/`GetProcAddress`, which are Win32. Everything else here — the structs, the
//! statistics decode, the pick — is platform-neutral, so porting is a `dlopen`/`libvulkan.so.1`
//! arm and nothing more.
//!
//! ⚠️ The cost of that gate is stated because this module distinguishes skips from absences
//! everywhere else: on a non-Windows host this binary compiles to NOTHING and reports
//! `0 tests`, which reads identically to "ran and skipped". The `eprintln!` skips inside the test
//! are the ones that can be told apart; the `cfg` cannot. It is accepted here (the target platform
//! is Windows/Linux x86_64 and the GPU legs are Windows-only throughout this tree) and recorded so
//! a reader of a Linux CI log does not count this as a green run.

#![cfg(windows)]

use std::ffi::{CStr, c_char, c_void};
use std::ptr;

use boyko_rhi_vulkan::compute::{
    particle_sim_sdf_spirv, particle_sim_spirv, particle_sim_stats_spirv,
};
use boyko_rhi_vulkan::ffi::{
    VkExtensionProperties, VkPhysicalDeviceProperties, VkQueueFamilyProperties, os,
};

// ---- The ABI this probe needs, declared LOCALLY -----------------------------------------------
//
// Deliberately not added to `boyko_rhi_vulkan::ffi`: none of it is reachable from the engine's own
// device (which does not enable the extension), and a shared FFI module that carries structs no
// shipping path can use is a surface that rots unnoticed. Every `s_type` below is the literal from
// `vulkan_core.h`, named in the comment beside it so a reader can check the number without the
// header.

/// `VK_STRUCTURE_TYPE_APPLICATION_INFO`.
const ST_APPLICATION_INFO: i32 = 0;
/// `VK_STRUCTURE_TYPE_INSTANCE_CREATE_INFO`.
const ST_INSTANCE_CREATE_INFO: i32 = 1;
/// `VK_STRUCTURE_TYPE_DEVICE_QUEUE_CREATE_INFO`.
const ST_DEVICE_QUEUE_CREATE_INFO: i32 = 2;
/// `VK_STRUCTURE_TYPE_DEVICE_CREATE_INFO`.
const ST_DEVICE_CREATE_INFO: i32 = 3;
/// `VK_STRUCTURE_TYPE_SHADER_MODULE_CREATE_INFO`.
const ST_SHADER_MODULE_CREATE_INFO: i32 = 16;
/// `VK_STRUCTURE_TYPE_PIPELINE_SHADER_STAGE_CREATE_INFO`.
const ST_PIPELINE_SHADER_STAGE_CREATE_INFO: i32 = 18;
/// `VK_STRUCTURE_TYPE_COMPUTE_PIPELINE_CREATE_INFO`.
const ST_COMPUTE_PIPELINE_CREATE_INFO: i32 = 29;
/// `VK_STRUCTURE_TYPE_PIPELINE_LAYOUT_CREATE_INFO`.
const ST_PIPELINE_LAYOUT_CREATE_INFO: i32 = 30;
/// `VK_STRUCTURE_TYPE_DESCRIPTOR_SET_LAYOUT_CREATE_INFO`.
const ST_DESCRIPTOR_SET_LAYOUT_CREATE_INFO: i32 = 32;
/// `VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_PIPELINE_EXECUTABLE_PROPERTIES_FEATURES_KHR`.
const ST_PIPELINE_EXECUTABLE_FEATURES_KHR: i32 = 1_000_269_000;
/// `VK_STRUCTURE_TYPE_PIPELINE_INFO_KHR`.
const ST_PIPELINE_INFO_KHR: i32 = 1_000_269_001;
/// `VK_STRUCTURE_TYPE_PIPELINE_EXECUTABLE_PROPERTIES_KHR`.
const ST_PIPELINE_EXECUTABLE_PROPERTIES_KHR: i32 = 1_000_269_002;
/// `VK_STRUCTURE_TYPE_PIPELINE_EXECUTABLE_INFO_KHR`.
const ST_PIPELINE_EXECUTABLE_INFO_KHR: i32 = 1_000_269_003;
/// `VK_STRUCTURE_TYPE_PIPELINE_EXECUTABLE_STATISTIC_KHR`.
const ST_PIPELINE_EXECUTABLE_STATISTIC_KHR: i32 = 1_000_269_004;

/// `VK_PIPELINE_CREATE_CAPTURE_STATISTICS_BIT_KHR` — asks the driver to KEEP the compilation
/// statistics it would otherwise discard. Without it the statistic query returns nothing.
const VK_PIPELINE_CREATE_CAPTURE_STATISTICS_BIT_KHR: u32 = 0x0000_0040;

/// `VK_MAX_DESCRIPTION_SIZE` — the fixed char array every KHR name/description field uses.
const VK_MAX_DESCRIPTION_SIZE: usize = 256;

/// `VK_SHADER_STAGE_COMPUTE_BIT`.
const VK_SHADER_STAGE_COMPUTE_BIT: u32 = 0x0000_0020;
/// `VK_DESCRIPTOR_TYPE_STORAGE_BUFFER`.
const VK_DESCRIPTOR_TYPE_STORAGE_BUFFER: i32 = 7;
/// `VK_QUEUE_COMPUTE_BIT`.
const VK_QUEUE_COMPUTE_BIT: u32 = 0x0000_0002;
/// `VK_PHYSICAL_DEVICE_TYPE_DISCRETE_GPU`.
const VK_PHYSICAL_DEVICE_TYPE_DISCRETE_GPU: i32 = 2;
/// `VK_API_VERSION_1_3`.
const VK_API_VERSION_1_3: u32 = (1 << 22) | (3 << 12);

/// The Set-0 binding count the particle compute vocabulary spans (0..=10 — see any
/// `particle_*.comp.hlsl` header's `# Set / binding vocabulary` block).
const PARTICLE_BINDING_COUNT: u32 = 11;
/// The sim's `COMPUTE` push range, in bytes (`uint steps` + `float timestep` + `uint capacity`).
///
/// A LOCAL mirror of `boyko_rhi_vulkan::compute::PARTICLE_SIM_PUSH_BYTES` — this probe stands up
/// its own instance/device/pipelines and deliberately links nothing of the engine's boot path, so
/// the value is re-declared here. It must not be SMALLER than the range the committed module
/// declares, or every pipeline create below fails with a range/shader mismatch.
const PARTICLE_SIM_PUSH_BYTES: u32 = 12;

type VkResult = i32;
/// A dispatchable handle (`VkInstance`, `VkPhysicalDevice`, `VkDevice`) — a pointer.
type VkHandle = *mut c_void;
/// A non-dispatchable handle (`VkPipeline`, `VkShaderModule`, …) — 64 bits on every platform.
type VkNonDispatchable = u64;

#[repr(C)]
struct VkApplicationInfo {
    s_type: i32,
    p_next: *const c_void,
    p_application_name: *const c_char,
    application_version: u32,
    p_engine_name: *const c_char,
    engine_version: u32,
    api_version: u32,
}

#[repr(C)]
struct VkInstanceCreateInfo {
    s_type: i32,
    p_next: *const c_void,
    flags: u32,
    p_application_info: *const VkApplicationInfo,
    enabled_layer_count: u32,
    pp_enabled_layer_names: *const *const c_char,
    enabled_extension_count: u32,
    pp_enabled_extension_names: *const *const c_char,
}

#[repr(C)]
struct VkDeviceQueueCreateInfo {
    s_type: i32,
    p_next: *const c_void,
    flags: u32,
    queue_family_index: u32,
    queue_count: u32,
    p_queue_priorities: *const f32,
}

#[repr(C)]
struct VkDeviceCreateInfo {
    s_type: i32,
    p_next: *const c_void,
    flags: u32,
    queue_create_info_count: u32,
    p_queue_create_infos: *const VkDeviceQueueCreateInfo,
    enabled_layer_count: u32,
    pp_enabled_layer_names: *const *const c_char,
    enabled_extension_count: u32,
    pp_enabled_extension_names: *const *const c_char,
    p_enabled_features: *const c_void,
}

#[repr(C)]
struct VkPipelineExecutableFeaturesKhr {
    s_type: i32,
    p_next: *mut c_void,
    pipeline_executable_info: u32,
}

// The three DRIVER-WRITTEN enumeration structs are the crate's own
// (`boyko_rhi_vulkan::ffi`), imported rather than re-declared here. That is not tidiness: a
// hand-rolled `VkPhysicalDeviceProperties` whose `limits` blob is `[u8; 504]` at align 1 collapses
// the 4 bytes of padding after `pipelineCacheUUID` AND the 4 of tail padding, giving an
// 816-byte/align-4 struct that `vkGetPhysicalDeviceProperties` overruns by 8 bytes on EVERY
// enumerated device — benign only by stack-layout luck, which is precisely why such a probe reports
// plausible numbers while smashing its own frame. The crate's type carries
// `VkPhysicalDeviceLimitsBlob`'s `#[repr(C, align(8))]` and a `const _: () = assert!(size == 824)`
// beside it. Structs this probe WRITES and the driver only reads are still declared locally (their
// `sType` tags are KHR ones the crate's `VkStructureType` does not name), and each carries its own
// size guard below.

#[repr(C)]
struct VkDescriptorSetLayoutBinding {
    binding: u32,
    descriptor_type: i32,
    descriptor_count: u32,
    stage_flags: u32,
    p_immutable_samplers: *const c_void,
}

#[repr(C)]
struct VkDescriptorSetLayoutCreateInfo {
    s_type: i32,
    p_next: *const c_void,
    flags: u32,
    binding_count: u32,
    p_bindings: *const VkDescriptorSetLayoutBinding,
}

#[repr(C)]
struct VkPushConstantRange {
    stage_flags: u32,
    offset: u32,
    size: u32,
}

#[repr(C)]
struct VkPipelineLayoutCreateInfo {
    s_type: i32,
    p_next: *const c_void,
    flags: u32,
    set_layout_count: u32,
    p_set_layouts: *const VkNonDispatchable,
    push_constant_range_count: u32,
    p_push_constant_ranges: *const VkPushConstantRange,
}

#[repr(C)]
struct VkShaderModuleCreateInfo {
    s_type: i32,
    p_next: *const c_void,
    flags: u32,
    code_size: usize,
    p_code: *const u32,
}

#[repr(C)]
struct VkPipelineShaderStageCreateInfo {
    s_type: i32,
    p_next: *const c_void,
    flags: u32,
    stage: u32,
    module: VkNonDispatchable,
    p_name: *const c_char,
    p_specialization_info: *const c_void,
}

#[repr(C)]
struct VkComputePipelineCreateInfo {
    s_type: i32,
    p_next: *const c_void,
    flags: u32,
    stage: VkPipelineShaderStageCreateInfo,
    layout: VkNonDispatchable,
    base_pipeline_handle: VkNonDispatchable,
    base_pipeline_index: i32,
}

#[repr(C)]
struct VkPipelineInfoKhr {
    s_type: i32,
    p_next: *const c_void,
    pipeline: VkNonDispatchable,
}

#[repr(C)]
struct VkPipelineExecutablePropertiesKhr {
    s_type: i32,
    p_next: *mut c_void,
    stages: u32,
    name: [c_char; VK_MAX_DESCRIPTION_SIZE],
    description: [c_char; VK_MAX_DESCRIPTION_SIZE],
    subgroup_size: u32,
}

#[repr(C)]
struct VkPipelineExecutableInfoKhr {
    s_type: i32,
    p_next: *const c_void,
    pipeline: VkNonDispatchable,
    executable_index: u32,
}

/// `VkPipelineExecutableStatisticKHR`. The trailing `value` is a union of `VkBool32`/`i64`/`u64`/
/// `f64` — 8 bytes, 8-aligned — carried as raw bits and decoded by `format` in
/// [`StatValue::decode`], which is the only place the discriminant is interpreted.
#[repr(C)]
struct VkPipelineExecutableStatisticKhr {
    s_type: i32,
    p_next: *mut c_void,
    name: [c_char; VK_MAX_DESCRIPTION_SIZE],
    description: [c_char; VK_MAX_DESCRIPTION_SIZE],
    format: i32,
    value: u64,
}

// Layout guards on the two KHR structs the DRIVER writes through an out-pointer. Same class as the
// crate's own guards, and the reason C1 existed: a struct the driver fills must equal the C ABI or
// the write lands past the Rust local, and the numbers LOOK fine either way.
//
// `VkPipelineExecutablePropertiesKHR`, field by field on the 64-bit ABI:
//   sType 4 @0 | pad 4 | pNext 8 @8 | stages 4 @16 | name[256] @20 | description[256] @276 |
//   subgroupSize 4 @532  =>  536, align 8 (536 is already a multiple of 8, so NO tail pad).
// `VkPipelineExecutableStatisticKHR`:
//   sType 4 @0 | pad 4 | pNext 8 @8 | name[256] @16 | description[256] @272 | format 4 @528 |
//   pad 4 | value (union of u32/i64/u64/f64, so 8-aligned) 8 @536  =>  544, align 8.
//
// These two asserts EARNED THEMSELVES ON THE FIRST COMPILE: the properties struct was written 544
// here by hand and the guard rejected it. That is the C1 defect's own shape — an ABI size derived
// by eye rather than by field — caught before it could reach a driver.
const _: () = assert!(size_of::<VkPipelineExecutablePropertiesKhr>() == 536);
const _: () = assert!(align_of::<VkPipelineExecutablePropertiesKhr>() == 8);
const _: () = assert!(size_of::<VkPipelineExecutableStatisticKhr>() == 544);
const _: () = assert!(align_of::<VkPipelineExecutableStatisticKhr>() == 8);

type PfnVoidFunction = Option<unsafe extern "system" fn()>;
type PfnGetInstanceProcAddr =
    unsafe extern "system" fn(instance: VkHandle, p_name: *const c_char) -> PfnVoidFunction;
type PfnGetDeviceProcAddr =
    unsafe extern "system" fn(device: VkHandle, p_name: *const c_char) -> PfnVoidFunction;
type PfnCreateInstance = unsafe extern "system" fn(
    *const VkInstanceCreateInfo,
    *const c_void,
    *mut VkHandle,
) -> VkResult;
type PfnDestroyInstance = unsafe extern "system" fn(VkHandle, *const c_void);
type PfnEnumeratePhysicalDevices =
    unsafe extern "system" fn(VkHandle, *mut u32, *mut VkHandle) -> VkResult;
type PfnGetPhysicalDeviceProperties =
    unsafe extern "system" fn(VkHandle, *mut VkPhysicalDeviceProperties);
type PfnGetPhysicalDeviceQueueFamilyProperties =
    unsafe extern "system" fn(VkHandle, *mut u32, *mut VkQueueFamilyProperties);
type PfnEnumerateDeviceExtensionProperties = unsafe extern "system" fn(
    VkHandle,
    *const c_char,
    *mut u32,
    *mut VkExtensionProperties,
) -> VkResult;
type PfnCreateDevice = unsafe extern "system" fn(
    VkHandle,
    *const VkDeviceCreateInfo,
    *const c_void,
    *mut VkHandle,
) -> VkResult;
type PfnDestroyDevice = unsafe extern "system" fn(VkHandle, *const c_void);
type PfnCreateShaderModule = unsafe extern "system" fn(
    VkHandle,
    *const VkShaderModuleCreateInfo,
    *const c_void,
    *mut VkNonDispatchable,
) -> VkResult;
type PfnDestroyShaderModule =
    unsafe extern "system" fn(VkHandle, VkNonDispatchable, *const c_void);
type PfnCreateDescriptorSetLayout = unsafe extern "system" fn(
    VkHandle,
    *const VkDescriptorSetLayoutCreateInfo,
    *const c_void,
    *mut VkNonDispatchable,
) -> VkResult;
type PfnDestroyDescriptorSetLayout =
    unsafe extern "system" fn(VkHandle, VkNonDispatchable, *const c_void);
type PfnCreatePipelineLayout = unsafe extern "system" fn(
    VkHandle,
    *const VkPipelineLayoutCreateInfo,
    *const c_void,
    *mut VkNonDispatchable,
) -> VkResult;
type PfnDestroyPipelineLayout =
    unsafe extern "system" fn(VkHandle, VkNonDispatchable, *const c_void);
type PfnCreateComputePipelines = unsafe extern "system" fn(
    VkHandle,
    VkNonDispatchable,
    u32,
    *const VkComputePipelineCreateInfo,
    *const c_void,
    *mut VkNonDispatchable,
) -> VkResult;
type PfnDestroyPipeline = unsafe extern "system" fn(VkHandle, VkNonDispatchable, *const c_void);
type PfnGetPipelineExecutableProperties = unsafe extern "system" fn(
    VkHandle,
    *const VkPipelineInfoKhr,
    *mut u32,
    *mut VkPipelineExecutablePropertiesKhr,
) -> VkResult;
type PfnGetPipelineExecutableStatistics = unsafe extern "system" fn(
    VkHandle,
    *const VkPipelineExecutableInfoKhr,
    *mut u32,
    *mut VkPipelineExecutableStatisticKhr,
) -> VkResult;

// ---- Helpers ----------------------------------------------------------------------------------

/// One decoded statistic value, in the four shapes the KHR union can hold.
#[derive(Debug, Clone, Copy, PartialEq)]
enum StatValue {
    Bool(bool),
    I64(i64),
    U64(u64),
    F64(f64),
}

impl StatValue {
    /// Decodes the union's raw bits under its `format` discriminant.
    ///
    /// The four format values are `VkPipelineExecutableStatisticFormatKHR`'s, in declaration order;
    /// an unknown one is carried as raw `U64` rather than guessed, so a driver reporting a format
    /// this probe predates prints a number a reader can still recognize.
    fn decode(format: i32, bits: u64) -> Self {
        match format {
            0 => StatValue::Bool(bits as u32 != 0),
            1 => StatValue::I64(bits as i64),
            2 => StatValue::U64(bits),
            3 => StatValue::F64(f64::from_bits(bits)),
            _ => StatValue::U64(bits),
        }
    }
}

impl std::fmt::Display for StatValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StatValue::Bool(b) => write!(f, "{b}"),
            StatValue::I64(v) => write!(f, "{v}"),
            StatValue::U64(v) => write!(f, "{v}"),
            StatValue::F64(v) => write!(f, "{v}"),
        }
    }
}

/// Reads a NUL-terminated fixed char array into an owned `String`.
///
/// # Safety
///
/// The caller guarantees `buf` is a fixed array the driver filled with a NUL-terminated ASCII
/// string, which is what every `VK_MAX_DESCRIPTION_SIZE` field's contract states.
unsafe fn c_array_to_string(buf: &[c_char]) -> String {
    // SAFETY: `buf` is a driver-filled `VK_MAX_DESCRIPTION_SIZE` array, NUL-terminated by the
    // Vulkan spec's own contract for these fields; `CStr::from_ptr` therefore stops inside it.
    let cstr = unsafe { CStr::from_ptr(buf.as_ptr()) };
    cstr.to_string_lossy().into_owned()
}

/// Resolves an instance-level entry point, or panics naming the symbol.
///
/// # Safety
///
/// The caller guarantees `gipa` is the loader's real `vkGetInstanceProcAddr` and that `T` is the
/// function-pointer type declared for `name` in `vulkan_core.h` — a mismatch is a call through the
/// wrong ABI.
unsafe fn load_instance_fn<T: Copy>(
    gipa: PfnGetInstanceProcAddr,
    instance: VkHandle,
    name: &CStr,
) -> T {
    // SAFETY: `gipa` is the loader's entry point; `name` is NUL-terminated by `CStr`. A null return
    // is checked below before the transmute, so only a real address is ever reinterpreted.
    let raw = unsafe { gipa(instance, name.as_ptr()) };
    let raw = raw.unwrap_or_else(|| panic!("invariant: {name:?} must resolve"));
    const {
        assert!(size_of::<T>() == size_of::<PfnVoidFunction>());
    }
    // SAFETY: `raw` is a live function pointer the loader returned for `name`, and `T` is that
    // entry point's declared signature (the caller's contract). Both are pointer-sized, asserted
    // above.
    unsafe { std::mem::transmute_copy::<PfnVoidFunction, T>(&Some(raw)) }
}

/// Resolves a device-level entry point, or returns `None` when the driver does not expose it.
///
/// # Safety
///
/// Same contract as [`load_instance_fn`], for `vkGetDeviceProcAddr`.
unsafe fn load_device_fn<T: Copy>(
    gdpa: PfnGetDeviceProcAddr,
    device: VkHandle,
    name: &CStr,
) -> Option<T> {
    // SAFETY: `gdpa` is the device's proc-addr entry point and `name` is NUL-terminated.
    let raw = unsafe { gdpa(device, name.as_ptr()) }?;
    const {
        assert!(size_of::<T>() == size_of::<PfnVoidFunction>());
    }
    // SAFETY: as `load_instance_fn` — a live function pointer reinterpreted at its declared
    // signature, both pointer-sized.
    Some(unsafe { std::mem::transmute_copy::<PfnVoidFunction, T>(&Some(raw)) })
}

/// One module's report: its label, its `.spv`, and the statistics the driver returned.
struct ModuleReport {
    label: &'static str,
    spirv_words: usize,
    subgroup_size: u32,
    executable_name: String,
    stats: Vec<(String, StatValue)>,
}

/// **The occupancy figure for the three sim modules.** See the module doc.
#[test]
#[ignore = "needs a real GPU exposing VK_KHR_pipeline_executable_properties; rung P1b deliverable 2"]
fn particle_sim_modules_report_their_register_footprint() {
    let Some(probe) = VulkanProbe::open() else {
        return;
    };
    let reports = probe.measure([
        ("base            ", particle_sim_spirv()),
        ("sdf             ", particle_sim_sdf_spirv()),
        ("sdf_stats       ", particle_sim_stats_spirv()),
    ]);

    println!("\n=== particle_sim occupancy / register report ===");
    println!("device: {}", probe.device_name);
    for r in &reports {
        println!(
            "\n[{}] {} SPIR-V words, executable \"{}\", subgroupSize {}",
            r.label.trim(),
            r.spirv_words,
            r.executable_name,
            r.subgroup_size
        );
        for (name, value) in &r.stats {
            println!("    {name:.<44} {value}");
        }
    }

    // The mechanical claims. None of them is a number this driver chose — they are the conditions
    // under which the numbers above mean anything.
    assert_eq!(reports.len(), 3, "all three sim modules must have compiled");
    for r in &reports {
        assert!(
            !r.stats.is_empty(),
            "[{}] the driver returned no statistics — the CAPTURE_STATISTICS bit did not take \
             effect, so the columns above would be empty rather than equal",
            r.label.trim()
        );
    }

    // The statistic SETS must agree across the three modules, or the columns are not comparable and
    // any "the stats module uses N more registers" claim would be comparing two different tables.
    let names_of = |r: &ModuleReport| -> Vec<String> {
        let mut n: Vec<String> = r.stats.iter().map(|(k, _)| k.clone()).collect();
        n.sort();
        n
    };
    let base_names = names_of(&reports[0]);
    for r in &reports[1..] {
        assert_eq!(
            names_of(r),
            base_names,
            "[{}] reports a different statistic SET than the base module — the three columns are \
             not comparable",
            r.label.trim()
        );
    }

    // Non-vacuity: the three modules are genuinely different code. If the driver handed back
    // identical statistics for all three, either the pipelines were built from one `.spv` or the
    // statistics are not a function of the shader — and every conclusion below would be empty.
    let all_identical = reports[1].stats == reports[0].stats && reports[2].stats == reports[0].stats;
    assert!(
        !all_identical,
        "all three modules reported IDENTICAL statistics. They differ by ~27 KB of SPIR-V and by \
         three atomic sites, so identical numbers mean the statistics are not reading the shader."
    );
}

/// The minimal headless Vulkan this probe stands up, plus the entry points it resolved.
struct VulkanProbe {
    module: *mut c_void,
    instance: VkHandle,
    device: VkHandle,
    device_name: String,
    destroy_instance: PfnDestroyInstance,
    destroy_device: PfnDestroyDevice,
    create_shader_module: PfnCreateShaderModule,
    destroy_shader_module: PfnDestroyShaderModule,
    create_descriptor_set_layout: PfnCreateDescriptorSetLayout,
    destroy_descriptor_set_layout: PfnDestroyDescriptorSetLayout,
    create_pipeline_layout: PfnCreatePipelineLayout,
    destroy_pipeline_layout: PfnDestroyPipelineLayout,
    create_compute_pipelines: PfnCreateComputePipelines,
    destroy_pipeline: PfnDestroyPipeline,
    get_executable_properties: PfnGetPipelineExecutableProperties,
    get_executable_statistics: PfnGetPipelineExecutableStatistics,
}

impl VulkanProbe {
    /// Opens the loader, creates an instance, picks a device that EXPOSES the extension and creates
    /// a device with it enabled. `None` (with a printed reason) when any step is unavailable.
    fn open() -> Option<Self> {
        // SAFETY: the string is a static NUL-terminated ANSI literal; `LoadLibraryA` returns the
        // module handle or NULL, and the result is null-checked immediately.
        let module = unsafe { os::LoadLibraryA(c"vulkan-1.dll".as_ptr()) };
        if module.is_null() {
            eprintln!("SKIP particle_sim_occupancy: vulkan-1.dll did not load");
            return None;
        }
        // SAFETY: `module` is the live HMODULE just returned by `LoadLibraryA`, and the symbol name
        // is a static NUL-terminated literal. A null result is checked before any call.
        let gipa_raw = unsafe { os::GetProcAddress(module, c"vkGetInstanceProcAddr".as_ptr()) };
        if gipa_raw.is_null() {
            eprintln!("SKIP particle_sim_occupancy: vkGetInstanceProcAddr not exported");
            return None;
        }
        // SAFETY: `gipa_raw` is the loader's exported `vkGetInstanceProcAddr`, whose signature is
        // `PfnGetInstanceProcAddr` by definition; both are pointer-sized.
        let gipa: PfnGetInstanceProcAddr = unsafe { std::mem::transmute(gipa_raw) };

        // SAFETY: `gipa` with a NULL instance resolves the global entry points, which is exactly
        // what `vkCreateInstance` is; the signature matches `vulkan_core.h`.
        let create_instance: PfnCreateInstance =
            unsafe { load_instance_fn(gipa, ptr::null_mut(), c"vkCreateInstance") };

        let app = VkApplicationInfo {
            s_type: ST_APPLICATION_INFO,
            p_next: ptr::null(),
            p_application_name: c"boyko particle_sim occupancy probe".as_ptr(),
            application_version: 0,
            p_engine_name: c"boyko-engine".as_ptr(),
            engine_version: 0,
            api_version: VK_API_VERSION_1_3,
        };
        let ici = VkInstanceCreateInfo {
            s_type: ST_INSTANCE_CREATE_INFO,
            p_next: ptr::null(),
            flags: 0,
            p_application_info: &app,
            enabled_layer_count: 0,
            pp_enabled_layer_names: ptr::null(),
            enabled_extension_count: 0,
            pp_enabled_extension_names: ptr::null(),
        };
        let mut instance: VkHandle = ptr::null_mut();
        // SAFETY: `ici` and the `VkApplicationInfo` it points at live for the whole call; no layer
        // or extension array is passed, so the two null pointers match their zero counts.
        let r = unsafe { create_instance(&ici, ptr::null(), &mut instance) };
        if r != 0 || instance.is_null() {
            eprintln!("SKIP particle_sim_occupancy: vkCreateInstance failed ({r})");
            return None;
        }

        // SAFETY (all four): each entry point is resolved from the live instance under its own
        // `vulkan_core.h` signature.
        let destroy_instance: PfnDestroyInstance =
            unsafe { load_instance_fn(gipa, instance, c"vkDestroyInstance") };
        let enumerate_devices: PfnEnumeratePhysicalDevices =
            unsafe { load_instance_fn(gipa, instance, c"vkEnumeratePhysicalDevices") };
        let get_props: PfnGetPhysicalDeviceProperties =
            unsafe { load_instance_fn(gipa, instance, c"vkGetPhysicalDeviceProperties") };
        let get_queue_props: PfnGetPhysicalDeviceQueueFamilyProperties = unsafe {
            load_instance_fn(gipa, instance, c"vkGetPhysicalDeviceQueueFamilyProperties")
        };
        let enumerate_dev_ext: PfnEnumerateDeviceExtensionProperties =
            unsafe { load_instance_fn(gipa, instance, c"vkEnumerateDeviceExtensionProperties") };
        let create_device: PfnCreateDevice =
            unsafe { load_instance_fn(gipa, instance, c"vkCreateDevice") };

        let mut count = 0u32;
        // SAFETY: the two-call idiom — the first pass writes only the count.
        unsafe { enumerate_devices(instance, &mut count, ptr::null_mut()) };
        let mut physical: Vec<VkHandle> = vec![ptr::null_mut(); count as usize];
        // SAFETY: `physical` holds exactly `count` slots, which is what the first call reported.
        unsafe { enumerate_devices(instance, &mut count, physical.as_mut_ptr()) };

        // The pick: a DISCRETE device that exposes the extension, else any device that does. The
        // preference matters because this laptop also exposes an integrated AMD device whose
        // register file is a different machine's — reporting its numbers under "RTX 3060" would be
        // a wrong measurement rather than a missing one.
        let mut chosen: Option<(VkHandle, String, u32)> = None;
        for &pd in &physical {
            let mut props: VkPhysicalDeviceProperties =
                // SAFETY: every field is a plain integer or a byte array; the driver overwrites
                // the whole struct before anything reads it.
                unsafe { std::mem::zeroed() };
            // SAFETY: `pd` is a live physical device from the enumeration; `props` is the full
            // 824-byte struct the driver writes.
            unsafe { get_props(pd, &mut props) };
            // SAFETY: `device_name` is the driver-filled NUL-terminated array.
            let name = unsafe { c_array_to_string(&props.device_name) };

            let mut ext_count = 0u32;
            // SAFETY: two-call idiom, count-only pass.
            unsafe { enumerate_dev_ext(pd, ptr::null(), &mut ext_count, ptr::null_mut()) };
            let mut exts: Vec<VkExtensionProperties> =
                // SAFETY: the struct is a char array plus a u32; the driver fills every slot.
                (0..ext_count).map(|_| unsafe { std::mem::zeroed() }).collect();
            // SAFETY: `exts` holds exactly `ext_count` slots.
            unsafe { enumerate_dev_ext(pd, ptr::null(), &mut ext_count, exts.as_mut_ptr()) };
            let has_ext = exts.iter().any(|e| {
                // SAFETY: `extension_name` is the driver-filled NUL-terminated array.
                let name = unsafe { c_array_to_string(&e.extension_name) };
                name == "VK_KHR_pipeline_executable_properties"
            });
            if !has_ext {
                continue;
            }

            let mut qf_count = 0u32;
            // SAFETY: two-call idiom, count-only pass.
            unsafe { get_queue_props(pd, &mut qf_count, ptr::null_mut()) };
            let mut qfs: Vec<VkQueueFamilyProperties> = (0..qf_count)
                // SAFETY: the struct is four integer fields; the driver fills every slot.
                .map(|_| unsafe { std::mem::zeroed() })
                .collect();
            // SAFETY: `qfs` holds exactly `qf_count` slots.
            unsafe { get_queue_props(pd, &mut qf_count, qfs.as_mut_ptr()) };
            let Some(qi) = qfs
                .iter()
                .position(|q| q.queue_flags & VK_QUEUE_COMPUTE_BIT != 0)
            else {
                continue;
            };

            let discrete = props.device_type == VK_PHYSICAL_DEVICE_TYPE_DISCRETE_GPU;
            if discrete || chosen.is_none() {
                chosen = Some((pd, name, qi as u32));
                if discrete {
                    break;
                }
            }
        }

        let Some((pd, device_name, queue_family)) = chosen else {
            eprintln!(
                "SKIP particle_sim_occupancy: no physical device exposes \
                 VK_KHR_pipeline_executable_properties"
            );
            // SAFETY: `instance` is live and nothing else references it.
            unsafe { destroy_instance(instance, ptr::null()) };
            return None;
        };

        let priority = 1.0f32;
        let qci = VkDeviceQueueCreateInfo {
            s_type: ST_DEVICE_QUEUE_CREATE_INFO,
            p_next: ptr::null(),
            flags: 0,
            queue_family_index: queue_family,
            queue_count: 1,
            p_queue_priorities: &priority,
        };
        let mut exec_features = VkPipelineExecutableFeaturesKhr {
            s_type: ST_PIPELINE_EXECUTABLE_FEATURES_KHR,
            p_next: ptr::null_mut(),
            pipeline_executable_info: 1,
        };
        let ext_name = c"VK_KHR_pipeline_executable_properties";
        let ext_ptrs = [ext_name.as_ptr()];
        let dci = VkDeviceCreateInfo {
            s_type: ST_DEVICE_CREATE_INFO,
            p_next: (&raw mut exec_features).cast(),
            flags: 0,
            queue_create_info_count: 1,
            p_queue_create_infos: &qci,
            enabled_layer_count: 0,
            pp_enabled_layer_names: ptr::null(),
            enabled_extension_count: 1,
            pp_enabled_extension_names: ext_ptrs.as_ptr(),
            p_enabled_features: ptr::null(),
        };
        let mut device: VkHandle = ptr::null_mut();
        // SAFETY: every pointed-at struct (`qci`, `exec_features`, `ext_ptrs`, `ext_name`) outlives
        // the call; the extension count matches the array length; the feature struct is the one the
        // extension defines for `pNext` and asks for the capability the statistics query needs.
        let r = unsafe { create_device(pd, &dci, ptr::null(), &mut device) };
        if r != 0 || device.is_null() {
            eprintln!("SKIP particle_sim_occupancy: vkCreateDevice failed ({r})");
            // SAFETY: `instance` is live and nothing else references it.
            unsafe { destroy_instance(instance, ptr::null()) };
            return None;
        }

        // SAFETY: resolved from the live instance under its `vulkan_core.h` signature.
        let gdpa: PfnGetDeviceProcAddr =
            unsafe { load_instance_fn(gipa, instance, c"vkGetDeviceProcAddr") };

        // The two teardown entry points FIRST, because everything after this point is inside the
        // one window where a panic would strand a live `VkDevice`: `Self`'s `Drop` does not exist
        // until `Self` is constructed, so the guard below is what covers the gap.
        //
        // SAFETY (both): resolved from the live device under their `vulkan_core.h` signatures;
        // both are Vulkan 1.0 core, so a `None` here means the loader is broken, not that the
        // function is optional — hence the hard `expect` rather than a skip.
        let destroy_device: PfnDestroyDevice =
            unsafe { load_device_fn(gdpa, device, c"vkDestroyDevice") }
                .expect("invariant: vkDestroyDevice is Vulkan 1.0 core and must resolve");

        /// Tears the device, instance and loader module down in creation-reverse order if `open`
        /// leaves the resolution window without disarming it — a panic from any `expect` below, or
        /// an early return added later.
        ///
        /// Test-only, and the failure it covers is not expected to happen: every remaining entry
        /// point is core or is guaranteed by the extension that was just enabled. It exists anyway
        /// because "the leak is unreachable" is the same shape of claim this whole rung exists to
        /// stop taking on trust, and because a stranded `VkDevice` in a `--test-threads=1` binary
        /// is invisible until some later probe cannot create one.
        struct OpenGuard {
            module: *mut c_void,
            instance: VkHandle,
            device: VkHandle,
            destroy_device: PfnDestroyDevice,
            destroy_instance: PfnDestroyInstance,
            armed: bool,
        }
        impl Drop for OpenGuard {
            fn drop(&mut self) {
                if !self.armed {
                    return;
                }
                // SAFETY: both handles are live (created above, and nothing has destroyed them —
                // the guard is disarmed on the one path that transfers ownership to `Self`), and
                // no pipeline or shader module exists yet, so nothing outstanding references the
                // device.
                unsafe {
                    (self.destroy_device)(self.device, ptr::null());
                    (self.destroy_instance)(self.instance, ptr::null());
                    os::FreeLibrary(self.module);
                }
            }
        }
        let mut guard =
            OpenGuard { module, instance, device, destroy_device, destroy_instance, armed: true };

        // SAFETY (each): resolved from the live device under its own signature. The two KHR entry
        // points resolve only because the extension was enabled above; the rest are 1.0 core.
        let get_executable_properties: PfnGetPipelineExecutableProperties = unsafe {
            load_device_fn(gdpa, device, c"vkGetPipelineExecutablePropertiesKHR")
        }
        .expect("invariant: the extension was enabled, so its entry point must resolve");
        let get_executable_statistics: PfnGetPipelineExecutableStatistics = unsafe {
            load_device_fn(gdpa, device, c"vkGetPipelineExecutableStatisticsKHR")
        }
        .expect("invariant: the extension was enabled, so its entry point must resolve");
        let create_shader_module: PfnCreateShaderModule =
            unsafe { load_device_fn(gdpa, device, c"vkCreateShaderModule") }
                .expect("invariant: core");
        let destroy_shader_module: PfnDestroyShaderModule =
            unsafe { load_device_fn(gdpa, device, c"vkDestroyShaderModule") }
                .expect("invariant: core");
        let create_descriptor_set_layout: PfnCreateDescriptorSetLayout =
            unsafe { load_device_fn(gdpa, device, c"vkCreateDescriptorSetLayout") }
                .expect("invariant: core");
        let destroy_descriptor_set_layout: PfnDestroyDescriptorSetLayout =
            unsafe { load_device_fn(gdpa, device, c"vkDestroyDescriptorSetLayout") }
                .expect("invariant: core");
        let create_pipeline_layout: PfnCreatePipelineLayout =
            unsafe { load_device_fn(gdpa, device, c"vkCreatePipelineLayout") }
                .expect("invariant: core");
        let destroy_pipeline_layout: PfnDestroyPipelineLayout =
            unsafe { load_device_fn(gdpa, device, c"vkDestroyPipelineLayout") }
                .expect("invariant: core");
        let create_compute_pipelines: PfnCreateComputePipelines =
            unsafe { load_device_fn(gdpa, device, c"vkCreateComputePipelines") }
                .expect("invariant: core");
        let destroy_pipeline: PfnDestroyPipeline =
            unsafe { load_device_fn(gdpa, device, c"vkDestroyPipeline") }
                .expect("invariant: core");

        // Ownership transfers to `Self`, whose `Drop` performs the identical teardown.
        guard.armed = false;
        Some(Self {
            module,
            instance,
            device,
            device_name,
            destroy_instance,
            destroy_device,
            create_shader_module,
            destroy_shader_module,
            create_descriptor_set_layout,
            destroy_descriptor_set_layout,
            create_pipeline_layout,
            destroy_pipeline_layout,
            create_compute_pipelines,
            destroy_pipeline,
            get_executable_properties,
            get_executable_statistics,
        })
    }

    /// Builds one compute pipeline per module WITH the capture bit and reads back its statistics.
    fn measure<const N: usize>(
        &self,
        modules: [(&'static str, &'static [u32]); N],
    ) -> Vec<ModuleReport> {
        // One layout serves all three modules — which is a fact about the engine, not a convenience
        // here: the three sim variants are interface-identical (see the manifest row), so the same
        // eleven storage bindings and the same 8-byte push range describe every one of them.
        let bindings: Vec<VkDescriptorSetLayoutBinding> = (0..PARTICLE_BINDING_COUNT)
            .map(|b| VkDescriptorSetLayoutBinding {
                binding: b,
                descriptor_type: VK_DESCRIPTOR_TYPE_STORAGE_BUFFER,
                descriptor_count: 1,
                stage_flags: VK_SHADER_STAGE_COMPUTE_BIT,
                p_immutable_samplers: ptr::null(),
            })
            .collect();
        let dslci = VkDescriptorSetLayoutCreateInfo {
            s_type: ST_DESCRIPTOR_SET_LAYOUT_CREATE_INFO,
            p_next: ptr::null(),
            flags: 0,
            binding_count: PARTICLE_BINDING_COUNT,
            p_bindings: bindings.as_ptr(),
        };
        let mut set_layout: VkNonDispatchable = 0;
        // SAFETY: `bindings` outlives the call and holds exactly `PARTICLE_BINDING_COUNT` entries.
        let r = unsafe {
            (self.create_descriptor_set_layout)(
                self.device,
                &dslci,
                ptr::null(),
                &mut set_layout,
            )
        };
        assert_eq!(r, 0, "vkCreateDescriptorSetLayout failed ({r})");

        let push = VkPushConstantRange {
            stage_flags: VK_SHADER_STAGE_COMPUTE_BIT,
            offset: 0,
            size: PARTICLE_SIM_PUSH_BYTES,
        };
        let plci = VkPipelineLayoutCreateInfo {
            s_type: ST_PIPELINE_LAYOUT_CREATE_INFO,
            p_next: ptr::null(),
            flags: 0,
            set_layout_count: 1,
            p_set_layouts: &set_layout,
            push_constant_range_count: 1,
            p_push_constant_ranges: &push,
        };
        let mut layout: VkNonDispatchable = 0;
        // SAFETY: `set_layout` and `push` outlive the call and match their declared counts.
        let r = unsafe {
            (self.create_pipeline_layout)(self.device, &plci, ptr::null(), &mut layout)
        };
        assert_eq!(r, 0, "vkCreatePipelineLayout failed ({r})");

        let mut reports = Vec::with_capacity(N);
        for (label, spirv) in modules {
            let smci = VkShaderModuleCreateInfo {
                s_type: ST_SHADER_MODULE_CREATE_INFO,
                p_next: ptr::null(),
                flags: 0,
                code_size: std::mem::size_of_val(spirv),
                p_code: spirv.as_ptr(),
            };
            let mut shader: VkNonDispatchable = 0;
            // SAFETY: `spirv` is a `'static` word slice from `embed_spirv!` — 4-aligned by its own
            // construction — and `code_size` is its length in BYTES, which is what Vulkan wants.
            let r = unsafe {
                (self.create_shader_module)(self.device, &smci, ptr::null(), &mut shader)
            };
            assert_eq!(r, 0, "[{label}] vkCreateShaderModule failed ({r})");

            let cpci = VkComputePipelineCreateInfo {
                s_type: ST_COMPUTE_PIPELINE_CREATE_INFO,
                p_next: ptr::null(),
                // THE CAPTURE BIT — without it the driver discards the statistics and the query
                // below returns an empty set, which the test asserts against.
                flags: VK_PIPELINE_CREATE_CAPTURE_STATISTICS_BIT_KHR,
                stage: VkPipelineShaderStageCreateInfo {
                    s_type: ST_PIPELINE_SHADER_STAGE_CREATE_INFO,
                    p_next: ptr::null(),
                    flags: 0,
                    stage: VK_SHADER_STAGE_COMPUTE_BIT,
                    module: shader,
                    p_name: c"main".as_ptr(),
                    p_specialization_info: ptr::null(),
                },
                layout,
                base_pipeline_handle: 0,
                base_pipeline_index: -1,
            };
            let mut pipeline: VkNonDispatchable = 0;
            // SAFETY: `cpci` and the entry-point name outlive the call; `layout` and `shader` are
            // live handles created above; one pipeline is requested and one slot is provided.
            let r = unsafe {
                (self.create_compute_pipelines)(
                    self.device,
                    0,
                    1,
                    &cpci,
                    ptr::null(),
                    &mut pipeline,
                )
            };
            assert_eq!(r, 0, "[{label}] vkCreateComputePipelines failed ({r})");

            let pinfo = VkPipelineInfoKhr {
                s_type: ST_PIPELINE_INFO_KHR,
                p_next: ptr::null(),
                pipeline,
            };
            let mut exec_count = 0u32;
            // SAFETY: two-call idiom — the count-only pass writes only `exec_count`.
            unsafe {
                (self.get_executable_properties)(
                    self.device,
                    &pinfo,
                    &mut exec_count,
                    ptr::null_mut(),
                )
            };
            assert!(exec_count > 0, "[{label}] the pipeline reports no executable");
            let mut execs: Vec<VkPipelineExecutablePropertiesKhr> = (0..exec_count)
                .map(|_| VkPipelineExecutablePropertiesKhr {
                    s_type: ST_PIPELINE_EXECUTABLE_PROPERTIES_KHR,
                    p_next: ptr::null_mut(),
                    stages: 0,
                    name: [0; VK_MAX_DESCRIPTION_SIZE],
                    description: [0; VK_MAX_DESCRIPTION_SIZE],
                    subgroup_size: 0,
                })
                .collect();
            // SAFETY: `execs` holds exactly `exec_count` slots, each with its `sType` set as the
            // spec requires before the driver fills the rest.
            unsafe {
                (self.get_executable_properties)(
                    self.device,
                    &pinfo,
                    &mut exec_count,
                    execs.as_mut_ptr(),
                )
            };

            // Executable 0: a compute pipeline has exactly one stage, so the first executable is
            // THE kernel. (`exec_count > 1` would mean the driver splits it, which this vendor does
            // not do for compute; the name is printed so a reader can see which one was read.)
            let einfo = VkPipelineExecutableInfoKhr {
                s_type: ST_PIPELINE_EXECUTABLE_INFO_KHR,
                p_next: ptr::null(),
                pipeline,
                executable_index: 0,
            };
            let mut stat_count = 0u32;
            // SAFETY: two-call idiom — the count-only pass writes only `stat_count`.
            unsafe {
                (self.get_executable_statistics)(
                    self.device,
                    &einfo,
                    &mut stat_count,
                    ptr::null_mut(),
                )
            };
            let mut stats: Vec<VkPipelineExecutableStatisticKhr> = (0..stat_count)
                .map(|_| VkPipelineExecutableStatisticKhr {
                    s_type: ST_PIPELINE_EXECUTABLE_STATISTIC_KHR,
                    p_next: ptr::null_mut(),
                    name: [0; VK_MAX_DESCRIPTION_SIZE],
                    description: [0; VK_MAX_DESCRIPTION_SIZE],
                    format: 0,
                    value: 0,
                })
                .collect();
            // SAFETY: `stats` holds exactly `stat_count` slots, each with its `sType` set.
            unsafe {
                (self.get_executable_statistics)(
                    self.device,
                    &einfo,
                    &mut stat_count,
                    stats.as_mut_ptr(),
                )
            };

            let decoded: Vec<(String, StatValue)> = stats
                .iter()
                .map(|s| {
                    // SAFETY: `name` is the driver-filled NUL-terminated array.
                    let n = unsafe { c_array_to_string(&s.name) };
                    (n, StatValue::decode(s.format, s.value))
                })
                .collect();
            // SAFETY: `execs[0].name` is the driver-filled NUL-terminated array.
            let executable_name = unsafe { c_array_to_string(&execs[0].name) };

            reports.push(ModuleReport {
                label,
                spirv_words: spirv.len(),
                subgroup_size: execs[0].subgroup_size,
                executable_name,
                stats: decoded,
            });

            // SAFETY: both handles are live, created above, and nothing references them after the
            // statistics were copied out into owned `String`s and `StatValue`s.
            unsafe {
                (self.destroy_pipeline)(self.device, pipeline, ptr::null());
                (self.destroy_shader_module)(self.device, shader, ptr::null());
            }
        }

        // SAFETY: both layouts are live and no pipeline referencing them survives the loop above.
        unsafe {
            (self.destroy_pipeline_layout)(self.device, layout, ptr::null());
            (self.destroy_descriptor_set_layout)(self.device, set_layout, ptr::null());
        }
        reports
    }
}

impl Drop for VulkanProbe {
    fn drop(&mut self) {
        // SAFETY: device before instance, and the loader module last — the reverse of creation
        // order. Every pipeline, layout and shader module was destroyed in `measure` before this
        // runs, so nothing outstanding references the device.
        unsafe {
            (self.destroy_device)(self.device, ptr::null());
            (self.destroy_instance)(self.instance, ptr::null());
            os::FreeLibrary(self.module);
        }
    }
}
