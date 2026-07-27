//! **The SV0 rung-S1 fixture scene** (`docs/VB-SV0-SDF-SHADOW-PLAN.md`, "S1 — the fixture") — the
//! ONE definition of the geometry, the lights, the camera and the SDF occluder shared by the two
//! S1 dump fixtures (`vb_both_sdf.rs`, `vb_both_sdf_tex.rs`) and by the S1 adequacy oracle.
//!
//! Cargo does NOT auto-discover `tests/<dir>/mod.rs` as its own integration-test binary, so this
//! is a plain `mod sv0_scene;` include — the `tests/common/mod.rs` pattern this crate already
//! uses.
//!
//! # Why a shared module, when `vb_mesh.rs` / `vb_both.rs` deliberately duplicate `uv_sphere`
//!
//! Those copies are frozen on purpose: each backs a BLESSED golden, so a shared edit would move
//! already-pinned bytes without anyone touching the test. That trade-off does not apply here, and
//! the opposite one dominates. S1's whole product is a CPU oracle whose job is to certify that
//! **this** scene carries enough shadowed / contact-AO mesh pixels for every SV0 gate downstream
//! to be non-vacuous. If the oracle rasterised its own copy of the mesh, its own camera or its own
//! SDF body, a silent divergence would let it certify a scene that is never rendered — the exact
//! "green gate quantified over an empty selection" defect this rung exists to close. Duplication
//! that can silently diverge is the failure mode here, not the protection.
//!
//! Neither `[vb_both_sdf]` nor `[vb_both_sdf_tex]` is blessed yet, so there are no frozen bytes to
//! protect; and `vb_mesh.rs` / `vb_both.rs` are deliberately NOT migrated onto this module — their
//! local copies stay verbatim and pinned.
//!
//! # The scene
//!
//! `vb_both.rs`'s five-sphere `grand_showcase_2mat` row (same mesh generation, same sun/sky, same
//! camera, same 512² dump), plus ONE SDF sphere ([`sdf_body_edit`]) placed on the segment running
//! from the CENTRE mesh sphere toward the key light. [`SDF_CENTER_DISTANCE`] carries the placement
//! derivation and the separating mutation the S1 gate requires.
//!
//! # What this module is NOT
//!
//! It holds no gate floors (`SV0_MIN_SHADOWED_PIXELS` / `SV0_MIN_AO_PIXELS`). Those are the S1
//! oracle's, and the plan requires them fixed independently of the fixture — a floor that lives
//! next to the scene it measures invites being edited until the scene passes.

// Each including test binary uses a different subset of this module (the flat fixture never calls
// the textured helpers, the oracle never spawns anything), so the unused-item warning here would
// fire on correct code. Same reason `tests/common/mod.rs` carries the allow.
#![allow(dead_code)]

use boyko_app::prelude::*;
use boyko_render::generate_tangents;
use boyko_render::mesh::Vertex;
use boyko_render::{LightingConfig, SsaoConfig, SsaoQuality};

/// The sun direction TO the light — byte-identical to `vb_both.rs`'s / `vb_mesh.rs`'s /
/// `grand_showcase_2mat.rs`'s literal.
///
/// NOT unit: `|SUN_DIR| == 0.999_399_8`. The shader normalises (`l = normalize(L.dir)`), so every
/// direction-sensitive derivation in this module goes through [`sun_dir_unit`] rather than using
/// these components raw — otherwise the SDF body's stand-off distance would be off by the same
/// 6e-4 and the gap arithmetic in [`SDF_CENTER_DISTANCE`] would not say what it means.
pub const SUN_DIR: [f32; 3] = [-0.40, 0.78, 0.48];

/// Mesh sphere radius — `vb_both.rs`'s `uv_sphere(0.62, ...)`.
pub const MESH_SPHERE_RADIUS: f32 = 0.62;
/// Mesh sphere stack count — `vb_both.rs`'s `uv_sphere(..., 28, ...)`.
pub const MESH_SPHERE_STACKS: u32 = 28;
/// Mesh sphere slice count — `vb_both.rs`'s `uv_sphere(..., 40, ...)`.
pub const MESH_SPHERE_SLICES: u32 = 40;
/// Mesh sphere vertex colour — `vb_both.rs`'s `uv_sphere(..., [0.7, 0.7, 0.72, 1.0])`.
pub const MESH_SPHERE_COLOR: [f32; 4] = [0.7, 0.7, 0.72, 1.0];

/// Instances in the row — `vb_both.rs`'s five.
pub const MESH_ROW_COUNT: usize = 5;
/// Row pitch along `+X` — `vb_both.rs`'s `spacing`.
pub const MESH_ROW_SPACING: f32 = 1.55;
/// Row height — every instance sits at `y = 0.6`.
pub const MESH_ROW_Y: f32 = 0.6;

/// The row index the SDF body is anchored to: the CENTRE sphere, at the world origin's `x`.
///
/// Centre rather than an end sphere so the shadowed cap faces the camera squarely and the body's
/// own silhouette clears every neighbour (see [`SDF_CENTER_DISTANCE`]'s framing note).
pub const SDF_ANCHOR_INDEX: usize = 2;

/// Radius of the single SDF occluder.
///
/// Chosen against [`MESH_SPHERE_RADIUS`], not in isolation: the shadow ray from a mesh surface
/// point `P = M + r_mesh * n` along the unit light `L` passes the body centre at perpendicular
/// distance `r_mesh * sin(angle(n, L))`, so the body is hit exactly for
/// `angle(n, L) < asin(r_sdf / r_mesh) = asin(0.7258) = 46.6°` — a cap wide enough to darken a
/// large, obviously-visible patch of the lit hemisphere, and narrow enough that the shadow reads
/// as a shadow rather than as a terminator.
pub const SDF_SPHERE_RADIUS: f32 = 0.45;

/// **The mutation knob.** Distance from the anchor mesh sphere's CENTRE to the SDF body's centre,
/// measured along the unit light direction ([`sun_dir_unit`]).
///
/// # The two predicates, and why one constant separates them
///
/// Write `r_m = `[`MESH_SPHERE_RADIUS`], `r_s = `[`SDF_SPHERE_RADIUS`], `D = SDF_CENTER_DISTANCE`,
/// and `g = D - r_m - r_s` for the surface-to-surface gap (`0.25` as shipped).
///
/// * **Shadow (S1 gate 2).** The body is hit by the shadow ray from `P = M + r_m * n` iff
///   `r_m * sin(angle(n, L)) < r_s`. `D` does **not** appear: sliding the body along `L` changes
///   *when* a continuous march hits, never *whether* it hits. The count is therefore invariant
///   under this knob — measured at 1477 pixels at `D` = 1.32, 2.20 and 2.40 alike. Note the SHIPPED
///   march is discrete with a `SHADOW_MINT_STEP` floor, so that invariance is measured at those
///   placements rather than proved for all of them; `sv0_adequacy.rs`'s separating-mutation test
///   carries the argument.
/// * **Contact AO (S1 gate 3).** `sdf_ao` takes five taps along the SHADING normal at
///   `h_i = i * AO_STEP`, `i ∈ 1..=5`, `AO_STEP = 0.1`, accumulates
///   `occ = Σ (h_i − d_i) · AO_FALLOFF^i` and returns `clamp(1 − AO_STRENGTH·occ, 0, 1)`. It
///   darkens the pixel iff `occ > 0`. On the axis (`n == L`) each tap sees `d_i = g − h_i`, so
///   `occ = 2·Σ h_i·AO_FALLOFF^i − g·Σ AO_FALLOFF^i = 2.490811 − g·4.298162`, and the term is
///   armed iff `g < 0.579506`, i.e. iff `D < r_m + r_s + 0.579506 = 1.649506`.
///
///   ⚠️ **NOT `g < 2 · 5 · AO_STEP = 1.0`.** That is the range over which a tap can merely SEE the
///   body; it is not the range over which the leaf darkens anything, because taps with `d_i > h_i`
///   contribute NEGATIVE terms. The `1.0` reading is false in the false-GREEN direction and was
///   this rung's headline review finding (C1) — see `sv0_oracle::sdf_ao`.
///
/// **The separating mutation the gate requires: raise this constant to `2.40`.** The AO count
/// collapses to zero while the shadow count is bit-for-bit unchanged, because its bound has no `D`
/// in it. The analytic boundary is `1.6495`; both documented mutation values sit far past it (see
/// [`SDF_CENTER_DISTANCE_AO_DEFEATING`]).
///
/// # Why 1.32 and not "as close as possible"
///
/// `g = 0.25` keeps the body clear of the mesh surface (no intersection, so no marcher/raster
/// interpenetration seam) while sitting well inside the `0.5795` darkening boundary — with the
/// corrected predicate the armed set is the cap `∠(n, L) < 31.8°` (solve `occ(θ) = 0` with
/// `d_i = ‖n·(r_m + h_i) − L·D‖ − r_s`, which is the SDF distance and stays finite past the
/// `asin(r_s/D) = 19.9°` where the normal RAY stops hitting the body). That cap measures 958 of the
/// 28362 covered mesh pixels. It also keeps the body's own silhouette off every mesh sphere: at 512²
/// the body owns a ~66 px disc centred ~82 px from the centre sphere's ~84 px disc, and the
/// covered-mesh-pixel count is 28362 with the body present or absent — i.e. it eclipses nothing
/// (asserted: `MeshSelection::sdf_occluded == 0`).
pub const SDF_CENTER_DISTANCE: f32 = 1.32;

/// The separating mutation's slid-out distances — the values the S1 oracle re-measures BOTH
/// counts at to prove gate 2 and gate 3 are independent assertions and not one wearing two names.
///
/// The plan states the mutation as an edit to [`SDF_CENTER_DISTANCE`] ("raise this constant to
/// `2.40`"); the S1 oracle instead RUNS it, recomputing the counts at each of these placements
/// through the same [`sdf_body_edit_at`] the shipped placement goes through. `2.40` is the value
/// the module doc names; `2.20` is carried alongside it because a mutation that only holds at one
/// hand-picked distance is a coincidence, not a separation.
///
/// Both sit far past the analytic AO boundary `r_m + r_s + 0.579506 = 1.6495` (see
/// [`SDF_CENTER_DISTANCE`]'s derivation) — `0.55` and `0.75` of clearance, against a boundary whose
/// own float noise is in the last ULP of an `f32`. They were chosen against the WITHDRAWN `2.07`
/// boundary and are kept unchanged: the correction moved the boundary IN, so every margin they
/// carried only grew.
pub const SDF_CENTER_DISTANCE_AO_DEFEATING: [f32; 2] = [2.20, 2.40];

/// Camera eye — `vb_both.rs`'s `Vec3::new(0.0, 1.1, 7.8)`.
pub const CAMERA_EYE: [f32; 3] = [0.0, 1.1, 7.8];
/// Camera look-at target — `vb_both.rs`'s `Vec3::new(0.0, 0.55, 0.0)`.
pub const CAMERA_TARGET: [f32; 3] = [0.0, 0.55, 0.0];
/// Camera up axis — `+Y`.
pub const CAMERA_UP: [f32; 3] = [0.0, 1.0, 0.0];
/// Vertical field of view in DEGREES — `vb_both.rs`'s `52.0`.
pub const CAMERA_FOV_Y_DEGREES: f32 = 52.0;
/// Projection near plane — `vb_both.rs`'s `0.1`.
pub const CAMERA_NEAR: f32 = 0.1;
/// Projection far plane — `vb_both.rs`'s `100.0`.
pub const CAMERA_FAR: f32 = 100.0;
/// Square dump extent in pixels — both fixtures dump 512×512, and the S1 oracle's pixel counts are
/// quantified over exactly this raster.
pub const DUMP_EXTENT: u32 = 512;

// ===========================================================================================
// The rung-S4 arming knobs (env-driven, DEFAULT OFF)
// ===========================================================================================
//
// Rung S4's gate (ii) needs, for each of the eight SV0-armable variant rows, THREE renders of
// the same scene: `sv0_mode = 0`, shadow-bit-only, and AO-bit-only. That is 24 dumps across two
// fixtures. Driving them from env vars rather than from 24 committed fixtures is what keeps the
// scene single-sourced (the whole point of this module) — the alternative is 24 near-copies that
// can silently drift apart, which is the defect [`spawn_scene`]'s doc already argues against.
//
// EVERY knob below defaults to OFF/absent, so a plain `cargo test` run of either fixture — and
// `scripts\golden.ps1`, which WIPES any `BOYKO_*` its pin does not name — renders exactly the
// configuration `[vb_both_sdf]` / `[vb_both_sdf_tex]` were blessed under.

/// Selects the SV0 gate mode the fixture REQUESTS: `0` (off, the default), `1` (shadow bit only,
/// gate ii-a), `2` (contact-AO bit only, gate ii-b), `3` (both).
///
/// A REQUEST, not a guarantee: `sync_sv0_light_gate` clamps it against the boot's resolved
/// capability and prints a diagnostic when it cannot honour it.
pub const SV0_MODE_ENV: &str = "BOYKO_SV0_MODE";
/// Set to `1` to arm the froxel light cull (`LightingConfig::clusters_enabled`), which selects
/// the `_froxel` lit-producer rows (2, 5, 6).
///
/// Read at BOOT (`boyko_app::runner`'s `clusters_wanted` probe), which is why this has to reach
/// the world through an `insert_resource` before `App::run` rather than through a startup system.
pub const SV0_FROXEL_ENV: &str = "BOYKO_SV0_FROXEL";
/// Set to `1` to arm SSAO, which arms `mesh_geo_shade_split` and therefore selects the SPLIT
/// lit-producer rows (7, 8). Also boot-read, same reason as [`SV0_FROXEL_ENV`].
pub const SV0_SSAO_ENV: &str = "BOYKO_SV0_SSAO";

/// The SSAO quality the split rows boot with when [`SV0_SSAO_ENV`] is set — `High`, matching
/// `vb_mesh_ssao.rs`'s own shipped fixture so the split tail is exercised in its blessed shape.
const SV0_SSAO_QUALITY: SsaoQuality = SsaoQuality::High;
/// The à-trous level count paired with [`SV0_SSAO_QUALITY`] — again `vb_mesh_ssao.rs`'s value.
const SV0_SSAO_ATROUS_LEVELS: u32 = 3;

/// Reads an env var as a boolean knob: present and exactly `"1"` is on, everything else is off.
///
/// Deliberately strict rather than "any non-empty value": a stale `BOYKO_SV0_FROXEL=0` left in a
/// shell would otherwise arm the froxel rows and silently move every count in this campaign.
fn env_flag(name: &str) -> bool {
    std::env::var(name).is_ok_and(|v| v == "1")
}

/// The SV0 gate mode this run requests, from [`SV0_MODE_ENV`] — `0` when unset.
///
/// # Panics
///
/// Panics on a value outside `0..=3`. A typo'd mode must not silently degrade to `0`: that
/// renders the UNARMED image under an "armed" filename, and the gate that compares them would
/// then report a changed-pixel count of zero and read it as "the term is dead".
pub fn sv0_mode_from_env() -> u32 {
    let Ok(raw) = std::env::var(SV0_MODE_ENV) else { return 0 };
    let mode: u32 = raw
        .parse()
        .unwrap_or_else(|_| panic!("invariant: {SV0_MODE_ENV} must be 0..=3, got {raw:?}"));
    assert!(mode <= 3, "invariant: {SV0_MODE_ENV} must be 0..=3, got {mode}");
    mode
}

/// The [`LightingConfig`] a fixture inserts AFTER `add_plugins`: `LightingConfig::default()` with
/// this run's SV0 request and froxel arming applied.
///
/// With no env set this is bit-identical to the `LightingConfig::default()` that
/// `EnginePlugins::build` already seeded, so inserting it unconditionally cannot move a blessed
/// pin — which is why the fixtures do exactly that rather than branching.
pub fn lighting_config_from_env() -> LightingConfig {
    let mode = sv0_mode_from_env();
    LightingConfig {
        clusters_enabled: env_flag(SV0_FROXEL_ENV),
        // Bit 0 of the mode is the shadow term, bit 1 the contact AO — the SAME lane assignment
        // `boyko_render::light`'s `VB_SDF_MESH_SHADOW_BIT`/`VB_SDF_MESH_AO_BIT` and the shader's
        // `load_vb_sdf_mesh_mode` decode use, so `BOYKO_SV0_MODE` IS the shader's `sv0_mode`.
        vb_sdf_mesh_shadow: (mode & boyko_render::VB_SDF_MESH_SHADOW_BIT) != 0,
        vb_sdf_mesh_ao: (mode & boyko_render::VB_SDF_MESH_AO_BIT) != 0,
        ..LightingConfig::default()
    }
}

/// The [`SsaoConfig`] the split rows boot with, or `None` when [`SV0_SSAO_ENV`] is unset.
///
/// `None` means the fixture inserts NOTHING — not `SsaoConfig::default()` — because the boot
/// probe is `try_resource::<SsaoConfig>().is_some_and(|c| c.enabled())` and an absent Resource is
/// the shipped 0%-gate state every blessed VB pin was rendered under.
pub fn ssao_config_from_env() -> Option<SsaoConfig> {
    env_flag(SV0_SSAO_ENV)
        .then_some(SsaoConfig { quality: SV0_SSAO_QUALITY, atrous_levels: SV0_SSAO_ATROUS_LEVELS })
}

/// [`SUN_DIR`] normalised — the direction the shader actually shades and marches with
/// (`l = normalize(L.dir)`), and the axis the SDF body is placed on.
pub fn sun_dir_unit() -> [f32; 3] {
    let d = SUN_DIR;
    let inv_len = 1.0 / (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
    [d[0] * inv_len, d[1] * inv_len, d[2] * inv_len]
}

/// World-space centre of row instance `i` (`0..`[`MESH_ROW_COUNT`]) — `vb_both.rs`'s
/// `x = (i - 2.0) * spacing`, `y = 0.6`, `z = 0`.
///
/// # Panics
///
/// Panics when `i >= MESH_ROW_COUNT`: every caller indexes a compile-time-known instance, so an
/// out-of-range index is a fixture bug, not a runtime condition.
pub fn mesh_center(i: usize) -> [f32; 3] {
    assert!(i < MESH_ROW_COUNT, "invariant: row index {i} is outside the {MESH_ROW_COUNT}-sphere row");
    [(i as f32 - 2.0) * MESH_ROW_SPACING, MESH_ROW_Y, 0.0]
}

/// World-space centre of the SDF occluder placed `distance` along the unit light from the anchor
/// sphere's centre — the ONE placement formula, parameterised so the separating mutation is the
/// same construction with one number changed rather than a second, drift-prone derivation.
pub fn sdf_body_center_at(distance: f32) -> [f32; 3] {
    let m = mesh_center(SDF_ANCHOR_INDEX);
    let l = sun_dir_unit();
    [m[0] + l[0] * distance, m[1] + l[1] * distance, m[2] + l[2] * distance]
}

/// World-space centre of the SDF occluder: [`SDF_CENTER_DISTANCE`] along the unit light from the
/// anchor sphere's centre.
pub fn sdf_body_center() -> [f32; 3] {
    sdf_body_center_at(SDF_CENTER_DISTANCE)
}

/// The scene's ONE SDF edit — the occluder both S1 predicates are quantified over, and the only
/// entry `collect_sdf_edits` finds (so `edit_count == 1`, S1 gate 1).
///
/// Material lane is left at `0` (the `SdfEdit::sphere` default), i.e. the engine-minted default
/// material — never the fixture's first `Assets::add`. That is what keeps the textured sibling's
/// material off the SDF surface, which reads `base_color` only.
pub fn sdf_body_edit() -> SdfEdit {
    sdf_body_edit_at(SDF_CENTER_DISTANCE)
}

/// The scene's SDF edit with the body placed `distance` along the unit light — [`sdf_body_edit`]
/// generalised, so the S1 oracle's separating mutation exercises the shipped construction verbatim
/// (same radius, same op, same material lane) with only the placement moved.
pub fn sdf_body_edit_at(distance: f32) -> SdfEdit {
    SdfEdit::sphere(sdf_body_center_at(distance), SDF_SPHERE_RADIUS, sdf_op::UNION, 0.0)
}

/// Verbatim copy of `vb_both.rs::uv_sphere` / `vb_mesh.rs::uv_sphere` — INCLUDING the pole-fan
/// triangles.
///
/// Deliberately NOT `tests/common/mod.rs::uv_sphere`: that one SKIPS the degenerate pole quads, so
/// it emits a different index buffer and a different tangent basis. The S1 fixtures clone
/// `vb_both.rs`'s scene, and the S1 oracle rasterises whatever the fixtures render, so the mesh
/// generation has to be this one.
pub fn uv_sphere(radius: f32, stacks: u32, slices: u32, color: [f32; 4]) -> (Vec<Vertex>, Vec<u32>) {
    let pi = core::f32::consts::PI;
    let mut verts = Vec::with_capacity(((stacks + 1) * (slices + 1)) as usize);
    for i in 0..=stacks {
        let phi = (i as f32 / stacks as f32) * pi; // 0..π, north pole to south
        let (sp, cp) = phi.sin_cos();
        let v = i as f32 / stacks as f32; // phi / π
        for j in 0..=slices {
            let theta = (j as f32 / slices as f32) * (2.0 * pi); // 0..2π
            let (st, ct) = theta.sin_cos();
            let n = [sp * ct, cp, sp * st]; // unit outward normal
            let u = j as f32 / slices as f32; // theta / 2π
            let mut vertex = Vertex::new([n[0] * radius, n[1] * radius, n[2] * radius], n, color);
            vertex.uv = [u, v];
            verts.push(vertex);
        }
    }
    let stride = slices + 1;
    let mut idx = Vec::with_capacity((stacks * slices * 6) as usize);
    for i in 0..stacks {
        for j in 0..slices {
            let a = i * stride + j;
            let b = (i + 1) * stride + j;
            idx.extend_from_slice(&[a, b, a + 1, a + 1, b, b + 1]);
        }
    }
    generate_tangents(&mut verts, &idx);
    (verts, idx)
}

/// The fixtures' sphere mesh: [`uv_sphere`] at this scene's frozen radius / tessellation / colour.
pub fn scene_sphere_mesh() -> (Vec<Vertex>, Vec<u32>) {
    uv_sphere(MESH_SPHERE_RADIUS, MESH_SPHERE_STACKS, MESH_SPHERE_SLICES, MESH_SPHERE_COLOR)
}

/// **The scene's ONE entry point.** Spawns the five-sphere row, the SDF occluder, the sun + sky and
/// the camera — everything the S1 fixtures render and the S1 oracle measures — parameterised only
/// by what genuinely differs between the two fixtures: the GPU mesh handle and the material row.
///
/// # Why this is one call and not four (review C2)
///
/// The plan's first S1 mutation is "remove the SDF spawns → gates (1), (2) and (3) all fail". That
/// mutation only reds if the body is STRUCTURALLY inseparable from the scene the oracle quantifies
/// over. While a fixture could spawn the row, the sun and the camera itself and simply omit
/// `spawn_sdf_body`, all four gates stayed green over a pin rendering the boot-seeded EMPTY edit
/// list — precisely the vacuity this rung exists to close, and invisible to every assertion S1
/// makes. So the row / sun / camera helpers below are PRIVATE to this module and this is the only
/// way in: dropping the body now requires editing the module the gates are quantified over, which
/// is what makes the mutation red.
///
/// [`spawn_sdf_body`] stays public on purpose — exposing the body ALONE cannot produce a
/// body-less scene, and the `#[ignore]`d real-runner staging tripwire needs an SDF primitive
/// without a GPU mesh to hang it on.
pub fn spawn_scene(
    commands: &mut Commands,
    sphere: MeshHandle,
    materials_row: &[Option<u16>; MESH_ROW_COUNT],
) {
    spawn_mesh_row(commands, sphere, materials_row);
    spawn_sdf_body(commands);
    spawn_sun_and_sky(commands);
    spawn_camera(commands);
}

/// Spawns the five-sphere row: instance `i` at [`mesh_center`], carrying `materials_row[i]` when
/// it is `Some` (a `None` entry leaves `MeshBundle::new`'s default material slot 0).
///
/// The material row is a parameter because it is the ONLY thing the flat and textured fixtures
/// differ in — everything else about the scene is this module's, so the two dumps cannot drift
/// apart in geometry, lighting or framing.
///
/// PRIVATE — reachable only through [`spawn_scene`]; see its doc for why.
fn spawn_mesh_row(
    commands: &mut Commands,
    sphere: MeshHandle,
    materials_row: &[Option<u16>; MESH_ROW_COUNT],
) {
    for (i, mat) in materials_row.iter().enumerate() {
        let c = mesh_center(i);
        let e = commands
            .spawn(MeshBundle::new(sphere, Transform::from_translation(Vec3::new(c[0], c[1], c[2]))))
            .id();
        if let Some(id) = mat {
            commands.entity(e).insert(MaterialHandle(*id));
        }
    }
}

/// Spawns the key light + the ambient sky — `vb_both.rs`'s sun/sky verbatim.
///
/// PRIVATE — reachable only through [`spawn_scene`]; see its doc for why.
fn spawn_sun_and_sky(commands: &mut Commands) {
    let sun_pose = Affine3A::look_at_rh(
        Vec3::ZERO,
        Vec3::new(SUN_DIR[0], SUN_DIR[1], SUN_DIR[2]),
        Vec3::new(CAMERA_UP[0], CAMERA_UP[1], CAMERA_UP[2]),
    );
    commands.spawn(DirectionalLightObject {
        transform: Transform {
            translation: Vec3::ZERO,
            rotation: Quat::from_mat3(sun_pose.matrix3),
            scale: Vec3::ONE,
        },
        global: GlobalTransform::IDENTITY,
        light: DirectionalLight::new(SUN_DIR, [1.0, 0.97, 0.92], 3.1),
    });

    commands.spawn(SkyLight::new([0.38, 0.44, 0.55], [0.20, 0.20, 0.22]));
}

/// Spawns the SDF occluder ([`sdf_body_edit`]) — the component whose PRESENCE is what puts an
/// entry in `collect_sdf_edits`' gather, and therefore the whole reason this scene exists.
///
/// **Deleting this call from [`spawn_scene`] is the plan's first S1 mutation.** It must red S1
/// gates 1, 2 and 3 together: gate 1 gathers zero edits, and gates 2/3 are quantified over that
/// same gathered list, so both counts fall to zero.
pub fn spawn_sdf_body(commands: &mut Commands) {
    commands.spawn(SdfPrimitive(sdf_body_edit()));
}

// ===========================================================================================
// The GPU-free gather harness — shared by the S1 adequacy gate and the S4 arming matrix
// ===========================================================================================

/// The mesh handle the GPU-free harness hands [`spawn_scene`].
///
/// `MeshHandle` is a plain dense index into `Assets<MeshGpu>`; spawning `MeshBundle`s that name a
/// non-existent slot is inert in an app with no render plugin (nothing walks the table, and
/// `MeshHandle`'s refcount hook no-ops when `RefcountDeltas` is absent). The row is spawned anyway
/// because the point of this harness is to drive the SHARED entry point, not a subset of it.
const ORACLE_MESH_HANDLE: MeshHandle = MeshHandle(0);

/// The material row the GPU-free harness hands [`spawn_scene`] — all default, since no material is
/// registered and none is read.
const ORACLE_MATERIALS_ROW: [Option<u16>; MESH_ROW_COUNT] = [None; MESH_ROW_COUNT];

/// Spawns the WHOLE fixture scene through the shared entry point — the startup system the GPU-free
/// gather harness drives, and the reason a body dropped from [`spawn_scene`] reds the CPU gates
/// instead of only the GPU dumps.
fn spawn_scene_system(mut commands: Commands) {
    spawn_scene(&mut commands, ORACLE_MESH_HANDLE, &ORACLE_MATERIALS_ROW);
}

/// Builds the GPU-free gather harness: `SdfPlugin` (which only inserts the staging resource), the
/// fixtures' own shared scene spawn, and the runner's explicit post-`finish()` gather.
///
/// This reproduces `boyko_app/src/runner.rs:589`'s ordering exactly — `collect_sdf_edits` is run
/// ONCE by hand after `finish()` has drained every startup system, which is the order-proof site
/// the host chose precisely so a plugin-registered startup gather could not race the user's later
/// `add_startup_system(setup)`.
///
/// Lives HERE rather than in one of the two test binaries that need it: both the S1 adequacy gate
/// and the S4 arming matrix quantify their pixel counts over the GATHERED edit list, and a second
/// copy of this construction is exactly the silently-divergent duplicate [`spawn_scene`]'s own doc
/// (review C2) argues against.
pub fn gathered_app() -> App {
    let mut app = App::new();
    app.add_plugins(boyko_render::SdfPlugin);
    app.add_startup_system(spawn_scene_system);
    app.finish();
    app.world_mut().run_system(boyko_render::collect_sdf_edits);
    app
}

/// The SDF edit list AS THE RENDERED SCENE PRODUCES IT — [`gathered_app`]'s staging output.
///
/// Every pixel count in this campaign is quantified over THIS list, never over a locally
/// reconstructed one, so deleting the body from [`spawn_scene`] empties them all together.
pub fn gathered_edits() -> Vec<SdfEdit> {
    let app = gathered_app();
    app.world().resource::<boyko_render::SdfEditStaging>().edits().to_vec()
}

/// The camera pose the dumps and the S1 oracle share.
pub fn camera_transform() -> Transform {
    let pose = Affine3A::look_at_rh(
        Vec3::new(CAMERA_EYE[0], CAMERA_EYE[1], CAMERA_EYE[2]),
        Vec3::new(CAMERA_TARGET[0], CAMERA_TARGET[1], CAMERA_TARGET[2]),
        Vec3::new(CAMERA_UP[0], CAMERA_UP[1], CAMERA_UP[2]),
    );
    Transform { translation: pose.translation, rotation: Quat::from_mat3(pose.matrix3), scale: Vec3::ONE }
}

/// The projection the dumps and the S1 oracle share (`aspect = 1.0`, matching the square dump).
pub fn camera_projection() -> Projection {
    Projection::Perspective {
        fov_y: CAMERA_FOV_Y_DEGREES * core::f32::consts::PI / 180.0,
        aspect: 1.0,
        near: CAMERA_NEAR,
        far: CAMERA_FAR,
    }
}

/// Spawns the camera rig — `vb_both.rs`'s framing verbatim.
///
/// PRIVATE — reachable only through [`spawn_scene`]; see its doc for why.
fn spawn_camera(commands: &mut Commands) {
    commands.spawn(CameraRig {
        transform: camera_transform(),
        global: GlobalTransform::IDENTITY,
        camera: Camera::DEFAULT,
        projection: camera_projection(),
    });
}
