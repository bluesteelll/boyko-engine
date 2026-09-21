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
//!
//! Every test is device-free and heap-light; under Miri the property tests shrink to 16 cases
//! at n ≤ 24 and the kernel is the scalar arm.

use proptest::prelude::*;

use boyko_ecs::ecs::core::component::scratch::ScratchColumn;
use boyko_ecs::ecs::identifiers::primitives::EntityId;

use crate::components::ColliderShape;
use crate::math::Vec3;
use crate::resources::{BodyState, ContactPairs};
use crate::row_identity::{NO_ROW, RowIdentity, RowRemap};

use super::bvh::{Item, LANES, LEAF_R, LEAF_ROW, LEAF_X, LEAF_Y, LEAF_Z, Node8, PackedBvh8};
use super::kernel::{QueryBox, box_mask, box_mask_scalar, leaf_mask, leaf_mask_scalar};
use super::{
    ADMIT_BUILD_RATIO, BroadphaseTree, JUMPER, KIND_EXCLUDED, KIND_WIDE, TREE_BRUTE_MAX_ROWS,
    TreeDiag, all_pairs_into, classify, mark_jumpers, merge_into_sorted, sphere_bound_feasible,
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
struct Sim {
    tree: BroadphaseTree,
    rows: RowIdentity,
    out: ContactPairs,
    oracle: ContactPairs,
    bodies: Vec<BodyState>,
    ids: Vec<usize>,
    next_id: usize,
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
            fresh: Vec::new(),
        }
    }

    /// Feeds one gather of the current rows.
    fn gather(&mut self) {
        self.rows.begin_gather();
        {
            let (mut cur, mut add) = self.rows.gather_views();
            for (r, &id) in self.ids.iter().enumerate() {
                cur.push(EntityId(id));
                if self.fresh.contains(&id) {
                    add.push(r as u32);
                }
            }
        }
        self.rows.finish_gather();
        self.fresh.clear();
    }

    /// One tree step against the oracle, without a gather (direct drive, or a missed gather).
    fn step_no_gather(&mut self) -> TreeDiag {
        self.tree.step(&self.bodies, &self.rows, &mut self.out);
        all_pairs_into(&self.bodies, &mut self.oracle);
        assert_eq!(
            self.out.pairs(),
            self.oracle.pairs(),
            "the tree's pair set differs from all-pairs' (n = {})",
            self.bodies.len()
        );
        self.tree.diag()
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
        self.bodies.insert(row, body);
        self.ids.insert(row, id);
        self.fresh.push(id);
        id
    }

    /// Spawns `body` at `row` under a recycled `id`.
    fn respawn_at(&mut self, row: usize, id: usize, body: BodyState) {
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
    ) {
        let bodies = world(&specs, sign);
        let mut sim = Sim::new(bodies);
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
    // The same id, the same pose, back in row 2, flagged added: a new body to the map.
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
    sim.tree.run(&sim.bodies, RowRemap::Rows(&map), &mut sim.out);
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
    ) {
        let mut sim = settled(dynamics);
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
