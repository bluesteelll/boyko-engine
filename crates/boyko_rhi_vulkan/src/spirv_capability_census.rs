//! **Gate B — every SPIR-V capability a committed shader declares is licensed on the device.**
//!
//! A shader module that declares a capability whose device feature was never enabled is invalid
//! (`VUID-VkShaderModuleCreateInfo-pCode-08740`) — and the validation layer reports it only when
//! the module is created on a validated boot, which is how `Geometry` (`vb_raster.fs`) and
//! `DemoteToHelperInvocation` (`smaa_edge.fs`) shipped on every boot unnoticed. This census needs
//! no device: it walks every `crates/*/shaders/*.spv`, reads each `OpCapability`, and requires
//! each declared capability to have a row saying how it is licensed:
//!
//! * [`Requirement::Always`] — the Vulkan SPIR-V environment table names no feature for it, or
//!   core Vulkan 1.3 satisfies it.
//! * [`Requirement::Required`] — a device feature that MUST be a row of
//!   [`REQUIRED_CORE`] / [`REQUIRED_V13`], the tables `create_device` queries, enables and refuses
//!   by. Cross-checked here, so the census and the device read one source.
//! * [`Requirement::SubgroupOperation`] — a bit of `subgroupSupportedOperations`, a device
//!   PROPERTY with no enable bit. It MUST be a row of [`REQUIRED_SUBGROUP_OPERATIONS`], the table
//!   `create_device` checks before `vkCreateDevice`. Cross-checked here. Because the property also
//!   has a stage dimension (`subgroupSupportedStages`, `VUID-RuntimeSpirv-None-06343`), every
//!   entry point of every module declaring such a capability must run in a stage inside
//!   [`REQUIRED_SUBGROUP_STAGES`].
//! * [`Requirement::Conditional`] — a feature enabled outside those tables, behind a gate. The
//!   gate is RECORDED, not verified.
//!
//! It also fails on a stale row (a capability no committed module declares — the waiver-list
//! staleness rule), on a malformed module (named), and on a vacuous walk. The two capabilities
//! that motivated it are pinned to their files, which also proves they were fixed on the device
//! side rather than by recompiling the shaders. Every `GroupNonUniform*` row is pinned to the
//! `SubgroupOperation` bit the environment table names for it, because row 61's class is
//! otherwise unobservable: every module declaring it also declares row 64.
//!
//! # What it cannot check
//!
//! The descriptor-indexing rows (`RuntimeDescriptorArray`, `SampledImageArrayNonUniformIndexing`)
//! are `Conditional`: their bits are enabled in `create_device` outside the `REQUIRED_*` tables
//! and refused through `DeviceCaps::bindless_capable`, so the census records that gate rather than
//! reading it. The two bits `DeviceEnables::enable_vb_geometry_table` enables
//! (`shaderStorageBufferArrayNonUniformIndexing`, `descriptorBindingStorageBufferUpdateAfterBind`)
//! correspond to no capability any committed module declares (`StorageBufferArrayNonUniformIndexing`
//! appears nowhere; the second is a binding flag, not a capability), so they are outside what a
//! capability census can see at all.
//!
//! Capability ids are the SPIR-V unified grammar's (`spirv.core.grammar.json`).

use std::path::{Path, PathBuf};

use crate::device::{REQUIRED_CORE, REQUIRED_SUBGROUP_OPERATIONS, REQUIRED_SUBGROUP_STAGES, REQUIRED_V13};
use crate::ffi::{VK_SHADER_STAGE_COMPUTE_BIT, VK_SHADER_STAGE_FRAGMENT_BIT, VK_SHADER_STAGE_VERTEX_BIT, VkFlags};

/// The SPIR-V module magic number, in the module's own (little-endian) word order.
const SPIRV_MAGIC: u32 = 0x0723_0203;
/// Words in the SPIR-V module header (magic, version, generator, bound, schema).
const HEADER_WORDS: usize = 5;
/// `OpCapability`'s opcode.
const OP_CAPABILITY: u32 = 17;
/// `OpEntryPoint`'s opcode.
const OP_ENTRY_POINT: u32 = 15;
/// `OpEntryPoint`'s minimum word count: the opcode word, the ExecutionModel, the entry-point id
/// and at least one word of the NUL-terminated name.
const ENTRY_POINT_MIN_WORDS: usize = 4;

/// SPIR-V `ExecutionModel` `Vertex`.
const EXECUTION_MODEL_VERTEX: u32 = 0;
/// SPIR-V `ExecutionModel` `Fragment`.
const EXECUTION_MODEL_FRAGMENT: u32 = 4;
/// SPIR-V `ExecutionModel` `GLCompute`.
const EXECUTION_MODEL_GL_COMPUTE: u32 = 5;

/// Floor on the committed modules the walk must find (117 as of 2026-09-18).
const MIN_MODULES: usize = 117;
/// Floor on the crates the walk must find shaders in (`boyko_rhi_vulkan`, `boyko_render`).
const MIN_CRATES: usize = 2;

/// `Shader` — every graphics/compute module declares it.
const CAP_SHADER: u32 = 1;
/// `Geometry` — declared by DXC for any `SV_PrimitiveID` read.
const CAP_GEOMETRY: u32 = 2;
/// `DemoteToHelperInvocation` — DXC's lowering of `discard` under `-fspv-target-env=vulkan1.3`.
const CAP_DEMOTE_TO_HELPER_INVOCATION: u32 = 5379;
/// `GroupNonUniform` — DXC's lowering of `WaveIsFirstLane` (`OpGroupNonUniformElect`), and
/// declared alongside every other `GroupNonUniform*` capability.
const CAP_GROUP_NON_UNIFORM: u32 = 61;
/// `GroupNonUniformBallot` — DXC's lowering of `WaveActiveCountBits` / `WavePrefixCountBits` /
/// `WaveReadLaneFirst`.
const CAP_GROUP_NON_UNIFORM_BALLOT: u32 = 64;

/// How a declared capability is licensed on every device this engine boots.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Requirement {
    /// No feature to enable: the environment table's field is blank, or core 1.3 satisfies it.
    Always,
    /// The named device feature, which must be a row of `REQUIRED_CORE` / `REQUIRED_V13`.
    Required(&'static str),
    /// The named `VkSubgroupFeatureFlagBits` bit of `subgroupSupportedOperations`, which must be a
    /// row of `REQUIRED_SUBGROUP_OPERATIONS`; every declarer must run only in
    /// `REQUIRED_SUBGROUP_STAGES`.
    SubgroupOperation(&'static str),
    /// The named device feature, enabled only behind `gate` — recorded, not verified.
    Conditional {
        feature: &'static str,
        gate: &'static str,
    },
}

/// One census row: a capability id, its grammar name, and its license.
struct CensusRow {
    id: u32,
    name: &'static str,
    requirement: Requirement,
}

/// The gate for the descriptor-indexing rows: `create_device` enables these bits on every boot,
/// outside `REQUIRED_*`, and `boot_with_instance` refuses a device that lacks them.
const BINDLESS_GATE: &str = "create_device enables it on every boot; a device without it is \
                             refused (DeviceCaps::bindless_capable -> BootError::BindlessUnsupported)";

/// Every capability the committed modules declare, built from a measurement of the tree
/// (2026-09-18) and the Vulkan SPIR-V environment table, one row per capability.
const CENSUS: &[CensusRow] = &[
    CensusRow { id: CAP_SHADER, name: "Shader", requirement: Requirement::Always },
    CensusRow {
        id: CAP_GEOMETRY,
        name: "Geometry",
        requirement: Requirement::Required("geometryShader"),
    },
    CensusRow { id: 49, name: "StorageImageExtendedFormats", requirement: Requirement::Always },
    // Core Vulkan guarantees BASIC (with COMPUTE in `subgroupSupportedStages`) on any device with
    // a graphics or compute queue, so the device row can refuse nothing the engine could boot;
    // the row exists so the stage restriction that guarantee carries is checked, not assumed.
    CensusRow {
        id: CAP_GROUP_NON_UNIFORM,
        name: "GroupNonUniform",
        requirement: Requirement::SubgroupOperation("VK_SUBGROUP_FEATURE_BASIC_BIT"),
    },
    // `particle_sim.comp`'s wave aggregation (`WaveActiveCountBits`, `WavePrefixCountBits`,
    // `WaveReadLaneFirst`). Not guaranteed by core Vulkan; `create_device` refuses a device
    // without it.
    CensusRow {
        id: CAP_GROUP_NON_UNIFORM_BALLOT,
        name: "GroupNonUniformBallot",
        requirement: Requirement::SubgroupOperation("VK_SUBGROUP_FEATURE_BALLOT_BIT"),
    },
    CensusRow {
        id: 4472,
        name: "RayQueryKHR",
        requirement: Requirement::Conditional {
            feature: "rayQuery",
            gate: "DeviceEnables::enable_ray_query (feature `hwrt` AND supports_ray_query)",
        },
    },
    CensusRow { id: 5301, name: "ShaderNonUniform", requirement: Requirement::Always },
    CensusRow {
        id: 5302,
        name: "RuntimeDescriptorArray",
        requirement: Requirement::Conditional { feature: "runtimeDescriptorArray", gate: BINDLESS_GATE },
    },
    CensusRow {
        id: 5307,
        name: "SampledImageArrayNonUniformIndexing",
        requirement: Requirement::Conditional {
            feature: "shaderSampledImageArrayNonUniformIndexing",
            gate: BINDLESS_GATE,
        },
    },
    CensusRow {
        id: CAP_DEMOTE_TO_HELPER_INVOCATION,
        name: "DemoteToHelperInvocation",
        requirement: Requirement::Required("shaderDemoteToHelperInvocation"),
    },
];

/// The Vulkan SPIR-V environment table's license for each core `GroupNonUniform*` capability: the
/// `VkSubgroupFeatureFlagBits` bit of `subgroupSupportedOperations` that enables it. A census row
/// for any of these ids must be exactly `SubgroupOperation(<that bit>)` (test (e)).
const SUBGROUP_CAPABILITY_BITS: &[(u32, &str)] = &[
    (CAP_GROUP_NON_UNIFORM, "VK_SUBGROUP_FEATURE_BASIC_BIT"),
    (62, "VK_SUBGROUP_FEATURE_VOTE_BIT"),
    (63, "VK_SUBGROUP_FEATURE_ARITHMETIC_BIT"),
    (CAP_GROUP_NON_UNIFORM_BALLOT, "VK_SUBGROUP_FEATURE_BALLOT_BIT"),
    (65, "VK_SUBGROUP_FEATURE_SHUFFLE_BIT"),
    (66, "VK_SUBGROUP_FEATURE_SHUFFLE_RELATIVE_BIT"),
    (67, "VK_SUBGROUP_FEATURE_CLUSTERED_BIT"),
    (68, "VK_SUBGROUP_FEATURE_QUAD_BIT"),
];

/// Grammar names for capability ids a DXC-for-Vulkan module can plausibly declare — used ONLY to
/// name an id that has no census row in a failure message. Not a license of any kind.
const CAPABILITY_NAMES: &[(u32, &str)] = &[
    (1, "Shader"),
    (2, "Geometry"),
    (3, "Tessellation"),
    (9, "Float16"),
    (10, "Float64"),
    (11, "Int64"),
    (12, "Int64Atomics"),
    (22, "Int16"),
    (25, "ImageGatherExtended"),
    (32, "ClipDistance"),
    (33, "CullDistance"),
    (34, "ImageCubeArray"),
    (35, "SampleRateShading"),
    (42, "MinLod"),
    (49, "StorageImageExtendedFormats"),
    (50, "ImageQuery"),
    (51, "DerivativeControl"),
    (55, "StorageImageReadWithoutFormat"),
    (56, "StorageImageWriteWithoutFormat"),
    (57, "MultiViewport"),
    (61, "GroupNonUniform"),
    (62, "GroupNonUniformVote"),
    (63, "GroupNonUniformArithmetic"),
    (64, "GroupNonUniformBallot"),
    (65, "GroupNonUniformShuffle"),
    (66, "GroupNonUniformShuffleRelative"),
    (67, "GroupNonUniformClustered"),
    (68, "GroupNonUniformQuad"),
    (4427, "DrawParameters"),
    (4472, "RayQueryKHR"),
    (4479, "RayTracingKHR"),
    (5301, "ShaderNonUniform"),
    (5302, "RuntimeDescriptorArray"),
    (5306, "UniformBufferArrayNonUniformIndexing"),
    (5307, "SampledImageArrayNonUniformIndexing"),
    (5308, "StorageBufferArrayNonUniformIndexing"),
    (5309, "StorageImageArrayNonUniformIndexing"),
    (5345, "VulkanMemoryModel"),
    (5347, "PhysicalStorageBufferAddresses"),
    (5379, "DemoteToHelperInvocation"),
];

/// What the census reads from one module: its `OpCapability` operands and the ExecutionModel of
/// each `OpEntryPoint`, both in declaration order.
#[derive(Debug, PartialEq, Eq)]
struct ParsedModule {
    capabilities: Vec<u32>,
    execution_models: Vec<u32>,
}

/// One committed module and what it declares.
struct Module {
    /// The crate directory the module lives under (`crates/<crate>/shaders/`).
    crate_dir: String,
    /// The file name, e.g. `vb_raster.fs.spv`.
    file: String,
    capabilities: Vec<u32>,
    /// The ExecutionModel of every entry point.
    execution_models: Vec<u32>,
}

/// The `VkShaderStageFlagBits` an ExecutionModel runs in, or `None` for a model this census does
/// not map (which the stage check treats as outside every allowed mask — it fails closed).
const fn stage_bit_of(execution_model: u32) -> Option<VkFlags> {
    match execution_model {
        EXECUTION_MODEL_VERTEX => Some(VK_SHADER_STAGE_VERTEX_BIT),
        EXECUTION_MODEL_FRAGMENT => Some(VK_SHADER_STAGE_FRAGMENT_BIT),
        EXECUTION_MODEL_GL_COMPUTE => Some(VK_SHADER_STAGE_COMPUTE_BIT),
        _ => None,
    }
}

/// The grammar name of an ExecutionModel, for a message.
const fn execution_model_name(execution_model: u32) -> &'static str {
    match execution_model {
        EXECUTION_MODEL_VERTEX => "Vertex",
        EXECUTION_MODEL_FRAGMENT => "Fragment",
        EXECUTION_MODEL_GL_COMPUTE => "GLCompute",
        _ => "an execution model this census does not map",
    }
}

/// The grammar name of capability `id`, for a message.
fn capability_name(id: u32) -> &'static str {
    CENSUS
        .iter()
        .find(|row| row.id == id)
        .map(|row| row.name)
        .or_else(|| CAPABILITY_NAMES.iter().find(|(i, _)| *i == id).map(|(_, n)| *n))
        .unwrap_or("not in this census's name list")
}

/// Every `OpCapability` operand and every `OpEntryPoint` ExecutionModel in the module `bytes`, in
/// declaration order.
///
/// Walks every instruction, not just the prologue, so a zero word count or an instruction that
/// overruns the module anywhere is reported. `label` names the module in the error.
fn parse_module(label: &str, bytes: &[u8]) -> Result<ParsedModule, String> {
    if !bytes.len().is_multiple_of(4) {
        return Err(format!("{label}: {} bytes is not a whole number of SPIR-V words", bytes.len()));
    }
    let (chunks, _) = bytes.as_chunks::<4>();
    let words: Vec<u32> = chunks.iter().map(|w| u32::from_le_bytes(*w)).collect();
    if words.len() < HEADER_WORDS {
        return Err(format!("{label}: {} words is shorter than the SPIR-V header", words.len()));
    }
    if words[0] != SPIRV_MAGIC {
        return Err(format!("{label}: magic word {:#010x} is not {SPIRV_MAGIC:#010x}", words[0]));
    }
    let mut capabilities = Vec::new();
    let mut execution_models = Vec::new();
    let mut i = HEADER_WORDS;
    while i < words.len() {
        let word_count = (words[i] >> 16) as usize;
        let opcode = words[i] & 0xFFFF;
        if word_count == 0 {
            return Err(format!("{label}: instruction at word {i} has a word count of 0"));
        }
        if i + word_count > words.len() {
            return Err(format!(
                "{label}: instruction at word {i} ({word_count} words) overruns the {}-word module",
                words.len()
            ));
        }
        if opcode == OP_CAPABILITY {
            if word_count != 2 {
                return Err(format!("{label}: OpCapability at word {i} has {word_count} words, not 2"));
            }
            capabilities.push(words[i + 1]);
        }
        if opcode == OP_ENTRY_POINT {
            if word_count < ENTRY_POINT_MIN_WORDS {
                return Err(format!(
                    "{label}: OpEntryPoint at word {i} has {word_count} words, fewer than \
                     {ENTRY_POINT_MIN_WORDS}"
                ));
            }
            execution_models.push(words[i + 1]);
        }
        i += word_count;
    }
    Ok(ParsedModule { capabilities, execution_models })
}

/// Every `crates/*/shaders/*.spv` of the workspace, parsed, in a stable order. Panics naming every
/// malformed or unreadable module.
fn committed_modules() -> Vec<Module> {
    let crates_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("invariant: the crate lives under the workspace's crates/ directory");
    let mut crate_dirs: Vec<PathBuf> = std::fs::read_dir(crates_root)
        .unwrap_or_else(|e| panic!("cannot list {}: {e}", crates_root.display()))
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|p| p.join("shaders").is_dir())
        .collect();
    crate_dirs.sort();

    let mut modules = Vec::new();
    let mut errors = Vec::new();
    for crate_dir in &crate_dirs {
        let shaders = crate_dir.join("shaders");
        let mut files: Vec<PathBuf> = std::fs::read_dir(&shaders)
            .unwrap_or_else(|e| panic!("cannot list {}: {e}", shaders.display()))
            .filter_map(|entry| entry.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|x| x == "spv"))
            .collect();
        files.sort();
        let crate_name = crate_dir.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
        for path in files {
            let file = path.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
            let label = format!("{crate_name}/shaders/{file}");
            match std::fs::read(&path) {
                Ok(bytes) => match parse_module(&label, &bytes) {
                    Ok(parsed) => modules.push(Module {
                        crate_dir: crate_name.clone(),
                        file,
                        capabilities: parsed.capabilities,
                        execution_models: parsed.execution_models,
                    }),
                    Err(e) => errors.push(e),
                },
                Err(e) => errors.push(format!("{label}: unreadable: {e}")),
            }
        }
    }
    assert!(errors.is_empty(), "malformed committed SPIR-V module(s):\n  {}", errors.join("\n  "));
    modules
}

/// The files declaring capability `id`, comma-joined, for a message.
fn declarers(modules: &[Module], id: u32) -> String {
    modules
        .iter()
        .filter(|m| m.capabilities.contains(&id))
        .map(|m| m.file.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

/// Whether `feature` is the `name` of a row of `REQUIRED_CORE` or `REQUIRED_V13`.
fn required_by_the_device_tables(feature: &str) -> bool {
    REQUIRED_CORE.iter().any(|row| row.name == feature) || REQUIRED_V13.iter().any(|row| row.name == feature)
}

/// Whether `operation` is the `name` of a row of `REQUIRED_SUBGROUP_OPERATIONS`.
fn checked_by_the_device_subgroup_table(operation: &str) -> bool {
    REQUIRED_SUBGROUP_OPERATIONS.iter().any(|row| row.name == operation)
}

/// Whether capability `id` has a census row of the `SubgroupOperation` class.
fn is_subgroup_operation(id: u32) -> bool {
    CENSUS
        .iter()
        .any(|row| row.id == id && matches!(row.requirement, Requirement::SubgroupOperation(_)))
}

/// One message per entry point of a module declaring a `SubgroupOperation` capability whose
/// stage is not inside `allowed`, and one per such module with no entry point at all.
fn subgroup_stage_violations(modules: &[Module], allowed: VkFlags) -> Vec<String> {
    let mut violations = Vec::new();
    for module in modules {
        let subgroup_caps: Vec<&str> = module
            .capabilities
            .iter()
            .filter(|id| is_subgroup_operation(**id))
            .map(|&id| capability_name(id))
            .collect();
        if subgroup_caps.is_empty() {
            continue;
        }
        if module.execution_models.is_empty() {
            violations.push(format!(
                "{}: declares {} but has no OpEntryPoint, so its stage cannot be checked",
                module.file,
                subgroup_caps.join(", ")
            ));
        }
        for &model in &module.execution_models {
            let inside = stage_bit_of(model).is_some_and(|bit| bit & allowed == bit);
            if !inside {
                violations.push(format!(
                    "{}: declares {} and has an entry point with execution model {} ({model}, stage \
                     {:#x}), outside REQUIRED_SUBGROUP_STAGES ({allowed:#x})",
                    module.file,
                    subgroup_caps.join(", "),
                    execution_model_name(model),
                    stage_bit_of(model).unwrap_or(0)
                ));
            }
        }
    }
    violations
}

/// (a) Every capability a committed module declares has a census row.
#[test]
fn every_declared_capability_has_a_census_row() {
    let modules = committed_modules();
    let mut ids: Vec<u32> = modules.iter().flat_map(|m| m.capabilities.iter().copied()).collect();
    ids.sort_unstable();
    ids.dedup();
    let missing: Vec<String> = ids
        .iter()
        .filter(|id| !CENSUS.iter().any(|row| row.id == **id))
        .map(|&id| {
            format!(
                "capability {id} ({}) declared by {} has no census row",
                capability_name(id),
                declarers(&modules, id)
            )
        })
        .collect();
    assert!(
        missing.is_empty(),
        "{}\nLook up the capability in the Vulkan SPIR-V environment table. If its feature is not \
         enabled by create_device, that is a defect to fix on the device or in the shader, not a \
         row to add.",
        missing.join("\n")
    );
}

/// (b) Every `Required` row names a feature the device tables query, enable and refuse by.
#[test]
fn every_required_row_is_enabled_by_the_device_feature_tables() {
    let modules = committed_modules();
    let unresolved: Vec<String> = CENSUS
        .iter()
        .filter_map(|row| match row.requirement {
            Requirement::Required(feature) if !required_by_the_device_tables(feature) => Some(format!(
                "{} ({}) requires {feature}, which REQUIRED_* does not enable",
                row.name,
                declarers(&modules, row.id)
            )),
            _ => None,
        })
        .collect();
    assert!(unresolved.is_empty(), "{}", unresolved.join("\n"));
}

/// (b') Every `SubgroupOperation` row names a bit the device's subgroup table checks before
/// `vkCreateDevice`.
#[test]
fn every_subgroup_row_is_checked_by_the_device_subgroup_table() {
    let modules = committed_modules();
    let unchecked: Vec<String> = CENSUS
        .iter()
        .filter_map(|row| match row.requirement {
            Requirement::SubgroupOperation(operation) if !checked_by_the_device_subgroup_table(operation) => {
                Some(format!(
                    "{} ({}) requires {operation}, which REQUIRED_SUBGROUP_OPERATIONS does not check",
                    row.name,
                    declarers(&modules, row.id)
                ))
            }
            _ => None,
        })
        .collect();
    assert!(unchecked.is_empty(), "{}", unchecked.join("\n"));
}

/// (b'') Every entry point of every committed module that declares a `SubgroupOperation`
/// capability runs in a stage inside `REQUIRED_SUBGROUP_STAGES` (`VUID-RuntimeSpirv-None-06343`).
#[test]
fn every_subgroup_declarer_runs_only_in_required_subgroup_stages() {
    let violations = subgroup_stage_violations(&committed_modules(), REQUIRED_SUBGROUP_STAGES);
    assert!(
        violations.is_empty(),
        "{}\nEither add the stage to REQUIRED_SUBGROUP_STAGES (which refuses devices without it) or \
         remove the wave operation from the shader.",
        violations.join("\n")
    );
}

/// The stage check on synthetic modules: `GroupNonUniformBallot` in a `Fragment` entry point is
/// rejected, naming the module and the model; the same module as `GLCompute` passes.
#[test]
fn the_stage_check_rejects_a_fragment_subgroup_module_by_name() {
    fn module(label: &str, execution_model: u32) -> Module {
        let words = [
            SPIRV_MAGIC,
            0x0001_0600,
            0,
            8,
            0,
            (2 << 16) | OP_CAPABILITY,
            CAP_SHADER,
            (2 << 16) | OP_CAPABILITY,
            CAP_GROUP_NON_UNIFORM_BALLOT,
            (4 << 16) | OP_ENTRY_POINT,
            execution_model,
            1,
            u32::from_le_bytes(*b"main"),
        ];
        let bytes: Vec<u8> = words.iter().flat_map(|w| w.to_le_bytes()).collect();
        let parsed = parse_module(label, &bytes).expect("the synthetic module is well-formed");
        Module {
            crate_dir: "synthetic".to_owned(),
            file: label.to_owned(),
            capabilities: parsed.capabilities,
            execution_models: parsed.execution_models,
        }
    }

    let fragment = module("wave.fs.spv", EXECUTION_MODEL_FRAGMENT);
    assert_eq!(fragment.execution_models, [EXECUTION_MODEL_FRAGMENT]);
    let violations = subgroup_stage_violations(&[fragment], REQUIRED_SUBGROUP_STAGES);
    assert_eq!(violations.len(), 1, "{violations:?}");
    assert!(
        violations[0].starts_with("wave.fs.spv:") && violations[0].contains("Fragment"),
        "{}",
        violations[0]
    );

    let compute = module("wave.comp.spv", EXECUTION_MODEL_GL_COMPUTE);
    assert_eq!(subgroup_stage_violations(&[compute], REQUIRED_SUBGROUP_STAGES), Vec::<String>::new());
}

/// (c) No census row is stale: every row's capability is declared by some committed module.
#[test]
fn no_census_row_is_stale() {
    let modules = committed_modules();
    let stale: Vec<String> = CENSUS
        .iter()
        .filter(|row| !modules.iter().any(|m| m.capabilities.contains(&row.id)))
        .map(|row| format!("stale census row: {} ({}) is declared by no committed .spv", row.name, row.id))
        .collect();
    assert!(stale.is_empty(), "{}", stale.join("\n"));
}

/// (d) The walk is not vacuous, and the two capabilities that motivated this census are still
/// declared by the modules that declared them — fixed on the device, not by a recompile.
#[test]
fn the_census_walk_is_not_vacuous() {
    let modules = committed_modules();
    assert!(
        modules.len() >= MIN_MODULES,
        "the walk found {} committed .spv modules, below the floor of {MIN_MODULES}",
        modules.len()
    );
    let mut crates: Vec<&str> = modules.iter().map(|m| m.crate_dir.as_str()).collect();
    crates.sort_unstable();
    crates.dedup();
    assert!(crates.len() >= MIN_CRATES, "the walk found shaders in only {crates:?}");

    let without_shader: Vec<&str> =
        modules.iter().filter(|m| !m.capabilities.contains(&CAP_SHADER)).map(|m| m.file.as_str()).collect();
    assert!(without_shader.is_empty(), "modules not declaring `Shader`: {without_shader:?}");

    for (file, cap, feature) in [
        ("vb_raster.fs.spv", CAP_GEOMETRY, "geometryShader"),
        ("smaa_edge.fs.spv", CAP_DEMOTE_TO_HELPER_INVOCATION, "shaderDemoteToHelperInvocation"),
    ] {
        let hits: Vec<&Module> = modules.iter().filter(|m| m.file == file).collect();
        assert_eq!(hits.len(), 1, "expected exactly one committed {file}");
        assert!(
            hits[0].capabilities.contains(&cap),
            "{file} no longer declares {} ({cap})",
            capability_name(cap)
        );
        let row = CENSUS.iter().find(|r| r.id == cap).expect("anchor capability has a census row");
        assert_eq!(row.requirement, Requirement::Required(feature), "{file}'s anchor row");
    }
}

/// (e) Every `GroupNonUniform*` row is the `SubgroupOperation` its environment-table bit names.
///
/// Row 61 (`GroupNonUniform`, BASIC) is what this pins in practice. Every committed module that
/// declares it also declares `GroupNonUniformBallot` (64), so relabelling it `Always` changes no
/// other test's verdict — the stage check (b'') still reaches all three declarers through row 64 —
/// while it drops BASIC from the device cross-check (b') and silently exempts any future module
/// that declares 61 alone from the stage check.
#[test]
fn every_group_non_uniform_row_is_the_subgroup_operation_its_environment_bit_names() {
    let mut pinned = Vec::new();
    for row in CENSUS {
        let Some(&(_, bit)) = SUBGROUP_CAPABILITY_BITS.iter().find(|(id, _)| *id == row.id) else {
            continue;
        };
        assert_eq!(
            row.requirement,
            Requirement::SubgroupOperation(bit),
            "{} ({}): the Vulkan SPIR-V environment table licenses it by {bit}",
            row.name,
            row.id
        );
        pinned.push(row.id);
    }
    assert!(
        pinned.contains(&CAP_GROUP_NON_UNIFORM) && pinned.contains(&CAP_GROUP_NON_UNIFORM_BALLOT),
        "the loop pinned {pinned:?}, not both shipped subgroup rows (61, 64)"
    );
    // A mistyped id in the table would pin nothing, so the table must name the family it claims.
    for &(id, bit) in SUBGROUP_CAPABILITY_BITS {
        assert!(
            capability_name(id).starts_with("GroupNonUniform"),
            "SUBGROUP_CAPABILITY_BITS maps {bit} to capability {id} ({}), not a GroupNonUniform* id",
            capability_name(id)
        );
    }
}

/// The census rows' names agree with the diagnostic name list, and ids are unique.
#[test]
fn census_rows_are_unique_and_named_per_the_grammar() {
    for (i, row) in CENSUS.iter().enumerate() {
        assert!(
            CENSUS[i + 1..].iter().all(|other| other.id != row.id),
            "duplicate census row for capability {}",
            row.id
        );
        let listed = CAPABILITY_NAMES.iter().find(|(id, _)| *id == row.id).map(|(_, n)| *n);
        assert_eq!(listed, Some(row.name), "capability {} name", row.id);
    }
    for (i, (id, _)) in CAPABILITY_NAMES.iter().enumerate() {
        assert!(CAPABILITY_NAMES[i + 1..].iter().all(|(other, _)| other != id), "duplicate name-list id {id}");
    }
}

/// The parser rejects each malformed shape, naming the module, and reads a well-formed one.
#[test]
fn the_parser_rejects_malformed_modules_by_name() {
    fn bytes(words: &[u32]) -> Vec<u8> {
        words.iter().flat_map(|w| w.to_le_bytes()).collect()
    }
    let header = [SPIRV_MAGIC, 0x0001_0600, 0, 8, 0];
    let op_cap = |cap: u32| [(2 << 16) | OP_CAPABILITY, cap];

    let mut good = header.to_vec();
    good.extend_from_slice(&op_cap(CAP_SHADER));
    good.extend_from_slice(&op_cap(CAP_GEOMETRY));
    // OpEntryPoint Fragment %1 "main" (the name's NUL falls in the next word, omitted here).
    good.extend_from_slice(&[(4 << 16) | OP_ENTRY_POINT, EXECUTION_MODEL_FRAGMENT, 1, u32::from_le_bytes(*b"main")]);
    good.extend_from_slice(&[(1 << 16) | 0xFD]); // a 1-word instruction with no operands
    assert_eq!(
        parse_module("good", &bytes(&good)),
        Ok(ParsedModule {
            capabilities: vec![CAP_SHADER, CAP_GEOMETRY],
            execution_models: vec![EXECUTION_MODEL_FRAGMENT]
        })
    );

    let mut ragged = bytes(&good);
    ragged.push(0);
    let err = parse_module("ragged.spv", &ragged).expect_err("a ragged length is rejected");
    assert!(err.starts_with("ragged.spv:"), "{err}");

    let mut bad_magic = good.clone();
    bad_magic[0] = SPIRV_MAGIC.swap_bytes();
    let err = parse_module("magic.spv", &bytes(&bad_magic)).expect_err("a bad magic is rejected");
    assert!(err.starts_with("magic.spv:"), "{err}");

    let mut zero = header.to_vec();
    zero.push(OP_CAPABILITY);
    let err = parse_module("zero.spv", &bytes(&zero)).expect_err("a zero word count is rejected");
    assert!(err.starts_with("zero.spv:") && err.contains("word count of 0"), "{err}");

    let mut overrun = header.to_vec();
    overrun.push((3 << 16) | OP_CAPABILITY);
    let err = parse_module("overrun.spv", &bytes(&overrun)).expect_err("an overrun is rejected");
    assert!(err.starts_with("overrun.spv:"), "{err}");

    let mut short_entry = header.to_vec();
    short_entry.extend_from_slice(&[(3 << 16) | OP_ENTRY_POINT, EXECUTION_MODEL_GL_COMPUTE, 1]);
    let err = parse_module("entry.spv", &bytes(&short_entry)).expect_err("a nameless OpEntryPoint is rejected");
    assert!(err.starts_with("entry.spv:") && err.contains("OpEntryPoint"), "{err}");

    let err = parse_module("short.spv", &bytes(&header[..3])).expect_err("a short module is rejected");
    assert!(err.starts_with("short.spv:"), "{err}");
}
