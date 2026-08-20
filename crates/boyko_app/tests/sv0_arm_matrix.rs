//! **The VB-SV0 arming matrix — the half that survives on the CPU**, restored from the file the
//! inline SV0 revert (`13f1c9a3`) deleted and re-pointed at the DEDICATED-PASS implementation that
//! replaced it (`sdf_mesh_shadow.comp`, default OFF).
//!
//! # What the old matrix was, and why most of it cannot come back
//!
//! `13f1c9a3^:crates/boyko_app/tests/sv0_arm_matrix.rs` was a truth table over the arming inputs
//! whose LOAD-BEARING half was a 24-dump GPU sweep: eight armable VB lit-producer variants × three
//! header modes, each cell a real windowed render, driven by `scripts\sv0_arm_matrix.ps1` and
//! measured as a changed-pixel fraction against that row's unarmed dump. Both the script and the
//! per-row `.spv` fixtures went with the revert. That half is **not restored here** and cannot be:
//! it needs a GPU, a dump directory, and a driver script that no longer exists.
//!
//! Its other rows have since been re-covered elsewhere, and duplicating them would be noise:
//!
//! * the boot-capability truth table (`vb_sdf_mesh_armable()` over path × legs × hwrt × device
//!   caps) came back verbatim as `boyko_render::render_path_config::tests::sv0_never_arms_under_hwrt`
//!   and `::sv0_armable_only_on_vb_with_both_legs`;
//! * the `sync_sv0_light_gate` clamp, the two terms' independence, the value-gated write and the
//!   word-7 bit packing are `boyko_render::light::tests::sv0_gate_*` /
//!   `::vb_sv0_gate_bits_are_independent`.
//!
//! # What IS new, and is this file's whole reason to exist
//!
//! The inline implementation carried the arming decision in exactly ONE place: light-header word 7
//! bits 5..6, which every VB lit producer decoded with `load_vb_sdf_mesh_mode`. The dedicated pass
//! added a SECOND carrier of the same number — `GBufferScene::vb_sdf_mesh_mode`, threaded by
//! `boyko_app::runner` into `GpuScene::scene(..)` and read by
//! `boyko_rhi_vulkan::present::graph_bridge` as the pass-declaration predicate
//! (`mesh_leg && scene.vb_sdf_mesh_mode != 0`).
//!
//! The two carriers are computed by two different expressions in two different crates, and nothing
//! joins them. A disagreement is not a cosmetic drift:
//!
//! * header non-zero while the graph mode is `0` ⇒ the pass is never declared, the term image is
//!   never written, and the lit producers `.Load` whatever the build-time seed left there while
//!   their own `sv0_mode` says the term is live;
//! * the converse ⇒ a declared prepass, its barriers, and its dispatch on a frame whose shaders
//!   take neither gated branch — which is exactly the always-paid dark cost the inline stage was
//!   reverted for.
//!
//! So the matrix restored here is `(request_shadow, request_ao) × boot shape → mode`, asserted on
//! BOTH carriers at once, with the clamp `effective = request && armable` and the pass-declaration
//! predicate `mode != 0` read off the same cell.
//!
//! # Stated residual — this file does not read `runner.rs`
//!
//! `boyko_app::runner` builds its argument as `u32::from(shadow_armed) | (u32::from(ao_armed) << 1)`
//! — a bare bit layout, not the named constants. [`sv0_mode_bit_layout_is_the_one_the_runner_hardcodes`]
//! pins that layout against `VB_SDF_MESH_SHADOW_BIT` / `VB_SDF_MESH_AO_BIT`, so moving either
//! constant reds a test instead of silently unhooking the graph gate from the header. It does NOT
//! observe the runner's own line: an edit that rewrites that expression in place is out of this
//! gate's reach, and the only instrument that would catch it is a live frame.

use boyko_app::prelude::App;
use boyko_render::{
    GeometryLegs, LightTableDirty, LightingConfig, RenderPath, RenderPathConfig,
    RenderPathConsumers, RenderPathDeviceCaps, ResolvedRenderPath, VB_SDF_MESH_AO_BIT,
    VB_SDF_MESH_MODE_MASK, VB_SDF_MESH_MODE_SHIFT, VB_SDF_MESH_SHADOW_BIT, resolve_render_path,
    sync_sv0_light_gate,
};

// ===========================================================================================
// The matrix axes
// ===========================================================================================

/// One boot shape of the matrix: the inputs `resolve_render_path` turns into a
/// [`ResolvedRenderPath`], plus the armability that resolve is expected to produce.
struct Boot {
    /// The label a failing cell reports itself under.
    name: &'static str,
    /// The owner's requested render path.
    path: RenderPath,
    /// The owner's requested geometry legs.
    legs: GeometryLegs,
    /// Arms `mesh_geo_shade_split` on a VB × Both boot (the old matrix's split rows 7/8).
    ssao_on: bool,
    /// The hardware shadow chain — what displaces `ShadowSources::SDF_SOFT_MARCH` and therefore
    /// makes SV0 structurally unarmable (the old matrix's absent rows 9/10).
    hwrt_on: bool,
    /// `shaderStorageBufferArrayNonUniformIndexing`. `false` degrades VB to Deferred.
    descriptor_indexing: bool,
    /// The armability this boot MUST resolve to. Asserted per cell rather than inferred, so a
    /// resolve that silently stopped arming anything cannot make the whole sweep pass by covering
    /// nothing — the failure mode the old row table's own vacuity guard existed for.
    armable: bool,
    /// Why this boot lands where it does; quoted in the failure message so a red cell names its
    /// term instead of only its index.
    why: &'static str,
}

/// **The boot shapes SV0 arming is quantified over.** The armable rows are the configuration the
/// `vb_both_sdf` / `vb_both_sdf_tex` fixtures boot under (`VisibilityBuffer × Both`, the runner's
/// hardwired `sdf_shadows_wanted`); every other row is unarmable for its own distinct reason.
const BOOTS: [Boot; 9] = [
    Boot {
        name: "VB x Both (fused)",
        path: RenderPath::VisibilityBuffer,
        legs: GeometryLegs::Both,
        ssao_on: false,
        hwrt_on: false,
        descriptor_indexing: true,
        armable: true,
        why: "the SDF soft march is the shadow source and there are mesh pixels to shade",
    },
    Boot {
        name: "VB x Both + SSAO (split tail)",
        path: RenderPath::VisibilityBuffer,
        legs: GeometryLegs::Both,
        ssao_on: true,
        hwrt_on: false,
        descriptor_indexing: true,
        armable: true,
        why: "SSAO selects the split lit producer, which is armable exactly like the fused one",
    },
    Boot {
        name: "VB x Both + hwrt",
        path: RenderPath::VisibilityBuffer,
        legs: GeometryLegs::Both,
        ssao_on: true,
        hwrt_on: true,
        descriptor_indexing: true,
        armable: false,
        why: "the hwrt carrier displaces SDF_SOFT_MARCH, and it is what selects the _hwrt tails",
    },
    Boot {
        name: "VB x Mesh",
        path: RenderPath::VisibilityBuffer,
        legs: GeometryLegs::Mesh,
        ssao_on: false,
        hwrt_on: false,
        descriptor_indexing: true,
        armable: false,
        why: "no SDF leg, so there is no field to march",
    },
    Boot {
        name: "VB x Sdf",
        path: RenderPath::VisibilityBuffer,
        legs: GeometryLegs::Sdf,
        ssao_on: false,
        hwrt_on: false,
        descriptor_indexing: true,
        armable: false,
        why: "no mesh leg, so the term would be quantified over an empty pixel set",
    },
    Boot {
        name: "Deferred x Both",
        path: RenderPath::Deferred,
        legs: GeometryLegs::Both,
        ssao_on: false,
        hwrt_on: false,
        descriptor_indexing: true,
        armable: false,
        why: "Deferred ships this visual through the marcher's own composite, not through SV0",
    },
    Boot {
        name: "Forward x Both",
        path: RenderPath::Forward,
        legs: GeometryLegs::Both,
        ssao_on: false,
        hwrt_on: false,
        descriptor_indexing: true,
        armable: false,
        why: "no VB lit-producer tail exists on this path",
    },
    Boot {
        name: "ForwardPlus x Both",
        path: RenderPath::ForwardPlus,
        legs: GeometryLegs::Both,
        ssao_on: false,
        hwrt_on: false,
        descriptor_indexing: true,
        armable: false,
        why: "no VB lit-producer tail exists on this path",
    },
    Boot {
        name: "VB x Both, no descriptor indexing",
        path: RenderPath::VisibilityBuffer,
        legs: GeometryLegs::Both,
        ssao_on: false,
        hwrt_on: false,
        descriptor_indexing: false,
        armable: false,
        why: "the device degrades VB to Deferred, which takes SV0 with it",
    },
];

/// The four owner requests. Both terms are gated independently — SV0 is two terms, and a matrix
/// that only ever moved them together would pass a plumbing that drove both from one bit.
const REQUESTS: [(bool, bool); 4] = [(false, false), (true, false), (false, true), (true, true)];

// ===========================================================================================
// The two carriers, and the request they are clamped from
// ===========================================================================================

/// Resolves one boot shape through the PRODUCTION entry point, so a change to the arming rules
/// reaches this matrix instead of being mirrored past it.
///
/// `sdf_shadows_wanted: true` is not a choice here — it is what `boyko_app::runner` hardwires, so
/// it is the only consumer set an SV0 boot can actually occur under.
fn resolve(boot: &Boot) -> ResolvedRenderPath {
    let (resolved, _) = resolve_render_path(
        &RenderPathConfig { path: boot.path, legs: boot.legs },
        RenderPathConsumers {
            sdf_shadows_wanted: true,
            ssao_on: boot.ssao_on,
            hwrt_denoise_or_vis_on: boot.hwrt_on,
            ..Default::default()
        },
        RenderPathDeviceCaps::new(boot.descriptor_indexing),
    );
    resolved
}

/// The world every cell is driven through, built ONCE.
///
/// `App::new` stands up a whole worker pool, and the matrix has 36 cells — but the reuse is sound
/// for a stronger reason than cost: [`arm`] overwrites BOTH resources wholesale, so each cell
/// starts from a default `LightingConfig` with the `_armed` pair clear, exactly as a fresh world
/// would. Nothing in `sync_sv0_light_gate` reads state older than the resources it is handed.
fn matrix_app() -> App {
    let mut app = App::new();
    app.insert_resource(ResolvedRenderPath::default());
    app.insert_resource(LightingConfig::default());
    app.insert_resource(LightTableDirty(false));
    app
}

/// Runs the SOLE production writer of the `_armed` pair over one (boot, request) cell and returns
/// the config it published — the same system `boyko_app::plugins` registers
/// `.before_set(LightCollectSet)`.
fn arm(
    app: &mut App,
    resolved: ResolvedRenderPath,
    request_shadow: bool,
    request_ao: bool,
) -> LightingConfig {
    *app.world_mut().resource_mut::<ResolvedRenderPath>() = resolved;
    *app.world_mut().resource_mut::<LightingConfig>() = LightingConfig {
        vb_sdf_mesh_shadow: request_shadow,
        vb_sdf_mesh_ao: request_ao,
        ..LightingConfig::default()
    };
    app.world_mut().run_system(sync_sv0_light_gate);
    *app.world().resource::<LightingConfig>()
}

/// **Carrier 1 — the GRAPH's mode**: what `boyko_app::runner` threads into
/// `GBufferScene::vb_sdf_mesh_mode`, and what `graph_bridge` tests for `!= 0` before declaring the
/// `sdf_mesh_shadow` pass.
///
/// Written from the NAMED constants rather than from the runner's bare `| (.. << 1)`; the two
/// spellings are pinned equal by [`sv0_mode_bit_layout_is_the_one_the_runner_hardcodes`].
fn graph_mode(cfg: &LightingConfig) -> u32 {
    (u32::from(cfg.vb_sdf_mesh_shadow_armed) * VB_SDF_MESH_SHADOW_BIT)
        | (u32::from(cfg.vb_sdf_mesh_ao_armed) * VB_SDF_MESH_AO_BIT)
}

/// **Carrier 2 — the SHADERS' mode**: word-7 bits 5..6 of the packed light header, decoded exactly
/// as `light_table.hlsli`'s `load_vb_sdf_mesh_mode` decodes it (`(LightBuf[7] >> 5) & 3`).
fn header_mode(cfg: &LightingConfig) -> u32 {
    (cfg.shadow_gate_word() >> VB_SDF_MESH_MODE_SHIFT) & VB_SDF_MESH_MODE_MASK
}

/// The mode a request WOULD produce on an armable boot — the upper bound the clamp may never
/// exceed.
fn request_mode(request_shadow: bool, request_ao: bool) -> u32 {
    (u32::from(request_shadow) * VB_SDF_MESH_SHADOW_BIT)
        | (u32::from(request_ao) * VB_SDF_MESH_AO_BIT)
}

// ===========================================================================================
// The gates
// ===========================================================================================

/// **The bit layout `boyko_app::runner` hardcodes.**
///
/// The runner does not name `VB_SDF_MESH_SHADOW_BIT` / `VB_SDF_MESH_AO_BIT`; it writes
/// `u32::from(shadow_armed) | (u32::from(ao_armed) << 1)`. That is correct only while the shadow
/// term is bit 0 and the AO term is bit 1 of the sub-field, which is a property of the constants
/// `LightingConfig::shadow_gate_word` packs with. Moving either constant silently unhooks the graph
/// gate from the header the shaders decode; this test is that move's red.
///
/// Restored from the old matrix's `sv0_row_table_covers_the_eight_armable_variants_exactly` tail,
/// which pinned the same numbering for the dump filenames' sake. Under the dedicated pass the same
/// three literals became load-bearing at runtime.
#[test]
fn sv0_mode_bit_layout_is_the_one_the_runner_hardcodes() {
    assert_eq!(
        VB_SDF_MESH_SHADOW_BIT, 1,
        "`runner.rs` builds the graph-side mode with `u32::from(shadow_armed)`, i.e. the shadow \
         term at bit 0 of the sub-field"
    );
    assert_eq!(
        VB_SDF_MESH_AO_BIT, 2,
        "`runner.rs` builds the graph-side mode with `u32::from(ao_armed) << 1`, i.e. the contact-AO \
         term at bit 1 of the sub-field"
    );
    assert_eq!(
        VB_SDF_MESH_MODE_MASK,
        VB_SDF_MESH_SHADOW_BIT | VB_SDF_MESH_AO_BIT,
        "the 2-bit sub-field mask must cover exactly the two term bits and nothing else"
    );

    for (shadow, ao) in REQUESTS {
        let cfg = LightingConfig {
            vb_sdf_mesh_shadow_armed: shadow,
            vb_sdf_mesh_ao_armed: ao,
            ..LightingConfig::default()
        };
        assert_eq!(
            graph_mode(&cfg),
            u32::from(shadow) | (u32::from(ao) << 1),
            "(shadow={shadow}, ao={ao}): the constants-derived mode and `runner.rs`'s literal \
             `shadow | (ao << 1)` disagree — the graph would gate the `sdf_mesh_shadow` pass on a \
             different number than the shaders decode from word 7"
        );
    }
}

/// **The matrix.** Every request against every boot shape: the mode both carriers report is
/// `request && armable`, exactly — never more, and never a different number on the two ends.
///
/// # The mutations this gate is required to go red under
///
/// * drop the `&& armable` from either term in `sync_sv0_light_gate` → the unarmable rows arm;
/// * route the owner's REQUEST fields into `shadow_gate_word` instead of the `_armed` pair → the
///   two carriers keep agreeing but the unarmable rows stop clamping;
/// * change `LightingConfig::shadow_gate_word`'s SV0 sub-field shift or mask → the carriers split.
#[test]
fn sv0_arm_matrix_clamps_every_request_to_the_boots_capability() {
    // Anti-vacuity, before a single cell: a table of only-unarmable boots would satisfy every
    // assertion below by covering nothing, and a table of only-armable ones would never exercise
    // the clamp at all.
    assert!(BOOTS.iter().any(|b| b.armable), "the matrix must contain an ARMABLE boot");
    assert!(BOOTS.iter().any(|b| !b.armable), "the matrix must contain an UNARMABLE boot");

    let mut app = matrix_app();
    for boot in &BOOTS {
        let resolved = resolve(boot);
        assert_eq!(
            resolved.vb_sdf_mesh_armable(),
            boot.armable,
            "`{}` resolved to armable={} but the matrix declares {} ({}) — the boot-capability \
             truth table is `render_path_config`'s; this cell only depends on it",
            boot.name,
            resolved.vb_sdf_mesh_armable(),
            boot.armable,
            boot.why
        );

        for (request_shadow, request_ao) in REQUESTS {
            let requested = request_mode(request_shadow, request_ao);
            let expected = if boot.armable { requested } else { 0 };
            let cfg = arm(&mut app, resolved, request_shadow, request_ao);

            let graph = graph_mode(&cfg);
            let header = header_mode(&cfg);

            assert_eq!(
                graph, expected,
                "`{}` (shadow={request_shadow}, ao={request_ao}): the graph-side mode is {graph}, \
                 expected {expected} = request({requested}) && armable({}). {}",
                boot.name, boot.armable, boot.why
            );
            // The join. A cell that disagrees here declares the prepass on a frame whose shaders
            // take no gated branch, or arms those branches over a term image no pass ever wrote.
            assert_eq!(
                graph, header,
                "`{}` (shadow={request_shadow}, ao={request_ao}): the graph gates the \
                 `sdf_mesh_shadow` pass on {graph} while the lit producers decode {header} from \
                 light-header word 7 — the two carriers of the SV0 mode disagree",
                boot.name
            );
            // The safety direction, stated on its own: the clamp is monotone DOWNWARD. A mode bit
            // the owner never asked for is how a pre-SV0 pin stops being byte-identical.
            assert_eq!(
                graph & !requested,
                0,
                "`{}` (shadow={request_shadow}, ao={request_ao}): the resolved mode {graph} carries \
                 a bit the request ({requested}) does not — the gate must only ever clamp DOWN",
                boot.name
            );
        }
    }
}

/// **The graph predicate's second conjunct is redundant, and must stay that way.**
///
/// `graph_bridge` declares the prepass on `mesh_leg && scene.vb_sdf_mesh_mode != 0`. The `mesh_leg`
/// term is belt-and-braces: `vb_sdf_mesh_armable()` already requires it, so a non-zero mode implies
/// it. Pinning the implication is what keeps that redundancy honest — if `vb_sdf_mesh_armable` were
/// ever widened to a mesh-legless boot, the pass would become the ONLY thing standing between an
/// armed header and a prepass that reads a `vb_id` no raster wrote, and the widening would show up
/// here rather than as a blank term image.
#[test]
fn sv0_mode_nonzero_implies_the_mesh_leg() {
    let mut app = matrix_app();
    for boot in &BOOTS {
        let resolved = resolve(boot);
        // Both terms requested: the strongest request, so any boot that can produce a non-zero mode
        // does so here.
        let cfg = arm(&mut app, resolved, true, true);
        if graph_mode(&cfg) != 0 {
            assert!(
                resolved.mesh_leg,
                "`{}` resolves a non-zero SV0 mode without a mesh leg — `graph_bridge`'s \
                 `mesh_leg &&` conjunct would then be the only thing disarming the prepass, and \
                 the light header would still tell the lit producers the term is live",
                boot.name
            );
        }
    }
}
