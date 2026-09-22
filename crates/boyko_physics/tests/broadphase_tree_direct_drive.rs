//! The tree broadphase's public direct-drive seam (`docs/physics/perf-campaign/levers/broadphase/
//! 04-DESIGN-REV2.md`, commit C3): [`BroadphaseTree::step_direct`], [`BroadphaseTree::step_translated`]
//! and [`NO_PREV_ROW`], which the G4 bench (`benches/broadphase.rs`) drives from outside the crate.
//! These tests pin what that bench relies on, through the same public API and nothing else:
//!
//! * the brute threshold: at `N <= brute_max_rows` the seam runs the brute loop and leaves the
//!   tree's state untouched; one row above it takes the tree path;
//! * the rent rule's shape under direct drive: a still static is admitted on the third step
//!   (the bench's `TREE_WARM_STEPS`), and 64 pending rows into `m` members are admitted on the
//!   step the design's D3.5 arithmetic names (`ceil((320 + m) / 256)` with `ADMIT_BUILD_RATIO`
//!   = 1/4) — the count the maintenance arm `admission_of_64` loops for, which is how the bench's
//!   step cap and the design's "≈ 6 / 40 / 390 steps" are shown reachable;
//! * a `Rows` step through a hand-made previous-row map: a one-row shift translates once with no
//!   patch; a spawn marked [`NO_PREV_ROW`] is queried; a non-injective map carries the duplicate
//!   only once (the consumed mark, design M9); ruling W1's high jumper diverts at least two
//!   entries;
//! * the length precondition of `step_translated` panics.
//!
//! Every step's output is compared with [`all_pairs_into`] (design D1) — the seam is exact, not
//! only cheap.
//!
//! # What makes each test red
//!
//! * `brute_*`: the threshold compare flipped to `<` (row 64 takes the tree path and gains
//!   members), or the brute branch dropped.
//! * `rent_*`: the ratio or the rent accumulation changed — the admission step moves.
//! * `shift_*` / `spawn_*`: the `Rows` locator replaced by the identity one (design M5: every
//!   member's bits then mismatch its neighbour's record and `evictions` reads `m`, not 0).
//! * `duplicate_*`: the consumed mark not written (M9: the duplicate row keeps its neighbour's
//!   leaf slot and the neighbour's pairs are lost).
//! * `high_jumper_*`: the patch rule's diversion count (ruling W1) below 2, or the pair set
//!   missing the jumper's entries (M11).
//!
//! The lattice constants mirror the bench's maintenance lattice (pitch 0.9, radius 0.65: an
//! interior member has 18 partners), so the small cases here have the same list shape as the
//! bench's m ∈ {1 240, 10k, 100k}.

use boyko_physics::broadphase_tree::{
    BroadphaseTree, NO_PREV_ROW, TREE_BRUTE_MAX_ROWS, TreeDiag, all_pairs_into,
};
use boyko_physics::components::ColliderShape;
use boyko_physics::manifold::BodyIndex;
use boyko_physics::math::Vec3;
use boyko_physics::resources::{BodyState, ContactPairs};

/// The bench's maintenance lattice pitch.
const LATTICE_PITCH: f32 = 0.9;
/// The bench's maintenance lattice radius (18 partners per interior member).
const LATTICE_RADIUS: f32 = 0.65;
/// The bench's teleport distance: far enough that a moved member pairs with nothing at home.
const TELEPORT_X: f32 = 1_000.0;
/// The bench's step cap on a maintenance cycle (`MAINT_STEP_CAP`), mirrored so the reach test
/// says the same thing the bench's panic would.
const MAINT_STEP_CAP: usize = 4_000;

/// A static sphere (`inv_mass == 0` is the default).
fn sphere(position: Vec3, radius: f32) -> BodyState {
    BodyState { position, shape: ColliderShape::Sphere { radius }, ..Default::default() }
}

/// `m` static spheres on the bench's lattice, in x-fastest order.
fn statics_lattice(m: usize) -> Vec<BodyState> {
    let side = (m as f64).cbrt().ceil() as usize;
    let mut bodies = Vec::with_capacity(m);
    'outer: for z in 0..side {
        for y in 0..side {
            for x in 0..side {
                if bodies.len() == m {
                    break 'outer;
                }
                bodies.push(sphere(
                    Vec3::new(
                        x as f32 * LATTICE_PITCH,
                        y as f32 * LATTICE_PITCH,
                        z as f32 * LATTICE_PITCH,
                    ),
                    LATTICE_RADIUS,
                ));
            }
        }
    }
    assert_eq!(bodies.len(), m, "test setup: the lattice holds m statics");
    bodies
}

/// The exact set of `bodies`.
fn oracle(bodies: &[BodyState]) -> ContactPairs {
    let mut out = ContactPairs::with_capacity(0);
    all_pairs_into(bodies, &mut out);
    out
}

/// A tree with the tree path forced, stepped [`WARM`] times under direct drive so every still
/// static is a member.
const WARM: usize = 3;
fn warm_tree(bodies: &[BodyState]) -> (BroadphaseTree, ContactPairs) {
    let mut tree = BroadphaseTree::with_capacity(bodies.len() + 1);
    tree.set_brute_max_rows(0);
    let mut out = ContactPairs::with_capacity(0);
    for _ in 0..WARM {
        tree.step_direct(bodies, &mut out);
    }
    (tree, out)
}

fn assert_exact(out: &ContactPairs, bodies: &[BodyState], what: &str) {
    let want = oracle(bodies);
    assert!(!want.pairs().is_empty(), "anti-vacuity: {what} must produce pairs");
    assert_eq!(out.pairs(), want.pairs(), "{what}: the seam's pair set is AllPairs'");
}

// ── The brute threshold ───────────────────────────────────────────────────────

#[test]
fn brute_step_direct_at_the_threshold_leaves_the_state_untouched() {
    let n = TREE_BRUTE_MAX_ROWS as usize;
    let bodies = statics_lattice(n);
    let mut tree = BroadphaseTree::with_capacity(n);
    assert_eq!(tree.brute_max_rows(), TREE_BRUTE_MAX_ROWS, "the default threshold");
    let mut out = ContactPairs::with_capacity(0);
    for _ in 0..WARM {
        tree.step_direct(&bodies, &mut out);
    }
    assert_exact(&out, &bodies, "brute at N == brute_max_rows");
    assert_eq!(
        tree.diag(),
        TreeDiag::default(),
        "N == brute_max_rows is the brute loop: no member, no rebuild, no counter moves"
    );
}

#[test]
fn brute_step_direct_one_row_above_the_threshold_takes_the_tree_path() {
    let n = TREE_BRUTE_MAX_ROWS as usize + 1;
    let bodies = statics_lattice(n);
    let mut tree = BroadphaseTree::with_capacity(n);
    let mut out = ContactPairs::with_capacity(0);
    for _ in 0..WARM {
        tree.step_direct(&bodies, &mut out);
    }
    assert_exact(&out, &bodies, "tree at N == brute_max_rows + 1");
    let d = tree.diag();
    assert_eq!(d.members, n as u64, "every still static is a member after the warm-up");
    assert_eq!(d.static_rebuilds, 1, "one admission");
}

#[test]
fn brute_step_translated_at_the_threshold_ignores_the_map() {
    let n = TREE_BRUTE_MAX_ROWS as usize;
    let bodies = statics_lattice(n);
    let mut tree = BroadphaseTree::with_capacity(n);
    let mut out = ContactPairs::with_capacity(0);
    // A map that would be nonsense on the tree path (every row names row 0): the brute loop
    // never reads it.
    let prev = vec![0u32; n];
    for _ in 0..WARM {
        tree.step_translated(&bodies, &prev, &mut out);
    }
    assert_exact(&out, &bodies, "brute step_translated");
    assert_eq!(tree.diag(), TreeDiag::default(), "the brute loop reads no map and moves no counter");
}

// ── The rent rule under direct drive ──────────────────────────────────────────

#[test]
fn rent_still_statics_are_admitted_on_the_third_direct_step() {
    let m = 125;
    let bodies = statics_lattice(m);
    let mut tree = BroadphaseTree::with_capacity(m);
    tree.set_brute_max_rows(0);
    let mut out = ContactPairs::with_capacity(0);

    tree.step_direct(&bodies, &mut out);
    assert_exact(&out, &bodies, "step 1");
    assert_eq!(tree.diag().members, 0, "step 1: no record to be still against");
    assert_eq!(tree.diag().static_rebuilds, 0);

    tree.step_direct(&bodies, &mut out);
    assert_exact(&out, &bodies, "step 2");
    assert_eq!(tree.diag().members, 0, "step 2: pending, rent = m < m + m/4");
    assert_eq!(tree.diag().static_rebuilds, 0);

    tree.step_direct(&bodies, &mut out);
    assert_exact(&out, &bodies, "step 3");
    assert_eq!(tree.diag().members, m as u64, "step 3: rent = 2m >= m + m/4, admitted");
    assert_eq!(tree.diag().static_rebuilds, 1);
}

/// The step on which 64 pending rows are admitted into `m` members: `rent = 64·k`, admit when
/// `4·rent >= 4·64 + (m + 64)`, i.e. the first `k` with `256·k >= 320 + m`.
fn admission_step_of_64_into(m: usize) -> usize {
    (320 + m).div_ceil(256)
}

/// Runs the bench's `admission_of_64` cycle once at `m` and returns the pending steps it took.
fn admission_of_64_reach(m: usize) -> usize {
    let mut bodies = statics_lattice(m);
    let extra: Vec<BodyState> = statics_lattice(64)
        .into_iter()
        .map(|mut b| {
            b.position = b.position + Vec3::new(-3_000.0, 0.0, 0.0);
            b
        })
        .collect();
    bodies.extend(extra);
    let (mut tree, mut out) = warm_tree(&bodies);
    assert_eq!(tree.diag().members, (m + 64) as u64, "warm-up: every static is a member");

    for body in &mut bodies[m..] {
        body.position = body.position + Vec3::new(TELEPORT_X, 0.0, 0.0);
    }
    let before = tree.diag();
    tree.step_direct(&bodies, &mut out);
    let d = tree.diag();
    assert_eq!(d.evictions - before.evictions, 64, "the 64 teleported rows are evicted");
    assert_eq!(d.members, m as u64, "the lattice keeps its members");
    assert_eq!(d.static_rebuilds, before.static_rebuilds, "64 dead lanes do not compact");
    assert_exact(&out, &bodies, "the eviction step");

    for k in 1..=MAINT_STEP_CAP {
        let before = tree.diag();
        tree.step_direct(&bodies, &mut out);
        let d = tree.diag();
        if d.static_rebuilds != before.static_rebuilds {
            assert_eq!(d.static_rebuilds - before.static_rebuilds, 1, "one admission");
            assert_eq!(d.members, (m + 64) as u64, "the 64 are admitted");
            assert_exact(&out, &bodies, "the admission step");
            return k;
        }
        assert_eq!(d.members, m as u64, "pending rows are not members");
    }
    panic!("the rent rule did not admit 64 rows into {m} members within {MAINT_STEP_CAP} steps");
}

#[test]
fn rent_admission_of_64_arithmetic_matches_the_design_at_the_small_size() {
    // The design's D3.5: "for 64 pending rows, admission comes after ≈ 6 / 40 / 390 steps" at
    // m = 1 240 / 10k / 100k, from the ratio 1/4; the exact integer is ceil((320 + m) / 256).
    assert_eq!(admission_step_of_64_into(1_240), 7);
    assert_eq!(admission_step_of_64_into(10_000), 41);
    assert_eq!(admission_step_of_64_into(100_000), 392);
    // A lattice of 125: the arm's step count is the arithmetic's.
    let k = admission_of_64_reach(125);
    assert_eq!(k, admission_step_of_64_into(125), "125 members: admitted on step ceil(445/256) = 2");
}

#[test]
#[cfg_attr(
    miri,
    ignore = "miri-slow: a 1 240-row lattice (9.5k pairs) verified and copied on every one of \
              ten direct-drive steps; the arithmetic is pinned by the 125-row test above"
)]
fn rent_admission_of_64_reaches_within_the_bench_cap_at_the_design_sizes() {
    // The bench's `admission_of_64` arm loops until the admission step; the design's D3.5 names
    // ≈ 6 / 40 / 390 steps. 100k is release-only here (an unoptimised 868k-pair copy per step
    // for 392 steps is minutes); the bench's own dry run covers it in the bench profile.
    let sizes: &[usize] = if cfg!(debug_assertions) { &[1_240, 10_000] } else { &[1_240, 10_000, 100_000] };
    for &m in sizes {
        let k = admission_of_64_reach(m);
        eprintln!("admission_of_64 into {m} members: admitted on pending step {k}");
        assert_eq!(k, admission_step_of_64_into(m), "m = {m}: the admission step is the arithmetic's");
        assert!(k <= MAINT_STEP_CAP, "m = {m}: inside the bench's step cap");
    }
}

// ── `Rows` steps through a hand-made map ──────────────────────────────────────

#[test]
#[should_panic(expected = "prev_row is indexed by the current rows")]
fn step_translated_panics_when_the_map_length_differs_from_the_rows() {
    let bodies = statics_lattice(125);
    let mut tree = BroadphaseTree::with_capacity(125);
    tree.set_brute_max_rows(0);
    let mut out = ContactPairs::with_capacity(0);
    let prev = vec![NO_PREV_ROW; 124];
    tree.step_translated(&bodies, &prev, &mut out);
}

#[test]
fn shift_by_one_row_translates_once_with_no_patch_and_no_eviction() {
    let m = 125;
    let bodies = statics_lattice(m);
    let (mut tree, mut out) = warm_tree(&bodies);
    assert_eq!(tree.diag().members, m as u64);

    // A dynamic spawned at the front: every member shifts up by one.
    let mut shifted = Vec::with_capacity(m + 1);
    let mut front = sphere(Vec3::new(-3_000.0, 0.0, 0.0), 0.5);
    front.inv_mass = 1.0;
    shifted.push(front);
    shifted.extend_from_slice(&bodies);
    let mut prev: Vec<u32> = Vec::with_capacity(m + 1);
    prev.push(NO_PREV_ROW);
    prev.extend((0..m).map(|r| r as u32));

    let before = tree.diag();
    tree.step_translated(&shifted, &prev, &mut out);
    let d = tree.diag();
    assert_exact(&out, &shifted, "the spawn-at-front step");
    assert_eq!(d.translations - before.translations, 1, "a shift is one translation");
    assert_eq!(d.patches - before.patches, 0, "a shift is monotone: nothing diverted");
    assert_eq!(d.evictions - before.evictions, 0, "every member is located through the map");
    assert_eq!(d.static_rebuilds, before.static_rebuilds, "no rebuild on a row change");
    assert_eq!(d.members, m as u64, "every member kept");

    // Its despawn: every member shifts back down.
    let prev_back: Vec<u32> = (1..=m).map(|r| r as u32).collect();
    let before = tree.diag();
    tree.step_translated(&bodies, &prev_back, &mut out);
    let d = tree.diag();
    assert_exact(&out, &bodies, "the despawn-at-front step");
    assert_eq!(d.translations - before.translations, 1);
    assert_eq!(d.patches - before.patches, 0);
    assert_eq!(d.evictions - before.evictions, 0);
    assert_eq!(d.members, m as u64);
}

#[test]
fn spawn_marked_no_prev_row_is_queried_and_its_pairs_found() {
    let m = 125;
    let bodies = statics_lattice(m);
    let (mut tree, mut out) = warm_tree(&bodies);

    // A dynamic spawned at the tail, overlapping the lattice's last member: found by the query
    // of a row that had no previous row.
    let mut with_spawn = bodies.clone();
    let mut tail = bodies[m - 1];
    tail.inv_mass = 1.0;
    tail.position = tail.position + Vec3::new(0.3, 0.0, 0.0);
    with_spawn.push(tail);
    let mut prev: Vec<u32> = (0..m).map(|r| r as u32).collect();
    prev.push(NO_PREV_ROW);

    let before = tree.diag();
    tree.step_translated(&with_spawn, &prev, &mut out);
    let d = tree.diag();
    assert_exact(&out, &with_spawn, "the tail spawn step");
    let spawned = BodyIndex(m as u32);
    assert!(
        out.pairs().iter().any(|&(a, b)| a == BodyIndex(m as u32 - 1) && b == spawned),
        "the spawned row pairs with the member it overlaps"
    );
    assert_eq!(d.evictions - before.evictions, 0, "an append evicts nothing");
    assert_eq!(d.members, m as u64, "a dynamic is never admitted");
    // The counter counts `Rows` steps with at least one member (the churn bench's Δ40 over 40
    // spawn steps), so an append with every member in place still reads +1, with nothing
    // diverted.
    assert_eq!(d.translations - before.translations, 1, "a Rows step with members translates");
    assert_eq!(d.patches - before.patches, 0, "an append is monotone: nothing diverted");
}

#[test]
fn duplicate_prev_row_carries_the_record_once_and_keeps_the_set_exact() {
    // Design M9 (the consumed mark): two rows naming the same previous row, with identical bits
    // (a duplicate position), must not both carry it. The duplicate becomes Q and is queried;
    // the un-located record vanishes.
    let m = 125;
    let mut bodies = statics_lattice(m);
    let k = 62;
    bodies[k] = bodies[k - 1];
    let (mut tree, mut out) = warm_tree(&bodies);
    assert_eq!(tree.diag().members, m as u64, "both duplicates are members after the warm-up");

    let mut prev: Vec<u32> = (0..m).map(|r| r as u32).collect();
    prev[k] = (k - 1) as u32;
    let before = tree.diag();
    tree.step_translated(&bodies, &prev, &mut out);
    let d = tree.diag();
    assert_exact(&out, &bodies, "the non-injective map step");
    assert!(
        out.pairs().contains(&(BodyIndex((k - 1) as u32), BodyIndex(k as u32))),
        "the duplicate pair itself is in the set"
    );
    assert_eq!(d.evictions - before.evictions, 1, "record k, located by nobody, vanishes");
    assert_eq!(d.members, m as u64 - 1, "row k is not carried: it is Q this step");
    assert_eq!(d.static_rebuilds, before.static_rebuilds, "no rebuild");
}

#[test]
fn high_jumper_migration_diverts_at_least_two_entries_and_keeps_the_set_exact() {
    // Ruling W1: an interior member that is the min endpoint of ≥ 2 entries migrates to the
    // top row (and back). The patch rule diverts its entries and merges them; the set stays
    // exact and no member is evicted.
    let m = 125;
    let bodies = statics_lattice(m);
    let (mut tree, mut out) = warm_tree(&bodies);
    let j = m / 2;
    let higher = out.pairs().iter().filter(|&&(a, b)| a == BodyIndex(j as u32) && b.0 > j as u32).count();
    assert!(higher >= 2, "premise: row {j} has {higher} higher-row partners, needs >= 2");

    // Up: row j moves to the top row; rows above it shift down by one.
    let mut up = bodies.clone();
    let mover = up.remove(j);
    up.push(mover);
    let mut prev: Vec<u32> = (0..j).map(|r| r as u32).collect();
    prev.extend((j + 1..m).map(|r| r as u32));
    prev.push(j as u32);
    let before = tree.diag();
    tree.step_translated(&up, &prev, &mut out);
    let d = tree.diag();
    assert_exact(&out, &up, "the migration-up step");
    assert_eq!(d.translations - before.translations, 1, "a migration is one translation");
    assert!(d.patches - before.patches >= 2, "the jumper's entries are diverted (ruling W1): {}", d.patches - before.patches);
    assert_eq!(d.evictions - before.evictions, 0, "a translation evicts nothing");
    assert_eq!(d.static_rebuilds, before.static_rebuilds, "no rebuild on a row change");
    assert_eq!(d.members, m as u64);

    // Back: the top row returns to j; rows j.. shift up by one.
    let mut prev_back: Vec<u32> = (0..j).map(|r| r as u32).collect();
    prev_back.push((m - 1) as u32);
    prev_back.extend((j..m - 1).map(|r| r as u32));
    let before = tree.diag();
    tree.step_translated(&bodies, &prev_back, &mut out);
    let d = tree.diag();
    assert_exact(&out, &bodies, "the migration-back step");
    assert_eq!(d.translations - before.translations, 1);
    assert!(d.patches - before.patches >= 2, "the way back diverts too: {}", d.patches - before.patches);
    assert_eq!(d.evictions - before.evictions, 0);
    assert_eq!(d.members, m as u64);
}

#[test]
fn teleported_member_is_evicted_and_re_admitted_by_the_rent_rule() {
    // The bench's `eviction_filter` step, then the rent rule's re-admission of one row into
    // m − 1 members: rent = k, admit when 4k >= 4 + (m − 1 + 1), i.e. k = ceil((4 + m) / 4).
    let m = 125;
    let mut bodies = statics_lattice(m);
    let (mut tree, mut out) = warm_tree(&bodies);
    bodies[0].position = bodies[0].position + Vec3::new(TELEPORT_X, 0.0, 0.0);
    let before = tree.diag();
    tree.step_direct(&bodies, &mut out);
    let d = tree.diag();
    assert_exact(&out, &bodies, "the teleport step");
    assert_eq!(d.evictions - before.evictions, 1, "the teleported member is evicted");
    assert_eq!(d.members, m as u64 - 1);
    assert_eq!(d.static_rebuilds, before.static_rebuilds, "one dead lane does not compact");

    let expect_k = (4 + m).div_ceil(4);
    for k in 1..=MAINT_STEP_CAP {
        let before = tree.diag();
        tree.step_direct(&bodies, &mut out);
        let d = tree.diag();
        assert_exact(&out, &bodies, "a pending step");
        if d.static_rebuilds != before.static_rebuilds {
            assert_eq!(k, expect_k, "re-admitted on the rent rule's step");
            assert_eq!(d.members, m as u64, "the row is a member again");
            return;
        }
    }
    panic!("the teleported row was never re-admitted");
}
