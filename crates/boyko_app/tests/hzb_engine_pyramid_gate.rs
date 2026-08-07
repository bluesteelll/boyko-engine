//! VG R3 piece 1, step P1-8 — **gate G8: the pyramid the ENGINE built equals the host oracle.**
//!
//! Gate G3 (`hzb_build_oracle_gate.rs`) proves the SHADER equals `boyko_render::hzb::build_pyramid`
//! — but it builds its own depth, its own image, its own views, its own descriptor sets and its own
//! barriers, so it is structurally blind to a wrong source, a wrong extent, a stale descriptor, a
//! missing barrier or a build pass that never ran. G8 is the comparison that can see those: the
//! engine renders a real frame, `BOYKO_HZB_DUMP` copies the engine's own `vb_depth` and every mip
//! of the engine's own pyramid into one file, and this test rebuilds the pyramid from the DUMPED
//! DEPTH and compares `to_bits()` at every texel of every level.
//!
//! # ⚠️ Why a comparison alone would be VACUOUS, and what carries the non-vacuity
//!
//! Step P1-6 ran exactly that comparison on the `vb_mesh` scene and got **0 mismatches over all
//! 349 525 texels** — and the plan (§13) records why that number means much less than it sounds.
//! The scene covers ~11% of the framebuffer; the rest is the reverse-Z far plane `0.0`; a `min`
//! footprint containing any `0.0` is `0.0`. So **89.3% of the pyramid is `0.0`, and levels 6..9 are
//! ENTIRELY so** — and levels 6..9 are precisely what the SECOND build pass writes. A pyramid image
//! that a driver zero-filled and NOBODY WROTE would have matched the oracle at every one of those
//! texels.
//!
//! The fix is NOT a bigger fixture. Step P1-8 poisons the pyramid IMAGE (`hzb_poison`, a declared
//! framegraph pass gated on the dump probe) with
//! [`HZB_PYRAMID_POISON`] before the first build dispatch, so an
//! unwritten texel holds a value the reduce cannot produce — at ANY scene coverage. **The poison,
//! not the coverage, is what makes this gate mean something**, which is why the coverage below is
//! REPORTED rather than gated on: plan §5 proposed a full-coverage fixture, the poison removes the
//! need for one, and no such VB fixture exists in the tree today.
//!
//! # The five non-vacuity clauses, each of which FAILS when unmet
//!
//! 1. **No pyramid texel is the poison.** The load-bearing one: it says every level was WRITTEN.
//! 2. **The header's extents equal the oracle's** — `levels` and every `level_extent(k)`. A wrong
//!    engine extent surfaces here, by name, instead of as a wall of texel mismatches. The extents
//!    are read FROM THE HEADER and never re-derived: G8 exists to catch a wrong extent, and a host
//!    that computes the extent it expects agrees with itself no matter what the engine did.
//! 3. **At least two distinct depths, and at least one `> 0.0`** — otherwise the frame rendered
//!    nothing and the comparison is over a constant field.
//! 4. **No pyramid texel is `+INFINITY`** — the boundary rule's `min` identity must never survive
//!    into a written texel, because a written texel always has a live child.
//! 5. **No texel of either payload is NaN** — the value the host driver prefills the staging with
//!    (`boyko_app::hzb_dump`), which neither payload can legitimately hold. This is the clause that
//!    separates "the copy never ran" from "the build never ran"; without it a failed copy would
//!    read as a failed build.
//!
//! # VG R3 piece 3 step P3-7 (plan D10) — the dump carries TWO depths, and this file decodes both
//!
//! The pyramid is built from the depth as of the EARLY raster scope; the frame-end `hzb_dump` pass
//! copies that image again after the LATE scope has drawn into it. Through piece 2 those were the
//! same bytes (the late scope drew nothing), so this gate compared against a depth it had no way of
//! knowing was the right one. The dump now carries both regions plus a `flags` bit saying which are
//! live and a `frame_index` the RECORDER stamps inside the copy frame's command buffer.
//!
//! Each leg therefore DECLARES its regime ([`DepthRegime`]) and the gate checks the file against
//! that declaration — never the reverse. G8's leg declares "no early region" and additionally
//! asserts that region is untouched staging prefill; G5's declares the converged split and gains
//! G-P3-E's clause 2' (`depth_early == depth_final`, byte for byte); and step P3-8's FORCE-LATE leg
//! declares the regime in which the two depths DIFFER, which is what makes clauses 2 and 3 — the
//! pair that proves the pyramid was built at the right point in the frame — assertable at all.
//!
//! The magic was bumped in the same step, so a stale pre-P3-7 file cannot decode against the moved
//! offsets. [`a_stale_pre_p3_7_dump_fails_to_decode_instead_of_being_believed`] EXECUTES that
//! failure — it needs no device and runs in every sweep.
//!
//! # How this test drives the engine, and why THAT harness
//!
//! `vg_density_census.rs`'s shape: one `#[ignore]`d WORKER test that boots the app, and a DRIVER
//! test that re-executes this same binary's worker as a child process with the env knob set, then
//! reads the artifact it produced. That is the only harness that fits, for two mechanical reasons:
//!
//! * The dump is armed by ENV, read once by `HzbDump::from_env()` inside `app.run()`, and the host
//!   loop RETURNS when the dump completes (`boyko_app::runner`'s "exit once every armed capture has
//!   completed"). So a dump IS a process, exactly as a census rung is.
//! * Arming it in-process would mean `std::env::set_var` — `unsafe` in Rust 2024 and genuinely
//!   racy once the engine's threadpool exists. A child process inherits the variable from its own
//!   `Command`, before any thread of its own is spawned.
//!
//! `vb_mesh.rs` alone does not fit: it is the windowed dump SHAPE (and this file's worker copies
//! it), but a golden-pin test writes a screenshot and asserts nothing, whereas G8 must read a file
//! back and adjudicate it in the same `cargo test` invocation.
//!
//! # THREE worker+driver pairs since VG R3 piece 3 step P3-8
//!
//! Piece 2's occlusion split moves the whole `[hzb_poison, hzb_build_*]` block BETWEEN the two
//! raster scopes on an armed-split frame (plan D6), which makes the block's slot **conditional**.
//! So this file runs the same comparison three times, once per depth regime:
//!
//! * [`hzb_engine_pyramid_equals_the_oracle`] — the UNSPLIT frame (gate G8, unchanged). This is
//!   the configuration every shipping frame and all 26 golden pins take.
//! * [`hzb_engine_pyramid_equals_the_oracle_occ`] — the ARMED-SPLIT, CONVERGED frame (gate G5),
//!   where the block sits at its early slot and plan D12's fixed point makes the two depths
//!   byte-identical (G-P3-E clause 2').
//! * [`hzb_engine_pyramid_equals_the_oracle_force_late`] — the ARMED-SPLIT frame under
//!   `VB_CULL_OCC_FORCE_LATE` on the `vb_occ_mixed` scene (G-P3-E clauses 2 and 3), the ONLY
//!   configuration on a static scene in which the late scope draws and therefore the only one in
//!   which the pyramid's POSITION in the frame is falsifiable.
//!
//! ⚠️ Each is an **ADDITION**. Re-pointing the existing worker at a marked scene would have DELETED
//! the unsplit leg — the only engine-level gate over the path piece 2 newly made conditional. **All
//! three must be green in the same sitting.**
//!
//! # Run
//!
//! `cargo test -p boyko-app --test hzb_engine_pyramid_gate -- --ignored --nocapture
//! --test-threads=1` with `BOYKO_DISABLE_VALIDATION=1`. Each driver spawns its own worker by name
//! (`--exact`) into its own dump path; a worker run directly needs `BOYKO_HZB_DUMP=<path.bin>` and
//! SKIPS (rather than looping forever) without it.

#![cfg(windows)]

use std::path::{Path, PathBuf};
use std::process::Command;

use boyko_app::prelude::*;
use boyko_ecs::ecs::core::system::ResMut;
use boyko_render::hzb::{HzbLayout, build_pyramid};
use boyko_render::mesh::Vertex;
use boyko_render::{
    GeometryLegs, HzbConfig, HzbMode, Material, MeshAssetsVbExt, MeshGeometryTableSlot,
    OcclusionCulling, RenderPath, RenderPathConfig, generate_tangents,
};
use boyko_rhi_vulkan::present::{
    HZB_DUMP_FLAG_DEPTH_EARLY, HZB_DUMP_HEADER_BYTES, HZB_DUMP_HEADER_SCALAR_WORDS,
    HZB_DUMP_HEADER_WORDS, HZB_DUMP_MAGIC, HZB_DUMP_SAMPLE_BYTES, HZB_DUMP_WORD_FLAGS,
    HZB_DUMP_WORD_FRAME_INDEX, HZB_PYRAMID_POISON, MAX_HZB_LEVELS,
};

mod occ_fixture;
mod vb_occ_mixed_scene;

/// The env knob that arms `boyko_app::hzb_dump` — the value is the output path.
const ENV_DUMP: &str = "BOYKO_HZB_DUMP";

/// The worker test the UNSPLIT driver re-executes.
const WORKER: &str = "hzb_engine_pyramid_dump";

/// VG R3 piece 2 step P2-6 — **gate G5's** worker: the same scene with `OcclusionCulling` in the
/// spawn bundle, which arms the occlusion split and therefore moves the whole
/// `[hzb_poison, hzb_build_*]` block between the two raster scopes (plan D6).
///
/// ⚠️ **An ADDITION, not a conversion.** [`WORKER`] above stays exactly as it was, and both pairs
/// must be green in the same sitting. The unsplit path is the one every shipping frame and all 25
/// golden pins take, and piece 2 is precisely the change that makes its `hzb_build` slot
/// CONDITIONAL — converting this file to the marked scene would have deleted the only
/// engine-level gate over that path. Nothing else covers it: `hzb_build_spv_sync` is a byte gate,
/// `hzb_build_oracle_gate` is structurally blind by its own header, gate G4 is a synthetic
/// declaration replica, and the declarator's parity asserts cover `vb_raster_late`, not the HZB
/// slot.
const WORKER_OCC: &str = "hzb_engine_pyramid_dump_occ";

/// VG R3 piece 3 step P3-8 — **G-P3-E's FORCE-LATE worker**: the `vb_occ_mixed` scene with
/// `BOYKO_VG_OCC_FORCE=late`, where the early phase defers EVERY marked instance regardless of the
/// pyramid.
///
/// It is the only configuration in this repository in which the two dumped depths can DIFFER on a
/// static scene, and therefore the only one in which clauses 2 and 3 — the pair that proves the
/// pyramid was built at the right point in the frame — are non-vacuous. The five-sphere scene cannot
/// serve: with every instance marked, FORCE-LATE empties the early depth entirely and trips this
/// file's own SHIPPED non-vacuity clauses. `vb_occ_mixed` carries an UNMARKED occluder and an
/// UNMARKED filler precisely so the early depth is populated and varied.
const WORKER_OCC_LATE: &str = "hzb_engine_pyramid_dump_occ_late";

/// The env that selects the FORCE-LATE regime. Read ONCE at `GpuSceneBundles::boot`, so a regime IS
/// a process — which is why it is set on the CHILD rather than toggled in one.
const ENV_OCC_FORCE: [(&str, &str); 1] = [("BOYKO_VG_OCC_FORCE", "late")];

/// The worker window's client extent. 512² is the extent step P1-6 measured the dump at
/// (`levels = 10`, two build passes), so the coverage this gate reports is directly comparable with
/// plan §13's table — and two passes is what makes the SECOND pass (§13's blind spot) reachable.
const EXTENT: u32 = 512;

/// The sun direction TO the light (byte-identical to `vb_mesh.rs`'s).
const SUN_DIR: [f32; 3] = [-0.40, 0.78, 0.48];

/// How many `[w, h]` pairs the fixed-size header carries — `levels` of them are live and the rest
/// are zero padding. Derived from the header's own word count rather than spelled, and tied to the
/// capacity it must equal, so the two cannot drift into a reader that walks the wrong tail.
///
/// ⚠️ **The `4` here was a LITERAL until VG R3 piece 3 step P3-7, and that made this guard's
/// ability to see a drift depend on the parity of the drift.** Widening the scalar prefix to 6
/// reds this assertion (`(40 - 4) / 2 == 18 != 17`) — but widening it to any ODD count truncates in
/// the division and passes silently. With [`HZB_DUMP_HEADER_SCALAR_WORDS`] exported, the writer's
/// word indices and this reader's derived offsets come from ONE number, and what remains here is a
/// relation neither side can restate.
const HEADER_LEVEL_SLOTS: usize = (HZB_DUMP_HEADER_WORDS - HZB_DUMP_HEADER_SCALAR_WORDS) / 2;
const _: () = assert!(HEADER_LEVEL_SLOTS == MAX_HZB_LEVELS);

/// The `f32` bit pattern the host driver prefills the whole dump staging with
/// (`boyko_app::hzb_dump::prefill_with_poison`'s `0xFF` byte fill) — a quiet NaN, which neither
/// payload can legitimately hold.
///
/// Spelled here because step P3-7 needs to assert its PRESENCE, not only its absence: on an unsplit
/// dump frame the early-depth region is never written, and "every texel of it is exactly this" is
/// the falsifiable form of "the early copy did not run".
const STAGING_PREFILL_BITS: u32 = 0xFFFF_FFFF;

// ===============================================================================================
// The worker: one process, one engine boot, one dump file
// ===============================================================================================

/// Verbatim copy of `vb_mesh.rs::uv_sphere` — a local copy for the same reason that file gives:
/// a fixture whose geometry is compared against a recorded measurement keeps its own mesh
/// generation, instead of moving under it when a shared helper is edited for someone else.
fn uv_sphere(radius: f32, stacks: u32, slices: u32, color: [f32; 4]) -> (Vec<Vertex>, Vec<u32>) {
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

/// `vb_mesh.rs`'s five-sphere scene — the one plan §13's coverage table was measured on — with
/// exactly two deliberate deltas, so the coverage this gate prints is COMPARABLE with §13's ~11%
/// rather than equal to it:
///
/// * ONE material instead of four. This gate reads depth, and a material affects nothing it reads.
/// * A per-sphere Z stagger. `vb_mesh` puts all five spheres on one plane, which gives the dumped
///   depth two values per column and no gradient; staggered, the base map's `⌈t·S/P⌉` partition is
///   exercised against a depth field that actually varies inside a level-0 footprint.
///
/// Neither delta is what makes the gate non-vacuous — the IMAGE poison is (see the module header).
/// They make the comparison less likely to agree by coincidence, which is a different property.
///
/// VG R3 piece 2 step P2-6: `marked` puts [`OcclusionCulling`] in every spawn bundle, which is the
/// ONLY difference between gate G8's scene and gate G5's. One function, two constants at the two
/// call sites — so "the same scene apart from the marker" is a property of the code rather than of
/// two copies a later edit can desynchronise.
fn setup_scene(
    commands: &mut Commands,
    meshes: &mut Assets<MeshGpu>,
    materials: &mut Assets<Material>,
    geo_table: &mut MeshGeometryTableSlot,
    dev: &GpuDevice,
    marked: bool,
) {
    let (verts, idx) = uv_sphere(0.62, 28, 40, [0.7, 0.7, 0.72, 1.0]);
    let sphere = match geo_table.0.as_mut() {
        Some(table) => meshes.register_mesh_vb(dev.get(), &verts, &idx, table),
        None => meshes.register_mesh(dev.get(), &verts, &idx),
    };
    let mat = materials.add(Material::new([0.72, 0.04, 0.04, 1.0], 0.0, 0.38, 0.5, [0.0; 3], 0));
    let mat_index = mat.index() as u16;

    let spacing = 1.55;
    for i in 0..5u32 {
        let x = (i as f32 - 2.0) * spacing;
        // A per-sphere Z stagger: five DISTINCT depth ranges on screen, so the base map's
        // `⌈t·S/P⌉` partition is exercised against a gradient instead of one constant plane.
        let z = (i as f32 - 2.0) * 0.55;
        let e = commands
            .spawn(MeshBundle::new(sphere, Transform::from_translation(Vec3::new(x, 0.6, z))))
            .id();
        // ⚠️ Queued into the SAME command flush as the spawn — NOT into the bundle. This kernel
        // has no tuple `Bundle` impl (`Bundle` is sealed and implemented per type), so
        // `spawn((MeshBundle, OcclusionCulling))` does not compile. What would arm the split one
        // frame late is an insert from a LATER frame; queued together, spawn and insert are
        // applied by ONE flush before any gather runs — the route `MaterialHandle` below already
        // takes.
        if marked {
            commands.entity(e).insert(OcclusionCulling);
        }
        commands.entity(e).insert(MaterialHandle(mat_index));
    }

    let sun_pose = Affine3A::look_at_rh(
        Vec3::ZERO,
        Vec3::new(SUN_DIR[0], SUN_DIR[1], SUN_DIR[2]),
        Vec3::new(0.0, 1.0, 0.0),
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

    let pose = Affine3A::look_at_rh(
        Vec3::new(0.0, 1.1, 7.8),
        Vec3::new(0.0, 0.55, 0.0),
        Vec3::new(0.0, 1.0, 0.0),
    );
    commands.spawn(CameraRig {
        transform: Transform {
            translation: pose.translation,
            rotation: Quat::from_mat3(pose.matrix3),
            scale: Vec3::ONE,
        },
        global: GlobalTransform::IDENTITY,
        camera: Camera::DEFAULT,
        projection: Projection::Perspective {
            fov_y: 52.0 * core::f32::consts::PI / 180.0,
            aspect: 1.0,
            near: 0.1,
            far: 100.0,
        },
    });
}

/// [`setup_scene`] with NO marker — gate G8's scene, the one every shipping frame's pass order
/// matches.
fn setup(
    mut commands: Commands,
    mut meshes: NonSendResMut<Assets<MeshGpu>>,
    mut materials: ResMut<Assets<Material>>,
    mut geo_table: NonSendResMut<MeshGeometryTableSlot>,
    dev: NonSendRes<GpuDevice>,
) {
    setup_scene(&mut commands, &mut meshes, &mut materials, &mut geo_table, &dev, false);
}

/// [`setup_scene`] WITH the marker — gate G5's scene: the occlusion split is armed, so the
/// `[hzb_poison, hzb_build_*]` block is declared and recorded BETWEEN the two raster scopes
/// instead of after the `lit` producer.
fn setup_occ(
    mut commands: Commands,
    mut meshes: NonSendResMut<Assets<MeshGpu>>,
    mut materials: ResMut<Assets<Material>>,
    mut geo_table: NonSendResMut<MeshGeometryTableSlot>,
    dev: NonSendRes<GpuDevice>,
) {
    setup_scene(&mut commands, &mut meshes, &mut materials, &mut geo_table, &dev, true);
}

/// The dump path, or `None` after announcing a SKIP.
///
/// A worker booted without [`ENV_DUMP`] arms no capture, so the host loop has nothing to complete
/// and `app.run()` never returns — a hang, which is the worst failure mode a test sweep can have.
/// The drivers always set it.
fn dump_path_or_skip(label: &str) -> Option<String> {
    match std::env::var(ENV_DUMP) {
        Ok(path) => {
            eprintln!("{label}: dumping to {path}");
            Some(path)
        }
        Err(_) => {
            eprintln!(
                "{label}: {ENV_DUMP} is unset -- SKIPPED. This worker exists to be spawned by its \
                 driver; booted without the knob it would render forever, since no armed capture \
                 could ever complete."
            );
            None
        }
    }
}

/// The path both workers request. Inserted AFTER `add_plugins` so it overrides
/// `RenderPathPlugin`'s `Deferred` default — `vb_mesh.rs`'s own post-plugins override discipline.
const VB_MESH_PATH: RenderPathConfig =
    RenderPathConfig { path: RenderPath::VisibilityBuffer, legs: GeometryLegs::Mesh };

/// **The G8 WORKER** — one process, one `VisibilityBuffer × Mesh` boot with the pyramid armed and
/// the occlusion split UNARMED, one `BOYKO_HZB_DUMP` file.
///
/// `HzbMode::Build` is what makes `GBufferScene::hzb` `Some`, which is what the dump, the poison
/// and the build chain all read.
#[test]
#[ignore = "needs a real windowed GPU device; the G8 driver spawns it with BOYKO_HZB_DUMP set"]
fn hzb_engine_pyramid_dump() {
    if dump_path_or_skip("hzb_engine_pyramid_dump").is_none() {
        return;
    }
    let mut app = App::new();
    app.add_plugins(EnginePlugins::window("boyko_engine hzb G8", EXTENT, EXTENT));
    app.add_startup_system(setup);
    app.insert_resource(VB_MESH_PATH);
    app.insert_resource(HzbConfig { mode: HzbMode::Build });
    app.run();
}

/// **The G5 WORKER** (VG R3 piece 2 step P2-6) — the same boot with the occlusion split ARMED, so
/// the `[hzb_poison, hzb_build_*]` block is declared and recorded BETWEEN the two raster scopes
/// and the pyramid is built from the EARLY scope's depth.
///
/// Armed-split AND armed-poison in the same frame, by construction: that is the configuration D6's
/// whole-block move exists for, and the one whose halves the declarator's `poison < build` assert
/// refuses to let drift apart (dev profile — which is what every golden and gate run uses).
#[test]
#[ignore = "needs a real windowed GPU device; the G5 driver spawns it with BOYKO_HZB_DUMP set"]
fn hzb_engine_pyramid_dump_occ() {
    if dump_path_or_skip("hzb_engine_pyramid_dump_occ").is_none() {
        return;
    }
    let mut app = App::new();
    app.add_plugins(EnginePlugins::window("boyko_engine hzb G5 (split)", EXTENT, EXTENT));
    app.add_startup_system(setup_occ);
    app.insert_resource(VB_MESH_PATH);
    app.insert_resource(HzbConfig { mode: HzbMode::Build });
    // VG R3 piece 4 rung P4-4: the OWNER conjunct, through THE single insert site. The regime is
    // fixed by the test (`None` — G5's subject is the poison/build block's POSITION on an armed
    // split, not a forced verdict), so the direct route rather than the env-driven one.
    occ_fixture::arm_occlusion_with(
        &mut app,
        boyko_render::OcclusionMode::TwoPhase,
        boyko_app::OcclusionForce::None,
    );
    app.run();
}

/// The `vb_occ_mixed` scene, fully marked, as an ECS startup system — G-P3-E's FORCE-LATE worker's
/// setup. One scene definition shared with `vb_mesh.rs`, `vb_occ_mixed.rs` and
/// `vg_occ_verdict_census.rs`; a second copy here would be a second text that can disagree with the
/// pins it is supposed to describe.
fn setup_mixed(
    mut commands: Commands,
    mut meshes: NonSendResMut<Assets<MeshGpu>>,
    mut materials: ResMut<Assets<Material>>,
    mut geo_table: NonSendResMut<MeshGeometryTableSlot>,
    dev: NonSendRes<GpuDevice>,
) {
    vb_occ_mixed_scene::spawn_mixed(
        &mut commands,
        &mut meshes,
        &mut materials,
        &mut geo_table,
        &dev,
        true,
    );
}

/// **G-P3-E's FORCE-LATE WORKER** (VG R3 piece 3 step P3-8) — the `vb_occ_mixed` scene under
/// `BOYKO_VG_OCC_FORCE=late`, so the early scope draws only the two UNMARKED instances and the LATE
/// scope draws the two marked survivors.
///
/// The regime comes from the env the driver sets: `occ_fixture` decodes `BOYKO_VG_OCC_FORCE` at
/// app setup and inserts it as the `OcclusionForce` Resource, once, before the first frame — so it
/// is still a property of the PROCESS, and the driver still selects it on the CHILD (VG R3 piece 4
/// rung P4-4 moved that decode out of `GpuSceneBundles::boot`, where it was shipping code).
#[test]
#[ignore = "needs a real windowed GPU device; the G-P3-E driver spawns it with BOYKO_HZB_DUMP and BOYKO_VG_OCC_FORCE=late"]
fn hzb_engine_pyramid_dump_occ_late() {
    if dump_path_or_skip("hzb_engine_pyramid_dump_occ_late").is_none() {
        return;
    }
    let mut app = App::new();
    app.add_plugins(EnginePlugins::window("boyko_engine hzb G-P3-E (force late)", EXTENT, EXTENT));
    app.add_startup_system(setup_mixed);
    app.insert_resource(VB_MESH_PATH);
    app.insert_resource(HzbConfig { mode: HzbMode::Build });
    // The MODE is the fixture's (`setup_mixed` always marks); the REGIME is the driver's env, and
    // it is G-P3-E's one variable — FORCE-LATE is the only configuration in which the two dumped
    // depths can differ on a static scene.
    let (_, force) = occ_fixture::occlusion_from_env();
    occ_fixture::arm_occlusion_with(&mut app, boyko_render::OcclusionMode::TwoPhase, force);
    app.run();
}

// ===============================================================================================
// The dump file, decoded from ITS OWN header
// ===============================================================================================

/// One decoded `BOYKO_HZB_DUMP` file.
///
/// Every field comes from the FILE. Nothing here is re-derived from the extent this test asked the
/// window for: the gate's whole subject is whether the engine's numbers are right, and a reader
/// that supplies its own numbers can only ever agree with itself.
struct Dump {
    /// `[width, height]` of the depth the pyramid reduced, from header words 1..2.
    source: [u32; 2],
    /// The pyramid's level count, from header word 3.
    levels: u32,
    /// The `flags` bitfield, from header word [`HZB_DUMP_WORD_FLAGS`].
    /// [`HZB_DUMP_FLAG_DEPTH_EARLY`] is its only bit today.
    flags: u32,
    /// The ENGINE frame the capture came from, from header word [`HZB_DUMP_WORD_FRAME_INDEX`] —
    /// stamped by the RECORDER inside the copy frame's command buffer.
    frame_index: u32,
    /// `level_extent[k]`, from header words `SCALAR + 2k` / `SCALAR + 2k + 1`. Exactly `levels`
    /// entries.
    level_extent: Vec<[u32; 2]>,
    /// The FRAME-END depth, row-major, `source[0] * source[1]` samples — this image as the frame
    /// left it, after the late raster scope (if the frame had one).
    depth_final: Vec<f32>,
    /// The EARLY-SCOPE depth, same shape — the bytes the pyramid was built from on a split frame.
    /// Live iff [`HZB_DUMP_FLAG_DEPTH_EARLY`]; otherwise every texel is the staging prefill.
    depth_early: Vec<f32>,
    /// Every mip, finest first, back to back, each row-major — kept as raw BITS, because that is
    /// the fidelity the comparison runs at.
    pyramid_bits: Vec<u32>,
}

/// Little-endian `u32` at word index `i`.
///
/// The dump is a `memcpy` of host memory on an x86_64 target (and this file is `cfg(windows)`), so
/// the file's endianness is the writer's; `from_le_bytes` states that rather than assuming it.
fn word(bytes: &[u8], i: usize) -> u32 {
    let o = i * 4;
    u32::from_le_bytes([bytes[o], bytes[o + 1], bytes[o + 2], bytes[o + 3]])
}

/// Decodes the dump, asserting every STRUCTURAL property before a payload byte is read.
///
/// A structural failure here names the instrument (a truncated file, an unrelated file, a level
/// count no layout admits) instead of surfacing as a texel mismatch, which would read as a broken
/// shader.
fn decode(bytes: &[u8], path: &Path) -> Dump {
    assert!(
        bytes.len() >= HZB_DUMP_HEADER_BYTES as usize,
        "{}: {} bytes is shorter than the {}-byte header — the dump was truncated, or the file is \
         not one",
        path.display(),
        bytes.len(),
        HZB_DUMP_HEADER_BYTES
    );
    let magic = word(bytes, 0);
    assert_eq!(
        magic, HZB_DUMP_MAGIC,
        "{}: leading word is 0x{magic:08x}, not HZB_DUMP_MAGIC (0x{HZB_DUMP_MAGIC:08x}). A stale \
         or unrelated file decoded as a pyramid would mismatch at every texel — a red gate naming \
         the wrong defect. ⚠️ VG R3 piece 3 step P3-7 BUMPED this value (from 0x485a4244) because \
         it widened the header and moved every payload offset: a pre-P3-7 file decoded here would \
         read `levels` out of what is now the `flags` slot, so the bump is what turns \"this file \
         predates the format\" into this one assertion.",
        path.display()
    );

    let source = [word(bytes, 1), word(bytes, 2)];
    let levels = word(bytes, 3);
    let flags = word(bytes, HZB_DUMP_WORD_FLAGS);
    let frame_index = word(bytes, HZB_DUMP_WORD_FRAME_INDEX);
    // Structural, like the two below it: an unknown bit means the writer carries a region or a
    // meaning this reader does not model, and every clause past here would be adjudicating a file
    // it does not understand.
    assert_eq!(
        flags & !HZB_DUMP_FLAG_DEPTH_EARLY,
        0,
        "{}: the header's flags are 0x{flags:08x}, which carries bits outside the set this reader \
         models (0x{HZB_DUMP_FLAG_DEPTH_EARLY:08x})",
        path.display()
    );
    assert!(
        source[0] > 0 && source[1] > 0,
        "{}: the header's source extent is {source:?} — a zero axis has no pyramid",
        path.display()
    );
    assert!(
        levels >= 1 && (levels as usize) <= MAX_HZB_LEVELS,
        "{}: the header's level count is {levels}, outside 1..={MAX_HZB_LEVELS}",
        path.display()
    );

    // The level-extent pairs start at the END of the scalar prefix — derived, never spelled, so the
    // writer's `header_words` and this walk cannot index two different tables.
    let extent_word = |k: usize| {
        let w0 = HZB_DUMP_HEADER_SCALAR_WORDS + 2 * k;
        [word(bytes, w0), word(bytes, w0 + 1)]
    };
    let mut level_extent = Vec::with_capacity(levels as usize);
    for k in 0..levels as usize {
        let e = extent_word(k);
        assert!(
            e[0] > 0 && e[1] > 0,
            "{}: the header's level {k} extent is {e:?} — a level with a zero axis has no storage",
            path.display()
        );
        level_extent.push(e);
    }
    // The tail past `levels` is written as ZERO on purpose (`HzbDumpLayout::header_words`), so a
    // reader cannot mistake the plan's padding for a real level.
    for k in levels as usize..HEADER_LEVEL_SLOTS {
        let e = extent_word(k);
        assert_eq!(
            e,
            [0, 0],
            "{}: header slot {k} is past `levels` ({levels}) but holds {e:?} rather than zero — \
             the padding would read as a plausible extent for a level with no storage behind it",
            path.display()
        );
    }

    let depth_texels = source[0] as usize * source[1] as usize;
    let pyramid_texels: usize =
        level_extent.iter().map(|e| e[0] as usize * e[1] as usize).sum();
    // TWO depth regions since VG R3 piece 3 step P3-7 — final then early — and both are counted
    // whether or not the early one was written, because the staging is sized before the frame that
    // fills it decides whether it splits.
    let want_bytes = HZB_DUMP_HEADER_BYTES as usize
        + (2 * depth_texels + pyramid_texels) * HZB_DUMP_SAMPLE_BYTES as usize;
    assert_eq!(
        bytes.len(),
        want_bytes,
        "{}: {} bytes, but the header describes {want_bytes} ({HZB_DUMP_HEADER_BYTES} header + \
         2 x {depth_texels} depth + {pyramid_texels} pyramid samples). The file and its own header \
         disagree, so nothing below it can be trusted.",
        path.display(),
        bytes.len()
    );

    let final_word0 = HZB_DUMP_HEADER_BYTES as usize / 4;
    let depth_final: Vec<f32> =
        (0..depth_texels).map(|i| f32::from_bits(word(bytes, final_word0 + i))).collect();
    let early_word0 = final_word0 + depth_texels;
    let depth_early: Vec<f32> =
        (0..depth_texels).map(|i| f32::from_bits(word(bytes, early_word0 + i))).collect();
    let pyramid_word0 = early_word0 + depth_texels;
    let pyramid_bits: Vec<u32> =
        (0..pyramid_texels).map(|i| word(bytes, pyramid_word0 + i)).collect();

    Dump { source, levels, flags, frame_index, level_extent, depth_final, depth_early, pyramid_bits }
}

/// ⚠️ **The OLD-MAGIC gate for step P3-7's format change, and the control that keeps it honest.**
///
/// The magic was bumped precisely so a pre-P3-7 dump fails LOUDLY instead of decoding against the
/// new offsets — where it would read `levels` out of what is now the `flags` slot, walk the extent
/// table two words early, and report a mismatch at every texel, i.e. a red gate naming the wrong
/// defect. A property stated only in prose is a property nobody executed, so this runs the decoder
/// over a file in the OLD format and requires it to panic.
///
/// # What makes it able to fail
///
/// * The stale file is **long enough for every other structural assertion to pass**
///   (`>= HZB_DUMP_HEADER_BYTES`, and its byte count is exactly what the OLD writer would have
///   produced), and the panic message is required to NAME the magic — so a green here cannot come
///   from the length check, and the test cannot pass merely because `decode` panics on everything.
/// * The old magic is spelled as a LITERAL, not derived from [`HZB_DUMP_MAGIC`]. Reverting the bump
///   would make the two equal and this test reds; deriving it (`HZB_DUMP_MAGIC - 18` and such)
///   would make it green by construction forever.
/// * The **positive control** below decodes a synthetic file in the CURRENT format and requires it
///   to succeed. Without it, deleting the format's payload arithmetic would leave the negative leg
///   green.
#[test]
fn a_stale_pre_p3_7_dump_fails_to_decode_instead_of_being_believed() {
    /// The `"HZBD"` magic every dump written before VG R3 piece 3 step P3-7 carries.
    const OLD_MAGIC: u32 = 0x485A_4244;
    assert_ne!(
        OLD_MAGIC, HZB_DUMP_MAGIC,
        "the P3-7 magic bump has been reverted: a stale dump would now decode against offsets it \
         was not written with, and mismatch at every texel instead of failing by name"
    );

    // A 4x4 source with a 3-level pyramid (4x4, 2x2, 1x1) — small, and shaped exactly as a real
    // header is, so the ONLY thing wrong with the stale file is its leading word.
    let source = [4u32, 4u32];
    let levels = [[4u32, 4u32], [2, 2], [1, 1]];
    let depth_texels = (source[0] * source[1]) as usize;
    let pyramid_texels: usize = levels.iter().map(|e| (e[0] * e[1]) as usize).sum();

    let build = |magic: u32, scalar_words: usize, depth_regions: usize| -> Vec<u8> {
        let mut words = vec![0u32; scalar_words + 2 * MAX_HZB_LEVELS];
        words[0] = magic;
        words[1] = source[0];
        words[2] = source[1];
        words[3] = levels.len() as u32;
        for (k, e) in levels.iter().enumerate() {
            words[scalar_words + 2 * k] = e[0];
            words[scalar_words + 2 * k + 1] = e[1];
        }
        words.resize(words.len() + depth_regions * depth_texels + pyramid_texels, 0);
        words.iter().flat_map(|w| w.to_le_bytes()).collect()
    };

    // The STALE file: the old magic, the old 4-word scalar prefix, ONE depth region — exactly what
    // the pre-P3-7 writer emitted.
    let stale = build(OLD_MAGIC, 4, 1);
    assert!(
        stale.len() >= HZB_DUMP_HEADER_BYTES as usize,
        "the stale fixture must clear the length check, or a green below would prove nothing about \
         the magic"
    );
    eprintln!(
        "a_stale_pre_p3_7_dump_fails_to_decode: the panic printed below is the ASSERTION UNDER \
         TEST, caught and inspected -- not a failure of this test."
    );
    // Matched rather than `expect_err`ed: `Dump` carries no `Debug`, and giving it one purely to
    // format a value this branch proves does not exist would be the tail wagging the dog.
    let err = match std::panic::catch_unwind(|| decode(&stale, Path::new("<stale pre-P3-7 dump>")))
    {
        Ok(_) => panic!(
            "a pre-P3-7 dump DECODED. The magic bump is the only thing standing between a stale \
             file and a texel-by-texel mismatch report that names the shader instead of the file."
        ),
        Err(e) => e,
    };
    let msg = err
        .downcast_ref::<String>()
        .map(String::as_str)
        .or_else(|| err.downcast_ref::<&str>().copied())
        .unwrap_or("<non-string panic payload>")
        .to_string();
    assert!(
        msg.contains("HZB_DUMP_MAGIC"),
        "the stale dump was rejected, but not BY THE MAGIC -- got {msg:?}. A rejection for another \
         reason (a length, a level count) would make this gate green on a file whose magic was \
         never checked."
    );

    // The POSITIVE CONTROL: the same fixture in the CURRENT format decodes. Without this leg, a
    // `decode` that panicked unconditionally would pass the assertion above.
    let current = build(HZB_DUMP_MAGIC, HZB_DUMP_HEADER_SCALAR_WORDS, 2);
    let dump = decode(&current, Path::new("<synthetic current-format dump>"));
    assert_eq!(dump.source, source);
    assert_eq!(dump.levels, levels.len() as u32);
    assert_eq!(dump.depth_final.len(), depth_texels);
    assert_eq!(dump.depth_early.len(), depth_texels);
    assert_eq!(dump.pyramid_bits.len(), pyramid_texels);
}

// ===============================================================================================
// The driver
// ===============================================================================================

/// Spawns `worker` (selected by name with `--exact`) and returns the file it wrote at
/// `out_name` — a DISTINCT path per worker, so two legs run in the same sitting without one
/// reading the other's artifact.
///
/// One process, as the census's `run_worker` is: the env knob has to exist before the engine boots,
/// and the host loop exits by returning from `app.run()` rather than by yielding control.
fn run_dump_worker(worker: &str, out_name: &str, extra_env: &[(&str, &str)]) -> (PathBuf, Vec<u8>) {
    let exe = std::env::current_exe().expect("invariant: the test binary knows its own path");
    let out = std::env::temp_dir().join(out_name);
    // A stale file from a previous run that this run failed to overwrite would be read as this
    // run's evidence.
    let _ = std::fs::remove_file(&out);

    let mut cmd = Command::new(&exe);
    cmd.args([worker, "--ignored", "--exact", "--test-threads=1", "--nocapture"])
        .env(ENV_DUMP, &out)
        .env("BOYKO_DISABLE_VALIDATION", "1")
        // The pyramid dump is the only capture this run is for. Another armed capture would render
        // the same frames for no reason AND hold the host loop open until it too completed.
        .env_remove("BOYKO_HOST_DUMP")
        .env_remove("BOYKO_VG_CENSUS")
        .env_remove("BOYKO_VB_PROBE")
        // VG R3 piece 3 step P3-8: the regime is a per-worker variable, so it is REMOVED first and
        // then set only by the leg that wants it. Inherited, a stray shell value would silently
        // make the converged legs forced ones and their fixed-point clause would red with no defect.
        .env_remove("BOYKO_VG_OCC_FORCE");
    for (k, v) in extra_env {
        cmd.env(k, v);
    }
    let status = cmd.status().expect("invariant: the worker process spawns");
    assert!(status.success(), "the dump worker `{worker}` exited {status}");

    let bytes = std::fs::read(&out).unwrap_or_else(|e| {
        panic!(
            "the dump worker `{worker}` wrote no file at {}: {e}. A worker that renders and \
             produces nothing is an instrument failure, not an empty pyramid.",
            out.display()
        )
    });
    (out, bytes)
}

/// VG R3 piece 3 step P3-7 (plan D10 / gate G-P3-E) — **which of the two dumped depths the caller
/// declares this worker produces, and what relation must hold between them.**
///
/// ⚠️ **The regime is DECLARED by the caller and then CHECKED against the file, never read out of
/// it.** Deriving it from the header's own `flags` bit would make a worker that silently failed to
/// arm the split take the unsplit branch and report green — the vacuity this campaign has shipped
/// twice. The caller knows whether its scene carries the marker; the file says what the recorder
/// did; the gate is the comparison of the two.
#[derive(Clone, Copy, PartialEq, Eq)]
enum DepthRegime {
    /// ONE raster scope: no `hzb_dump_depth_early` pass is declared, the early region is never
    /// written, and the pyramid was reduced from the frame-end depth because they are the same
    /// bytes by construction. The early region must still hold the host staging's prefill.
    NoEarlyRegion,
    /// TWO raster scopes on a CONVERGED, UNFORCED frame — G-P3-E's second regime. The pyramid was
    /// reduced from the EARLY depth, and by plan D12's fixed point the late scope draws zero, so
    /// the two depths must be BYTE-IDENTICAL. That is a positive falsifiable claim, not an
    /// embarrassment: if the late scope ever draws a pixel on a converged static frame, it reds.
    ///
    /// ⚠️ G-P3-E's FIRST regime landed at step P3-8 as [`Self::ForcedLateSplit`], with the
    /// `vb_occ_mixed` scene that can satisfy it. Until then this enum deliberately carried no
    /// variant for it: a variant with no constructor is dead code under `-D warnings`, and a clause
    /// asserted on a fixture that cannot satisfy it is a hard red with no defect present.
    ConvergedFixedPoint,
    /// TWO raster scopes with `VB_CULL_OCC_FORCE_LATE` set (VG R3 piece 3 step P3-8) — G-P3-E's
    /// FIRST regime, and the ONLY one on a static scene in which the late scope actually draws.
    ///
    /// The early phase defers every marked instance regardless of the pyramid, so the early depth
    /// holds only the UNMARKED occluder and filler while the frame-end depth also holds the two
    /// marked survivors the late scope drew. Two clauses become non-vacuous and both are asserted:
    ///
    /// * **clause 2** — `depth_early != depth_final` at ≥ 1 texel, guaranteed by construction;
    /// * **clause 3** — `build_pyramid(depth_final) != pyramid`, i.e. the pyramid was NOT built from
    ///   the final depth. **This is the ordering proof piece 2 could not make.** Clause 1 alone says
    ///   the pyramid agrees with a rebuild from the EARLY depth; on every earlier fixture the two
    ///   depths were the same bytes, so a build at EITHER slot agreed and the position was
    ///   unfalsifiable.
    ForcedLateSplit,
}

/// The comparison both gates run, over whichever worker's dump it is handed.
///
/// `leg` labels the configuration in every failure message and in the final report line, so a red
/// names WHICH of the two pass orders produced it instead of "the pyramid differs".
fn assert_engine_pyramid_equals_the_oracle(
    worker: &str,
    out_name: &str,
    leg: &str,
    regime: DepthRegime,
    extra_env: &[(&str, &str)],
) {
    let (path, bytes) = run_dump_worker(worker, out_name, extra_env);
    let dump = decode(&bytes, &path);
    let [source_w, source_h] = dump.source;

    // ---- clause 2: the header's shape IS the oracle's -------------------------------------------
    //
    // Asserted BEFORE the texel walk, because a wrong extent is a defect this gate exists to name.
    // Left to the comparison it would arrive as a wall of mismatches at every level, which reads as
    // a broken shader.
    let layout = HzbLayout::new(source_w, source_h).unwrap_or_else(|e| {
        panic!(
            "the dumped source extent {source_w}x{source_h} is not one the oracle admits ({e}) — \
             the engine built a pyramid over an extent `HzbLayout` refuses"
        )
    });
    assert_eq!(
        dump.levels,
        layout.levels(),
        "the engine's pyramid has {} levels over a {source_w}x{source_h} source; the oracle's has \
         {}. Level count is `msb(max(prev_pow2(w), prev_pow2(h))) + 1` and nothing else.",
        dump.levels,
        layout.levels()
    );
    for (k, got) in dump.level_extent.iter().enumerate() {
        let want = layout.level_extent(k as u32);
        assert_eq!(
            *got, want,
            "level {k}: the engine's extent is {got:?}, the oracle's {want:?}. Level 0 is \
             `prev_pow2` of each SOURCE axis (never the source extent itself) and every later \
             level is `max(1, base >> k)`."
        );
    }

    // ---- VG R3 piece 3 step P3-7 (plan D10 / G-P3-E): THE TWO DEPTHS ----------------------------
    //
    // The declared regime against the recorder's own stamp, FIRST — before any payload clause, for
    // the same reason clause 2 precedes the texel walk. A worker whose scene failed to arm the split
    // would otherwise fall through to a green comparison over the wrong region.
    let early_live = (dump.flags & HZB_DUMP_FLAG_DEPTH_EARLY) != 0;
    assert_eq!(
        early_live,
        regime != DepthRegime::NoEarlyRegion,
        "{leg}: the dump header's HZB_DUMP_FLAG_DEPTH_EARLY is {early_live}, but this worker's \
         scene declares the {} regime. The bit is latched AT the early-depth copy inside the \
         recorder, so a disagreement means the frame did not split the way the fixture says it \
         does — and every clause below would then be adjudicating the wrong region.",
        match regime {
            DepthRegime::NoEarlyRegion => "unsplit (no early region)",
            DepthRegime::ConvergedFixedPoint => "armed-split, converged",
            DepthRegime::ForcedLateSplit => "armed-split, FORCE-LATE",
        }
    );

    // THE depth the pyramid must equal a rebuild from. On a split frame that is the EARLY one — the
    // bytes the `hzb_build_*` dispatches reduced, copied between the two raster scopes. Choosing the
    // final one there is the exact blindness step P3-7 exists to remove.
    let source_depth: &[f32] = match regime {
        DepthRegime::NoEarlyRegion => dump.depth_final.as_slice(),
        DepthRegime::ConvergedFixedPoint | DepthRegime::ForcedLateSplit => {
            dump.depth_early.as_slice()
        }
    };

    match regime {
        // The early region was never written, so it must still be the host driver's prefill AT
        // EVERY TEXEL. Asserted rather than merely skipped: "nobody wrote it" and "something wrote
        // it and the reader ignored that" are different states, and only one of them is correct.
        DepthRegime::NoEarlyRegion => {
            let touched = dump.depth_early.iter().position(|d| d.to_bits() != STAGING_PREFILL_BITS);
            assert_eq!(
                touched, None,
                "{leg}: the early-depth region is not the 0x{STAGING_PREFILL_BITS:08x} staging \
                 prefill at index {touched:?} — but this frame declares ONE raster scope, so no \
                 `hzb_dump_depth_early` pass was declared and nothing should have copied there"
            );
        }
        // ---- G-P3-E clause 2' (the converged regime): the two depths are BYTE-IDENTICAL. ----
        //
        // Plan D12's fixed point, as a claim rather than a footnote: a rejected instance writes no
        // depth, so on a converged static frame the depth (and therefore the pyramid) is a fixed
        // point and both cull phases reject the same candidates — the late scope draws nothing and
        // cannot change these bytes. It reds the moment it does.
        //
        // ⚠️ Bit equality alone would ALSO be satisfied by two failed copies (both regions all
        // prefill), which is why the NaN clause below runs over BOTH regions in this regime.
        DepthRegime::ConvergedFixedPoint => {
            let differs = dump
                .depth_early
                .iter()
                .zip(dump.depth_final.iter())
                .position(|(e, f)| e.to_bits() != f.to_bits());
            assert_eq!(
                differs, None,
                "{leg}: the EARLY and FINAL depths differ at index {differs:?} \
                 (early={:?}, final={:?}). On a converged static frame the late scope draws ZERO \
                 (plan D12's fixed point: both phases evaluate one predicate over the same bytes), \
                 so a difference means the late scope drew — which no gate on this fixture can \
                 adjudicate as correct.",
                differs.map(|i| dump.depth_early[i]),
                differs.map(|i| dump.depth_final[i])
            );
            let final_nan = dump.depth_final.iter().position(|d| d.is_nan());
            assert_eq!(
                final_nan, None,
                "{leg}: the FINAL depth region holds a NaN at index {final_nan:?} — the staging's \
                 `0xFFFFFFFF` prefill showing through, i.e. the frame-end depth copy did not run"
            );
        }
        // ---- G-P3-E clause 2 (the FORCE-LATE regime): the two depths DIFFER. ----
        //
        // Guaranteed by construction here: the early scope draws only the two UNMARKED instances,
        // and the late scope then draws the two marked survivors into the same attachment. It is
        // ASSERTED rather than assumed because it is also the precondition clause 3 rests on — a
        // frame where the late scope drew nothing would make clause 3 a comparison of the pyramid
        // against a rebuild from the same bytes it was built from, i.e. green by construction.
        DepthRegime::ForcedLateSplit => {
            let final_nan = dump.depth_final.iter().position(|d| d.is_nan());
            assert_eq!(
                final_nan, None,
                "{leg}: the FINAL depth region holds a NaN at index {final_nan:?} — the staging's \
                 `0xFFFFFFFF` prefill showing through, i.e. the frame-end depth copy did not run"
            );
            let differs = dump
                .depth_early
                .iter()
                .zip(dump.depth_final.iter())
                .filter(|(e, f)| e.to_bits() != f.to_bits())
                .count();
            assert!(
                differs > 0,
                "{leg}: the EARLY and FINAL depths are BYTE-IDENTICAL over all {} texels. Under \
                 VB_CULL_OCC_FORCE_LATE the early phase defers EVERY marked instance, so the early \
                 scope draws only the two unmarked ones and the late scope draws the two marked \
                 survivors — the two depths differ BY CONSTRUCTION. Equality means the late scope \
                 drew nothing, and clause 3 below would then be green by construction.",
                dump.depth_final.len()
            );
            eprintln!(
                "{leg}: clause 2 — depth_early differs from depth_final at {differs} of {} texels",
                dump.depth_final.len()
            );
        }
    }

    // ---- clause 5 (depth half) + clause 3: the frame rendered something --------------------------
    let depth_nan = source_depth.iter().position(|d| d.is_nan());
    assert_eq!(
        depth_nan, None,
        "the dumped depth holds a NaN at index {:?}. A reverse-Z attachment is clamped to \
         [minDepth, maxDepth] and cannot carry one, and the host prefills the staging with \
         `0xFFFFFFFF` = NaN — so this is the depth COPY not having run, not a depth value.",
        depth_nan
    );
    let first_bits = source_depth[0].to_bits();
    let distinct = source_depth.iter().any(|d| d.to_bits() != first_bits);
    assert!(
        distinct,
        "every one of the {} dumped depth texels is the same value ({}) — the frame rendered \
         nothing, and comparing two reductions of a constant field proves nothing about either",
        source_depth.len(),
        source_depth[0]
    );
    let covered = source_depth.iter().filter(|d| **d > 0.0).count();
    assert!(
        covered > 0,
        "no dumped depth texel is > 0.0 — under reverse-Z that is the far plane at every pixel, \
         i.e. an empty frame"
    );

    // ---- the comparison ---------------------------------------------------------------------
    let mut oracle = vec![0.0f32; layout.pyramid_len()];
    build_pyramid(&layout, source_depth, &mut oracle);
    assert_eq!(
        dump.pyramid_bits.len(),
        oracle.len(),
        "the dumped pyramid holds {} texels and the oracle's layout {} — the two disagree about \
         the flat layout itself, which would report as a mismatch at every level but the first",
        dump.pyramid_bits.len(),
        oracle.len()
    );

    let poison_bits = HZB_PYRAMID_POISON.to_bits();
    let inf_bits = f32::INFINITY.to_bits();
    // Walked from the HEADER's extents on the dump side and from the oracle's layout on the other,
    // rather than from one shared offset table. The two are equal by the clause-2 assertions above;
    // deriving both from one of them would make that equality unfalsifiable here.
    let mut dump_off = 0usize;
    for level in 0..dump.levels {
        let [lw, lh] = dump.level_extent[level as usize];
        let oracle_off = layout.level_offset(level);
        assert_eq!(
            dump_off, oracle_off,
            "level {level} starts at dump texel {dump_off} but oracle texel {oracle_off} — the \
             two flat layouts have diverged"
        );

        let mut diff = 0usize;
        let mut first: Option<(u32, u32, u32, u32)> = None;
        for y in 0..lh {
            for x in 0..lw {
                let i = dump_off + (y as usize * lw as usize + x as usize);
                let gpu = dump.pyramid_bits[i];

                // ---- clause 1: the POISON. The one that holds at ANY scene coverage. ----
                assert_ne!(
                    gpu, poison_bits,
                    "level {level} texel ({x}, {y}) still holds the {HZB_PYRAMID_POISON} POISON — \
                     the engine never wrote it. The reduce is a `min` over reverse-Z depths in \
                     [0, 1] and cannot produce a negative value, so this texel was not written by \
                     any build dispatch. ⚠️ This is the clause plan §13 exists for: with the \
                     pyramid unpoisoned, an unwritten level reads `0.0`, which is the far plane, \
                     which is what the oracle computes for 89.3% of this scene — so the \
                     comparison below would have AGREED with a pyramid nobody built."
                );
                // ---- clause 5 (pyramid half): the staging poison. ----
                assert!(
                    !f32::from_bits(gpu).is_nan(),
                    "level {level} texel ({x}, {y}) is NaN (0x{gpu:08x}). `hzb_build`'s reduce \
                     collapses a NaN input to -INFINITY rather than propagating it, so a NaN here \
                     is the host staging's `0xFFFFFFFF` prefill showing through — the pyramid COPY \
                     did not cover this texel."
                );
                // ---- clause 4: the boundary rule's identity never survives. ----
                assert_ne!(
                    gpu, inf_bits,
                    "level {level} texel ({x}, {y}) is +INFINITY — the `min` identity a lane \
                     contributes when it folds NO live tap. Every texel that EXISTS has at least \
                     one live child, so this is a footprint that read nothing where it should have \
                     read something."
                );

                let want = oracle[oracle_off + (y as usize * lw as usize + x as usize)].to_bits();
                if gpu != want {
                    diff += 1;
                    if first.is_none() {
                        first = Some((x, y, gpu, want));
                    }
                }
            }
        }
        if let Some((x, y, gpu, want)) = first {
            // A ±0 difference is numerically EQUAL and no `<` in the chain can distinguish it —
            // plan §10 measured the driver fusing the shader's compare-and-select into a hardware
            // `min` whose tie-break returns the negative zero. Named here so a reader can tell that
            // case from a real disagreement; NOT tolerated, because on real reverse-Z depth (which
            // this dump is) it has never been observed and a silent allowance would be the third
            // way this campaign has found to make a gate agree with itself.
            let zero_tie = (gpu | want) & 0x7fff_ffff == 0;
            panic!(
                "level {level} ({lw}x{lh}) DIFFERS from the host oracle at ({x}, {y}): \
                 gpu_bits=0x{gpu:08x} oracle_bits=0x{want:08x} gpu={} oracle={} — {diff} of {} \
                 texels differ at this level{}",
                f32::from_bits(gpu),
                f32::from_bits(want),
                lw as usize * lh as usize,
                if zero_tie { " (a ±0 TIE: numerically equal, sign bit only — plan §10)" } else { "" }
            );
        }

        dump_off += lw as usize * lh as usize;
    }
    assert_eq!(
        dump_off,
        dump.pyramid_bits.len(),
        "the level walk covered {dump_off} of {} dumped texels",
        dump.pyramid_bits.len()
    );

    // ---- G-P3-E clause 3: the pyramid was NOT built from the FINAL depth ------------------------
    //
    // THE ORDERING PROOF. Clause 1 above says the pyramid equals a rebuild from the EARLY depth; on
    // its own that is satisfied by a build at either slot whenever the two depths are the same
    // bytes, which is every fixture in the tree before this one. Under FORCE-LATE they are not, so
    // the negative is decidable — and it is what turns "the build is correct" into "the build ran at
    // the right point in the frame".
    //
    // ⚠️ Asserted ONLY in this regime. On the converged one `depth_early == depth_final` byte for
    // byte (clause 2'), so the two rebuilds are the same array and this would be a hard red with no
    // defect present — the exact error the plan records round 1 making with clause 2.
    if regime == DepthRegime::ForcedLateSplit {
        let mut from_final = vec![0.0f32; layout.pyramid_len()];
        build_pyramid(&layout, &dump.depth_final, &mut from_final);
        let agreeing = from_final
            .iter()
            .zip(dump.pyramid_bits.iter())
            .filter(|(o, g)| o.to_bits() == **g)
            .count();
        assert_ne!(
            agreeing,
            from_final.len(),
            "{leg}: a pyramid rebuilt from the FINAL depth matches the engine's at EVERY one of \
             {} texels. The frame-end depth carries the late scope's two survivors, which the \
             early depth does not, so a build that reduced the EARLY depth cannot agree everywhere \
             with a reduction of the FINAL one. Equality here means the `[hzb_poison, hzb_build_*]` \
             block ran AFTER the late raster scope — the ordering defect this clause exists for, \
             and the one clause 1 alone is structurally blind to.",
            from_final.len()
        );
        eprintln!(
            "{leg}: clause 3 — build_pyramid(depth_final) agrees with the engine's pyramid at {} of \
             {} texels, i.e. NOT everywhere, so the pyramid was not reduced from the final depth",
            agreeing,
            from_final.len()
        );
    }

    // ---- the record ---------------------------------------------------------------------------
    //
    // The coverage is REPORTED, never gated on (plan §5 asked for a full-coverage fixture; the
    // poison removed the need and no such VB fixture exists). It is printed because §13's whole
    // point is that this number changes what a green here means: at 11% coverage the AGREEMENT
    // covers mostly far-plane zeros, and it is clause 1 — not the agreement — that says the
    // levels were written at all.
    let pct = 100.0 * covered as f64 / source_depth.len() as f64;
    println!(
        "hzb_build {leg}: engine pyramid at {source_w}x{source_h}, {} levels, {} texels BIT-EXACT \
         vs boyko_render::hzb::build_pyramid rebuilt from the engine's own {} depth (engine frame \
         {}, per the recorder's own header stamp). Depth coverage {covered}/{} = {pct:.2}% > 0.0 \
         (the rest is the reverse-Z far plane). Non-vacuity is carried by the -1.0 IMAGE poison, \
         not by this number.",
        dump.levels,
        dump.pyramid_bits.len(),
        if early_live { "EARLY-scope" } else { "frame-end" },
        dump.frame_index,
        source_depth.len()
    );
}

/// **GATE G8** — the pyramid the ENGINE built, through the engine's own extents, descriptors,
/// barriers and dispatches, equals `boyko_render::hzb::build_pyramid` rebuilt from the engine's own
/// dumped depth, to BITS, at every texel of every level — on the UNSPLIT frame, the one every
/// shipping frame and all 25 golden pins take.
///
/// See the module header for the five non-vacuity clauses and for why the POISON — not the scene's
/// coverage — is what makes the agreement mean anything.
#[test]
#[ignore = "live GPU gate (spawns a windowed worker); the orchestrator runs it with --test-threads=1"]
fn hzb_engine_pyramid_equals_the_oracle() {
    assert_engine_pyramid_equals_the_oracle(
        WORKER,
        "hzb_engine_pyramid_g8.bin",
        "G8 (unsplit)",
        DepthRegime::NoEarlyRegion,
        &[],
    );
}

/// **GATE G5** (VG R3 piece 2 step P2-6) — the same comparison on the ARMED-SPLIT frame, where the
/// `[hzb_poison, hzb_build_*]` block sits BETWEEN the two raster scopes instead of after the `lit`
/// producer.
///
/// # What it proves
///
/// The D6 slot move did not hand `hzb_build_0` a wrong or untransitioned image: the pyramid the
/// engine builds from the EARLIER slot is still bit-exact against the host oracle over the dumped
/// depth, with the `-1.0` poison and all five non-vacuity clauses intact. It also exercises, on a
/// real device, the configuration that is armed-split AND armed-poison at once — the one whose
/// declare-order asserts fire in the dev profile these runs use.
///
/// # VG R3 piece 3 step P3-7 (plan D10) — what the two-depth dump ADDS here, and what it still
/// # cannot claim on THIS fixture
///
/// Until step P3-7 the dump carried ONE depth, copied at frame end, and this leg compared the
/// pyramid against it. That was sound only while the late scope drew nothing; the sentences below
/// (kept, because they are the record of why) said so. The dump now carries BOTH depths, and this
/// leg is re-pointed at the EARLY one — the bytes the build actually reduced — so its clause 1 is
/// G-P3-E's clause 1 verbatim rather than a coincidence of the two being equal.
///
/// It also gains **G-P3-E clause 2'**: `depth_early == depth_final`, byte for byte, asserted. On a
/// converged static frame plan D12's fixed point makes the late scope draw zero, so this is a
/// positive falsifiable claim about the arming rather than an admission that nothing happened.
///
/// ⚠️ **Clauses 2 and 3 — `depth_early != depth_final` and `build_pyramid(depth_final) != pyramid`,
/// the pair that proves the ORDERING — are NOT here, and cannot be on this fixture.** They need a
/// frame where the late scope actually draws, which on a converged frame requires the FORCE-LATE
/// selector over a mixed-marking scene (`vb_occ_mixed_late`). That scene is authored at step P3-8;
/// asserting clause 2 here would be a hard red with no defect present, which is exactly the error
/// the plan records round 1 making.
///
/// # ⚠️ What it structurally CANNOT prove, in piece 2
///
/// **That the ORDERING is right.** The late scope draws nothing, so the early-scope depth and the
/// end-of-frame depth are the same bytes, and a pyramid built at EITHER slot agrees with the
/// oracle over the dumped depth. The ordering's real gate is piece 3's, and piece 3 must first
/// move the dump's own depth copy between the scopes (or dump both depths) — otherwise it would
/// compare the pyramid against a depth it was not built from. Naming this here is what stops a
/// green G5 from being read as ordering evidence.
///
/// # Why this is an ADDITION and not a conversion
///
/// See [`WORKER_OCC`]. Both pairs must be green in the same sitting; this one alone would leave
/// the unsplit pyramid path — every shipping frame's — with no engine-level gate at the very step
/// that makes its `hzb_build` slot conditional.
#[test]
#[ignore = "live GPU gate (spawns a windowed worker); the orchestrator runs it with --test-threads=1"]
fn hzb_engine_pyramid_equals_the_oracle_occ() {
    assert_engine_pyramid_equals_the_oracle(
        WORKER_OCC,
        "hzb_engine_pyramid_g5_occ.bin",
        "G5 (armed split)",
        DepthRegime::ConvergedFixedPoint,
        &[],
    );
}

/// **GATE G-P3-E, the FORCE-LATE regime** (VG R3 piece 3 step P3-8) — the same bit-exact comparison
/// on a frame where the LATE SCOPE ACTUALLY DRAWS, plus the two clauses that need it.
///
/// # What this leg adds that no other can
///
/// 1. **Clause 1 over a depth the late scope changed.** The pyramid is compared against a rebuild
///    from `depth_early`, which here holds ONLY the two unmarked instances — a genuinely different
///    array from the frame-end depth, so the choice of region is load-bearing rather than moot.
/// 2. **Clause 2**: `depth_early != depth_final` at ≥ 1 texel, asserted, and guaranteed by
///    construction — the two marked survivors are drawn by the late scope and by nothing else.
/// 3. **Clause 3**: `build_pyramid(depth_final) != pyramid`. **The ordering proof piece 2 could not
///    make.** Its own header says so in as many words: with the late scope drawing nothing, a
///    pyramid built at EITHER slot agreed with the oracle over the dumped depth.
///
/// # Controls
///
/// * **E1** — move the poison+build block back AFTER the late scope: clauses 1 and 3 both red.
/// * **E2** — swap the two depth regions in the dump writer: clause 1 reds HERE while clause 2'
///   stays green on [`hzb_engine_pyramid_equals_the_oracle_occ`], and that PAIR is what proves the
///   two regions are not interchangeable. E2's discriminating leg is this one: on the converged
///   fixture the two regions hold the same bytes, so swapping them is invisible there by
///   construction.
///
/// # What it CANNOT claim
///
/// That the UNFORCED early phase defers anything (that is `vb_occ_mixed.rs`'s clause 1), or anything
/// about the pyramid the FORCE-LATE early phase actually READ — under this regime `P_prev != P_cur`
/// in general and only one pyramid is dumped. That limit is narrowed, not closed, and closing it
/// would need a second dump pass.
#[test]
#[ignore = "live GPU gate (spawns one windowed worker); the orchestrator runs it with --test-threads=1"]
fn hzb_engine_pyramid_equals_the_oracle_force_late() {
    assert_engine_pyramid_equals_the_oracle(
        WORKER_OCC_LATE,
        "hzb_engine_pyramid_gp3e_late.bin",
        "G-P3-E (FORCE-LATE, vb_occ_mixed)",
        DepthRegime::ForcedLateSplit,
        &ENV_OCC_FORCE,
    );
}
