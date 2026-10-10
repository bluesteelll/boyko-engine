//! **VG R3 piece 3 step P3-8 — the CPU census of `vb_occ_mixed`'s occlusion verdicts, and the
//! ANALYTIC half of FIXTURE PRECONDITION VG-P3-MIXED-OCCLUDES.**
//!
//! Every clause of gate G-P3-B is meaningless on a scene that cannot occlude. This file is the leg
//! that decides that question **before any GPU runs**, from the same view-projection and the same
//! world AABBs the engine uploads, through `boyko_render::hzb`'s own
//! `project_aabb → select_texels` chain — the `vg_cull_granularity_census.rs` shape: measure the
//! fixture's own numbers, print them, and pin the properties the later gates rest on.
//!
//! # Why "wholly behind the slab's silhouette" is NOT the property
//!
//! [`select_texels`] returns the **ALIGNED** expansion — `containing_texel(t, level) = t >> level` —
//! and [`boyko_render::hzb::occluder_depth`] folds all four selected texels with a conservative
//! `min`. With the reverse-Z clear at `0.0`, **one background texel anywhere in that footprint
//! forces KEEP**. At 512² a rect that merely straddles `x = 256` selects level 8, whose footprint is
//! the entire image. So a fixture can be perfectly hidden and still be structurally incapable of
//! being deferred, and a gate asserting `Σ n_defer > 0` on it would red a CORRECT engine.
//!
//! The property that actually holds is [`vb_occ_mixed_scene`]'s design rule: each hidden instance's
//! rect lies wholly inside ONE `2^(L+1)`-aligned block of width `2^(L+1)` that is itself wholly
//! inside the slab's rect. Then `msb(tx0 ^ tx1) <= L`, the 2×2 footprint IS that block, and every
//! texel in it belongs to the slab. This file asserts exactly that, per instance, by name.
//!
//! # What this census CANNOT claim
//!
//! **That the ENGINE produced that pyramid.** It is pure host arithmetic over the fixture's own
//! transforms — no device, no dump, no readback. The MEASURED half of the same precondition (the
//! oracle's verdict over the DUMPED pyramid) is clause 0 of `vb_occ_mixed.rs`'s gate, and the two
//! are deliberately worded differently so a red names which one fired.
//!
//! # The control, EXECUTED here
//!
//! [`nudging_one_hidden_instance_across_a_block_boundary_reds_the_precondition`] moves one hidden
//! instance by half a block and requires this file's own evaluation to FAIL on the level clause.
//! Without it, a precondition that could never fire would be indistinguishable from one that always
//! held — the vacuous-gate failure this campaign has shipped six times.

#![cfg(windows)]

use boyko_render::hzb::{HzbLayout, ScreenRect, TexelSelection, project_aabb, select_texels};

mod vb_occ_mixed_scene;

use vb_occ_mixed_scene::{
    EXTENT, HIDDEN_BLOCKS, HIDDEN_BLOCK_SIDE, MIXED_INSTANCES, MIXED_MAX_LEVEL, Role,
    SLAB_COVERS_AT_LEAST, instance_world_aabb, occluder_index, view_proj_rows,
};

/// The sentence every failure below carries. Spelled ONCE, so a red is recognisable as a fixture
/// error at a glance rather than being read as an engine defect — which is what a message that
/// merely said "the instance was not rejected" would be read as.
const FIXTURE: &str = "FIXTURE PRECONDITION — the mixed scene's geometry does not produce the \
                       intended occlusion at this framebuffer size; this is a FIXTURE error, not \
                       an engine defect.";

/// One instance's row of the census.
#[derive(Debug, Clone, Copy)]
struct Row {
    name: &'static str,
    role: Role,
    rect: ScreenRect,
    selection: TexelSelection,
    /// The half-open pixel span the SELECTED texels cover on each axis, `[x0, x1) × [y0, y1)` —
    /// the footprint the conservative `min` actually folds.
    covered: [u32; 4],
}

/// Inclusive-rect containment: does `outer` contain `inner`?
fn contains(outer: &ScreenRect, inner: [u32; 4]) -> bool {
    outer.min[0] <= inner[0]
        && outer.min[1] <= inner[1]
        && outer.max[0] >= inner[2]
        && outer.max[1] >= inner[3]
}

/// Inclusive-rect disjointness.
fn disjoint(a: &ScreenRect, b: &ScreenRect) -> bool {
    a.max[0] < b.min[0] || a.min[0] > b.max[0] || a.max[1] < b.min[1] || a.min[1] > b.max[1]
}

/// Evaluates the whole precondition over `aabbs` (one per [`MIXED_INSTANCES`] entry, in that order)
/// and returns the census rows, or the first violation as a message.
///
/// `Result` rather than `assert!` so the control below can run the SAME evaluation over a perturbed
/// fixture and require it to fail. A control that re-implements the check it is controlling is a
/// control for a different function.
fn evaluate(aabbs: &[([f32; 3], [f32; 3])]) -> Result<Vec<Row>, String> {
    assert_eq!(aabbs.len(), MIXED_INSTANCES.len(), "one world AABB per fixture instance");
    let layout = HzbLayout::new(EXTENT, EXTENT)
        .map_err(|e| format!("{FIXTURE} `HzbLayout::new({EXTENT}, {EXTENT})` refused: {e:?}"))?;
    let vp = view_proj_rows();
    let source = [EXTENT, EXTENT];

    let mut rows: Vec<Row> = Vec::with_capacity(aabbs.len());
    for (i, (mn, mx)) in aabbs.iter().enumerate() {
        let inst = MIXED_INSTANCES[i];
        let rect = project_aabb(&vp, source, *mn, *mx).map_err(|reason| {
            format!(
                "{FIXTURE} `{}` does not project to a usable screen rect ({reason:?}). Every \
                 instance of this fixture is in front of the camera and on screen by construction.",
                inst.name
            )
        })?;
        let selection = select_texels(&layout, &rect).map_err(|reason| {
            format!(
                "{FIXTURE} `{}` selects no pyramid level ({reason:?}) — its rect is \
                 {:?}..={:?}",
                inst.name, rect.min, rect.max
            )
        })?;
        let (ax, ay) = (layout.x(), layout.y());
        let xs = [
            ax.level_source_span(selection.level, selection.tx[0]),
            ax.level_source_span(selection.level, selection.tx[1]),
        ];
        let ys = [
            ay.level_source_span(selection.level, selection.ty[0]),
            ay.level_source_span(selection.level, selection.ty[1]),
        ];
        let covered = [
            xs[0].0.min(xs[1].0),
            ys[0].0.min(ys[1].0),
            xs[0].1.max(xs[1].1),
            ys[0].1.max(ys[1].1),
        ];
        rows.push(Row { name: inst.name, role: inst.role, rect, selection, covered });
    }

    let slab = rows[occluder_index()];

    // ---- The slab covers what the design says it covers -------------------------------------
    if !contains(&slab.rect, SLAB_COVERS_AT_LEAST) {
        return Err(format!(
            "{FIXTURE} the occluder's rect is {:?}..={:?}, which does not contain the \
             {SLAB_COVERS_AT_LEAST:?} region every hidden instance's aligned footprint lives in. \
             Containment, not equality, is the claim — slack here can only make the precondition \
             safer — so this fired on a slab that SHRANK.",
            slab.rect.min, slab.rect.max
        ));
    }

    // ---- Per instance ------------------------------------------------------------------------
    let mut hidden_seen = 0usize;
    for row in &rows {
        match row.role {
            Role::Occluder => {}
            Role::Filler => {
                // The filler exists to keep the EARLY depth non-empty under FORCE-LATE, where the
                // slab and it are the only two instances drawn. Outside the slab's rect, so it also
                // guarantees the early depth carries >= 2 distinct values — the SHIPPED non-vacuity
                // clause `hzb_engine_pyramid_gate.rs` asserts.
                if !disjoint(&row.rect, &slab.rect) {
                    return Err(format!(
                        "{FIXTURE} the filler `{}` at {:?}..={:?} overlaps the occluder's \
                         {:?}..={:?}. It sits outside on purpose: inside, it would contribute no \
                         depth of its own and the early depth under FORCE-LATE would be a single \
                         value.",
                        row.name, row.rect.min, row.rect.max, slab.rect.min, slab.rect.max
                    ));
                }
            }
            Role::Hidden => {
                let block = HIDDEN_BLOCKS[hidden_seen];
                hidden_seen += 1;
                if row.selection.level > MIXED_MAX_LEVEL {
                    return Err(format!(
                        "{FIXTURE} `{}` selects pyramid level {} (> MIXED_MAX_LEVEL = \
                         {MIXED_MAX_LEVEL}) for the rect {:?}..={:?}. Its rect straddles a \
                         {HIDDEN_BLOCK_SIDE}-aligned boundary, so the ALIGNED 2x2 footprint spills \
                         outside the occluder and the conservative `min` folds a background texel — \
                         after which this instance CANNOT be deferred and every count clause \
                         downstream would red on a correct engine.",
                        row.name, row.selection.level, row.rect.min, row.rect.max
                    ));
                }
                let inside_block = row.rect.min[0] >= block[0]
                    && row.rect.min[1] >= block[1]
                    && row.rect.max[0] < block[0] + HIDDEN_BLOCK_SIDE
                    && row.rect.max[1] < block[1] + HIDDEN_BLOCK_SIDE;
                if !inside_block {
                    return Err(format!(
                        "{FIXTURE} `{}` projects to {:?}..={:?}, which is not inside its \
                         {HIDDEN_BLOCK_SIDE}-aligned block at {block:?}",
                        row.name, row.rect.min, row.rect.max
                    ));
                }
                // The load-bearing clause: EVERY pixel the folded footprint covers belongs to the
                // occluder. Stated over the covered SPAN rather than over the rect, because the
                // span is what `occluder_depth` reads and the rect is not.
                let covered_inclusive =
                    [row.covered[0], row.covered[1], row.covered[2] - 1, row.covered[3] - 1];
                if !contains(&slab.rect, covered_inclusive) {
                    return Err(format!(
                        "{FIXTURE} `{}`'s level-{} footprint covers pixels {:?}, which is not \
                         inside the occluder's rect {:?}..={:?}. `occluder_depth` folds EVERY texel \
                         in that footprint, and one background texel makes `occ` the reverse-Z far \
                         plane, which KEEPS.",
                        row.name, row.selection.level, row.covered, slab.rect.min, slab.rect.max
                    ));
                }
                // Strictly BEHIND, under reverse-Z: a larger `depth_near` is nearer. Written `>=`
                // rather than `!(<)` because `project_aabb` returns `KeepReason::NonFinite` before
                // ever filling `depth_near` with a NaN, so the two forms cannot differ here.
                if row.rect.depth_near >= slab.rect.depth_near {
                    return Err(format!(
                        "{FIXTURE} `{}` has depth_near {} but the occluder's is {}. Under reverse-Z \
                         a LARGER value is NEARER, so a hidden instance must be strictly SMALLER — \
                         and strictly, because the reject predicate is a strict `<` and equality is \
                         a legitimate visible case.",
                        row.name, row.rect.depth_near, slab.rect.depth_near
                    ));
                }
            }
            Role::Visible => {
                if !disjoint(&row.rect, &slab.rect) {
                    return Err(format!(
                        "{FIXTURE} `{}` at {:?}..={:?} is NOT disjoint from the occluder's \
                         {:?}..={:?}. A marked-VISIBLE instance overlapping the slab could be \
                         deferred, and then `0 < S|K_b| < S n_defer` under FORCE-LATE would stop \
                         being derivable from the fixture.",
                        row.name, row.rect.min, row.rect.max, slab.rect.min, slab.rect.max
                    ));
                }
            }
        }
    }
    if hidden_seen != HIDDEN_BLOCKS.len() {
        return Err(format!(
            "{FIXTURE} the fixture declares {} hidden instances but {} blocks",
            hidden_seen,
            HIDDEN_BLOCKS.len()
        ));
    }

    Ok(rows)
}

/// The fixture's own world AABBs, one per instance, in [`MIXED_INSTANCES`] order.
fn fixture_aabbs() -> Vec<([f32; 3], [f32; 3])> {
    (0..MIXED_INSTANCES.len()).map(instance_world_aabb).collect()
}

// ===============================================================================================
// The census
// ===============================================================================================

/// The fixture's arithmetic about itself, checked without `boyko_render::hzb` — so a red here names
/// a broken constant table rather than a broken projection.
#[test]
fn the_mixed_fixture_is_internally_consistent() {
    vb_occ_mixed_scene::assert_fixture_invariants();
}

/// **THE ANALYTIC FIXTURE PRECONDITION, plus the census table it is read off.**
///
/// Prints one row per instance — rect, selected level, selected texels, folded footprint,
/// `depth_near` — because the numbers are what a later reader needs when a placement has to move,
/// and because a precondition whose inputs are invisible is a precondition nobody can re-derive.
#[test]
fn the_mixed_fixture_analytically_occludes() {
    let rows = evaluate(&fixture_aabbs()).unwrap_or_else(|e| panic!("{e}"));
    let slab = rows[occluder_index()];

    eprintln!(
        "VG-R3-P3-8 vb_occ_mixed census @ {EXTENT}x{EXTENT} (prev_pow2 = {EXTENT}, so level 0 IS \
         the pixel grid and `texel_of` is the identity):"
    );
    for row in &rows {
        eprintln!(
            "  {:<11} {:<9} rect=[{},{}]..[{},{}] depth_near={:.6} level={} tx={:?} ty={:?} \
             footprint=[{},{})x[{},{})",
            row.name,
            format!("{:?}", row.role),
            row.rect.min[0],
            row.rect.min[1],
            row.rect.max[0],
            row.rect.max[1],
            row.rect.depth_near,
            row.selection.level,
            row.selection.tx,
            row.selection.ty,
            row.covered[0],
            row.covered[2],
            row.covered[1],
            row.covered[3],
        );
    }
    eprintln!(
        "  occluder rect contains {SLAB_COVERS_AT_LEAST:?}; every hidden footprint is inside it; \
         every hidden depth_near < {:.6}",
        slab.rect.depth_near
    );

    // The early depth must carry at least two DISTINCT values and at least one texel `> 0.0` —
    // `hzb_engine_pyramid_gate.rs`'s SHIPPED non-vacuity clauses, restated here as a property of the
    // GEOMETRY so a fixture edit that would trip them reds on the CPU first.
    let mut depths: Vec<f32> =
        rows.iter().filter(|r| !r.role.is_marked()).map(|r| r.rect.depth_near).collect();
    depths.sort_by(|a, b| a.partial_cmp(b).expect("finite depths"));
    assert!(
        depths.len() >= 2 && depths[0] < depths[depths.len() - 1],
        "{FIXTURE} the UNMARKED instances carry {depths:?}. Under FORCE-LATE they are the only ones \
         the early scope draws, so two distinct depths among them is what keeps the early depth \
         from being a constant field — the state the shipped pyramid gate refuses to compare over."
    );
    assert!(
        depths[0] > 0.0,
        "{FIXTURE} an unmarked instance projects to depth_near {} <= 0.0, i.e. at or behind the \
         reverse-Z far plane, and would contribute no depth at all",
        depths[0]
    );
}

// ===============================================================================================
// The control — EXECUTED, not described
// ===============================================================================================

/// **The control that makes the precondition falsifiable.**
///
/// Nudges ONE hidden instance by half a block along X, so its rect straddles a
/// [`HIDDEN_BLOCK_SIDE`]-aligned boundary. `msb(tx0 ^ tx1)` then jumps above [`MIXED_MAX_LEVEL`],
/// the aligned 2×2 footprint doubles in each axis, and the fold reaches outside the occluder.
///
/// The plan names this hazard class (Bevy #14042) and round 2 then failed to carry it into the
/// fixture; this is the carry. It reds **before any GPU runs**, which is the whole reason the
/// analytic form exists beside the measured one.
#[test]
fn nudging_one_hidden_instance_across_a_block_boundary_reds_the_precondition() {
    let mut aabbs = fixture_aabbs();
    let victim = MIXED_INSTANCES
        .iter()
        .position(|i| i.role == Role::Hidden)
        .expect("invariant: the mixed fixture has hidden instances");

    // Half a block, expressed in the WORLD units the instance lives in: at `view_distance` the
    // pixel-to-world scale is `2 * tan(fov/2) * d / EXTENT` per pixel, and the nudge is along the
    // camera's own right vector so it moves the rect in X and nothing else.
    let inst = MIXED_INSTANCES[victim];
    let nudged_centre = vb_occ_mixed_scene::world_position(
        [inst.pixel[0] + (HIDDEN_BLOCK_SIDE as f32) * 0.5, inst.pixel[1]],
        inst.view_distance,
    );
    let original_centre = vb_occ_mixed_scene::world_position(inst.pixel, inst.view_distance);
    let delta = nudged_centre - original_centre;
    let (mn, mx) = aabbs[victim];
    aabbs[victim] = (
        [mn[0] + delta.x, mn[1] + delta.y, mn[2] + delta.z],
        [mx[0] + delta.x, mx[1] + delta.y, mx[2] + delta.z],
    );

    let err = evaluate(&aabbs).expect_err(
        "the nudged fixture PASSED the precondition. A control that cannot fire leaves the \
         precondition indistinguishable from a tautology — and this campaign has shipped six gates \
         that were green in exactly the state they existed to catch.",
    );
    eprintln!("control (block-straddle) fired as required:\n  {err}");
    assert!(
        err.contains("MIXED_MAX_LEVEL") || err.contains("not inside its"),
        "the nudged fixture was rejected, but not by the LEVEL/BLOCK clause — got {err:?}. A \
         rejection for another reason would leave the clause this control exists for unexercised."
    );
}
