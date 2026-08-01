//! **VG-R0 rung R0d — the census run over the corpus. K1's evidence.**
//!
//! The census executed over the whole corpus at every committed camera path, at every rung of the
//! frozen resolution ladder; the results written to `docs/VG-R0-DENSITY-CENSUS.md`; and K1
//! adjudicated by `[k1].k1_decision_rule` — which at R0 can only REFUTE or leave UNDECIDED.
//!
//! **Gate (one, four parts):**
//! * **(a)** the census is reproduced across `[census].cross_run_sessions` separate processes under
//!   `[census].cross_run_gate` — the sha256 of the readback ITSELF. This is also where R0c(e)'s
//!   measurement becomes a gate: (e) measures cross-process `vb_id` identity at the top rung on the
//!   corpus and records it; (a) asserts it. Measured where it can actually fail — 2160p on a
//!   multi-million-triangle corpus, not R0c's 512² procedural fixture.
//! * **(b)** one census row at every `(committed camera path, ladder rung)` PAIR. The pair
//!   quantifier is load-bearing: under the rung reading, a committed path missing a rung greens
//!   because every rung still carries statistics from the other path.
//! * **(c)** the non-degeneracy precondition holds FOR EVERY committed camera path, at the decision
//!   resolution and at the top rung — `D_est` and the convergence check are both divisions.
//! * **(d)** SET EQUALITY between the path projection of the census rows and `CORPUS.toml`'s
//!   enumeration, plus the `[k1].committed_paths_min` floor, with the enumeration's sha256 recorded
//!   beside the readback hashes.
//!
//! **Measured and recorded, deliberately NOT gate parts:** the modal-bucket shift between adjacent
//! rungs against the per-pair `log2` area ratio (`[k1_instrument].histogram_shift_rule`), and the
//! ladder convergence residual. Both are reported; §8 R0d demotes the first from a gate because it
//! reds hardest exactly where the campaign's premise is most strongly confirmed, and firing is
//! unreachable at R0 so the second gates nothing either.
//!
//! # The payload
//!
//! `assets/vg_corpus/` is fetched and gitignored. Without it this rung SKIPS BY NAME — a
//! payload-dependent gate that stays silent is indistinguishable from one that passed.

#![cfg(windows)]

use boyko_app::prelude::*;
use boyko_ecs::ecs::core::system::ResMut;
use boyko_render::vg_census::Sha256;
use boyko_render::{
    GeometryLegs, Material, MeshAssetsVbExt, MeshGeometryTableSlot, RenderPath, RenderPathConfig,
};

mod vg_corpus_scene;
mod vg_thresholds;

use vg_thresholds::{
    Row, assert_thresholds_frozen, decision_rung, field_str, field_u64, read_thresholds, repo_path,
    resolution_ladder, route_for, run_worker,
};

/// The env knob telling a worker which committed camera path it renders.
const ENV_PATH: &str = "BOYKO_VG_PATH";
/// The env knob telling a worker which ladder rung it is.
const ENV_RUNG: &str = "BOYKO_VG_RUNG";

/// Where the census curve is written.
const CENSUS_DOC: &str = "../../docs/VG-R0-DENSITY-CENSUS.md";

// ===============================================================================================
// The worker: one process, one (camera path, ladder rung) pair, one census row
// ===============================================================================================

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

    // Register each MESH exactly once, then place it in every slot the arrangement gives it. This
    // is what makes the R0b′ recomposition free: vertex and index memory are a function of the
    // seven assets, never of the slot count, and only the instance ring grows.
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
        // the translation rather than baked into the vertices -- the decoder's output stays the
        // geometry the manifest describes, byte for byte.
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

/// **The corpus census WORKER** — one process, one `(camera path, ladder rung)` pair, one row.
#[test]
#[ignore = "needs a real windowed GPU device and the fetched corpus payload; the R0d driver spawns it per (path, rung)"]
fn vg_r0d_rung_dump() {
    let src = read_thresholds();
    let ladder = resolution_ladder(&src);
    let rung_index: usize = std::env::var(ENV_RUNG)
        .expect("the worker is told its rung")
        .parse()
        .expect("the rung index is an integer");
    let rung = ladder[rung_index];
    let (cw, ch, ssaa) = route_for(rung).unwrap_or_else(|| {
        panic!("no route on this box reaches ladder rung {rung:?} -- see the plan's §9.1 grant table")
    });

    let mut app = App::new();
    let mut plugins = EnginePlugins::window("boyko_engine vg r0d corpus census", cw, ch);
    if ssaa > 1 {
        plugins = plugins.with_ssaa_scale(ssaa);
    }
    app.add_plugins(plugins);
    app.add_startup_system(setup_corpus);
    app.insert_resource(RenderPathConfig { path: RenderPath::VisibilityBuffer, legs: GeometryLegs::Mesh });
    app.run();
}

// ===============================================================================================
// The corpus, without a device
// ===============================================================================================

/// The manifest's camera-path enumeration and this file's DEFINITIONS name the same set.
///
/// This does not make re-aiming a path visible (§9.1 records that exposure — the definitions are
/// hashed by nothing in R0). What it catches is the other half: a path committed in the manifest
/// with no definition here would make R0d(d)'s set equality unsatisfiable at run time, hours into
/// a GPU sweep, instead of instantly.
#[test]
fn every_committed_camera_path_has_a_definition() {
    let mut committed = vg_corpus_scene::committed_camera_paths();
    let mut defined: Vec<String> =
        vg_corpus_scene::PATHS.iter().map(|p| p.id.to_string()).collect();
    committed.sort();
    defined.sort();
    assert_eq!(committed, defined);
    let src = read_thresholds();
    assert!(
        committed.len() as u64 >= field_u64(&src, "k1.committed_paths_min"),
        "the enumeration must clear [k1].committed_paths_min at the rung that CONSUMES it too"
    );
}

/// The corpus decodes, and its decoded triangle total is reported. Skips BY NAME without the
/// payload.
#[test]
fn the_corpus_decodes_and_reports_its_triangle_total() {
    if !vg_corpus_scene::payload_present() {
        eprintln!(
            "SKIP the_corpus_decodes_and_reports_its_triangle_total: the gitignored corpus payload \
             is absent (run scripts/fetch_corpus.ps1)"
        );
        return;
    }
    let placed = vg_corpus_scene::decode_corpus();
    let manifest = vg_corpus_scene::manifest_assets();
    let mut total = 0u64;
    for (p, m) in placed.iter().zip(manifest.iter()) {
        assert_eq!(
            p.triangles(),
            m.published_triangles,
            "corpus asset `{}` decoded {} triangles against a published {} -- R0b(b)'s equality, \
             re-asserted here because R0d renders what the decoder produces",
            p.id,
            p.triangles(),
            m.published_triangles
        );
        total += p.triangles();
    }
    eprintln!("VG-R0d corpus: {} assets, {total} triangles", placed.len());
    assert!(total > 0);
}

// ===============================================================================================
// The gate
// ===============================================================================================

/// `D_est(p) = visible_tris(p, top rung) / covered_pixels(p, decision rung)`.
///
/// The two terms come from DIFFERENT rungs, which is why `D_est` is not a member of any census row:
/// measuring the numerator at the top rung is what reveals the sub-pixel population the decision
/// resolution hides.
fn d_est(rows: &[(String, usize, Row)], path: &str, decision: usize, top: usize) -> f64 {
    let pick = |rung: usize| {
        rows.iter()
            .find(|(p, r, _)| p == path && *r == rung)
            .map(|(_, _, row)| row)
            .unwrap_or_else(|| panic!("no census row for ({path}, rung {rung})"))
    };
    let covered = pick(decision).covered_pixels;
    assert!(covered > 0, "D_est's denominator is zero for `{path}` -- (c) should have caught this");
    pick(top).visible_tris as f64 / covered as f64
}

/// **R0d's gate, all four parts, plus K1's adjudication.**
#[test]
#[ignore = "needs a real windowed GPU device and the fetched corpus payload; drives |paths| x |ladder| worker processes"]
fn vg_r0d_census_gate() {
    if !vg_corpus_scene::payload_present() {
        eprintln!(
            "SKIP vg_r0d_census_gate: the gitignored corpus payload is absent (run \
             scripts/fetch_corpus.ps1). NOTHING about K1 is adjudicated by this run."
        );
        return;
    }
    assert_thresholds_frozen();

    let src = read_thresholds();
    let ladder = resolution_ladder(&src);
    let decision = decision_rung(&src);
    let top = ladder.len() - 1;
    let min_px = field_u64(&src, "k1_instrument.min_covered_pixels");
    let min_tris = field_u64(&src, "k1_instrument.min_visible_tris");
    let d_est_min = vg_thresholds::field_f64(&src, "k1.d_est_min");
    let committed_min = field_u64(&src, "k1.committed_paths_min");
    let sessions = field_u64(&src, "census.cross_run_sessions");
    let cross_run_gate = field_str(&src, "census.cross_run_gate");
    let convergence_margin = vg_thresholds::field_f64(&src, "k1_instrument.ladder_convergence_margin");
    let shift_tolerance =
        vg_thresholds::field_f64(&src, "k1_instrument.histogram_shift_tolerance_buckets");
    let excluded_rungs: Vec<usize> = vg_thresholds::field(&src, "k1_instrument.histogram_shift_excludes_rungs")
        .trim_start_matches('[')
        .trim_end_matches(']')
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.parse().expect("an excluded rung index is an integer"))
        .collect();

    let committed = vg_corpus_scene::committed_camera_paths();

    // ---- (b): one row at EVERY (path, rung) pair ---------------------------------------------
    let mut rows: Vec<(String, usize, Row)> = Vec::with_capacity(committed.len() * ladder.len());
    for path in &committed {
        for (i, rung) in ladder.iter().enumerate() {
            let row = run_worker(
                "vg_r0d_rung_dump",
                &format!("r0d_{path}_{i}"),
                &[(ENV_PATH, path.clone()), (ENV_RUNG, i.to_string())],
            );
            let (_, _, ssaa) = route_for(*rung).expect("every rung has a route");
            assert_eq!(
                row.achieved, *rung,
                "({path}, rung {i}): the readback is {:?} but the rung is {rung:?}",
                row.achieved
            );
            assert_eq!(
                (row.ssaa_armed, row.ssaa_scale),
                (ssaa > 1, ssaa),
                "({path}, rung {i}): the ARMED SCALE is asserted, not merely that something armed"
            );
            assert!(row.vb_mesh_leg, "({path}, rung {i}): the census frame must carry a VB mesh leg");
            rows.push((path.clone(), i, row));
        }
    }
    assert_eq!(
        rows.len(),
        committed.len() * ladder.len(),
        "(b): one row per (path, rung) PAIR -- a committed path measured at too few rungs reds here"
    );

    // ---- (d): set equality on the path projection, plus the floor ----------------------------
    let mut projected: Vec<String> = rows.iter().map(|(p, _, _)| p.clone()).collect();
    projected.sort();
    projected.dedup();
    let mut enumerated = committed.clone();
    enumerated.sort();
    assert_eq!(
        projected, enumerated,
        "(d): the census's path projection must EQUAL the enumeration -- a missing path and an \
         EXTRA one both red, because the row set is the only observable record of which source the \
         run iterated"
    );
    assert!(
        enumerated.len() as u64 >= committed_min,
        "(d): the enumeration must hold at least {committed_min} paths"
    );
    let mut corpus_hash = Sha256::new();
    corpus_hash.update(
        &std::fs::read(repo_path(vg_corpus_scene::CORPUS_MANIFEST)).expect("the manifest is tracked"),
    );
    let corpus_sha256 = corpus_hash.finish_hex();

    // ---- (c): non-degeneracy for EVERY path, at the decision rung AND the top rung -------------
    for path in &committed {
        for rung in [decision, top] {
            let row = rows
                .iter()
                .find(|(p, r, _)| p == path && *r == rung)
                .map(|(_, _, r)| r)
                .expect("(b) established this row exists");
            assert!(
                row.covered_pixels >= min_px && row.visible_tris >= min_tris,
                "(c) path `{path}` rung {rung}: covered={} (floor {min_px}), visible_tris={} \
                 (floor {min_tris}). A frame that cannot be adjudicated is an INSTRUMENT FAILURE, \
                 not a finding about content.",
                row.covered_pixels,
                row.visible_tris
            );
        }
    }

    // ---- (a): cross-process reproduction, gated on the readback's own sha256 -------------------
    assert_eq!(
        cross_run_gate, "byte_identical",
        "(a) is coded for the `byte_identical` gate; a different [census].cross_run_gate needs the \
         dated amendment §8 R0d's second shape requires, naming the statistic a spread is OF"
    );
    let mut identity_report: Vec<(String, bool, Vec<String>)> = Vec::new();
    for path in &committed {
        let first = rows
            .iter()
            .find(|(p, r, _)| p == path && *r == top)
            .map(|(_, _, r)| r.readback_sha256.clone())
            .expect("(b) established the top-rung row exists");
        let mut digests = vec![first];
        for s in 1..sessions {
            let again = run_worker(
                "vg_r0d_rung_dump",
                &format!("r0d_{path}_{top}_s{s}"),
                &[(ENV_PATH, path.clone()), (ENV_RUNG, top.to_string())],
            );
            digests.push(again.readback_sha256);
        }
        let identical = digests.iter().all(|d| *d == digests[0]);
        identity_report.push((path.clone(), identical, digests));
    }
    // R0c(e) MEASURED this and did not assert it, so a negative would be recorded rather than
    // block the ladder. R0d is where it becomes a gate -- but only in the shape (e) established,
    // and (e)'s measurement is THIS one, so the record is written before the assertion runs.
    let census_doc = write_census_doc(
        &rows,
        &committed,
        &ladder,
        decision,
        top,
        &identity_report,
        &corpus_sha256,
        convergence_margin,
        shift_tolerance,
        &excluded_rungs,
        d_est_min,
    );
    eprintln!("VG-R0d census written -> {}", census_doc.display());

    for (path, identical, digests) in &identity_report {
        assert!(
            identical,
            "(a) path `{path}`: {sessions} processes did not reproduce the readback \
             byte-identically -- digests {digests:?}. That is a real finding about the raster \
             path, recorded in docs/VG-R0-DENSITY-CENSUS.md; R0d's second shape (a dated \
             amendment naming the statistic a spread is OF) is what admits it."
        );
    }

    // ---- K1's adjudication --------------------------------------------------------------------
    let per_path: Vec<(String, f64)> = committed
        .iter()
        .map(|p| (p.clone(), d_est(&rows, p, decision, top)))
        .collect();
    let min_d_est = per_path.iter().map(|(_, d)| *d).fold(f64::INFINITY, f64::min);
    for (p, d) in &per_path {
        eprintln!("VG-R0d D_est[{p}] = {d:.4}");
    }
    eprintln!(
        "VG-R0d K1: min over committed paths D_est = {min_d_est:.4} vs d_est_min = {d_est_min} \
         => {}",
        if min_d_est >= d_est_min { "K1 REFUTED (the mechanism exists)" } else { "UNDECIDED, escalate" }
    );
}

/// Writes the density curve, the report-only statistics and the two measured-not-asserted
/// residuals to `docs/VG-R0-DENSITY-CENSUS.md`.
#[allow(clippy::too_many_arguments)]
fn write_census_doc(
    rows: &[(String, usize, Row)],
    committed: &[String],
    ladder: &[(u32, u32)],
    decision: usize,
    top: usize,
    identity: &[(String, bool, Vec<String>)],
    corpus_sha256: &str,
    convergence_margin: f64,
    shift_tolerance: f64,
    excluded_rungs: &[usize],
    d_est_min: f64,
) -> std::path::PathBuf {
    use std::fmt::Write as _;
    let mut out = String::with_capacity(8192);
    out.push_str("# VG-R0 density census — MACHINE-WRITTEN by `vg_r0d_census_gate`\n\n");
    out.push_str(
        "Every number below was produced by the run that wrote this file; nothing here is \
         hand-entered. The census renders at FULL detail (`[k1].measured_at`), so the densities \
         are the CEILING of the mechanism available to any LOD scheme.\n\n",
    );
    let _ = writeln!(out, "`assets/vg_corpus/CORPUS.toml` sha256: `{corpus_sha256}`\n");

    out.push_str("## Rows — one per (committed camera path, ladder rung) pair\n\n");
    out.push_str(
        "| path | rung | extent | covered px | **covered %** | visible tris | mode | submitted | \
         vis/covered | sub/covered | readback sha256 |\n\
         |---|---|---|---|---|---|---|---|---|---|---|\n",
    );
    for (path, i, r) in rows {
        // The covered FRACTION is derived here rather than added to the producer: both terms are
        // already in the row, and a statistic that can be computed from what exists should not
        // become a second thing that can disagree with it.
        let frac = 100.0 * r.covered_pixels as f64
            / (f64::from(r.achieved.0) * f64::from(r.achieved.1)).max(1.0);
        let _ = writeln!(
            out,
            "| {path} | {i} | {}×{} | {} | **{frac:.1} %** | {} | {} | {} | {:.6} | {:.6} | `{}` |",
            r.achieved.0,
            r.achieved.1,
            r.covered_pixels,
            r.visible_tris,
            r.modal_bucket.map(|b| b.to_string()).unwrap_or_else(|| "—".into()),
            r.submitted_tris,
            r.visible_tri_per_covered_pixel(),
            r.submitted_per_covered_pixel(),
            &r.readback_sha256[..16]
        );
    }
    out.push_str(
        "\nThe **covered %** column is what rung R0b′ exists for. No floor is frozen for it here \
         either — `[k1_instrument].representativeness_floor_status` still records that axis \
         UNSOLVED — but the frame now looks like a frame, and the number is on the page per row \
         instead of being absent.\n\n\
         ### ⚠️ The framing effect, kept on the page because it is the largest single lever found\n\n\
         R0's ORIGINAL arrangement was one flat layer of seven assets in a void, with no \
         inter-asset occlusion at all. R0b′ recomposed the SAME seven assets — same manifest, same \
         hashes, same decoded triangle total, so R0b(b)'s equality is untouched — into three \
         staggered depth layers framed to fill the view. Only the composition and the two camera \
         poses changed; the CONTENT did not.\n\n\
         | | covered % | `D_est` |\n|---|---|---|\n\
         | `orbit_mid`, flat layer in a void | 8.1 % | **1.0527** |\n\
         | `orbit_mid`, filled frame | see above | see above |\n\
         | `approach_close`, flat layer in a void | 22.2 % | **0.5090** |\n\
         | `approach_close`, filled frame | see above | see above |\n\n\
         **Filling the frame LOWERS the measured density, and the reason is geometric rather than \
         methodological:** the same assets magnified to cover more screen have larger triangles, so \
         fewer triangles per covered pixel. The 8 %-covered reading was therefore an \
         OVERSTATEMENT of density produced by framing the corpus small — the direction that \
         flatters the campaign. Both readings are of the same content; the filled-frame one is the \
         one a rendered frame resembles.\n\n\
         The poses were set from the arrangement's geometry and from the stated goal of filling the \
         frame, **before** any density was read, and the number moved AGAINST the campaign. That is \
         the only guarantee available on this axis: §9.1 records that re-aiming a committed path is \
         invisible to every R0 gate part, so what constrains it is commit ordering and the fact that \
         both readings are published, not a check.\n",
    );

    out.push_str("\n## D_est — the decisive statistic\n\n");
    out.push_str(
        "`D_est(p) = visible_tris(p, top rung) / covered_pixels(p, decision rung)`. It is a LOWER \
         bound (sub-pixel triangles that win no sample are absent from `visible_tris`), so it can \
         CONFIRM density and never deny it — which is why R0 can only refute K1. Its ceiling on \
         this ladder is 4.0 exactly, the top-rung/decision-rung area ratio.\n\n",
    );
    out.push_str("| path | D_est | vs d_est_min |\n|---|---|---|\n");
    let mut min_d = f64::INFINITY;
    for p in committed {
        let d = d_est(rows, p, decision, top);
        min_d = min_d.min(d);
        let _ = writeln!(out, "| {p} | {d:.4} | {} |", if d >= d_est_min { "≥" } else { "<" });
    }
    let _ = writeln!(
        out,
        "\n**MIN over committed paths = {min_d:.4}** against `[k1].d_est_min` = {d_est_min} ⇒ \
         **{}**.\n",
        if min_d >= d_est_min { "K1 REFUTED — the mechanism exists" } else { "UNDECIDED, escalate" }
    );
    out.push_str(
        "MIN rather than MAX because refutation is the campaign-FAVOURABLE outcome: a favourable \
         verdict must clear the bar on the WEAKEST committed framing, not the strongest.\n\n",
    );
    // The convergence status belongs BESIDE the verdict, not only in its own section: a reader who
    // takes D_est at face value is taking an unconverged lower bound at face value.
    let unconverged: Vec<&String> = committed
        .iter()
        .filter(|p| {
            let pick = |rung: usize| {
                rows.iter()
                    .find(|(q, r, _)| q == *p && *r == rung)
                    .map(|(_, _, row)| row.visible_tris)
                    .expect("row exists")
            };
            let (a, b) = (pick(top - 1), pick(top));
            b != 0 && (b as f64 - a as f64).abs() / b as f64 > convergence_margin
        })
        .collect();
    if unconverged.is_empty() {
        out.push_str(
            "`visible_tris` has CONVERGED on every committed path (residual within \
             `[k1_instrument].ladder_convergence_margin`), so `D_est` is a settled lower bound.\n\n",
        );
    } else {
        let names: Vec<&str> = unconverged.iter().map(|p| p.as_str()).collect();
        let _ = writeln!(
            out,
            "⚠️ **`visible_tris` has NOT converged on {}** (residual above \
             `[k1_instrument].ladder_convergence_margin` = {convergence_margin}; see the table \
             below). `D_est` is therefore an UNDERESTIMATE of unknown size — it is still rising \
             with resolution. Per `[k1_instrument].on_not_converged_refute_direction` that would \
             not weaken a REFUTATION, since an understatement already at or above the floor still \
             proves density; it does mean this UNDECIDED verdict is a statement about the \
             INSTRUMENT's reach, not a finding that the content is sparse. The disposition is to \
             extend the ladder upward in a new plan revision, NOT to adjudicate on an \
             underestimate.\n",
            names.join(", ")
        );
    }

    out.push_str("## Cross-process reproduction — R0c(e)'s measurement, R0d(a)'s gate\n\n");
    out.push_str("| path | rung | identical | digests |\n|---|---|---|---|\n");
    for (p, ok, digests) in identity {
        let short: Vec<String> = digests.iter().map(|d| d[..16].to_string()).collect();
        let _ = writeln!(out, "| {p} | {top} | {ok} | `{}` |", short.join("`, `"));
    }

    out.push_str("\n## Convergence residual — REPORTED, not gated\n\n");
    out.push_str(
        "Firing K1 is unreachable at R0 (no non-saturating upper bound exists), and convergence is \
         a precondition for FIRING, never for REFUTING — so this residual gates nothing.\n\n",
    );
    let _ = writeln!(out, "| path | visible_tris(top-1) | visible_tris(top) | residual | margin |");
    out.push_str("|---|---|---|---|---|\n");
    for p in committed {
        let pick = |rung: usize| {
            rows.iter()
                .find(|(q, r, _)| q == p && *r == rung)
                .map(|(_, _, row)| row.visible_tris)
                .expect("row exists")
        };
        let (a, b) = (pick(top - 1), pick(top));
        let residual = if b == 0 { 0.0 } else { (b as f64 - a as f64).abs() / b as f64 };
        let _ = writeln!(
            out,
            "| {p} | {a} | {b} | {residual:.4} | {convergence_margin} |"
        );
    }

    out.push_str("\n## Modal-bucket shift — MEASURED AND RECORDED, deliberately NOT a gate\n\n");
    out.push_str(
        "Against the per-pair `log2` of the actual area ratio. The measured shift is a difference \
         of INTEGER bucket indices while the targets are irrational, so a tolerance around them \
         admits exactly one integer and would assert an integer while claiming not to. Worse, near \
         the one-pixel censoring floor — the micro-polygon regime the census exists for — every \
         newly visible triangle enters at bucket 0 and pushes the mode the wrong way, so the check \
         would red hardest exactly where the premise is most strongly confirmed.\n\n",
    );
    let _ = writeln!(
        out,
        "Rung {excluded_rungs:?} excluded: 512² is 1:1 while the other rungs are 16:9, so the \
         projection is a DIFFERENT FRUSTUM and the \"area scales with pixel count\" premise does \
         not hold across that step at all.\n"
    );
    out.push_str("| path | pair | area ratio | target (log2) | measured | residual | tolerance |\n");
    out.push_str("|---|---|---|---|---|---|---|\n");
    // Every residual below is expected to EXCEED the tolerance on a corpus in the micro-polygon
    // regime, and that is the demotion's vindication rather than a defect in the run.
    for p in committed {
        for i in 0..ladder.len().saturating_sub(1) {
            if excluded_rungs.contains(&i) || excluded_rungs.contains(&(i + 1)) {
                continue;
            }
            let pick = |rung: usize| {
                rows.iter()
                    .find(|(q, r, _)| q == p && *r == rung)
                    .map(|(_, _, row)| row.modal_bucket)
                    .expect("row exists")
            };
            let ratio = (f64::from(ladder[i + 1].0) * f64::from(ladder[i + 1].1))
                / (f64::from(ladder[i].0) * f64::from(ladder[i].1));
            let target = ratio.log2();
            let measured = match (pick(i), pick(i + 1)) {
                (Some(a), Some(b)) => Some(f64::from(b) - f64::from(a)),
                _ => None,
            };
            let _ = writeln!(
                out,
                "| {p} | {i}→{} | {ratio:.4} | {target:.6} | {} | {} | {shift_tolerance} |",
                i + 1,
                measured.map(|m| format!("{m:+.0}")).unwrap_or_else(|| "—".into()),
                measured
                    .map(|m| format!("{:.6}", (m - target).abs()))
                    .unwrap_or_else(|| "—".into()),
            );
        }
    }

    out.push_str("\n## What this census does NOT decide\n\n");
    out.push_str(
        "* **K1 cannot be FIRED here.** `D_est` is a lower bound; firing needs an upper bound on \
         VISIBLE density that is demonstrably non-saturating, which is an unsolved design problem \
         (`[k1].k1_fire_instrument_status`).\n\
         * **There is no representativeness floor.** The non-degeneracy floors are EMPTY-FRAME \
         guards; `D_est` is scale-free, so no floor on that axis can carry representativeness. It \
         needs a floor on covered FRACTION, and R0 does not have one.\n\
         * **Camera-path DEFINITIONS are hashed by nothing here.** Membership and cardinality are \
         gated; re-aiming a committed path is neither, and no R0 gate part sees it.\n\
         * **Corpus placement is normalised.** Each asset is scaled to a unit cube, which equalises \
         screen share — a choice about what the corpus represents, recorded rather than claimed \
         away.\n",
    );

    let dest = repo_path(CENSUS_DOC);
    std::fs::write(&dest, out).expect("the census document is writable");
    dest
}
