//! SDF brick-atlas campaign M5a — TOROIDAL streaming primitives (gate (b), CPU-only, no GPU).
//!
//! The integer-arithmetic heart of camera-follow streaming, verified independently of any bake:
//!
//!  - `toroidal_slot` is a BIJECTION over any `M2_GRID_DIM³` CONTIGUOUS world-cell box (every slot hit
//!    exactly once) — so a full grid scatter never collides two world cells onto one storage slot.
//!  - `for_each_revealed_cell(old, new)` emits EXACTLY `new_box \ old_box` (brute-force set check) over
//!    random old/new pairs incl. axis moves, diagonals, `|Δ| >= DIM` teleports (whole new box) and
//!    `Δ == 0` (empty) — the slab a scroll re-bakes.
//!  - The revealed slab's slots EQUAL the departed cells' slots (the toroidal OVERWRITE property: an
//!    entering cell reuses exactly the slot a leaving cell vacated, so the atlas never grows).
//!
//! `boyko_sdf_math` is a TRUE LEAF (zero third-party deps); like the M3 pointer-grid proptest this file
//! uses a hand-rolled deterministic SplitMix64 PRNG instead of `proptest`. The existing brick.rs unit
//! tests (`toroidal_slot_off_identity_and_negative_wrap`, `revealed_cells_match_box_difference_no_dup`,
//! `revealed_slots_overwrite_departed_slots`) cover a small FIXED case matrix; this file sweeps the same
//! invariants over hundreds of RANDOM old/new pairs (and random WALKS of accumulating small steps), so a
//! shell-decomposition or wrap bug that only triggers at an unusual offset is caught.

use boyko_sdf_math::brick::{for_each_revealed_cell, toroidal_slot, M2_GRID_DIM};
use std::collections::BTreeSet;

/// A fixed base seed so any failure is reproducible (printed in the panic message).
const SEED_BASE: u64 = 0x_5D_F0_05_A0_7E_2B_1C_4E;

const DIM: i32 = M2_GRID_DIM as i32;

// ─────────────────────────────────────────────────────────────────────────────
// Deterministic PRNG (SplitMix64) — keeps `boyko_sdf_math` a zero-dep leaf.
// ─────────────────────────────────────────────────────────────────────────────

struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Rng(seed)
    }
    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    /// A signed cell offset in `[-lo, hi]` inclusive.
    fn offset(&mut self, lo: i32, hi: i32) -> i32 {
        let span = (hi - lo + 1) as u64;
        lo + (self.next_u64() % span) as i32
    }
    /// A world-cell origin spread over a wide signed range (so the negative `rem_euclid` wrap is
    /// exercised, not just the OFF box).
    fn origin_cell(&mut self) -> [i32; 3] {
        [self.offset(-200, 200), self.offset(-200, 200), self.offset(-200, 200)]
    }
}

/// Whether world cell `c` is inside the half-open box `[lo, lo + DIM)³`.
fn in_box(c: [i32; 3], lo: [i32; 3]) -> bool {
    (0..3).all(|a| c[a] >= lo[a] && c[a] < lo[a] + DIM)
}

/// The brute-force set difference `new_box \ old_box` (the revealed-cell oracle).
fn box_difference(old_oc: [i32; 3], new_oc: [i32; 3]) -> Vec<[i32; 3]> {
    let mut out = Vec::new();
    for z in new_oc[2]..new_oc[2] + DIM {
        for y in new_oc[1]..new_oc[1] + DIM {
            for x in new_oc[0]..new_oc[0] + DIM {
                if !in_box([x, y, z], old_oc) {
                    out.push([x, y, z]);
                }
            }
        }
    }
    out
}

// ═════════════════════════════════════════════════════════════════════════════
// (b1) toroidal_slot is a BIJECTION over every contiguous DIM³ world-cell box.
// ═════════════════════════════════════════════════════════════════════════════

/// For a CONTIGUOUS `DIM³` world-cell box at ANY origin (incl. deeply negative), the `DIM³` toroidal
/// slots are a PERMUTATION of `[0, DIM)³` — every slot hit exactly once. A collision would scatter two
/// world cells onto one atlas tile (a streaming corruption).
#[test]
fn toroidal_slot_is_bijection_over_any_dim_box() {
    let mut rng = Rng::new(SEED_BASE ^ 0xB1);
    // The OFF box + the M2 frozen origin cell (-2) + many random origins, incl. negative.
    let mut origins: Vec<[i32; 3]> = vec![[0, 0, 0], [-2, -2, -2], [-1, 3, -7]];
    for _ in 0..512 {
        origins.push(rng.origin_cell());
    }
    let total = (DIM * DIM * DIM) as usize;
    for oc in origins {
        let mut slots: BTreeSet<[u32; 3]> = BTreeSet::new();
        for z in 0..DIM {
            for y in 0..DIM {
                for x in 0..DIM {
                    slots.insert(toroidal_slot([oc[0] + x, oc[1] + y, oc[2] + z]));
                }
            }
        }
        assert_eq!(
            slots.len(),
            total,
            "toroidal_slot collided two world cells onto one slot in the DIM box at origin {oc:?} \
             (only {} of {total} distinct slots) — a streaming scatter corruption",
            slots.len()
        );
        // Each slot coordinate is in [0, DIM).
        for s in &slots {
            assert!(
                s[0] < M2_GRID_DIM && s[1] < M2_GRID_DIM && s[2] < M2_GRID_DIM,
                "slot {s:?} out of [0, DIM) range at origin {oc:?}"
            );
        }
    }
}

// ═════════════════════════════════════════════════════════════════════════════
// (b2) for_each_revealed_cell == new_box \ old_box, no dup, over random pairs + walks.
// ═════════════════════════════════════════════════════════════════════════════

/// Asserts the emitted revealed set equals the brute-force box difference, with no duplicate emission.
fn assert_revealed_equals_difference(old_oc: [i32; 3], new_oc: [i32; 3], ctx: &str) {
    let oracle = box_difference(old_oc, new_oc);

    let mut got: Vec<[i32; 3]> = Vec::new();
    for_each_revealed_cell(old_oc, new_oc, |c| got.push(c));

    // No duplicate emission (the three shells must be disjoint).
    let mut sorted = got.clone();
    sorted.sort_unstable();
    let mut dedup = sorted.clone();
    dedup.dedup();
    assert_eq!(
        sorted.len(),
        dedup.len(),
        "{ctx}: a revealed cell was emitted TWICE (old={old_oc:?} new={new_oc:?}) — shells overlap"
    );

    let mut oracle_sorted = oracle.clone();
    oracle_sorted.sort_unstable();
    assert_eq!(
        sorted, oracle_sorted,
        "{ctx}: revealed set != new_box\\old_box (old={old_oc:?} new={new_oc:?})"
    );

    // Every emitted cell lies inside the NEW box (so its NEW-grid box index is in [0, DIM)).
    for c in &got {
        assert!(
            in_box(*c, new_oc),
            "{ctx}: revealed cell {c:?} lies OUTSIDE the new box (old={old_oc:?} new={new_oc:?})"
        );
    }
}

/// Random INDEPENDENT old/new pairs spanning small shifts, diagonals, full teleports (`|Δ| >= DIM`), and
/// no-move (`Δ == 0`). Each axis's `Δ` is drawn from `[-(DIM+2), DIM+2]` so both the partial-overlap and
/// the disjoint-box (teleport) regimes are hit on every axis independently.
#[test]
fn revealed_set_equals_box_difference_over_random_pairs() {
    let mut rng = Rng::new(SEED_BASE ^ 0x2C);
    for i in 0..4096usize {
        let old_oc = rng.origin_cell();
        let d = [
            rng.offset(-(DIM + 2), DIM + 2),
            rng.offset(-(DIM + 2), DIM + 2),
            rng.offset(-(DIM + 2), DIM + 2),
        ];
        let new_oc = [old_oc[0] + d[0], old_oc[1] + d[1], old_oc[2] + d[2]];
        assert_revealed_equals_difference(old_oc, new_oc, &format!("pair#{i} (seed={SEED_BASE:#x})"));
    }
}

/// Δ == 0 reveals NOTHING (the early-out no-move case the scroll-update fast path relies on).
#[test]
fn revealed_set_zero_delta_is_empty() {
    let mut rng = Rng::new(SEED_BASE ^ 0x99);
    for _ in 0..256 {
        let oc = rng.origin_cell();
        let mut n = 0usize;
        for_each_revealed_cell(oc, oc, |_| n += 1);
        assert_eq!(n, 0, "Δ==0 at origin {oc:?} revealed {n} cells (must be empty)");
    }
}

/// `|Δ| >= DIM` on ANY axis makes the boxes disjoint on that axis ⇒ the WHOLE new box is revealed (a
/// teleport degrades to a full re-bake — never an over-skip).
#[test]
fn revealed_set_teleport_reveals_whole_new_box() {
    let mut rng = Rng::new(SEED_BASE ^ 0x7E);
    let total = (DIM * DIM * DIM) as usize;
    for _ in 0..256 {
        let old_oc = rng.origin_cell();
        // Force a teleport: shift every axis by >= DIM (positive or negative).
        let sign = |r: &mut Rng| if r.next_u64() & 1 == 0 { 1 } else { -1 };
        let new_oc = [
            old_oc[0] + sign(&mut rng) * (DIM + rng.offset(0, 8)),
            old_oc[1] + sign(&mut rng) * (DIM + rng.offset(0, 8)),
            old_oc[2] + sign(&mut rng) * (DIM + rng.offset(0, 8)),
        ];
        let mut n = 0usize;
        for_each_revealed_cell(old_oc, new_oc, |c| {
            assert!(in_box(c, new_oc), "teleport revealed an out-of-new-box cell {c:?}");
            n += 1;
        });
        assert_eq!(
            n, total,
            "teleport old={old_oc:?} new={new_oc:?} revealed {n}/{total} cells (must be the whole box)"
        );
    }
}

/// A camera WALK: a SEQUENCE of accumulating small ±1-2-cell steps (with occasional larger jumps). At
/// every step the revealed set must STILL equal the box difference against the PREVIOUS origin — the
/// streaming property is verified incrementally exactly as `scroll_update` advances `origin_cell`.
#[test]
fn revealed_set_equals_difference_over_random_walks() {
    let n_walks = 256usize;
    let steps_per_walk = 32usize;
    for w in 0..n_walks {
        let seed = SEED_BASE.wrapping_add(w as u64 * 0x1000_0193);
        let mut rng = Rng::new(seed);
        let mut oc = rng.origin_cell();
        for step in 0..steps_per_walk {
            // ~1/8 of steps are large jumps; the rest are small ±1-2 cell drifts.
            let big = rng.next_u64().is_multiple_of(8);
            let d = if big {
                [rng.offset(-(DIM + 3), DIM + 3), rng.offset(-(DIM + 3), DIM + 3), rng.offset(-(DIM + 3), DIM + 3)]
            } else {
                [rng.offset(-2, 2), rng.offset(-2, 2), rng.offset(-2, 2)]
            };
            let new_oc = [oc[0] + d[0], oc[1] + d[1], oc[2] + d[2]];
            assert_revealed_equals_difference(oc, new_oc, &format!("walk={w} step={step} seed={seed:#x}"));
            oc = new_oc;
        }
    }
}

// ═════════════════════════════════════════════════════════════════════════════
// (b3) The toroidal OVERWRITE property: revealed slots == departed slots, over random walks.
// ═════════════════════════════════════════════════════════════════════════════

/// The departed cells of a scroll (`old_box \ new_box`) mapped to their toroidal slots.
fn departed_slots(old_oc: [i32; 3], new_oc: [i32; 3]) -> BTreeSet<[u32; 3]> {
    let mut out = BTreeSet::new();
    for z in old_oc[2]..old_oc[2] + DIM {
        for y in old_oc[1]..old_oc[1] + DIM {
            for x in old_oc[0]..old_oc[0] + DIM {
                if !in_box([x, y, z], new_oc) {
                    out.insert(toroidal_slot([x, y, z]));
                }
            }
        }
    }
    out
}

/// For ANY old/new pair the REVEALED slab's slots EQUAL the DEPARTED cells' slots: an entering world cell
/// reuses exactly the storage slot a leaving cell vacated, so a scroll's slot writes never overflow the
/// fixed `DIM³` atlas (the atlas never grows — the streaming invariant). Verified over random walks incl.
/// teleports (where departed == old box, revealed == new box, and both map onto all DIM³ slots).
#[test]
fn revealed_slots_equal_departed_slots_over_random_walks() {
    let n_walks = 256usize;
    let steps_per_walk = 24usize;
    for w in 0..n_walks {
        let seed = SEED_BASE.wrapping_add(0x55 + w as u64 * 0x1000_0193);
        let mut rng = Rng::new(seed);
        let mut oc = rng.origin_cell();
        for step in 0..steps_per_walk {
            let big = rng.next_u64().is_multiple_of(6);
            let d = if big {
                [rng.offset(-(DIM + 2), DIM + 2), rng.offset(-(DIM + 2), DIM + 2), rng.offset(-(DIM + 2), DIM + 2)]
            } else {
                [rng.offset(-2, 2), rng.offset(-2, 2), rng.offset(-2, 2)]
            };
            let new_oc = [oc[0] + d[0], oc[1] + d[1], oc[2] + d[2]];

            let departed = departed_slots(oc, new_oc);
            let mut revealed: BTreeSet<[u32; 3]> = BTreeSet::new();
            for_each_revealed_cell(oc, new_oc, |c| {
                revealed.insert(toroidal_slot(c));
            });
            assert_eq!(
                revealed, departed,
                "walk={w} step={step} seed={seed:#x}: revealed slab slots != departed cell slots \
                 (old={oc:?} new={new_oc:?}) — a scroll would overwrite the wrong slot (atlas grows/corrupts)"
            );
            oc = new_oc;
        }
    }
}
