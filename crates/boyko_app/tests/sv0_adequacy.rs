//! **The SV0 rung-S1 adequacy gate** (`docs/VB-SV0-SDF-SHADOW-PLAN.md`, "S1 — the fixture") — the
//! CPU-side, GPU-FREE proof that the `vb_both_sdf` / `vb_both_sdf_tex` fixtures can actually
//! exercise SV0.
//!
//! # What this gate is for
//!
//! Every VB golden shipped before this campaign renders an EMPTY SDF edit list, so SV0's shadow
//! and contact-AO terms are both exactly `1.0` on them. A byte-identity gate quantified over such
//! a scene is VACUOUS — green over an empty selection, no matter what SV0 does. The fixtures make
//! the selection non-empty; THIS file proves it non-empty, on the CPU, before any shader work.
//!
//! # The four gates (plan §S1)
//!
//! 1. `edit_count > 0` — [`sv0_fixture_gathers_a_non_empty_edit_list`];
//! 2. at least [`SV0_MIN_SHADOWED_PIXELS`] covered mesh pixels are fully shadowed by the SDF body
//!    — [`sv0_fixture_clears_the_shadowed_pixel_floor`];
//! 3. at least [`SV0_MIN_AO_PIXELS`] covered mesh pixels are actually DARKENED by the contact-AO
//!    term — [`sv0_fixture_clears_the_contact_ao_pixel_floor`];
//! 4. the re-sited R11 tripwire: `SdfEditStaging::is_dirty()` is false after the one-shot upload
//!    and STAYS false across later frames —
//!    [`sv0_edit_staging_is_not_dirty_after_the_one_shot_upload`] (GPU-free) and
//!    [`sv0_edit_staging_stays_clean_under_the_real_runner`] (`#[ignore]`, real runner). The two
//!    halves cover DIFFERENT regression shapes and gate 4 needs both — see the first one's doc.
//!
//! Plus the plan's two required mutations:
//! * delete the occluder → gates 1, 2 and 3 all fail. Demonstrated in two runnable halves,
//!   [`sv0_a_harness_without_the_shared_spawn_gathers_nothing`] (gate 1) and
//!   [`sv0_removing_the_body_empties_both_counts`] (gates 2 and 3), because the mutation itself is a
//!   source deletion. What makes it red at all is that the body is spawned ONLY through
//!   [`sv0_scene::spawn_scene`] and gates 2/3 are quantified over the GATHERED edit list.
//! * [`sv0_sliding_the_body_out_along_the_light_separates_the_two_counts`] — slide it out along the
//!   light and the AO count collapses while the shadow count is unchanged. This is the one that
//!   proves gates 2 and 3 are not one assertion wearing two names.
//!
//! # Why the counts are trustworthy
//!
//! The instrument ([`sv0_oracle`]) rasterizes the fixtures' OWN geometry
//! ([`sv0_scene::scene_sphere_mesh`]) with the fixtures' OWN camera, projected through the
//! engine's own `ViewUniform` / `forward_view_proj_rows` construction sites — not a re-derivation
//! that could drift from what the GPU draws. The shadow predicate runs the SHIPPED
//! `sdf_soft_shadow` leaf (`boyko_shaderdsl::shadow::sdf_soft_shadow_body` over `EvalCf`, the same
//! generator that emits the committed HLSL span), and the field is the frozen
//! `boyko_sdf_math::sdf_edit_list`.
//!
//! # What this gate certifies for the TEXTURED fixture, and what it does not
//!
//! Both fixtures share this scene exactly — same mesh, same instances, same camera, same edit
//! list — so the coverage and the two counts apply to `vb_both_sdf_tex` as much as to
//! `vb_both_sdf`, with ONE qualification worth stating plainly. Both predicates run along the
//! SHADING normal, and on the textured fixture the normal map perturbs it, so the quantity
//! certified there is the GEOMETRIC predicate, not the normal-mapped one.
//!
//! That does not weaken the gate, because the margins are geometric rather than marginal: the
//! AO-darkened set is a `31.8°`-wide cap and the body sits `0.25` from the surface against a
//! `0.579506` darkening gap, so a normal perturbation would have to swing tens of degrees
//! COHERENTLY across hundreds of pixels to empty it, which a bump map does not do. But an SV0 gate
//! that ever needs the textured fixture's AO set to be BIT-identical to the flat one must not read
//! that claim out of this file: it is not asserted here.

use std::path::PathBuf;

use boyko_app::prelude::*;
use boyko_render::mesh::Vertex;
use boyko_render::{SdfEditStaging, SdfPlugin, collect_sdf_edits};
use boyko_scene::ViewUniform;

mod sv0_oracle;
mod sv0_scene;

use sv0_oracle::{Coverage, MeshSelection, OracleVertex};

// ===========================================================================================
// The floors
// ===========================================================================================

// Both floors come from the SAME derivation, which is stated once here and referenced by each.
//
// **The floors are derived from rung S4(ii), NOT from any measurement of this scene** (plan Rev 5,
// §S1 gate (3)). Rev 4's rule — *fixed BEFORE the fixture is authored, never lowered* — is
// unsatisfiable in this ordering, because authoring the fixture requires measuring it; both S1
// implementers reported so. The rule's PURPOSE (no floor fitted to an observed count) is preserved
// by deriving the number from a DOWNSTREAM requirement instead:
//
//   * S4(ii) accepts an armed-vs-unarmed changed-pixel count in `[1%, 60%]` of covered mesh pixels.
//   * A fixture that cannot clear S4's LOWER band is useless downstream, whatever it measures here.
//   * So each S1 floor is `2×` that lower band: a fixture must arm each term over at least twice
//     the minimum S4 will accept.
//   * This scene covers ~28.4k mesh pixels, so 1% is ~284 and `2×` is ~568, rounded to the round
//     number **500** — 1.76× S4's lower band, chosen so no digit of the floor is fitted to a
//     measurement.
//
// The inputs are S4(ii)'s band and the raster's covered-pixel count. Neither observed count below
// is an input. Both are recorded beside their gate for DIAGNOSIS — so a future drop can be
// attributed (did the fixture change, or the predicate?) — and are explicitly labelled as not the
// basis for the number.
//
// The "do not edit these literals to make a failing run pass" discipline still applies to both:
// a floor may be RAISED on new evidence, never lowered to rescue a failing run. A run that falls
// below one means the FIXTURE stopped being adequate, and the fixture is what must change.

/// **Derived from S4(ii) — do not edit this literal to make a failing run pass.**
///
/// The minimum number of covered mesh pixels that must be FULLY shadowed by the SDF body for any
/// downstream SV0 shadow gate to be non-vacuous (plan §S1 gate 2). See the derivation above: it is
/// `2×` S4(ii)'s lower band over this raster's covered-pixel count, rounded down to `500`.
///
/// # Recorded observation — NOT the basis for the number above
///
/// At the shipped placement this oracle counts **1477** of **28362** covered mesh pixels. Recorded
/// for diagnosis only. Note the predicate UNDERCOUNTS by construction: it counts only hard-hit
/// pixels, never the penumbra the Quilez term also darkens (see [`sv0_oracle::is_fully_shadowed`]),
/// so the pixels SV0 actually moves are strictly more than this.
const SV0_MIN_SHADOWED_PIXELS: usize = 500;

/// **Derived from S4(ii) — do not edit this literal to make a failing run pass.**
///
/// The minimum number of covered mesh pixels whose contact-AO term actually darkens, i.e. whose
/// `sdf_ao` falls below its far-field `1.0` (plan §S1 gate 3). Same derivation as
/// [`SV0_MIN_SHADOWED_PIXELS`] — `2×` S4(ii)'s lower band, rounded to `500` — and deliberately the
/// same number: S4(ii) applies the same band to both terms.
///
/// # Why this gate exists separately
///
/// SV0 ships two independently gated terms (plan §3.1 gives them separate bits), and every gate
/// in the plan's Rev 3 was satisfied by the shadow half ALONE — the AO half's "sits near a mesh
/// surface" requirement was prose, not a gate. `sdf_ao` returns exactly `1.0` when the body is
/// farther than a `0.579506` surface-to-surface gap, which is the identical vacuity trap this rung
/// closes for the shadow half. Hence a second floor, with its own count and its own mutation.
///
/// # Recorded observation — NOT the basis for the number above
///
/// At the shipped placement this oracle counts **958** of **28362** covered mesh pixels, and at
/// both [`sv0_scene::SDF_CENTER_DISTANCE_AO_DEFEATING`] placements exactly 0. An independent
/// analytic-sphere raycast (no tessellation, no interpolated normals) of the same cap predicts 966,
/// i.e. the measured count sits 0.8% under it — the inscribed `uv_sphere(28, 40)` deficit, which is
/// what a correct instrument should show.
///
/// That count is the CORRECTED one. Rung S1's first implementation used a "does any tap see the
/// body" predicate and counted 2523 — 2.6× too many, in the false-GREEN direction, because taps
/// with `d > h` contribute negative terms the shortcut ignored (review C1; see
/// [`sv0_oracle::sdf_ao`]). Had this floor been fitted to that inflated observation (it was: the
/// literal was 1500) it would now be FAILING at 958; deriving it from S4(ii) instead is what makes
/// it survive its own instrument being corrected.
const SV0_MIN_AO_PIXELS: usize = 500;

// ===========================================================================================
// Shared scene construction
// ===========================================================================================

/// The fixtures' projection, taken from the engine's OWN construction site.
///
/// `forward_view_proj_rows` is what the Forward / VisibilityBuffer raster uploads
/// (`boyko_app/src/runner.rs:1751-1784`), so the oracle places pixels exactly where the VB fixture
/// does. Its `clip.x` / `clip.y` / `clip.w` rows are byte-identical to `marcher_view_proj_rows`'
/// (both are documented as sharing the extent-derived aspect and the marcher y-flip); only the
/// depth row differs, and coverage is invariant under that monotone remap — so this choice cannot
/// silently misplace a pixel relative to the Deferred sibling either.
fn scene_view_proj_rows() -> [[f32; 4]; 4] {
    let view = ViewUniform::from_camera(
        sv0_scene::camera_transform().to_affine(),
        sv0_scene::camera_projection(),
    );
    boyko_render::forward_view_proj_rows(&view, sv0_scene::DUMP_EXTENT, sv0_scene::DUMP_EXTENT)
}

/// Rasterizes the fixtures' five-sphere row exactly as they spawn it.
///
/// Uses [`sv0_scene::scene_sphere_mesh`] — the same generator both fixtures hand to
/// `register_mesh_vb` — and [`sv0_scene::mesh_center`] for the five instance translations, so the
/// oracle's coverage is the fixtures' coverage rather than an analytic approximation of it.
fn scene_coverage() -> Coverage {
    let (verts, idx) = sv0_scene::scene_sphere_mesh();
    let oracle_verts: Vec<OracleVertex> = verts
        .iter()
        .map(|v: &Vertex| OracleVertex { position: v.position, normal: v.normal })
        .collect();
    let instances: Vec<[f32; 3]> =
        (0..sv0_scene::MESH_ROW_COUNT).map(sv0_scene::mesh_center).collect();

    sv0_oracle::rasterize(
        &oracle_verts,
        &idx,
        &instances,
        scene_view_proj_rows(),
        sv0_scene::DUMP_EXTENT,
        sv0_scene::DUMP_EXTENT,
        sv0_scene::CAMERA_NEAR,
    )
}

/// The mesh pixels SV0 can shade for a body placed `distance` along the unit light — the
/// denominator the SEPARATING MUTATION quantifies over at each of its placements.
///
/// Reconstructs the edit list from [`sv0_scene::sdf_body_edit_at`] rather than gathering it,
/// because the mutation's whole point is to move the body to a placement the scene does not spawn.
/// The shipped placement is NOT read through here — see [`gathered_selection`].
fn selection_at(coverage: &Coverage, distance: f32) -> (Vec<boyko_render::SdfEdit>, MeshSelection) {
    let edits = vec![sv0_scene::sdf_body_edit_at(distance)];
    let selection = sv0_oracle::select_mesh_pixels(coverage, &edits, sv0_scene::CAMERA_EYE);
    (edits, selection)
}

/// **The edit list AS THE RENDERED SCENE PRODUCES IT**, plus the mesh pixels SV0 can shade under
/// it — the shared denominator gate 2, gate 3 and the S4(ii) comparator all quantify over.
///
/// # Why this gathers instead of calling `sdf_body_edit()` (review C2)
///
/// The plan's first S1 mutation is "remove the SDF spawns → gates (1), (2), (3) all fail". If gates
/// 2 and 3 reconstructed their own edit list, that mutation would red gate 1 only: the two counts
/// would go on measuring a body the frame no longer contains, and a fixture rendering the
/// boot-seeded EMPTY list would keep three green gates. Routing the counts through
/// `collect_sdf_edits`' output — produced by the SAME [`sv0_scene::spawn_scene`] the two dump
/// fixtures call — is what makes all three fail together.
fn gathered_selection(coverage: &Coverage) -> (Vec<boyko_render::SdfEdit>, MeshSelection) {
    let app = gathered_app();
    let edits = app.world().resource::<SdfEditStaging>().edits().to_vec();
    let selection = sv0_oracle::select_mesh_pixels(coverage, &edits, sv0_scene::CAMERA_EYE);
    (edits, selection)
}

// ===========================================================================================
// The instrument's own soundness
// ===========================================================================================

/// The rasterizer is sound on this scene BEFORE any count is read off it.
///
/// Three things could make every count below a quiet lie, and none of them announces itself:
/// a triangle straddling the near plane (this rasterizer drops such triangles whole rather than
/// clipping them), an empty raster, and an SDF body that has drifted in front of the mesh and is
/// eating the denominator. Each is asserted explicitly.
#[test]
fn sv0_oracle_raster_is_sound_on_the_fixture_scene() {
    let coverage = scene_coverage();
    assert_eq!(
        coverage.near_rejected_triangles, 0,
        "the oracle does not clip polygons: {} triangle(s) crossed the near plane, so coverage \
         was silently dropped and every count below is a lower bound of unknown depth",
        coverage.near_rejected_triangles
    );

    let covered = coverage.covered_count();
    assert!(
        covered > 10_000,
        "anti-vacuity: the five-sphere row must cover a substantial part of the {0}x{0} raster \
         (covered {covered})",
        sv0_scene::DUMP_EXTENT
    );

    let (_, selection) = gathered_selection(&coverage);
    assert_eq!(
        selection.sdf_occluded, 0,
        "the SDF body must not eclipse the mesh row — it owns its own pixels beside the spheres, \
         not in front of them ({} covered mesh pixels are behind it)",
        selection.sdf_occluded
    );
    assert_eq!(
        selection.len(),
        covered,
        "with nothing occluded the selection IS the covered set"
    );
}

// ===========================================================================================
// Gate 1 — the edit list is non-empty
// ===========================================================================================

/// Spawns ONLY the SDF occluder — for the real-runner staging tripwire, which asserts a property of
/// `SdfEditStaging` and needs no mesh, no material and no camera.
///
/// Deliberately NOT routed through [`sv0_scene::spawn_scene`]: that test runs a real windowed
/// device, where a `MeshBundle` naming an unregistered slot is not the inert thing it is here. Its
/// scene is not measured by any count in this file, so it is outside review C2's seam.
fn spawn_body_only(mut commands: Commands) {
    sv0_scene::spawn_sdf_body(&mut commands);
}

/// The GPU-free gather harness — [`sv0_scene::gathered_app`], which is where it moved at rung S4
/// so the S4 arming matrix measures the SAME gathered edit list these gates do (review C2's seam,
/// applied to the second consumer). The construction is unchanged.
fn gathered_app() -> App {
    sv0_scene::gathered_app()
}

/// **Gate 1.** `collect_sdf_edits` finds the fixtures' occluder, so the rendered edit list is
/// non-empty — the single fact every other VB pin lacks.
///
/// The scene is spawned through [`sv0_scene::spawn_scene`], the same entry point `vb_both_sdf.rs`
/// and `vb_both_sdf_tex.rs` call, so this asserts a property of what those fixtures RENDER rather
/// than of a harness-local reconstruction.
#[test]
fn sv0_fixture_gathers_a_non_empty_edit_list() {
    let app = gathered_app();
    let staging = app.world().resource::<SdfEditStaging>();
    assert_eq!(
        staging.edits().len(),
        1,
        "the S1 scene spawns exactly one SdfPrimitive; the gather must find it"
    );

    // The gathered bytes must BE the scene's placement — a gather that found some other primitive
    // would satisfy the count while measuring a different scene than gates 2 and 3 do.
    let expected = sv0_scene::sdf_body_edit();
    let got = staging.edits()[0];
    assert_eq!(got.center[0].to_bits(), expected.center[0].to_bits(), "edit center.x");
    assert_eq!(got.center[1].to_bits(), expected.center[1].to_bits(), "edit center.y");
    assert_eq!(got.center[2].to_bits(), expected.center[2].to_bits(), "edit center.z");
    assert_eq!(got.params[0].to_bits(), expected.params[0].to_bits(), "edit radius");
}

// ===========================================================================================
// Gates 2 and 3 — the two counts
// ===========================================================================================

/// **Gate 2.** Enough covered mesh pixels are fully shadowed by the SDF body.
///
/// The predicate is `dot(N, L) > SHADOW_NDOTL_EPS` AND the shipped `sdf_soft_shadow` leaf
/// returning `0.0` from `P + face_N * SHADOW_NORMAL_BIAS`. It counts FULLY-occluded pixels and
/// therefore UNDERCOUNTS the pixels SV0 actually darkens (the Quilez accumulator drops below `1.0`
/// on the whole penumbra) — see [`sv0_oracle::is_fully_shadowed`] for why that is safe but not
/// equivalent.
///
/// Quantified over the GATHERED edit list ([`gathered_selection`]), so deleting the body from
/// [`sv0_scene::spawn_scene`] reds this gate and not only gate 1.
#[test]
fn sv0_fixture_clears_the_shadowed_pixel_floor() {
    let coverage = scene_coverage();
    let (edits, selection) = gathered_selection(&coverage);
    let light = sv0_scene::sun_dir_unit();

    let shadowed = sv0_oracle::shadowed_pixel_count(&coverage, &selection, &edits, light);
    println!(
        "S1 gate 2: {shadowed} fully-shadowed of {} covered mesh pixels (floor {SV0_MIN_SHADOWED_PIXELS})",
        selection.len()
    );
    assert!(
        shadowed >= SV0_MIN_SHADOWED_PIXELS,
        "the fixture is NOT adequate for SV0's shadow term: {shadowed} fully-shadowed covered \
         mesh pixels, floor {SV0_MIN_SHADOWED_PIXELS}. Fix the FIXTURE (move the SDF body), not \
         the floor."
    );
}

/// **Gate 3.** Enough covered mesh pixels carry a contact-AO term that actually DARKENS them.
///
/// `sdf_ao` returns exactly `1.0` — SV0's no-op — unless its accumulated
/// `occ = Σ (h_i − d_i)·AO_FALLOFF^i` is positive, which on-axis means a surface-to-surface gap
/// below `0.579506`. This is the half the plan's Rev 3 left ungated, and the half a body placed for
/// shadowing alone silently fails.
///
/// The predicate is the shipped accumulation, not "some tap sees the body" — the latter counts
/// pixels the leaf leaves at exactly `1.0` and inflates this number ~2.6× on this fixture
/// (review C1; see [`sv0_oracle::sdf_ao`]). Quantified over the GATHERED edit list, for the same
/// reason as gate 2.
#[test]
fn sv0_fixture_clears_the_contact_ao_pixel_floor() {
    let coverage = scene_coverage();
    let (edits, selection) = gathered_selection(&coverage);

    let ao = sv0_oracle::contact_ao_pixel_count(&coverage, &selection, &edits);
    println!(
        "S1 gate 3: {ao} contact-AO of {} covered mesh pixels (floor {SV0_MIN_AO_PIXELS})",
        selection.len()
    );
    assert!(
        ao >= SV0_MIN_AO_PIXELS,
        "the fixture is NOT adequate for SV0's contact-AO term: {ao} covered mesh pixels where \
         sdf_ao darkens, floor {SV0_MIN_AO_PIXELS}. Fix the FIXTURE (bring the SDF body inside the \
         {AO_DARKENING_GAP} surface-to-surface darkening gap), not the floor."
    );
}

/// The on-axis surface-to-surface gap below which the shipped `sdf_ao` accumulation darkens a
/// pixel: `2·Σ h_i·AO_FALLOFF^i / Σ AO_FALLOFF^i = 2.490811 / 4.298162`.
///
/// Named here rather than inlined because it is the number the failure text must quote — the
/// `AO_TAPS · AO_STEP` probe REACH (`0.5`) and `2 · AO_TAPS · AO_STEP` (`1.0`) are both larger and
/// both wrong for this purpose, and quoting either is what review finding C1 was about.
const AO_DARKENING_GAP: f32 = 0.579_506;

// ===========================================================================================
// The separating mutation — the control that makes gates 2 and 3 two assertions, not one
// ===========================================================================================

/// **The plan's required mutation, RUN rather than described.**
///
/// Slide the SDF body OUT along the light ([`sv0_scene::SDF_CENTER_DISTANCE_AO_DEFEATING`]) and
/// the two counts must part company: the AO count falls BELOW its floor while the shadow count
/// stays ABOVE its own. Without this, gates 2 and 3 could be one assertion wearing two names —
/// which is exactly what the plan's earlier revision shipped.
///
/// # What the separation rests on, and how far the proof actually reaches
///
/// The CONTINUOUS shadow-hit condition is `r_mesh · sin∠(n, L) < r_sdf`, in which the placement
/// distance does not appear at all: sliding the body along `L` changes WHEN a continuous ray hits,
/// never WHETHER it does. The AO condition, by contrast, is purely a function of the
/// surface-to-surface gap against the `0.579506` darkening threshold. That asymmetry is why the two
/// counts can be separated at all, and it is a proof about the continuous problem.
///
/// The SHIPPED march is not continuous. It advances `t += max(d / FIELD_LIPSCHITZ_L,
/// SHADOW_MINT_STEP)` with `SHADOW_MINT_STEP = 0.008`, so wherever `d / 1.414 < 0.008` the march
/// steps FURTHER than the field's own clearance and can stride across a thin near-tangential
/// corridor. The discrete hit set is therefore not exactly the continuous cap, and its boundary
/// ring — the pixels within one step of tangency — CAN move with `D`.
///
/// So the `assert_eq!` below states a MEASURED invariance at the swept placements, not a theorem.
/// A red on it means the boundary ring moved under the discrete schedule; it does NOT mean the
/// fixture broke, and the first thing to check is the magnitude (a handful of pixels is the ring,
/// hundreds is a real change of scene).
#[test]
fn sv0_sliding_the_body_out_along_the_light_separates_the_two_counts() {
    let coverage = scene_coverage();
    let light = sv0_scene::sun_dir_unit();

    let (shipped_edits, shipped_sel) = gathered_selection(&coverage);
    let shipped_shadow =
        sv0_oracle::shadowed_pixel_count(&coverage, &shipped_sel, &shipped_edits, light);
    let shipped_ao = sv0_oracle::contact_ao_pixel_count(&coverage, &shipped_sel, &shipped_edits);
    println!(
        "S1 mutation baseline @ D={}: shadow {shipped_shadow}, ao {shipped_ao}, sdf_occluded {}",
        sv0_scene::SDF_CENTER_DISTANCE,
        shipped_sel.sdf_occluded
    );

    for distance in sv0_scene::SDF_CENTER_DISTANCE_AO_DEFEATING {
        let (edits, selection) = selection_at(&coverage, distance);
        let shadowed = sv0_oracle::shadowed_pixel_count(&coverage, &selection, &edits, light);
        let ao = sv0_oracle::contact_ao_pixel_count(&coverage, &selection, &edits);
        println!(
            "S1 mutation @ D={distance}: shadow {shadowed}, ao {ao}, sdf_occluded {}",
            selection.sdf_occluded
        );

        // The instrument's own soundness, RE-asserted per placement (review W4). Only the shipped
        // placement was checked before, so a mutation value that happened to put the body in front
        // of a sphere would shrink the denominator and red the `assert_eq!` below for a reason
        // having nothing to do with the property under test — an undiagnosable failure.
        assert_eq!(
            selection.sdf_occluded, 0,
            "the mutation must not move the body in FRONT of the mesh row: at D={distance} it \
             eclipses {} covered mesh pixels, which shrinks the denominator and makes every count \
             below incomparable with the baseline's",
            selection.sdf_occluded
        );
        assert!(
            ao < SV0_MIN_AO_PIXELS,
            "the mutation must DEFEAT the AO gate: at D={distance} the surface-to-surface gap is \
             {gap} against the {AO_DARKENING_GAP} darkening threshold, yet {ao} pixels are still \
             darkened by sdf_ao (floor {SV0_MIN_AO_PIXELS})",
            gap = distance - sv0_scene::MESH_SPHERE_RADIUS - sv0_scene::SDF_SPHERE_RADIUS,
        );
        assert!(
            shadowed >= SV0_MIN_SHADOWED_PIXELS,
            "the mutation must leave the SHADOW gate standing — otherwise it moved both counts \
             and proves nothing about their independence: at D={distance} only {shadowed} pixels \
             are shadowed (floor {SV0_MIN_SHADOWED_PIXELS})"
        );
        assert_eq!(
            shadowed, shipped_shadow,
            "MEASURED invariance broken: the continuous shadow-hit bound \
             `r_mesh * sin(angle(n, L)) < r_sdf` carries no placement distance, and the discrete \
             march reproduced that at every placement swept so far. {shadowed} at D={distance} vs \
             {shipped_shadow} at D={}. A small delta is the near-tangential boundary ring moving \
             under the `SHADOW_MINT_STEP` floor (see this test's doc), not a broken fixture",
            sv0_scene::SDF_CENTER_DISTANCE
        );
    }
}

/// **The plan's other required mutation, HALF 1 OF 2: the gather is the body's only source.**
///
/// The mutation is "delete `spawn_sdf_body` from [`sv0_scene::spawn_scene`] → gates 1, 2, 3 all
/// fail". It cannot be *executed* as a test — the edit under test is a source deletion — so it is
/// demonstrated in two runnable halves that together cover all three gates:
///
/// * **this test**: an app built exactly like [`gathered_app`] but WITHOUT the shared spawn gathers
///   an EMPTY list. Since `spawn_scene` is the only thing `gathered_app` adds, gate 1's non-emptiness
///   comes from there and nowhere else — nothing in `SdfPlugin`, `App::new()` or the boot seeds an
///   edit. Delete the body and gate 1 asserts `1 == 0`.
/// * [`sv0_removing_the_body_empties_both_counts`]: over an EMPTY edit list — which is exactly what
///   the gather above returns — both counts are zero, so gates 2 and 3 fail against their floors.
///
/// Gates 2 and 3 read the GATHERED list ([`gathered_selection`]), which is what joins the two
/// halves: the empty list the first half proves you would get is the list the second half shows
/// both counts collapse over.
#[test]
fn sv0_a_harness_without_the_shared_spawn_gathers_nothing() {
    let mut app = App::new();
    app.add_plugins(SdfPlugin);
    // Deliberately NO `add_startup_system(spawn_scene_system)` — this is `gathered_app()` minus the
    // one line the mutation deletes.
    app.finish();
    app.world_mut().run_system(collect_sdf_edits);

    assert_eq!(
        app.world().resource::<SdfEditStaging>().edits().len(),
        0,
        "nothing but sv0_scene::spawn_scene puts an edit in this gather — if the boot, SdfPlugin \
         or App::new() seeded one, gate 1 would be green with the body deleted and the fixtures \
         would render an edit list nobody authored"
    );
}

/// **The plan's other required mutation, HALF 2 OF 2: over an empty edit list both counts vanish.**
///
/// This is the control that proves the counts are driven by the OCCLUDER and not by some artifact
/// of the raster, the camera or the predicates themselves. It is the exact state every VB golden
/// shipped before this campaign renders — an empty edit list — and it is where SV0's two terms are
/// identically `1.0`, which is the vacuity the whole rung exists to refute.
///
/// Both counts must be EXACTLY zero, not merely below their floors: with no edits in the field
/// there is nothing for a shadow ray to hit and nothing for an AO tap to find, so any nonzero
/// count would mean a predicate is reading something other than the field.
///
/// See [`sv0_a_harness_without_the_shared_spawn_gathers_nothing`] for the other half.
#[test]
fn sv0_removing_the_body_empties_both_counts() {
    let coverage = scene_coverage();
    let light = sv0_scene::sun_dir_unit();
    let empty: Vec<boyko_render::SdfEdit> = Vec::new();
    let selection = sv0_oracle::select_mesh_pixels(&coverage, &empty, sv0_scene::CAMERA_EYE);

    assert_eq!(
        selection.len(),
        coverage.covered_count(),
        "with no SDF body nothing can be SDF-owned"
    );
    assert_eq!(
        sv0_oracle::shadowed_pixel_count(&coverage, &selection, &empty, light),
        0,
        "an empty edit list has nothing to cast a shadow — a nonzero count means the predicate \
         is not reading the field"
    );
    assert_eq!(
        sv0_oracle::contact_ao_pixel_count(&coverage, &selection, &empty),
        0,
        "an empty edit list leaves sdf_ao saturated at its far-field 1.0 on every pixel"
    );
}

// ===========================================================================================
// Gate 4 — the re-sited R11 tripwire
// ===========================================================================================

/// **Gate 4, GPU-free.** The edit list is a ONE-SHOT boot-static upload: after the host's
/// `mark_uploaded()` the staging stays clean across every later frame.
///
/// The R11 hazard the plan names is "the edit list becomes per-frame dirty", which under VB would
/// need a barrier that does not exist. The original guard was a `debug_assert!` at the upload
/// site — inside the frame loop, and therefore compiled out in release, which is where the
/// goldens run. This assertion is profile-independent.
///
/// # Exactly which regression shape this covers — and which it does NOT (review W1)
///
/// `collect_sdf_edits` is called from ONE place today, `boyko_app/src/runner.rs:589`, imperatively,
/// after `finish()`. `SdfPlugin::build` inserts the staging resource and registers NOTHING
/// (`boyko_render/src/sdf_edit.rs:150-153`). So nothing reachable from `app.update()` can dirty the
/// staging in the current shape, and this loop is a TRIPWIRE for a future one, not a demonstration
/// against the present one.
///
/// * **Covered:** a future rung that registers a gather (or any staging write) into a per-frame
///   SCHEDULE — `add_system(collect_sdf_edits)`, an observer, a hook. `app.update()` runs schedules,
///   so this reds.
/// * **NOT covered:** a second imperative `run_system(collect_sdf_edits)` added inside the RUNNER's
///   frame loop — arguably the most likely R11 regression, since that is where the one existing
///   call already lives. `app.update()` does not execute the runner's loop, so this test stays
///   green through it.
///
/// The runner-loop shape is covered ONLY by the `#[ignore]`d
/// [`sv0_edit_staging_stays_clean_under_the_real_runner`]. **Gate 4 is therefore not discharged by
/// a green `cargo test` alone** — that sibling must be RUN on hardware for the gate to count.
///
/// GPU-free by construction: `SdfPlugin::build` only inserts the staging resource, and the gather
/// is a pure `Query<&SdfPrimitive>` walk with no device, material or path input.
#[test]
fn sv0_edit_staging_is_not_dirty_after_the_one_shot_upload() {
    let mut app = gathered_app();
    assert!(
        app.world().resource::<SdfEditStaging>().is_dirty(),
        "the gather must mark the staging dirty — otherwise the host's one-shot write never runs \
         and the fixture renders the boot-seeded EMPTY list, the exact vacuity S1 exists to close"
    );

    // The host's own frame-1 action, verbatim (`runner.rs:1195`).
    app.world_mut().resource_mut::<SdfEditStaging>().mark_uploaded();

    for frame in 1..=3u32 {
        app.update();
        assert!(
            !app.world().resource::<SdfEditStaging>().is_dirty(),
            "R11 tripwire: the edit list went dirty again on frame {frame}. The upload is a \
             one-shot boot-static write with no barrier for a per-frame rewrite under VB"
        );
    }
}

/// **Gate 4, against the REAL runner.** The plan's literal wording: drive the windowed runner for
/// at least two frames and assert the staging is clean afterwards.
///
/// `#[ignore]`: needs a real windowed GPU device. The orchestrator runs it on hardware with
/// `BOYKO_DISABLE_VALIDATION=1 BOYKO_WIN_HIDDEN=1 BOYKO_WINDOW_FRAMES=2` and `--test-threads=1`.
/// The frame cap is asserted rather than defaulted: without it `app.run()` spins forever and the
/// "≥2 frames" the invariant is quantified over would never be established.
///
/// **This half is load-bearing, not a nicety.** It is the ONLY half that covers the runner's own
/// frame loop — where `collect_sdf_edits`' single call site already lives, and therefore where a
/// second, per-frame call is most likely to be added. The GPU-free sibling above cannot see that
/// shape at all (see its doc). Gate 4 is discharged only once this has been run on hardware.
///
/// Spawns [`spawn_body_only`] rather than the shared scene: it asserts a property of
/// `SdfEditStaging`, needs no mesh/material/camera, and this binary registers no GPU mesh for a
/// `MeshHandle` to name.
#[test]
#[ignore = "needs a real windowed GPU device; run with BOYKO_WINDOW_FRAMES=2 and --test-threads=1"]
#[cfg(windows)]
fn sv0_edit_staging_stays_clean_under_the_real_runner() {
    let frames: u64 = std::env::var("BOYKO_WINDOW_FRAMES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    assert!(
        frames >= 2,
        "this test asserts an invariant over >=2 frames: set BOYKO_WINDOW_FRAMES=2 (or more), \
         otherwise app.run() never terminates"
    );

    let mut app = App::new();
    app.add_plugins(EnginePlugins::window(
        "boyko_engine sv0 edit staging",
        sv0_scene::DUMP_EXTENT,
        sv0_scene::DUMP_EXTENT,
    ));
    app.add_startup_system(spawn_body_only);
    app.run();

    assert!(
        !app.world().resource::<SdfEditStaging>().is_dirty(),
        "R11 tripwire: after {frames} real frames the SDF edit staging is still dirty — the \
         one-shot boot-static upload has become a per-frame rewrite, which under VB has no barrier"
    );
}

// ===========================================================================================
// The S4(ii) changed-pixel comparator
// ===========================================================================================

/// Writes a 32-bpp BMP in exactly the shape `boyko_app::host_dump::write_bmp` emits — 54-byte
/// header, `BI_RGB`, POSITIVE height (bottom-up rows) — from TOP-DOWN BGRA input.
///
/// Deliberately a separate implementation from the decoder under test: a round-trip through the
/// decoder's own inverse would agree with itself no matter which way it flipped the rows.
fn write_test_bmp(path: &std::path::Path, width: u32, height: u32, bgra_top_down: &[[u8; 4]]) {
    let row_bytes = (width as usize) * 4;
    let data_len = row_bytes * (height as usize);
    let mut out = Vec::with_capacity(54 + data_len);
    out.extend_from_slice(b"BM");
    out.extend_from_slice(&((54 + data_len) as u32).to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&54u32.to_le_bytes());
    out.extend_from_slice(&40u32.to_le_bytes());
    out.extend_from_slice(&(width as i32).to_le_bytes());
    out.extend_from_slice(&(height as i32).to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&32u16.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&(data_len as u32).to_le_bytes());
    out.extend_from_slice(&[0u8; 16]);
    for row in (0..height as usize).rev() {
        for x in 0..width as usize {
            out.extend_from_slice(&bgra_top_down[row * (width as usize) + x]);
        }
    }
    std::fs::write(path, &out).expect("invariant: the test BMP path is writable");
}

/// The S4(ii) comparator counts ONLY the selected pixels, and reads the dumps' rows in the right
/// order.
///
/// Two independent things are pinned here, because getting either wrong yields a plausible number
/// rather than a failure:
///
/// * the denominator is the [`MeshSelection`], not the frame — a difference OUTSIDE the selection
///   must not move the fraction at all;
/// * `read_bmp32` un-flips the writer's bottom-up rows — a pixel changed at a known TOP-DOWN
///   coordinate must be reported at that coordinate, not at its vertical mirror.
#[test]
fn sv0_changed_pixel_comparator_is_scoped_to_the_selection_and_row_ordered() {
    let width = 4u32;
    let height = 4u32;
    let count = (width * height) as usize;

    let base = vec![[10u8, 20, 30, 255]; count];

    // Select the whole TOP row (top-down y == 0) and nothing else.
    let selection = MeshSelection {
        width,
        height,
        indices: (0..width).collect(),
        sdf_occluded: 0,
    };

    let dir = std::env::temp_dir();
    let a_path: PathBuf = dir.join("boyko_sv0_cmp_a.bmp");
    let b_path: PathBuf = dir.join("boyko_sv0_cmp_b.bmp");
    let c_path: PathBuf = dir.join("boyko_sv0_cmp_c.bmp");

    // B differs at ONE selected pixel: top-down (1, 0).
    let mut b = base.clone();
    b[1] = [11, 20, 30, 255];
    // C differs at ONE UNselected pixel: top-down (1, 3), the vertical mirror of B's change. If
    // the decoder flipped rows the wrong way, C would look like B and this test would fail.
    let mut c = base.clone();
    c[(3 * width + 1) as usize] = [11, 20, 30, 255];

    write_test_bmp(&a_path, width, height, &base);
    write_test_bmp(&b_path, width, height, &b);
    write_test_bmp(&c_path, width, height, &c);

    let img_a = sv0_oracle::read_bmp32(&a_path).expect("invariant: the test BMP decodes");
    let img_b = sv0_oracle::read_bmp32(&b_path).expect("invariant: the test BMP decodes");
    let img_c = sv0_oracle::read_bmp32(&c_path).expect("invariant: the test BMP decodes");

    assert_eq!(img_a.bgra, base, "the decoder must round-trip a top-down image through the \
                                  writer's bottom-up rows unchanged");

    let ab = sv0_oracle::changed_covered_pixels(&selection, &img_a, &img_b)
        .expect("invariant: the extents match the selection");
    assert_eq!(ab.covered, width as usize, "the denominator is the selection, not the frame");
    assert_eq!(ab.changed, 1, "exactly one SELECTED pixel differs");
    assert!((ab.fraction() - 0.25).abs() < 1e-12, "1 of 4 selected pixels changed");

    let ac = sv0_oracle::changed_covered_pixels(&selection, &img_a, &img_c)
        .expect("invariant: the extents match the selection");
    assert_eq!(
        ac.changed, 0,
        "a difference outside the selection must not register — and would register if the \
         decoder mirrored the rows"
    );

    // An extent mismatch is an error, never a silent resize.
    let wrong = MeshSelection { width: 8, height: 8, indices: vec![0], sdf_occluded: 0 };
    assert!(
        sv0_oracle::changed_covered_pixels(&wrong, &img_a, &img_b).is_err(),
        "comparing images against a differently-sized selection must fail loudly"
    );

    for p in [a_path, b_path, c_path] {
        let _ = std::fs::remove_file(p);
    }
}
