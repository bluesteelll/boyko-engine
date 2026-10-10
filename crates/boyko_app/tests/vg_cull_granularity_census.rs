//! **VG cull granularity — the per-INSTANCE vs per-BATCH rejection counts, MEASURED on the corpus.**
//!
//! The virtual-geometry campaign shipped a per-BATCH frustum cull (rung R2c). Moving to per-INSTANCE
//! granularity is justified by one pair of numbers per committed camera path: how many of the
//! arrangement's [`vg_corpus_scene::SLOT_COUNT`] instances the frustum rejects, against how many of
//! the seven per-mesh batches it rejects. Until now those numbers existed only as HAND
//! COMPUTATIONS, done twice:
//!
//! | camera path | per-INSTANCE rejects | per-BATCH rejects |
//! |---|---|---|
//! | `orbit_mid` | 0 of 45 (SOFT) | 0 of 7 |
//! | `approach_close` | 9 of 45 | 0 of 7 |
//!
//! **Both hand computations were LOWER BOUNDS, and this file supersedes them.** They used the
//! worst-case half-extent `0.5` that normalisation guarantees ([`vg_corpus_scene::NORMALISED_SIZE`]
//! scales each asset's LARGEST extent to one unit, so the other two axes are smaller) instead of the
//! decoded per-asset bounds. A smaller true extent gives a smaller projected radius in
//! [`aabb_outside_frustum`](boyko_render::frustum::aabb_outside_frustum)'s `dist + radius < 0` test
//! and therefore MORE rejections — so the real counts can only be greater or equal, never less. The
//! `orbit_mid` zero was additionally SOFT: three slots cleared their plane by under ~0.1 against a
//! radius of ~0.72. Nothing below is predicted; everything below is measured from the decoded
//! payload and then pinned.
//!
//! # The frustum is the PRODUCTION one, taken from the PUSH BYTES
//!
//! The planes are not re-derived here. They are extracted from the first 64 bytes of the raster
//! vertex push — the same bytes the VB raster's VS reads as `pc.view_proj` — by the same call the
//! armed cull makes (`crates/boyko_app/src/gpu_scene/mod.rs:6071`). The route, end to end:
//!
//! 1. `Affine3A::look_at_rh(eye, target, +Y)` → a camera WORLD pose (`crates/boyko_math/src/affine.rs:69`),
//!    folded into the `Transform` the corpus spawns (`crates/boyko_app/tests/vg_r0d_census.rs:123-142`);
//! 2. `Transform::to_affine()` — a ROOT entity's `GlobalTransform` is exactly this
//!    (`crates/boyko_scene/src/transform.rs:106`);
//! 3. `ViewUniform::from_camera(global, Projection::Perspective { .. })`
//!    (`crates/boyko_scene/src/camera.rs:355`), with the census's own `near = 0.05` / `far = 200.0`;
//! 4. `forward_gbuffer_push_from_view(&view, w, h, true)` (`crates/boyko_render/src/view.rs:605`);
//! 5. `frustum_planes_from_push_bytes(&push[0..64])` (`crates/boyko_render/src/frustum.rs:84`).
//!
//! **Step 4 is the VB path's push, not Deferred's.** `crates/boyko_app/src/runner.rs:2065-2079`
//! selects the Forward-family arm for `RenderPath::VisibilityBuffer`, so the VB raster is drawn with
//! `forward_view_proj_rows`' REVERSE-Z projection — not `gbuffer_push_from_view`'s marcher-aligned
//! `clip.z == clip.w` matrix. Hand-deriving a near plane the OpenGL way against reverse-Z silently
//! rejects geometry IN FRONT of the camera (`crates/boyko_render/src/frustum.rs:52-54`), which is
//! why no matrix is built here at all. The push is taken UNJITTERED: `runner.rs:2071` jitters only
//! when `taa_armed_now`, which needs `ResolvedAa::mode == AaMode::Taa` (`runner.rs:1479-1485`), and
//! the corpus census worker inserts no `AaConfig` — `AaMode::Off` is the default
//! (`crates/boyko_render/src/aa_config.rs:67`).
//!
//! # The predicate is the shipped host oracle, at two granularities
//!
//! Both counts run [`batch_instance_count_after_cull`] (`crates/boyko_render/src/frustum.rs:135`) —
//! mesh-local AABB → [`batch_world_aabb`]'s Arvo fold over the instance ring → `aabb_outside_frustum`.
//! The ONLY difference between the two measurements is the batching: per-INSTANCE asks it about a
//! one-instance batch, per-BATCH about the whole per-mesh bucket. Folding the union by hand would
//! have made the comparison partly about two different AABB computations instead of purely about
//! granularity.
//!
//! # The payload
//!
//! `assets/vg_corpus/` is fetched and gitignored. Without it this test SKIPS BY NAME — a
//! payload-dependent gate that stays silent is indistinguishable from one that passed
//! (`crates/boyko_app/tests/vg_corpus_scene/mod.rs:286-290`).

use boyko_math::{Affine3A, Quat, Vec3};
use boyko_render::csm_caster::batch_world_aabb;
use boyko_render::frustum::{
    FRUSTUM_PLANE_COUNT, Plane, batch_instance_count_after_cull, frustum_planes_from_push_bytes,
};
use boyko_render::instance_model::InstanceModelCol;
use boyko_render::mesh_draw::DrawBatch;
use boyko_rhi::IndexType;
use boyko_scene::{GlobalTransform, Projection, Transform, ViewUniform};

mod vg_corpus_scene;
mod vg_thresholds;

use vg_corpus_scene::{CameraPath, PATHS, PlacedAsset, SLOT_COUNT};
use vg_thresholds::{decision_rung, read_thresholds, resolution_ladder, route_for};

/// The committed camera paths this file measures, as a length the pin array must match.
const PATH_COUNT: usize = PATHS.len();

/// The ladder rung the pinned counts are measured at: rung 0, the only 1:1 rung
/// (`docs/VG-CAMPAIGN-THRESHOLDS.toml` `[census].resolution_ladder`). The extent is read from that
/// frozen ladder rather than hardcoded, because the frustum's aspect is `width / height` — the raster
/// push derives it from the EXTENT, never from the authored `Projection::aspect`
/// (`crates/boyko_render/src/view.rs:519-529`) — so an extent invented here would measure a
/// different frustum than the census renders.
const MEASURED_RUNG: usize = 0;

/// The extent [`MEASURED_RUNG`] must resolve to. Asserted rather than assumed: every other ladder
/// rung is 16:9, and a 16:9 frustum is a WIDER horizontal field that rejects fewer instances, so a
/// ladder edit that moved rung 0 would silently re-measure the pins against a different frustum.
const MEASURED_EXTENT: (u32, u32) = (512, 512);

/// The slot count the pinned counts are out of. The pins are counts, not fractions, so a
/// re-arranged grid must red here with its own message instead of at four raw equalities.
const MEASURED_SLOT_COUNT: usize = 45;

/// The batch count the pinned per-BATCH counts are out of: one batch per registered mesh, and
/// `slot_asset` cycles the manifest's assets over the slots. A manifest that gains an asset
/// re-deals every slot, so it must red here rather than at the four equalities.
const MEASURED_BATCH_COUNT: usize = 7;

/// A count that has not been MEASURED yet. `usize::MAX` cannot be mistaken for a plausible
/// rejection count (there are 45 slots and 7 batches) and cannot satisfy any equality below, so an
/// unfilled pin fails loudly rather than asserting something convenient.
const UNPINNED: usize = usize::MAX;

/// **PLACEHOLDERS — TO BE FILLED FROM THE FIRST RUN.**
///
/// One row per committed camera path, in [`PATHS`] order: `(per-INSTANCE rejects, per-BATCH
/// rejects)`. The first run prints the measured table AND a copy-pasteable `PINNED = [...]` line;
/// those values go here, and from then on this array is the regression guard on the rung's premise.
///
/// They are deliberately NOT pre-filled with the hand-computed `(0, 0)` / `(9, 0)`: predicting a
/// number and then confirming it is a failure mode this campaign has already paid for. The numbers
/// are measured first, pinned second.
/// MEASURED on the first run and pinned second, never predicted. In `PATHS` order:
/// `orbit_mid` rejects 1 instance and 0 batches; `approach_close` rejects 14 and 0.
///
/// The hand computations these supersede said 0 and 9. Both were LOWER BOUNDS: neither could read
/// the decoded per-asset extents, so both used the worst-case half-extent `0.5` normalisation
/// guarantees, and a smaller true extent means a smaller projected radius and therefore MORE
/// rejections. The measured numbers moved in exactly that direction.
const PINNED: [(usize, usize); PATH_COUNT] = [(1, 0), (14, 0)];

/// One camera path's rejection counts at one extent.
struct Census {
    /// Instances whose OWN world AABB is wholly outside the frustum.
    per_instance: usize,
    /// Per-mesh batches whose UNION AABB is wholly outside the frustum.
    per_batch: usize,
    /// **The comparison unit.** Instances the per-BATCH cull actually removes — the sum of
    /// `instance_count` over the rejected batches, which is the datum production loses when the
    /// GPU cull zeroes a batch's `instanceCount`.
    ///
    /// `per_batch` counts BATCHES and `per_instance` counts INSTANCES, so comparing the two
    /// directly is a units error that makes the premise nearly free: on this corpus one rejected
    /// batch carries 6 or 7 members, so `per_instance > per_batch` is satisfied by rejecting one
    /// batch and exactly its own members — the case where granularity bought NOTHING. Every
    /// relationship below is stated against this field instead.
    instances_removed_by_batch: usize,
    /// The arrangement slots behind `per_instance`, for the log.
    rejected_slots: Vec<usize>,
    /// The decoded-asset indices behind `per_batch`, for the log.
    rejected_batches: Vec<usize>,
}

/// The instance ring and the per-mesh batches the runtime builds for the corpus arrangement.
///
/// The ring is asset-MAJOR because that is what a batch is: one bucket per mesh handle, drawn as
/// `ring[base_instance .. base_instance + instance_count]`
/// (`crates/boyko_render/src/mesh_draw.rs:91-97`). Slot order inside a bucket is irrelevant to a
/// union, and is kept ascending so the log reads in arrangement order.
struct Arrangement {
    ring: Vec<InstanceModelCol>,
    batches: Vec<DrawBatch>,
    /// `ring[i]` fills arrangement slot `ring_slot[i]`.
    ring_slot: Vec<usize>,
    /// `ring[i]` draws decoded asset `ring_asset[i]`.
    ring_asset: Vec<usize>,
}

/// The six PRODUCTION frustum planes for one committed camera path at a `width × height` extent.
///
/// Every step is the engine's own, so this is definitionally the frustum the VB raster draws with —
/// see the module header for the route and its call sites.
fn production_frustum_planes(
    path: &CameraPath,
    width: u32,
    height: u32,
) -> [Plane; FRUSTUM_PLANE_COUNT] {
    let pose = Affine3A::look_at_rh(
        Vec3::new(path.eye[0], path.eye[1], path.eye[2]),
        Vec3::new(path.target[0], path.target[1], path.target[2]),
        Vec3::new(0.0, 1.0, 0.0),
    );
    let transform = Transform {
        translation: pose.translation,
        rotation: Quat::from_mat3(pose.matrix3),
        scale: Vec3::ONE,
    };
    let projection = Projection::Perspective {
        fov_y: path.fov_y_degrees * core::f32::consts::PI / 180.0,
        aspect: 1.0,
        near: 0.05,
        far: 200.0,
    };
    let view = ViewUniform::from_camera(transform.to_affine(), projection);
    // `instanced = true`: the corpus always submits a non-empty batch list. It selects the VS arm at
    // push byte 84 and cannot touch bytes 0..64.
    let push = boyko_render::view::forward_gbuffer_push_from_view(&view, width, height, true);
    frustum_planes_from_push_bytes(
        push[0..64].try_into().expect("invariant: the raster push's leading 64 bytes are view_proj"),
    )
}

/// The instance-model row for one arrangement slot, packed by the PRODUCTION packer
/// (`InstanceModelCol::from_global`) off the `Transform` `setup_corpus` spawns
/// (`crates/boyko_app/tests/vg_r0d_census.rs:94-104`).
fn instance_row(asset: &PlacedAsset, slot: usize) -> InstanceModelCol {
    let pos = vg_corpus_scene::slot_position(slot);
    let s = asset.scale;
    let transform = Transform {
        translation: Vec3::new(
            pos[0] - s * asset.centre[0],
            pos[1] - s * asset.centre[1],
            pos[2] - s * asset.centre[2],
        ),
        rotation: Quat::IDENTITY,
        scale: Vec3::new(s, s, s),
    };
    InstanceModelCol::from_global(&GlobalTransform(transform.to_affine()))
}

/// Builds the ring and the per-mesh batches for the whole arrangement.
fn build_arrangement(assets: &[PlacedAsset]) -> Arrangement {
    let mut ring = Vec::with_capacity(SLOT_COUNT);
    let mut ring_slot = Vec::with_capacity(SLOT_COUNT);
    let mut ring_asset = Vec::with_capacity(SLOT_COUNT);
    let mut batches = Vec::with_capacity(assets.len());
    for (ai, asset) in assets.iter().enumerate() {
        let base = ring.len() as u32;
        for slot in 0..SLOT_COUNT {
            if vg_corpus_scene::slot_asset(slot, assets.len()) == ai {
                ring.push(instance_row(asset, slot));
                ring_slot.push(slot);
                ring_asset.push(ai);
            }
        }
        batches.push(DrawBatch {
            mesh_id: ai as u32,
            // `index_count` / `index_type` are unread by the cull (it reads only the ring range and
            // returns `instance_count`); they are filled from the asset so the batch is the one the
            // runtime would build rather than a stripped-down stand-in.
            index_count: asset.indices.len() as u32,
            index_type: IndexType::Uint32,
            base_instance: base,
            instance_count: ring.len() as u32 - base,
        });
    }
    Arrangement { ring, batches, ring_slot, ring_asset }
}

/// The world AABB of one slot's instance, folded DIRECTLY: rotation is identity and scale is
/// uniform, so the world box is exactly `scale * local + translation`.
fn world_aabb_direct(
    local: ([f32; 3], [f32; 3]),
    asset: &PlacedAsset,
    slot: usize,
) -> ([f32; 3], [f32; 3]) {
    let (lo, hi) = local;
    let pos = vg_corpus_scene::slot_position(slot);
    let s = asset.scale;
    let t = [
        pos[0] - s * asset.centre[0],
        pos[1] - s * asset.centre[1],
        pos[2] - s * asset.centre[2],
    ];
    (
        [s * lo[0] + t[0], s * lo[1] + t[1], s * lo[2] + t[2]],
        [s * hi[0] + t[0], s * hi[1] + t[1], s * hi[2] + t[2]],
    )
}

/// Cross-checks the packed ring against the direct fold and returns the largest component
/// deviation, in world units.
///
/// `InstanceModelCol::rows` is a ROW-major 3×4 affine; a transposed pack would produce boxes that
/// are plausible rather than absurd, and the rejection counts would be quietly wrong instead of
/// obviously wrong. Comparing the two folds is what makes the packing observable — this is the one
/// place the two routes are allowed to be independent.
fn ring_packing_deviation(
    assets: &[PlacedAsset],
    locals: &[([f32; 3], [f32; 3])],
    arr: &Arrangement,
) -> f32 {
    let mut worst = 0.0f32;
    for (i, (&slot, &ai)) in arr.ring_slot.iter().zip(arr.ring_asset.iter()).enumerate() {
        let one = single_instance_batch(&arr.batches[ai], i as u32);
        let packed = batch_world_aabb(&one, &arr.ring, locals[ai])
            .expect("invariant: a single-instance batch inside the ring with non-degenerate bounds");
        let direct = world_aabb_direct(locals[ai], &assets[ai], slot);
        let deviation = packed
            .0
            .iter()
            .chain(packed.1.iter())
            .zip(direct.0.iter().chain(direct.1.iter()))
            .map(|(p, d)| (p - d).abs())
            .fold(0.0f32, f32::max);
        worst = worst.max(deviation);
    }
    worst
}

/// The one-instance batch covering ring row `row`, keeping the owning batch's mesh identity.
fn single_instance_batch(owner: &DrawBatch, row: u32) -> DrawBatch {
    DrawBatch { base_instance: row, instance_count: 1, ..*owner }
}

/// Counts rejections at both granularities against one set of planes.
fn measure(
    planes: &[Plane; FRUSTUM_PLANE_COUNT],
    arr: &Arrangement,
    locals: &[([f32; 3], [f32; 3])],
) -> Census {
    let mut rejected_slots = Vec::new();
    for (i, (&slot, &ai)) in arr.ring_slot.iter().zip(arr.ring_asset.iter()).enumerate() {
        let one = single_instance_batch(&arr.batches[ai], i as u32);
        if batch_instance_count_after_cull(planes, &one, &arr.ring, Some(locals[ai])) == 0 {
            rejected_slots.push(slot);
        }
    }

    let mut rejected_batches = Vec::new();
    let mut instances_removed_by_batch = 0usize;
    for (ai, batch) in arr.batches.iter().enumerate() {
        if batch_instance_count_after_cull(planes, batch, &arr.ring, Some(locals[ai])) == 0 {
            rejected_batches.push(ai);
            // A rejected batch removes ALL of its members, so this is what the batch cull buys
            // measured in the same unit as `per_instance`.
            instances_removed_by_batch += batch.instance_count as usize;
        }
    }

    Census {
        per_instance: rejected_slots.len(),
        per_batch: rejected_batches.len(),
        instances_removed_by_batch,
        rejected_slots,
        rejected_batches,
    }
}

/// Asserts the per-instance rung's PREMISE over one extent's measurements.
///
/// Both statements are in INSTANCE units on both sides, which is the point:
///
/// * `per_instance >= instances_removed_by_batch` on every path — the union implication. A batch is
///   the union of its instances, so a batch wholly outside the frustum has every member wholly
///   outside it too. Fewer per-instance rejections than the batch cull removes means the two
///   granularities are not testing the same geometry, i.e. an instrument fault rather than a
///   finding.
/// * strictly greater on at least one path — "granularity buys something". This is the whole reason
///   the per-instance rung exists; if it is ever false, the rung buys nothing on this corpus and
///   that must be loud.
///
/// Neither needs a measured value, so both run at EVERY extent rather than only at the pinned one.
/// That matters because aspect moves the count: `forward_view_proj_rows_jittered` sets
/// `sx = 1/(aspect*tan)`, so a 16:9 field is horizontally WIDER than a 1:1 one and rejects no more.
/// A premise that held only at the pinned 1:1 rung while failing at the decision rung would be a
/// false premise wearing a green test.
fn assert_premise(label: &str, measured: &[Census]) {
    for (p, c) in PATHS.iter().zip(measured.iter()) {
        assert!(
            c.per_instance >= c.instances_removed_by_batch,
            "{label} `{}`: per-INSTANCE rejected {} instances but the per-BATCH cull removes {} \
             instances ({} batches). A batch is the union of its instances, so rejecting the union \
             must imply rejecting every member — this says the two granularities are not testing \
             the same geometry",
            p.id,
            c.per_instance,
            c.instances_removed_by_batch,
            c.per_batch
        );
    }
    assert!(
        measured.iter().any(|c| c.per_instance > c.instances_removed_by_batch),
        "{label}: per-INSTANCE granularity removes NO MORE INSTANCES than the per-BATCH cull \
         already does, on any committed camera path — the premise of the per-instance rung is \
         false at this extent and the rung buys nothing"
    );
    for (p, c) in PATHS.iter().zip(measured.iter()) {
        assert!(
            c.per_instance < SLOT_COUNT,
            "{label} `{}`: every one of {SLOT_COUNT} instances was rejected. A camera path that \
             sees nothing is an instrument failure, not a finding about the arrangement",
            p.id
        );
    }
}

/// **The census: the four numbers the per-instance rung's premise rests on, MEASURED and pinned.**
///
/// Asserted, in order of what each one is worth:
///
/// * the RELATIONSHIP `per_instance >= per_batch` on every committed path, strictly greater on at
///   least one — the statement "granularity buys something", which needs no measured value and must
///   not silently become false;
/// * NON-VACUITY — `approach_close` rejects strictly between none and all of the slots, and every
///   path keeps at least one instance, so a predicate stuck at "reject everything" or "reject
///   nothing" cannot satisfy a bare equality;
/// * the four counts themselves, against [`PINNED`].
#[test]
fn vg_cull_granularity_census() {
    if !vg_corpus_scene::payload_present() {
        eprintln!(
            "SKIP vg_cull_granularity_census: the gitignored corpus payload is absent (run \
             scripts/fetch_corpus.ps1). NOTHING about per-instance vs per-batch cull granularity is \
             measured by this run."
        );
        return;
    }

    assert_eq!(
        SLOT_COUNT, MEASURED_SLOT_COUNT,
        "the pinned counts are rejections out of {MEASURED_SLOT_COUNT} slots; the arrangement now \
         has {SLOT_COUNT}, so every pin below is a count of a different thing"
    );

    // The pins below are counts against a FROZEN corpus and ladder; a silently edited thresholds
    // file would move them without moving anything this test can see. Same call, same reason, as
    // `vg_r0d_census.rs`.
    vg_thresholds::assert_thresholds_frozen();

    // The extent comes from the census's OWN ladder + route table, not from a number invented here.
    let thresholds = read_thresholds();
    let ladder = resolution_ladder(&thresholds);
    let rung = ladder[MEASURED_RUNG];
    let (cw, ch, ssaa) = route_for(rung)
        .unwrap_or_else(|| panic!("ladder rung {MEASURED_RUNG} {rung:?} has no route on this box"));
    assert_eq!(
        ssaa, 1,
        "rung {MEASURED_RUNG} must render at its client extent: with SSAA armed the composite \
         extent the push is built from is not {cw}x{ch}"
    );
    assert_eq!(
        (cw, ch),
        MEASURED_EXTENT,
        "the pins were measured against the {MEASURED_EXTENT:?} (1:1) frustum; this rung now \
         renders {cw}x{ch}, a different aspect and therefore a different horizontal field"
    );

    let assets = vg_corpus_scene::decode_corpus();
    assert!(!assets.is_empty(), "the corpus decoded no assets");
    let locals: Vec<([f32; 3], [f32; 3])> =
        assets.iter().map(|a| vg_corpus_scene::bounds(&a.vertices)).collect();
    for (asset, local) in assets.iter().zip(locals.iter()) {
        // A degenerate or non-finite box makes `batch_instance_count_after_cull` return KEEP
        // unconditionally (frustum.rs:141-144), which would read as "nothing was rejected" —
        // a vacuous census wearing the same face as a real one.
        assert!(
            local
                .0
                .iter()
                .zip(local.1.iter())
                .all(|(lo, hi)| lo.is_finite() && hi.is_finite() && lo <= hi),
            "corpus asset `{}` decoded a degenerate AABB {local:?} — the cull would KEEP it \
             unconditionally and the census would silently measure nothing",
            asset.id
        );
        assert!(
            asset.scale.is_finite() && asset.scale > 0.0,
            "corpus asset `{}` normalised to scale {} — a non-positive or non-finite scale makes \
             every world box below meaningless",
            asset.id,
            asset.scale
        );
    }

    let arr = build_arrangement(&assets);
    assert_eq!(arr.ring.len(), SLOT_COUNT, "the ring must hold one row per arrangement slot");
    assert_eq!(arr.batches.len(), assets.len(), "one batch per registered mesh");
    assert_eq!(
        arr.batches.len(),
        MEASURED_BATCH_COUNT,
        "the pinned per-BATCH counts are out of {MEASURED_BATCH_COUNT} batches; the manifest now \
         holds {} assets, which also re-deals every slot through `slot_asset`",
        assets.len()
    );

    let packing_deviation = ring_packing_deviation(&assets, &locals, &arr);
    assert!(
        packing_deviation < 1.0e-3,
        "the packed instance ring and the direct `scale * local + translation` fold disagree by \
         {packing_deviation} world units — the 3x4 row-major pack is wrong, so every box below is \
         wrong too"
    );

    let measured: Vec<Census> = PATHS
        .iter()
        .map(|p| measure(&production_frustum_planes(p, cw, ch), &arr, &locals))
        .collect();

    // ---- the table --------------------------------------------------------------------------
    eprintln!(
        "\nVG cull granularity census — MEASURED at {cw}x{ch} (ladder rung {MEASURED_RUNG}, aspect \
         {:.4}), {} assets in {SLOT_COUNT} slots, VB push = forward_gbuffer_push_from_view",
        f64::from(cw) / f64::from(ch),
        assets.len()
    );
    eprintln!(
        "| camera path | per-INSTANCE rejects | per-BATCH rejects | instances the BATCH cull removes |"
    );
    eprintln!("|---|---|---|---|");
    for (p, c) in PATHS.iter().zip(measured.iter()) {
        // The last column is the only one comparable with the first: a batch count cannot be
        // weighed against an instance count, and the difference between columns 2 and 4 IS what
        // per-instance granularity buys.
        eprintln!(
            "| {} | {} of {SLOT_COUNT} | {} of {} | {} of {SLOT_COUNT} |",
            p.id,
            c.per_instance,
            c.per_batch,
            arr.batches.len(),
            c.instances_removed_by_batch
        );
    }
    for (p, c) in PATHS.iter().zip(measured.iter()) {
        let names: Vec<&str> = c.rejected_batches.iter().map(|ai| assets[*ai].id.as_str()).collect();
        eprintln!(
            "  {}: rejected slots {:?}; rejected batches {names:?}",
            p.id, c.rejected_slots
        );
    }
    eprintln!(
        "  PINNED = [{}]   <- copy into `PINNED` (per_instance, per_batch), in PATHS order",
        measured
            .iter()
            .map(|c| format!("({}, {})", c.per_instance, c.per_batch))
            .collect::<Vec<_>>()
            .join(", ")
    );
    eprintln!("  ring packing max deviation from the direct fold: {packing_deviation:e} world units");

    // ---- (d) the relationship at the PINNED rung ------------------------------------------------
    assert_premise(&format!("at the pinned rung {cw}x{ch}"), &measured);

    // ---- the same counts at the DECISION rung: counts REPORTED, premise ASSERTED ----------------
    // 1920x1080 is where the campaign adjudicates, and it is 16:9 — a horizontally WIDER field than
    // the 1:1 rung above, so it rejects no more. The four COUNTS stay unpinned there (a second pin
    // set would be a second thing to keep true), but the PREMISE is asserted, because it needs no
    // measured value and could otherwise hold at 1:1 while being false where the decision is made.
    let decision = ladder[decision_rung(&thresholds)];
    match route_for(decision) {
        Some((dw, dh, 1)) => {
            let at_decision: Vec<Census> = PATHS
                .iter()
                .map(|p| measure(&production_frustum_planes(p, dw, dh), &arr, &locals))
                .collect();
            eprintln!(
                "  REPORTED, NOT PINNED — the same counts at the decision rung {dw}x{dh} (aspect \
                 {:.4}):",
                f64::from(dw) / f64::from(dh)
            );
            for (p, c) in PATHS.iter().zip(at_decision.iter()) {
                eprintln!(
                    "    {}: per-instance {}, per-batch {}, instances removed by batch cull {}",
                    p.id, c.per_instance, c.per_batch, c.instances_removed_by_batch
                );
            }
            assert_premise(&format!("at the decision rung {dw}x{dh}"), &at_decision);
        }
        other => {
            // Naming the skip, for the same reason the payload skip names itself: a comparison that
            // silently did not run is indistinguishable from one that agreed.
            eprintln!(
                "  NOT COMPUTED — the decision rung {decision:?} routes to {other:?}; the 16:9 \
                 comparison and its premise assertion did NOT run in this process"
            );
        }
    }

    // ---- (e) non-vacuity ------------------------------------------------------------------------
    let close = PATHS
        .iter()
        .position(|p| p.id == "approach_close")
        .expect("invariant: `approach_close` is a committed camera path");
    let close_rejects = measured[close].per_instance;
    assert!(
        close_rejects > 0 && close_rejects < SLOT_COUNT,
        "`approach_close` rejected {close_rejects} of {SLOT_COUNT} instances. 0 and {SLOT_COUNT} \
         are the two answers a broken predicate gives (reject nothing / reject everything), and \
         either would satisfy a bare equality against a pin measured from the same broken run"
    );
    // The "every instance rejected" guard lives in `assert_premise`, which runs at both extents.

    // ---- the pins -------------------------------------------------------------------------------
    for ((p, c), (want_instance, want_batch)) in PATHS.iter().zip(measured.iter()).zip(PINNED.iter())
    {
        assert!(
            *want_instance != UNPINNED && *want_batch != UNPINNED,
            "`{}` is UNPINNED: `PINNED` still holds its placeholder. Copy the `PINNED = [...]` line \
             printed above into this file — the numbers are measured first and pinned second, never \
             predicted",
            p.id
        );
        assert_eq!(
            c.per_instance, *want_instance,
            "`{}`: per-INSTANCE rejections moved from the pinned {want_instance} to {}. Either the \
             corpus arrangement, a committed camera path, or the production frustum route changed",
            p.id, c.per_instance
        );
        assert_eq!(
            c.per_batch, *want_batch,
            "`{}`: per-BATCH rejections moved from the pinned {want_batch} to {}",
            p.id, c.per_batch
        );
    }
}
