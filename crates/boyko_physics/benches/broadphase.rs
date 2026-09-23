//! O2 broadphase A/B: the uniform-grid CSR broadphase ([`BroadphaseGrid::build`])
//! vs the shipped O(n²) all-pairs loop, at {100, 1k, 10k} bodies.
//!
//! Reports the measured crossover (the plan promises only "expect O(100s)", not a
//! number). Each scene is a moderately dense cluster so the candidate set is
//! non-trivial; every benched scene asserts `pairs.len() > 0` (anti-vacuity) so a
//! degenerate empty broadphase never reads as a win.
//!
//! # G4 of the tree broadphase (`bp_g4_*` groups)
//!
//! Gate G4 of `docs/physics/perf-campaign/levers/broadphase/04-DESIGN-REV2.md` (commit C3),
//! with the maintenance arm ruling W1 of `levers/00-RULINGS.md` ("Broadphase rev 2") added.
//! Nothing here is a pass/fail gate by itself: the design's stop rules (the J snapshot above
//! 0.30 ms; the Tree slower than the Grid at the same W above the brute threshold; any
//! maintenance arm above 2× the upper value of the design's D3.5 cost table) are read off the
//! medians by whoever runs the quiet window, and its outputs (`TREE_BRUTE_MAX_ROWS`,
//! `AUTO_TREE_LO` / `AUTO_TREE_HI`, `ADMIT_BUILD_RATIO`) are derived there (commit C4).
//!
//! **Pair finding** (`bp_g4_uniform`, `bp_g4_disparity`, `bp_g4_scene`): `all_pairs` (the
//! crate's [`all_pairs_into`], the Tree's own brute path and the gates' oracle), `grid_w1`
//! ([`BroadphaseGrid::build`]), `grid_w8` (`build_parallel` under an 8-worker pool; below
//! `MIN_PARALLEL_BODIES` = 4096 it takes the serial path, so those rows read like `grid_w1`)
//! and `tree` ([`BroadphaseTree::step_direct`] with the tree path forced, timed in its steady
//! state: three untimed steps first, so a static is a member and every timed step is an
//! `Identity` verify, the active build, the queries and the assembly). `tree` runs the default
//! query kernel ([`QueryKernel::LeafList`], C3b); `tree_rowwalk` is the same step with
//! [`QueryKernel::RowWalk`], C1's per-row walk, so the two kernels are compared in one binary.
//! Before timing, each family and size prints a receipt per kernel — the `TreeDiag` counts of the
//! active leaf nodes each path answered (`leaf_list_leaves`, `fallback_leaves`,
//! `row_walk_leaves`) — and asserts that the kernel it names is the one that ran; each timed arm
//! asserts it again on its own instance. Sizes: the design's
//! {17, 64, 128, 256, 1k, 10k, 100k} for `uniform` (unit spheres on a jittered lattice) and
//! `disparity` (the O2 W1 scene: small spheres plus four giants); `scene` is Jolt's pyramid on
//! its static floor at 1 240 boxes (J at t = 0), 10k and 100k (taller pyramids, truncated), and
//! the **J snapshot**: the real colored schedule's `SolverScratch` bodies after
//! [`J_SNAPSHOT_STEPS`] steps of J with the Tree, which is the row the 0.30 ms stop rule names.
//! The `uniform` and `disparity` bodies are made dynamic (`inv_mass = 1`), so the Tree has no
//! persistent set there and the comparison with AllPairs and the Grid is one of pair finding
//! alone; `scene` carries one static, the floor, as J does. The brute threshold is read as the
//! largest size at which `all_pairs` is not slower than `tree`.
//!
//! **Maintenance** (`bp_g4_maintenance`, m ∈ {1 240, 10k, 100k} static members of a lattice of
//! radius-0.65 spheres at pitch 0.9, so every interior member has 18 partners and the static
//! list holds 7.6 / 8.3 / 8.7 entries per member at the three sizes — the boundary rows have
//! fewer — beside J's 7.7 per row): each arm times exactly one step of a cycle, the step that
//! performs the operation, with the cycle's other steps untimed:
//!
//! | arm | the timed step | untimed around it |
//! |---|---|---|
//! | `stable` | an `Identity` step with no change: the verify plus the copy of the persistent pairs | — |
//! | `admission_from_empty` | the rent rule admits all m rows (rev 1's rebuild) | every member teleported (evict, filter, compact), then one pending step |
//! | `admission_of_64` | the rent rule admits 64 pending rows into m members | 64 extra statics teleported, then the pending steps the rent rule takes (the design's ≈ 6 / 40 / 390) |
//! | `eviction_filter` | one member teleported: the eviction and the list filter | a step on which the rent rule re-admits the accumulated pending rows is excluded and redone |
//! | `shift_translation` | a `Rows` step with every row shifted by one (a spawn at the front, or its despawn): the translation, `patches = 0` | — |
//! | `compaction` | one eviction that takes the dead lanes to the live count: the filter plus the compaction (subtract `eviction_filter`) | the members below half teleported, then teleported back and re-admitted |
//! | `high_jumper` | ruling W1: an interior member with ≥ 2 higher-row partners migrated to the top row (and back on the next cycle): the translation with its entries diverted and merged | — |
//!
//! Each maintenance arm prints a structural receipt before timing (the step's `TreeDiag`
//! deltas and the pair count against the oracle), and asserts the deltas the arm is defined by
//! (a translation step translates exactly once, an admission rebuilds exactly once, the jumper
//! diverts at least two entries), so an arm that measured a different step cannot run.

use std::sync::Arc;
use std::time::{Duration, Instant};

use criterion::{BenchmarkId, Criterion, SamplingMode, criterion_group, criterion_main};
use std::hint::black_box;

use boyko_physics::broadphase_tree::{
    BroadphaseTree, NO_PREV_ROW, QueryKernel, TreeDiag, all_pairs_into,
};
use boyko_physics::components::ColliderShape;
use boyko_physics::manifold::BodyIndex;
use boyko_physics::math::Vec3;
use boyko_physics::resources::{BodyState, BroadphaseGrid, ContactPairs};
use boyko_physics::systems::body_bounding_radius;
use boyko_threadpool::{ThreadPool, ThreadPoolBuilder};

// The G4 scenes live in a module the C3b counting driver (`tests/bp_query_counts.rs`) includes
// too, so the bodies it counts are the bodies this bench times.
#[path = "support/bp_g4_scenes.rs"]
mod scenes;

use scenes::{
    BOX_INV_MASS, BOX_SIZE, FLOOR_HALF_EXTENTS, HALF_BOX, J_SNAPSHOT_STEPS, JOLT_SEPARATION,
    TREE_WARM_STEPS, disparity_scene, j_snapshot, make_dynamic, scene, sphere,
};

/// A TRANSCRIPTION of the shipped `AllPairs` arm
/// ([`physics_broadphase`](boyko_physics::systems::physics_broadphase)), not the arm
/// itself.
///
/// ⚠ The distinction is load-bearing and this comment used to hide it: it read "the
/// shipped `AllPairs` arm", which reads as *this row executes that code*. It does
/// not — the bench never builds a schedule, so nothing here can catch a regression
/// that lives in the system. What this row measures is the LOOP SHAPE, in
/// isolation, which is why it must be kept container-identical to the arm: it took
/// `&mut Vec` while the arm moved to a `ScratchColumn` (audit Stage 4), and a row
/// that still measures the old container reads GREEN across exactly the change it
/// was pointed at.
///
/// The arm end-to-end is covered by `jolt_parity_pyramid`, whose cfg-A sets
/// `PhysicsConfig::broadphase = AllPairs` explicitly (cfg-B sets `Grid`) and runs it
/// through the real system.
fn all_pairs(bodies: &[BodyState], out: &mut ContactPairs) {
    let mut out = out.pairs_build();
    out.clear();
    let n = bodies.len();
    for i in 0..n {
        for j in (i + 1)..n {
            let bound = body_bounding_radius(&bodies[i]) + body_bounding_radius(&bodies[j]);
            let delta = bodies[j].position - bodies[i].position;
            if delta.length_squared() <= bound * bound {
                out.push((BodyIndex(i as u32), BodyIndex(j as u32)));
            }
        }
    }
}

fn bench_broadphase(c: &mut Criterion) {
    let mut group = c.benchmark_group("broadphase");
    for &n in &[100usize, 1_000, 10_000] {
        let bodies = scene(n);

        // Anti-vacuity: confirm the scene yields real pairs before benching.
        let mut probe = BroadphaseGrid::with_capacity(n);
        let mut probe_out = ContactPairs::with_capacity(0);
        probe.build(&bodies, &mut probe_out);
        assert!(
            !probe_out.pairs().is_empty(),
            "benched scene (n={n}) must produce pairs (anti-vacuity)"
        );

        group.bench_with_input(BenchmarkId::new("all_pairs", n), &bodies, |b, bodies| {
            let mut out = ContactPairs::with_capacity(0);
            b.iter(|| {
                all_pairs(black_box(bodies), &mut out);
                black_box(out.pairs().len());
            });
        });

        group.bench_with_input(BenchmarkId::new("grid", n), &bodies, |b, bodies| {
            let mut grid = BroadphaseGrid::with_capacity(bodies.len());
            let mut out = ContactPairs::with_capacity(0);
            // Warm the grid so the timed iterations measure the steady-state
            // (capacity-reused, alloc-free) build, not first-build growth.
            grid.build(bodies, &mut out);
            b.iter(|| {
                grid.build(black_box(bodies), &mut out);
                black_box(out.pairs().len());
            });
        });
    }
    group.finish();
}

/// O2 W1 size-disparity criterion: one batch of typical small bodies + a few
/// giants. The decoupled-floor grid must beat all-pairs at scale (where the old
/// `max_radius`-floored grid would have tied/lost by coarsening to all-pairs).
fn bench_disparity(c: &mut Criterion) {
    let mut group = c.benchmark_group("broadphase_disparity");
    for &n in &[1_000usize, 10_000] {
        let bodies = disparity_scene(n);

        // Anti-vacuity: the giants pair with many small bodies.
        let mut probe = BroadphaseGrid::with_capacity(bodies.len());
        let mut probe_out = ContactPairs::with_capacity(0);
        probe.build(&bodies, &mut probe_out);
        assert!(
            !probe_out.pairs().is_empty(),
            "disparity scene (n={n}) must produce pairs (anti-vacuity)"
        );
        assert!(
            probe.oversized_len() >= 2,
            "disparity scene (n={n}) must route >= 2 bodies oversized (got {})",
            probe.oversized_len()
        );

        group.bench_with_input(BenchmarkId::new("all_pairs", n), &bodies, |b, bodies| {
            let mut out = ContactPairs::with_capacity(0);
            b.iter(|| {
                all_pairs(black_box(bodies), &mut out);
                black_box(out.pairs().len());
            });
        });

        group.bench_with_input(BenchmarkId::new("grid", n), &bodies, |b, bodies| {
            let mut grid = BroadphaseGrid::with_capacity(bodies.len());
            let mut out = ContactPairs::with_capacity(0);
            grid.build(bodies, &mut out);
            b.iter(|| {
                grid.build(black_box(bodies), &mut out);
                black_box(out.pairs().len());
            });
        });
    }
    group.finish();
}

/// O3 Gate 7: PARALLEL candidate-emit scaling. `BroadphaseGrid::build_parallel`
/// dispatched through a real `boyko_threadpool` at workers ∈ {1, 2, 4} on a DENSE
/// scene (n_cells ≈ n → a genuine multi-cell candidate set), at n ∈ {1k, 10k,
/// 100k}.
///
/// The headline gate is the speedup @100k: 4 workers vs 1 worker should reach a
/// ratio of at least 2.8x (the plan's Amdahl estimate is f ~ 0.04-0.10, i.e. a
/// ~3.08x ceiling at the serial CSR + final-sort fraction). Criterion reports each
/// lane's median; the 4-vs-1 ratio at 100k is read from the medians.
///
/// **What the `w1` row measures after KE16 App-1 (2026-09-02).** App-1 set
/// `lanes = pool.num_threads()` and re-aimed the previously dead `lanes < 2` guard,
/// so a ONE-worker pool no longer dispatches at all: `w1` is the O2 serial `build`
/// plus one `try_with_active_pool` probe, and `w1 / serial_o2` is therefore ≈ 1.00x
/// BY CONSTRUCTION. It is a receipt that the guard routes as documented, not a probe.
///
/// The W=1-vs-O2-serial SHAPED regression probe this doc used to claim for that row
/// is RETIRED, not moved: after App-1 the one-lane shaped path is unreachable through
/// `build_parallel` at any worker count, so the regression it guarded (a one-lane
/// caller paying Pass A + prefix-sum + per-chunk sort over the serial `build`) can no
/// longer occur. `crates/boyko_physics/tests/broadphase_grid.rs::
/// one_worker_build_parallel_takes_the_serial_fallback` is the gate on that routing,
/// with an allocation receipt; the shaped path is measured from `w2` upward.
///
/// Consequently the 4-vs-1 ratio's DENOMINATOR is now the serial path rather than the
/// one-lane shaped path. The serial path is the faster of the two, so the same 2.8x
/// line is now at least as strict as when it was calibrated — the threshold is not
/// loosened by App-1, but the number is against a different reference and the results
/// file records which.
///
/// Below MIN_PARALLEL_BODIES (= 4096) `build_parallel` takes the no-pool serial
/// shaped path regardless of the pool — so n=1k is a single-lane reference; the
/// scaling claim is read at 10k and (the gate) 100k where the dispatched branch
/// is live. The bench is DENSE + the parallel path is gated — both stated so a
/// degenerate scene can never read as a win (every scene asserts pairs > 0).
fn bench_parallel(c: &mut Criterion) {
    let mut group = c.benchmark_group("broadphase_parallel");
    // The parallel-emit dispatch + final sort dominate the per-iter cost at scale;
    // a modest sample count keeps the 100k × 3-worker matrix wall-clock reasonable
    // while criterion still reports a stable median.
    group.sample_size(20);

    for &n in &[1_000usize, 10_000, 100_000] {
        let bodies = scene(n);

        // Anti-vacuity: confirm the scene yields real pairs before benching, and
        // report whether it is at/above the parallel-dispatch threshold (4096) so
        // the reader knows which n's actually exercise the dispatched branch.
        let mut probe = BroadphaseGrid::with_capacity(n);
        let mut probe_out = ContactPairs::with_capacity(0);
        probe.build(&bodies, &mut probe_out);
        assert!(
            !probe_out.pairs().is_empty(),
            "parallel bench scene (n={n}) must produce pairs (anti-vacuity)"
        );

        // The O2 serial `build` baseline at this n. After App-1 this is also what
        // the `w1` row runs (the guard routes a one-worker pool here), so the two
        // medians read alike by construction; the pair is the routing receipt.
        group.bench_with_input(BenchmarkId::new("serial_o2", n), &bodies, |b, bodies| {
            let mut grid = BroadphaseGrid::with_capacity(bodies.len());
            let mut out = ContactPairs::with_capacity(0);
            grid.build(bodies, &mut out);
            b.iter(|| {
                grid.build(black_box(bodies), &mut out);
                black_box(out.pairs().len());
            });
        });

        for &workers in &[1usize, 2, 4] {
            let id = BenchmarkId::new(format!("w{workers}"), n);
            group.bench_with_input(id, &bodies, |b, bodies| {
                let pool = ThreadPoolBuilder::new().num_threads(workers).build();
                // Warm + time INSIDE one install frame so `try_with_active_pool`
                // finds the ambient pool every iteration (the dispatched branch at
                // workers >= 2; at workers = 1 the App-1 guard routes to the O2
                // serial `build`, which is what that row reports); warm-up grows
                // every scratch Vec so the timed builds are the steady-state,
                // capacity-reused (bounded-alloc) path.
                pool.install(|_scope| {
                    let mut grid = BroadphaseGrid::with_capacity(bodies.len());
                    let mut out = ContactPairs::with_capacity(0);
                    for _ in 0..3 {
                        grid.build_parallel(bodies, &mut out);
                    }
                    b.iter(|| {
                        grid.build_parallel(black_box(bodies), &mut out);
                        black_box(out.pairs().len());
                    });
                });
            });
        }
    }
    group.finish();
}

// ── G4: the tree broadphase (design C3; module docs, "G4 of the tree broadphase") ────────────

/// G4 pair-finding sizes of the `uniform` and `disparity` families: the design's list.
const G4_SIZES: [usize; 7] = [17, 64, 128, 256, 1_000, 10_000, 100_000];
/// G4 `scene` sizes, in dynamic boxes (the floor is one more row): J, and the design's 10k and
/// 100k.
const G4_SCENE_SIZES: [usize; 3] = [1_240, 10_000, 100_000];
/// Maintenance member counts: the columns of the design's D3.5 cost table.
const G4_MEMBERS: [usize; 3] = [1_240, 10_000, 100_000];
/// Pending rows the `admission_of_64` arm admits (D3.5's row).
const ADMISSION_PENDING: usize = 64;
/// Steps a maintenance cycle may take before it is a construction error (the rent rule admits
/// 64 pending rows into 100k members after about 390 steps).
const MAINT_STEP_CAP: usize = 4_000;
/// The maintenance lattice's pitch: the 6 axis neighbours sit at 0.9, the 12 face diagonals at
/// 1.273, the 8 cube diagonals at 1.559.
const LATTICE_PITCH: f32 = 0.9;
/// The maintenance lattice's radius: a bound of 1.3 reaches the face diagonals and not the cube
/// ones, so an interior member has 18 partners.
const LATTICE_RADIUS: f32 = 0.65;
/// The teleport of the maintenance arms: far from the lattice, so a moved member pairs only with
/// other moved members (which form the same lattice there).
const TELEPORT_X: f32 = 1_000.0;
/// Where the extra rows of the maintenance arms sit (the 64 to admit, the shifting dynamic): far
/// from the lattice, and still far from it after a teleport.
const EXTRA_X: f32 = -3_000.0;
/// Workers of the `grid_w8` arm.
const GRID_WORKERS: usize = 8;

// ── G4 scenes ─────────────────────────────────────────────────────────────────

/// A `BodyState` of a box (`inv_mass == 0` is a static).
fn boxed(position: Vec3, half_extents: Vec3, inv_mass: f32) -> BodyState {
    BodyState {
        position,
        shape: ColliderShape::Box { half_extents },
        inv_mass,
        ..Default::default()
    }
}

/// The pyramid height whose box count `h(h+1)(2h+1)/6` reaches `n` (15 at J's 1 240).
fn pyramid_height_for(n: usize) -> i32 {
    let mut h = 1usize;
    while h * (h + 1) * (2 * h + 1) / 6 < n {
        h += 1;
    }
    i32::try_from(h).expect("construction: a pyramid height fits i32")
}

/// Jolt's pyramid loop, index for index, on J's static floor at row 0: `n` dynamic boxes, the
/// pyramid as tall as `n` needs and truncated at `n`. At `n = 1 240` it is J at t = 0.
fn in_scene(n: usize) -> Vec<BodyState> {
    let height = pyramid_height_for(n);
    let mut bodies = Vec::with_capacity(n + 1);
    bodies.push(boxed(Vec3::new(0.0, -1.0, 0.0), FLOOR_HALF_EXTENTS, 0.0));
    'outer: for i in 0..height {
        let lo = i / 2;
        let hi = height - (i + 1) / 2;
        for j in lo..hi {
            for k in lo..hi {
                if bodies.len() == n + 1 {
                    break 'outer;
                }
                let odd = if i & 1 != 0 { HALF_BOX } else { 0.0 };
                let position = Vec3::new(
                    -(height as f32) + BOX_SIZE * j as f32 + odd,
                    1.0 + (BOX_SIZE + JOLT_SEPARATION) * i as f32,
                    -(height as f32) + BOX_SIZE * k as f32 + odd,
                );
                bodies.push(boxed(position, Vec3::new(HALF_BOX, HALF_BOX, HALF_BOX), BOX_INV_MASS));
            }
        }
    }
    assert_eq!(bodies.len(), n + 1, "construction: the scene holds n boxes and the floor");
    bodies
}

// ── G4 pair finding ───────────────────────────────────────────────────────────

/// Asserts that `d`, a tree's counters after tree-path steps under `kernel`, names `kernel` as
/// the path that answered its active leaves: the leaf list (some of them possibly through its
/// fallback) or the per-row walk, and never the other.
fn assert_kernel_receipt(d: &TreeDiag, kernel: QueryKernel, what: &str) {
    match kernel {
        QueryKernel::LeafList => {
            assert!(d.leaf_list_leaves + d.fallback_leaves > 0, "{what}: the leaf list answered no leaf: {d:?}");
            assert_eq!(d.row_walk_leaves, 0, "{what}: the per-row walk ran under the leaf list: {d:?}");
        }
        QueryKernel::RowWalk => {
            assert!(d.row_walk_leaves > 0, "{what}: the per-row walk answered no leaf: {d:?}");
            assert_eq!(
                (d.leaf_list_leaves, d.fallback_leaves),
                (0, 0),
                "{what}: the leaf list ran under the per-row walk: {d:?}"
            );
        }
    }
}

/// The tree arm under `kernel`: its steady state, then the timed step.
fn bench_tree_arm(
    group: &mut criterion::BenchmarkGroup<'_, criterion::measurement::WallTime>,
    name: &str,
    kernel: QueryKernel,
    param: &str,
    bodies: &[BodyState],
) {
    group.bench_with_input(BenchmarkId::new(name, param), bodies, |b, bodies| {
        let mut tree = BroadphaseTree::with_capacity(bodies.len());
        tree.set_brute_max_rows(0);
        tree.set_query_kernel(kernel);
        let mut out = ContactPairs::with_capacity(0);
        for _ in 0..TREE_WARM_STEPS {
            tree.step_direct(bodies, &mut out);
        }
        assert_kernel_receipt(&tree.diag(), kernel, name);
        b.iter(|| {
            tree.step_direct(black_box(bodies), &mut out);
            black_box(out.pairs().len());
        });
    });
}

/// The five pair-finding arms over one body set, labelled `param`.
fn bench_g4_arms(
    group: &mut criterion::BenchmarkGroup<'_, criterion::measurement::WallTime>,
    pool: &Arc<ThreadPool>,
    family: &str,
    param: &str,
    bodies: &[BodyState],
) {
    let n = bodies.len();
    let mut oracle = ContactPairs::with_capacity(0);
    all_pairs_into(bodies, &mut oracle);
    assert!(!oracle.pairs().is_empty(), "anti-vacuity: {family}/{param} must produce pairs");
    for kernel in [QueryKernel::LeafList, QueryKernel::RowWalk] {
        // The Tree's steady state equals the oracle under each kernel, and the receipt names
        // what it holds and the kernel that answered.
        let mut tree = BroadphaseTree::with_capacity(n);
        tree.set_brute_max_rows(0);
        tree.set_query_kernel(kernel);
        let mut out = ContactPairs::with_capacity(0);
        for _ in 0..TREE_WARM_STEPS {
            tree.step_direct(bodies, &mut out);
        }
        assert_eq!(
            out.pairs(),
            oracle.pairs(),
            "{family}/{param} ({kernel:?}): the tree's steady-state pair set is AllPairs'"
        );
        let d = tree.diag();
        assert_kernel_receipt(&d, kernel, &format!("bp_g4_{family}/{param}"));
        eprintln!(
            "bp_g4_{family}/{param}: kernel {kernel:?} rows {n} pairs {} tree members {} \
             static_rebuilds {} wide {} excluded {} leaf_list_leaves {} fallback_leaves {} \
             row_walk_leaves {}",
            oracle.pairs().len(),
            d.members,
            d.static_rebuilds,
            d.wide_rows,
            d.excluded_rows,
            d.leaf_list_leaves,
            d.fallback_leaves,
            d.row_walk_leaves
        );
    }

    group.bench_with_input(BenchmarkId::new("all_pairs", param), bodies, |b, bodies| {
        let mut out = ContactPairs::with_capacity(0);
        b.iter(|| {
            all_pairs_into(black_box(bodies), &mut out);
            black_box(out.pairs().len());
        });
    });
    group.bench_with_input(BenchmarkId::new("grid_w1", param), bodies, |b, bodies| {
        let mut grid = BroadphaseGrid::with_capacity(bodies.len());
        let mut out = ContactPairs::with_capacity(0);
        grid.build(bodies, &mut out);
        b.iter(|| {
            grid.build(black_box(bodies), &mut out);
            black_box(out.pairs().len());
        });
    });
    group.bench_with_input(BenchmarkId::new("grid_w8", param), bodies, |b, bodies| {
        pool.install(|_scope| {
            let mut grid = BroadphaseGrid::with_capacity(bodies.len());
            let mut out = ContactPairs::with_capacity(0);
            for _ in 0..TREE_WARM_STEPS {
                grid.build_parallel(bodies, &mut out);
            }
            b.iter(|| {
                grid.build_parallel(black_box(bodies), &mut out);
                black_box(out.pairs().len());
            });
        });
    });
    bench_tree_arm(group, "tree", QueryKernel::LeafList, param, bodies);
    bench_tree_arm(group, "tree_rowwalk", QueryKernel::RowWalk, param, bodies);
}

/// G4 pair finding: the `uniform` and `disparity` families at the design's sizes, and the
/// `scene` family (J at t = 0, taller pyramids, the J snapshot).
fn bench_g4_pairs(c: &mut Criterion) {
    let pool = ThreadPoolBuilder::new().num_threads(GRID_WORKERS).build();
    {
        let mut group = c.benchmark_group("bp_g4_uniform");
        group.sampling_mode(SamplingMode::Flat).sample_size(10);
        for &n in &G4_SIZES {
            let mut bodies = scene(n);
            make_dynamic(&mut bodies);
            bench_g4_arms(&mut group, &pool, "uniform", &n.to_string(), &bodies);
        }
        group.finish();
    }
    {
        let mut group = c.benchmark_group("bp_g4_disparity");
        group.sampling_mode(SamplingMode::Flat).sample_size(10);
        for &n in &G4_SIZES {
            let mut bodies = disparity_scene(n);
            make_dynamic(&mut bodies);
            bench_g4_arms(&mut group, &pool, "disparity", &n.to_string(), &bodies);
        }
        group.finish();
    }
    {
        let mut group = c.benchmark_group("bp_g4_scene");
        group.sampling_mode(SamplingMode::Flat).sample_size(10);
        for &n in &G4_SCENE_SIZES {
            let bodies = in_scene(n);
            bench_g4_arms(&mut group, &pool, "scene", &n.to_string(), &bodies);
        }
        let snapshot = j_snapshot();
        bench_g4_arms(&mut group, &pool, "scene", &format!("j{J_SNAPSHOT_STEPS}"), &snapshot);
        group.finish();
    }
}

// ── G4 maintenance ────────────────────────────────────────────────────────────

/// `m` static spheres on the maintenance lattice.
fn statics_lattice(m: usize) -> Vec<BodyState> {
    let side = (m as f64).cbrt().ceil() as usize;
    let mut bodies = Vec::with_capacity(m);
    'outer: for z in 0..side {
        for y in 0..side {
            for x in 0..side {
                if bodies.len() == m {
                    break 'outer;
                }
                let p = Vec3::new(
                    x as f32 * LATTICE_PITCH,
                    y as f32 * LATTICE_PITCH,
                    z as f32 * LATTICE_PITCH,
                );
                bodies.push(sphere(p, LATTICE_RADIUS));
            }
        }
    }
    assert_eq!(bodies.len(), m, "construction: the lattice holds m statics");
    bodies
}

/// `k` extra static spheres on a small lattice at [`EXTRA_X`].
fn extra_statics(k: usize) -> Vec<BodyState> {
    let mut extra = statics_lattice(k);
    for body in &mut extra {
        body.position = body.position + Vec3::new(EXTRA_X, 0.0, 0.0);
    }
    extra
}

/// The per-step deltas of the cumulative counters, plus the members after the step.
#[derive(Clone, Copy, Debug)]
struct DiagDelta {
    evictions: u64,
    translations: u64,
    patches: u64,
    static_rebuilds: u64,
    members: u64,
}

impl DiagDelta {
    fn between(before: TreeDiag, after: TreeDiag) -> Self {
        Self {
            evictions: after.evictions - before.evictions,
            translations: after.translations - before.translations,
            patches: after.patches - before.patches,
            static_rebuilds: after.static_rebuilds - before.static_rebuilds,
            members: after.members,
        }
    }
}

/// One timed step's wall time and counter deltas.
struct Timed {
    wall: Duration,
    delta: DiagDelta,
}

/// The maintenance harness: one tree under direct drive, the bodies it reads, and the bench's
/// bookkeeping of which rows are teleported.
struct Maint {
    tree: BroadphaseTree,
    bodies: Vec<BodyState>,
    out: ContactPairs,
    /// `moved[r]`: row `r` is at its teleported pose.
    moved: Vec<bool>,
    /// The previous-row map of a `Rows` step, refilled per step.
    prev: Vec<u32>,
}

impl Maint {
    /// A tree over `bodies` in its steady state: [`TREE_WARM_STEPS`] steps, after which every
    /// still static is a member.
    fn new(bodies: Vec<BodyState>) -> Self {
        let n = bodies.len();
        let mut tree = BroadphaseTree::with_capacity(n + 1);
        tree.set_brute_max_rows(0);
        let mut maint = Self {
            tree,
            bodies,
            out: ContactPairs::with_capacity(0),
            moved: vec![false; n],
            prev: Vec::with_capacity(n + 1),
        };
        for _ in 0..TREE_WARM_STEPS {
            maint.identity_step();
        }
        maint
    }

    /// An `Identity` step (direct drive), timed.
    fn identity_step(&mut self) -> Timed {
        let before = self.tree.diag();
        let start = Instant::now();
        self.tree.step_direct(&self.bodies, &mut self.out);
        let wall = start.elapsed();
        Timed { wall, delta: DiagDelta::between(before, self.tree.diag()) }
    }

    /// A `Rows` step through `self.prev`, timed.
    fn rows_step(&mut self) -> Timed {
        let before = self.tree.diag();
        let start = Instant::now();
        self.tree.step_translated(&self.bodies, &self.prev, &mut self.out);
        let wall = start.elapsed();
        Timed { wall, delta: DiagDelta::between(before, self.tree.diag()) }
    }

    /// Toggles row `r` between its home pose and the teleported one.
    fn teleport(&mut self, r: usize) {
        let dx = if self.moved[r] { -TELEPORT_X } else { TELEPORT_X };
        self.bodies[r].position = self.bodies[r].position + Vec3::new(dx, 0.0, 0.0);
        self.moved[r] = !self.moved[r];
    }

    /// Asserts the last step's output is the exact set of the bodies it read.
    fn check_oracle(&self, what: &str) {
        let mut oracle = ContactPairs::with_capacity(0);
        all_pairs_into(&self.bodies, &mut oracle);
        assert_eq!(self.out.pairs(), oracle.pairs(), "{what}: the tree's pair set is AllPairs'");
        assert!(!oracle.pairs().is_empty(), "anti-vacuity: {what} must produce pairs");
    }
}

/// The maintenance arms.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MaintArm {
    Stable,
    AdmissionFromEmpty,
    AdmissionOf64,
    EvictionFilter,
    ShiftTranslation,
    Compaction,
    HighJumper,
}

impl MaintArm {
    const ALL: [Self; 7] = [
        Self::Stable,
        Self::AdmissionFromEmpty,
        Self::AdmissionOf64,
        Self::EvictionFilter,
        Self::ShiftTranslation,
        Self::Compaction,
        Self::HighJumper,
    ];

    fn name(self) -> &'static str {
        match self {
            Self::Stable => "stable",
            Self::AdmissionFromEmpty => "admission_from_empty",
            Self::AdmissionOf64 => "admission_of_64",
            Self::EvictionFilter => "eviction_filter",
            Self::ShiftTranslation => "shift_translation",
            Self::Compaction => "compaction",
            Self::HighJumper => "high_jumper",
        }
    }
}

/// The per-arm state of a cycle.
struct MaintCycle {
    maint: Maint,
    arm: MaintArm,
    /// The member count the arm keeps (the lattice's `m`).
    m: usize,
    /// `eviction_filter`: the next row to teleport.
    cursor: usize,
    /// `shift_translation`: the dynamic front row is present; `high_jumper`: the mover is at
    /// the top row.
    phase: bool,
    /// `high_jumper`: the mover's home row.
    jumper: usize,
}

impl MaintCycle {
    fn new(arm: MaintArm, m: usize) -> Self {
        let mut bodies = statics_lattice(m);
        if arm == MaintArm::AdmissionOf64 {
            bodies.extend(extra_statics(ADMISSION_PENDING));
        }
        let maint = Maint::new(bodies);
        let d = maint.tree.diag();
        assert_eq!(d.members as usize, maint.bodies.len(), "every static is a member after the warm-up");
        assert_eq!(d.static_rebuilds, 1, "one admission in the warm-up");
        Self { maint, arm, m, cursor: 0, phase: false, jumper: m / 2 }
    }

    /// One cycle: returns the timed step, after asserting the deltas the arm is defined by.
    fn run(&mut self) -> Timed {
        let m = self.m as u64;
        match self.arm {
            MaintArm::Stable => {
                let t = self.maint.identity_step();
                assert_eq!(t.delta.evictions, 0, "stable: nothing evicted");
                assert_eq!(t.delta.static_rebuilds, 0, "stable: nothing rebuilt");
                assert_eq!(t.delta.members, m, "stable: every member kept");
                t
            }
            MaintArm::AdmissionFromEmpty => {
                for r in 0..self.m {
                    self.maint.teleport(r);
                }
                let t = self.maint.identity_step();
                assert_eq!(t.delta.evictions, m, "every member evicted");
                assert_eq!(t.delta.members, 0);
                assert_eq!(t.delta.static_rebuilds, 1, "the dead lanes are compacted away");
                let t = self.maint.identity_step();
                assert_eq!(t.delta.static_rebuilds, 0, "the first still step only pays rent");
                let t = self.maint.identity_step();
                assert_eq!(t.delta.static_rebuilds, 1, "the second still step admits");
                assert_eq!(t.delta.members, m, "every static re-admitted");
                t
            }
            MaintArm::AdmissionOf64 => {
                for r in self.m..self.m + ADMISSION_PENDING {
                    self.maint.teleport(r);
                }
                let t = self.maint.identity_step();
                assert_eq!(t.delta.evictions, ADMISSION_PENDING as u64, "the 64 are evicted");
                assert_eq!(t.delta.members, m, "the lattice keeps its members");
                assert_eq!(t.delta.static_rebuilds, 0, "64 dead lanes do not compact");
                for _ in 0..MAINT_STEP_CAP {
                    let t = self.maint.identity_step();
                    if t.delta.static_rebuilds == 1 {
                        assert_eq!(t.delta.members, m + ADMISSION_PENDING as u64, "the 64 admitted");
                        return t;
                    }
                    assert_eq!(t.delta.static_rebuilds, 0);
                }
                panic!("construction: the rent rule did not admit 64 rows within {MAINT_STEP_CAP} steps");
            }
            MaintArm::EvictionFilter => {
                for _ in 0..MAINT_STEP_CAP {
                    let r = self.cursor;
                    self.cursor = (self.cursor + 1) % self.m;
                    self.maint.teleport(r);
                    let t = self.maint.identity_step();
                    if t.delta.static_rebuilds == 0 {
                        assert_eq!(t.delta.evictions, 1, "the teleported member is evicted");
                        return t;
                    }
                }
                panic!("construction: every step within {MAINT_STEP_CAP} re-admitted");
            }
            MaintArm::ShiftTranslation => {
                let n = self.maint.bodies.len();
                self.maint.prev.clear();
                if self.phase {
                    self.maint.bodies.remove(0);
                    self.maint.moved.remove(0);
                    self.maint.prev.extend((1..n).map(|r| r as u32));
                } else {
                    self.maint.bodies.insert(0, sphere(Vec3::new(EXTRA_X, 0.0, 0.0), 0.5));
                    self.maint.bodies[0].inv_mass = 1.0;
                    self.maint.moved.insert(0, false);
                    self.maint.prev.push(NO_PREV_ROW);
                    self.maint.prev.extend((0..n).map(|r| r as u32));
                }
                self.phase = !self.phase;
                let t = self.maint.rows_step();
                assert_eq!(t.delta.translations, 1, "a shift is one translation");
                assert_eq!(t.delta.patches, 0, "a shift is monotone: nothing diverted");
                assert_eq!(t.delta.evictions, 0);
                assert_eq!(t.delta.static_rebuilds, 0);
                assert_eq!(t.delta.members, m);
                t
            }
            MaintArm::Compaction => {
                let h = self.m.div_ceil(2);
                for r in 0..h - 1 {
                    self.maint.teleport(r);
                }
                let t = self.maint.identity_step();
                assert_eq!(t.delta.evictions, h as u64 - 1);
                assert_eq!(t.delta.static_rebuilds, 0, "dead below live: no compaction yet");
                self.maint.teleport(h - 1);
                let timed = self.maint.identity_step();
                assert_eq!(timed.delta.evictions, 1, "one more eviction");
                assert_eq!(timed.delta.static_rebuilds, 1, "dead reaches live: the compaction");
                assert_eq!(timed.delta.members, m - h as u64);
                for r in 0..h {
                    self.maint.teleport(r);
                }
                let mut back = false;
                for _ in 0..MAINT_STEP_CAP {
                    let t = self.maint.identity_step();
                    if t.delta.members == m {
                        back = true;
                        break;
                    }
                }
                assert!(back, "construction: the teleported members were not re-admitted");
                timed
            }
            MaintArm::HighJumper => {
                let n = self.maint.bodies.len();
                let j = self.jumper;
                self.maint.prev.clear();
                if self.phase {
                    let mover = self.maint.bodies.pop().expect("construction: n >= 1");
                    self.maint.bodies.insert(j, mover);
                    self.maint.prev.extend((0..j).map(|r| r as u32));
                    self.maint.prev.push((n - 1) as u32);
                    self.maint.prev.extend((j..n - 1).map(|r| r as u32));
                } else {
                    let mover = self.maint.bodies.remove(j);
                    self.maint.bodies.push(mover);
                    self.maint.prev.extend((0..j).map(|r| r as u32));
                    self.maint.prev.extend((j + 1..n).map(|r| r as u32));
                    self.maint.prev.push(j as u32);
                }
                self.phase = !self.phase;
                let t = self.maint.rows_step();
                assert_eq!(t.delta.translations, 1, "a migration is one translation");
                assert!(t.delta.patches >= 2, "the jumper's entries are diverted (ruling W1)");
                assert_eq!(t.delta.evictions, 0, "a translation evicts nothing");
                assert_eq!(t.delta.static_rebuilds, 0, "no rebuild on a row change");
                assert_eq!(t.delta.members, m);
                t
            }
        }
    }
}

/// G4 maintenance: the arms of the module docs' table at m ∈ {1 240, 10k, 100k}.
fn bench_g4_maintenance(c: &mut Criterion) {
    let mut group = c.benchmark_group("bp_g4_maintenance");
    group.sampling_mode(SamplingMode::Flat).sample_size(10).warm_up_time(Duration::from_secs(1));
    for &m in &G4_MEMBERS {
        for arm in MaintArm::ALL {
            let mut cycle = MaintCycle::new(arm, m);
            if arm == MaintArm::HighJumper {
                // Ruling W1's premise: the mover is the min endpoint of at least two entries.
                let j = cycle.jumper as u32;
                let higher = cycle
                    .maint
                    .out
                    .pairs()
                    .iter()
                    .filter(|&&(a, b)| a == BodyIndex(j) && b.0 > j)
                    .count();
                assert!(higher >= 2, "high_jumper/{m}: row {j} has {higher} higher-row partners, needs >= 2");
                eprintln!("bp_g4_maintenance/high_jumper/{m}: the mover (row {j}) has {higher} higher-row partners");
            }
            let receipt = cycle.run();
            cycle.maint.check_oracle(&format!("bp_g4_maintenance/{}/{m}", arm.name()));
            let d = receipt.delta;
            eprintln!(
                "bp_g4_maintenance/{}/{m}: timed step: evictions +{} translations +{} patches +{} \
                 static_rebuilds +{}; members {}; pairs {} (oracle ok)",
                arm.name(),
                d.evictions,
                d.translations,
                d.patches,
                d.static_rebuilds,
                d.members,
                cycle.maint.out.pairs().len(),
            );
            group.bench_function(BenchmarkId::new(arm.name(), m), |b| {
                b.iter_custom(|iters| {
                    let mut total = Duration::ZERO;
                    for _ in 0..iters {
                        total += cycle.run().wall;
                    }
                    total
                });
            });
        }
    }
    group.finish();
}

criterion_group!(
    benches,
    bench_broadphase,
    bench_disparity,
    bench_parallel,
    bench_g4_pairs,
    bench_g4_maintenance
);
criterion_main!(benches);
