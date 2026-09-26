//! G0 and G1 of the tree broadphase design, plus its unit tests.
//!
//! * **G0** — the 8-wide leaf test equals the scalar AllPairs expression lane for lane (and the
//!   AVX2 arm equals the scalar arm) on exact-boundary constructions, coordinates up to ±1e6,
//!   subnormals, ±0, negative radii, ±inf and NaN.
//! * **G1** — the pair set equals [`all_pairs_into`] on every step, the tree path forced
//!   (`brute_max_rows = 0`) except in the brute case: single-step worlds (a property test) and
//!   multi-step scripts over a real `RowIdentity`, driven the way `row_identity.rs`'s tests
//!   drive it, each with its structural counts.
//! * **Unit** — the rent rule, the patch merge (the review's traced high-jumper case, with `a`
//!   pinned), Morton codes over huge ranges, and the consumed mark against a non-injective map.
//! * **G-LL** (C3b, the leaf-list query) — G-LL1: after every tree-path step of every [`Sim`]
//!   scene the query stage is re-run under both kernels and its bytes (the stream and every
//!   row's `(seg, nrev, nfwd)`) compared; G-LL2: the G1 property tests draw the step's kernel,
//!   so the oracle sees both; G-LL4: every leaf-list test's kernel arm equals its scalar arm;
//!   G-LL5: a lowered collection cap takes the fallback, which keeps the bytes; the one-leaf
//!   active tree over a multi-leaf static tree (C3b review, W1); the max row the cut reads and
//!   the kept-count pin, the cut's gate in the default test command (W2); the small-segment
//!   network (F2) against `sort_unstable` on every length it takes.
//!
//! Every test is device-free and heap-light; under Miri the property tests shrink to 16 cases
//! at n ≤ 24 and the kernel is the scalar arm.

use proptest::prelude::*;

use boyko_ecs::ecs::core::component::scratch::ScratchColumn;

use crate::components::ColliderShape;
use crate::math::Vec3;
use crate::resources::{BodyState, ContactPairs};
use crate::row_identity::{NO_ROW, RowIdentity, RowKey, RowRemap};

use super::bvh::{
    Item, LANES, LEAF_MAXROW, LEAF_R, LEAF_ROW, LEAF_X, LEAF_Y, LEAF_Z, NO_LANE_ROW, Node8,
    PackedBvh8,
};
use super::kernel::{
    CandList, LEAF_LIST_CAP, NETWORK_SORT_MAX, QueryBox, box_mask, box_mask_scalar, leaf_mask,
    leaf_mask_above, leaf_mask_above_scalar, leaf_mask_scalar, sort_network, sort_network_scalar,
};
use super::{
    ADMIT_BUILD_RATIO, BroadphaseTree, JUMPER, KIND_EXCLUDED, KIND_WIDE, LeafListCounts, NoHint,
    QueryKernel, SET_NONE, SET_S, SET_Z, SleepHint, TREE_BRUTE_MAX_ROWS, TreeDiag, all_pairs_into,
    classify, mark_jumpers, merge_into_sorted, sphere_bound_feasible,
};
use crate::systems::body_bounding_radius;
use crate::scratch_ids::{TREE_ACTIVE, TREE_SORT_A, TREE_SORT_B, TREE_SS, tree_column_id};

// ── Helpers ──────────────────────────────────────────────────────────────────

/// A body row: a sphere unless `half_extents` is given.
fn sphere(pos: [f32; 3], radius: f32, inv_mass: f32, kinematic: bool) -> BodyState {
    BodyState {
        position: Vec3::new(pos[0], pos[1], pos[2]),
        inv_mass,
        kinematic,
        simulated: inv_mass != 0.0,
        shape: ColliderShape::Sphere { radius },
        ..BodyState::default()
    }
}

fn boxed(pos: [f32; 3], half: [f32; 3], inv_mass: f32) -> BodyState {
    BodyState {
        position: Vec3::new(pos[0], pos[1], pos[2]),
        inv_mass,
        simulated: inv_mass != 0.0,
        shape: ColliderShape::Box { half_extents: Vec3::new(half[0], half[1], half[2]) },
        ..BodyState::default()
    }
}

/// A tree, a row identity, the tree's output and the oracle's, plus the scene the scripts edit.
///
/// The rows are keyed the way the engine keys them (A1b, H-03): a [`RowKey`] is the slot
/// index AND its generation, so a recycled slot never carries the dead body's key.
struct Sim {
    tree: BroadphaseTree,
    rows: RowIdentity,
    out: ContactPairs,
    oracle: ContactPairs,
    bodies: Vec<BodyState>,
    ids: Vec<usize>,
    next_id: usize,
    /// The generation of each slot ever minted, indexed by id. The kernel bumps it when the
    /// slot's entity despawns; a dead slot's key is never pushed, so the harness folds the
    /// bump into [`respawn_at`](Self::respawn_at), the one place the slot's key reappears.
    generation: Vec<u32>,
    /// Ids whose `RigidBody` is flagged added at the next gather.
    fresh: Vec<usize>,
}

impl Sim {
    fn new(bodies: Vec<BodyState>) -> Self {
        let n = bodies.len();
        let mut tree = BroadphaseTree::with_capacity(0);
        tree.set_brute_max_rows(0);
        Self {
            tree,
            rows: RowIdentity::with_capacity(0),
            out: ContactPairs::with_capacity(0),
            oracle: ContactPairs::with_capacity(0),
            bodies,
            ids: (0..n).collect(),
            next_id: n,
            generation: vec![0; n],
            fresh: Vec::new(),
        }
    }

    /// Feeds one gather of the current rows.
    fn gather(&mut self) {
        self.rows.begin_gather();
        {
            let (mut cur, mut add) = self.rows.gather_views();
            for (r, &id) in self.ids.iter().enumerate() {
                cur.push(RowKey::new(id, self.generation[id]));
                if self.fresh.contains(&id) {
                    add.push(r as u32);
                }
            }
        }
        self.rows.finish_gather();
        self.fresh.clear();
    }

    /// One tree step against the oracle, without a gather (direct drive, or a missed gather).
    /// A tree-path step is followed by G-LL1: its query stage, re-run under both kernels, writes
    /// the same bytes.
    fn step_no_gather(&mut self) -> TreeDiag {
        self.tree.step(&self.bodies, &self.rows, &mut self.out);
        self.check_step()
    }

    /// G-LL1 on a tree-path step, then the oracle.
    fn check_step(&mut self) -> TreeDiag {
        let n = self.bodies.len();
        if n > self.tree.brute_max_rows() as usize {
            assert_kernels_agree(&mut self.tree, n);
        }
        self.check_oracle();
        self.tree.diag()
    }

    /// The logical pair set — the stream ⊎ the withheld pairs (L10 C3c, design 04 T3) —
    /// equals all-pairs'.
    fn check_oracle(&mut self) {
        all_pairs_into(&self.bodies, &mut self.oracle);
        assert_eq!(
            self.out.pairs(),
            self.oracle.pairs(),
            "the tree's pair set differs from all-pairs' (n = {})",
            self.bodies.len()
        );
    }

    /// A gather then a step with the sleep hint `hint` (L10 C3c), against the oracle.
    fn step_hinted(&mut self, hint: &BitHint) -> TreeDiag {
        self.gather();
        self.tree.step_hinted(&self.bodies, &self.rows, &mut self.out, hint);
        self.check_step()
    }

    /// Releases the sleeper-set members among `rows` (T4), then checks the oracle again: a
    /// release only moves pairs from the withheld list into the stream.
    fn release(&mut self, rows: &[u32]) -> u32 {
        let released = self.tree.release(&mut self.out, |r| rows.contains(&r));
        self.check_oracle();
        released
    }

    /// A gather then a step against the oracle.
    fn step(&mut self) -> TreeDiag {
        self.gather();
        self.step_no_gather()
    }

    fn pairs(&self) -> usize {
        self.out.pairs().len()
    }

    /// Spawns `body` at `row`, shifting the rows after it (a spawn into an earlier archetype,
    /// or an append at `row == len`).
    fn spawn_at(&mut self, row: usize, body: BodyState) -> usize {
        let id = self.next_id;
        self.next_id += 1;
        debug_assert_eq!(self.generation.len(), id, "harness: one generation per minted id");
        self.generation.push(0);
        self.bodies.insert(row, body);
        self.ids.insert(row, id);
        self.fresh.push(id);
        id
    }

    /// Spawns `body` at `row` under a recycled `id`: the slot at its next generation, so its
    /// key is a new one to the map, as the kernel's despawn-time bump guarantees.
    fn respawn_at(&mut self, row: usize, id: usize, body: BodyState) {
        assert!(
            id < self.next_id && !self.ids.contains(&id),
            "harness: a recycled id is a dead one"
        );
        self.generation[id] = self.generation[id].wrapping_add(1);
        self.bodies.insert(row, body);
        self.ids.insert(row, id);
        self.fresh.push(id);
    }

    /// Despawns `row` by swap-remove: the last row moves into it.
    fn despawn_swap(&mut self, row: usize) -> usize {
        self.bodies.swap_remove(row);
        self.ids.swap_remove(row)
    }

    /// Moves `row` to `to`, shifting the rows between (an archetype migration).
    fn migrate(&mut self, row: usize, to: usize) {
        let b = self.bodies.remove(row);
        let id = self.ids.remove(row);
        self.bodies.insert(to, b);
        self.ids.insert(to, id);
    }
}

/// A static floor, `dynamics` resting boxes in a grid, and three static pillars that overlap
/// the floor and nothing else. Rows: 0 floor, 1..=3 pillars, then the boxes.
fn scene(dynamics: usize) -> Vec<BodyState> {
    let mut v = vec![boxed([0.0, -1.0, 0.0], [40.0, 1.0, 40.0], 0.0)];
    for k in 0..3 {
        v.push(boxed([-30.0 + 8.0 * k as f32, 0.5, -30.0], [0.5, 0.5, 0.5], 0.0));
    }
    for k in 0..dynamics {
        let x = 2.0 * (k % 10) as f32;
        let z = 2.0 * (k / 10) as f32;
        v.push(boxed([x, 0.49, z], [0.5, 0.5, 0.5], 1.0));
    }
    v
}

/// Settles `scene`: three steps, after which the four statics are members.
fn settled(dynamics: usize) -> Sim {
    let mut sim = Sim::new(scene(dynamics));
    let d0 = sim.step();
    assert_eq!(d0.static_rebuilds, 0, "step 0: nothing is located, nothing pending");
    let d1 = sim.step();
    assert_eq!(d1.static_rebuilds, 0, "step 1: rent 4 < 4 + 1 (ratio 1/4)");
    assert_eq!(d1.members, 0);
    let d2 = sim.step();
    assert_eq!(d2.static_rebuilds, 1, "step 2: rent 8 ≥ 5, the statics are admitted");
    assert_eq!(d2.members, 4);
    assert_eq!(d2.evictions, 0);
    assert!(sim.pairs() >= 3 + dynamics, "anti-vacuity: the statics and the boxes touch the floor");
    sim
}

/// The bytes the query stage of the last tree-path step writes: the stream and every row's
/// `(seg, nrev, nfwd)`.
type QueryStage = (Vec<u32>, Vec<(u32, u32, u32)>);

/// Re-runs the query stage of the last tree-path step (`n` rows) under `kernel` and returns its
/// bytes. The trees and the records' bits are the step's and the stage reads nothing else, so
/// the re-run is the step's stage; the selected kernel and the counters are restored.
fn query_stage(tree: &mut BroadphaseTree, n: usize, kernel: QueryKernel) -> QueryStage {
    let (kernel_before, diag_before) = (tree.kernel, tree.diag);
    tree.kernel = kernel;
    tree.query_all(n);
    tree.kernel = kernel_before;
    tree.diag = diag_before;
    let stream = tree.aux.as_read_slice().to_vec();
    let records = tree.rec[usize::from(tree.cur)]
        .as_read_slice()
        .iter()
        .take(n)
        .map(|r| (r.seg, r.nrev, r.nfwd))
        .collect();
    (stream, records)
}

/// G-LL1: the leaf list writes the per-row walk's bytes on the state of the last tree-path step.
///
/// Each mutation below was applied when the gate was written, and this module's tests went RED
/// under it through this check or the oracle: `>=` for `>` in `leaf_mask_above` (a row emits
/// itself), the collection run with the leaf's first row's box in place of the leaf's box, the
/// static tree's one leaf node skipped, the max-row cut flipped to `maxrow < row`, the max row
/// built as the smallest live row, the fallback skipped (G-LL5). Two mutations of the cut keep
/// candidates that emit nothing and so write the same bytes — invisible here BY DESIGN: the cut
/// as `maxrow >= row` (RED only in G-LL4, whose reference definition is the strict cut) and the
/// active prefilter without the cut at all (RED only in the kept-count pin).
fn assert_kernels_agree(tree: &mut BroadphaseTree, n: usize) {
    let leaf_list = query_stage(tree, n, QueryKernel::LeafList);
    let row_walk = query_stage(tree, n, QueryKernel::RowWalk);
    assert_eq!(leaf_list.0, row_walk.0, "G-LL1: the leaf list's stream is the per-row walk's (n = {n})");
    assert_eq!(leaf_list.1, row_walk.1, "G-LL1: the leaf list's records are the per-row walk's (n = {n})");
}

/// The `SS` list holds exactly the floor–pillar pairs.
fn assert_ss_is_floor_pillars(sim: &Sim, floor: u32, pillars: &[u32]) {
    let mut want: Vec<u64> = pillars
        .iter()
        .map(|&p| (u64::from(floor.min(p)) << 32) | u64::from(floor.max(p)))
        .collect();
    want.sort_unstable();
    assert_eq!(sim.tree.ss.as_read_slice(), want.as_slice(), "SS holds the floor–pillar pairs");
}

// ── G0: the kernel ────────────────────────────────────────────────────────────

/// Adversarial `f32`s: exact-boundary material, large coordinates, subnormals, signed zeros,
/// negative values, infinities and NaN.
fn any_f32() -> BoxedStrategy<f32> {
    prop_oneof![
        8 => -1.0e6f32..1.0e6f32,
        4 => -4.0f32..4.0f32,
        2 => (1u32..64u32).prop_map(f32::from_bits), // subnormal
        1 => Just(0.0f32),
        1 => Just(-0.0f32),
        1 => Just(f32::INFINITY),
        1 => Just(f32::NEG_INFINITY),
        1 => Just(f32::NAN),
        1 => Just(f32::MIN_POSITIVE),
        1 => (0u32..24u32).prop_map(|k| 1.0f32 + f32::from_bits((1.0f32).to_bits() + k)),
    ]
    .boxed()
}

fn leaf_node(lanes: &[(f32, f32, f32, f32, u32); LANES]) -> Node8 {
    let mut node = Node8 { p: [[0.0; 8]; 6] };
    for (k, &(x, y, z, r, row)) in lanes.iter().enumerate() {
        node.p[LEAF_X][k] = x;
        node.p[LEAF_Y][k] = y;
        node.p[LEAF_Z][k] = z;
        node.p[LEAF_R][k] = r;
        node.p[LEAF_ROW][k] = f32::from_bits(row);
    }
    node
}

/// Unit directions of the boundary constructions: one axis, then four with every component
/// non-zero (Pythagorean triples and quadruples, exact in `f32`).
const DIRECTIONS: [[f32; 3]; 5] = [
    [1.0, 0.0, 0.0],
    [0.6, 0.48, 0.64],
    [0.36, 0.48, 0.8],
    [0.2, 0.4, 0.894_427_2],
    [0.577_350_3, 0.577_350_3, 0.577_350_3],
];

#[cfg(not(miri))]
const G0_CASES: u32 = 8192;
#[cfg(miri)]
const G0_CASES: u32 = 16;

proptest! {
    // `failure_persistence: None`: no regression file is read or written, so the file system is
    // never touched and the tests run under Miri's default isolation (the crate's convention,
    // `row_identity.rs`, `narrowphase/dispatch.rs`).
    #![proptest_config(ProptestConfig {
        cases: G0_CASES,
        failure_persistence: None,
        ..ProptestConfig::default()
    })]

    /// G0: every lane of the leaf test is the AllPairs expression in both argument orders, and
    /// the compiled kernel arm equals the scalar arm bit for bit.
    #[test]
    fn g0_leaf_test_is_the_all_pairs_expression(
        lanes in prop::array::uniform8((any_f32(), any_f32(), any_f32(), any_f32())),
        q in (any_f32(), any_f32(), any_f32(), any_f32()),
        boundary in (0.001f32..1000.0f32, -6i32..6i32, 0usize..DIRECTIONS.len(), -6i32..6i32),
    ) {
        let mut rows = [(0.0f32, 0.0f32, 0.0f32, 0.0f32, 0u32); LANES];
        for (k, &(x, y, z, r)) in lanes.iter().enumerate() {
            rows[k] = (x, y, z, r, k as u32);
        }
        // Lanes 6 and 7 are exact-boundary constructions: `|d| = |r_q + r| ± k ulp` along a
        // direction, one axis-aligned and one with every component non-zero, so the last
        // addition of the sum of squares rounds and a fused multiply-add there is visible.
        let (rq_mag, k6, dir, k7) = boundary;
        let (qx, qy, qz, qr) = q;
        for (lane, (kk, dir, scale)) in [(6usize, (k6, dir, 0.37f32)), (7, (k7, 0, 0.61))] {
            let r = rq_mag * scale;
            let sum = qr + r;
            let mut d = sum.abs();
            for _ in 0..kk.abs() {
                d = if kk > 0 { d.next_up() } else { d.next_down() };
            }
            let u = DIRECTIONS[dir];
            rows[lane] = (qx + d * u[0], qy + d * u[1], qz + d * u[2], r, lane as u32);
        }
        let node = leaf_node(&rows);

        let mask = leaf_mask(&node, qx, qy, qz, qr);
        prop_assert_eq!(mask, leaf_mask_scalar(&node, qx, qy, qz, qr), "kernel arm ≠ scalar arm");
        for (kk, &(x, y, z, r, _)) in rows.iter().enumerate() {
            let hit = mask >> kk & 1 == 1;
            let a = sphere_bound_feasible(Vec3::new(x, y, z), r, Vec3::new(qx, qy, qz), qr);
            let b = sphere_bound_feasible(Vec3::new(qx, qy, qz), qr, Vec3::new(x, y, z), r);
            prop_assert_eq!(hit, a, "lane {} ≠ all-pairs (lane first)", kk);
            prop_assert_eq!(hit, b, "lane {} ≠ all-pairs (query first)", kk);
        }
    }

    /// G0: the box test's kernel arm equals the scalar arm.
    #[test]
    fn g0_box_test_arms_agree(
        lo in prop::array::uniform8((any_f32(), any_f32(), any_f32())),
        hi in prop::array::uniform8((any_f32(), any_f32(), any_f32())),
        q in (any_f32(), any_f32(), any_f32(), any_f32()),
    ) {
        let mut node = Node8 { p: [[0.0; 8]; 6] };
        for k in 0..LANES {
            node.p[0][k] = lo[k].0;
            node.p[1][k] = lo[k].1;
            node.p[2][k] = lo[k].2;
            node.p[3][k] = hi[k].0;
            node.p[4][k] = hi[k].1;
            node.p[5][k] = hi[k].2;
        }
        let qb = QueryBox { lo: [q.0, q.1, q.2], hi: [q.0 + q.3.abs(), q.1 + q.3.abs(), q.2 + q.3.abs()] };
        prop_assert_eq!(box_mask(&node, &qb), box_mask_scalar(&node, &qb));
    }
}

// ── G1: single-step worlds ────────────────────────────────────────────────────

/// One body's generator output.
#[derive(Clone, Copy, Debug)]
struct Spec {
    pos: [f32; 3],
    r_mag: f32,
    neg: bool,
    /// 0 dynamic, 1 static, 2 kinematic.
    class: u8,
    /// 0 normal, 1 duplicate of the previous position, 2..=5 Excluded variants, 6..=9 Wide.
    kind: u8,
}

fn spec() -> BoxedStrategy<Spec> {
    (
        prop::array::uniform3(-10.0f32..10.0f32),
        (-3.0f32..3.0f32).prop_map(|e| 10f32.powf(e)),
        any::<bool>(),
        prop_oneof![7 => Just(0u8), 2 => Just(1u8), 1 => Just(2u8)],
        prop_oneof![
            90 => Just(0u8),
            2 => Just(1u8),
            1 => Just(2u8), 1 => Just(3u8), 1 => Just(4u8), 1 => Just(5u8),
            1 => Just(6u8), 1 => Just(7u8), 1 => Just(8u8), 1 => Just(9u8),
        ],
    )
        .prop_map(|(pos, r_mag, neg, class, kind)| Spec { pos, r_mag, neg, class, kind })
        .boxed()
}

/// Builds the rows of a single-step world: `sign` is 0 all-positive, 1 all-negative, 2 mixed.
fn world(specs: &[Spec], sign: u8) -> Vec<BodyState> {
    let mut v = Vec::with_capacity(specs.len() + 3);
    let mut prev = [0.0f32; 3];
    for s in specs {
        let mut pos = s.pos;
        let mut r = s.r_mag;
        match s.kind {
            1 => pos = prev,
            2 => pos[0] = f32::NAN,
            3 => r = f32::NAN,
            4 => pos[1] = f32::INFINITY,
            5 => pos[2] = f32::NEG_INFINITY,
            6 => r = f32::INFINITY,
            7 => r = f32::NEG_INFINITY,
            8 => r = 2.0f32.powi(61),
            9 => pos[0] = 2.0f32.powi(61),
            _ => {}
        }
        let negative = match sign {
            0 => false,
            1 => true,
            _ => s.neg,
        };
        if negative {
            r = -r;
        }
        let (inv_mass, kinematic) = match s.class {
            1 => (0.0, false),
            2 => (0.0, true),
            _ => (1.0, false),
        };
        v.push(sphere(pos, r, inv_mass, kinematic));
        prev = pos;
    }
    // The hub: a body with at least 40 partners (every finite row of the ±10 cube).
    v.push(sphere([0.0, 0.0, 0.0], if sign == 1 { -50.0 } else { 50.0 }, 0.0, false));
    // The directed pair r = −1, −1 at distance 1.5 (its bound is (−2)² = 4 ≥ 2.25).
    v.push(sphere([100.0, 0.0, 0.0], -1.0, 1.0, false));
    v.push(sphere([101.5, 0.0, 0.0], -1.0, 1.0, false));
    v
}

#[cfg(not(miri))]
const G1_CASES: u32 = 256;
#[cfg(miri)]
const G1_CASES: u32 = 16;
#[cfg(not(miri))]
const G1_MAX_N: usize = 297;
#[cfg(miri)]
const G1_MAX_N: usize = 21;

proptest! {
    #![proptest_config(ProptestConfig {
        cases: G1_CASES,
        failure_persistence: None,
        ..ProptestConfig::default()
    })]

    /// G1: a single-step world, three steps under direct drive (the statics are admitted at
    /// the third), equals all-pairs on each.
    #[test]
    fn g1_single_step_worlds_equal_all_pairs(
        specs in prop::collection::vec(spec(), 1..G1_MAX_N),
        sign in 0u8..3u8,
        row_walk in any::<bool>(),
    ) {
        let bodies = world(&specs, sign);
        let mut sim = Sim::new(bodies);
        // G-LL2: the oracle checks the step's own kernel, either one.
        sim.tree.set_query_kernel(if row_walk { QueryKernel::RowWalk } else { QueryKernel::LeafList });
        for _ in 0..3 {
            sim.step_no_gather();
        }
        let hub = sim.bodies.len() - 3;
        let partners = sim.out.pairs().iter().filter(|(a, b)| a.0 as usize == hub || b.0 as usize == hub).count();
        // Every finite body of the ±10 cube whose radius keeps the hub's bound clear of the
        // cube's diagonal (17.4): |r| < 30 gives |bound| > 20, |r| > 70 gives |bound| > 20.
        let reached = sim
            .bodies
            .iter()
            .take(hub)
            .filter(|b| {
                let r = body_bounding_radius(b);
                b.position.x.is_finite()
                    && b.position.y.is_finite()
                    && b.position.z.is_finite()
                    && b.position.x.abs() < 11.0
                    && r.is_finite()
                    && (r.abs() < 30.0 || r.abs() > 70.0)
            })
            .count();
        prop_assert!(partners >= reached, "the hub pairs with every reachable body of the cube");
        let d = sim.tree.diag();
        let kinds: Vec<u32> = sim
            .bodies
            .iter()
            .map(|b| classify(b.position.x, b.position.y, b.position.z, body_bounding_radius(b)))
            .collect();
        let wide = kinds.iter().filter(|&&k| k == KIND_WIDE).count() as u64;
        let excluded = kinds.iter().filter(|&&k| k == KIND_EXCLUDED).count() as u64;
        prop_assert_eq!(d.wide_rows, 3 * wide, "every Wide row is counted on each of the 3 steps");
        prop_assert_eq!(d.excluded_rows, 3 * excluded);
    }

    /// G1, the brute case: at or below `TREE_BRUTE_MAX_ROWS` the Tree runs the all-pairs loop.
    #[test]
    fn g1_brute_case_below_the_threshold(
        specs in prop::collection::vec(spec(), 1..(TREE_BRUTE_MAX_ROWS as usize - 3).min(G1_MAX_N)),
        sign in 0u8..3u8,
    ) {
        let bodies = world(&specs, sign);
        let mut sim = Sim::new(bodies);
        sim.tree.set_brute_max_rows(TREE_BRUTE_MAX_ROWS);
        prop_assert!(sim.bodies.len() <= TREE_BRUTE_MAX_ROWS as usize);
        for _ in 0..2 {
            sim.step_no_gather();
        }
        prop_assert_eq!(sim.tree.diag().static_rebuilds, 0, "the brute path touches no state");
    }
}

/// G1 boundary cases: touching pairs whose exact test passes only through rounding, at scales
/// from 2^-20 to 2^20 and 1e6 offsets, both radius signs. A cull whose slack is below the
/// rounding need (mutation M1) or which uses `r` in place of `|r|` (M2) misses one.
#[test]
fn g1_boundary_pairs_are_found() {
    let mut bodies = Vec::new();
    // `r_a = 2^k`, `r_b = 3·2^(k−25)`: `fl(r_a + r_b)` rounds UP to `2^k(1 + 2^-23)`, so the
    // pair at `d = 2^k(1 + 2^-23)` passes although `d > r_a + r_b` exactly.
    for k in -20..=20 {
        let ra = 2.0f32.powi(k);
        let rb = 3.0 * 2.0f32.powi(k - 25);
        let d = ra * (1.0 + 2.0f32.powi(-23));
        for (sa, sb) in [(1.0, 1.0), (-1.0, -1.0)] {
            let base = 4.0 * bodies.len() as f32 * 1.0e3;
            bodies.push(sphere([base, 0.0, 0.0], sa * ra, 0.0, false));
            bodies.push(sphere([base + d, 0.0, 0.0], sb * rb, 1.0, false));
            bodies.push(sphere([base, 7.0e5, 0.0], sa * ra, 0.0, false));
            bodies.push(sphere([base, 7.0e5 - d, 0.0], sb * rb, 1.0, false));
        }
    }
    // Exactly touching at unit scale, on the diagonal, and with the directed pair.
    bodies.push(sphere([-5.0e5, 0.0, 0.0], 1.0, 0.0, false));
    bodies.push(sphere([-5.0e5 + 2.0, 0.0, 0.0], 1.0, 1.0, false));
    bodies.push(sphere([-6.0e5, 0.0, 0.0], -1.0, 1.0, false));
    bodies.push(sphere([-6.0e5 + 1.5, 0.0, 0.0], -1.0, 1.0, false));
    let mut sim = Sim::new(bodies);
    for _ in 0..3 {
        sim.step_no_gather();
    }
    assert!(sim.pairs() >= 2 * 41 * 2 + 2, "anti-vacuity: the touching pairs are pairs");
    assert!(sim.tree.diag().members >= 41 * 2, "the statics were admitted, so the S tree was queried");
}

/// G1 sub-normal pairs (the review of C1, W1): rows at `|p| ≤ 2^-59` with `|r| ≤ 2^-76`, whose
/// `bound²` and squared separations all round to `0`, so the exact test accepts separations up
/// to `2^-75` per axis that no relative slack covers. Two groups — one at the origin, where the
/// relative slack is `|r|·2^-20`, one at `2^-60·(1, 1, 1)`, where it is `2^-80` per side — each
/// of 72 dynamic rows offset from the base position by `2^-80 ..= 2^-75` along each axis and
/// sign, with the radii `2^-90`, `-2^-90`, `0` and `2^-76` (every `bound²` rounds to `0`), then
/// 24 static rows at the base position (three leaves of base rows alone, whose boxes are the
/// base box). The offsets precede the base rows, so an offset owns its pairs with them: on the
/// active tree (steps 0 and 1) its query must reach a base-row leaf, and on the static tree
/// (step 2, the base rows admitted) it is the Q row querying S. With an absolute slack of
/// `2^-126` the cull misses every offset above `2^-79` at the origin and above `2^-78` at
/// `2^-60`, and the offset–offset pairs along different axes besides.
#[test]
fn g1_subnormal_pairs_are_found() {
    const R: f32 = 1.0 / (1u128 << 90) as f32;
    let mut bodies = Vec::new();
    let mut offsets = Vec::new();
    for base in [0.0f32, 1.0 / (1u64 << 60) as f32] {
        for k in 75..=80 {
            let sep = 1.0 / (1u128 << k) as f32;
            for axis in 0..3 {
                for (sign, r) in [(1.0f32, R), (-1.0, -R), (1.0, 0.0), (-1.0, 1.0 / (1u128 << 76) as f32)] {
                    let mut pos = [base; 3];
                    pos[axis] = base + sign * sep;
                    assert_ne!(pos[axis], base, "2^-{k} is representable at 2^-60");
                    offsets.push((base, pos, r));
                    bodies.push(sphere(pos, r, 1.0, false));
                }
            }
        }
        for _ in 0..24 {
            bodies.push(sphere([base; 3], R, 0.0, false));
        }
    }
    let n = bodies.len();
    let accepted = offsets
        .iter()
        .filter(|&&(base, pos, r)| {
            sphere_bound_feasible(Vec3::new(base, base, base), R, Vec3::new(pos[0], pos[1], pos[2]), r)
        })
        .count();
    assert_eq!(accepted, offsets.len(), "every offset is within 2^-75 per axis: the exact test accepts it");

    let mut sim = Sim::new(bodies);
    for step in 0..3 {
        sim.step_no_gather();
        let active = sim.tree.active.leaves();
        assert!(active > LANES as u32, "step {step}: the active tree has internal nodes, so the cull ran");
        assert!(sim.pairs() >= 24 * accepted, "step {step}: every offset pairs with its 24 base rows");
    }
    let d = sim.tree.diag();
    assert_eq!(d.members, 48, "the base rows were admitted, so the static tree was queried");
    assert_eq!(d.wide_rows + d.excluded_rows, 0, "every row is Normal (n = {n})");
}

/// G1: a Wide row (`r = +inf`) pairs with an Excluded row (`±inf` position) — the Wide loop
/// must not skip Excluded rows (mutation M10).
#[test]
fn g1_wide_row_pairs_with_excluded_rows() {
    let bodies = vec![
        sphere([0.0, 0.0, 0.0], 1.0, 1.0, false),
        sphere([f32::INFINITY, 0.0, 0.0], 1.0, 1.0, false),
        sphere([0.0, f32::NEG_INFINITY, 0.0], 1.0, 0.0, false),
        sphere([3.0, 0.0, 0.0], f32::INFINITY, 1.0, false),
        sphere([f32::NAN, 0.0, 0.0], 1.0, 1.0, false),
        sphere([5.0, 5.0, 5.0], f32::NEG_INFINITY, 1.0, false),
    ];
    let mut sim = Sim::new(bodies);
    for _ in 0..3 {
        sim.step_no_gather();
    }
    let d = sim.tree.diag();
    assert_eq!(d.wide_rows, 3 * 2);
    assert_eq!(d.excluded_rows, 3 * 3);
    assert!(
        sim.out.pairs().contains(&(crate::manifold::BodyIndex(1), crate::manifold::BodyIndex(3))),
        "the +inf-radius row pairs with the +inf-position row: {:?}",
        sim.out.pairs()
    );
}

// ── G1: multi-step scripts over a real RowIdentity ───────────────────────────

#[test]
fn script_append_spawn_translates_without_patches() {
    let mut sim = settled(30);
    let before = sim.tree.diag();
    sim.spawn_at(sim.bodies.len(), boxed([5.0, 3.0, 5.0], [0.5; 3], 1.0));
    let d = sim.step();
    assert_eq!(d.translations, before.translations + 1, "a Rows step with members translates");
    assert_eq!(d.patches, before.patches, "an append moves no member");
    assert_eq!(d.evictions, before.evictions);
    assert_eq!(d.members, 4);
    assert_eq!(sim.step().translations, d.translations, "the next step is Identity");
}

#[test]
fn script_shift_spawn_into_earlier_archetype() {
    let mut sim = settled(30);
    let before = sim.tree.diag();
    sim.spawn_at(0, boxed([5.0, 3.0, 5.0], [0.5; 3], 1.0));
    let d = sim.step();
    assert_eq!(d.translations, before.translations + 1);
    assert_eq!(d.patches, 0, "a shift is monotone: nothing is diverted");
    assert_eq!(d.evictions, 0);
    assert_eq!(d.members, 4);
    assert_ss_is_floor_pillars(&sim, 1, &[2, 3, 4]);
    // A static spawned at the front is not pending on its spawn step (no record), then pends
    // three steps: rent 1, 2 < 1 + (4 + 1) / 4 = 2.25, rent 3 admits.
    sim.spawn_at(0, boxed([20.0, 0.5, 20.0], [0.5; 3], 0.0));
    sim.step();
    sim.step();
    let d = sim.step();
    assert_eq!(d.members, 4, "rent 2 < 2.25: still pending");
    let d = sim.step();
    assert_eq!(d.static_rebuilds, 2, "the new static is admitted incrementally");
    assert_eq!(d.members, 5);
}

#[test]
fn script_swap_remove_despawn_of_a_member_and_of_a_non_member() {
    let mut sim = settled(30);
    // A member (pillar row 2): it vanishes, the last row (a box) lands in row 2.
    sim.despawn_swap(2);
    let d = sim.step();
    assert_eq!(d.evictions, 1, "the despawned member vanished");
    assert_eq!(d.members, 3);
    assert_eq!(d.static_rebuilds, 1, "no rebuild on a despawn");
    assert_ss_is_floor_pillars(&sim, 0, &[1, 3]);
    // A non-member (a box in the middle): the last row moves, no member is touched.
    let d0 = sim.tree.diag();
    sim.despawn_swap(10);
    let d = sim.step();
    assert_eq!(d.evictions, d0.evictions);
    assert_eq!(d.members, 3);
    assert_eq!(d.translations, d0.translations + 1);
}

#[test]
fn script_migration_high_jumper_diverts_only_its_entries() {
    let mut sim = settled(30);
    let n = sim.bodies.len();
    // Pillar row 1 moves to the last row: its one entry `(0,1)` becomes `(0, n−1)`, a jumper.
    sim.migrate(1, n - 1);
    let d = sim.step();
    assert_eq!(d.patches, 1, "exactly the jumper's entry is diverted (ruling W1)");
    assert_eq!(d.evictions, 0, "a translation evicts nothing");
    assert_eq!(d.members, 4);
    assert_eq!(d.static_rebuilds, 1);
    assert_ss_is_floor_pillars(&sim, 0, &[1, 2, n as u32 - 1]);
}

#[test]
fn script_migration_low_jumper_diverts_only_its_entries() {
    let mut sim = settled(30);
    // Pillar row 3 moves to row 1: rows 1, 2 shift up; `(0,3)` becomes `(0,1)`, out of order.
    sim.migrate(3, 1);
    let d = sim.step();
    assert_eq!(d.patches, 1);
    assert_eq!(d.evictions, 0);
    assert_eq!(d.members, 4);
    assert_ss_is_floor_pillars(&sim, 0, &[1, 2, 3]);
}

#[test]
fn script_burst_of_32_shifts_monotonically() {
    let mut sim = settled(30);
    for k in 0..32 {
        sim.spawn_at(0, boxed([-20.0 + k as f32, 5.0, 20.0], [0.4; 3], 1.0));
    }
    let d = sim.step();
    assert_eq!(d.patches, 0);
    assert_eq!(d.evictions, 0);
    assert_eq!(d.members, 4);
    assert_ss_is_floor_pillars(&sim, 32, &[33, 34, 35]);
}

#[test]
fn script_recycled_id_respawned_at_the_same_pose() {
    let mut sim = settled(30);
    let pose = sim.bodies[2];
    let id = sim.ids[2];
    sim.despawn_swap(2);
    let d = sim.step();
    assert_eq!(d.evictions, 1);
    assert_eq!(d.members, 3);
    // The same slot at a bumped generation, the same pose, back in row 2, flagged added: a
    // new body to the map by its key alone (A1b), and by the flag besides.
    sim.respawn_at(2, id, pose);
    let d = sim.step();
    assert_eq!(d.members, 3, "a fresh row is not a member on its first step");
    sim.step();
    let d = sim.step();
    assert_eq!(d.members, 4, "still for two steps: admitted");
    assert_eq!(d.static_rebuilds, 2);
}

#[test]
fn script_missed_gather_resets_the_locator() {
    let mut sim = settled(30);
    let before = sim.tree.diag();
    // Two gathers, one step: the cursor is two gathers behind.
    sim.gather();
    sim.bodies[1].position.x += 1.0; // a member moved across the missed gather
    let d = sim.step();
    assert_eq!(d.locator_resets, before.locator_resets + 1);
    assert_eq!(d.evictions, before.evictions + 1, "the moved member is evicted by its bits");
    assert_eq!(d.members, 3);
    assert_eq!(d.translations, before.translations, "a Reset translates nothing");
}

#[test]
fn script_direct_drive_grows_and_shrinks() {
    let mut sim = Sim::new(scene(20));
    for _ in 0..3 {
        sim.step_no_gather();
    }
    assert_eq!(sim.tree.diag().members, 4);
    // Grow: five more boxes and a static at the tail.
    for k in 0..5 {
        sim.bodies.push(boxed([-10.0, 0.49, 10.0 + 2.0 * k as f32], [0.5; 3], 1.0));
    }
    sim.bodies.push(boxed([30.0, 0.5, 30.0], [0.5; 3], 0.0));
    sim.step_no_gather();
    sim.step_no_gather();
    let d = sim.step_no_gather();
    assert_eq!(d.members, 4, "rent 2 < 1 + (4 + 1) / 4: still pending");
    let d = sim.step_no_gather();
    assert_eq!(d.members, 5, "the tail static was admitted at rent 3");
    // Shrink: the tail, member included, is removed.
    let keep = sim.bodies.len() - 6;
    sim.bodies.truncate(keep);
    let d = sim.step_no_gather();
    assert_eq!(d.members, 4);
    assert_eq!(d.evictions, 1, "the tail member vanished");
    assert_eq!(d.locator_resets, 0, "direct drive never resets");
}

#[test]
fn script_teleported_and_reshaped_statics_are_evicted_then_readmitted() {
    let mut sim = settled(30);
    sim.bodies[1].position.x += 3.0;
    let d = sim.step();
    assert_eq!(d.evictions, 1);
    assert_eq!(d.members, 3);
    sim.bodies[2].shape = ColliderShape::Box { half_extents: Vec3::new(0.7, 0.5, 0.5) };
    let d = sim.step();
    assert_eq!(d.evictions, 2);
    assert_eq!(d.members, 2);
    // Both are still again: pending for two steps, then admitted together.
    sim.step();
    let d = sim.step();
    assert_eq!(d.members, 4);
    assert_eq!(d.static_rebuilds, 2);
}

#[test]
fn script_static_rewritten_every_step_is_never_admitted() {
    let mut bodies = scene(10);
    bodies.truncate(1); // the floor alone as a static
    bodies.extend(scene(10).into_iter().skip(4));
    let mut sim = Sim::new(bodies);
    for k in 0..10 {
        // Scene sync rewrites the pose every step: one ulp of jitter.
        sim.bodies[0].position.x = if k % 2 == 0 { 0.0 } else { f32::MIN_POSITIVE };
        let d = sim.step();
        assert_eq!(d.static_rebuilds, 0, "a static that is never still is never admitted");
        assert_eq!(d.members, 0);
    }
}

#[test]
fn script_kinematic_toggle_and_class_flips() {
    let mut sim = settled(30);
    sim.bodies[1].kinematic = true;
    let d = sim.step();
    assert_eq!(d.evictions, 1, "a kinematic no longer fits S");
    assert_eq!(d.members, 3);
    sim.bodies[1].kinematic = false;
    sim.step();
    sim.step();
    assert_eq!(sim.tree.diag().members, 4, "kinematic off: readmitted");
    // Static → dynamic.
    sim.bodies[2].inv_mass = 1.0;
    let d = sim.step();
    assert_eq!(d.evictions, 2);
    assert_eq!(d.members, 3);
    // Dynamic → static: a resting box becomes static, and is admitted once still.
    sim.bodies[10].inv_mass = 0.0;
    sim.step();
    sim.step();
    let d = sim.step();
    assert_eq!(d.members, 4);
}

// ── Unit tests ────────────────────────────────────────────────────────────────

/// The rent rule at ratio 1/4: one pending row against an empty set is admitted on its second
/// pending step (rent 1 < 1.25, rent 2 ≥ 1.25); 64 pending rows against 1,240 members after 6.
#[test]
fn rent_rule_arithmetic() {
    let (num, den) = ADMIT_BUILD_RATIO;
    let admit = |rent: u64, pending: u64, members: u64| rent * den >= pending * den + num * (members + pending);
    assert!(!admit(1, 1, 0));
    assert!(admit(2, 1, 0));
    let mut rent = 0;
    let mut steps = 0;
    while !admit(rent, 64, 1240) {
        rent += 64;
        steps += 1;
    }
    // rent·4 ≥ 64·4 + (1240 + 64) ⇒ rent ≥ 390 ⇒ ⌈390 / 64⌉ = 7 (D3.5's "≈ 6").
    assert_eq!(steps, 7, "64 rows wait 7 steps at J scale");
}

/// The review's traced high-jumper case: old list `(5,9) (7,8) (7,12) (7,15) (8,9) (8,10) (9,12)`,
/// row 7 moves to 20 and the rows after it shift down by one. Exactly the three entries with
/// endpoint 7 are diverted (`a = 3`), and the merged list is sorted.
#[test]
fn patch_rule_high_jumper_pins_a() {
    // The columns need their layouts registered; alone in a process, nothing else did.
    crate::scratch_ids::register_tree_column_layouts();
    // inv over old rows 0..=21: identity below 7, 7 → 20, 8..=20 → 7..=19, 21 → 21.
    let mut inv: Vec<u32> = (0..22u32).collect();
    inv[7] = 20;
    for (m, v) in inv.iter_mut().enumerate().take(21).skip(8) {
        *v = m as u32 - 1;
    }
    mark_jumpers(&mut inv);
    let jumpers: Vec<usize> = inv.iter().enumerate().filter(|&(_, &v)| v & JUMPER != 0).map(|(m, _)| m).collect();
    assert_eq!(jumpers, vec![7], "the mover is the one jumper");

    let old = [(5u32, 9u32), (7, 8), (7, 12), (7, 15), (8, 9), (8, 10), (9, 12)];
    let mut ss = ScratchColumn::<u64>::new(tree_column_id(TREE_SS), 4096);
    let mut sort_a = ScratchColumn::<u64>::new(tree_column_id(TREE_SORT_A), 4096);
    {
        let mut v = ss.build_view();
        for (a, b) in old {
            v.push((u64::from(a) << 32) | u64::from(b));
        }
    }
    let mut list = ss.build_view();
    let l = list.as_mut_slice();
    let mut w = 0;
    let mut last = 0u64;
    let mut diverted = sort_a.build_view();
    for i in 0..l.len() {
        let (a, b) = ((l[i] >> 32) as usize, (l[i] & 0xffff_ffff) as usize);
        let (na, nb) = (inv[a], inv[b]);
        let jumper = (na | nb) & JUMPER != 0;
        let (na, nb) = (na & !JUMPER, nb & !JUMPER);
        let key = (u64::from(na.min(nb)) << 32) | u64::from(na.max(nb));
        if !jumper && key > last {
            l[w] = key;
            w += 1;
            last = key;
        } else {
            assert!(jumper, "a non-jumper entry is in order");
            diverted.push(key);
        }
    }
    assert_eq!(diverted.len(), 3, "a = 3: the three entries with endpoint 7");
    list.truncate(w);
    let d = diverted.as_mut_slice();
    d.sort_unstable();
    merge_into_sorted(&mut list, d, false);
    let got: Vec<(u32, u32)> = list.as_slice().iter().map(|&k| ((k >> 32) as u32, k as u32)).collect();
    assert_eq!(got, vec![(5, 8), (7, 8), (7, 9), (7, 20), (8, 11), (11, 20), (14, 20)]);
}

/// The patch merge's fallback: a diverted run larger than a quarter of the kept list sorts the
/// whole list; both routes give the same sorted result.
#[test]
fn patch_merge_edge_cases() {
    // The columns need their layouts registered; alone in a process, nothing else did.
    crate::scratch_ids::register_tree_column_layouts();
    for (kept, added) in [
        (vec![], vec![3u64, 1, 2]),
        (vec![1u64, 5, 9], vec![]),
        (vec![2u64, 4, 6, 8], vec![1, 3, 5, 7, 9]),
        (vec![10u64, 20], vec![1, 2, 3, 4, 5, 6, 7, 8, 9]),
    ] {
        for whole in [false, true] {
            let mut col = ScratchColumn::<u64>::new(tree_column_id(TREE_SORT_B), 4096);
            let mut v = col.build_view();
            for &k in &kept {
                v.push(k);
            }
            let mut a = added.clone();
            a.sort_unstable();
            merge_into_sorted(&mut v, &a, whole);
            let mut want = kept.clone();
            want.extend_from_slice(&added);
            want.sort_unstable();
            assert_eq!(v.as_slice(), want.as_slice());
        }
    }
}

/// Jumpers of a block move: 32 rows from the tail to the front (the rest shift up) and the
/// reverse; the block is the jumper set both ways.
#[test]
fn jumpers_of_a_block_move() {
    let n = 1300usize;
    // Tail 1268..1300 → 64..96; rows 64..1268 → 96..1300; rows 0..64 stay.
    let mut inv: Vec<u32> = (0..n)
        .map(|m| match m {
            0..64 => m as u32,
            64..1268 => m as u32 + 32,
            _ => (m - 1268 + 64) as u32,
        })
        .collect();
    mark_jumpers(&mut inv);
    let jumpers = inv.iter().filter(|&&v| v != NO_ROW && v & JUMPER != 0).count();
    assert_eq!(jumpers, 32);
    assert!((1268..1300).all(|m| inv[m] & JUMPER != 0));
    // Front 0..32 → 1268..1300; the rest shift down by 32.
    let mut inv: Vec<u32> =
        (0..n).map(|m| if m < 32 { (m + 1268) as u32 } else { (m - 32) as u32 }).collect();
    mark_jumpers(&mut inv);
    let jumpers = inv.iter().filter(|&&v| v != NO_ROW && v & JUMPER != 0).count();
    assert_eq!(jumpers, 32);
    assert!((0..32).all(|m| inv[m] & JUMPER != 0));
}

/// Morton codes over huge ranges stay finite, in range, and order a line of items.
#[test]
fn morton_codes_over_huge_ranges() {
    // The columns need their layouts registered; alone in a process, nothing else did.
    crate::scratch_ids::register_tree_column_layouts();
    let lo = [-(2.0f32.powi(60)); 3];
    let hi = [2.0f32.powi(60); 3];
    let cells = 1023.0f32;
    let scale = [cells / (hi[0] - lo[0]); 3];
    let a = super::bvh::morton(lo, lo, scale);
    let b = super::bvh::morton(hi, lo, scale);
    let c = super::bvh::morton([0.0; 3], lo, scale);
    assert_eq!(a, 0);
    assert_eq!(b, (1 << 30) - 1);
    assert!(a < c && c < b);
    // A build over items spanning the whole range, with duplicates, then a query.
    let mut tree = PackedBvh8::new(tree_column_id(TREE_ACTIVE), 4096);
    let mut ka = ScratchColumn::<u64>::new(tree_column_id(TREE_SORT_A), 4096);
    let mut kb = ScratchColumn::<u64>::new(tree_column_id(TREE_SORT_B), 4096);
    let items: Vec<Item> = (0..100u32)
        .map(|i| {
            let t = (i % 50) as f32 / 49.0;
            let x = lo[0] + (hi[0] - lo[0]) * t;
            Item::new(x, 0.0, 0.0, 1.0, i)
        })
        .collect();
    tree.build(&items, &mut ka, &mut kb);
    assert_eq!(tree.leaves(), 100);
    let mut hits = Vec::new();
    tree.query(lo[0], 0.0, 0.0, 1.0, |row| hits.push(row));
    hits.sort_unstable();
    assert_eq!(hits, vec![0, 50], "the two items at the lower corner");
    let mut rows: Vec<u32> = (0..100).map(|s| tree.leaf_row(s)).collect();
    rows.sort_unstable();
    assert_eq!(rows, (0..100).collect::<Vec<_>>(), "every item has a leaf slot");
}

/// M9: a hand-made non-injective `Rows` map locates one old record from two rows. The consumed
/// mark makes the second locate fail, so the duplicate row is queried and keeps its pairs.
#[test]
fn consumed_mark_rejects_a_duplicate_locate() {
    // Two statics at one pose (they pair), one more static, and a few boxes.
    let mut bodies = vec![
        boxed([0.0, 0.5, 0.0], [0.5; 3], 0.0),
        boxed([0.0, 0.5, 0.0], [0.5; 3], 0.0),
        boxed([5.0, 0.5, 0.0], [0.5; 3], 0.0),
    ];
    for k in 0..6 {
        bodies.push(boxed([0.0, 1.49 + k as f32, 0.0], [0.5; 3], 1.0));
    }
    let mut sim = Sim::new(bodies);
    for _ in 0..3 {
        sim.step_no_gather();
    }
    assert_eq!(sim.tree.diag().members, 3);
    // rows 0 and 1 both claim old row 0; old row 1 is claimed by nobody.
    let n = sim.bodies.len();
    let mut map: Vec<u32> = (0..n as u32).collect();
    map[1] = 0;
    sim.tree.run(&sim.bodies, RowRemap::Rows(&map), &mut sim.out, &NoHint);
    all_pairs_into(&sim.bodies, &mut sim.oracle);
    assert_eq!(sim.out.pairs(), sim.oracle.pairs(), "the duplicate row keeps its pairs");
    let d = sim.tree.diag();
    assert_eq!(d.members, 2, "row 0 carried old 0; row 1's locate failed; old 1 vanished");
    assert_eq!(d.evictions, 1);
    // And the world goes on: row 1 is still, so it is readmitted.
    sim.step_no_gather();
    sim.step_no_gather();
    assert_eq!(sim.tree.diag().members, 3);
}

/// Compaction: after enough evictions the static tree is rebuilt over its live lanes.
#[test]
fn compaction_after_many_evictions() {
    let mut bodies = Vec::new();
    for k in 0..200 {
        bodies.push(boxed([3.0 * (k % 20) as f32, 0.5, 3.0 * (k / 20) as f32], [0.5; 3], 0.0));
    }
    bodies.push(boxed([0.0, 5.0, 0.0], [0.5; 3], 1.0));
    let mut sim = Sim::new(bodies);
    for _ in 0..3 {
        sim.step_no_gather();
    }
    assert_eq!(sim.tree.diag().members, 200);
    assert_eq!(sim.tree.diag().static_rebuilds, 1);
    // Evict 140 statics by teleporting them: dead 140 ≥ max(live 60, 64) → compaction.
    for k in 0..140 {
        sim.bodies[k].position.y += 100.0;
    }
    let d = sim.step_no_gather();
    assert_eq!(d.evictions, 140);
    assert_eq!(d.members, 60);
    assert_eq!(d.static_rebuilds, 2, "the eviction step compacted the tree");
    assert_eq!(sim.tree.statics.dead(), 0);
}

// ── Tester's additions (C1 review): the production translate, the S set under churn ──────

/// The review's traced high-jumper case driven through the PRODUCTION `translate()` (the unit
/// test above re-implements the keep loop, so it cannot see a defect in the real one): seven
/// statics whose `SS` is exactly `(5,9) (7,8) (7,12) (7,15) (8,9) (8,10) (9,12)`, row 7 migrated
/// to row 20. Exactly the three entries with endpoint 7 are diverted (`a = 3`, ruling W1), the
/// list is the translated list in order, and nothing is evicted or rebuilt.
///
/// RED under: "mark no jumpers" (the kept-in-place `(7,20)` breaks the order, the debug assert
/// or the oracle fires), "patch keeps out-of-order entries" (M11), and a wrong `patches` count.
#[test]
fn script_review_traced_high_jumper_through_translate() {
    // Radius-1 spheres pair at distance ≤ 2. The seven statics sit at 1.8 from each intended
    // partner and ≥ 2.55 from every other static.
    let statics: [(usize, [f32; 3]); 7] = [
        (5, [1.8, 3.6, 0.0]),
        (7, [0.0, 0.0, 0.0]),
        (8, [1.8, 0.0, 0.0]),
        (9, [1.8, 1.8, 0.0]),
        (10, [3.6, 0.0, 0.0]),
        (12, [0.0, 1.8, 0.0]),
        (15, [0.0, 0.0, 1.8]),
    ];
    let n = 22usize;
    let mut bodies: Vec<BodyState> =
        (0..n).map(|k| sphere([100.0 + 3.0 * k as f32, 0.0, 0.0], 0.5, 1.0, false)).collect();
    for &(row, pos) in &statics {
        bodies[row] = sphere(pos, 1.0, 0.0, false);
    }
    let mut sim = Sim::new(bodies);
    for _ in 0..3 {
        sim.step();
    }
    let d = sim.tree.diag();
    assert_eq!(d.members, 7, "the seven statics are admitted at step 2");
    assert_eq!(d.static_rebuilds, 1);
    let key = |a: u32, b: u32| (u64::from(a) << 32) | u64::from(b);
    let old: Vec<u64> = [(5, 9), (7, 8), (7, 12), (7, 15), (8, 9), (8, 10), (9, 12)]
        .iter()
        .map(|&(a, b)| key(a, b))
        .collect();
    assert_eq!(sim.tree.ss.as_read_slice(), old.as_slice(), "SS is the review's old list");

    sim.migrate(7, 20);
    let d = sim.step();
    assert_eq!(d.patches, 3, "a = 3: exactly the three entries with endpoint 7 (ruling W1)");
    assert_eq!(d.evictions, 0, "a translation evicts nothing");
    assert_eq!(d.members, 7);
    assert_eq!(d.static_rebuilds, 1, "no rebuild on a row change");
    assert_eq!(d.translations, 1);
    let want: Vec<u64> = [(5, 8), (7, 8), (7, 9), (7, 20), (8, 11), (11, 20), (14, 20)]
        .iter()
        .map(|&(a, b)| key(a, b))
        .collect();
    assert_eq!(sim.tree.ss.as_read_slice(), want.as_slice(), "SS is the translated list, in order");
    // And the world goes on in place.
    let d = sim.step();
    assert_eq!(d.translations, 1, "the next step is Identity");
    assert_eq!(d.members, 7);
}

/// A swap-remove despawn of a NON-member whose tail row is a member: the tail static lands
/// in the freed row below another member, so it is a low jumper and only its entries are
/// diverted. RED under M11 and under "mark no jumpers".
#[test]
fn script_swap_remove_of_a_non_member_makes_the_tail_member_a_low_jumper() {
    // Rows: 0 A, 1 B (pairs with A), 2..=4 dynamics, 5 D (pairs with C), 6 dynamic,
    // 7 C (pairs with B and D). SS = (0,1) (1,7) (5,7).
    let mut bodies: Vec<BodyState> =
        (0..8).map(|k| sphere([100.0 + 3.0 * k as f32, 0.0, 0.0], 0.5, 1.0, false)).collect();
    bodies[0] = sphere([0.0, 0.0, 0.0], 1.0, 0.0, false);
    bodies[1] = sphere([1.8, 0.0, 0.0], 1.0, 0.0, false);
    bodies[5] = sphere([5.4, 0.0, 0.0], 1.0, 0.0, false);
    bodies[7] = sphere([3.6, 0.0, 0.0], 1.0, 0.0, false);
    let mut sim = Sim::new(bodies);
    for _ in 0..3 {
        sim.step();
    }
    let key = |a: u32, b: u32| (u64::from(a) << 32) | u64::from(b);
    assert_eq!(sim.tree.diag().members, 4);
    assert_eq!(sim.tree.ss.as_read_slice(), &[key(0, 1), key(1, 7), key(5, 7)]);

    // Row 3 (a dynamic) goes; C moves 7 → 3.
    sim.despawn_swap(3);
    let d = sim.step();
    assert_eq!(d.patches, 2, "C's two entries are diverted; (0,1) stays");
    assert_eq!(d.evictions, 0, "no member vanished: the despawned row was not a member");
    assert_eq!(d.members, 4);
    assert_eq!(d.static_rebuilds, 1);
    assert_eq!(sim.tree.ss.as_read_slice(), &[key(0, 1), key(1, 3), key(3, 5)]);
}

/// A swap-remove whose tail member lands ABOVE every other member keeps the monotone chain
/// (the weighted chain includes the moved run), so nothing is diverted.
#[test]
fn script_swap_remove_with_a_monotone_landing_diverts_nothing() {
    // Rows: 0 A, 1 B (pairs A), 2..=6 dynamics, 7 C (pairs B). SS = (0,1) (1,7).
    let mut bodies: Vec<BodyState> =
        (0..8).map(|k| sphere([100.0 + 3.0 * k as f32, 0.0, 0.0], 0.5, 1.0, false)).collect();
    bodies[0] = sphere([0.0, 0.0, 0.0], 1.0, 0.0, false);
    bodies[1] = sphere([1.8, 0.0, 0.0], 1.0, 0.0, false);
    bodies[7] = sphere([3.6, 0.0, 0.0], 1.0, 0.0, false);
    let mut sim = Sim::new(bodies);
    for _ in 0..3 {
        sim.step();
    }
    let key = |a: u32, b: u32| (u64::from(a) << 32) | u64::from(b);
    assert_eq!(sim.tree.ss.as_read_slice(), &[key(0, 1), key(1, 7)]);
    sim.despawn_swap(3);
    let d = sim.step();
    assert_eq!(d.patches, 0, "0 → 0, 1 → 1, 7 → 3 is still increasing: nothing diverted");
    assert_eq!(d.members, 3);
    assert_eq!(sim.tree.ss.as_read_slice(), &[key(0, 1), key(1, 3)]);
}

/// The patch's merge route (`a ≤ kept / 4`): a chain of sixteen statics (15 `SS` entries), the
/// last migrated to the front. One entry is diverted and merged from the end; the list is
/// strictly sorted and the oracle holds.
#[test]
fn script_one_jumper_among_many_kept_entries_takes_the_merge_route() {
    let mut bodies: Vec<BodyState> = (0..16).map(|k| sphere([1.8 * k as f32, 0.0, 0.0], 1.0, 0.0, false)).collect();
    for k in 0..5 {
        bodies.push(sphere([100.0 + 3.0 * k as f32, 0.0, 0.0], 0.5, 1.0, false));
    }
    let mut sim = Sim::new(bodies);
    for _ in 0..3 {
        sim.step();
    }
    assert_eq!(sim.tree.diag().members, 16);
    assert_eq!(sim.tree.ss.as_read_slice().len(), 15, "a chain of 16 has 15 touching pairs");
    sim.migrate(15, 0);
    let d = sim.step();
    assert_eq!(d.patches, 1, "the mover's one entry is diverted; 14 are kept (a ≤ w / 4)");
    assert_eq!(d.evictions, 0);
    assert_eq!(d.members, 16);
    let ss = sim.tree.ss.as_read_slice();
    assert!(ss.windows(2).all(|w| w[0] < w[1]), "SS is strictly sorted after the merge");
    assert_eq!(ss.len(), 15);
    assert_eq!(ss[0], 15u64, "(0,15): the mover at row 0 pairs with old row 14, now 15");
}

/// D3.3's last row: an `N ≤ brute_max_rows` step runs the brute loop, touches no state and
/// does not stamp the cursor, so the next tree step is a `Reset` whose identity locator
/// carries every unmoved member.
#[test]
fn script_brute_crossing_leaves_the_state_and_resets_the_locator() {
    let mut sim = settled(30);
    let before = sim.tree.diag();
    sim.tree.set_brute_max_rows(1 << 20);
    let d = sim.step();
    assert_eq!(d, before, "the brute path changes no counter");
    sim.tree.set_brute_max_rows(0);
    let d = sim.step();
    assert_eq!(d.locator_resets, before.locator_resets + 1, "the unstamped gather is a Reset");
    assert_eq!(d.members, 4, "the identity locator carries the unmoved members");
    assert_eq!(d.evictions, before.evictions);
    assert_eq!(d.static_rebuilds, before.static_rebuilds);
    assert_eq!(d.translations, before.translations);
}

/// A member whose kind leaves Normal (a half-extent past 2^60 makes it Wide) is evicted and
/// served by the exact Wide loop; it is never readmitted while Wide.
#[test]
fn script_member_becoming_wide_is_evicted_and_looped() {
    let mut sim = settled(30);
    let before = sim.tree.diag();
    sim.bodies[1].shape = ColliderShape::Box { half_extents: Vec3::new(2.0f32.powi(61), 0.0, 0.0) };
    let d = sim.step();
    assert_eq!(d.evictions, before.evictions + 1, "a Wide member is evicted");
    assert_eq!(d.members, 3);
    assert_eq!(d.wide_rows, before.wide_rows + 1);
    let n = sim.bodies.len();
    assert!(sim.pairs() >= n - 1, "anti-vacuity: the Wide row reaches every other row");
    let d = sim.step();
    assert_eq!(d.members, 3, "a Wide row is never pending");
    assert_eq!(d.wide_rows, before.wide_rows + 2);
    assert_eq!(d.static_rebuilds, before.static_rebuilds);
}

/// Compaction on a `Rows` step: the translation kills the evicted lanes, and `dead ≥ max(live,
/// 64)` rebuilds the tree in the same maintenance pass.
#[test]
fn script_compaction_on_a_rows_step() {
    let mut bodies = Vec::new();
    for k in 0..200 {
        bodies.push(boxed([3.0 * (k % 20) as f32, 0.5, 3.0 * (k / 20) as f32], [0.5; 3], 0.0));
    }
    bodies.push(boxed([0.0, 5.0, 0.0], [0.5; 3], 1.0));
    let mut sim = Sim::new(bodies);
    for _ in 0..3 {
        sim.step();
    }
    assert_eq!(sim.tree.diag().members, 200);
    for k in 0..140 {
        sim.bodies[k].position.y += 100.0;
    }
    sim.spawn_at(sim.bodies.len(), boxed([0.0, 9.0, 0.0], [0.5; 3], 1.0));
    let d = sim.step();
    assert_eq!(d.translations, 1, "a Rows step with members");
    assert_eq!(d.evictions, 140);
    assert_eq!(d.members, 60);
    assert_eq!(d.static_rebuilds, 2, "compacted on the Rows step");
    assert_eq!(d.patches, 0, "an append is monotone");
    assert_eq!(sim.tree.statics.dead(), 0);
}

/// A pure shift (every carried row displaced by the same amount) has no jumper.
#[test]
fn mark_jumpers_marks_nothing_on_a_pure_shift() {
    let mut inv: Vec<u32> = (0..500u32).map(|m| if m % 3 == 0 { NO_ROW } else { m + 5 }).collect();
    mark_jumpers(&mut inv);
    assert!(inv.iter().all(|&v| v == NO_ROW || v & JUMPER == 0));
}

/// Past `MAX_RUNS` displacement runs every carried row is a jumper (the whole list is sorted).
#[test]
fn mark_jumpers_past_max_runs_marks_every_carried_row() {
    // Alternating displacements: 600 runs of one row each, injective.
    let mut inv: Vec<u32> =
        (0..600u32).map(|m| if m % 2 == 0 { m / 2 } else { 300 + m / 2 }).collect();
    mark_jumpers(&mut inv);
    assert!(inv.iter().all(|&v| v & JUMPER != 0), "every carried row is a jumper");
}

#[cfg(not(miri))]
const JUMPER_CASES: u32 = 512;
#[cfg(miri)]
const JUMPER_CASES: u32 = 8;

proptest! {
    #![proptest_config(ProptestConfig {
        cases: JUMPER_CASES,
        failure_persistence: None,
        ..ProptestConfig::default()
    })]

    /// The invariant `translate()` rests on: the non-jumper rows form a strictly increasing
    /// old → new map, so every entry with two non-jumper endpoints keeps its order.
    #[test]
    fn mark_jumpers_non_jumpers_form_a_strictly_increasing_map(
        n in 1usize..200,
        seed in any::<u64>(),
        carried_pct in 0u32..=100,
    ) {
        // A pseudo-random permutation of 0..n (Fisher–Yates on an xorshift) and a carried mask.
        let mut s = seed | 1;
        let mut next = || {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            s
        };
        let mut perm: Vec<u32> = (0..n as u32).collect();
        for i in (1..n).rev() {
            let j = (next() % (i as u64 + 1)) as usize;
            perm.swap(i, j);
        }
        let mut inv: Vec<u32> = perm
            .iter()
            .map(|&v| if next() % 100 < u64::from(carried_pct) { v } else { NO_ROW })
            .collect();
        mark_jumpers(&mut inv);
        let kept: Vec<u32> = inv.iter().filter(|&&v| v != NO_ROW && v & JUMPER == 0).copied().collect();
        prop_assert!(kept.windows(2).all(|w| w[0] < w[1]), "non-jumpers are increasing: {kept:?}");
        let carried = inv.iter().filter(|&&v| v != NO_ROW).count();
        if carried > 0 {
            prop_assert!(!kept.is_empty(), "with ≤ MAX_RUNS runs, the heaviest chain is non-empty");
        }
    }
}

/// One structural edit of a random churn script.
#[derive(Clone, Copy, Debug)]
enum ChurnOp {
    /// Nothing: an `Identity` step.
    Hold,
    /// An append or a shift spawn; static or dynamic.
    Spawn { at: usize, is_static: bool },
    /// A swap-remove despawn.
    DespawnSwap(usize),
    /// An archetype migration.
    Migrate { from: usize, to: usize },
    /// A member or a box moves.
    Teleport(usize),
    /// A body's collider changes.
    Reshape(usize),
    /// Static ↔ dynamic.
    ClassFlip(usize),
    /// A gather the tree does not see: the next step is a `Reset`.
    MissGather,
    /// The brute path for one step.
    Brute,
}

fn churn_op() -> BoxedStrategy<ChurnOp> {
    prop_oneof![
        2 => Just(ChurnOp::Hold),
        3 => (0usize..64, any::<bool>()).prop_map(|(at, is_static)| ChurnOp::Spawn { at, is_static }),
        3 => (0usize..64).prop_map(ChurnOp::DespawnSwap),
        3 => (0usize..64, 0usize..64).prop_map(|(from, to)| ChurnOp::Migrate { from, to }),
        2 => (0usize..64).prop_map(ChurnOp::Teleport),
        1 => (0usize..64).prop_map(ChurnOp::Reshape),
        2 => (0usize..64).prop_map(ChurnOp::ClassFlip),
        1 => Just(ChurnOp::MissGather),
        1 => Just(ChurnOp::Brute),
    ]
    .boxed()
}

#[cfg(not(miri))]
const CHURN_CASES: u32 = 96;
#[cfg(miri)]
const CHURN_CASES: u32 = 3;

proptest! {
    #![proptest_config(ProptestConfig {
        cases: CHURN_CASES,
        failure_persistence: None,
        ..ProptestConfig::default()
    })]

    /// G1 under random churn over a real `RowIdentity`: spawns, swap-remove despawns,
    /// migrations, teleports, reshapes, class flips, missed gathers and brute steps, in any
    /// order; the tree equals all-pairs on every step, `SS` is strictly sorted, and the static
    /// set never holds more rows than the scene's still statics.
    #[test]
    fn g1_random_churn_scripts_equal_all_pairs(
        ops in prop::collection::vec(churn_op(), 4..24),
        dynamics in 4usize..14,
        row_walk in any::<bool>(),
    ) {
        let mut sim = settled(dynamics);
        // G-LL2: the oracle checks the step's own kernel, either one.
        sim.tree.set_query_kernel(if row_walk { QueryKernel::RowWalk } else { QueryKernel::LeafList });
        for op in ops {
            let n = sim.bodies.len();
            match op {
                ChurnOp::Hold => {}
                ChurnOp::Spawn { at, is_static } => {
                    let at = at % (n + 1);
                    let inv_mass = if is_static { 0.0 } else { 1.0 };
                    let x = -30.0 + 2.0 * (at % 30) as f32;
                    sim.spawn_at(at, boxed([x, 0.49, 2.0 * (at / 30) as f32 + 14.0], [0.5; 3], inv_mass));
                }
                ChurnOp::DespawnSwap(row) => {
                    if n > 2 {
                        sim.despawn_swap(row % n);
                    }
                }
                ChurnOp::Migrate { from, to } => {
                    sim.migrate(from % n, to % n);
                }
                ChurnOp::Teleport(row) => {
                    sim.bodies[row % n].position.x += 3.0;
                }
                ChurnOp::Reshape(row) => {
                    let b = &mut sim.bodies[row % n];
                    b.shape = match b.shape {
                        ColliderShape::Box { half_extents } => {
                            ColliderShape::Box { half_extents: half_extents + Vec3::new(0.25, 0.0, 0.0) }
                        }
                        ColliderShape::Sphere { radius } => ColliderShape::Sphere { radius: radius + 0.25 },
                    };
                }
                ChurnOp::ClassFlip(row) => {
                    let b = &mut sim.bodies[row % n];
                    b.inv_mass = if b.inv_mass == 0.0 { 1.0 } else { 0.0 };
                }
                ChurnOp::MissGather => {
                    sim.gather();
                }
                ChurnOp::Brute => {
                    sim.tree.set_brute_max_rows(1 << 20);
                }
            }
            // `step` asserts the oracle.
            let d = sim.step();
            sim.tree.set_brute_max_rows(0);
            let statics = sim
                .bodies
                .iter()
                .filter(|b| b.inv_mass == 0.0 && !b.kinematic)
                .count() as u64;
            prop_assert!(d.members <= statics, "members {} ≤ statics {}", d.members, statics);
            prop_assert_eq!(u64::from(sim.tree.statics.live()), d.members, "live lanes equal members");
            let ss = sim.tree.ss.as_read_slice();
            prop_assert!(ss.windows(2).all(|w| w[0] < w[1]), "SS strictly sorted");
            prop_assert!(
                ss.iter().all(|&k| (k >> 32) < (k & 0xffff_ffff) && ((k & 0xffff_ffff) as usize) < sim.bodies.len()),
                "every SS key is (min < max) within the rows"
            );
        }
    }
}

// ── Tester's additions (C1 resume): the tree path at its smallest sizes ──────────────────

/// The tree path with one row (`brute_max_rows = 0`): a one-lane active tree, no query hits,
/// no pairs, one queried row. RED under a build that mis-sizes a one-item level or a query
/// that reads past a one-node tree.
#[test]
fn edge_single_row_on_the_tree_path_has_no_pairs() {
    let mut sim = Sim::new(vec![sphere([0.0, 0.0, 0.0], 1.0, 1.0, false)]);
    for _ in 0..3 {
        sim.step_no_gather();
    }
    assert_eq!(sim.pairs(), 0, "one row pairs with nothing");
    assert_eq!(sim.tree.active.leaves(), 1, "the one dynamic row is the active tree's one lane");
    assert_eq!(sim.tree.diag().members, 0);
}

/// Two touching rows on the tree path: exactly the pair `(0, 1)`, found once — by row 0's
/// query (`row > query` rule), never by row 1's. RED if the owner rule is dropped (the pair
/// would be emitted twice) or inverted (never).
#[test]
fn edge_two_touching_rows_on_the_tree_path_pair_once() {
    let mut sim = Sim::new(vec![
        sphere([0.0, 0.0, 0.0], 1.0, 1.0, false),
        sphere([1.5, 0.0, 0.0], 1.0, 1.0, false),
    ]);
    sim.step_no_gather();
    assert_eq!(
        sim.out.pairs(),
        &[(crate::manifold::BodyIndex(0), crate::manifold::BodyIndex(1))],
        "the one pair, once, in (min, max) order"
    );
}

/// A world of statics only: once admitted, Q is empty, the active tree has zero levels and no
/// row queries anything — every pair is served from `SS` by the assembly alone, and the set
/// still equals all-pairs. RED if an empty active tree is walked (its `levels == 0` guard
/// removed: the root level underflows) or if the assembly needs a stream entry to place an
/// `SS` run.
#[test]
fn edge_all_static_world_is_served_from_ss_alone() {
    // A 4 × 4 grid of touching statics: 24 axis-neighbour pairs (spheres of radius 0.6 at
    // unit spacing touch along axes only; the diagonal is 1.414 > 1.2).
    let mut bodies = Vec::new();
    for k in 0..16 {
        bodies.push(sphere([(k % 4) as f32, 0.0, (k / 4) as f32], 0.6, 0.0, false));
    }
    let mut sim = Sim::new(bodies);
    for _ in 0..3 {
        sim.step_no_gather();
    }
    let d = sim.tree.diag();
    assert_eq!(d.members, 16, "every static is a member after the admission step");
    assert_eq!(d.static_rebuilds, 1);
    assert_eq!(sim.tree.active.leaves(), 0, "Q is empty: the active tree has no lane");
    assert_eq!(sim.tree.ss.as_read_slice().len(), 24, "SS holds the 24 axis-neighbour pairs");
    assert_eq!(sim.pairs(), 24, "anti-vacuity: the pairs are emitted from SS");
    // Two more steps with nothing to query: the counters stand still and the oracle holds.
    sim.step_no_gather();
    let d = sim.step_no_gather();
    assert_eq!(d.static_rebuilds, 1);
    assert_eq!(d.evictions, 0);
    assert_eq!(sim.pairs(), 24);
}

/// A world of Excluded rows only (a NaN coordinate each): no Normal row, no Wide row, no tree,
/// no pair; every row-step is counted Excluded. RED if an Excluded row is put in a tree (the
/// member/kind debug check) or paired against another Excluded row.
#[test]
fn edge_all_excluded_world_has_no_pairs() {
    let mut bodies = Vec::new();
    for k in 0..5 {
        bodies.push(sphere([f32::NAN, k as f32, 0.0], 1.0, if k % 2 == 0 { 0.0 } else { 1.0 }, false));
    }
    let mut sim = Sim::new(bodies);
    for _ in 0..3 {
        sim.step_no_gather();
    }
    let d = sim.tree.diag();
    assert_eq!(sim.pairs(), 0);
    assert_eq!(d.excluded_rows, 3 * 5, "every row is Excluded on each of the 3 steps");
    assert_eq!(d.wide_rows, 0);
    assert_eq!(d.members, 0, "an Excluded static is never pending");
    assert_eq!(sim.tree.active.leaves(), 0);
}

// ── C3b: the leaf-list query (G-LL4, G-LL5, the review's W1 and W2) ─────────────────────────

/// A row's bits for the leaf-list tests: small rows (so rows collide with the query row), rows
/// near `2^24`, and the empty-lane sentinel.
fn lane_row() -> BoxedStrategy<u32> {
    prop_oneof![
        4 => 0u32..24,
        2 => (1u32 << 24) - 24..(1u32 << 24),
        1 => Just(NO_LANE_ROW),
    ]
    .boxed()
}

/// A query row for the leaf-list tests, drawn from the same ranges as [`lane_row`] (no sentinel:
/// a query row is a row).
fn query_row() -> BoxedStrategy<u32> {
    prop_oneof![0u32..24, (1u32 << 24) - 24..(1u32 << 24)].boxed()
}

/// A level-1 node whose lanes hold the given boxes.
fn internal_node(lo: &[(f32, f32, f32); LANES], hi: &[(f32, f32, f32); LANES]) -> Node8 {
    let mut node = Node8 { p: [[0.0; 8]; 6] };
    for k in 0..LANES {
        node.p[0][k] = lo[k].0;
        node.p[1][k] = lo[k].1;
        node.p[2][k] = lo[k].2;
        node.p[3][k] = hi[k].0;
        node.p[4][k] = hi[k].1;
        node.p[5][k] = hi[k].2;
    }
    node
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: G0_CASES,
        failure_persistence: None,
        ..ProptestConfig::default()
    })]

    /// G-LL4: `leaf_mask_above`'s kernel arm equals its scalar arm and is `leaf_mask` masked to
    /// the lanes whose row is above the query row.
    #[test]
    fn gll4_leaf_mask_above_arms_agree(
        lanes in prop::array::uniform8((any_f32(), any_f32(), any_f32(), any_f32(), lane_row())),
        q in (any_f32(), any_f32(), any_f32(), any_f32()),
        row in query_row(),
    ) {
        let node = leaf_node(&lanes);
        let (qx, qy, qz, qr) = q;
        let got = leaf_mask_above(&node, qx, qy, qz, qr, row);
        prop_assert_eq!(got, leaf_mask_above_scalar(&node, qx, qy, qz, qr, row), "kernel arm ≠ scalar arm");
        let mut above = 0u32;
        for (k, lane) in lanes.iter().enumerate() {
            if lane.4 != NO_LANE_ROW && lane.4 > row {
                above |= 1 << k;
            }
        }
        prop_assert_eq!(got, leaf_mask(&node, qx, qy, qz, qr) & above, "the mask is the exact test above the row");
    }

    /// G-LL4: two left-packed pushes and a seal through the kernel arm equal the scalar arm lane
    /// for lane (boxes, leaf indices, max rows), and both prefilters equal their scalar arms and
    /// the reference definition on every chunk — the pad lanes never pass.
    #[test]
    fn gll4_left_pack_and_prefilter_arms_agree(
        lo in prop::array::uniform8((any_f32(), any_f32(), any_f32())),
        hi in prop::array::uniform8((any_f32(), any_f32(), any_f32())),
        masks in (0u32..256, 0u32..256),
        firsts in (0u32..4, 0u32..4),
        maxrows in prop::collection::vec(query_row(), 32),
        q in (-4.0f32..4.0f32, -4.0f32..4.0f32, -4.0f32..4.0f32, 0.0f32..4.0f32),
        row in query_row(),
    ) {
        let node = internal_node(&lo, &hi);
        // 32 leaf nodes whose max rows the active pushes read.
        let mut leaves = vec![Node8 { p: [[0.0; 8]; 6] }; 32];
        for (leaf, &m) in leaves.iter_mut().zip(&maxrows) {
            leaf.p[LEAF_MAXROW][0] = f32::from_bits(m);
        }
        let (mut kernel_slot, mut scalar_slot) = (core::mem::MaybeUninit::uninit(), core::mem::MaybeUninit::uninit());
        let kernel = CandList::init_in(&mut kernel_slot);
        let scalar = CandList::init_in(&mut scalar_slot);
        for (mask, first) in [(masks.0, firsts.0 * 8), (masks.1, firsts.1 * 8)] {
            prop_assert!(kernel.push_lanes_arm::<false>(&node, mask, first, LEAF_LIST_CAP, Some(&leaves)));
            prop_assert!(scalar.push_lanes_arm::<true>(&node, mask, first, LEAF_LIST_CAP, Some(&leaves)));
        }
        kernel.seal();
        scalar.seal();
        let len = (masks.0.count_ones() + masks.1.count_ones()) as usize;
        prop_assert_eq!(kernel.len(), len);
        prop_assert_eq!(scalar.len(), len);
        prop_assert_eq!(kernel.chunks(), len.div_ceil(LANES));
        // The reference: the two pushes' lanes in ascending lane order.
        let mut want = Vec::new();
        for (mask, first) in [(masks.0, firsts.0 * 8), (masks.1, firsts.1 * 8)] {
            for k in 0..LANES {
                if mask >> k & 1 == 1 {
                    let b = [node.p[0][k], node.p[1][k], node.p[2][k], node.p[3][k], node.p[4][k], node.p[5][k]];
                    let leaf = first + k as u32;
                    want.push((b, leaf, maxrows[leaf as usize]));
                }
            }
        }
        for (i, w) in want.iter().enumerate() {
            let (kb, kl, km) = kernel.lane(i);
            let (sb, sl, sm) = scalar.lane(i);
            let bits = |b: [f32; 6]| b.map(f32::to_bits);
            prop_assert_eq!(bits(kb), bits(w.0), "kernel lane {} box", i);
            prop_assert_eq!(bits(sb), bits(w.0), "scalar lane {} box", i);
            prop_assert_eq!((kl, km), (w.1, w.2), "kernel lane {} leaf / max row", i);
            prop_assert_eq!((sl, sm), (w.1, w.2), "scalar lane {} leaf / max row", i);
        }
        let qb = QueryBox { lo: [q.0, q.1, q.2], hi: [q.0 + q.3, q.1 + q.3, q.2 + q.3] };
        for c in 0..kernel.chunks() {
            let boxed = kernel.prefilter_box(c, &qb);
            let cut = kernel.prefilter_box_maxrow(c, &qb, row);
            prop_assert_eq!(boxed, kernel.prefilter_scalar::<false>(c, &qb, row), "box prefilter arms, chunk {}", c);
            prop_assert_eq!(cut, kernel.prefilter_scalar::<true>(c, &qb, row), "max-row prefilter arms, chunk {}", c);
            prop_assert_eq!(boxed, scalar.prefilter_box(c, &qb), "the two lists agree, chunk {}", c);
            for k in 0..LANES {
                let i = c * LANES + k;
                let (want_box, want_cut) = match want.get(i) {
                    Some((b, _, m)) => {
                        let meets = qb.lo[0] <= b[3] && qb.hi[0] >= b[0]
                            && qb.lo[1] <= b[4] && qb.hi[1] >= b[1]
                            && qb.lo[2] <= b[5] && qb.hi[2] >= b[2];
                        (meets, meets && *m > row)
                    }
                    None => (false, false),
                };
                prop_assert_eq!(boxed >> k & 1 == 1, want_box, "candidate {} box", i);
                prop_assert_eq!(cut >> k & 1 == 1, want_cut, "candidate {} cut", i);
                if want_box {
                    prop_assert_eq!(kernel.leaf(c, k), want[i].1);
                }
            }
        }
    }
}

/// The review's W2: after a build, every leaf node's `LEAF_MAXROW` is its largest live row —
/// the partial last leaf included — and the cut's validity flag follows builds, kills and
/// re-rows.
#[test]
fn leaf_maxrow_is_the_largest_live_row_after_a_build() {
    crate::scratch_ids::register_tree_column_layouts();
    let mut tree = PackedBvh8::new(tree_column_id(TREE_ACTIVE), 4096);
    let mut ka = ScratchColumn::<u64>::new(tree_column_id(TREE_SORT_A), 4096);
    let mut kb = ScratchColumn::<u64>::new(tree_column_id(TREE_SORT_B), 4096);
    // 21 items (two full leaves and one of five), rows scattered and not in position order.
    let items: Vec<Item> = (0..21u32)
        .map(|i| Item::new((i * 7 % 21) as f32, (i % 3) as f32, 0.0, 0.5, (i * 37) % 101 + 3))
        .collect();
    tree.build(&items, &mut ka, &mut kb);
    assert!(tree.maxrow_valid(), "a build makes the max rows exact");
    let leaves = tree.leaf_nodes();
    assert_eq!(leaves.len(), 3);
    for (l, node) in leaves.iter().enumerate() {
        let live: Vec<u32> = node.p[LEAF_ROW].iter().map(|b| b.to_bits()).filter(|&r| r != NO_LANE_ROW).collect();
        assert_eq!(live.len(), if l == 2 { 5 } else { 8 }, "leaf {l}'s live lanes");
        assert_eq!(
            node.p[LEAF_MAXROW][0].to_bits(),
            *live.iter().max().expect("a leaf node has a live lane"),
            "leaf {l}: the max row is the largest live row"
        );
    }
    tree.set_leaf_row(3, 999);
    assert!(!tree.maxrow_valid(), "a re-row invalidates the max rows");
    tree.build(&items, &mut ka, &mut kb);
    assert!(tree.maxrow_valid());
    tree.kill(20);
    assert!(!tree.maxrow_valid(), "a kill invalidates the max rows");
}

/// The review's W1: a one-leaf active tree (at most eight Q rows) over a multi-leaf static
/// tree. The active collection is the one leaf with its own box, the static collection runs
/// over the static tree's internal nodes, and every pair of the Q rows is found once.
#[test]
fn gll_one_leaf_active_tree_over_a_multi_leaf_static_tree() {
    // 72 static spheres on a line (nine leaf nodes, two levels), five dynamic spheres resting on
    // five of them and touching each other in a row.
    let mut bodies: Vec<BodyState> = (0..72).map(|k| sphere([1.5 * k as f32, 0.0, 0.0], 0.8, 0.0, false)).collect();
    for k in 0..5 {
        bodies.push(sphere([3.0 + 1.2 * k as f32, 1.2, 0.0], 0.7, 1.0, false));
    }
    let mut sim = Sim::new(bodies);
    for _ in 0..4 {
        sim.step_no_gather();
    }
    assert_eq!(sim.tree.diag().members, 72, "the statics were admitted");
    assert_eq!(sim.tree.active.leaves(), 5, "Q is the five dynamics");
    assert_eq!(sim.tree.active.levels(), 1, "a one-leaf active tree");
    assert!(sim.tree.statics.levels() > 1, "a static tree with internal nodes");
    let dynamic_pairs = sim.out.pairs().iter().filter(|(a, b)| a.0 >= 72 || b.0 >= 72).count();
    assert!(dynamic_pairs >= 4 + 5, "anti-vacuity: the dynamics touch each other and the line");
    let d = sim.tree.diag();
    assert!(d.leaf_list_leaves > 0 && d.fallback_leaves == 0, "the leaf list answered: {d:?}");
}

/// G-LL5: a collection over the cap answers its leaf with the per-row walk. With the cap
/// lowered below every collection (a cluster in which every leaf's box meets every leaf) each
/// active leaf falls back, on the active and on the static side; the pair set is the oracle's
/// and the bytes are the leaf list's (both checked by `Sim`). At the cap the leaf list answers
/// every leaf again.
#[test]
fn gll5_a_collection_over_the_cap_falls_back() {
    // 40 spheres of radius 2 in a 1.8-wide cube: every pair touches, five leaf nodes.
    let cluster = |inv_mass: f32, offset: f32| -> Vec<BodyState> {
        (0..40)
            .map(|k| {
                let p = [(k % 3) as f32 * 0.9 + offset, (k / 3 % 3) as f32 * 0.9, (k / 9) as f32 * 0.45];
                sphere(p, 2.0, inv_mass, false)
            })
            .collect()
    };
    // Active overflow: five leaves each collect five, cap 4.
    let mut sim = Sim::new(cluster(1.0, 0.0));
    sim.tree.set_leaf_list_cap(4);
    let d = sim.step_no_gather();
    assert_eq!(sim.tree.active.leaves(), 40);
    assert_eq!((d.fallback_leaves, d.leaf_list_leaves), (5, 0), "every active leaf fell back: {d:?}");
    assert_eq!(sim.pairs(), 40 * 39 / 2, "anti-vacuity: every pair touches");
    sim.tree.set_leaf_list_cap(5);
    let d = sim.step_no_gather();
    assert_eq!((d.fallback_leaves, d.leaf_list_leaves), (5, 5), "at the cap the leaf list answers");

    // Static overflow: the same cluster static (five static leaves) under eight dynamics.
    let mut bodies = cluster(0.0, 0.0);
    bodies.extend(cluster(1.0, 0.3).into_iter().take(8));
    let mut sim = Sim::new(bodies);
    for _ in 0..3 {
        sim.step_no_gather();
    }
    assert_eq!(sim.tree.diag().members, 40, "the cluster was admitted");
    assert!(sim.tree.statics.levels() > 1);
    let before = sim.tree.diag();
    sim.tree.set_leaf_list_cap(4);
    let d = sim.step_no_gather();
    assert_eq!(d.fallback_leaves - before.fallback_leaves, 1, "the one active leaf fell back on its static collection");
    assert_eq!(d.leaf_list_leaves, before.leaf_list_leaves);
    sim.tree.set_leaf_list_cap(LEAF_LIST_CAP);
    let d = sim.step_no_gather();
    assert_eq!(d.leaf_list_leaves - before.leaf_list_leaves, 1);
}

/// The kernel receipts: every active leaf node of a tree-path step is counted once, by the path
/// that answered it, and a brute step counts nothing.
#[test]
fn receipts_name_the_kernel_that_ran() {
    let mut sim = settled(30);
    let before = sim.tree.diag();
    let leaves = u64::from(sim.tree.active.leaves().div_ceil(LANES as u32));
    assert!(leaves >= 4, "anti-vacuity: several active leaf nodes");
    let d = sim.step();
    assert_eq!(d.leaf_list_leaves - before.leaf_list_leaves, leaves, "the default kernel is the leaf list");
    assert_eq!((d.row_walk_leaves, d.fallback_leaves), (before.row_walk_leaves, before.fallback_leaves));
    sim.tree.set_query_kernel(QueryKernel::RowWalk);
    let d2 = sim.step();
    assert_eq!(d2.row_walk_leaves - d.row_walk_leaves, leaves, "the per-row walk when selected");
    assert_eq!(d2.leaf_list_leaves, d.leaf_list_leaves);
    assert_eq!(sim.tree.query_kernel(), QueryKernel::RowWalk);
}

/// The kept-count reference of the review's W2 on the active tree of the last step: per Q row,
/// the active leaf nodes whose box meets the row's query box, without and with the max-row cut.
/// It reads every leaf node, not the collection — `L`'s box contains each of its rows' query
/// boxes, so every leaf node whose box meets a row's query box is in `L`'s collection — and
/// takes each node's largest row from its lanes, not from `LEAF_MAXROW`, so it shares nothing
/// with the pass but the tree.
fn kept_reference(tree: &PackedBvh8) -> (u64, u64) {
    let leaves = tree.leaf_nodes();
    let boxes: Vec<QueryBox> = (0..leaves.len()).map(|m| tree.leaf_box(m)).collect();
    let maxrows: Vec<u32> = leaves
        .iter()
        .map(|node| node.p[LEAF_ROW].iter().map(|b| b.to_bits()).filter(|&r| r != NO_LANE_ROW).max().unwrap_or(0))
        .collect();
    let (mut without_cut, mut with_cut) = (0u64, 0u64);
    for slot in 0..tree.leaves() {
        let leaf = tree.leaf(slot);
        let q = QueryBox::of(leaf.x, leaf.y, leaf.z, leaf.r);
        for (b, &maxrow) in boxes.iter().zip(&maxrows) {
            let meets = (0..3).all(|a| q.lo[a] <= b.hi[a] && q.hi[a] >= b.lo[a]);
            without_cut += u64::from(meets);
            with_cut += u64::from(meets && maxrow > leaf.row);
        }
    }
    (without_cut, with_cut)
}

/// The kept-count pin's scene counts (`kept_reference`), pinned so a change of the scene or of
/// the reference is visible: without the cut, and with it.
const KEPT_PIN_WITHOUT_CUT: u64 = 48;
const KEPT_PIN_WITH_CUT: u64 = 42;

/// The review's W2, the max-row cut's gate in the default test command: on a line of touching
/// spheres whose rows are scattered against their positions (so a leaf node's rows span the
/// row range and the cut decides), the pass keeps exactly the reference's candidates with the
/// cut, strictly fewer than without it. G-LL1 cannot see the cut — a candidate it removes emits
/// nothing — so this is the gate that goes RED when the cut is dropped (the pass's active
/// prefilter run without `maxrow > row`) or loosened to `maxrow >= row`.
#[test]
fn gll3_kept_count_pin_sees_the_max_row_cut() {
    // 40 radius-0.6 spheres at unit spacing: row `r` sits at `x = (17·r) mod 40`.
    let bodies: Vec<BodyState> = (0..40u32).map(|r| sphere([((17 * r) % 40) as f32, 0.0, 0.0], 0.6, 1.0, false)).collect();
    let mut sim = Sim::new(bodies);
    sim.step_no_gather();
    assert_eq!(sim.pairs(), 39, "anti-vacuity: the line's 39 touching neighbours");
    let (without_cut, with_cut) = kept_reference(&sim.tree.active);
    let c: LeafListCounts = sim.tree.ll_counts;
    assert_eq!(c.kept_active, with_cut, "the pass keeps the reference's candidates under the cut: {c:?}");
    assert!(with_cut < without_cut, "anti-vacuity: the cut removes candidates on this scene");
    assert_eq!((without_cut, with_cut), (KEPT_PIN_WITHOUT_CUT, KEPT_PIN_WITH_CUT), "the scene's pinned counts");
    assert_eq!((c.leaves, c.rows), (5, 40));
    assert_eq!(c.emitted, 39, "one owner per pair");
    assert_eq!((c.cands_static, c.kept_static), (0, 0), "no static set");
}

/// Values for the network's property test: a few small ones (so duplicates are common), the
/// padding value itself, and the full range.
fn network_value() -> BoxedStrategy<u32> {
    prop_oneof![4 => 0u32..6, 1 => Just(u32::MAX), 1 => Just(u32::MAX - 1), 2 => any::<u32>()].boxed()
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: G0_CASES,
        failure_persistence: None,
        ..ProptestConfig::default()
    })]

    /// F2's gate: on every length `0..=16`, with duplicates and `u32::MAX` (the padding value)
    /// in the input, the network's kernel arm and its scalar arm both equal `sort_unstable`.
    #[test]
    fn f2_network_sorts_like_sort_unstable(values in prop::collection::vec(network_value(), 0..=NETWORK_SORT_MAX)) {
        let mut want = values.clone();
        want.sort_unstable();
        let mut kernel = values.clone();
        sort_network(&mut kernel);
        prop_assert_eq!(&kernel, &want, "kernel arm");
        let mut scalar = values;
        sort_network_scalar(&mut scalar);
        prop_assert_eq!(&scalar, &want, "scalar arm");
    }
}

/// F2: every length `0..=16` is sorted, each from its reverse and from a rotation (the proptest
/// draws lengths at random; this one takes each).
#[test]
fn f2_network_sorts_every_length() {
    for n in 0..=NETWORK_SORT_MAX {
        let reversed: Vec<u32> = (0..n as u32).rev().collect();
        let rotated: Vec<u32> = (0..n as u32).map(|i| (i * 5 + 3) % (n as u32).max(1)).collect();
        for input in [reversed, rotated] {
            let mut want = input.clone();
            want.sort_unstable();
            let mut kernel = input.clone();
            sort_network(&mut kernel);
            assert_eq!(kernel, want, "kernel arm, n = {n}");
            let mut scalar = input;
            sort_network_scalar(&mut scalar);
            assert_eq!(scalar, want, "scalar arm, n = {n}");
        }
    }
}

// ── L10 C3c: the sleeper set (design C5 as amended by L10's T1–T4 and T6) ────────────────────

/// A test sleep hint (the design's `BitHint`, with L10's two answers): `frozen[r]` offers row
/// `r` to Z, `anchor[r]` lets it be a member of either set at all. Rows past either list read
/// "not frozen" and "may be a member".
#[derive(Clone, Debug, Default)]
struct BitHint {
    frozen: Vec<bool>,
    anchor: Vec<bool>,
}

impl SleepHint for BitHint {
    fn frozen(&self, r: usize) -> bool {
        self.frozen.get(r).copied().unwrap_or(false)
    }

    fn anchor_ok(&self, r: usize) -> bool {
        self.anchor.get(r).copied().unwrap_or(true)
    }
}

impl BitHint {
    /// Every non-static row of `bodies` frozen, every row anchored.
    fn all_dynamic(bodies: &[BodyState]) -> Self {
        Self {
            frozen: bodies.iter().map(|b| b.inv_mass != 0.0).collect(),
            anchor: vec![true; bodies.len()],
        }
    }
}

/// The structural invariants of the sleeper set after a step with `hint` (or after a release):
/// the withheld list is strictly sorted and every entry is `(min < max)` within the rows, a pair
/// of S ∪ Z with a Z endpoint whose both rows the hint anchored this step (Invariant V), and the
/// sleeper tree's live lanes are its member count.
fn assert_sleeper_invariants(sim: &Sim, hint: &BitHint) {
    let n = sim.bodies.len();
    let withheld = sim.out.withheld();
    assert!(withheld.windows(2).all(|w| w[0] < w[1]), "SL strictly sorted");
    let recs = sim.tree.rec[usize::from(sim.tree.cur)].as_read_slice();
    for &(a, b) in withheld {
        let (a, b) = (a.0 as usize, b.0 as usize);
        assert!(a < b && b < n, "an SL entry ({a}, {b}) is (min < max) within the rows");
        let (sa, sb) = (recs[a].set(), recs[b].set());
        assert!(
            sa != SET_NONE && sb != SET_NONE && (sa == SET_Z || sb == SET_Z),
            "({a}, {b}) is a pair of S ∪ Z with a Z endpoint"
        );
        assert!(hint.anchor_ok(a) && hint.anchor_ok(b), "Invariant V: ({a}, {b})'s rows may be members");
        for (row, set) in [(a, sa), (b, sb)] {
            let is_static = sim.bodies[row].inv_mass == 0.0 && !sim.bodies[row].kinematic;
            assert_eq!(set == SET_S, is_static, "row {row}: S holds the statics, Z the rest");
        }
    }
    assert_eq!(u64::from(sim.tree.sleepers.live()), sim.tree.sleeper_members(), "live sleeper lanes equal |Z|");
}

/// A floor and a 4 × 3 grid of touching boxes on it (unit pitch: every box pairs with its
/// neighbours and the floor).
fn grid_on_floor() -> Vec<BodyState> {
    let mut v = vec![boxed([0.0, -1.0, 0.0], [20.0, 1.0, 20.0], 0.0)];
    for k in 0..12 {
        v.push(boxed([(k % 4) as f32, 0.5, (k / 4) as f32], [0.5, 0.5, 0.5], 1.0));
    }
    v
}

/// T3 and T4, and bp G1's release script: with every box frozen the rent rule admits the boxes
/// into Z at step 1 and the floor into S at step 2, after which every pair is withheld and the
/// stream is empty; a release of two boxes (a D2 hit) moves exactly their pairs into the stream;
/// the next step, whose hint no longer offers them, keeps them out of Z; a release of every box
/// empties the withheld list, and the stream is then the brute set.
#[test]
fn c3c_release_script_moves_exactly_the_released_rows_pairs() {
    let mut sim = Sim::new(grid_on_floor());
    let hint = BitHint::all_dynamic(&sim.bodies);
    let d0 = sim.step_hinted(&hint);
    assert_eq!((d0.sleeper_rebuilds, d0.hint_candidates), (0, 12), "step 0: twelve candidates, rent 12 < 12 + 3");
    let d1 = sim.step_hinted(&hint);
    assert_eq!(d1.sleeper_rebuilds, 1, "step 1: rent 24 ≥ 15, the boxes are admitted");
    assert_eq!(sim.tree.sleeper_members(), 12);
    let d2 = sim.step_hinted(&hint);
    assert_eq!((d2.static_rebuilds, d2.members), (1, 13), "step 2: the floor is admitted");
    let d3 = sim.step_hinted(&hint);
    assert_eq!((d3.evictions, d3.hint_candidates), (0, 24), "no eviction; the 24 candidates of steps 0 and 1 only");
    assert!(sim.out.pairs_stream().is_empty(), "every pair is withheld: {:?}", sim.out.pairs_stream());
    let all = sim.out.withheld().len();
    assert_eq!(all, sim.oracle.pairs().len(), "anti-vacuity: the withheld list is the whole set");
    assert!(all > 12, "anti-vacuity: box–box pairs as well as box–floor pairs");
    assert_sleeper_invariants(&sim, &hint);

    // A D2 hit on rows 1 and 2: their pairs, and only theirs, move into the stream.
    let released = sim.release(&[1, 2]);
    assert_eq!(released, 2);
    assert_eq!(sim.tree.sleeper_members(), 10);
    let touches = |p: &(crate::manifold::BodyIndex, crate::manifold::BodyIndex)| {
        [1, 2].contains(&p.0.0) || [1, 2].contains(&p.1.0)
    };
    assert!(!sim.out.pairs_stream().is_empty(), "anti-vacuity: the released rows had pairs");
    assert!(sim.out.pairs_stream().iter().all(touches), "the stream holds only the released rows' pairs");
    assert!(!sim.out.withheld().iter().any(touches), "the withheld list holds none of them");
    assert_eq!(sim.out.pairs_stream().len() + sim.out.withheld().len(), all, "the release moves pairs, it drops none");
    assert_sleeper_invariants(&sim, &hint);

    // The next step's hint no longer offers them (their records restored): they are Q rows.
    let mut after = hint.clone();
    after.frozen[1] = false;
    after.frozen[2] = false;
    let d4 = sim.step_hinted(&after);
    assert_eq!(d4.hint_candidates, d3.hint_candidates, "rows 1 and 2 are not candidates");
    assert_eq!(sim.tree.sleeper_members(), 10);
    assert_sleeper_invariants(&sim, &after);

    // Every box released: the withheld list empties, the stream is the brute set.
    let released = sim.release(&(1..13).collect::<Vec<u32>>());
    assert_eq!(released, 10);
    assert!(sim.out.withheld().is_empty(), "nothing withheld once every sleeper is released");
    assert_eq!(sim.out.pairs_stream(), sim.oracle.pairs_stream(), "the stream is the brute set");
}

/// T6 and the hint-off rule: a brute step dissolves Z and empties the withheld list, and so does
/// a `Reset` (a missed gather), whose verify reads no hint; the hint then admits the rows again.
#[test]
fn c3c_brute_step_and_reset_dissolve_the_sleeper_set() {
    let mut sim = Sim::new(grid_on_floor());
    let hint = BitHint::all_dynamic(&sim.bodies);
    for _ in 0..4 {
        sim.step_hinted(&hint);
    }
    assert_eq!(sim.tree.sleeper_members(), 12, "construction: the boxes are sleepers");
    assert!(!sim.out.withheld().is_empty(), "construction: pairs are withheld");

    sim.tree.set_brute_max_rows(1 << 20);
    sim.step_hinted(&hint);
    sim.tree.set_brute_max_rows(0);
    assert_eq!(sim.tree.sleeper_members(), 0, "a brute step dissolves Z (T6)");
    assert!(sim.out.withheld().is_empty(), "a brute step empties the withheld list (T6)");
    assert_eq!(sim.tree.diag().members, 1, "the static set is untouched");

    // The next tree step is a Reset (the brute step stamped nothing): no hint, no candidate.
    let before = sim.tree.diag().hint_candidates;
    sim.step_hinted(&hint);
    assert_eq!(sim.tree.diag().hint_candidates, before, "a Reset reads no hint");
    assert_eq!(sim.tree.sleeper_members(), 0);
    for _ in 0..2 {
        sim.step_hinted(&hint);
    }
    assert_eq!(sim.tree.sleeper_members(), 12, "the hint admits the boxes again");

    // A missed gather: the next step is a Reset, which evicts every sleeper.
    sim.gather();
    let evictions = sim.tree.diag().evictions;
    sim.step_hinted(&hint);
    assert_eq!(sim.tree.sleeper_members(), 0, "a Reset dissolves Z");
    assert_eq!(sim.tree.diag().evictions - evictions, 12, "every sleeper is evicted");
    assert!(sim.out.withheld().is_empty(), "a Reset empties the withheld list");
    assert_sleeper_invariants(&sim, &hint);
}

/// T2 (M15's unit form): a static the hint does not let rest leaves S at once — its withheld
/// pairs with the sleepers leave the list and are queried again — and is never admitted while
/// the hint keeps saying so, though it is still.
#[test]
fn c3c_a_static_the_hint_does_not_anchor_leaves_the_static_set() {
    let mut sim = Sim::new(grid_on_floor());
    let mut hint = BitHint::all_dynamic(&sim.bodies);
    for _ in 0..4 {
        sim.step_hinted(&hint);
    }
    assert_eq!(sim.tree.diag().members, 13, "construction: the floor and the boxes");
    let floor_pairs = |sim: &Sim| sim.out.withheld().iter().filter(|p| p.0.0 == 0).count();
    assert_eq!(floor_pairs(&sim), 12, "construction: the floor's pairs are withheld");
    hint.anchor[0] = false;
    let d = sim.step_hinted(&hint);
    assert_eq!(d.members, 12, "the floor left S");
    assert_eq!(floor_pairs(&sim), 0, "its pairs left the withheld list");
    assert_eq!(sim.out.pairs_stream().len(), 12, "and are the stream: the floor is a Q row");
    for _ in 0..4 {
        let d = sim.step_hinted(&hint);
        assert_eq!(d.members, 12, "a static the hint does not anchor is never admitted");
    }
    assert_sleeper_invariants(&sim, &hint);
}

/// One pseudo-random `u64` per call (xorshift64*), for the hints and releases of the property
/// test below.
fn next_rand(state: &mut u64) -> u64 {
    *state ^= *state >> 12;
    *state ^= *state << 25;
    *state ^= *state >> 27;
    state.wrapping_mul(0x2545_f491_4f6c_dd1d)
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: CHURN_CASES,
        failure_persistence: None,
        ..ProptestConfig::default()
    })]

    /// G1 with the sleeper set (L10 C3c): random churn over a real `RowIdentity`, as in
    /// `g1_random_churn_scripts_equal_all_pairs`, a random hint on every step (most rows
    /// frozen, a few not anchored) and a random release after it. The logical pair set — the
    /// stream ⊎ the withheld pairs — equals all-pairs' after every step and every release, and
    /// the withheld list keeps its invariants.
    #[test]
    fn g1_sleeper_set_under_random_hints_releases_and_churn(
        ops in prop::collection::vec(churn_op(), 4..24),
        dynamics in 4usize..14,
        row_walk in any::<bool>(),
        seed in 1u64..u64::MAX,
    ) {
        let mut sim = settled(dynamics);
        sim.tree.set_query_kernel(if row_walk { QueryKernel::RowWalk } else { QueryKernel::LeafList });
        let mut rng = seed;
        for op in ops {
            let n = sim.bodies.len();
            match op {
                ChurnOp::Hold => {}
                ChurnOp::Spawn { at, is_static } => {
                    let at = at % (n + 1);
                    let inv_mass = if is_static { 0.0 } else { 1.0 };
                    let x = -30.0 + 2.0 * (at % 30) as f32;
                    sim.spawn_at(at, boxed([x, 0.49, 2.0 * (at / 30) as f32 + 14.0], [0.5; 3], inv_mass));
                }
                ChurnOp::DespawnSwap(row) => {
                    if n > 2 {
                        sim.despawn_swap(row % n);
                    }
                }
                ChurnOp::Migrate { from, to } => sim.migrate(from % n, to % n),
                ChurnOp::Teleport(row) => sim.bodies[row % n].position.x += 3.0,
                ChurnOp::Reshape(row) => {
                    let b = &mut sim.bodies[row % n];
                    b.shape = match b.shape {
                        ColliderShape::Box { half_extents } => {
                            ColliderShape::Box { half_extents: half_extents + Vec3::new(0.25, 0.0, 0.0) }
                        }
                        ColliderShape::Sphere { radius } => ColliderShape::Sphere { radius: radius + 0.25 },
                    };
                }
                ChurnOp::ClassFlip(row) => {
                    let b = &mut sim.bodies[row % n];
                    b.inv_mass = if b.inv_mass == 0.0 { 1.0 } else { 0.0 };
                }
                ChurnOp::MissGather => sim.gather(),
                ChurnOp::Brute => sim.tree.set_brute_max_rows(1 << 20),
            }
            let n = sim.bodies.len();
            let hint = BitHint {
                frozen: (0..n).map(|_| !next_rand(&mut rng).is_multiple_of(8)).collect(),
                anchor: (0..n).map(|_| !next_rand(&mut rng).is_multiple_of(16)).collect(),
            };
            // `step_hinted` asserts the oracle.
            sim.step_hinted(&hint);
            sim.tree.set_brute_max_rows(0);
            assert_sleeper_invariants(&sim, &hint);
            let released: Vec<u32> = (0..n as u32).filter(|_| next_rand(&mut rng).is_multiple_of(5)).collect();
            // Only the sleepers among them leave (a static stays in S).
            let leaving: Vec<u32> = {
                let recs = sim.tree.rec[usize::from(sim.tree.cur)].as_read_slice();
                released.iter().copied().filter(|&r| recs[r as usize].set() == SET_Z).collect()
            };
            // `release` asserts the oracle again.
            prop_assert_eq!(sim.release(&released) as usize, leaving.len(), "the sleepers among the rows leave");
            prop_assert!(
                !sim.out.withheld().iter().any(|p| leaving.contains(&p.0.0) || leaving.contains(&p.1.0)),
                "a released sleeper keeps no withheld pair"
            );
            assert_sleeper_invariants(&sim, &hint);
        }
    }
}
