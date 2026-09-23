//! The per-row entity identity of the rigid pipeline (defect A, interim).
//!
//! A body's solver row is its position in [`physics_gather`]'s walk, and rows are NOT
//! stable across steps: a despawn swap-removes (the archetype's tail body moves into the
//! freed row), a spawn appends, and a component insert or remove migrates a body to
//! another archetype and shifts every row walked after it. Five consumers keep state
//! keyed by row from one step to the next: the sleep latch ([`IslandSleep`]), the warm
//! start tables of both solvers, the box-box axis cache ([`BoxAxisCache`]) and the
//! narrowphase's pair carry (L9, `narrowphase/carry.rs`). Without this map each of them
//! reads the state of whichever body used to hold the row.
//!
//! The gather records the [`RowKey`] of every row (the entity's slot index and generation)
//! and the rows whose `RigidBody` was added since it last ran. When the rows changed,
//! [`RowIdentity::finish_gather`] builds
//! `prev_row[r]`: the row the body now in row `r` held one gather ago, or [`NO_ROW`] for
//! a body that is new. Each consumer carries its state through that map — the latch and
//! its island contact key are permuted, warm-start and axis reads are translated — and
//! classifies itself through a [`RemapCursor`], stamped where its state becomes keyed by
//! the current rows.
//!
//! # The key is the entity, not its slot
//!
//! A row is matched on its [`RowKey`]: the entity's slot index AND generation, packed into
//! one `u64`. A body that recycles a dead body's slot has a different key, so to the
//! compare it is a different row: the aligned walk does not pair it with the dead row,
//! stage 2 misses, and it resolves to [`NO_ROW`] — a new body to every consumer (a fresh
//! latch, no warm seed, no axis hint) — whether or not the gather saw its `RigidBody` as
//! added. That covers the one gather where `is_added` is false: a deferred spawn applied
//! *inside* the physics schedule run, before the gather, is stamped `this_run + 1`, and
//! that same run's gather sees `is_added == false` (the O1 window; hazard H-03).
//!
//! The added rows keep one job: a `RigidBody` removed and re-inserted on a *live* entity
//! between two gathers keeps its key, and only the flag makes it a new body. Its O1
//! window (the re-insert applied inside the run before the gather) remains, confined to
//! that case: for that one gather the re-inserted body carries its own previous row.
//!
//! # Degrade classes of the aligned walk
//!
//! The result is always exact; these only move rows from the aligned walk to the sort +
//! binary search of stage 2.
//! * **Window.** More than [`REMAP_WINDOW`] non-added rows inserted ahead of an aligned
//!   row in one step: 17 or more bodies gaining a component whose archetype is walked
//!   earlier, or 17 or more late-flagged spawns into an archetype that is not walked
//!   last. That step's walk loses alignment. Removals never lose alignment through the
//!   window: the walk realigns past each removed row.
//! * **Budget.** The lookahead is capped at [`REMAP_BUDGET_PER_ROW`]` · (n + m)` compares.
//!   A step that removes or moves a large fraction of all bodies at once can exhaust it,
//!   and every row after that point resolves through stage 2.
//!
//! Either way the step costs at most `9 · (n + m)` sequential compares plus stage 2 over
//! the rows it could not align.
//!
//! # The stage-2 rows (T5)
//!
//! [`RowIdentity::stage2_rows`] lists, ascending, the rows of a changed gather that stage 2
//! resolved. Every other row is either new ([`NO_ROW`]) or was matched by the aligned walk
//! (E3), which consumes both cursors in step, so the rows outside the list carry a strictly
//! increasing `prev_row`. A consumer that joins a list sorted by the previous rows against one
//! sorted by the current rows (the pair carry) merges the rows outside the list in one
//! monotone pass and searches only the pairs that touch a listed row — its "jumpers".
//!
//! # Deleted by the unification rungs
//!
//! U5 (slots, no row ever moves) deletes the aligned walk, stage 2 and the walk budget.
//! U6 moves the latch into `BodyGate` and deletes [`SleepLatch`]; the island contact key
//! moves with it (proposed to the unification plan as a separate `BodySleepKey`). U7
//! (`PairCache`) deletes the rest, and this file with it; its per-pair contact signal also
//! makes the island contact key deletable.
//!
//! [`physics_gather`]: crate::systems::physics_gather
//! [`IslandSleep`]: crate::resources::IslandSleep
//! [`BoxAxisCache`]: crate::narrowphase::axis_cache::BoxAxisCache

use std::ops::Range;
use std::sync::atomic::{AtomicU64, Ordering};

use boyko_ecs::ecs::core::component::scratch::{ScratchBuildView, ScratchColumn};
use boyko_ecs::ecs::core::entity::entity::Entity;

use crate::manifold::{Manifold, SDF_SENTINEL};
use crate::scratch_ids::{
    register_scratch_layouts, row_added_id, row_entity_id, row_entity_prev_id, row_prev_id,
    row_remap_sort_id, row_stage2_id, scratch_reserve_rows,
};

/// The `prev_row` value of a body that held no row one gather ago, or whose `RigidBody`
/// was just added.
///
/// Rows stay below `2^24` (the warm-start key invariant), so `u32::MAX` is never a row.
pub(crate) const NO_ROW: u32 = u32::MAX;

/// The island contact key of a row that had no island at the last
/// [`IslandSleep::begin_step`](crate::resources::IslandSleep::begin_step), and the key of a
/// row whose body held no row one gather ago.
///
/// A live key is the number of manifolds filed under one island, and the graph's island
/// manifold column holds at most `POOL_MAX_ROWS` (at most `2^24`) entries, so a live key
/// never equals this value.
pub(crate) const NO_ISLAND_KEY: u32 = u32::MAX;

/// Cold-path mark in `prev_row` for a row the aligned walk could not resolve. Stage 2
/// replaces every one, so none survives [`RowIdentity::finish_gather`].
pub(crate) const SEARCH: u32 = u32::MAX - 1;

/// Lookahead distance of the aligned walk, in rows.
pub(crate) const REMAP_WINDOW: usize = 16;

/// Lookahead compares the aligned walk may spend per row of `n + m`; every examined row
/// costs one, added rows included.
///
/// 8 rather than 4: a 16-row burst ahead of an aligned row spends 136 compares on the
/// current side and `16 · 16 = 256` on the previous side, which must fit the budget of
/// a world as small as `n + m ≈ 84`.
pub(crate) const REMAP_BUDGET_PER_ROW: usize = 8;

/// Bits of a gather sequence owned by one [`RowIdentity`]: `2^40` gathers per instance
/// (34.8 years at 1 kHz), over `2^24` instances.
pub(crate) const SEQ_BITS: u32 = 40;

/// Marks a stage-2 pool entry as consumed, in debug builds only.
const CONSUMED: u32 = 1 << 31;

/// Process-wide source of disjoint gather-sequence ranges.
///
/// Touched once per [`RowIdentity`] construction (setup), never per step. It starts at
/// `1`, so every `base` is at least `2^40` and `0` is never a valid stamp.
static ROW_IDENTITY_EPOCH: AtomicU64 = AtomicU64::new(1);

/// The identity a row is matched on across gathers: the entity's slot index in the high
/// 32 bits, its generation in the low 32. One `u64`, so a compare is one instruction and
/// [`keys_equal`] folds one lane per row.
///
/// Invariant: slot index `< 2^32`. A `2^32`-slot entity fast store is 64 GiB of
/// reservation (the default holds `2^26`); [`of`](Self::of) debug-asserts it per row, and a
/// release build past it would alias slots `2^32` apart, never read out of bounds. A
/// generation wrap (`2^32` recycles of one slot) is the kernel's own ABA window, inherited
/// unchanged.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct RowKey(u64);

impl RowKey {
    /// The key of a live entity handle.
    #[inline]
    pub(crate) fn of(entity: Entity) -> Self {
        let id = entity.id().0;
        debug_assert!(
            id <= u32::MAX as usize,
            "invariant: an entity slot index fits 32 bits (a 2^32-slot fast store is 64 GiB)"
        );
        Self(((id as u64) << 32) | u64::from(entity.generation()))
    }

    /// Test scripts: the key of slot `id` at `generation`.
    #[cfg(test)]
    pub(crate) const fn new(id: usize, generation: u32) -> Self {
        Self(((id as u64) << 32) | generation as u64)
    }

    /// The slot index half of the key.
    #[cfg(test)]
    pub(crate) fn id(self) -> usize {
        (self.0 >> 32) as usize
    }

    /// The generation half of the key.
    #[cfg(test)]
    pub(crate) fn generation(self) -> u32 {
        // Truncation is the point: the low 32 bits are the generation.
        self.0 as u32
    }
}

const _: () = assert!(
    size_of::<RowKey>() == 8 && align_of::<RowKey>() == 8,
    "RowKey is one u64: the 8 B, align 8 row id element"
);

/// How a row-keyed consumer's state relates to the rows of the current gather.
#[derive(Clone, Copy, Debug)]
pub(crate) enum RowRemap<'a> {
    /// The consumer's rows are this gather's rows: read and write in place.
    Identity,
    /// The rows changed since the consumer's state was keyed, one gather ago:
    /// `prev_row[r]` is the row the body now in row `r` held then, or [`NO_ROW`].
    Rows(&'a [u32]),
    /// The consumer missed at least one gather, or its stamp belongs to another
    /// [`RowIdentity`]: no row of its state can be trusted.
    Reset,
}

impl RowRemap<'_> {
    /// The row the body now in row `r` held when the consumer's state was keyed, or
    /// `None` when it held none.
    #[inline]
    pub(crate) fn row(self, r: u32) -> Option<u32> {
        match self {
            Self::Identity => Some(r),
            Self::Rows(prev_row) => {
                let p = prev_row[r as usize];
                (p != NO_ROW).then_some(p)
            }
            Self::Reset => None,
        }
    }

    /// The rows the current pair `(a, b)` held when the consumer's state was keyed, when
    /// both bodies held one and the two rows kept their order.
    ///
    /// A pair whose order flipped returns `None`: its feature ids now name swapped roles,
    /// so even a hit would read the wrong impulses.
    #[inline]
    pub(crate) fn pair(self, a: u32, b: u32) -> Option<(u32, u32)> {
        match self {
            Self::Identity => Some((a, b)),
            Self::Rows(prev_row) => {
                let pa = prev_row[a as usize];
                let pb = prev_row[b as usize];
                // `pa < pb` also rules out `pa == NO_ROW`, the largest `u32`.
                (pb != NO_ROW && pa < pb).then_some((pa, pb))
            }
            Self::Reset => None,
        }
    }

    /// The rows manifold `m`'s bodies held when a warm table was keyed: the body pair
    /// through [`pair`](Self::pair), or body A alone through [`row`](Self::row) for an SDF
    /// contact, whose `body_b` is the sentinel and is kept as is.
    #[inline]
    pub(crate) fn manifold_pair(self, m: &Manifold) -> Option<(u32, u32)> {
        if m.body_b == SDF_SENTINEL {
            self.row(m.body_a.0).map(|pa| (pa, m.body_b.0))
        } else {
            self.pair(m.body_a.0, m.body_b.0)
        }
    }
}

/// One row's sleep latch and island contact key: the element of `IslandSleep`'s permute
/// carry. 8 B, align 4.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub(crate) struct SleepLatch {
    /// The number of manifolds filed under the row's island at the last `begin_step`, or
    /// [`NO_ISLAND_KEY`]. First, so the struct has no interior padding.
    pub(crate) island_key: u32,
    /// Consecutive frames the row's island has been below the sleep threshold.
    pub(crate) below_count: u16,
    /// Whether the row is latched asleep.
    pub(crate) asleep: bool,
    _pad: u8,
}

impl SleepLatch {
    /// A row whose body held no row one gather ago: awake, no debounce, no key.
    pub(crate) const FRESH: Self = Self::new(0, false, NO_ISLAND_KEY);

    /// A latch holding `below_count`, `asleep` and `island_key`.
    #[inline]
    pub(crate) const fn new(below_count: u16, asleep: bool, island_key: u32) -> Self {
        Self {
            island_key,
            below_count,
            asleep,
            _pad: 0,
        }
    }
}

const _: () = assert!(
    size_of::<SleepLatch>() == 8 && align_of::<SleepLatch>() == 4,
    "SleepLatch is the 8 B, align 4 permute carry element"
);

/// One row-keyed consumer's place in the gather sequence. 16 B, `Copy`, no heap.
///
/// The protocol rests on one invariant, **P**: `synced_seq == s` ⇔ the consumer's
/// row-keyed state is keyed by the rows of gather `s`. [`remap`](Self::remap) classifies
/// and never stamps; [`stamp`](Self::stamp) is written once, where P becomes true, as a
/// postcondition — never inside a match arm.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct RemapCursor {
    /// The gather the consumer's state is keyed by; `0` means never stamped.
    synced_seq: u64,
    /// `Reset` classifications of a cursor that had been stamped (`synced_seq != 0`).
    resets: u64,
}

impl RemapCursor {
    /// Classifies the consumer against the current gather. Counts a `Reset` of a cursor
    /// that had been stamped; never stamps.
    #[inline]
    pub(crate) fn remap<'a>(&mut self, rows: &'a RowIdentity) -> RowRemap<'a> {
        let remap = rows.classify(self.synced_seq);
        // A never-stamped consumer holds no row-keyed state, so its Reset loses nothing.
        if matches!(remap, RowRemap::Reset) && self.synced_seq != 0 {
            self.resets += 1;
        }
        remap
    }

    /// Records that the consumer's state is now keyed by the current gather's rows.
    #[inline]
    pub(crate) fn stamp(&mut self, rows: &RowIdentity) {
        self.synced_seq = rows.gather_seq;
    }

    /// `Reset` classifications of this cursor after its first stamp.
    #[inline]
    pub(crate) fn resets(&self) -> u64 {
        self.resets
    }

    /// Test hook: the gather this cursor was last stamped with, `0` if never, so
    /// `resources_tests.rs` can assert that every arm of `IslandSleep::rekey_rows` stamps.
    #[cfg(test)]
    #[inline]
    pub(crate) fn synced_seq(&self) -> u64 {
        self.synced_seq
    }
}

/// Per-solve diagnostic of a solver's warm-start lookup side. 32 B, no padding, one per
/// solver.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WarmSeedStats {
    /// Manifolds whose points were pushed this solve.
    pub manifolds: u32,
    /// Of those, the manifolds whose lookup key resolved to rows of the previous gather:
    /// all of them on a step whose rows are unchanged, the carried ones on a step whose
    /// rows changed, and `0` after a `Reset` or with warm start disabled.
    pub translated: u32,
    /// Contact points pushed this solve: the denominator of [`point_hits`](Self::point_hits).
    pub points: u32,
    /// Of those, the points whose read key found an entry in the previous step's warm table,
    /// so the points that were actually warm-started. `translated` counts manifolds whose
    /// lookup was ATTEMPTED; this counts per-point lookups that SUCCEEDED. A hit may still
    /// seed a zero impulse (the entry stored one). `0` after a `Reset` or with warm start
    /// disabled.
    pub point_hits: u32,
    /// Live contact points of the manifolds FROZEN this solve (the colored solver's sleep
    /// skip): points that are not solved, whose warm entries the solver carries into the
    /// next table instead. `0` with sleeping off or warm start disabled, and always for the
    /// reference solver, which has no sleep path.
    pub carry_points: u32,
    /// Of those, the points whose entry was found in the previous step's table and
    /// re-inserted under this gather's rows, so a frozen island wakes warm. A miss (a pair
    /// whose row order flipped, a `Reset`, a feature id that changed) drops the entry, and
    /// the island then wakes without it.
    pub carry_hits: u32,
    /// This solver's warm-start cursor `Reset` count. The cursor is never classified or
    /// stamped while warm start is disabled.
    pub remap_resets: u64,
}

const _: () = assert!(
    size_of::<WarmSeedStats>() == 32,
    "WarmSeedStats is six u32 counters and one u64, with no padding"
);

/// Test-only walk counters. Outside `cfg(test)` this is a zero-sized type whose methods
/// are empty, so the walk carries no counting cost in a shipping build.
///
/// Both profiles share one impl whose counting statements are `#[cfg(test)]`-gated: a
/// second impl under a negated `test` predicate would be read as test-only by
/// `tests/production_reachability_census.rs`, which refuses that predicate in the tree.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct WalkCounters {
    /// Branches E1–E6 of the aligned walk, in that order.
    #[cfg(test)]
    pub(crate) branches: [u64; 6],
    /// Walks whose lookahead budget ran out.
    #[cfg(test)]
    pub(crate) budget_exhaustions: u64,
    /// Walks that left the bounded loop with rows still unvisited (a progress defect).
    #[cfg(test)]
    pub(crate) walk_stalls: u64,
}

impl WalkCounters {
    /// Counts one pass through branch `E(e + 1)` of the aligned walk.
    #[inline]
    fn branch(&mut self, e: usize) {
        #[cfg(test)]
        {
            self.branches[e] += 1;
        }
        // The only other reader of `e` is the gated statement above.
        let _ = e;
    }

    /// Counts one walk whose lookahead budget ran out, if `exhausted`.
    #[inline]
    fn exhaustion(&mut self, exhausted: bool) {
        #[cfg(test)]
        {
            self.budget_exhaustions += u64::from(exhausted);
        }
        // The only other reader of `exhausted` is the gated statement above.
        let _ = exhausted;
    }

    /// Counts one walk that left the bounded loop with rows still unvisited.
    #[inline]
    fn stall(&mut self) {
        #[cfg(test)]
        {
            self.walk_stalls += 1;
        }
    }
}

/// The gather's per-row entity identity and the previous-row map built from it. One
/// instance, inside [`SolverScratch`](crate::resources::SolverScratch).
pub(crate) struct RowIdentity {
    /// The current gather's sequence number; read once per consumer per step.
    gather_seq: u64,
    /// `epoch << SEQ_BITS`: this instance's first sequence value, held while it has never
    /// gathered.
    base: u64,
    /// Whether the current gather's rows equal the previous gather's and none was added.
    stable: bool,
    /// Row → [`RowKey`] of the current gather, in gather order. 8 B/row.
    cur: ScratchColumn<RowKey>,
    /// Row → [`RowKey`] of the previous gather; swapped with `cur` at each gather. 8 B/row.
    prev: ScratchColumn<RowKey>,
    /// Rows whose `RigidBody` was added, ascending. Empty in the steady state.
    added_rows: ScratchColumn<u32>,
    /// `prev_row[r]` for the current gather; valid iff `!stable`. 4 B/row.
    prev_row: ScratchColumn<u32>,
    /// Stage 2's pool of previous rows the aligned walk left unconsumed. Cold only.
    sort_buf: ScratchColumn<(RowKey, u32)>,
    /// The current rows stage 2 resolved, ascending (T5); valid iff `!stable`. Cold only:
    /// written by the build, read on the steps whose rows changed.
    stage2: ScratchColumn<u32>,
    /// Previous-row maps built (steps whose rows changed). Structural diagnostic.
    remap_builds: u64,
    /// Current rows resolved by stage 2. Structural cost diagnostic.
    remap_searched: u64,
    /// Test-only walk counters; zero-sized in a shipping build.
    walk: WalkCounters,
}

impl RowIdentity {
    /// Builds an empty identity map pre-sized for `rows` bodies, taking a fresh
    /// gather-sequence range.
    pub(crate) fn with_capacity(rows: usize) -> Self {
        register_scratch_layouts();
        // Relaxed: uniqueness needs only the atomicity of this one read-modify-write, and
        // no other atomic is reasoned about together with it. Setup only, never on a step.
        let epoch = ROW_IDENTITY_EPOCH.fetch_add(1, Ordering::Relaxed);
        debug_assert!(
            epoch < 1 << 24,
            "invariant: at most 2^24 RowIdentity instances per process, or sequence ranges alias"
        );
        let base = epoch << SEQ_BITS;
        let id_reserve = rows.max(scratch_reserve_rows(size_of::<RowKey>()));
        let row_reserve = rows.max(scratch_reserve_rows(size_of::<u32>()));
        let sort_reserve = rows.max(scratch_reserve_rows(size_of::<(RowKey, u32)>()));
        Self {
            gather_seq: base,
            base,
            stable: true,
            cur: ScratchColumn::new(row_entity_id(), id_reserve),
            prev: ScratchColumn::new(row_entity_prev_id(), id_reserve),
            added_rows: ScratchColumn::new(row_added_id(), row_reserve),
            prev_row: ScratchColumn::new(row_prev_id(), row_reserve),
            sort_buf: ScratchColumn::new(row_remap_sort_id(), sort_reserve),
            stage2: ScratchColumn::new(row_stage2_id(), row_reserve),
            remap_builds: 0,
            remap_searched: 0,
            walk: WalkCounters::default(),
        }
    }

    /// Opens a gather: the current ids become the previous ones, and the sequence advances.
    pub(crate) fn begin_gather(&mut self) {
        core::mem::swap(&mut self.cur, &mut self.prev);
        self.cur.build_view().clear();
        self.added_rows.build_view().clear();
        self.gather_seq += 1;
        debug_assert!(
            self.gather_seq - self.base < 1 << SEQ_BITS,
            "invariant: a RowIdentity gathers at most 2^SEQ_BITS times"
        );
    }

    /// The refill views the gather pushes into: one [`RowKey`] per row, and the row index
    /// of every row whose `RigidBody` was added, in ascending order.
    #[inline]
    pub(crate) fn gather_views(
        &mut self,
    ) -> (ScratchBuildView<'_, RowKey>, ScratchBuildView<'_, u32>) {
        (self.cur.build_view(), self.added_rows.build_view())
    }

    /// Closes a gather: decides whether the rows are unchanged and, when they are not,
    /// builds the previous-row map.
    pub(crate) fn finish_gather(&mut self) {
        let cur = self.cur.as_read_slice();
        let prev = self.prev.as_read_slice();
        debug_assert!(
            self.added_rows
                .as_read_slice()
                .windows(2)
                .all(|w| w[0] < w[1]),
            "invariant: added rows are pushed in ascending gather order"
        );
        // The added term catches a `RigidBody` removed and re-inserted on one live entity
        // between two gathers, whose key the compare alone calls unchanged. A recycled slot
        // needs no flag: its key differs from the dead body's.
        let stable = self.added_rows.is_empty() && cur.len() == prev.len() && keys_equal(cur, prev);
        self.stable = stable;
        if !stable {
            self.build_prev_row();
        }
    }

    /// The number of rows in the current gather.
    #[inline]
    pub(crate) fn rows_len(&self) -> usize {
        self.cur.len()
    }

    /// The current gather's sequence number: the stamp a consumer compares with a stamp it
    /// recorded, such as the pair list's (`ContactPairs`, L9).
    #[inline]
    pub(crate) fn gather_seq(&self) -> u64 {
        self.gather_seq
    }

    /// The rows of the current gather that stage 2 resolved, ascending (T5), or none when the
    /// rows are unchanged. Every row outside the list is new or was aligned, so the rows
    /// outside it that held a row carry a strictly increasing `prev_row` (module docs, "The
    /// stage-2 rows").
    #[inline]
    pub(crate) fn stage2_rows(&self) -> &[u32] {
        if self.stable {
            &[]
        } else {
            self.stage2.as_read_slice()
        }
    }

    /// Previous-row maps built so far.
    #[inline]
    pub(crate) fn remap_builds(&self) -> u64 {
        self.remap_builds
    }

    /// Current rows resolved by stage 2 so far.
    #[inline]
    pub(crate) fn remap_searched(&self) -> u64 {
        self.remap_searched
    }

    /// Classifies a consumer whose cursor holds `synced_seq`. Total over every `u64`.
    #[inline]
    fn classify(&self, synced_seq: u64) -> RowRemap<'_> {
        // This instance never gathered: direct drive, whose rows are whatever the caller
        // put in the snapshot.
        if self.gather_seq == self.base {
            return RowRemap::Identity;
        }
        // A never-stamped consumer holds no row-keyed state.
        let synced = if synced_seq == 0 {
            self.base
        } else {
            synced_seq
        };
        if synced == self.gather_seq {
            return RowRemap::Identity;
        }
        if synced.wrapping_add(1) == self.gather_seq {
            return if self.stable {
                RowRemap::Identity
            } else {
                RowRemap::Rows(self.prev_row.as_read_slice())
            };
        }
        // A missed gather, or a stamp from another RowIdentity's range.
        RowRemap::Reset
    }

    #[cold]
    #[inline(never)]
    fn build_prev_row(&mut self) {
        self.remap_builds += 1;
        let mut prev_row = self.prev_row.build_view();
        let mut pool = self.sort_buf.build_view();
        let mut stage2 = self.stage2.build_view();
        let searched = build_prev_row_into(
            self.cur.as_read_slice(),
            self.prev.as_read_slice(),
            self.added_rows.as_read_slice(),
            &mut prev_row,
            &mut pool,
            &mut stage2,
            &mut self.walk,
        );
        self.remap_searched += searched as u64;
    }
}

/// Whether two key slices of equal length hold the same keys, as a branch-free OR fold
/// with no early exit so it vectorises.
#[inline]
fn keys_equal(cur: &[RowKey], prev: &[RowKey]) -> bool {
    cur.iter()
        .zip(prev)
        .fold(0u64, |acc, (c, p)| acc | (c.0 ^ p.0))
        == 0
}

/// Builds `prev_row` for `cur` against `prev`, lists the rows stage 2 resolved in `stage2`
/// (ascending), and returns their number.
///
/// **Stage 1, aligned walk.** Cursor `i` over `prev` (length `m`), `j` over `cur` (length
/// `n`), `a` over `added` (ascending). Both lookahead offsets are distances of at least 1
/// from their cursor. Every iteration advances `j` (E1, E2, E6), both cursors (E3), or `i`
/// by at least 1 (E4, E5), so `n + m` iterations always reach `j == n`.
///
/// **Stage 2.** The previous rows the walk did not consume are sorted by key, and every
/// row the walk marked [`SEARCH`] takes its old row by binary search, or [`NO_ROW`].
///
/// A previous row is consumed only on exact key equality with a non-added current row, and
/// keys are unique within a gather (one live entity per slot), so the result is exact
/// whatever the walk resolves.
fn build_prev_row_into(
    cur: &[RowKey],
    prev: &[RowKey],
    added: &[u32],
    prev_row: &mut ScratchBuildView<'_, u32>,
    pool: &mut ScratchBuildView<'_, (RowKey, u32)>,
    stage2: &mut ScratchBuildView<'_, u32>,
    walk: &mut WalkCounters,
) -> usize {
    let n = cur.len();
    let m = prev.len();
    prev_row.clear();
    prev_row.resize(n, NO_ROW);
    pool.clear();
    stage2.clear();
    let out = prev_row.as_mut_slice();

    let mut budget = REMAP_BUDGET_PER_ROW * (n + m);
    let mut exhausted = false;
    let mut searched = 0usize;
    let (mut i, mut j, mut a) = (0usize, 0usize, 0usize);
    for _ in 0..n + m {
        if j == n {
            break;
        }
        // E1: a new body. Final NO_ROW; it consumes no previous row.
        if a < added.len() && added[a] as usize == j {
            walk.branch(0);
            out[j] = NO_ROW;
            a += 1;
            j += 1;
            continue;
        }
        // E2: the previous rows are exhausted, or the lookahead budget is.
        if i == m || budget == 0 {
            walk.branch(1);
            exhausted |= i != m;
            out[j] = SEARCH;
            searched += 1;
            j += 1;
            continue;
        }
        // E3: aligned.
        if prev[i] == cur[j] {
            walk.branch(2);
            out[j] = i as u32;
            i += 1;
            j += 1;
            continue;
        }
        // E4: prev[i] is gone, or lies beyond the window.
        let Some(db) = scan_cur(cur, &added[a..], j, prev[i], &mut budget, &mut exhausted) else {
            walk.branch(3);
            push_pool(pool, prev, i..i + 1);
            i += 1;
            continue;
        };
        // E5: `da` previous rows were removed or moved away before cur[j].
        if let Some(da) = scan_prev(prev, i, cur[j], &mut budget, &mut exhausted)
            && da < db
        {
            walk.branch(4);
            push_pool(pool, prev, i..i + da);
            i += da;
            continue;
        }
        // E6: cur[j] moved in from elsewhere (ties land here).
        walk.branch(5);
        out[j] = SEARCH;
        searched += 1;
        j += 1;
    }
    if j < n {
        // Unreachable while every branch makes progress. A defect that breaks progress
        // exits the bounded loop instead of hanging the gather, and the tail below still
        // sends every remaining row to stage 2, so the map stays exact in release.
        walk.stall();
        debug_assert!(
            false,
            "row remap walk: progress bound violated (i={i}, j={j}, n={n}, m={m})"
        );
        while j < n {
            if a < added.len() && added[a] as usize == j {
                a += 1;
                out[j] = NO_ROW;
            } else {
                out[j] = SEARCH;
                searched += 1;
            }
            j += 1;
        }
    }
    push_pool(pool, prev, i..m);
    walk.exhaustion(exhausted);

    if searched != 0 {
        let pool = pool.as_mut_slice();
        // Keys are unique within one gather, so the order is total and the result is
        // deterministic.
        pool.sort_unstable_by_key(|&(key, _)| key);
        for (r, slot) in out.iter_mut().enumerate() {
            if *slot != SEARCH {
                continue;
            }
            stage2.push(r as u32);
            *slot = match pool.binary_search_by_key(&cur[r], |&(key, _)| key) {
                Ok(hit) => take_pool_row(pool, hit),
                Err(_) => NO_ROW,
            };
        }
    }

    debug_assert!(
        out.iter().all(|&p| p != SEARCH),
        "invariant: stage 2 resolves every SEARCH row"
    );
    debug_assert_eq!(stage2.len(), searched, "invariant: stage 2 lists every row it resolved");
    debug_assert!(
        out.iter().all(|&p| p == NO_ROW || (p as usize) < m),
        "invariant: every previous row names a row of the previous gather"
    );
    searched
}

/// Distance `k - j` to the first `k > j` with `cur[k] == target`, over at most
/// [`REMAP_WINDOW`] non-added rows. `added` is the unconsumed tail of the added rows, all
/// greater than `j`. Charges one budget unit per examined row, added rows included.
#[inline]
fn scan_cur(
    cur: &[RowKey],
    added: &[u32],
    j: usize,
    target: RowKey,
    budget: &mut usize,
    exhausted: &mut bool,
) -> Option<usize> {
    debug_assert!(
        added.first().is_none_or(|&r| r as usize > j),
        "invariant: every unconsumed added row lies after the walk's current row"
    );
    let mut skip = added.iter().map(|&r| r as usize).peekable();
    let mut examined = 0usize;
    for (offset, &key) in cur[j + 1..].iter().enumerate() {
        if examined == REMAP_WINDOW {
            return None;
        }
        if *budget == 0 {
            *exhausted = true;
            return None;
        }
        *budget -= 1;
        let k = j + 1 + offset;
        if skip.peek() == Some(&k) {
            skip.next();
            continue;
        }
        examined += 1;
        if key == target {
            return Some(k - j);
        }
    }
    None
}

/// Distance `k - i` to the first `k` in `i + 1 ..= min(i + REMAP_WINDOW, m - 1)` with
/// `prev[k] == target`. Requires `i < m`. Charges one budget unit per examined row.
#[inline]
fn scan_prev(
    prev: &[RowKey],
    i: usize,
    target: RowKey,
    budget: &mut usize,
    exhausted: &mut bool,
) -> Option<usize> {
    let end = (i + REMAP_WINDOW + 1).min(prev.len());
    for (offset, &key) in prev[i + 1..end].iter().enumerate() {
        if *budget == 0 {
            *exhausted = true;
            return None;
        }
        *budget -= 1;
        if key == target {
            return Some(offset + 1);
        }
    }
    None
}

/// Pushes the previous rows `rows` into stage 2's pool as `(key, row)`.
#[inline]
fn push_pool(pool: &mut ScratchBuildView<'_, (RowKey, u32)>, prev: &[RowKey], rows: Range<usize>) {
    let start = rows.start;
    for (offset, &key) in prev[rows].iter().enumerate() {
        pool.push((key, (start + offset) as u32));
    }
}

/// The previous row of pool entry `hit`; in debug builds, also marks it consumed and
/// asserts it was not consumed before.
#[inline]
fn take_pool_row(pool: &mut [(RowKey, u32)], hit: usize) -> u32 {
    let entry = &mut pool[hit].1;
    let row = *entry & !CONSUMED;
    if cfg!(debug_assertions) {
        debug_assert!(
            *entry & CONSUMED == 0,
            "invariant: no previous row is used twice"
        );
        *entry |= CONSUMED;
    }
    row
}

#[cfg(test)]
mod tests {
    //! T6, T6c and T12 of the defect A interim fix (Design v1 + Rev 2 + Rev 3), the
    //! `RowRemap` lookups, and U1/U2 of A1b (the key carries the generation, hazard H-03).
    //! Every test here is device-free and allocation-light, so it runs under Miri as it is
    //! (the property test shrinks its case count there).

    use std::cell::Cell;

    use proptest::prelude::*;

    use super::*;

    /// Walk ids of the fixed traces, after the bodies of `row_keyed_state_defect_a.rs`.
    const F: usize = 100;
    const P1: usize = 101;
    const P2: usize = 102;
    const P3: usize = 103;
    const P4: usize = 104;
    const D: usize = 105;
    const E_FRESH: usize = 106;
    const C: usize = 107;
    const M: usize = 110;
    /// `NO_ROW`, short enough for a literal map.
    const X: u32 = NO_ROW;

    /// The keys of `ids` at generation 0: the fixed scripts name slots only.
    fn keys(ids: &[usize]) -> Vec<RowKey> {
        ids.iter().map(|&id| RowKey::new(id, 0)).collect()
    }

    /// Feeds one gather: `ids` (slots at generation 0) in walk order and the ascending rows
    /// whose `RigidBody` was added.
    fn gather(rows: &mut RowIdentity, ids: &[usize], added: &[u32]) {
        gather_keys(rows, &keys(ids), added);
    }

    /// Feeds one gather: `keys` in walk order and the ascending rows whose `RigidBody` was
    /// added.
    fn gather_keys(rows: &mut RowIdentity, keys: &[RowKey], added: &[u32]) {
        rows.begin_gather();
        {
            let (mut cur, mut add) = rows.gather_views();
            for &key in keys {
                cur.push(key);
            }
            for &r in added {
                add.push(r);
            }
        }
        rows.finish_gather();
    }

    /// Every counter the walk keeps, read at one moment.
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    struct Counts {
        /// E1..E6.
        branches: [u64; 6],
        exhaustions: u64,
        stalls: u64,
        searched: u64,
        builds: u64,
    }

    impl Counts {
        fn of(rows: &RowIdentity) -> Self {
            Self {
                branches: rows.walk.branches,
                exhaustions: rows.walk.budget_exhaustions,
                stalls: rows.walk.walk_stalls,
                searched: rows.remap_searched,
                builds: rows.remap_builds,
            }
        }

        fn since(self, before: Self) -> Self {
            let mut branches = [0u64; 6];
            for (d, (now, then)) in branches
                .iter_mut()
                .zip(self.branches.iter().zip(before.branches))
            {
                *d = now - then;
            }
            Self {
                branches,
                exhaustions: self.exhaustions - before.exhaustions,
                stalls: self.stalls - before.stalls,
                searched: self.searched - before.searched,
                builds: self.builds - before.builds,
            }
        }

        fn plus(self, other: Self) -> Self {
            let mut branches = self.branches;
            for (sum, add) in branches.iter_mut().zip(other.branches) {
                *sum += add;
            }
            Self {
                branches,
                exhaustions: self.exhaustions + other.exhaustions,
                stalls: self.stalls + other.stalls,
                searched: self.searched + other.searched,
                builds: self.builds + other.builds,
            }
        }
    }

    /// The previous-row map as the design defines it, computed with no walk at all:
    /// `NO_ROW` for an added row, otherwise the row the same key (slot AND generation) held
    /// in `prev`, or `NO_ROW`. Also returns the `stable` decision.
    // `clippy::disallowed_types`: test oracle. The std `HashMap` is the independent reference
    // model the aligned walk and stage 2 are checked against; it exists in test builds only.
    #[allow(clippy::disallowed_types)]
    fn oracle(prev: &[RowKey], cur: &[RowKey], added: &[u32]) -> (bool, Vec<u32>) {
        let row_of: std::collections::HashMap<RowKey, u32> = prev
            .iter()
            .enumerate()
            .map(|(r, &key)| (key, r as u32))
            .collect();
        let stable = added.is_empty() && prev == cur;
        let map = cur
            .iter()
            .enumerate()
            .map(|(r, key)| {
                if added.contains(&(r as u32)) {
                    NO_ROW
                } else {
                    row_of.get(key).copied().unwrap_or(NO_ROW)
                }
            })
            .collect();
        (stable, map)
    }

    /// The map `finish_gather` published: the identity on a stable gather (where `prev_row`
    /// is not maintained), otherwise `prev_row`.
    fn published_map(rows: &RowIdentity) -> Vec<u32> {
        if rows.stable {
            (0..rows.rows_len() as u32).collect()
        } else {
            rows.prev_row.as_read_slice().to_vec()
        }
    }

    /// T5: `stage2_rows` lists, ascending, exactly the `searched` rows stage 2 resolved (none on
    /// a stable gather), and every row outside the list that held a row carries a strictly
    /// increasing `prev_row` — the property the pair carry's merge-join rests on
    /// (`narrowphase/carry.rs`).
    fn check_stage2_rows(rows: &RowIdentity, map: &[u32], searched: u64) -> Result<(), String> {
        let listed = rows.stage2_rows();
        if rows.stable && !listed.is_empty() {
            return Err(format!("a stable gather lists stage-2 rows {listed:?}"));
        }
        if listed.len() as u64 != searched {
            return Err(format!(
                "stage2_rows {listed:?} lists {} rows, stage 2 resolved {searched}",
                listed.len()
            ));
        }
        if !listed.windows(2).all(|w| w[0] < w[1])
            || listed.iter().any(|&r| r as usize >= map.len())
        {
            return Err(format!(
                "stage2_rows {listed:?} is not an ascending list of rows below {}",
                map.len()
            ));
        }
        let mut last: Option<u32> = None;
        for (r, &p) in map.iter().enumerate() {
            if p == NO_ROW || listed.binary_search(&(r as u32)).is_ok() {
                continue;
            }
            if last.is_some_and(|l| p <= l) {
                return Err(format!(
                    "row {r} is outside stage2_rows {listed:?} but its prev_row {p} does not \
                     rise above the previous such row's {last:?} (map {map:?})"
                ));
            }
            last = Some(p);
        }
        Ok(())
    }

    /// [`check_gather_keys`] over slots at generation 0.
    fn check_gather(
        rows: &mut RowIdentity,
        prev: &[usize],
        cur: &[usize],
        added: &[u32],
    ) -> Result<(), String> {
        check_gather_keys(rows, &keys(prev), &keys(cur), added)
    }

    /// Feeds one gather of `cur` after `prev` and checks it against the oracle: the `stable`
    /// decision, the published map, what a cursor stamped one gather ago classifies as, and
    /// that the walk made progress.
    fn check_gather_keys(
        rows: &mut RowIdentity,
        prev: &[RowKey],
        cur: &[RowKey],
        added: &[u32],
    ) -> Result<(), String> {
        let mut cursor = RemapCursor::default();
        cursor.stamp(rows);
        let searched_before = rows.remap_searched;
        gather_keys(rows, cur, added);
        let (stable, expected) = oracle(prev, cur, added);
        if rows.stable != stable {
            return Err(format!("stable {} (oracle {stable})", rows.stable));
        }
        let got = published_map(rows);
        if got != expected {
            return Err(format!("prev_row {got:?} (oracle {expected:?})"));
        }
        check_stage2_rows(rows, &got, rows.remap_searched - searched_before)?;
        match cursor.remap(rows) {
            RowRemap::Identity if stable => {}
            RowRemap::Rows(map) if !stable && map == expected.as_slice() => {}
            other => {
                return Err(format!(
                    "a cursor stamped one gather ago classified {other:?} (oracle stable {stable})"
                ));
            }
        }
        if rows.walk.walk_stalls != 0 {
            return Err(format!(
                "walk_stalls {} (the bounded walk left rows unvisited)",
                rows.walk.walk_stalls
            ));
        }
        Ok(())
    }

    /// One hand trace: the walks either side of a structural change and what the walk must
    /// produce for them.
    struct Trace {
        name: &'static str,
        prev: &'static [usize],
        cur: &'static [usize],
        added: &'static [u32],
        prev_row: &'static [u32],
        searched: u64,
        /// E1..E6 taken by the change gather.
        branches: [u64; 6],
    }

    /// T6 fixed cases: the A1 (both orders), A1c, A2 and A3 id scripts of Rev 3's P18 trace
    /// table, each with its literal `prev_row`, stage-2 count and branch counts, traced by
    /// hand through `build_prev_row_into`. Red under M10 (`REMAP_WINDOW = 0`: A1c, A2 and A3
    /// resolve more rows by stage 2) and under M12 (offsets from 0: A3's E5 stalls with
    /// `i = 0, j = 0`, a debug-assert panic, and `walk_stalls == 1` in any build).
    #[test]
    fn walk_matches_the_hand_traces_of_the_red_scenes() {
        let traces = [
            Trace {
                name: "A1 delete then spawn (E recycles P2's id)",
                prev: &[F, P1, P2, P3, P4],
                cur: &[F, P1, P4, P3, P2],
                added: &[4],
                prev_row: &[0, 1, 4, 3, X],
                searched: 1,
                branches: [1, 0, 3, 1, 0, 1],
            },
            Trace {
                name: "A1 spawn then delete",
                prev: &[F, P1, P2, P3, P4],
                cur: &[F, P1, E_FRESH, P3, P4],
                added: &[2],
                prev_row: &[0, 1, X, 3, 4],
                searched: 0,
                branches: [1, 0, 4, 1, 0, 0],
            },
            Trace {
                name: "A1c (C swap-moves into P2's row)",
                prev: &[F, P1, P2, P3, P4, C],
                cur: &[F, P1, C, P3, P4],
                added: &[],
                prev_row: &[0, 1, 5, 3, 4],
                searched: 1,
                branches: [0, 0, 4, 1, 0, 1],
            },
            Trace {
                name: "A2 (P4 swap-moves into D's row)",
                prev: &[F, D, P1, P2, P3, P4],
                cur: &[F, P4, P1, P2, P3],
                added: &[],
                prev_row: &[0, 5, 2, 3, 4],
                searched: 1,
                branches: [0, 0, 4, 1, 0, 1],
            },
            Trace {
                name: "A3 (M migrates from the first-walked archetype to the end)",
                prev: &[M, F, P1, P2, P3, P4, D],
                cur: &[F, P1, P2, P3, P4, D, M],
                added: &[],
                prev_row: &[1, 2, 3, 4, 5, 6, 0],
                searched: 1,
                branches: [0, 1, 6, 0, 1, 0],
            },
        ];
        for t in &traces {
            let (_, oracle_map) = oracle(&keys(t.prev), &keys(t.cur), t.added);
            assert_eq!(
                oracle_map, t.prev_row,
                "{}: the hand trace disagrees with the oracle - fix the trace, not the walk",
                t.name
            );
            let mut rows = RowIdentity::with_capacity(0);
            let all: Vec<u32> = (0..t.prev.len() as u32).collect();
            gather(&mut rows, t.prev, &all);
            let before = Counts::of(&rows);
            check_gather(&mut rows, t.prev, t.cur, t.added)
                .unwrap_or_else(|e| panic!("{}: {e}", t.name));
            let d = Counts::of(&rows).since(before);
            assert_eq!(
                rows.prev_row.as_read_slice(),
                t.prev_row,
                "{}: prev_row differs from the hand trace",
                t.name
            );
            assert_eq!(
                (d.searched, d.branches, d.exhaustions, d.stalls, d.builds),
                (t.searched, t.branches, 0, 0, 1),
                "{}: (stage-2 rows, branches E1..E6, budget exhaustions, walk stalls, maps built) \
                 differ from the hand trace",
                t.name
            );
        }
    }

    /// T6 edge cases: empty walks on either side, `n != m`, every row added, the same key in
    /// its own row flagged and not flagged (a `RigidBody` re-inserted on a live entity, the
    /// residual O1 case the flag alone covers), and a full reversal that runs the lookahead
    /// budget out. Each is checked against the oracle; the reversal must also exhaust the
    /// budget, or it does not exercise the cut-off.
    #[test]
    fn walk_matches_the_oracle_on_edge_cases() {
        /// One edge case: the walks either side of the gather and the added rows.
        struct EdgeCase {
            name: &'static str,
            prev: Vec<usize>,
            cur: Vec<usize>,
            added: Vec<u32>,
        }
        let case = |name, prev: &[usize], cur: &[usize], added: &[u32]| EdgeCase {
            name,
            prev: prev.to_vec(),
            cur: cur.to_vec(),
            added: added.to_vec(),
        };
        let reversed_prev: Vec<usize> = (1..=64).collect();
        let reversed_cur: Vec<usize> = (1..=64).rev().collect();
        let cases = [
            case("n = 0, m = 0", &[], &[], &[]),
            case("m = 0, nothing flagged", &[], &[1, 2, 3], &[]),
            case("m = 0, every row added", &[], &[1, 2, 3], &[0, 1, 2]),
            case("n = 0, every body despawned", &[1, 2, 3], &[], &[]),
            case("n < m", &[1, 2, 3, 4, 5], &[1, 5, 3], &[]),
            case("n > m, one late-flagged fresh id", &[1, 2, 3], &[4, 1, 2, 3, 5], &[4]),
            case("every row added over the same keys", &[1, 2, 3], &[3, 2, 1], &[0, 1, 2]),
            case("same key in its own row, flagged", &[7], &[7], &[0]),
            case(
                "same key in its own row, not flagged (a RigidBody re-inserted inside the O1 window)",
                &[7],
                &[7],
                &[],
            ),
            case("full reversal of 64 rows", &reversed_prev, &reversed_cur, &[]),
        ];
        for EdgeCase {
            name,
            prev,
            cur,
            added,
        } in &cases
        {
            let mut rows = RowIdentity::with_capacity(0);
            if !prev.is_empty() {
                let all: Vec<u32> = (0..prev.len() as u32).collect();
                gather(&mut rows, prev, &all);
            }
            let before = Counts::of(&rows);
            check_gather(&mut rows, prev, cur, added).unwrap_or_else(|e| panic!("{name}: {e}"));
            if name.starts_with("full reversal") {
                let d = Counts::of(&rows).since(before);
                assert!(
                    d.exhaustions == 1 && d.searched > 16,
                    "{name}: must run the lookahead budget out (exhaustions {}, stage-2 rows {})",
                    d.exhaustions,
                    d.searched
                );
            }
        }
    }

    /// U1 (H-03): a recycled slot in its dead body's own row, not flagged — the O1 window of
    /// a deferred spawn — is a new body on the key alone: the gather is not stable, the
    /// published map is `[NO_ROW]`, a cursor stamped one gather ago reads `Rows([NO_ROW])`,
    /// and one map was built. Red under M-A (`RowKey::of` packs generation 0 — through the
    /// `RowKey` round trip below) and M-B (`keys_equal` masks the generation: the gather
    /// reads stable). The expected map is a literal, cross-checked by a linear search over
    /// `prev` rather than by the hash-map oracle.
    #[test]
    fn recycled_slot_in_its_own_row_is_a_new_body_without_the_flag() {
        let dead = RowKey::new(7, 0);
        let recycled = RowKey::of(Entity::new(
            boyko_ecs::ecs::identifiers::primitives::EntityId(7),
            1,
        ));
        assert_eq!(
            (recycled.id(), recycled.generation()),
            (7, 1),
            "construction: `RowKey::of` packs the slot and the generation"
        );
        assert_ne!(dead, recycled, "construction: a bumped generation is a different key");
        let prev = [dead];
        let cur = [recycled];
        assert_eq!(
            prev.iter().position(|&k| k == cur[0]),
            None,
            "the linear cross-check: the recycled key held no previous row"
        );

        let mut rows = RowIdentity::with_capacity(0);
        gather_keys(&mut rows, &prev, &[0]);
        let mut cursor = RemapCursor::default();
        cursor.stamp(&rows);
        let builds = rows.remap_builds();
        gather_keys(&mut rows, &cur, &[]);
        assert!(
            !rows.stable,
            "U1: a recycled slot in its own row, not flagged, must not read as a stable gather"
        );
        assert_eq!(published_map(&rows), [X], "U1: the recycled slot's row maps to NO_ROW");
        let remap = cursor.remap(&rows);
        assert!(
            matches!(remap, RowRemap::Rows(map) if map == [X]),
            "U1: a cursor stamped one gather ago must read Rows([NO_ROW]); got {remap:?}"
        );
        assert_eq!(rows.remap_builds(), builds + 1, "U1: exactly one map built");
    }

    /// U2 (H-03): prev `[F, P1, X, P3, P4]`, cur `[F, P1, P4, P3, X']` with `X'` = X's slot at
    /// the next generation, nothing flagged: X despawned, P4 swap-moved into its row, X'
    /// spawned at the tail inside the O1 window. Hand trace: E3 F, E3 P1; E4 pushes X (X' is
    /// not X); E6 marks P4; E3 P3; E4 pushes P4; E2 marks X'; stage 2: P4 → 4, X' → miss.
    /// Map `[0, 1, 4, 3, NO_ROW]`, 2 stage-2 rows, branches `[0, 1, 3, 2, 0, 1]`, no
    /// exhaustion, no stall. Red under M-D (stage 2 and E3 compare the slot half only: X'
    /// takes X's row 2, and E3 pairs them so P4 reaches stage 2 alone). The expected map is
    /// a literal, cross-checked by a linear search over `prev`.
    #[test]
    fn recycled_slot_after_a_swap_remove_resolves_to_no_row() {
        let x = RowKey::new(P2, 0);
        let x_next = RowKey::new(P2, 1);
        let prev = [
            RowKey::new(F, 0),
            RowKey::new(P1, 0),
            x,
            RowKey::new(P3, 0),
            RowKey::new(P4, 0),
        ];
        let cur = [
            RowKey::new(F, 0),
            RowKey::new(P1, 0),
            RowKey::new(P4, 0),
            RowKey::new(P3, 0),
            x_next,
        ];
        const EXPECTED: [u32; 5] = [0, 1, 4, 3, X];
        let linear: Vec<u32> = cur
            .iter()
            .map(|k| prev.iter().position(|p| p == k).map_or(X, |r| r as u32))
            .collect();
        assert_eq!(
            linear, EXPECTED,
            "the hand trace disagrees with the linear cross-check - fix the trace, not the walk"
        );

        let mut rows = RowIdentity::with_capacity(0);
        gather_keys(&mut rows, &prev, &[0, 1, 2, 3, 4]);
        let before = Counts::of(&rows);
        check_gather_keys(&mut rows, &prev, &cur, &[]).unwrap_or_else(|e| panic!("U2: {e}"));
        assert_eq!(rows.prev_row.as_read_slice(), EXPECTED, "U2: prev_row");
        let d = Counts::of(&rows).since(before);
        assert_eq!(
            (d.searched, d.branches, d.exhaustions, d.stalls, d.builds),
            (2, [0, 1, 3, 2, 0, 1], 0, 0, 1),
            "U2: (stage-2 rows, branches E1..E6, budget exhaustions, walk stalls, maps built) \
             differ from the hand trace"
        );
    }

    /// A deterministic generator over one `u64` seed (splitmix64), so a failing case is
    /// reproduced from its seed alone.
    struct Rng(u64);

    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut z = self.0;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            z ^ (z >> 31)
        }

        fn below(&mut self, n: usize) -> usize {
            (self.next() % n as u64) as usize
        }

        fn percent(&mut self, p: u64) -> bool {
            self.next() % 100 < p
        }
    }

    /// The ECS side the walk is fed from: archetypes walked in order, swap-remove despawns,
    /// LIFO slot recycling with the kernel's generation bump on despawn, and the keys whose
    /// `RigidBody` counts as added on the next gather.
    struct Model {
        archetypes: Vec<Vec<RowKey>>,
        free: Vec<usize>,
        /// The generation of each slot, bumped when its body despawns.
        generation: Vec<u32>,
        next_id: usize,
        added: Vec<RowKey>,
    }

    impl Model {
        fn new(rng: &mut Rng) -> Self {
            let mut model = Self {
                archetypes: vec![Vec::new(); 3 + rng.below(2)],
                free: Vec::new(),
                generation: Vec::new(),
                next_id: 0,
                added: Vec::new(),
            };
            for a in 0..model.archetypes.len() {
                for _ in 0..rng.below(31) {
                    model.spawn(rng, a);
                }
            }
            model
        }

        fn walk(&self) -> Vec<RowKey> {
            self.archetypes.concat()
        }

        fn added_rows(&self, walk: &[RowKey]) -> Vec<u32> {
            walk.iter()
                .enumerate()
                .filter(|(_, key)| self.added.contains(key))
                .map(|(r, _)| r as u32)
                .collect()
        }

        /// A fresh slot at generation 0.
        fn mint(&mut self) -> usize {
            self.next_id += 1;
            self.generation.resize(self.next_id + 1, 0);
            self.next_id
        }

        /// Appends a body to archetype `a`, recycling the most recently freed slot most of
        /// the time; a recycled slot carries its bumped generation, so its key never equals
        /// the dead body's. One spawn in five is not flagged: over a recycled slot that is
        /// the O1 case (a deferred spawn the gather sees as not added), which the key alone
        /// must send to `NO_ROW`.
        fn spawn(&mut self, rng: &mut Rng, a: usize) {
            let id = match self.free.pop() {
                Some(id) if rng.percent(70) => id,
                Some(id) => {
                    self.free.push(id);
                    self.mint()
                }
                None => self.mint(),
            };
            let key = RowKey::new(id, self.generation[id]);
            self.archetypes[a].push(key);
            if rng.percent(80) {
                self.added.push(key);
            }
        }

        fn despawn(&mut self, a: usize, idx: usize) {
            let key = self.archetypes[a].swap_remove(idx);
            self.added.retain(|&k| k != key);
            let id = key.id();
            self.generation[id] = self.generation[id].wrapping_add(1);
            self.free.push(id);
        }

        fn non_empty(&self, rng: &mut Rng) -> Option<usize> {
            let candidates: Vec<usize> =
                (0..self.archetypes.len()).filter(|&a| !self.archetypes[a].is_empty()).collect();
            (!candidates.is_empty()).then(|| candidates[rng.below(candidates.len())])
        }

        /// Zero to three structural operations; zero leaves the gather stable.
        fn mutate(&mut self, rng: &mut Rng) {
            self.added.clear();
            for _ in 0..rng.below(4) {
                let k = self.archetypes.len();
                match rng.below(9) {
                    0 | 1 => {
                        let a = rng.below(k);
                        self.spawn(rng, a);
                    }
                    2 | 3 => {
                        if let Some(a) = self.non_empty(rng) {
                            let idx = rng.below(self.archetypes[a].len());
                            self.despawn(a, idx);
                        }
                    }
                    4 => {
                        if let Some(from) = self.non_empty(rng) {
                            let idx = rng.below(self.archetypes[from].len());
                            let to = (from + 1 + rng.below(k - 1)) % k;
                            let id = self.archetypes[from].swap_remove(idx);
                            self.archetypes[to].push(id);
                        }
                    }
                    5 => {
                        // Burst despawn of more than a window from the front: each one
                        // swap-moves a survivor.
                        if let Some(a) = self.non_empty(rng) {
                            let burst = REMAP_WINDOW + 1 + rng.below(8);
                            for _ in 0..burst.min(self.archetypes[a].len()) {
                                self.despawn(a, 0);
                            }
                        }
                    }
                    6 => {
                        // Burst migration of more than a window into an earlier archetype,
                        // tail first so nothing swap-moves: the walk loses alignment.
                        let from = 1 + rng.below(k - 1);
                        let to = rng.below(from);
                        let burst = (REMAP_WINDOW + 1 + rng.below(8)).min(self.archetypes[from].len());
                        for _ in 0..burst {
                            let id = self.archetypes[from].pop().expect("burst bounded by len");
                            self.archetypes[to].push(id);
                        }
                    }
                    7 => {
                        let a = rng.below(k);
                        let len = self.archetypes[a].len();
                        for i in (1..len).rev() {
                            let j = rng.below(i + 1);
                            self.archetypes[a].swap(i, j);
                        }
                    }
                    _ => {
                        // Burst spawn into the first-walked archetype.
                        for _ in 0..REMAP_WINDOW + 1 + rng.below(8) {
                            self.spawn(rng, 0);
                        }
                    }
                }
            }
        }
    }

    /// Gathers per property case.
    const GATHERS_PER_CASE: usize = 12;

    /// T6 property: over random swap-removes, fresh and recycled spawns (flagged, and not
    /// flagged per O1 — a recycled slot at its bumped generation, which the key alone must
    /// resolve), migrations across three or four archetypes, bursts of more than
    /// `REMAP_WINDOW` removals, migrations and spawns, and random permutations, the published
    /// map, the `stable` decision and the cursor classification equal a `HashMap` oracle
    /// keyed by `(slot, generation)` after every gather, and no walk stalls. Anti-vacuity:
    /// across the run every branch E1..E6, the budget cut-off and stage 2 are each taken at
    /// least once.
    #[test]
    fn walk_matches_a_hash_map_oracle_on_random_structural_churn() {
        let totals = Cell::new(Counts::default());
        let config = ProptestConfig {
            cases: if cfg!(miri) { 4 } else { 256 },
            failure_persistence: None,
            ..ProptestConfig::default()
        };
        proptest!(config, |(seed in any::<u64>())| {
            let mut rng = Rng(seed);
            let mut model = Model::new(&mut rng);
            let mut rows = RowIdentity::with_capacity(0);
            let mut prev: Vec<RowKey> = Vec::new();
            for g in 0..GATHERS_PER_CASE {
                if g > 0 {
                    model.mutate(&mut rng);
                }
                let walk = model.walk();
                let added = model.added_rows(&walk);
                if let Err(e) = check_gather_keys(&mut rows, &prev, &walk, &added) {
                    prop_assert!(false, "seed {:#x}, gather {}: {}", seed, g, e);
                }
                prev = walk;
            }
            totals.set(totals.get().plus(Counts::of(&rows)));
        });
        let t = totals.get();
        if !cfg!(miri) {
            assert!(
                t.branches.iter().all(|&b| b > 0) && t.exhaustions > 0 && t.searched > 0,
                "anti-vacuity: the generator must reach every walk branch, the budget cut-off and \
                 stage 2 at least once; branches E1..E6 {:?}, budget exhaustions {}, stage-2 rows {}",
                t.branches,
                t.exhaustions,
                t.searched
            );
        }
        assert_eq!(t.stalls, 0, "no walk may leave the bounded loop with rows unvisited");
    }

    /// T6c: prev = `[h, f, s1..s40]`, cur = `[h, s40, s39, .., s(41-b), f, s1..s(40-b)]`, no
    /// row added. For every `b` in `1..=16` the map equals the oracle, exactly `b` rows reach
    /// stage 2 and the budget never runs out; at `b = 17` the map still equals the oracle and
    /// more than `b` rows reach stage 2 (the documented window degrade class exists).
    ///
    /// The window is pinned by the literal `PINNED_WINDOW`, not by `REMAP_WINDOW`: M10 and
    /// M10b mutate that constant, and a loop bound or branch keyed on it moves with the
    /// mutant. Keyed on the constant, `REMAP_WINDOW = 0` left `b = 1` as the only case, in the
    /// past-the-window arm, and `REMAP_WINDOW = 15` put `b = 16` in that arm too; this test
    /// passed under both mutants in that form (measured).
    ///
    /// Budget arithmetic, recorded so a constant change is noticed: `n + m = 84`, budget
    /// `REMAP_BUDGET_PER_ROW * 84 = 672`; the costliest in-window case, `b = 16`, spends
    /// `(16 + 15 + .. + 1) + 16 * 16 = 136 + 256 = 392`. Hand trace for `b = 17`: 24 E4 rows,
    /// 16 E6 rows, then the budget runs out, so 40 rows reach stage 2 with one exhaustion.
    /// Red under M10 (`REMAP_WINDOW = 0`: `b = 1` sends 40 rows to stage 2) and M10b
    /// (`REMAP_WINDOW = 15`: `b = 16` loses alignment).
    #[test]
    fn walk_window_boundary() {
        /// The lookahead window this test pins (Rev 3 P18); a literal on purpose.
        const PINNED_WINDOW: usize = 16;
        const H: usize = 5000;
        const FF: usize = 5001;
        let s = |k: usize| 6000 + k;
        let prev: Vec<usize> = [H, FF].into_iter().chain((1..=40).map(s)).collect();
        for b in 1..=PINNED_WINDOW + 1 {
            let cur: Vec<usize> = std::iter::once(H)
                .chain((41 - b..=40).rev().map(s))
                .chain(std::iter::once(FF))
                .chain((1..=40 - b).map(s))
                .collect();
            assert_eq!(cur.len(), prev.len(), "construction: b = {b} keeps the row count");
            let mut rows = RowIdentity::with_capacity(0);
            let all: Vec<u32> = (0..prev.len() as u32).collect();
            gather(&mut rows, &prev, &all);
            let before = Counts::of(&rows);
            check_gather(&mut rows, &prev, &cur, &[]).unwrap_or_else(|e| panic!("T6c b = {b}: {e}"));
            let d = Counts::of(&rows).since(before);
            if b <= PINNED_WINDOW {
                assert_eq!(
                    (d.searched, d.exhaustions),
                    (b as u64, 0),
                    "T6c b = {b}: a burst within the window must resolve exactly b rows by stage 2 \
                     without running the budget out (stage-2 rows {}, exhaustions {})",
                    d.searched,
                    d.exhaustions
                );
            } else {
                assert!(
                    d.searched > b as u64,
                    "T6c b = {b}: a burst past the window must lose alignment (stage-2 rows {}, \
                     exhaustions {})",
                    d.searched,
                    d.exhaustions
                );
            }
        }
    }

    /// T12: a stamp taken against one `RowIdentity` never classifies as `Identity` or `Rows`
    /// against another, in either epoch order, and an instance that never gathered is
    /// `Identity` for any cursor. Red under M13 (every instance starts at sequence 0: A's
    /// stamp classifies `Identity` against B) and M14 (an ordering `debug_assert!` in
    /// `classify` panics on the B-stamp-against-A case).
    #[test]
    fn a_stamp_from_another_row_identity_never_carries() {
        let mut a = RowIdentity::with_capacity(0);
        let mut b = RowIdentity::with_capacity(0);
        let c = RowIdentity::with_capacity(0);
        for _ in 0..2 {
            gather(&mut a, &[1, 2, 3], &[]);
            gather(&mut b, &[1, 2, 3], &[]);
        }
        let mut own = RemapCursor::default();
        own.stamp(&b);
        assert!(
            matches!(own.remap(&b), RowRemap::Identity),
            "anti-vacuity: a cursor stamped by B is Identity against B itself"
        );

        let mut from_a = RemapCursor::default();
        from_a.stamp(&a);
        let against_b = from_a.remap(&b);
        assert!(
            matches!(against_b, RowRemap::Reset) && from_a.resets() == 1,
            "cursor stamped by A classified {against_b:?} against B (resets {}); expected Reset, 1",
            from_a.resets()
        );

        let mut from_b = RemapCursor::default();
        from_b.stamp(&b);
        let against_a = from_b.remap(&a);
        assert!(
            matches!(against_a, RowRemap::Reset) && from_b.resets() == 1,
            "cursor stamped by B (higher epoch) classified {against_a:?} against A (resets {}); \
             expected Reset, 1",
            from_b.resets()
        );

        let against_c = from_a.remap(&c);
        assert!(
            matches!(against_c, RowRemap::Identity),
            "an instance that never gathered is direct drive: expected Identity, got {against_c:?}"
        );
    }

    /// `RowRemap::row` and `RowRemap::pair`: a pair carries only when both bodies held rows
    /// and those rows kept their order; `Identity` is the pair itself and `Reset` carries
    /// nothing.
    #[test]
    fn row_remap_carries_only_mapped_pairs_that_keep_their_order() {
        let prev_row = [2, NO_ROW, 0, 3];
        let rows = RowRemap::Rows(&prev_row);
        assert_eq!(rows.pair(0, 3), Some((2, 3)), "both mapped, order kept");
        assert_eq!(rows.pair(0, 2), None, "current (0, 2) held (2, 0): the order flipped");
        assert_eq!(rows.pair(1, 3), None, "body a is new");
        assert_eq!(rows.pair(2, 1), None, "body b is new");
        assert_eq!((rows.row(1), rows.row(3)), (None, Some(3)), "row maps through prev_row");
        assert_eq!(RowRemap::Identity.pair(4, 9), Some((4, 9)), "Identity is the pair itself");
        assert_eq!(
            (RowRemap::Reset.row(0), RowRemap::Reset.pair(0, 1)),
            (None, None),
            "Reset carries nothing"
        );
    }
}
