//! **VG rung R2d-6 — the CORPUS gate: the armed cull against an INDEPENDENT ORACLE.**
//!
//! The real 45-instance, 7-asset corpus (`crates/boyko_app/tests/vg_corpus_scene`) booted on
//! `RenderPath::VisibilityBuffer × GeometryLegs::Mesh` at ladder rung 0 with the cull-readback
//! probe armed, at BOTH committed camera paths. The synthetic fixtures
//! (`vb_inst_cull_narrow.rs` / `vb_inst_cull_wide.rs`) prove the MECHANISM; this proves it on
//! content nobody arranged for it, which is what the previous design was blocked for lacking.
//!
//! # ⚠️ THIS IS A CROSS-ORACLE AGREEMENT CHECK, NOT A PREDICTION
//!
//! The instance counts below are NOT guesses about what the GPU will do. They were MEASURED by the
//! committed CPU census `crates/boyko_app/tests/vg_cull_granularity_census.rs`, whose `PINNED`
//! array — `[(1, 0), (14, 0)]` in `PATHS` order — was read off the decoded payload at 512×512
//! against the PRODUCTION frustum (extracted from the raster push bytes, the same six planes the
//! cull is handed) and then pinned. That census is an independent implementation of the same
//! predicate: `boyko_render::frustum`'s host oracle over the same instance ring and the same
//! per-mesh bounds.
//!
//! So a disagreement here is a FINDING, and it is a SHADER bug rather than a math bug — the planes
//! are extracted once, on the host, and pushed; two evaluators of one predicate on one set of
//! numbers must agree. **It must be reported, never "fixed" by editing the expectation below.**
//! Editing these numbers to match a red run destroys the only cross-oracle evidence the rung has.
//!
//! # What THIS rung asserts
//!
//! | | `batches` | `visible` | Σ `inst` | rejected |
//! |---|---|---|---|---|
//! | `orbit_mid` | 7 | 7 | **44** | 1 of 45 |
//! | `approach_close` | 7 | 7 | **31** | 14 of 45 |
//!
//! Rung R2d-5 asserted Σ `inst` = 45 at both paths, because `keep` was hardwired `true`.
//!
//! `batches = 7` is a property of the HOST's submission (one batch per registered mesh) and cannot
//! move with a cull at all.
//!
//! `visible = 7` is DERIVED, and the derivation is what makes it assertable rather than carried
//! over. The counter's gate is `visible && k > 0u`, so a batch drops out only if level 1 rejects it
//! (the census measures 0 of 7 at both paths) or if EVERY one of its instances fails level 2. The
//! arrangement gives each of the seven assets 6 or 7 slots (`slot_asset` is `slot % 7` over 45
//! slots), scattered across all three depth layers because 7 is coprime with the 15-slot layer —
//! so emptying one batch would take at least 6 rejections landing on one asset's scattered slots,
//! out of 14 total that are spatially clustered near the frustum edge. If a path nonetheless
//! reports `visible < 7`, that is a finding about the arrangement, to be read from the `vis=`
//! groups rather than assumed away.
//!
//! ⚠️ **`orbit_mid` is a CONTRAST control, not a zero-rejection control.** An earlier hand
//! computation said zero; the committed measurement says ONE. The contrast that matters is 14
//! against 1 — a framing where granularity buys a seventh of the scene against one where it buys
//! a single instance — not "some against none". Do not restate it as zero.
//!
//! # The per-BATCH cull removes ZERO instances at BOTH paths
//!
//! (`vg_cull_granularity_census.rs`, the `0` in each `PINNED` pair.) So every instance this gate
//! observes removed is bought by per-INSTANCE granularity and by nothing else. That is the rung's
//! premise, measured on committed content and now observed on the GPU.
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

/// **The independent oracle's numbers**: instances the per-INSTANCE frustum test rejects at each
/// committed camera path, keyed BY PATH ID.
///
/// Copied from the committed CPU census `crates/boyko_app/tests/vg_cull_granularity_census.rs`,
/// whose `PINNED` array is `[(1, 0), (14, 0)]` in `PATHS` order — the first element of each pair.
/// Measured there at [`PROBE_EXTENT`] against the production frustum, on the decoded payload, by
/// `boyko_render::frustum`'s host oracle. Not predicted here; see this file's header for why a
/// disagreement is a shader bug to REPORT rather than an expectation to edit.
///
/// Keyed by id rather than positionally: `committed_camera_paths()` and `PATHS` could be reordered
/// without either noticing, and a silently transposed pair would compare `orbit_mid` against
/// `approach_close`'s count — a wrong number wearing a green test.
const CENSUS_REJECTIONS: [(&str, usize); 2] = [("orbit_mid", 1), ("approach_close", 14)];

/// Instances the armed cull must DRAW at `path` — [`EXPECTED_INSTANCES`] minus what the CPU census
/// measured as rejected there.
///
/// # Panics
///
/// Panics for a camera path the census never measured: a gate that quietly assumed "45, nothing
/// rejected" for a new path would report the arming as inert on it.
fn expected_drawn(path: &str) -> usize {
    let rejected = CENSUS_REJECTIONS
        .iter()
        .find(|(id, _)| *id == path)
        .map(|(_, n)| *n)
        .unwrap_or_else(|| {
            panic!(
                "camera path `{path}` has no measured rejection count in `CENSUS_REJECTIONS`. \
                 Measure it with `vg_cull_granularity_census.rs` and copy the number in — do not \
                 default it, because the default would read as `the cull rejects nothing here`"
            )
        });
    EXPECTED_INSTANCES - rejected
}

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
#[ignore = "needs a real windowed GPU device and the fetched corpus payload; the corpus gate spawns it per camera path"]
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

/// **The corpus gate.** Every committed camera path, on the real content, with the ARMED numbers
/// asserted per path — each of them a count the CPU census measured independently first.
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
        eprintln!("VG R2d-6 corpus `{path}`: {}", probe.raw);

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
             `vg_cull_granularity_census.rs`), so a batch can drop out of this count only by \
             losing EVERY one of its 6-7 instances to the armed level-2 test -- see this file's \
             header for why the arrangement's scattering makes that unreachable with \
             {} rejections. A smaller number here is a finding about WHICH instances were \
             rejected, readable from the `vis=` groups -- got {:?}",
            EXPECTED_INSTANCES - expected_drawn(path),
            probe.raw
        );
        assert_eq!(
            probe.inst.len(),
            EXPECTED_BATCHES,
            "`{path}`: one indirect record per drawn batch -- got {:?}",
            probe.raw
        );
        let drawn = expected_drawn(path);
        assert_eq!(
            probe.drawn_instances() as usize,
            drawn,
            "`{path}`: the armed cull must draw {drawn} of the arrangement's {EXPECTED_INSTANCES} \
             instances -- {} rejected, the count the CPU census `vg_cull_granularity_census.rs` \
             MEASURED with the same predicate on the same frustum. {EXPECTED_INSTANCES} means the \
             level-2 predicate is still rung R2d-5's constant `true`; any other number is a \
             DISAGREEMENT BETWEEN TWO ORACLES over one predicate, which is a shader bug and must \
             be reported rather than pinned away by editing `CENSUS_REJECTIONS` -- got {:?}",
            EXPECTED_INSTANCES - drawn,
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
            region_total, drawn,
            "`{path}`: the per-batch regions must account for every DRAWN instance -- got {:?}",
            probe.raw
        );
        // Bases are strictly ascending — the property `vb_cull_batch_count_visible_clamp`'s prefix
        // argument rests on, observed here on real content rather than argued. Checked BEFORE the
        // per-batch loop below, which uses the next base as a region's upper bound.
        for w in probe.vis.windows(2) {
            assert!(
                w[1].0 > w[0].0,
                "`{path}`: batch bases must be strictly ascending -- got {:?}",
                probe.raw
            );
        }
        for (b, (base, members)) in probe.vis.iter().enumerate() {
            assert_eq!(
                members.len() as u32, probe.inst[b],
                "`{path}` batch {b}: the region length must equal the record's `instanceCount`, \
                 which is what the rasterizer fetches -- a longer region means the rasterizer will \
                 not read entries the cull wrote; a shorter one means it reads slots nobody wrote \
                 this frame (INVARIANT R2d-REGION-DEFINED) -- got {:?}",
                probe.raw
            );
            // Every batch is present (`batches == EXPECTED_BATCHES` above), so the gather's
            // `base_instance = running` prefix sum is CONTIGUOUS here: batch `b` owns
            // `[base, next_base)`, and the last one runs to the arrangement's instance total. (The
            // gaps the region printer guards against come from SKIPPED batches, and there are
            // none on this frame.)
            let end = probe.vis.get(b + 1).map_or(EXPECTED_INSTANCES as u32, |(next, _)| *next);
            let mut previous: Option<u32> = None;
            for (slot, &id) in members.iter().enumerate() {
                assert!(
                    (*base..end).contains(&id),
                    "`{path}` batch {b}: survivor slot {slot} holds {id}, outside this batch's own \
                     region [{base}, {end}). The list stores ORIGINAL GLOBAL ring indices \
                     (`vb_raster.vs.hlsl`'s INVARIANT R2d-EXPORT-IS-GLOBAL); a value outside the \
                     region means a compacted SLOT number, another batch's instance, or an \
                     unwritten slot's residue -- got {:?}",
                    probe.raw
                );
                assert!(
                    id >= *base + slot as u32,
                    "`{path}` batch {b}: survivor slot {slot} holds {id}, below `base + slot`. \
                     Compaction preserves the ring's order and only SKIPS, so the id at slot `s` \
                     is at least `base + s` -- got {:?}",
                    probe.raw
                );
                if let Some(p) = previous {
                    assert!(
                        id > p,
                        "`{path}` batch {b}: survivor ids must be strictly ascending ({id} after \
                         {p}); a repeat means the compaction cursor did not advance and one \
                         instance is drawn twice while another is dropped -- got {:?}",
                        probe.raw
                    );
                }
                previous = Some(id);
            }
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

    // Every committed camera path must have a MEASURED rejection count, and every measured count
    // must name a committed path. Without both directions a path added to `PATHS` would be gated
    // against `expected_drawn`'s panic only at GPU time — after a windowed boot — and a stale
    // entry here would keep asserting a number about a path that no longer exists.
    assert_eq!(
        CENSUS_REJECTIONS.len(),
        vg_corpus_scene::PATHS.len(),
        "the corpus commits {} camera paths but {} have measured rejection counts; re-run \
         `vg_cull_granularity_census.rs` and copy its `PINNED` line's per-instance numbers in",
        vg_corpus_scene::PATHS.len(),
        CENSUS_REJECTIONS.len()
    );
    for p in &vg_corpus_scene::PATHS {
        let drawn = expected_drawn(p.id);
        assert!(
            drawn > 0 && drawn < EXPECTED_INSTANCES,
            "`{}`: the measured expectation is {drawn} of {EXPECTED_INSTANCES} drawn. 0 and \
             {EXPECTED_INSTANCES} are the two answers a broken predicate gives (reject everything \
             / reject nothing), and either would let the GPU gate pass on a cull that decides \
             nothing",
            p.id
        );
    }
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
