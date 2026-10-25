//! IBL1 R1 (`ibl1/cut.md` C11, D-P2b/D-P2c) — **an unarmed boot builds today's modules; an armed
//! boot builds the `-D REFL=1` twin of every lit producer and nothing else** (host; no device).
//!
//! 1. `refl_selection_truth_table`: for every `LitProducer`, `spirv(false)` is the base module and
//!    `spirv(true)` the REFL twin, and the two differ.
//! 2. `the_boot_selects_every_lit_producer_through_the_selector`: a source census over
//!    `gpu_scene/mod.rs` — no boot site names a lit producer's base accessor directly (so no
//!    producer can stay on its base module on an armed boot), every producer is created through
//!    `LitProducer::X.spirv(<the boot's arming>)`, and the VIS / VIS+MV stages (no `lit`, no twin)
//!    keep their base accessors.
//! 3. `armed_layout_counts`: the environment's binding in each set and the armed counts the rule
//!    (xv) report states — software 20 → 21, hwrt 22 → 23, DENOISED 23 → 24 (VIS 23 and VIS+MV 25
//!    unchanged), `forward_layout1` 4 → 5, the VB split Set 1 8/9 → 9/10 — all within the cap of 25.
//!
//! The unarmed boot's byte identity on the device is the pin sweep's (every existing leg
//! byte-identical, ibl1/cut.md §4); this file is the boot's structural half.

use boyko_rhi::MAX_BIND_GROUP_BINDINGS;
use boyko_rhi_vulkan::compute::LitProducer;
use boyko_rhi_vulkan::present::{
    ENV_BINDING_FORWARD_SET1, ENV_BINDING_RESOLVE_DENOISED, ENV_BINDING_RESOLVE_HWRT,
    ENV_BINDING_RESOLVE_SOFTWARE, ENV_BINDING_VB_SPLIT_SET1,
};

/// `(variant, the base accessor its boot site used to call)` — the census's ban list.
const BASE_ACCESSORS: [&str; 20] = [
    "deferred_pbr_spirv()",
    "deferred_pbr_wrap_spirv()",
    "deferred_pbr_hwrt_spirv()",
    "deferred_pbr_denoised_spirv()",
    "forward_opaque_fs_spirv()",
    "forward_opaque_froxel_fs_spirv()",
    "sdf_forward_march_spirv()",
    "sdf_forward_march_sdfonly_spirv()",
    "sdf_forward_march_viewt_spirv()",
    "sdf_forward_march_sdfonly_viewt_spirv()",
    "vb_resolve_spirv()",
    "vb_resolve_froxel_spirv()",
    "vb_shade_spirv()",
    "vb_shade_froxel_spirv()",
    "vb_shade_tex_spirv()",
    "vb_shade_tex_froxel_spirv()",
    "vb_shade_split_spirv()",
    "vb_shade_split_tex_spirv()",
    "vb_shade_split_hwrt_spirv()",
    "vb_shade_split_tex_hwrt_spirv()",
];

/// Every `LitProducer` variant name, as the census expects it spelled at a boot site.
const VARIANTS: [&str; 20] = [
    "DeferredPbr",
    "DeferredPbrWrap",
    "DeferredPbrHwrt",
    "DeferredPbrHwrtDenoised",
    "ForwardOpaqueFs",
    "ForwardOpaqueFroxelFs",
    "SdfForwardMarch",
    "SdfForwardMarchSdfonly",
    "SdfForwardMarchViewt",
    "SdfForwardMarchSdfonlyViewt",
    "VbResolve",
    "VbResolveFroxel",
    "VbShade",
    "VbShadeFroxel",
    "VbShadeTex",
    "VbShadeTexFroxel",
    "VbShadeSplit",
    "VbShadeSplitTex",
    "VbShadeSplitHwrt",
    "VbShadeSplitTexHwrt",
];

#[test]
fn refl_selection_truth_table() {
    assert_eq!(LitProducer::ALL.len(), if cfg!(feature = "hwrt") { 20 } else { 16 }, "the lit producers this build compiles");
    for &p in LitProducer::ALL {
        let base = p.base_spirv();
        let refl = p.refl_spirv();
        assert!(core::ptr::eq(p.spirv(false), base), "{p:?}: an unarmed boot builds the base module");
        assert!(core::ptr::eq(p.spirv(true), refl), "{p:?}: an armed boot builds the REFL twin");
        assert_ne!(base, refl, "{p:?}: the REFL twin is a different module");
        assert_eq!(base[0], 0x0723_0203, "{p:?}: the base module is SPIR-V");
        assert_eq!(refl[0], 0x0723_0203, "{p:?}: the REFL module is SPIR-V");
    }
}

/// The text of `gpu_scene/mod.rs` with `//` comments removed.
fn gpu_scene_code() -> String {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/src/gpu_scene/mod.rs");
    let text = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{path}: {e}"));
    text.lines().map(|l| l.split("//").next().unwrap_or("")).collect::<Vec<_>>().join("\n")
}

#[test]
fn the_boot_selects_every_lit_producer_through_the_selector() {
    let code = gpu_scene_code();
    let mut problems = Vec::new();
    for acc in BASE_ACCESSORS {
        // `LitProducer::base_spirv` lives in `boyko_rhi_vulkan`; a boot site naming the accessor
        // would build the base module whatever the arming.
        let direct = code.matches(acc).count();
        if direct != 0 {
            problems.push(format!("gpu_scene/mod.rs calls {acc} directly ({direct}x)"));
        }
    }
    for v in VARIANTS {
        let armed = code.matches(&format!("LitProducer::{v}.spirv(env_armed)")).count()
            + code.matches(&format!("LitProducer::{v}.spirv(self.env_armed)")).count();
        if armed != 1 {
            problems.push(format!("LitProducer::{v} is selected {armed}x with the boot's arming (want exactly once)"));
        }
    }
    for vis in ["deferred_pbr_vis_spirv()", "deferred_pbr_vis_mv_spirv()"] {
        if !code.contains(vis) {
            problems.push(format!("the VIS stage {vis} must stay on its base module (no `lit`, no twin)"));
        }
    }
    assert!(problems.is_empty(), "the boot's REFL selection:\n{}", problems.join("\n"));
}

#[test]
fn armed_layout_counts() {
    // The environment sits at the first index past each set, so the armed count is binding + 1.
    let counts = [
        ("software resolve", ENV_BINDING_RESOLVE_SOFTWARE, 20, 21),
        ("RESOLVE_INLINE-hwrt resolve", ENV_BINDING_RESOLVE_HWRT, 22, 23),
        ("DENOISED resolve", ENV_BINDING_RESOLVE_DENOISED, 23, 24),
        ("forward_layout1", ENV_BINDING_FORWARD_SET1, 4, 5),
    ];
    for (name, binding, base, armed) in counts {
        assert_eq!(binding, base, "{name}: the environment is the first index past the base set");
        assert_eq!(binding + 1, armed, "{name}: armed count");
        assert!(armed as usize <= MAX_BIND_GROUP_BINDINGS, "{name}: {armed} exceeds the binding cap");
    }
    // The VB split Set 1 ends at @7 (not hwrt) or @8 (hwrt); the environment is @9 on both.
    assert_eq!(ENV_BINDING_VB_SPLIT_SET1, 9);
    // VIS (23) and VIS+MV (25) carry no environment: their counts are not raised.
    assert!((ENV_BINDING_RESOLVE_DENOISED as usize) < MAX_BIND_GROUP_BINDINGS, "the fullest armed set stays under the cap (AD-26 stays R2's)");
}
