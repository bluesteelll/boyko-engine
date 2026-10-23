//! Render wave PX1a **O6 — `post_final`'s occupancy row** (`px1a/cut.md` C12; plan H4 Budgets "O6
//! occupancy: uber `post_final` vs arms compiled out — recorded; > 5 % ⇒ spec-constant stripping",
//! decided at the WR row; orchestrator ruling 67 Q7: record it at PX1a).
//!
//! `post_final` is ONE generated module whose tonemapper is a runtime mode word: a `[branch]`
//! dispatch over ACES Hill, Khronos PBR Neutral and Reinhard-Jodie. The question the row answers is
//! what that uber shape costs the compiled kernel against a module with the arms compiled out. The
//! twin is NOT committed: `boyko_shaderdsl::emit::emit_post_final_comp_with(PostFinalTwin::AcesOnly)`
//! prints the same traced body with the mode word replaced by the ACES constant, and this binary
//! DXCs it at test time under the committed module's frozen recipe, so DXC folds the dispatch away.
//!
//! - `the_aces_only_twin_compiles_the_dispatch_away` (host; SKIPS with an `eprintln` when no `dxc`
//!   resolves — a skipped run is not a pass): the anti-vacuity — the twin's bytes differ from the
//!   committed `post_final.comp.spv`, and it carries FEWER `DontFlatten` selections (the `[branch]`
//!   dispatch is gone), so the two columns below measure two different kernels.
//! - `post_final_uber_and_aces_only_twin_report_their_register_footprint` (`gpu-cap:`): both modules'
//!   `VK_KHR_pipeline_executable_properties` statistics, printed side by side (run with
//!   `--nocapture`). RECORDED, NOT GATED: no threshold, no register count is asserted (the
//!   `particle_sim_occupancy.rs` rule — a pinned count reds on a driver update and says nothing
//!   about the engine). It asserts only the mechanical conditions under which the columns mean
//!   anything: both pipelines compiled, each reported statistics, and the statistic NAMES agree.
//!
//! The headless probe below (own instance, own device with the one extension, no surface, no
//! submission) is `particle_sim_occupancy.rs`'s, adapted to `post_final`'s set-0 layout — the
//! engine's own device does not enable the extension, and must not (see that file's doc for why).
//!
//! Since render wave PX1b the `AcesOnly` twin's literal word also clears the sharpen bit, so its
//! column is "the tonemapper arms AND the RCAS stage compiled out" (critique O5).
//!
//! # Render wave PX1b — the register twin, GATED (`px1b/cut.md` D-R10 (3), ruling 94 Q7, rule (xviii))
//!
//! The RCAS stage is runtime-gated code in a shared row (the one `post_final` every path runs), so
//! its zero cost when off is a GATE, not a record: `PostFinalTwin::NoRcas` traces the same body
//! with the mode word masked by `!POST_MODE_SHARPEN`, so DXC folds the sharpen branch away (the
//! tonemapper stays the runtime word).
//!
//! - `the_no_rcas_twin_compiles_the_sharpen_branch_away` (host; SKIPS without `dxc` — a skipped
//!   run is not a pass): the anti-vacuity — the twin's bytes differ from the committed module, it
//!   carries ONE `OpImageFetch` against the uber module's five and one `DontFlatten` selection fewer.
//! - `post_final_sharpen_off_module_and_no_rcas_twin_registers_agree` (`gpu-cap:`; PX1b's
//!   `post_final_uber_and_no_rcas_twin_registers_agree`, re-aimed by render wave PXt): rule
//!   (xviii)'s thresholds — the statistic NAMES agree, every register statistic of the module a
//!   sharpen-off frame binds (`compute::post_final_spirv_for(false)`) is at most
//!   [`REGISTER_DELTA_MAX`] above the twin's, every occupancy statistic is equal where reported; no
//!   absolute count. It re-checks the twin's anti-vacuity itself, and it PANICS where it cannot
//!   measure (no `dxc`, no loader, no device exposing the extension, no register statistic): a gated
//!   row that skipped is red, never green (critique W2 item 3). PX1b measured the uber module at 36
//!   registers against the twin's 18 (OPEN-ITEMS 19); the priced escape — a second `post_final`
//!   pipeline chosen per frame by the host — is render wave PXt's.
//!
//! # Render wave PXt — the sharpen-off module (OPEN-ITEMS 19, `pxt/cut.md` D-O1…D-O4)
//!
//! - `the_sharpen_off_module_is_the_body_without_rcas` (host): (a) the module a sharpen-off frame
//!   binds carries ONE `OpImageFetch` and one `DontFlatten` selection fewer than the sharpen-on
//!   module, and different bytes — the anti-vacuity of the register gate above.
//! - `the_committed_sharpen_off_module_is_the_no_rcas_twin` (host; SKIPS without `dxc`): (b) that
//!   module IS the `NoRcas` twin, instruction for instruction once the debug-only instructions are
//!   dropped. So, once it holds, the register gate above compares the twin with itself and reds
//!   only where (b) does (critique O3): it is not an independent measurement of "zero cost when
//!   off" — (a) and (b) are, and the gate stays as the device-side row of rule (xviii).
//! - `the_agx_twin_compiles_the_arm_away` (host) and `post_final_agx_arm_registers_agree`
//!   (`gpu-cap:`): rule (xviii) on the AgX arm (critique W3) — both shipped modules (the sharpen-off
//!   default row and the RCAS heaviest) against themselves with the arm compiled out (the generated
//!   text's AgX dispatch test made `if (false)`, the `-D <BODY>=0` of a generated module),
//!   anti-vacuity on the arm's `Log2` / `Pow` / `DontFlatten`.
//! - Recorded, not gated (`gpu-cap:`): `post_final_sharpen_on_and_off_modules_report_their_registers`
//!   (the cost when the sharpen is on) and
//!   `post_final_sharpen_off_and_agx_only_twin_report_their_register_footprint` (O6 re-aimed at the
//!   default curve's path; its host anti-vacuity is `the_agx_only_twin_compiles_the_other_arms_away`).
//!
//! # Usage
//!
//! ```text
//! cargo test -p boyko_rhi_vulkan --test post_final_occupancy --locked -- --ignored --nocapture
//! ```
//!
//! `#![cfg(windows)]` for the same single reason as `particle_sim_occupancy.rs`: the loader is opened
//! through `boyko_rhi_vulkan::ffi::os` (`LoadLibraryA` / `GetProcAddress`); on another host this
//! binary compiles to nothing and reports `0 tests` — not a green run.

#![cfg(windows)]

use std::ffi::{CStr, c_char, c_void};
use std::path::PathBuf;
use std::process::Command;
use std::ptr;

use boyko_rhi_vulkan::compute::{post_final_spirv, post_final_spirv_for};
use boyko_shaderdsl::emit::{PostFinalTwin, emit_post_final_comp_with};
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
/// `VK_DESCRIPTOR_TYPE_SAMPLED_IMAGE`.
const VK_DESCRIPTOR_TYPE_SAMPLED_IMAGE: i32 = 2;
/// `VK_DESCRIPTOR_TYPE_STORAGE_IMAGE`.
const VK_DESCRIPTOR_TYPE_STORAGE_IMAGE: i32 = 3;
/// `VK_DESCRIPTOR_TYPE_UNIFORM_BUFFER`.
const VK_DESCRIPTOR_TYPE_UNIFORM_BUFFER: i32 = 6;
/// `VK_QUEUE_COMPUTE_BIT`.
const VK_QUEUE_COMPUTE_BIT: u32 = 0x0000_0002;
/// `VK_PHYSICAL_DEVICE_TYPE_DISCRETE_GPU`.
const VK_PHYSICAL_DEVICE_TYPE_DISCRETE_GPU: i32 = 2;
/// `VK_API_VERSION_1_3`.
const VK_API_VERSION_1_3: u32 = (1 << 22) | (3 << 12);

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

/// `post_final`'s `COMPUTE` push range, in bytes: the engine declares a 4-byte range it never pushes
/// (`create_compute_pipeline` rejects 0 — PX1a C5's deviation), so the probe's layout matches the
/// shipped pipeline layout exactly.
const POST_FINAL_PUSH_BYTES: u32 = 4;

/// `post_final`'s set 0, in `boyko_shaderdsl::post::POST_FINAL_BINDINGS` order: `gScene` (a sampled
/// image read at `GENERAL`) @0, `gDisplay` (a storage image) @1, the `PostFinal` cbuffer @2.
const POST_FINAL_LAYOUT: [(u32, i32); 3] = [
    (0, VK_DESCRIPTOR_TYPE_SAMPLED_IMAGE),
    (1, VK_DESCRIPTOR_TYPE_STORAGE_IMAGE),
    (2, VK_DESCRIPTOR_TYPE_UNIFORM_BUFFER),
];

/// The pinned SDK's `dxc`, then `$VULKAN_SDK/Bin`, then `PATH` (`post_chain_spv_sync.rs`'s order).
fn find_dxc() -> Option<PathBuf> {
    let pinned = PathBuf::from("C:/VulkanSDK/1.4.350.0/Bin/dxc.exe");
    if pinned.exists() {
        return Some(pinned);
    }
    if let Ok(sdk) = std::env::var("VULKAN_SDK") {
        let c = PathBuf::from(sdk).join("Bin").join("dxc.exe");
        if c.exists() {
            return Some(c);
        }
    }
    Command::new("dxc.exe").arg("--version").output_within(DXC_DEADLINE).is_ok().then(|| PathBuf::from("dxc.exe"))
}

/// The `PostFinalTwin::AcesOnly` twin, DXC'd under `post_final.comp.hlsl`'s frozen recipe
/// (`-spirv -T cs_6_0 -E main -fspv-target-env=vulkan1.3`, no `-O`, no `-D`) into a scratch file.
fn compile_twin(dxc: &PathBuf) -> Vec<u32> {
    compile_twin_of(dxc, PostFinalTwin::AcesOnly, "post_final_aces_only")
}

/// `twin`, DXC'd under the frozen recipe into scratch files named after `stem`.
fn compile_twin_of(dxc: &PathBuf, twin: PostFinalTwin, stem: &str) -> Vec<u32> {
    compile_source(dxc, &emit_post_final_comp_with(twin), stem)
}

/// `source` (a whole `post_final` module), DXC'd under the frozen recipe into scratch files named
/// after `stem`.
fn compile_source(dxc: &PathBuf, source: &str, stem: &str) -> Vec<u32> {
    let hlsl = child_guard::scratch_path(&format!("{stem}.comp.hlsl"));
    let spv = child_guard::scratch_path(&format!("{stem}.comp.spv"));
    std::fs::write(&hlsl, source).expect("invariant: the scratch dir accepts the twin's source");
    let status = Command::new(dxc)
        .args(["-spirv", "-T", "cs_6_0", "-E", "main", "-fspv-target-env=vulkan1.3"])
        .arg(&hlsl)
        .arg("-Fo")
        .arg(&spv)
        .status_within(DXC_DEADLINE)
        .expect("invariant: dxc was located and must run");
    assert!(status.success(), "dxc failed on the `{stem}` twin under post_final's frozen recipe");
    let bytes = std::fs::read(&spv).expect("invariant: dxc wrote the twin's .spv");
    let _ = std::fs::remove_file(&hlsl);
    let _ = std::fs::remove_file(&spv);
    let (words, rest) = bytes.as_chunks::<4>();
    assert!(rest.is_empty(), "a SPIR-V module is a whole number of words");
    words.iter().map(|w| u32::from_le_bytes(*w)).collect()
}

/// The `OpSelectionMerge` (opcode 247) instructions whose selection control carries `DontFlatten`
/// (0x2) — the `[branch]` attribute's lowering — in a SPIR-V module.
fn dont_flatten_count(words: &[u32]) -> usize {
    let mut i = 5; // past the header
    let mut n = 0;
    while i < words.len() {
        let (count, op) = ((words[i] >> 16) as usize, words[i] & 0xFFFF);
        assert!(count > 0, "a zero-length SPIR-V instruction at word {i}");
        if op == 247 && count >= 3 && words[i + 2] & 0x2 != 0 {
            n += 1;
        }
        i += count;
    }
    n
}

/// The `OpImageFetch` (opcode 95) instructions of a SPIR-V module.
fn image_fetch_count(words: &[u32]) -> usize {
    let mut i = 5; // past the header
    let mut n = 0;
    while i < words.len() {
        let (count, op) = ((words[i] >> 16) as usize, words[i] & 0xFFFF);
        assert!(count > 0, "a zero-length SPIR-V instruction at word {i}");
        n += usize::from(op == 95);
        i += count;
    }
    n
}

/// Render wave PX1b — how far a register statistic of the uber module may exceed the `NoRcas`
/// twin's (rule (xviii); `00-INDEX.md` §10).
const REGISTER_DELTA_MAX: f64 = 8.0;

/// `Err(why)` unless `twin` is the committed `uber` module with the sharpen branch compiled out:
/// different bytes, ONE `OpImageFetch` against five, one `DontFlatten` selection fewer.
fn no_rcas_anti_vacuity(uber: &[u32], twin: &[u32]) -> Result<(), String> {
    if twin == uber {
        return Err("the NoRcas twin's SPIR-V equals the committed module: the two columns would be one kernel".to_string());
    }
    let (uf, tf) = (image_fetch_count(uber), image_fetch_count(twin));
    if (uf, tf) != (5, 1) {
        return Err(format!("OpImageFetch: uber {uf}, NoRcas twin {tf} (want 5 and 1): the sharpen branch was not compiled out"));
    }
    let (ud, td) = (dont_flatten_count(uber), dont_flatten_count(twin));
    if td + 1 != ud {
        return Err(format!("DontFlatten selections: uber {ud}, NoRcas twin {td} (want exactly one fewer)"));
    }
    Ok(())
}

/// The NoRcas twin is the committed body with the sharpen stage compiled out. SKIPS without `dxc`.
#[test]
fn the_no_rcas_twin_compiles_the_sharpen_branch_away() {
    let uber_text = emit_post_final_comp_with(PostFinalTwin::Uber);
    let twin_text = emit_post_final_comp_with(PostFinalTwin::NoRcas);
    assert_ne!(twin_text, uber_text, "the NoRcas twin must differ from the committed template");
    assert!(twin_text.starts_with("// TEST-ONLY REGISTER TWIN"), "the twin says what it is");
    assert!(twin_text.contains("uint post_mode_words = mode_words & 4294967293u;"), "the twin masks the sharpen bit off the runtime word");
    let Some(dxc) = find_dxc() else {
        eprintln!("SKIP post_final_occupancy: no dxc resolves on this host — a skipped run is NOT a pass");
        return;
    };
    let twin = compile_twin_of(&dxc, PostFinalTwin::NoRcas, "post_final_no_rcas");
    let uber = post_final_spirv();
    if let Err(why) = no_rcas_anti_vacuity(uber, &twin) {
        panic!("{why}");
    }
    // The red-first twin: the uber body itself (no mask) keeps the five fetches, so the check above
    // cannot pass on a twin that did not fold.
    assert!(no_rcas_anti_vacuity(uber, uber).is_err(), "the anti-vacuity must reject the unmasked module");
    eprintln!(
        "post_final_occupancy: OpImageFetch uber {}, no_rcas twin {}; DontFlatten {} vs {}; SPIR-V words {} vs {}",
        image_fetch_count(uber),
        image_fetch_count(&twin),
        dont_flatten_count(uber),
        dont_flatten_count(&twin),
        uber.len(),
        twin.len()
    );
}

// ---- Render wave PXt — the module a sharpen-off frame binds (OPEN-ITEMS 19, `pxt/cut.md` D-O4) ----

/// **(a)** The module a sharpen-off frame binds (`post_final_spirv_for(false)`) is the body without
/// RCAS: ONE `OpImageFetch` (the scene texel), one `DontFlatten` selection fewer than the module a
/// sharpen-on frame binds, and different bytes — the anti-vacuity of the sharpen-off register gate
/// below (host, no `dxc`).
#[test]
fn the_sharpen_off_module_is_the_body_without_rcas() {
    let (on, off) = (post_final_spirv_for(true), post_final_spirv_for(false));
    let fetches = image_fetch_count(off);
    assert_eq!(fetches, 1, "post_final_spirv_for(false) carries {fetches} OpImageFetch, not 1 — a sharpen-off frame binds the RCAS body (OPEN-ITEMS 19)");
    assert_eq!(image_fetch_count(on), 5, "post_final_spirv_for(true) must carry RCAS's four ring taps beside the scene texel");
    assert_eq!(dont_flatten_count(off) + 1, dont_flatten_count(on), "the sharpen-off module must carry exactly one DontFlatten selection fewer (the sharpen stage's)");
    assert!(on != off, "the two modules must differ");
}

/// The SPIR-V words a driver compiles: the module without its debug-only instructions (`OpSource*`,
/// `OpString`, `OpName`, `OpMemberName`, `OpLine`, `OpNoLine`, `OpModuleProcessed`), header kept.
fn semantic_words(words: &[u32]) -> Vec<u32> {
    const DEBUG_OPS: [u32; 9] = [2, 3, 4, 5, 6, 7, 8, 317, 330];
    let mut out = words[..5].to_vec();
    let mut i = 5; // past the header
    while i < words.len() {
        let (count, op) = ((words[i] >> 16) as usize, words[i] & 0xFFFF);
        assert!(count > 0, "a zero-length SPIR-V instruction at word {i}");
        if !DEBUG_OPS.contains(&op) {
            out.extend_from_slice(&words[i..i + count]);
        }
        i += count;
    }
    out
}

/// **(b)** The module a sharpen-off frame binds IS the body-out twin: the test-only `NoRcas`
/// emission, DXC'd under the frozen recipe, equals `post_final_spirv_for(false)` instruction for
/// instruction once the debug-only instructions are dropped — on every device, without a driver's
/// fold. SKIPS without `dxc` (a skipped run is not a pass).
#[test]
fn the_committed_sharpen_off_module_is_the_no_rcas_twin() {
    let Some(dxc) = find_dxc() else {
        eprintln!("SKIP post_final_occupancy: no dxc resolves on this host — a skipped run is NOT a pass");
        return;
    };
    let twin = compile_twin_of(&dxc, PostFinalTwin::NoRcas, "post_final_no_rcas");
    let off = post_final_spirv_for(false);
    assert!(
        semantic_words(off) == semantic_words(&twin),
        "post_final_spirv_for(false) ({} words) is not the NoRcas twin ({} words) — a sharpen-off frame does not bind the body without RCAS (OPEN-ITEMS 19)",
        off.len(),
        twin.len()
    );
}

/// A statistic's numeric value (`None` for a boolean).
fn stat_number(v: &StatValue) -> Option<f64> {
    match *v {
        StatValue::Bool(_) => None,
        StatValue::I64(x) => Some(x as f64),
        StatValue::U64(x) => Some(x as f64),
        StatValue::F64(x) => Some(x),
    }
}

/// Rule (xviii)'s thresholds on two reports (a shipped module, its register twin with one
/// runtime-gated body compiled out): `Err(why)` when the statistic names differ, when no statistic
/// names a register count (the row could not measure), when a register statistic of the shipped
/// module exceeds the twin's by more than [`REGISTER_DELTA_MAX`], or when an occupancy statistic
/// differs. No absolute count is asserted.
fn registers_agree(uber: &[(String, StatValue)], twin: &[(String, StatValue)]) -> Result<String, String> {
    let names = |r: &[(String, StatValue)]| {
        let mut n: Vec<String> = r.iter().map(|(k, _)| k.clone()).collect();
        n.sort();
        n
    };
    if names(uber) != names(twin) {
        return Err(format!("the statistic sets differ: uber {:?}, twin {:?}", names(uber), names(twin)));
    }
    let mut seen_register = false;
    let mut report = Vec::new();
    for (name, uv) in uber {
        let lower = name.to_ascii_lowercase();
        let tv = &twin.iter().find(|(k, _)| k == name).expect("invariant: the name sets are equal").1;
        if lower.contains("register") {
            let (Some(u), Some(t)) = (stat_number(uv), stat_number(tv)) else {
                return Err(format!("`{name}` is not a number on both modules ({uv} / {tv})"));
            };
            seen_register = true;
            report.push(format!("{name}: uber {u}, twin {t}, delta {}", u - t));
            if u - t > REGISTER_DELTA_MAX {
                return Err(format!("`{name}`: the shipped module uses {u}, the twin {t} — delta {} > {REGISTER_DELTA_MAX}", u - t));
            }
        } else if lower.contains("occupancy") {
            report.push(format!("{name}: uber {uv}, twin {tv}"));
            if stat_number(uv) != stat_number(tv) {
                return Err(format!("`{name}`: the shipped module's {uv} differs from the twin's {tv}"));
            }
        }
    }
    if !seen_register {
        return Err(format!("no statistic names a register count ({:?}): the row cannot gate", names(uber)));
    }
    Ok(report.join("; "))
}

/// **Render wave PX1b — the register twin, GATED; render wave PXt — re-aimed at the module a
/// sharpen-off frame binds** (OPEN-ITEMS 19, `pxt/cut.md` D-O4 (c)): `post_final_spirv_for(false)`
/// against the `NoRcas` twin. The twin's anti-vacuity is taken against the RCAS module (it folds
/// the sharpen branch: 5 → 1 `OpImageFetch`). See the module doc.
#[test]
#[ignore = "gpu-cap: needs a real GPU exposing VK_KHR_pipeline_executable_properties and the pinned dxc; PX1b D-R10 (3) re-aimed at PXt's sharpen-off module, the post_final register twin GATED by rule (xviii) (a run that cannot measure panics); run with --nocapture"]
fn post_final_sharpen_off_module_and_no_rcas_twin_registers_agree() {
    let dxc = find_dxc().expect("GATE post_final_occupancy: no dxc resolves on this host — the gated row cannot measure, so it is red");
    let twin = compile_twin_of(&dxc, PostFinalTwin::NoRcas, "post_final_no_rcas");
    if let Err(why) = no_rcas_anti_vacuity(post_final_spirv_for(true), &twin) {
        panic!("{why}");
    }
    let off = post_final_spirv_for(false);
    let probe = VulkanProbe::open()
        .expect("GATE post_final_occupancy: the probe could not open a device with VK_KHR_pipeline_executable_properties (the SKIP line above says why) — the gated row cannot measure, so it is red");
    let reports = probe.measure(&[("sharpen-off (committed)", off), ("no_rcas (twin)", twin.as_slice())]);
    println!("\n=== post_final register twin (PX1b D-R10 (3), GATED by rule (xviii)) ===");
    println!("device: {}", probe.device_name);
    for r in &reports {
        println!("\n[{}] {} SPIR-V words, executable \"{}\", subgroupSize {}", r.label, r.spirv_words, r.executable_name, r.subgroup_size);
        for (name, value) in &r.stats {
            println!("    {name:.<44} {value}");
        }
    }
    assert_eq!(reports.len(), 2, "both modules must have compiled");
    for r in &reports {
        assert!(!r.stats.is_empty(), "[{}] the driver returned no statistics — the CAPTURE_STATISTICS bit did not take effect", r.label);
    }
    match registers_agree(&reports[0].stats, &reports[1].stats) {
        Ok(summary) => println!("\nGATE green: {summary}"),
        Err(why) => panic!("GATE red: {why}"),
    }
}

/// The gate's comparator reds on each breach it exists for, every run (host; no GPU).
#[test]
fn the_register_gate_rejects_its_fixtures() {
    let row = |pairs: &[(&str, u64)]| -> Vec<(String, StatValue)> { pairs.iter().map(|&(k, v)| (k.to_string(), StatValue::U64(v))).collect() };
    let uber = row(&[("Register Count", 40), ("Occupancy", 16), ("Binary Size", 900)]);
    assert!(registers_agree(&uber, &row(&[("Register Count", 32), ("Occupancy", 16), ("Binary Size", 700)])).is_ok(), "a delta of 8 is within the rule");
    assert!(registers_agree(&uber, &row(&[("Register Count", 31), ("Occupancy", 16), ("Binary Size", 700)])).is_err(), "a delta of 9 is red");
    assert!(registers_agree(&uber, &row(&[("Register Count", 40), ("Occupancy", 24), ("Binary Size", 900)])).is_err(), "an occupancy change is red");
    assert!(registers_agree(&uber, &row(&[("Register Count", 40), ("Binary Size", 900)])).is_err(), "a different statistic set is red");
    let blind = row(&[("Binary Size", 900)]);
    assert!(registers_agree(&blind, &blind).is_err(), "a report with no register statistic cannot gate, so it is red");
    assert!(registers_agree(&row(&[("Register Count", 24)]), &row(&[("Register Count", 40)])).is_ok(), "the uber module using FEWER registers is not a cost");
}

// ---- Render wave PXt — the AgX arm's register twin (rule (xviii); `pxt/critique.md` W3) -----------

/// The AgX dispatch test as the generator prints it (`post::tonemap_select`'s first arm).
const AGX_DISPATCH_TEST: &str = "if (post_tonemap_mode == TONEMAP_AGX) {";

/// `GLSL.std.450 Pow`.
const GLSL_POW: u32 = 26;
/// `GLSL.std.450 Log2`.
const GLSL_LOG2: u32 = 30;

/// Rule (xviii)'s `-D <BODY>=0` for a GENERATED module (the printer has no preprocessor node): the
/// module's text with its AgX dispatch test made `if (false)`, so DXC folds the AgX arm away while
/// the other arms keep reading the runtime word. `Err` unless the test occurs exactly once.
fn without_the_agx_arm(text: &str) -> Result<String, String> {
    match text.matches(AGX_DISPATCH_TEST).count() {
        1 => Ok(text.replacen(AGX_DISPATCH_TEST, "if (false) {", 1)),
        n => Err(format!(
            "the AgX dispatch test `{AGX_DISPATCH_TEST}` occurs {n} times in the module (want 1): there is no AgX arm to compile out"
        )),
    }
}

/// The `GLSL.std.450` extended instructions numbered `inst` in a SPIR-V module.
fn glsl_ext_count(words: &[u32], inst: u32) -> usize {
    let mut glsl = None;
    let mut i = 5; // past the header
    let mut n = 0;
    while i < words.len() {
        let (count, op) = ((words[i] >> 16) as usize, words[i] & 0xFFFF);
        assert!(count > 0, "a zero-length SPIR-V instruction at word {i}");
        if op == 11 && count >= 3 {
            // OpExtInstImport: <result id> <name, NUL-terminated, little-endian words>.
            let name: Vec<u8> = words[i + 2..i + count].iter().flat_map(|w| w.to_le_bytes()).take_while(|b| *b != 0).collect();
            if name == b"GLSL.std.450" {
                glsl = Some(words[i + 1]);
            }
        } else if op == 12 && count >= 5 && Some(words[i + 3]) == glsl && words[i + 4] == inst {
            // OpExtInst: <type> <result> <set> <instruction> <operands...>.
            n += 1;
        }
        i += count;
    }
    n
}

/// `Err(why)` unless `twin` is `shipped` with the AgX arm compiled out — rule (xviii)'s anti-vacuity
/// on the arm's named marker ops: different bytes; `GLSL.std.450 Log2` 3 → 0 (the arm's three
/// `log2`, the module's only ones); `Pow` exactly three fewer (the arm's `pow(·, 2.2)`; the OETF's
/// three stay); one `DontFlatten` selection fewer (the arm's dispatch level).
fn agx_arm_anti_vacuity(shipped: &[u32], twin: &[u32]) -> Result<(), String> {
    if twin == shipped {
        return Err("the AgX twin's SPIR-V equals the shipped module: the two columns would be one kernel".to_string());
    }
    let (sl, tl) = (glsl_ext_count(shipped, GLSL_LOG2), glsl_ext_count(twin, GLSL_LOG2));
    if (sl, tl) != (3, 0) {
        return Err(format!("GLSL.std.450 Log2: shipped {sl}, AgX twin {tl} (want 3 and 0): the AgX arm was not compiled out"));
    }
    let (sp, tp) = (glsl_ext_count(shipped, GLSL_POW), glsl_ext_count(twin, GLSL_POW));
    if sp != tp + 3 {
        return Err(format!("GLSL.std.450 Pow: shipped {sp}, AgX twin {tp} (want exactly three fewer)"));
    }
    let (sd, td) = (dont_flatten_count(shipped), dont_flatten_count(twin));
    if td + 1 != sd {
        return Err(format!("DontFlatten selections: shipped {sd}, AgX twin {td} (want exactly one fewer)"));
    }
    Ok(())
}

/// The shipped `post_final` modules that carry the AgX arm, each with its generated source and its
/// committed SPIR-V: `(label, source, spirv)` — rule (xviii)'s "default row and heaviest" of the
/// shared row: the sharpen-off module (most frames) and the RCAS module.
fn agx_rows() -> Vec<(&'static str, String, &'static [u32])> {
    vec![
        ("post_final (committed)", boyko_shaderdsl::emit::emit_post_final_comp(), post_final_spirv_for(true)),
        (
            "post_final_nosharpen (committed)",
            boyko_shaderdsl::emit::emit_post_final_nosharpen_comp(),
            post_final_spirv_for(false),
        ),
    ]
}

/// **Render wave PXt — the AgX arm's twin is the shipped module with the arm compiled out** (host;
/// the substitution is checked without `dxc`, the compiled twin SKIPS without it — a skipped run is
/// not a pass). Red-first: the shipped module itself, as its own twin, is rejected.
#[test]
fn the_agx_twin_compiles_the_arm_away() {
    for (label, source, shipped) in agx_rows() {
        let twin_text = without_the_agx_arm(&source).unwrap_or_else(|why| panic!("{label}: {why}"));
        let Some(dxc) = find_dxc() else {
            eprintln!("SKIP post_final_occupancy: no dxc resolves on this host — a skipped run is NOT a pass");
            return;
        };
        let twin = compile_source(&dxc, &twin_text, "post_final_no_agx");
        if let Err(why) = agx_arm_anti_vacuity(shipped, &twin) {
            panic!("{label}: {why}");
        }
        assert!(agx_arm_anti_vacuity(shipped, shipped).is_err(), "{label}: the anti-vacuity must reject the shipped module as its own twin");
        eprintln!(
            "post_final_occupancy: {label}: Log2 {} vs {}, Pow {} vs {}, DontFlatten {} vs {} (shipped vs no_agx twin)",
            glsl_ext_count(shipped, GLSL_LOG2),
            glsl_ext_count(&twin, GLSL_LOG2),
            glsl_ext_count(shipped, GLSL_POW),
            glsl_ext_count(&twin, GLSL_POW),
            dont_flatten_count(shipped),
            dont_flatten_count(&twin)
        );
    }
}

/// **Render wave PXt — the AgX arm's register twin, GATED by rule (xviii)** (`pxt/critique.md` W3):
/// every shipped module against itself with the AgX arm compiled out — the arm's register cost on
/// every path that does not run it (ACES Hill, Neutral, Reinhard-Jodie). Panics where it cannot
/// measure.
#[test]
#[ignore = "gpu-cap: needs a real GPU exposing VK_KHR_pipeline_executable_properties and the pinned dxc; PXt rule (xviii), the AgX arm's register twin GATED (a run that cannot measure panics); run with --nocapture"]
fn post_final_agx_arm_registers_agree() {
    let dxc = find_dxc().expect("GATE post_final_occupancy: no dxc resolves on this host — the gated row cannot measure, so it is red");
    let probe = VulkanProbe::open()
        .expect("GATE post_final_occupancy: the probe could not open a device with VK_KHR_pipeline_executable_properties (the SKIP line above says why) — the gated row cannot measure, so it is red");
    println!("\n=== post_final AgX arm register twin (PXt, GATED by rule (xviii)) ===");
    println!("device: {}", probe.device_name);
    let mut red = Vec::new();
    for (label, source, shipped) in agx_rows() {
        let twin_text = without_the_agx_arm(&source).unwrap_or_else(|why| panic!("GATE red: {label}: {why}"));
        let twin = compile_source(&dxc, &twin_text, "post_final_no_agx");
        if let Err(why) = agx_arm_anti_vacuity(shipped, &twin) {
            panic!("GATE red: {label}: {why}");
        }
        let reports = probe.measure(&[(label, shipped), ("no_agx (twin)", twin.as_slice())]);
        for r in &reports {
            println!("\n[{}] {} SPIR-V words, executable \"{}\", subgroupSize {}", r.label, r.spirv_words, r.executable_name, r.subgroup_size);
            for (name, value) in &r.stats {
                println!("    {name:.<44} {value}");
            }
        }
        assert_eq!(reports.len(), 2, "{label}: both modules must have compiled");
        for r in &reports {
            assert!(!r.stats.is_empty(), "[{}] the driver returned no statistics — the CAPTURE_STATISTICS bit did not take effect", r.label);
        }
        match registers_agree(&reports[0].stats, &reports[1].stats) {
            Ok(summary) => println!("\nGATE green ({label}): {summary}"),
            Err(why) => red.push(format!("{label}: {why}")),
        }
    }
    assert!(red.is_empty(), "GATE red: {}", red.join("; "));
}

/// **Render wave PXt — the O6 row re-aimed** (`pxt/cut.md` D-O4 (d)): the `AgxOnly` twin (the mode
/// word the AgX constant, the sharpen bit clear) is the sharpen-off module with the other three
/// arms compiled out — the dispatch folded to AgX's arm (its three `Log2` kept, no `DontFlatten`
/// left) and different bytes. SKIPS without `dxc`.
#[test]
fn the_agx_only_twin_compiles_the_other_arms_away() {
    let twin_text = emit_post_final_comp_with(PostFinalTwin::AgxOnly);
    assert!(twin_text.starts_with("// TEST-ONLY O6 TWIN (PostFinalTwin::AgxOnly)"), "the twin says what it is");
    let Some(dxc) = find_dxc() else {
        eprintln!("SKIP post_final_occupancy: no dxc resolves on this host — a skipped run is NOT a pass");
        return;
    };
    let twin = compile_twin_of(&dxc, PostFinalTwin::AgxOnly, "post_final_agx_only");
    let off = post_final_spirv_for(false);
    assert!(twin.as_slice() != off, "the AgxOnly twin's SPIR-V equals the sharpen-off module: the two columns would be one kernel");
    assert_eq!(dont_flatten_count(&twin), 0, "the AgxOnly twin keeps a DontFlatten selection: the dispatch was not folded");
    assert_eq!(glsl_ext_count(&twin, GLSL_LOG2), 3, "the AgxOnly twin must keep AgX's three Log2 (the arm it folds to)");
    eprintln!(
        "post_final_occupancy: DontFlatten sharpen-off {}, agx_only twin {}; SPIR-V words {} vs {}",
        dont_flatten_count(off),
        dont_flatten_count(&twin),
        off.len(),
        twin.len()
    );
}

/// Prints two modules' driver statistics side by side and asserts the mechanical conditions under
/// which the columns mean anything (both compiled, each reported statistics, the statistic NAMES
/// agree). RECORDED, NOT GATED — no threshold, no register count is asserted.
fn record_pair(title: &str, probe: &VulkanProbe, modules: &[(&'static str, &[u32]); 2]) {
    let reports = probe.measure(modules);
    println!("\n=== {title} (recorded, not gated) ===");
    println!("device: {}", probe.device_name);
    for r in &reports {
        println!("\n[{}] {} SPIR-V words, executable \"{}\", subgroupSize {}", r.label, r.spirv_words, r.executable_name, r.subgroup_size);
        for (name, value) in &r.stats {
            println!("    {name:.<44} {value}");
        }
    }
    assert_eq!(reports.len(), 2, "both modules must have compiled");
    for r in &reports {
        assert!(!r.stats.is_empty(), "[{}] the driver returned no statistics — the CAPTURE_STATISTICS bit did not take effect", r.label);
    }
    let names_of = |r: &ModuleReport| -> Vec<String> {
        let mut n: Vec<String> = r.stats.iter().map(|(k, _)| k.clone()).collect();
        n.sort();
        n
    };
    assert_eq!(names_of(&reports[1]), names_of(&reports[0]), "the two modules report different statistic SETS — the columns are not comparable");
}

/// **Render wave PXt — the cost when the sharpen is on** (`pxt/cut.md` D-O4 (d)): the RCAS module
/// beside the sharpen-off module. See [`record_pair`].
#[test]
#[ignore = "gpu-cap: needs a real GPU exposing VK_KHR_pipeline_executable_properties; PXt OPEN-ITEMS 19, post_final's sharpen-on and sharpen-off modules' register footprint (recorded, not gated); run with --nocapture"]
fn post_final_sharpen_on_and_off_modules_report_their_registers() {
    let Some(probe) = VulkanProbe::open() else {
        return;
    };
    record_pair(
        "post_final sharpen-on vs sharpen-off modules (PXt, OPEN-ITEMS 19)",
        &probe,
        &[("sharpen-on (post_final)", post_final_spirv_for(true)), ("sharpen-off (post_final_nosharpen)", post_final_spirv_for(false))],
    );
}

/// **Render wave PXt — the O6 row on the default curve's path** (`pxt/cut.md` D-O4 (d)): the
/// sharpen-off module beside its `AgxOnly` twin — the runtime dispatch's register cost where AgX
/// runs. The PX1a `AcesOnly` row stays for continuity. See [`record_pair`].
#[test]
#[ignore = "gpu-cap: needs a real GPU exposing VK_KHR_pipeline_executable_properties and the pinned dxc; PXt O6, post_final's sharpen-off module vs its AgxOnly twin (recorded, not gated); run with --nocapture"]
fn post_final_sharpen_off_and_agx_only_twin_report_their_register_footprint() {
    let Some(dxc) = find_dxc() else {
        eprintln!("SKIP post_final_occupancy: no dxc resolves on this host — a skipped run is NOT a pass");
        return;
    };
    let twin = compile_twin_of(&dxc, PostFinalTwin::AgxOnly, "post_final_agx_only");
    let Some(probe) = VulkanProbe::open() else {
        return;
    };
    record_pair(
        "post_final O6 on the default curve: sharpen-off module vs AgxOnly twin (PXt)",
        &probe,
        &[("sharpen-off (committed)", post_final_spirv_for(false)), ("agx_only (twin)", twin.as_slice())],
    );
}

/// The twin is the committed body with the dispatch compiled out: the generator prints the same
/// file but for the mode word (and a test-only header line), and DXC's output differs from the
/// committed module and carries fewer `DontFlatten` selections. SKIPS without `dxc`.
#[test]
fn the_aces_only_twin_compiles_the_dispatch_away() {
    let uber_text = emit_post_final_comp_with(PostFinalTwin::Uber);
    assert_eq!(uber_text, boyko_shaderdsl::emit::emit_post_final_comp(), "the Uber twin is the committed template");
    let twin_text = emit_post_final_comp_with(PostFinalTwin::AcesOnly);
    assert_ne!(twin_text, uber_text, "the AcesOnly twin must differ from the committed template");
    assert!(twin_text.starts_with("// TEST-ONLY O6 TWIN"), "the twin says what it is");
    let Some(dxc) = find_dxc() else {
        eprintln!("SKIP post_final_occupancy: no dxc resolves on this host — a skipped run is NOT a pass");
        return;
    };
    let twin = compile_twin(&dxc);
    let uber = post_final_spirv();
    assert!(twin.as_slice() != uber, "the twin's SPIR-V equals the committed module: the two columns would be one kernel");
    let (u, t) = (dont_flatten_count(uber), dont_flatten_count(&twin));
    assert!(u > 0, "the committed post_final carries no DontFlatten selection: the [branch] dispatch is not there");
    assert!(t < u, "the twin keeps {t} DontFlatten selections against the uber module's {u}: the dispatch was not compiled out");
    eprintln!("post_final_occupancy: DontFlatten selections uber {u}, aces_only twin {t}; SPIR-V words {} vs {}", uber.len(), twin.len());
}

/// **The O6 row** — both modules' driver statistics, side by side. See the module doc.
#[test]
#[ignore = "gpu-cap: needs a real GPU exposing VK_KHR_pipeline_executable_properties and the pinned dxc; PX1a O6, post_final's occupancy row (recorded, not gated); run with --nocapture"]
fn post_final_uber_and_aces_only_twin_report_their_register_footprint() {
    let Some(dxc) = find_dxc() else {
        eprintln!("SKIP post_final_occupancy: no dxc resolves on this host — a skipped run is NOT a pass");
        return;
    };
    let twin = compile_twin(&dxc);
    let Some(probe) = VulkanProbe::open() else {
        return;
    };
    let reports = probe.measure(&[("uber (committed)", post_final_spirv()), ("aces_only (twin)", twin.as_slice())]);

    println!("\n=== post_final O6 occupancy / register report (recorded, not gated) ===");
    println!("device: {}", probe.device_name);
    for r in &reports {
        println!(
            "\n[{}] {} SPIR-V words, executable \"{}\", subgroupSize {}",
            r.label, r.spirv_words, r.executable_name, r.subgroup_size
        );
        for (name, value) in &r.stats {
            println!("    {name:.<44} {value}");
        }
    }

    assert_eq!(reports.len(), 2, "both modules must have compiled");
    for r in &reports {
        assert!(
            !r.stats.is_empty(),
            "[{}] the driver returned no statistics — the CAPTURE_STATISTICS bit did not take effect",
            r.label
        );
    }
    let names_of = |r: &ModuleReport| -> Vec<String> {
        let mut n: Vec<String> = r.stats.iter().map(|(k, _)| k.clone()).collect();
        n.sort();
        n
    };
    assert_eq!(
        names_of(&reports[1]),
        names_of(&reports[0]),
        "the twin reports a different statistic SET than the uber module — the two columns are not comparable"
    );
    println!("\nO6: the row is the difference of the two columns above (H4: > 5 % ⇒ spec-constant stripping, decided at the WR row).");
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
            eprintln!("SKIP post_final_occupancy: vulkan-1.dll did not load");
            return None;
        }
        // SAFETY: `module` is the live HMODULE just returned by `LoadLibraryA`, and the symbol name
        // is a static NUL-terminated literal. A null result is checked before any call.
        let gipa_raw = unsafe { os::GetProcAddress(module, c"vkGetInstanceProcAddr".as_ptr()) };
        if gipa_raw.is_null() {
            eprintln!("SKIP post_final_occupancy: vkGetInstanceProcAddr not exported");
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
            p_application_name: c"boyko post_final occupancy probe".as_ptr(),
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
            eprintln!("SKIP post_final_occupancy: vkCreateInstance failed ({r})");
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
                "SKIP post_final_occupancy: no physical device exposes \
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
            eprintln!("SKIP post_final_occupancy: vkCreateDevice failed ({r})");
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
    fn measure(&self, modules: &[(&'static str, &[u32])]) -> Vec<ModuleReport> {
        // One layout serves both modules: the twin is the committed body with a constant mode word,
        // so its interface (set 0, the 4-byte push range) is the uber module's.
        let bindings: Vec<VkDescriptorSetLayoutBinding> = POST_FINAL_LAYOUT
            .iter()
            .map(|&(b, ty)| VkDescriptorSetLayoutBinding {
                binding: b,
                descriptor_type: ty,
                descriptor_count: 1,
                stage_flags: VK_SHADER_STAGE_COMPUTE_BIT,
                p_immutable_samplers: ptr::null(),
            })
            .collect();
        let dslci = VkDescriptorSetLayoutCreateInfo {
            s_type: ST_DESCRIPTOR_SET_LAYOUT_CREATE_INFO,
            p_next: ptr::null(),
            flags: 0,
            binding_count: POST_FINAL_LAYOUT.len() as u32,
            p_bindings: bindings.as_ptr(),
        };
        let mut set_layout: VkNonDispatchable = 0;
        // SAFETY: `bindings` outlives the call and holds exactly `POST_FINAL_LAYOUT.len()` entries.
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
            size: POST_FINAL_PUSH_BYTES,
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

        let mut reports = Vec::with_capacity(modules.len());
        for &(label, spirv) in modules {
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

// `status_within` / `output_within` and `scratch_path` live in `tests/child_guard/mod.rs`.
mod child_guard;
use child_guard::{BoundedRun, DXC_DEADLINE};
