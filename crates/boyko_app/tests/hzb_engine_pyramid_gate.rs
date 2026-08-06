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
//! # TWO worker+driver pairs since VG R3 piece 2 (gate G5)
//!
//! Piece 2's occlusion split moves the whole `[hzb_poison, hzb_build_*]` block BETWEEN the two
//! raster scopes on an armed-split frame (plan D6), which makes the block's slot **conditional**.
//! So this file now runs the same comparison twice:
//!
//! * [`hzb_engine_pyramid_equals_the_oracle`] — the UNSPLIT frame (gate G8, unchanged). This is
//!   the configuration every shipping frame and all 25 golden pins take.
//! * [`hzb_engine_pyramid_equals_the_oracle_occ`] — the ARMED-SPLIT frame (gate G5), where the
//!   block sits at its early slot.
//!
//! ⚠️ The second is an **ADDITION**. Re-pointing the existing worker at the marked scene would
//! have DELETED the unsplit leg — the only engine-level gate over the path piece 2 newly made
//! conditional. **Both must be green in the same sitting.**
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
    HZB_DUMP_HEADER_BYTES, HZB_DUMP_HEADER_WORDS, HZB_DUMP_MAGIC, HZB_DUMP_SAMPLE_BYTES,
    HZB_PYRAMID_POISON, MAX_HZB_LEVELS,
};

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

/// The worker window's client extent. 512² is the extent step P1-6 measured the dump at
/// (`levels = 10`, two build passes), so the coverage this gate reports is directly comparable with
/// plan §13's table — and two passes is what makes the SECOND pass (§13's blind spot) reachable.
const EXTENT: u32 = 512;

/// The sun direction TO the light (byte-identical to `vb_mesh.rs`'s).
const SUN_DIR: [f32; 3] = [-0.40, 0.78, 0.48];

/// How many `[w, h]` pairs the fixed-size header carries — `levels` of them are live and the rest
/// are zero padding. Derived from the header's own word count rather than spelled, and tied to the
/// capacity it must equal, so the two cannot drift into a reader that walks the wrong tail.
const HEADER_LEVEL_SLOTS: usize = (HZB_DUMP_HEADER_WORDS - 4) / 2;
const _: () = assert!(HEADER_LEVEL_SLOTS == MAX_HZB_LEVELS);

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
    /// `level_extent[k]`, from header words `4 + 2k` / `5 + 2k`. Exactly `levels` entries.
    level_extent: Vec<[u32; 2]>,
    /// The source depth, row-major, `source[0] * source[1]` samples.
    depth: Vec<f32>,
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
         the wrong defect.",
        path.display()
    );

    let source = [word(bytes, 1), word(bytes, 2)];
    let levels = word(bytes, 3);
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

    let mut level_extent = Vec::with_capacity(levels as usize);
    for k in 0..levels as usize {
        let e = [word(bytes, 4 + 2 * k), word(bytes, 5 + 2 * k)];
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
        let e = [word(bytes, 4 + 2 * k), word(bytes, 5 + 2 * k)];
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
    let want_bytes = HZB_DUMP_HEADER_BYTES as usize
        + (depth_texels + pyramid_texels) * HZB_DUMP_SAMPLE_BYTES as usize;
    assert_eq!(
        bytes.len(),
        want_bytes,
        "{}: {} bytes, but the header describes {want_bytes} (152 + {depth_texels} depth + \
         {pyramid_texels} pyramid samples). The file and its own header disagree, so nothing below \
         it can be trusted.",
        path.display(),
        bytes.len()
    );

    let depth_word0 = HZB_DUMP_HEADER_BYTES as usize / 4;
    let depth: Vec<f32> =
        (0..depth_texels).map(|i| f32::from_bits(word(bytes, depth_word0 + i))).collect();
    let pyramid_word0 = depth_word0 + depth_texels;
    let pyramid_bits: Vec<u32> =
        (0..pyramid_texels).map(|i| word(bytes, pyramid_word0 + i)).collect();

    Dump { source, levels, level_extent, depth, pyramid_bits }
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
fn run_dump_worker(worker: &str, out_name: &str) -> (PathBuf, Vec<u8>) {
    let exe = std::env::current_exe().expect("invariant: the test binary knows its own path");
    let out = std::env::temp_dir().join(out_name);
    // A stale file from a previous run that this run failed to overwrite would be read as this
    // run's evidence.
    let _ = std::fs::remove_file(&out);

    let status = Command::new(&exe)
        .args([worker, "--ignored", "--exact", "--test-threads=1", "--nocapture"])
        .env(ENV_DUMP, &out)
        .env("BOYKO_DISABLE_VALIDATION", "1")
        // The pyramid dump is the only capture this run is for. Another armed capture would render
        // the same frames for no reason AND hold the host loop open until it too completed.
        .env_remove("BOYKO_HOST_DUMP")
        .env_remove("BOYKO_VG_CENSUS")
        .env_remove("BOYKO_VB_PROBE")
        .status()
        .expect("invariant: the worker process spawns");
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

/// The comparison both gates run, over whichever worker's dump it is handed.
///
/// `leg` labels the configuration in every failure message and in the final report line, so a red
/// names WHICH of the two pass orders produced it instead of "the pyramid differs".
fn assert_engine_pyramid_equals_the_oracle(worker: &str, out_name: &str, leg: &str) {
    let (path, bytes) = run_dump_worker(worker, out_name);
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

    // ---- clause 5 (depth half) + clause 3: the frame rendered something --------------------------
    let depth_nan = dump.depth.iter().position(|d| d.is_nan());
    assert_eq!(
        depth_nan, None,
        "the dumped depth holds a NaN at index {:?}. A reverse-Z attachment is clamped to \
         [minDepth, maxDepth] and cannot carry one, and the host prefills the staging with \
         `0xFFFFFFFF` = NaN — so this is the depth COPY not having run, not a depth value.",
        depth_nan
    );
    let first_bits = dump.depth[0].to_bits();
    let distinct = dump.depth.iter().any(|d| d.to_bits() != first_bits);
    assert!(
        distinct,
        "every one of the {} dumped depth texels is the same value ({}) — the frame rendered \
         nothing, and comparing two reductions of a constant field proves nothing about either",
        dump.depth.len(),
        dump.depth[0]
    );
    let covered = dump.depth.iter().filter(|d| **d > 0.0).count();
    assert!(
        covered > 0,
        "no dumped depth texel is > 0.0 — under reverse-Z that is the far plane at every pixel, \
         i.e. an empty frame"
    );

    // ---- the comparison ---------------------------------------------------------------------
    let mut oracle = vec![0.0f32; layout.pyramid_len()];
    build_pyramid(&layout, &dump.depth, &mut oracle);
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

    // ---- the record ---------------------------------------------------------------------------
    //
    // The coverage is REPORTED, never gated on (plan §5 asked for a full-coverage fixture; the
    // poison removed the need and no such VB fixture exists). It is printed because §13's whole
    // point is that this number changes what a green here means: at 11% coverage the AGREEMENT
    // covers mostly far-plane zeros, and it is clause 1 — not the agreement — that says the
    // levels were written at all.
    let pct = 100.0 * covered as f64 / dump.depth.len() as f64;
    println!(
        "hzb_build {leg}: engine pyramid at {source_w}x{source_h}, {} levels, {} texels BIT-EXACT \
         vs boyko_render::hzb::build_pyramid rebuilt from the engine's own depth. Depth coverage \
         {covered}/{} = {pct:.2}% > 0.0 (the rest is the reverse-Z far plane). Non-vacuity is \
         carried by the -1.0 IMAGE poison, not by this number.",
        dump.levels,
        dump.pyramid_bits.len(),
        dump.depth.len()
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
    assert_engine_pyramid_equals_the_oracle(WORKER, "hzb_engine_pyramid_g8.bin", "G8 (unsplit)");
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
    );
}
