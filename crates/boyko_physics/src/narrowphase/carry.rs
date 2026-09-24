//! The pair carry of contact reuse (L9 D9, commit C2 of
//! `docs/physics/perf-campaign/levers/L9-contact-reuse/02-DESIGN-REV1.md`).
//!
//! Every candidate pair of a step writes one [`PairTag`] at its stream slot `k`. The next step
//! finds, for each of its pairs, the slot `j` the same two bodies held in the previous step's pair
//! list — the **join** — and reads the tag written there. A tag carries the exact fast path's
//! cached separating axis (L9a (ii)): a box pair the SAT separated on axis `s` evaluates `s` first
//! on the next step, and skips the SAT while it still separates. With contact reuse on (L9b,
//! commit C3, `narrowphase/reuse.rs`) a slow touching box pair also writes a [`ReuseRecord`] at
//! its slot `k` of a second double-buffered column and sets `REC` in its tag; the next step reads
//! the record at the joined slot `j`.
//!
//! # The join (D9, lemma L9-J)
//!
//! `j(k)` is the slot of `pairs_prev` equal to pair `k`'s key in the previous step's rows, if any.
//! The previous rows come from the gather's row identity (`row_identity.rs`):
//!
//! * **Identity** (rows unchanged): the key is the pair itself.
//! * **Rows** (rows moved): the key is `(prev_row[a], prev_row[b])`, ordered; a pair whose body is
//!   new has none. A pair whose two rows swapped order is still found — its key is the flipped
//!   pair, and the tag it reads is [flipped](PairTag::flipped) into the current roles (ruling W2:
//!   the carry survives an order flip).
//! * **Reset** (the carry missed a gather, or its stamp is not the pair list's): no pair has a key.
//!
//! `pairs_prev` is sorted, and the keys of the pairs whose two rows the aligned walk matched are
//! strictly increasing in stream order (the walk consumes both cursors in step, so those rows carry
//! a strictly increasing `prev_row`; `RowIdentity::stage2_rows`). Those pairs therefore merge-join
//! against `pairs_prev` with one monotone cursor. A pair with an endpoint that stage 2 resolved — a
//! **jumper**, marked in a per-row bitset built from `stage2_rows` on the steps whose rows moved,
//! at the start of the broadphase (`ContactPairs::rotate`; L10 C0 moved the build there from this
//! module's prologue, so the broadphase reads the same bits) — binary-searches instead. Either way the result is the definition's, so `j` depends neither on
//! how the pairs are cut into chunks nor on which implementation found it: a chunk's cursor starts
//! at the lower bound of its first merged key, which is where a cursor walked from pair 0 stands
//! when it reaches that key.
//!
//! # The stamps (review OQ4)
//!
//! The pair lists swap at the start of the broadphase (`ContactPairs::rotate`), the tag columns at
//! the start of the narrowphase ([`PairCarry::open`]). The two swaps are tied by stamps: the pair
//! list carries the gather sequence it was built on and its rotation ordinal, and the carry
//! records both for the list its tags index ([`PairCarry::stamp`], after the pair loop; the
//! gather stamp only when the list was built on the current gather and every pair was tagged).
//! [`PairCarry::source`] joins only when `pairs_prev` carries both — the ordinal tells apart two
//! lists built on one gather, which the gather stamp alone cannot; otherwise the step is a Reset.
//! A path that opens the carry without the system's epilogue (a direct caller) invalidates the stamp, so the step after it is
//! a Reset too. A wrong join could cost only hits, never correctness — a cached separating axis
//! that no longer separates falls through to the full SAT — but the stamps keep the join the
//! definition's.
//!
//! # Why the exact fast path gives today's bits (lemma L9-L2)
//!
//! The SAT reports "separated" iff some axis has `depth < 0`, and `eval_axis` is pure. The cached
//! axis is evaluated by the same `eval_axis` on the same raw axis as the SAT's candidate of that
//! index, so a negative depth there is a negative candidate of the SAT: the pair gets no manifold,
//! exactly as today. When it is not negative (or degenerate), the full SAT runs as today. A hit
//! calls no hint and writes no axis, which is what a separated pair does today, so the hysteresis
//! table is unchanged too.
//!
//! # Threads (W invariance)
//!
//! During the parallel narrowphase's scope, `pairs_prev`, the previous tags, the previous records,
//! the jumper bitset (built by the broadphase before the narrowphase runs) and the row map are
//! shared and read-only; each chunk writes the tags and the
//! records of its own slots `[lo, hi)` through the columns' solve views (`narrowphase/dispatch.rs`).
//! The serial loop and the chunks call the same cursor and the same per-pair function, so the tags
//! and the records equal the serial loop's for any partition.
//!
//! ZERO `unsafe` here; every column is a kernel `ScratchColumn` (principle 0), grown in place, no
//! per-step heap allocation.

use boyko_ecs::ecs::core::component::scratch::ScratchColumn;

use crate::manifold::BodyIndex;
use crate::narrowphase::axis_cache::SAT_AXIS_COUNT;
use crate::narrowphase::reuse::{Prev, ReuseRecord, ReuseStep};
use crate::resources::ContactPairs;
use crate::row_identity::{NO_ROW, RemapCursor, RowIdentity, RowRemap};
use crate::scratch_ids::{
    pair_tag_id, pair_tag_prev_id, register_narrowphase_column_layouts, reuse_id, reuse_prev_id,
    scratch_reserve_rows,
};

/// The stamp of a pair list or a carry that no step stamped, or whose stamp was invalidated.
/// A gather sequence is never `0`: each `RowIdentity` starts at `epoch · 2^40` with `epoch ≥ 1`.
pub(crate) const NO_SEQ: u64 = 0;

/// A cursor that has not merged a key yet.
const UNSET: usize = usize::MAX;

/// One candidate pair's record of what its collision did this step (D9), 2 B.
///
/// | bits | field |
/// |---|---|
/// | 0–3 | the SAT axis the pair chose this step (`15` = none) |
/// | 4 | `BOX`: both shapes are boxes |
/// | 5 | `PUSHED`: the pair emitted a manifold (into either stream) |
/// | 6, 7 | `SET`, `SETTLED`: L10's settle bits (design 08 B1′), written by [`PairTag::settled`] |
/// | 8 | `REC`: the pair's reuse record at this slot was written this step (L9b) |
/// | 9 | `SEP`: the pair is separated, on the axis in bits 12–15 |
/// | 10 | `HIT`: the pair's output came from its record, not the SAT (L9b) |
/// | 11 | `SEPHIT`: the carried separating axis still separated it; the SAT did not run |
/// | 12–15 | the separating axis, `0..15`, valid iff `SEP` |
///
/// A `REC` tag's axis field holds the SAT axis of the record's last full collision, which a hit
/// carries forward unchanged. A box pair's tag has at most one of `HIT` (from a record) and
/// `SEPHIT` (from the carried separating axis); neither means the full collision ran. That is the
/// closure of the narrowphase's per-step counters.
///
/// One layout shared with L10 (ruling O7): L10 rev 2.2 adopts this `u16` in place of rev 2's
/// `u8` (its Δ3), with its `SET` and `SETTLED` at bits 6 and 7, which L10 C3b writes on every
/// path (design 08 B1′). Every pair kind writes its tag every step (ruling O5), so no stale bit
/// survives the double buffer.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PairTag(u16);

const _: () = assert!(size_of::<PairTag>() == 2 && align_of::<PairTag>() == 2);

impl PairTag {
    /// Bits 0–3: the chosen SAT axis.
    const AXIS_MASK: u16 = 0x000F;
    /// Axis field value: no axis.
    const AXIS_NONE: u16 = 0x000F;
    /// Bit 4: both shapes are boxes.
    pub(crate) const BOX: u16 = 1 << 4;
    /// Bit 5: the pair emitted a manifold.
    pub(crate) const PUSHED: u16 = 1 << 5;
    /// Bit 6 (L10, design 08 B1′): the box pair made a contact this step — a full collision's
    /// contact, or a record hit's.
    pub(crate) const SET: u16 = 1 << 6;
    /// Bit 7 (L10, design 08 B1′): the pair's output is settled — a record hit, or a full
    /// contact on the axis its hint named (`hint == Some(chosen)`), so the next step's inputs
    /// choose the same axis again.
    pub(crate) const SETTLED: u16 = 1 << 7;
    /// Bit 8: the pair's reuse record at this slot was written this step.
    pub(crate) const REC: u16 = 1 << 8;
    /// Bit 9: the pair is separated on the axis in bits 12–15.
    pub(crate) const SEP: u16 = 1 << 9;
    /// Bit 10: the pair's output came from its reuse record.
    pub(crate) const HIT: u16 = 1 << 10;
    /// Bit 11: the carried separating axis still separated the pair.
    pub(crate) const SEPHIT: u16 = 1 << 11;
    /// The separating axis field's shift.
    const SEP_SHIFT: u32 = 12;

    /// The tag of a pair nothing was recorded for: no axis, no flag. A missed join reads it.
    pub(crate) const NONE: Self = Self(Self::AXIS_NONE);

    /// The tag of a non-box pair (sphere-sphere, sphere-box), which records only whether it
    /// emitted a manifold.
    #[inline]
    pub(crate) const fn non_box(pushed: bool) -> Self {
        Self(Self::AXIS_NONE | if pushed { Self::PUSHED } else { 0 })
    }

    /// The tag of a box pair that produced a contact on SAT axis `axis` (`0..15`); `pushed` is
    /// whether its manifold was emitted.
    #[inline]
    pub(crate) fn box_contact(axis: usize, pushed: bool) -> Self {
        debug_assert!(
            axis < usize::from(SAT_AXIS_COUNT),
            "invariant: a chosen SAT axis is 0..15"
        );
        Self(Self::BOX | if pushed { Self::PUSHED } else { 0 } | (axis as u16 & Self::AXIS_MASK))
    }

    /// The tag of a box pair separated on SAT axis `axis` (`0..15`); `hit` when the carried axis
    /// found it (L9a (ii)) rather than the SAT.
    #[inline]
    pub(crate) fn box_separated(axis: u8, hit: bool) -> Self {
        debug_assert!(
            axis < SAT_AXIS_COUNT,
            "invariant: a SEP tag's axis is 0..15"
        );
        Self(
            Self::AXIS_NONE
                | Self::BOX
                | Self::SEP
                | if hit { Self::SEPHIT } else { 0 }
                | (u16::from(axis) << Self::SEP_SHIFT),
        )
    }

    /// The tag of a slow box pair that wrote its reuse record this step (L9b), on the record's SAT
    /// axis `axis` (`0..15`): `hit` when the output came from the previous step's record rather than
    /// the full collision; `pushed` whether its manifold was emitted.
    #[inline]
    pub(crate) fn box_recorded(axis: usize, pushed: bool, hit: bool) -> Self {
        Self(Self::box_contact(axis, pushed).0 | Self::REC | if hit { Self::HIT } else { 0 })
    }

    /// The tag of a box pair whose edge record's axis now separates it (L9b): separated exactly on
    /// `axis`, found from the record.
    #[inline]
    pub(crate) fn box_separated_by_record(axis: u8) -> Self {
        Self(Self::box_separated(axis, false).0 | Self::HIT)
    }

    /// The tag of a box pair with no contact for a reason other than a separating axis (no face
    /// axis, a degenerate reference face, a fallback with no edge axis).
    pub(crate) const BOX_NO_CONTACT: Self = Self(Self::AXIS_NONE | Self::BOX);

    /// Whether every bit of `flags` is set.
    #[inline]
    pub(crate) const fn has(self, flags: u16) -> bool {
        self.0 & flags == flags
    }

    /// The raw bits (for fingerprints and counters).
    #[inline]
    pub(crate) const fn bits(self) -> u16 {
        self.0
    }

    /// The separating axis this tag carries, iff `SEP`.
    #[inline]
    pub(crate) fn sep_axis(self) -> Option<u8> {
        if self.has(Self::SEP) {
            let axis = (self.0 >> Self::SEP_SHIFT) as u8;
            debug_assert!(
                axis < SAT_AXIS_COUNT,
                "invariant: a SEP tag's axis is 0..15"
            );
            Some(axis)
        } else {
            None
        }
    }

    /// The SAT axis the pair chose, iff it produced a box contact.
    #[inline]
    pub(crate) fn axis(self) -> Option<u8> {
        let axis = (self.0 & Self::AXIS_MASK) as u8;
        (u16::from(axis) != Self::AXIS_NONE).then_some(axis)
    }

    /// Whether the pair that wrote this tag keys its hysteresis entry on a step whose key set
    /// changed: iff the tag carries a SAT axis — a contact from the full collision, a reuse
    /// record built from one, or a record hit (ruling W1 re-keys a hit on such a step). A full
    /// contact also writes its axis on every other step; a hit writes it only on such a step.
    ///
    /// The commit rule of L9's narrowphase, extracted as the one function L10's axis mirror
    /// calls too (L10 C0, design 06 D-C). L9 writes no `SET` bit, so the design's form
    /// `REC ∨ (SET ∧ ¬REC)` is read on this tree as the axis field, which is what L9's writes
    /// actually carry: `box_contact` and `box_recorded` set it, and every other tag — a
    /// separated pair, a record whose axis now separates, a box pair with no contact, a
    /// non-box pair, [`NONE`](Self::NONE) — leaves it at none.
    #[inline]
    pub(crate) fn rekeys(self) -> bool {
        self.axis().is_some()
    }

    /// The tag of a slot L10's sleep-skip did not collide this step because an endpoint is
    /// held (design 06 B1, `0x0C00` as ruled): `HIT | SEPHIT`, a pair no collision can write,
    /// since a box pair's tag carries at most one of the two. It has no `REC` and no `SEP`, so
    /// the next step's join reads it as stateless.
    ///
    /// Its axis nibble reads `0`, not none, and it carries no `BOX` bit, so every classifier
    /// tests [`is_held_skip`](Self::is_held_skip) BEFORE any single-bit test (design 08 B1′,
    /// rule 5): a `HIT` test would call it a reuse hit, a `SEPHIT` test a carried-axis hit, a
    /// `BOX` test a non-box pair, and [`rekeys`](Self::rekeys) must never see it.
    pub(crate) const HELD_SKIP: Self = Self(Self::HIT | Self::SEPHIT);

    /// The tag with raw bits `bits` (L10's exhaustive gates over every `u16`).
    #[cfg(test)]
    pub(crate) const fn from_bits(bits: u16) -> Self {
        Self(bits)
    }

    /// Whether this is [`HELD_SKIP`](Self::HELD_SKIP): the whole tag, not a bit of it.
    #[inline]
    pub(crate) const fn is_held_skip(self) -> bool {
        self.0 == Self::HELD_SKIP.0
    }

    /// The bits a reader of a KEPT tag may consume (design 08 B1′): every bit but `HIT` and
    /// `SEPHIT`. L9's join reads `REC`, `SEP` and the separating axis, L10's mirror reads
    /// [`rekeys`](Self::rekeys) and the axis nibble, and the admission test decides a `REC` pair
    /// by `HIT | SETTLED`, where a hit is always settled. So a pair captured on a settled miss and
    /// the same pair's later hit under `Off` agree on every bit of this mask.
    pub(crate) const HIT_INVARIANT: u16 = !(Self::HIT | Self::SEPHIT);

    /// This tag with L10's settle bits (`SET`, `SETTLED`, design 08 B1′) written as the path that
    /// produced it defines them, for a box pair whose full collision read the hysteresis hint
    /// `hint` (`None` when it read none, or the hint named no axis):
    ///
    /// | path | settle bits |
    /// |---|---|
    /// | a record hit (`REC \| HIT` with an axis) | `SET \| SETTLED` |
    /// | a full collision's contact (an axis, no hit) | `SET`, and `SETTLED` iff `hint` is the axis |
    /// | separated, a carried-axis hit, no contact | neither |
    ///
    /// Every other bit is the path's own. A pure function of the tag and the hint, so both
    /// narrowphase paths write the same bits.
    #[inline]
    pub(crate) fn settled(self, hint: Option<usize>) -> Self {
        let bits = self.0 & !(Self::SET | Self::SETTLED);
        match self.axis() {
            Some(_) if self.has(Self::REC | Self::HIT) => Self(bits | Self::SET | Self::SETTLED),
            Some(axis) => {
                let settled = hint == Some(usize::from(axis));
                Self(bits | Self::SET | if settled { Self::SETTLED } else { 0 })
            }
            None => Self(bits),
        }
    }

    /// This tag as seen by the pair with bodies A and B exchanged (a join whose two rows swapped
    /// order): both axis fields name the same geometric axes in the exchanged roles.
    #[inline]
    pub(crate) fn flipped(self) -> Self {
        let mut bits = self.0 & !(Self::AXIS_MASK | (Self::AXIS_MASK << Self::SEP_SHIFT));
        bits |= match self.axis() {
            Some(axis) => u16::from(flip_axis(axis)),
            None => Self::AXIS_NONE,
        };
        if let Some(axis) = self.sep_axis() {
            bits |= u16::from(flip_axis(axis)) << Self::SEP_SHIFT;
        }
        Self(bits)
    }
}

/// The canonical SAT axis index `axis` (`0..15`) with bodies A and B exchanged: A's face `i` is
/// B's face `i` (`i ↔ 3 + i`), and the edge axis `A.axes[ea] × B.axes[eb]` (`6 + 3·ea + eb`) is
/// `B.axes[eb] × A.axes[ea]`, the negated cross product of the edge `(eb, ea)` — the same line,
/// whose `eval_axis` depth is the same.
#[inline]
pub(crate) const fn flip_axis(axis: u8) -> u8 {
    if axis < 3 {
        axis + 3
    } else if axis < 6 {
        axis - 3
    } else {
        let e = axis - 6;
        6 + (e % 3) * 3 + e / 3
    }
}

/// A pair's sort key: `(a, b)` packed so the integer order is the pair list's `(min, max)` order.
#[inline]
const fn pack(a: u32, b: u32) -> u64 {
    ((a as u64) << 32) | b as u64
}

/// The external half of a step's pair carry (D9): how this step's rows map to the rows the
/// previous pair list was built on, that list, and the jumper bitset of the rows stage 2
/// resolved. Built by [`PairCarry::source`] in the narrowphase system, or
/// [`NONE`](Self::NONE) for a caller with no carry.
#[derive(Clone, Copy, Debug)]
pub(crate) struct CarryIn<'a> {
    /// Identity, Rows or Reset.
    remap: RowRemap<'a>,
    /// The previous step's candidate pairs, `(min, max)`-sorted; empty on Reset.
    pairs_prev: &'a [(BodyIndex, BodyIndex)],
    /// One bit per current row, set iff stage 2 resolved it (`ContactPairs::rotate` builds it,
    /// or [`jumper_words`] in a test); read only on a Rows step.
    jumpers: &'a [u64],
    /// The step's requested contact reuse (L9b); [`PairCarry::open`] turns it off when the record
    /// column cannot hold a record per pair.
    reuse: ReuseStep,
}

impl CarryIn<'static> {
    /// No carry: every pair misses its join, and the step still writes every tag. Contact reuse
    /// off.
    pub(crate) const NONE: Self = Self {
        remap: RowRemap::Reset,
        pairs_prev: &[],
        jumpers: &[],
        reuse: ReuseStep::OFF,
    };
}

impl<'a> CarryIn<'a> {
    /// A carry over an explicit map (tests, and [`PairCarry::source`]). A Reset carries no pairs.
    #[inline]
    pub(crate) fn new(
        remap: RowRemap<'a>,
        pairs_prev: &'a [(BodyIndex, BodyIndex)],
        jumpers: &'a [u64],
    ) -> Self {
        match remap {
            RowRemap::Reset => Self {
                remap,
                pairs_prev: &[],
                jumpers: &[],
                reuse: ReuseStep::OFF,
            },
            _ => Self {
                remap,
                pairs_prev,
                jumpers,
                reuse: ReuseStep::OFF,
            },
        }
    }

    /// This carry with the step's requested contact reuse.
    #[inline]
    pub(crate) fn with_reuse(self, reuse: ReuseStep) -> Self {
        Self { reuse, ..self }
    }

    /// The step's requested contact reuse.
    #[inline]
    pub(crate) fn reuse(&self) -> ReuseStep {
        self.reuse
    }
}

/// One found join: the slot `j` of `pairs_prev` the pair's two bodies held, and whether their rows
/// swapped order since.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct JoinHit {
    /// The slot in `pairs_prev`.
    pub(crate) j: u32,
    /// Whether the pair's body A was body B at slot `j`.
    pub(crate) flipped: bool,
}

/// The step's read-only view of the carry, shared by the serial loop and every chunk (D9):
/// `Copy`, shared slices only, so `Send + Sync` without an `unsafe impl`.
#[derive(Clone, Copy, Debug)]
pub(crate) struct PairJoin<'a> {
    /// Identity, Rows or Reset.
    remap: RowRemap<'a>,
    /// The previous step's candidate pairs; empty on Reset.
    pairs_prev: &'a [(BodyIndex, BodyIndex)],
    /// The previous step's tags, one per slot of `pairs_prev` (and stale rows past it).
    tag_prev: &'a [PairTag],
    /// One bit per current row: set iff stage 2 resolved it. Read only on a Rows step.
    jumpers: &'a [u64],
    /// The previous step's reuse records, one per slot of `pairs_prev` it wrote (L9b); empty when
    /// this step does not reuse.
    records: &'a [ReuseRecord],
    /// This step's contact reuse and parity, as [`PairCarry::open`] settled them.
    reuse: ReuseStep,
    /// Whether a record's parity is NOT the previous step's by construction: set on L10's
    /// restore source ([`restored`](Self::restored)), whose records were copied from the stream
    /// at a move-in any number of steps ago (design 06 B4's parity waiver).
    waive_parity: bool,
}

impl<'a> PairJoin<'a> {
    /// This step's contact reuse and parity (L9b).
    #[inline]
    pub(crate) fn reuse(&self) -> ReuseStep {
        self.reuse
    }

    /// The same join — this step's row map, jumper bitset and settled reuse — over L10's restore
    /// source instead of the previous pair list (design 06 B4, ruling W2): `keys` are the kept
    /// pairs of the records restored this step in the rows of the previous gather, `(min, max)`
    /// and strictly increasing; `tags` and `records` are slot-parallel to them, a record valid
    /// where its tag carries `REC`. The pairs L10 routes to the kept source find their previous
    /// state through the very key function L9's join uses — the monotone cursor, the jumper
    /// binary search and the order flip — so a restored pair reads what `Off`'s join reads.
    ///
    /// Its records' parity is waived: a kept record was written at its island's move-in, not in
    /// the previous step.
    #[inline]
    pub(crate) fn restored(
        self,
        keys: &'a [(BodyIndex, BodyIndex)],
        tags: &'a [PairTag],
        records: &'a [ReuseRecord],
    ) -> Self {
        debug_assert!(
            keys.windows(2).all(|w| w[0] < w[1]) && tags.len() >= keys.len(),
            "invariant: the restore source is strictly sorted and tags every key"
        );
        let records = if self.reuse.on { records } else { &[] };
        Self {
            pairs_prev: if matches!(self.remap, RowRemap::Reset) { &[] } else { keys },
            tag_prev: tags,
            records,
            waive_parity: true,
            ..self
        }
    }

    /// A cursor for one run of consecutive pairs (the serial loop's `[0, n)`, a chunk's
    /// `[lo, hi)`). It positions itself at the lower bound of the run's first merged key — one
    /// binary search per run — and merges forward from there.
    #[inline]
    pub(crate) fn cursor(self) -> JoinCursor<'a> {
        JoinCursor {
            join: self,
            pos: UNSET,
            #[cfg(debug_assertions)]
            last_merged: None,
        }
    }

    /// Whether current row `r` is a jumper. Valid only on a Rows step.
    #[inline]
    fn is_jumper(&self, r: u32) -> bool {
        (self.jumpers[(r >> 6) as usize] >> (r & 63)) & 1 != 0
    }
}

/// The join's position in `pairs_prev` for one run of pairs, visited in ascending order.
pub(crate) struct JoinCursor<'a> {
    /// The step's join.
    join: PairJoin<'a>,
    /// The merge position: every slot below it holds a key below the last merged key. [`UNSET`]
    /// until the first merge.
    pos: usize,
    /// The last merged key, for the monotonicity assertion.
    #[cfg(debug_assertions)]
    last_merged: Option<u64>,
}

impl<'a> JoinCursor<'a> {
    /// The join of the pair `(a, b)` (`a < b`, the run's next pair): the slot its two bodies held
    /// in `pairs_prev`, or `None`.
    #[inline]
    pub(crate) fn find(&mut self, a: BodyIndex, b: BodyIndex) -> Option<JoinHit> {
        debug_assert!(a < b, "invariant: a candidate pair is (min, max)");
        match self.join.remap {
            RowRemap::Identity => self
                .merge(pack(a.0, b.0))
                .map(|j| JoinHit { j, flipped: false }),
            RowRemap::Rows(prev_row) => {
                let pa = prev_row[a.0 as usize];
                let pb = prev_row[b.0 as usize];
                if pa == NO_ROW || pb == NO_ROW {
                    return None;
                }
                if self.join.is_jumper(a.0) || self.join.is_jumper(b.0) {
                    self.search(pa, pb)
                } else {
                    debug_assert!(
                        pa < pb,
                        "invariant: two aligned rows keep their order (row_identity.rs, T5)"
                    );
                    self.merge(pack(pa, pb))
                        .map(|j| JoinHit { j, flipped: false })
                }
            }
            RowRemap::Reset => None,
        }
    }

    /// The tag the pair `(a, b)` wrote in the previous step, in its current roles, or
    /// [`PairTag::NONE`] when it has no join: [`prev`](Self::prev)'s tag.
    #[cfg(test)]
    pub(crate) fn prev_tag(&mut self, a: BodyIndex, b: BodyIndex) -> PairTag {
        self.prev(a, b).tag
    }

    /// What the pair `(a, b)` (`a < b`, the run's next pair) left in the previous step (L9 D9): its
    /// tag in the current roles, and — when the tag carries `REC` and this step reuses — its record,
    /// as stored.
    #[inline]
    pub(crate) fn prev(&mut self, a: BodyIndex, b: BodyIndex) -> Prev<'a> {
        let Some(hit) = self.find(a, b) else {
            return Prev::NONE;
        };
        let stored = self.join.tag_prev[hit.j as usize];
        let tag = if hit.flipped { stored.flipped() } else { stored };
        let record = if self.join.reuse.on && stored.has(PairTag::REC) {
            let record = self.join.records.get(hit.j as usize);
            debug_assert!(
                record.is_some_and(|r| {
                    self.join.waive_parity || r.parity() != self.join.reuse.parity
                }),
                "invariant: a REC tag's record was written at its slot in the previous step \
                 (or kept by L10's held store)"
            );
            record
        } else {
            None
        };
        Prev { tag, record, flipped: hit.flipped }
    }

    /// The monotone merge: advances to the first slot whose key is not below `key`, and matches
    /// iff it equals `key`.
    #[inline]
    fn merge(&mut self, key: u64) -> Option<u32> {
        let prev = self.join.pairs_prev;
        #[cfg(debug_assertions)]
        {
            debug_assert!(
                self.last_merged.is_none_or(|last| last < key),
                "invariant: the merged keys of one run strictly increase (lemma L9-J)"
            );
            self.last_merged = Some(key);
        }
        let mut pos = self.pos;
        if pos == UNSET {
            pos = prev.partition_point(|&(pa, pb)| pack(pa.0, pb.0) < key);
        }
        while pos < prev.len() && pack(prev[pos].0.0, prev[pos].1.0) < key {
            pos += 1;
        }
        self.pos = pos;
        (pos < prev.len() && pack(prev[pos].0.0, prev[pos].1.0) == key).then_some(pos as u32)
    }

    /// A jumper's join: a binary search of `pairs_prev` for the previous rows `(pa, pb)` in
    /// either order. It leaves the merge position alone.
    #[inline]
    fn search(&self, pa: u32, pb: u32) -> Option<JoinHit> {
        let (lo, hi, flipped) = if pa < pb {
            (pa, pb, false)
        } else {
            (pb, pa, true)
        };
        let key = pack(lo, hi);
        self.join
            .pairs_prev
            .binary_search_by_key(&key, |&(x, y)| pack(x.0, y.0))
            .ok()
            .map(|j| JoinHit {
                j: j as u32,
                flipped,
            })
    }
}

/// The pair carry's state in [`Manifolds`](crate::resources::Manifolds) (D9): this step's and the
/// previous step's tags and reuse records, and the stamps that tie the tags to the pair list they
/// index. The join's jumper bitset lives beside the pair lists, in
/// [`ContactPairs`] (L10 C0). Kernel columns only (principle 0).
pub(crate) struct PairCarry {
    /// The tags being written this step, one per candidate pair; swapped with `tag_prev` at the
    /// start of each narrowphase. Its length only grows; rows at or past this step's pair count
    /// are stale and never read.
    tag: ScratchColumn<PairTag>,
    /// The previous step's tags, indexed by the slots of `ContactPairs::pairs_prev`.
    tag_prev: ScratchColumn<PairTag>,
    /// The reuse records being written this step, one slot per candidate pair (L9b); swapped
    /// with `rec_prev` at the start of each narrowphase. Grown only on a step that reuses, and a
    /// slot is valid only where this step's tag carries `REC`.
    rec: ScratchColumn<ReuseRecord>,
    /// The previous step's reuse records, indexed by the slots of `ContactPairs::pairs_prev`.
    rec_prev: ScratchColumn<ReuseRecord>,
    /// The parity of the last narrowphase path that opened the carry.
    parity: bool,
    /// The carry's place in the gather sequence (the row-identity protocol).
    cursor: RemapCursor,
    /// The stamp of the pair list the tags in `tag` index, or [`NO_SEQ`] when they index none a
    /// later step can trust (never stamped, or a path ran without the system's epilogue).
    seq: u64,
    /// The rotation ordinal of that list (`ContactPairs::rotations`), valid iff `seq` is not
    /// [`NO_SEQ`]: two lists built on one gather share a gather stamp but not an ordinal.
    list: u64,
    /// The number of tags written by the last narrowphase path (its pair count, or fewer when the
    /// column's reserve cut it short).
    len: usize,
    /// Steps whose carry was not joined although it had been stamped before: a missed gather or a
    /// stamp that is not `pairs_prev`'s. Diagnostic.
    resets: u64,
    /// Steps whose join ran across moved rows (a Rows step), reading the jumper bitset the
    /// broadphase rebuilt for them. Diagnostic; before L10 C0 the carry built the bitset itself
    /// on exactly these steps, and this counted the builds.
    jumper_builds: u64,
}

impl PairCarry {
    /// An empty carry, its tag columns reserved for `pairs` candidate pairs at least.
    pub(crate) fn with_capacity(pairs: usize) -> Self {
        register_narrowphase_column_layouts();
        let tag_reserve = pairs.max(scratch_reserve_rows(size_of::<PairTag>()));
        // A reservation is address space, not commit: the record columns commit only on a step
        // that reuses (2 × P × 128 B at P pairs, review O2).
        let rec_reserve = pairs.max(scratch_reserve_rows(size_of::<ReuseRecord>()));
        Self {
            tag: ScratchColumn::new(pair_tag_id(), tag_reserve),
            tag_prev: ScratchColumn::new(pair_tag_prev_id(), tag_reserve),
            rec: ScratchColumn::new(reuse_id(), rec_reserve),
            rec_prev: ScratchColumn::new(reuse_prev_id(), rec_reserve),
            parity: false,
            cursor: RemapCursor::default(),
            seq: NO_SEQ,
            list: 0,
            len: 0,
            resets: 0,
            jumper_builds: 0,
        }
    }

    /// The narrowphase system's prologue: classifies the carry against the current gather and
    /// returns the join's source — a Reset when the carry missed a gather, or when the list its
    /// tags index is not `pairs_prev` (module docs, "The stamps"). On a Rows step the source
    /// carries the jumper bitset `ContactPairs::rotate` built on this gather; a list rotated
    /// before this gather (a driver order the shipped schedule never takes) has none, and the
    /// step is a Reset too.
    pub(crate) fn source<'a>(
        &mut self,
        pairs: &'a ContactPairs,
        rows: &'a RowIdentity,
    ) -> CarryIn<'a> {
        let remap = self.cursor.remap(rows);
        if self.seq == NO_SEQ
            || pairs.seq_prev() != self.seq
            || pairs.rotations() != self.list.wrapping_add(1)
        {
            self.resets += u64::from(self.seq != NO_SEQ);
            return CarryIn::NONE;
        }
        let jumpers = match remap {
            RowRemap::Rows(_) => {
                if pairs.jumper_seq() != rows.gather_seq() {
                    self.resets += 1;
                    return CarryIn::NONE;
                }
                pairs.jumper_bits()
            }
            RowRemap::Identity | RowRemap::Reset => &[],
        };
        CarryIn::new(remap, pairs.pairs_prev(), jumpers)
    }

    /// A narrowphase path's prologue, before its first pair: swaps the tag and record columns and
    /// flips the parity, grows this step's tags to `n_pairs` (at most to its reserve), settles
    /// whether the step reuses (requested, and every pair has a tag and a record slot) and grows its
    /// record column if so, and returns the step's join and the columns the pairs' tags and records
    /// go into. Invalidates the stamp until [`stamp`](Self::stamp).
    pub(crate) fn open<'a>(
        &'a mut self,
        carry: CarryIn<'a>,
        n_pairs: usize,
    ) -> (PairJoin<'a>, &'a mut ScratchColumn<PairTag>, &'a mut ScratchColumn<ReuseRecord>) {
        let Self {
            tag,
            tag_prev,
            rec,
            rec_prev,
            parity,
            seq,
            len,
            jumper_builds,
            ..
        } = self;
        core::mem::swap(tag, tag_prev);
        core::mem::swap(rec, rec_prev);
        *parity = !*parity;
        *seq = NO_SEQ;
        if tag.len() < n_pairs {
            grow_tags(tag, n_pairs);
        }
        *len = n_pairs.min(tag.len());
        let mut reuse = carry.reuse;
        reuse.on &= *len == n_pairs && n_pairs <= rec.capacity();
        reuse.parity = *parity;
        if reuse.on && rec.len() < n_pairs {
            grow_records(rec, n_pairs);
        }
        debug_assert!(
            tag_prev.len() >= carry.pairs_prev.len(),
            "invariant: a joined step's previous tags cover every previous pair"
        );
        if let RowRemap::Rows(prev_row) = carry.remap {
            debug_assert!(
                carry.jumpers.len() >= prev_row.len().div_ceil(64),
                "invariant: a Rows step's jumper bitset holds a bit per current row"
            );
            *jumper_builds += 1;
        }
        let tag_prev: &'a ScratchColumn<PairTag> = tag_prev;
        let rec_prev: &'a ScratchColumn<ReuseRecord> = rec_prev;
        let join = PairJoin {
            remap: carry.remap,
            pairs_prev: carry.pairs_prev,
            tag_prev: tag_prev.as_read_slice(),
            jumpers: carry.jumpers,
            records: if reuse.on { rec_prev.as_read_slice() } else { &[] },
            reuse,
            waive_parity: false,
        };
        (join, tag, rec)
    }

    /// The narrowphase system's epilogue, after the pair loop: stamps the carry with the gather
    /// its tags are keyed by, and records the pair list they index (its gather stamp only when
    /// that list was built on this gather and every one of its pairs was tagged, and its
    /// rotation ordinal).
    pub(crate) fn stamp(&mut self, pairs: &ContactPairs, rows: &RowIdentity) {
        self.cursor.stamp(rows);
        self.seq = if pairs.seq() == rows.gather_seq() && self.len == pairs.pairs_stream().len() {
            pairs.seq()
        } else {
            NO_SEQ
        };
        self.list = pairs.rotations();
    }

    /// The tags the last narrowphase path wrote, in pair order.
    #[inline]
    pub(crate) fn tags(&self) -> &[PairTag] {
        &self.tag.as_read_slice()[..self.len]
    }

    /// The most candidate pairs a step can tag: the smaller reserve of the two tag columns.
    #[inline]
    pub(crate) fn capacity(&self) -> usize {
        self.tag.capacity().min(self.tag_prev.capacity())
    }

    /// The record column the last narrowphase path wrote (L9b): slot `k` is valid iff
    /// [`tags`](Self::tags)`[k]` carries `REC`.
    #[inline]
    pub(crate) fn records(&self) -> &[ReuseRecord] {
        self.rec.as_read_slice()
    }

    /// Steps whose carry was not joined although it had been stamped before.
    #[inline]
    pub(crate) fn resets(&self) -> u64 {
        self.resets + self.cursor.resets()
    }

    /// Steps whose join ran across moved rows, reading the jumper bitset.
    #[inline]
    pub(crate) fn jumper_builds(&self) -> u64 {
        self.jumper_builds
    }

    /// How this step's narrowphase will classify the carry against `rows`, without counting a
    /// Reset (L10 A1.1, design 06 B1: the broadphase prologue reads it before the narrowphase
    /// runs).
    #[inline]
    pub(crate) fn peek<'a>(&self, rows: &'a RowIdentity) -> RowRemap<'a> {
        self.cursor.peek(rows)
    }

    /// The tags and the records the last narrowphase wrote — one per slot of `pairs.pairs_prev()`,
    /// a record valid where its tag carries `REC` — when they index that list: read in the
    /// broadphase, after the rotation and before this step's narrowphase swaps the columns (L10
    /// A2.4, design 06 B2). `None` when the stamp does not tie them to `pairs_prev` (the step the
    /// narrowphase will run as a Reset).
    #[inline]
    pub(crate) fn written(&self, pairs: &ContactPairs) -> Option<(&[PairTag], &[ReuseRecord])> {
        (self.seq != NO_SEQ
            && pairs.seq_prev() == self.seq
            && pairs.rotations() == self.list.wrapping_add(1)
            && self.len == pairs.pairs_prev().len())
        .then(|| (self.tags(), self.rec.as_read_slice()))
    }
}

/// Grows this step's tag column to `n` rows, or to its reserve if `n` is past it (the serial loop
/// then leaves the pairs past the reserve untagged, and the stamp stays invalid).
#[cold]
#[inline(never)]
fn grow_tags(tag: &mut ScratchColumn<PairTag>, n: usize) {
    let n = n.min(tag.capacity());
    tag.build_view().resize(n, PairTag::NONE);
}

/// Grows this step's record column to `n` rows (the caller checked the reserve); the fill covers
/// only the new rows, which no pair reads before a step writes them.
#[cold]
#[inline(never)]
fn grow_records(rec: &mut ScratchColumn<ReuseRecord>, n: usize) {
    rec.build_view().resize(n, ReuseRecord::UNWRITTEN);
}

/// Rebuilds the jumper bitset for `rows` current rows: all clear, then one bit per row stage 2
/// resolved. `O(rows / 64 + stage2.len())`, on the steps whose rows moved; called by
/// `ContactPairs::rotate` at the start of the broadphase (L10 C0, design 06 Δ9).
#[cold]
#[inline(never)]
pub(crate) fn build_jumpers(bits: &mut ScratchColumn<u64>, rows: usize, stage2: &[u32]) {
    let mut view = bits.build_view();
    view.clear();
    view.resize(rows.div_ceil(64), 0);
    mark_jumpers(view.as_mut_slice(), rows, stage2);
}

/// Sets one bit per row of `stage2` in `words`, a cleared bitset of `rows` rows.
#[inline]
fn mark_jumpers(words: &mut [u64], rows: usize, stage2: &[u32]) {
    for &r in stage2 {
        debug_assert!(
            (r as usize) < rows,
            "invariant: a stage-2 row is a current row"
        );
        words[(r >> 6) as usize] |= 1 << (r & 63);
    }
}

/// The jumper bitset of `rows` current rows with `stage2` resolved by stage 2, as the broadphase
/// builds it: for a test that drives the carry with an explicit map.
#[cfg(test)]
pub(crate) fn jumper_words(rows: usize, stage2: &[u32]) -> Vec<u64> {
    let mut words = vec![0; rows.div_ceil(64)];
    mark_jumpers(&mut words, rows, stage2);
    words
}

#[cfg(test)]
mod tests {
    //! G-C-1 (`levers/L9-contact-reuse/02-DESIGN-REV1.md`, commit C2): scripted gathers drive the
    //! real row identity, pair lists and carry protocol, and every narrowphase's join is compared
    //! with a brute-force lookup of the pair's two ENTITIES in the pair list the carry's tags
    //! index. The tag each pair reads must be the tag written for the same two entities, flipped
    //! when their rows swapped order.

    #[cfg(not(miri))]
    use proptest::prelude::*;

    use super::*;
    use crate::row_identity::RowKey;

    /// The synthetic tag the harness writes for the pair whose bodies A and B are `ka` and `kb`:
    /// both axis fields and the `SEP` flag, so a read through the wrong slot or a missed flip is
    /// visible in the bits.
    fn synth(ka: RowKey, kb: RowKey) -> PairTag {
        let h = |key: RowKey, salt: u64| {
            let mut x = ((key.id() as u64) << 32) | u64::from(key.generation());
            x = x.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ salt;
            x ^ (x >> 29)
        };
        let mix = h(ka, 1).wrapping_add(h(kb, 2).rotate_left(17));
        let axis = (mix % 15) as u16;
        let sep = ((mix >> 8) % 15) as u16;
        PairTag(axis | PairTag::BOX | PairTag::SEP | (sep << PairTag::SEP_SHIFT))
    }

    /// What one narrowphase run recorded, for the next run's oracle.
    struct Record {
        /// The gather the run's tags were keyed by.
        gather: u64,
        /// Whether its pair list was built on that gather (the stamp is valid).
        pairs_on_gather: bool,
        /// The row keys of that gather.
        keys: Vec<RowKey>,
        /// Its pair list.
        pairs: Vec<(BodyIndex, BodyIndex)>,
    }

    /// Coverage of one script or random case.
    #[derive(Clone, Copy, Debug, Default)]
    struct Seen {
        hits: u64,
        misses: u64,
        flipped: u64,
        jumper_pairs: u64,
        rows_steps: u64,
        reset_steps: u64,
    }

    /// The protocol's pieces as the systems hold them: the gather's row identity, the pair lists
    /// and the carry. The harness mirrors the gathers' keys and the broadphase count for the
    /// oracle.
    struct Harness {
        rows: RowIdentity,
        pairs: ContactPairs,
        carry: PairCarry,
        /// The keys of the current gather, in row order.
        keys: Vec<RowKey>,
        /// The rows whose `RigidBody` the current gather flagged as added.
        added: Vec<u32>,
        /// Gathers run so far.
        gathers: u64,
        /// The gather the current pair list was built on.
        pairs_gather: u64,
        /// Broadphases since the last narrowphase.
        broadphases_since: usize,
        /// The last narrowphase's record.
        last: Option<Record>,
        seen: Seen,
    }

    impl Harness {
        fn new() -> Self {
            Self {
                rows: RowIdentity::with_capacity(0),
                pairs: ContactPairs::with_capacity(0),
                carry: PairCarry::with_capacity(0),
                keys: Vec::new(),
                added: Vec::new(),
                gathers: 0,
                pairs_gather: 0,
                broadphases_since: 0,
                last: None,
                seen: Seen::default(),
            }
        }

        /// One gather of `keys` in walk order, `added` flagged (ascending).
        fn gather(&mut self, keys: &[RowKey], added: &[u32]) {
            self.rows.begin_gather();
            {
                let (mut cur, mut add) = self.rows.gather_views();
                for &key in keys {
                    cur.push(key);
                }
                for &r in added {
                    add.push(r);
                }
            }
            self.rows.finish_gather();
            self.keys = keys.to_vec();
            self.added = added.to_vec();
            self.gathers += 1;
        }

        /// One broadphase: the pairs of the given entities present in the current gather, in
        /// `(min, max)` row order, after the rotation.
        fn broadphase(&mut self, entity_pairs: &[(RowKey, RowKey)]) {
            let row_of = |key: RowKey| self.keys.iter().position(|&x| x == key).map(|r| r as u32);
            let mut list: Vec<(BodyIndex, BodyIndex)> = entity_pairs
                .iter()
                .filter_map(|&(x, y)| {
                    let (rx, ry) = (row_of(x)?, row_of(y)?);
                    (rx != ry).then(|| (BodyIndex(rx.min(ry)), BodyIndex(rx.max(ry))))
                })
                .collect();
            list.sort_unstable();
            list.dedup();
            self.pairs.rotate(&self.rows);
            {
                let mut view = self.pairs.pairs_build();
                view.clear();
                for p in list {
                    view.push(p);
                }
            }
            self.pairs_gather = self.gathers;
            self.broadphases_since += 1;
        }

        /// The join by its definition: the slot of the recorded list whose two entities are the
        /// pair's, or none — none for every pair when the protocol calls for a Reset.
        fn expected(&self) -> Vec<Option<JoinHit>> {
            let now = self.pairs.pairs();
            let joinable = self.last.as_ref().filter(|r| {
                r.pairs_on_gather
                    && self.broadphases_since == 1
                    && (self.gathers == r.gather || self.gathers == r.gather + 1)
            });
            let Some(rec) = joinable else {
                return vec![None; now.len()];
            };
            let new_body = |r: u32| self.gathers == rec.gather + 1 && self.added.contains(&r);
            now.iter()
                .map(|&(a, b)| {
                    if new_body(a.0) || new_body(b.0) {
                        return None;
                    }
                    let (ka, kb) = (self.keys[a.0 as usize], self.keys[b.0 as usize]);
                    rec.pairs.iter().enumerate().find_map(|(j, &(pa, pb))| {
                        let (pka, pkb) = (rec.keys[pa.0 as usize], rec.keys[pb.0 as usize]);
                        if (pka, pkb) == (ka, kb) {
                            Some(JoinHit {
                                j: j as u32,
                                flipped: false,
                            })
                        } else if (pka, pkb) == (kb, ka) {
                            Some(JoinHit {
                                j: j as u32,
                                flipped: true,
                            })
                        } else {
                            None
                        }
                    })
                })
                .collect()
        }

        /// One narrowphase through the protocol (`source`, `open`, the pairs, `stamp`): checks
        /// every pair's join and the tag it reads against the oracle, then writes the synthetic
        /// tags.
        fn narrowphase(&mut self) -> Result<(), String> {
            let expected = self.expected();
            let now: Vec<(BodyIndex, BodyIndex)> = self.pairs.pairs().to_vec();
            let src = self.carry.source(&self.pairs, &self.rows);
            match src.remap {
                RowRemap::Rows(_) => self.seen.rows_steps += 1,
                RowRemap::Reset => self.seen.reset_steps += 1,
                RowRemap::Identity => {}
            }
            let (join, tag_column, _) = self.carry.open(src, now.len());
            let mut cursor = join.cursor();
            let mut reads = join.cursor();
            for (k, &(a, b)) in now.iter().enumerate() {
                let got = cursor.find(a, b);
                if got != expected[k] {
                    return Err(format!(
                        "gather {}, pair {k} ({}, {}) = entities ({:?}, {:?}): joined {got:?}, the \
                         entity lookup says {:?}",
                        self.gathers,
                        a.0,
                        b.0,
                        self.keys[a.0 as usize],
                        self.keys[b.0 as usize],
                        expected[k]
                    ));
                }
                let read = reads.prev_tag(a, b);
                let want = match expected[k] {
                    Some(hit) => {
                        let rec = self.last.as_ref().expect("invariant: a hit has a record");
                        let (pa, pb) = rec.pairs[hit.j as usize];
                        let tag = synth(rec.keys[pa.0 as usize], rec.keys[pb.0 as usize]);
                        if hit.flipped { tag.flipped() } else { tag }
                    }
                    None => PairTag::NONE,
                };
                if read != want {
                    return Err(format!(
                        "gather {}, pair {k}: read tag {read:?}, the pair's entities wrote {want:?}",
                        self.gathers
                    ));
                }
                if let Some(hit) = got {
                    self.seen.hits += 1;
                    self.seen.flipped += u64::from(hit.flipped);
                    if let RowRemap::Rows(_) = join.remap {
                        self.seen.jumper_pairs +=
                            u64::from(join.is_jumper(a.0) || join.is_jumper(b.0));
                    }
                } else {
                    self.seen.misses += 1;
                }
            }
            {
                let mut view = tag_column.build_view();
                let tags = view.as_mut_slice();
                for (k, &(a, b)) in now.iter().enumerate() {
                    tags[k] = synth(self.keys[a.0 as usize], self.keys[b.0 as usize]);
                }
            }
            self.carry.stamp(&self.pairs, &self.rows);
            self.last = Some(Record {
                gather: self.gathers,
                pairs_on_gather: self.pairs_gather == self.gathers,
                keys: self.keys.clone(),
                pairs: now,
            });
            self.broadphases_since = 0;
            Ok(())
        }
    }

    /// Slot `id` at generation 0.
    fn key(id: usize) -> RowKey {
        RowKey::new(id, 0)
    }

    /// The keys of `ids` at generation 0.
    fn keys(ids: &[usize]) -> Vec<RowKey> {
        ids.iter().map(|&id| key(id)).collect()
    }

    /// Every pair of `ids` whose slots differ by at most `reach`, as entity pairs.
    fn near_pairs(ids: &[usize], reach: usize) -> Vec<(RowKey, RowKey)> {
        let mut out = Vec::new();
        for (i, &x) in ids.iter().enumerate() {
            for &y in &ids[i + 1..] {
                if x.abs_diff(y) <= reach {
                    out.push((key(x), key(y)));
                }
            }
        }
        out
    }

    /// One scripted operation.
    enum Op {
        Gather(Vec<RowKey>, Vec<u32>),
        Broadphase(Vec<(RowKey, RowKey)>),
        Narrowphase,
    }

    fn run(ops: &[Op]) -> Result<Seen, String> {
        let mut h = Harness::new();
        for op in ops {
            match op {
                Op::Gather(keys, added) => h.gather(keys, added),
                Op::Broadphase(pairs) => h.broadphase(pairs),
                Op::Narrowphase => h.narrowphase()?,
            }
        }
        Ok(h.seen)
    }

    /// Gather, broadphase and narrowphase of one step over `ids` (nothing flagged added) and the
    /// pairs within `reach`.
    fn step(ids: &[usize], reach: usize) -> [Op; 3] {
        [
            Op::Gather(keys(ids), Vec::new()),
            Op::Broadphase(near_pairs(ids, reach)),
            Op::Narrowphase,
        ]
    }

    const TEN: [usize; 10] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9];

    /// One G-C-1 script: its name, its operations and what it must have exercised.
    type Script = (&'static str, Vec<Op>, fn(&Seen) -> bool);

    /// G-C-1, the design's six scripts (and the protocol's other Reset paths): every join equals
    /// the entity lookup, and every tag read is the one its two entities wrote.
    ///
    /// Mutations recorded red (C2's red-first log): M-c1, the untranslated key on a Rows step
    /// (the shift script); M-c2, jumper pairs merged instead of searched (the swap-remove and flip
    /// scripts); M-c3, re-specified by the review (O4: stamping the cursor after the
    /// classification but before the loop is unobservable) as M-c3a, the cursor stamped BEFORE
    /// its classification (the shift script reads the rows unmoved), and M-c3b, the pair list's
    /// stamp not compared (the narrowphase-twice script joins a list its tags do not index). And
    /// the rotation ordinal not compared: the two-broadphases-on-one-gather script joins the
    /// middle list, whose gather stamp equals the tagged list's.
    #[test]
    fn scripted_joins_equal_the_entity_lookup() {
        let shifted = [0, 1, 2, 4, 5, 6, 7, 8, 9];
        let swap_removed = [0, 1, 2, 9, 4, 5, 6, 7, 8];
        let migrated = [1, 2, 3, 4, 5, 6, 7, 8, 9, 0];
        let appended = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11];
        let recycled = vec![
            key(0),
            key(1),
            key(2),
            RowKey::new(3, 1),
            key(4),
            key(5),
            key(6),
            key(7),
            key(8),
            key(9),
        ];
        let cases: Vec<Script> = vec![
            (
                "identity: the same rows, the pair set changes",
                [step(&TEN, 2), step(&TEN, 3), step(&TEN, 1)]
                    .into_iter()
                    .flatten()
                    .collect(),
                |s| s.hits > 0 && s.misses > 0 && s.rows_steps == 0,
            ),
            (
                "shift: row 3 removed, every later row moves down one",
                [step(&TEN, 3), step(&shifted, 3)]
                    .into_iter()
                    .flatten()
                    .collect(),
                |s| s.hits > 0 && s.rows_steps == 1 && s.jumper_pairs == 0,
            ),
            (
                "swap-remove: row 3 despawned, the tail body moves into it (a jumper)",
                [step(&TEN, 9), step(&swap_removed, 9)]
                    .into_iter()
                    .flatten()
                    .collect(),
                |s| s.rows_steps == 1 && s.jumper_pairs > 0 && s.flipped > 0,
            ),
            (
                "append: two new bodies flagged added",
                vec![
                    Op::Gather(keys(&TEN), Vec::new()),
                    Op::Broadphase(near_pairs(&TEN, 3)),
                    Op::Narrowphase,
                    Op::Gather(keys(&appended), vec![10, 11]),
                    Op::Broadphase(near_pairs(&appended, 3)),
                    Op::Narrowphase,
                ],
                |s| s.hits > 0 && s.misses > 0 && s.rows_steps == 1,
            ),
            (
                "order flip: the first body migrates to the last row",
                [step(&TEN, 9), step(&migrated, 9)]
                    .into_iter()
                    .flatten()
                    .collect(),
                |s| s.rows_steps == 1 && s.flipped > 0 && s.jumper_pairs > 0,
            ),
            (
                "recycled slot: slot 3 despawned and reused at generation 1 in the same row",
                vec![
                    Op::Gather(keys(&TEN), Vec::new()),
                    Op::Broadphase(near_pairs(&TEN, 2)),
                    Op::Narrowphase,
                    Op::Gather(recycled.clone(), Vec::new()),
                    Op::Broadphase(near_pairs(&TEN, 2)),
                    Op::Narrowphase,
                ],
                |s| s.hits > 0 && s.misses > 0,
            ),
            (
                "reset: a gather the carry missed",
                vec![
                    Op::Gather(keys(&TEN), Vec::new()),
                    Op::Broadphase(near_pairs(&TEN, 2)),
                    Op::Narrowphase,
                    Op::Gather(keys(&TEN), Vec::new()),
                    Op::Gather(keys(&TEN), Vec::new()),
                    Op::Broadphase(near_pairs(&TEN, 2)),
                    Op::Narrowphase,
                ],
                |s| s.hits == 0 && s.reset_steps == 2,
            ),
            (
                "reset: the narrowphase ran twice on one pair list",
                vec![
                    Op::Gather(keys(&TEN), Vec::new()),
                    Op::Broadphase(near_pairs(&TEN, 2)),
                    Op::Narrowphase,
                    Op::Gather(keys(&TEN), Vec::new()),
                    Op::Broadphase(near_pairs(&TEN, 2)),
                    Op::Narrowphase,
                    Op::Narrowphase,
                ],
                |s| s.hits > 0 && s.reset_steps == 2,
            ),
            (
                "reset: the broadphase ran twice between two narrowphases",
                vec![
                    Op::Gather(keys(&TEN), Vec::new()),
                    Op::Broadphase(near_pairs(&TEN, 2)),
                    Op::Narrowphase,
                    Op::Gather(keys(&TEN), Vec::new()),
                    Op::Broadphase(near_pairs(&TEN, 2)),
                    Op::Broadphase(near_pairs(&TEN, 3)),
                    Op::Narrowphase,
                ],
                |s| s.hits == 0 && s.reset_steps == 2,
            ),
            (
                "reset: two broadphases on one gather between two narrowphases",
                vec![
                    Op::Gather(keys(&TEN), Vec::new()),
                    Op::Broadphase(near_pairs(&TEN, 2)),
                    Op::Narrowphase,
                    Op::Broadphase(near_pairs(&TEN, 1)),
                    Op::Broadphase(near_pairs(&TEN, 2)),
                    Op::Narrowphase,
                ],
                |s| s.hits == 0 && s.reset_steps == 2,
            ),
            (
                "one gather, two broadphase + narrowphase rounds: the second joins",
                vec![
                    Op::Gather(keys(&TEN), Vec::new()),
                    Op::Broadphase(near_pairs(&TEN, 2)),
                    Op::Narrowphase,
                    Op::Broadphase(near_pairs(&TEN, 3)),
                    Op::Narrowphase,
                ],
                |s| s.hits > 0 && s.reset_steps == 1,
            ),
        ];
        for (name, ops, covered) in cases {
            let seen = run(&ops).unwrap_or_else(|e| panic!("{name}: {e}"));
            println!("G-C-1 {name}: {seen:?}");
            assert!(
                covered(&seen),
                "{name}: the script did not exercise its case: {seen:?}"
            );
        }
    }

    /// A small seeded generator for the random arm.
    #[cfg(not(miri))]
    struct TestRng(u64);

    #[cfg(not(miri))]
    impl TestRng {
        fn below(&mut self, n: u64) -> u64 {
            self.0 ^= self.0 >> 12;
            self.0 ^= self.0 << 25;
            self.0 ^= self.0 >> 27;
            self.0.wrapping_mul(0x2545_F491_4F6C_DD1D) % n
        }
    }

    /// One random structural change between two gathers; returns the rows flagged added.
    #[cfg(not(miri))]
    fn churn(rng: &mut TestRng, live: &mut Vec<RowKey>, next_slot: &mut usize) -> Vec<u32> {
        let mut added = Vec::new();
        match rng.below(6) {
            // Swap-remove: the tail moves into the freed row.
            0 if live.len() > 2 => {
                let r = rng.below(live.len() as u64) as usize;
                live.swap_remove(r);
            }
            // An ordered removal: every later row moves down one.
            1 if live.len() > 2 => {
                let r = rng.below(live.len() as u64) as usize;
                live.remove(r);
            }
            // A migration to the last row.
            2 if live.len() > 2 => {
                let r = rng.below(live.len() as u64) as usize;
                let moved = live.remove(r);
                live.push(moved);
            }
            // A spawn, appended and flagged added.
            3 => {
                live.push(RowKey::new(*next_slot, 0));
                *next_slot += 1;
                added.push(live.len() as u32 - 1);
            }
            // A slot recycled at the next generation in the same row.
            4 if !live.is_empty() => {
                let r = rng.below(live.len() as u64) as usize;
                live[r] = RowKey::new(live[r].id(), live[r].generation() + 1);
            }
            _ => {}
        }
        added
    }

    /// G-C-1's random arm: sequences of gathers with random swap-removes, ordered removals,
    /// migrations, spawns and recycled slots, each followed by a broadphase over a random pair
    /// set and a narrowphase; every join and every tag read equals the entity lookup.
    #[test]
    #[cfg(not(miri))]
    fn random_joins_equal_the_entity_lookup() {
        let totals = std::cell::Cell::new(Seen::default());
        let config = ProptestConfig {
            cases: 256,
            failure_persistence: None,
            ..ProptestConfig::default()
        };
        proptest!(config, |(seed in any::<u64>())| {
            let mut rng = TestRng(seed | 1);
            let mut live: Vec<RowKey> = (0..12).map(|id| RowKey::new(id, 0)).collect();
            let mut next_slot = 12;
            let mut h = Harness::new();
            for step in 0..8 {
                let added = if step == 0 { Vec::new() } else { churn(&mut rng, &mut live, &mut next_slot) };
                h.gather(&live, &added);
                let mut pairs = Vec::new();
                for (i, &x) in live.iter().enumerate() {
                    for &y in &live[i + 1..] {
                        if rng.below(3) != 0 {
                            pairs.push((x, y));
                        }
                    }
                }
                h.broadphase(&pairs);
                if let Err(e) = h.narrowphase() {
                    prop_assert!(false, "seed {:#x}, step {}: {}", seed, step, e);
                }
            }
            let mut t = totals.get();
            t.hits += h.seen.hits;
            t.misses += h.seen.misses;
            t.flipped += h.seen.flipped;
            t.jumper_pairs += h.seen.jumper_pairs;
            t.rows_steps += h.seen.rows_steps;
            t.reset_steps += h.seen.reset_steps;
            totals.set(t);
        });
        let t = totals.get();
        println!("G-C-1 random coverage: {t:?}");
        assert!(
            t.hits > 0 && t.flipped > 0 && t.jumper_pairs > 0 && t.rows_steps > 0,
            "{t:?}"
        );
    }

    /// L10 C0 (design 06 D-C): `rekeys` over every `u16` is L9's commit field — the tag's axis
    /// field is not none — whatever the flag bits. The behaviour over the outcomes
    /// `collide_box_pair` reaches is `systems::pair_tag_rekeys_tests`.
    #[test]
    fn rekeys_is_the_axis_field_over_every_tag() {
        let mut rekeying = 0u32;
        for bits in 0..=u16::MAX {
            let tag = PairTag(bits);
            let axis_set = bits & PairTag::AXIS_MASK != PairTag::AXIS_NONE;
            assert_eq!(tag.rekeys(), axis_set, "tag {bits:#06x}");
            rekeying += u32::from(axis_set);
        }
        assert_eq!(rekeying, 15 << 12, "15 of the 16 axis values re-key, under every flag word");
    }

    /// The flip remap is an involution and exchanges A's faces with B's and `(ea, eb)` with
    /// `(eb, ea)`; a flipped tag keeps its flags.
    #[test]
    fn flip_axis_exchanges_the_roles() {
        for axis in 0..SAT_AXIS_COUNT {
            assert_eq!(flip_axis(flip_axis(axis)), axis);
        }
        assert_eq!(flip_axis(0), 3);
        assert_eq!(flip_axis(5), 2);
        assert_eq!(flip_axis(6 + 3 + 2), 6 + 3 * 2 + 1);
        assert_eq!(flip_axis(6 + 3 * 2 + 2), 6 + 3 * 2 + 2);
        let t = PairTag::box_separated(7, true);
        assert_eq!(t.flipped().sep_axis(), Some(flip_axis(7)));
        assert!(
            t.flipped()
                .has(PairTag::SEPHIT | PairTag::SEP | PairTag::BOX)
        );
        assert_eq!(PairTag::NONE.flipped(), PairTag::NONE);
        assert_eq!(PairTag::box_contact(4, true).flipped().axis(), Some(1));
    }
}
