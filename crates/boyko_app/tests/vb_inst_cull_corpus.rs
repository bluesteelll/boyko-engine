//! **VG rung R2d-5 — the CORPUS gate: the rung's own premise, on committed content.**
//!
//! The real 45-instance, 7-asset corpus (`crates/boyko_app/tests/vg_corpus_scene`) booted on
//! `RenderPath::VisibilityBuffer × GeometryLegs::Mesh` at ladder rung 0 with the cull-readback
//! probe armed, at BOTH committed camera paths. The synthetic fixtures
//! (`vb_inst_cull_narrow.rs` / `vb_inst_cull_wide.rs`) prove the MECHANISM; this proves it on
//! content nobody arranged for it, which is what the previous design was blocked for lacking.
//!
//! # What THIS rung asserts — the INERT numbers, and why each is what this build produces
//!
//! `vb_batch_cull.comp.hlsl` ships its level-2 `keep` predicate HARDWIRED `true` (rung R2d-3), so
//! every instance survives and every record keeps the count the host's own transfer fill wrote:
//!
//! | | `batches` | `visible` | Σ `inst` |
//! |---|---|---|---|
//! | `orbit_mid` | 7 | 7 | 45 |
//! | `approach_close` | 7 | 7 | 45 |
//!
//! `visible = 7` because the per-BATCH (level-1) cull rejects NOTHING on this corpus at either
//! path, and every batch carries at least one instance — the `visible && k > 0u` gate is therefore
//! satisfied by all seven. Σ `inst` = 45 = `vg_corpus_scene::SLOT_COUNT`.
//!
//! # What rung R2d-6 flips them to — MEASURED, not predicted
//!
//! The numbers below come from the committed CPU census
//! `crates/boyko_app/tests/vg_cull_granularity_census.rs`, whose `PINNED` array was measured at
//! 512×512 against the production frustum and then pinned:
//!
//! | camera path | instances rejected | instances drawn | batches rejected |
//! |---|---|---|---|
//! | `approach_close` | **14** of 45 | **31** | 0 of 7 |
//! | `orbit_mid` | **1** of 45 | **44** | 0 of 7 |
//!
//! **The per-BATCH cull removes ZERO instances at BOTH paths**, so every instance the armed rung
//! removes is bought by per-INSTANCE granularity and by nothing else. That is the rung's premise,
//! measured on committed content.
//!
//! ⚠️ **`orbit_mid` is a CONTRAST control, not a zero-rejection control.** An earlier hand
//! computation said zero; the committed measurement says ONE. The contrast that matters is 14
//! against 1 — a framing where granularity buys a seventh of the scene against one where it buys
//! a single instance — not "some against none". Do not restate it as zero.
//!
//! ⚠️ `visible` at rung R2d-6 must be MEASURED, not carried over. It counts batches retaining at
//! least one survivor, and whether any of the seven loses all of its members to a 14-instance
//! rejection is not something this file knows.
//!
//! # The payload
//!
//! `assets/vg_corpus/` is fetched and gitignored. Without it this gate SKIPS BY NAME — a
//! payload-dependent gate that stays silent is indistinguishable from one that passed.
//!
//! `#[ignore]`: needs a real windowed GPU device. The driver spawns one worker process per
//! committed camera path (a windowed boot owns the device singleton, so one process cannot render
//! two paths).

#![cfg(windows)]

use boyko_app::prelude::*;
use boyko_ecs::ecs::core::system::ResMut;
use boyko_render::{
    GeometryLegs, Material, MeshAssetsVbExt, MeshGeometryTableSlot, RenderPath, RenderPathConfig,
};

mod vb_inst_cull_scene;
mod vg_corpus_scene;
mod vg_thresholds;

use vg_thresholds::{assert_thresholds_frozen, read_thresholds, resolution_ladder, route_for};

/// The env knob telling a worker which committed camera path it renders — the same name
/// `vg_r0d_census.rs` uses, because it is the same question asked of the same scene.
const ENV_PATH: &str = "BOYKO_VG_PATH";

/// The ladder rung the corpus is probed at: rung 0, the only 1:1 rung. It is also the rung
/// `vg_cull_granularity_census.rs` measured its `PINNED` counts at, so the R2d-6 expectations in
/// this file's header are about the SAME frustum this gate renders.
const PROBE_RUNG: usize = 0;

/// The extent [`PROBE_RUNG`] must resolve to. Asserted rather than assumed: every other ladder rung
/// is 16:9, and a 16:9 frustum is a horizontally WIDER field that rejects fewer instances.
const PROBE_EXTENT: (u32, u32) = (512, 512);

/// Instances the arrangement places — `vg_corpus_scene::SLOT_COUNT`, restated so a re-arranged grid
/// reds here with its own message instead of at a raw equality.
const EXPECTED_INSTANCES: usize = 45;

/// Batches the arrangement produces: one per registered mesh, and the manifest holds seven assets.
const EXPECTED_BATCHES: usize = 7;

/// The window extent the worker opens, read from the FROZEN ladder rather than invented here.
fn probe_extent() -> (u32, u32) {
    let src = read_thresholds();
    let rung = resolution_ladder(&src)[PROBE_RUNG];
    let (cw, ch, ssaa) = route_for(rung)
        .unwrap_or_else(|| panic!("ladder rung {PROBE_RUNG} {rung:?} has no route on this box"));
    assert_eq!(
        ssaa, 1,
        "rung {PROBE_RUNG} must render at its client extent: with SSAA armed the composite extent \
         the raster push is built from is not {cw}x{ch}"
    );
    assert_eq!(
        (cw, ch),
        PROBE_EXTENT,
        "the R2d-6 expectations in this file's header were measured against the {PROBE_EXTENT:?} \
         (1:1) frustum; this rung now renders {cw}x{ch}"
    );
    (cw, ch)
}

// ===============================================================================================
// The worker: one process, one committed camera path, one probe line
// ===============================================================================================

/// The corpus scene, copied from `vg_r0d_census.rs`'s `setup_corpus` for the reason every fixture
/// in this directory keeps its own spawn code: a shared helper edited for one census silently
/// re-shapes the other, and these two must be able to disagree loudly rather than quietly.
fn setup_corpus(
    mut commands: Commands,
    mut meshes: NonSendResMut<Assets<MeshGpu>>,
    mut materials: ResMut<Assets<Material>>,
    mut geo_table: NonSendResMut<MeshGeometryTableSlot>,
    dev: NonSendRes<GpuDevice>,
) {
    let path = vg_corpus_scene::path_by_id(
        &std::env::var(ENV_PATH).expect("the worker is told its camera path"),
    );

    let mat = materials.add(Material::new([0.72, 0.70, 0.68, 1.0], 0.0, 0.45, 0.5, [0.0; 3], 0));
    let assets = vg_corpus_scene::decode_corpus();

    // Register each MESH exactly once, then place it in every slot the arrangement gives it — one
    // batch per asset, which is what makes `batches == 7`.
    let handles: Vec<MeshHandle> = assets
        .iter()
        .map(|asset| match geo_table.0.as_mut() {
            Some(table) => meshes.register_mesh_vb(dev.get(), &asset.vertices, &asset.indices, table),
            None => meshes.register_mesh(dev.get(), &asset.vertices, &asset.indices),
        })
        .collect();

    for slot in 0..vg_corpus_scene::SLOT_COUNT {
        let ai = vg_corpus_scene::slot_asset(slot, assets.len());
        let asset = &assets[ai];
        let pos = vg_corpus_scene::slot_position(slot);
        // `world = scale * v + translation`, so centring on the asset's own bounds is folded into
        // the translation rather than baked into the vertices.
        let s = asset.scale;
        let t = Vec3::new(
            pos[0] - s * asset.centre[0],
            pos[1] - s * asset.centre[1],
            pos[2] - s * asset.centre[2],
        );
        let e = commands
            .spawn(MeshBundle::new(
                handles[ai],
                Transform { translation: t, rotation: Quat::IDENTITY, scale: Vec3::new(s, s, s) },
            ))
            .id();
        commands.entity(e).insert(MaterialHandle(mat.index() as u16));
    }

    let sun = [-0.40, 0.78, 0.48];
    let sun_pose =
        Affine3A::look_at_rh(Vec3::ZERO, Vec3::new(sun[0], sun[1], sun[2]), Vec3::new(0.0, 1.0, 0.0));
    commands.spawn(DirectionalLightObject {
        transform: Transform {
            translation: Vec3::ZERO,
            rotation: Quat::from_mat3(sun_pose.matrix3),
            scale: Vec3::ONE,
        },
        global: GlobalTransform::IDENTITY,
        light: DirectionalLight::new(sun, [1.0, 0.97, 0.92], 3.1),
    });
    commands.spawn(SkyLight::new([0.38, 0.44, 0.55], [0.20, 0.20, 0.22]));

    let pose = Affine3A::look_at_rh(
        Vec3::new(path.eye[0], path.eye[1], path.eye[2]),
        Vec3::new(path.target[0], path.target[1], path.target[2]),
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
            fov_y: path.fov_y_degrees * core::f32::consts::PI / 180.0,
            aspect: 1.0,
            near: 0.05,
            far: 200.0,
        },
    });
}

/// **The corpus probe WORKER** — one process, one committed camera path, one `VB_CULL_READBACK`
/// line. Driven by [`vb_inst_cull_corpus_gate`]; the probe path arrives in the environment.
#[test]
#[ignore = "needs a real windowed GPU device and the fetched corpus payload; the R2d-5 corpus gate spawns it per camera path"]
fn vb_inst_cull_corpus_worker() {
    // The worker is meaningless without the camera path the gate hands it, and `--ignored` runs
    // BOTH tests in this binary — so a direct invocation lands here with nothing set. Skip BY NAME
    // rather than panicking: a panic here reads as a real failure of the corpus gate, and a silent
    // return reads as a pass. Neither is true; this process simply was not the one being asked.
    if std::env::var(ENV_PATH).is_err() {
        eprintln!(
            "SKIP vb_inst_cull_corpus_worker: no `{ENV_PATH}` in the environment, so this process \
             was not spawned by `vb_inst_cull_corpus_gate`. NOTHING about the corpus is measured \
             by this run — the gate is the test that measures it."
        );
        return;
    }
    if !vg_corpus_scene::payload_present() {
        eprintln!(
            "SKIP vb_inst_cull_corpus_worker: the gitignored corpus payload is absent (run \
             scripts/fetch_corpus.ps1). NOTHING about per-instance culling on the corpus is \
             measured by this run."
        );
        return;
    }
    let (cw, ch) = probe_extent();
    let mut app = App::new();
    app.add_plugins(EnginePlugins::window("boyko_engine vb inst cull corpus", cw, ch));
    app.add_startup_system(setup_corpus);
    app.insert_resource(RenderPathConfig {
        path: RenderPath::VisibilityBuffer,
        legs: GeometryLegs::Mesh,
    });
    app.run();
}

// ===============================================================================================
// The gate
// ===============================================================================================

/// **The corpus gate.** Every committed camera path, on the real content, with the inert numbers
/// this build produces asserted per path.
#[test]
#[ignore = "needs a real windowed GPU device and the fetched corpus payload; drives one worker process per committed camera path"]
fn vb_inst_cull_corpus_gate() {
    if !vg_corpus_scene::payload_present() {
        eprintln!(
            "SKIP vb_inst_cull_corpus_gate: the gitignored corpus payload is absent (run \
             scripts/fetch_corpus.ps1). NOTHING about per-instance culling on the corpus is \
             observed by this run."
        );
        return;
    }
    // The pins in this file's header are counts against a FROZEN ladder; a silently edited
    // thresholds file would move the extent without moving anything this gate can see.
    assert_thresholds_frozen();
    let _ = probe_extent();

    let committed = vg_corpus_scene::committed_camera_paths();
    assert!(
        committed.len() >= 2,
        "the corpus must commit at least two camera paths, or the 14-against-1 contrast rung R2d-6 \
         reads has only one side"
    );

    for path in &committed {
        let probe = vb_inst_cull_scene::run_cull_probe_worker(
            "vb_inst_cull_corpus_worker",
            &format!("corpus_{path}"),
            &[(ENV_PATH, path.clone())],
        );
        eprintln!("VG R2d-5 corpus `{path}`: {}", probe.raw);

        assert_eq!(
            probe.batches, EXPECTED_BATCHES,
            "`{path}`: the arrangement registers one mesh per manifest asset, so the frame must \
             submit {EXPECTED_BATCHES} batches. A smaller number means an asset was not `Loaded` \
             on the probed frame and the instance total below is about a partial scene -- got {:?}",
            probe.raw
        );
        assert_eq!(
            probe.visible as usize, EXPECTED_BATCHES,
            "`{path}`: the per-BATCH cull rejects NOTHING on this corpus (0 of 7, measured by \
             `vg_cull_granularity_census.rs`) and every batch carries instances, so all \
             {EXPECTED_BATCHES} satisfy the shader's `visible && k > 0u` gate. ⚠️ At rung R2d-6 \
             this must be RE-MEASURED, not carried over: it counts batches retaining at least one \
             SURVIVOR -- got {:?}",
            probe.raw
        );
        assert_eq!(
            probe.inst.len(),
            EXPECTED_BATCHES,
            "`{path}`: one indirect record per drawn batch -- got {:?}",
            probe.raw
        );
        assert_eq!(
            probe.drawn_instances() as usize,
            EXPECTED_INSTANCES,
            "`{path}`: with `keep` hardwired `true` every one of the arrangement's \
             {EXPECTED_INSTANCES} instances is still drawn. Rung R2d-6 makes this 31 for \
             `approach_close` (14 rejected) and 44 for `orbit_mid` (1 rejected) -- got {:?}",
            probe.raw
        );

        // The survivor REGIONS, per batch — the datum a flat prefix could not express.
        assert_eq!(
            probe.vis.len(),
            EXPECTED_BATCHES,
            "`{path}`: one survivor region per drawn batch -- got {:?}",
            probe.raw
        );
        let region_total: usize = probe.vis.iter().map(|(_, m)| m.len()).sum();
        assert_eq!(
            region_total, EXPECTED_INSTANCES,
            "`{path}`: the per-batch regions must account for every instance -- got {:?}",
            probe.raw
        );
        for (b, (base, members)) in probe.vis.iter().enumerate() {
            assert_eq!(
                members.len() as u32, probe.inst[b],
                "`{path}` batch {b}: the region length must equal the record's `instanceCount`, \
                 which is what the rasterizer fetches -- got {:?}",
                probe.raw
            );
            assert_eq!(
                *members,
                (0..members.len() as u32).map(|i| base + i).collect::<Vec<_>>(),
                "`{path}` batch {b}: the survivor region must be the IDENTITY run on this build. \
                 Rung R2d-6 makes these regions non-contiguous -- got {:?}",
                probe.raw
            );
        }
        // Bases are strictly ascending — the property `vb_cull_batch_count_visible_clamp`'s prefix
        // argument rests on, observed here on real content rather than argued.
        for w in probe.vis.windows(2) {
            assert!(
                w[1].0 > w[0].0,
                "`{path}`: batch bases must be strictly ascending -- got {:?}",
                probe.raw
            );
        }
    }
}

/// The counts this file pins are the corpus's own — checked without a device. Skips BY NAME
/// without the payload; NOT `#[ignore]`d otherwise.
#[test]
fn the_corpus_shape_matches_the_pinned_counts() {
    assert_eq!(
        vg_corpus_scene::SLOT_COUNT, EXPECTED_INSTANCES,
        "the arrangement now places {} instances; every instance count in this file is a count of \
         a different thing",
        vg_corpus_scene::SLOT_COUNT
    );
    if !vg_corpus_scene::payload_present() {
        eprintln!(
            "SKIP the_corpus_shape_matches_the_pinned_counts (asset half): the gitignored corpus \
             payload is absent (run scripts/fetch_corpus.ps1)"
        );
        return;
    }
    assert_eq!(
        vg_corpus_scene::manifest_assets().len(),
        EXPECTED_BATCHES,
        "one batch per registered mesh; the manifest now holds a different number of assets, which \
         also re-deals every slot through `slot_asset`"
    );
}
