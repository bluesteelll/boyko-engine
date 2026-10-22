//! OPEN-ITEMS 16 — the M2 first-crossing certificate's **register twin** (plan rule (xviii);
//! `oi1516/cut.md` D16-8 / C8; orchestrator ruling 93 Q8).
//!
//! `sdf_m2_certify.hlsli` adds runtime-gated code to two lit producers: the deferred marcher and the
//! forward march run the certificate only on an M2-accepted ray, and the shipped app threads M2 OFF
//! (`brick: None`), so the body is dead at run time — but it is compiled in, and its live ranges can
//! still raise the kernel's register footprint. The twin is NOT committed: this binary DXCs each
//! producer at test time under its frozen recipe plus `-D M2_CERTIFY=0`, which compiles the
//! certificate to `return true`.
//!
//! - `the_m2_certify_twin_compiles_the_certificate_away` (host; SKIPS with an `eprintln` when no
//!   `dxc` resolves — a skipped run is not a pass): the anti-vacuity — every twin's bytes differ from
//!   the committed row's, and it carries FEWER `OpLoopMerge` instructions (the certificate's two
//!   loops — the subdivision driver and the edit-range fold — are its named marker ops; its constants
//!   collide with other literals, critique O1), so the two columns below measure two different
//!   kernels. Red-first: a twin built WITHOUT the define equals the committed bytes.
//! - `marchers_and_their_m2_certify_twins_report_their_register_footprint` (`gpu-cap:`): each
//!   producer's and its twin's `VK_KHR_pipeline_executable_properties` statistics, printed side by
//!   side, checked per rule (xviii): the statistic names agree, the register delta is at most 8 and
//!   the occupancy is equal where the driver reports it, on the forward march's default row
//!   (`HAS_MESH`) and its heaviest (`HAS_MESH + VIEWT`); the deferred marcher's row is recorded. No
//!   absolute count is asserted.
//!
//! The headless probe below (own instance, own device with the one extension, no surface, no
//! submission) is `post_final_occupancy.rs`'s, with each pipeline's set-0 layout REFLECTED from its
//! SPIR-V (the marchers' layouts are wide, and a hand-copied table would rot).
//!
//! # Usage
//!
//! ```text
//! cargo test -p boyko_rhi_vulkan --test marcher_occupancy --locked -- --ignored --test-threads=1 --nocapture
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

use boyko_rhi_vulkan::compute::{
    sdf_forward_march_spirv, sdf_forward_march_viewt_spirv, sdf_gbuffer_composite_spirv,
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
/// `VK_DESCRIPTOR_TYPE_SAMPLER`.
const VK_DESCRIPTOR_TYPE_SAMPLER: i32 = 0;
/// `VK_DESCRIPTOR_TYPE_COMBINED_IMAGE_SAMPLER`.
const VK_DESCRIPTOR_TYPE_COMBINED_IMAGE_SAMPLER: i32 = 1;
/// `VK_DESCRIPTOR_TYPE_SAMPLED_IMAGE`.
const VK_DESCRIPTOR_TYPE_SAMPLED_IMAGE: i32 = 2;
/// `VK_DESCRIPTOR_TYPE_STORAGE_IMAGE`.
const VK_DESCRIPTOR_TYPE_STORAGE_IMAGE: i32 = 3;
/// `VK_DESCRIPTOR_TYPE_UNIFORM_BUFFER`.
const VK_DESCRIPTOR_TYPE_UNIFORM_BUFFER: i32 = 6;
/// `VK_DESCRIPTOR_TYPE_STORAGE_BUFFER`.
const VK_DESCRIPTOR_TYPE_STORAGE_BUFFER: i32 = 7;
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

impl StatValue {
    /// The value as an `f64` (a bool as 0 / 1) — the rule-(xviii) comparison's unit.
    fn as_f64(&self) -> f64 {
        match *self {
            StatValue::Bool(b) => f64::from(u8::from(b)),
            StatValue::I64(v) => v as f64,
            StatValue::U64(v) => v as f64,
            StatValue::F64(v) => v,
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

/// The push range the probe's layouts declare: wider than any marcher's push block (80 B deferred,
/// 40 B forward) — a layout's range may exceed what the shader declares.
const MARCHER_PUSH_BYTES: u32 = 128;

/// One producer row of the twin: its source, its recipe's `-D` defines, the committed module.
struct Row {
    label: &'static str,
    hlsl: &'static str,
    defines: &'static [&'static str],
    committed: fn() -> &'static [u32],
    /// Rule (xviii)'s register / occupancy check applies (the forward march's default and heaviest
    /// rows); else the row is recorded.
    gated: bool,
}

/// The affected producers: the forward march's default row and its heaviest, the deferred marcher.
const ROWS: [Row; 3] = [
    Row {
        label: "sdf_forward_march (HAS_MESH, default)",
        hlsl: "sdf_forward_march.comp.hlsl",
        defines: &["HAS_MESH=1"],
        committed: sdf_forward_march_spirv,
        gated: true,
    },
    Row {
        label: "sdf_forward_march_viewt (HAS_MESH + VIEWT, heaviest)",
        hlsl: "sdf_forward_march.comp.hlsl",
        defines: &["HAS_MESH=1", "VIEWT=1"],
        committed: sdf_forward_march_viewt_spirv,
        gated: true,
    },
    Row {
        label: "sdf_gbuffer_composite (deferred, recorded)",
        hlsl: "sdf_gbuffer_composite.hlsl",
        defines: &[],
        committed: sdf_gbuffer_composite_spirv,
        gated: false,
    },
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

/// `row`'s source DXC'd under its frozen recipe (`-spirv -T cs_6_0 -E main
/// -fspv-target-env=vulkan1.3`, the row's defines) plus `extra` defines, from the shader directory
/// (so its `#include`s resolve), into a scratch file.
fn compile(dxc: &PathBuf, row: &Row, extra: &[&str], tag: &str) -> Vec<u32> {
    let dir = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/shaders"));
    let spv = child_guard::scratch_path(&format!("{tag}.twin.spv"));
    let mut cmd = Command::new(dxc);
    cmd.current_dir(&dir).args(["-spirv", "-T", "cs_6_0", "-E", "main"]);
    for d in row.defines.iter().chain(extra) {
        cmd.arg("-D").arg(d);
    }
    let status = cmd
        .arg("-fspv-target-env=vulkan1.3")
        .arg(row.hlsl)
        .arg("-Fo")
        .arg(&spv)
        .status_within(DXC_DEADLINE)
        .expect("invariant: dxc was located and must run");
    assert!(status.success(), "dxc failed on {} {:?} + {extra:?}", row.hlsl, row.defines);
    let bytes = std::fs::read(&spv).expect("invariant: dxc wrote the twin's .spv");
    let _ = std::fs::remove_file(&spv);
    let (words, rest) = bytes.as_chunks::<4>();
    assert!(rest.is_empty(), "a SPIR-V module is a whole number of words");
    words.iter().map(|w| u32::from_le_bytes(*w)).collect()
}

/// The `OpLoopMerge` (opcode 246) instructions in a SPIR-V module — one per structured loop.
fn loop_merge_count(words: &[u32]) -> usize {
    let mut i = 5; // past the header
    let mut n = 0;
    while i < words.len() {
        let (count, op) = ((words[i] >> 16) as usize, words[i] & 0xFFFF);
        assert!(count > 0, "a zero-length SPIR-V instruction at word {i}");
        if op == 246 {
            n += 1;
        }
        i += count;
    }
    n
}

/// Set 0 of a compute module, reflected from its SPIR-V: `(binding, VkDescriptorType)` for every
/// `UniformConstant` / `Uniform` / `StorageBuffer` variable decorated `DescriptorSet 0` (an image
/// and a sampler sharing a binding are one combined image sampler — DXC's `t<N>` + `s<N>` pair).
fn reflect_set0(words: &[u32]) -> Vec<(u32, i32)> {
    use std::collections::BTreeMap;
    let mut binding: BTreeMap<u32, u32> = BTreeMap::new();
    let mut set: BTreeMap<u32, u32> = BTreeMap::new();
    let mut block: BTreeMap<u32, u32> = BTreeMap::new(); // struct id -> 2 Block / 3 BufferBlock
    let mut ptr_to: BTreeMap<u32, (u32, u32)> = BTreeMap::new(); // ptr type -> (storage, pointee)
    let mut kind: BTreeMap<u32, i32> = BTreeMap::new(); // type id -> descriptor class of that type
    let mut array_of: BTreeMap<u32, u32> = BTreeMap::new(); // array type -> element type
    let mut vars: Vec<(u32, u32, u32)> = Vec::new(); // (var, ptr type, storage)
    let mut i = 5;
    while i < words.len() {
        let (count, op) = ((words[i] >> 16) as usize, words[i] & 0xFFFF);
        assert!(count > 0, "a zero-length SPIR-V instruction at word {i}");
        let w = &words[i..i + count];
        match op {
            71 if w[2] == 33 => {
                binding.insert(w[1], w[3]);
            }
            71 if w[2] == 34 => {
                set.insert(w[1], w[3]);
            }
            71 if w[2] == 2 || w[2] == 3 => {
                block.insert(w[1], w[2]);
            }
            // OpTypeImage: Sampled operand (word 7) 1 = sampled, 2 = storage.
            25 => {
                kind.insert(w[1], if w[7] == 2 { VK_DESCRIPTOR_TYPE_STORAGE_IMAGE } else { VK_DESCRIPTOR_TYPE_SAMPLED_IMAGE });
            }
            26 => {
                kind.insert(w[1], VK_DESCRIPTOR_TYPE_SAMPLER);
            }
            27 => {
                kind.insert(w[1], VK_DESCRIPTOR_TYPE_COMBINED_IMAGE_SAMPLER);
            }
            28 | 29 => {
                array_of.insert(w[1], w[2]);
            }
            32 => {
                ptr_to.insert(w[1], (w[2], w[3]));
            }
            59 => vars.push((w[2], w[1], w[3])),
            _ => {}
        }
        i += count;
    }
    let mut out: BTreeMap<u32, i32> = BTreeMap::new();
    for (var, ptr_ty, storage) in vars {
        if set.get(&var) != Some(&0) {
            continue;
        }
        let Some(&b) = binding.get(&var) else { continue };
        let (_, mut pointee) = ptr_to[&ptr_ty];
        while let Some(&e) = array_of.get(&pointee) {
            pointee = e;
        }
        let ty = match storage {
            0 => kind[&pointee],
            2 => {
                if block.get(&pointee) == Some(&3) { VK_DESCRIPTOR_TYPE_STORAGE_BUFFER } else { VK_DESCRIPTOR_TYPE_UNIFORM_BUFFER }
            }
            12 => VK_DESCRIPTOR_TYPE_STORAGE_BUFFER,
            _ => continue,
        };
        let merged = match out.get(&b) {
            Some(&prev) if prev != ty => VK_DESCRIPTOR_TYPE_COMBINED_IMAGE_SAMPLER,
            _ => ty,
        };
        out.insert(b, merged);
    }
    out.into_iter().collect()
}

/// Every twin is the committed row with the certificate compiled out: its bytes differ, and it
/// carries fewer `OpLoopMerge` (the certificate's two loops). A twin built WITHOUT the define is the
/// committed row byte for byte — the red-first of the anti-vacuity, run here as its twin. SKIPS
/// without `dxc`.
#[test]
fn the_m2_certify_twin_compiles_the_certificate_away() {
    let Some(dxc) = find_dxc() else {
        eprintln!("SKIP marcher_occupancy: no dxc resolves on this host — a skipped run is NOT a pass");
        return;
    };
    for (k, row) in ROWS.iter().enumerate() {
        let committed = (row.committed)();
        let plain = compile(&dxc, row, &[], &format!("row{k}_plain"));
        assert!(
            plain.as_slice() == committed,
            "[{}] the recipe WITHOUT M2_CERTIFY=0 does not reproduce the committed module: the twin \
             would not be the committed row with one body out",
            row.label
        );
        let twin = compile(&dxc, row, &["M2_CERTIFY=0"], &format!("row{k}_twin"));
        assert!(twin.as_slice() != committed, "[{}] the twin equals the committed module", row.label);
        let (c, t) = (loop_merge_count(committed), loop_merge_count(&twin));
        assert!(
            t < c,
            "[{}] the twin keeps {t} OpLoopMerge against the committed {c}: the certificate's loops were not \
             compiled out",
            row.label
        );
        eprintln!(
            "marcher_occupancy [{}]: OpLoopMerge committed {c}, twin {t}; SPIR-V words {} vs {}",
            row.label,
            committed.len(),
            twin.len()
        );
    }
}

/// **Rule (xviii)'s register twin** — every row's and its twin's driver statistics, side by side.
/// See the module doc.
#[test]
#[ignore = "gpu-cap: needs a real GPU exposing VK_KHR_pipeline_executable_properties and the pinned dxc; OI16 rule (xviii) register twin of the M2 certificate; run with --nocapture"]
fn marchers_and_their_m2_certify_twins_report_their_register_footprint() {
    let Some(dxc) = find_dxc() else {
        eprintln!("SKIP marcher_occupancy: no dxc resolves on this host — a skipped run is NOT a pass");
        return;
    };
    let Some(probe) = VulkanProbe::open() else {
        return;
    };
    println!("\n=== OI16 rule (xviii): marcher register twins (M2_CERTIFY = 0) ===");
    println!("device: {}", probe.device_name);
    for (k, row) in ROWS.iter().enumerate() {
        let committed = (row.committed)();
        let twin = compile(&dxc, row, &["M2_CERTIFY=0"], &format!("row{k}_cap"));
        let layout = reflect_set0(committed);
        assert!(!layout.is_empty(), "[{}] set 0 reflected empty", row.label);
        let reports = probe.measure(&layout, &[("committed", committed), ("twin", twin.as_slice())]);
        println!("\n[{}] set 0: {} bindings", row.label, layout.len());
        for r in &reports {
            println!(
                "  [{}] {} SPIR-V words, executable \"{}\", subgroupSize {}",
                r.label, r.spirv_words, r.executable_name, r.subgroup_size
            );
            for (name, value) in &r.stats {
                println!("      {name:.<44} {value}");
            }
        }
        assert_eq!(reports.len(), 2, "[{}] both modules must have compiled", row.label);
        for r in &reports {
            assert!(!r.stats.is_empty(), "[{}/{}] the driver returned no statistics", row.label, r.label);
        }
        let names_of = |r: &ModuleReport| -> Vec<String> {
            let mut n: Vec<String> = r.stats.iter().map(|(k, _)| k.clone()).collect();
            n.sort();
            n
        };
        assert_eq!(names_of(&reports[1]), names_of(&reports[0]), "[{}] the statistic SETS differ", row.label);
        let stat = |r: &ModuleReport, needle: &str| -> Option<f64> {
            r.stats.iter().find(|(n, _)| n.to_lowercase().contains(needle)).map(|(_, v)| v.as_f64())
        };
        let regs = (stat(&reports[0], "register"), stat(&reports[1], "register"));
        let occ = (stat(&reports[0], "occupancy"), stat(&reports[1], "occupancy"));
        println!("  rule (xviii): registers committed {:?} vs twin {:?}; occupancy {:?} vs {:?}", regs.0, regs.1, occ.0, occ.1);
        if row.gated {
            if let (Some(c), Some(t)) = regs {
                assert!(c - t <= 8.0, "[{}] the certificate costs {} registers (> 8, rule (xviii))", row.label, c - t);
            }
            if let (Some(c), Some(t)) = occ {
                assert_eq!(c, t, "[{}] the certificate changes the reported occupancy (rule (xviii))", row.label);
            }
        }
    }
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
            eprintln!("SKIP marcher_occupancy: vulkan-1.dll did not load");
            return None;
        }
        // SAFETY: `module` is the live HMODULE just returned by `LoadLibraryA`, and the symbol name
        // is a static NUL-terminated literal. A null result is checked before any call.
        let gipa_raw = unsafe { os::GetProcAddress(module, c"vkGetInstanceProcAddr".as_ptr()) };
        if gipa_raw.is_null() {
            eprintln!("SKIP marcher_occupancy: vkGetInstanceProcAddr not exported");
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
            p_application_name: c"boyko marcher occupancy probe".as_ptr(),
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
            eprintln!("SKIP marcher_occupancy: vkCreateInstance failed ({r})");
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
                "SKIP marcher_occupancy: no physical device exposes \
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
            eprintln!("SKIP marcher_occupancy: vkCreateDevice failed ({r})");
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
    fn measure(&self, layout_table: &[(u32, i32)], modules: &[(&'static str, &[u32])]) -> Vec<ModuleReport> {
        // One layout serves both modules: the twin is the committed body with the certificate out,
        // so its interface is a subset of the committed module's (reflected by the caller).
        let bindings: Vec<VkDescriptorSetLayoutBinding> = layout_table
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
            binding_count: layout_table.len() as u32,
            p_bindings: bindings.as_ptr(),
        };
        let mut set_layout: VkNonDispatchable = 0;
        // SAFETY: `bindings` outlives the call and holds exactly `layout_table.len()` entries.
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
            size: MARCHER_PUSH_BYTES,
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
