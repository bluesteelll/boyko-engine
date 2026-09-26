//! L10: the frozen-pair skip ("sleep sets") of `docs/physics/perf-campaign/levers/L10-sleeping/`
//! (design `04-DESIGN-REV2.md` + `06-DESIGN-REV2.2.md` + `08-DESIGN-REV2.3.md`, and the rulings
//! of `levers/00-RULINGS.md`).
//!
//! With sleeping on and [`SleepSkip::Sets`] (the default mode), a frozen, clean island is
//! **held**: it pays no narrowphase, no SDF stage, no graph colouring and no solve, while every
//! public view still reports its contacts ([`PairsView`](crate::resources::PairsView),
//! [`ManifoldsView`](crate::resources::ManifoldsView)). [`SleepSkip::Off`] is the oracle: every
//! mode reproduces its observables bit for bit.
//!
//! # The step (design 04 A1–A6, 06, 08)
//!
//! The colored broadphase runs the **prologue** before its kind arm and the **epilogue** after it:
//!
//! * **A1.0** the step mode (`Off` with sleeping off or no colored solve, else the configured
//!   one); **A1.0b** compaction of records tombstoned on earlier steps;
//! * **A1.1** the cursors — L10's own, the awake mask's, the latch's and L9's pair carry's must
//!   classify alike, and the gather must have kept the resting baseline; otherwise the step is a
//!   D7 flush and nothing rests;
//! * **A1.2** on a Rows step every record's row table is translated through `inv`;
//! * **A1.3** every row's [`RowCls`] is rewritten (ruling W4): `RESTING` (R1–R4 of design 04
//!   D2), `PRE_HELD`, `CAND`, `LATCHED`, `SENSOR`;
//! * **A1.4** the per-record rules D1/D1a/D8 and the flushes D5 (the epoch), D6 (`wake_all`),
//!   D7 (the cursors);
//! * then, with a mode other than `Sets` and a live store, the **D-H flush**: every record is
//!   restored, the rows are classified `RESTORED`/`SENSOR` only, and — with the mode `Off` — the
//!   restored warm records are drained into the solver by the broadphase itself, never by the
//!   solve (ruling, open question 2); otherwise `SLEEPER` on the members of every record the
//!   prologue kept (design 04 T1);
//! * the kind arm: on a step with a sleeper the tree broadphase reads [`HeldHint`] (T1, T2) and
//!   withholds the pairs of its sleeper set from the stream (T3; `broadphase_tree`, "The sleeper
//!   set"); on any other step it reads no hint, and its sleeper set dissolves;
//! * the epilogue, `Sets` only: **A2.1** D3 (an Identity step whose axis table clears restores
//!   every island keeping a box-box manifold), **A2.2** the D2 scan (a held or candidate row's
//!   stream pair to a row that does not rest restores or refuses its island; a sensor pair marks
//!   `SENSOR_NBR`), **A2.5a** the restores (tombstone, `RESTORED`, the two restore sources, the
//!   `order` filter), **A2.4** the move-ins (M1–M4, B5′), the `order` merge, and `HELD`; then
//!   **A2.3**, the tree's release of the restored rows (T4), which moves their withheld pairs
//!   into the stream before the narrowphase.
//!
//! The narrowphase then skips a non-sensor pair with a `HELD` endpoint and computes a non-sensor
//! pair with a `RESTORED` endpoint from the restore pair source through L9's own join
//! (ruling W2); the SDF stage skips `HELD` rows; the graph pre-roots held islands; the solve reads
//! `HELD` through `effective_inv_mass`, searches the restore warm source after its own store, and
//! captures a move-in's warm records (A1′).

use boyko_ecs::ecs::core::component::scratch::ScratchColumn;
use boyko_macros::Resource;
use boyko_sdf_math::{MAX_SDF_EDITS, SdfEdit};

use crate::broadphase_tree::SleepHint;
use crate::components::ColliderShape;
use crate::held_store::{HeldIsland, HeldStore, HeldView, KeptManifold, KeptPair, NONE, RestoreSort};
use crate::manifold::{BodyIndex, Manifold, SDF_SENTINEL};
use crate::narrowphase::axis_cache::{BoxAxisCache, KeyChange};
use crate::narrowphase::carry::PairTag;
use crate::narrowphase::reuse::ReuseRecord;
use crate::profiling::{PHYS_SLEEP_CLASSIFY, PHYS_SLEEP_HELD, counter};
use crate::resources::{BodyState, ConstraintGraph, IslandSleep, PhysicsConfig, SdfNarrowphaseKernel, SleepSkip, HELD_BASE};
use crate::row_identity::{NO_ROW, RemapCursor, RowIdentity, RowRemap};
use crate::scratch_ids::{
    register_sleep_sets_layouts, scratch_reserve_rows, sleep_cand_id, sleep_inv_id,
    sleep_kept_rec_id, sleep_restore_pair_ids, sleep_restore_rec_ids, sleep_restore_sort_id,
    sleep_row_cls_id, sleep_scratch_id,
};
use crate::sdf_query::SdfField;
use crate::step_inputs::StepInputs;
use crate::solver::warm_records::{WarmRecord, WarmRecords, ord};

/// One row's sleep-skip classification for the step (design 04 A1.3, 06, 08 D-F), 8 B. Read by
/// the broadphase, the narrowphase, the SDF stage, the graph and the solve, so it is the only
/// per-step L10 read of the hot path (10 KB at the Jolt pyramid, L1-resident).
///
/// Rewritten for every row on every step L10 runs (ruling W4): a skipped pass would have to
/// zero it, which is the same pass with a constant.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct RowCls {
    /// The held record, or the previous step's candidate island, of the row, or
    /// [`NO_ISLAND`](Self::NO_ISLAND).
    pub(crate) island: u32,
    /// The row's flags: [`RESTING`](Self::RESTING), [`HELD`](Self::HELD),
    /// [`PRE_HELD`](Self::PRE_HELD), [`CAND`](Self::CAND), [`SENSOR`](Self::SENSOR),
    /// [`LATCHED`](Self::LATCHED), [`RESTORED`](Self::RESTORED),
    /// [`SENSOR_NBR`](Self::SENSOR_NBR), [`SLEEPER`](Self::SLEEPER).
    pub(crate) flags: u32,
}

const _: () = assert!(
    size_of::<RowCls>() == 8 && align_of::<RowCls>() == 4,
    "RowCls is the 8 B, align 4 per-row classification element"
);

impl RowCls {
    /// The row's inputs are bit-equal to the previous step's (R1–R4 of design 04 D2): it has a
    /// previous row, that row was immovable or not awake, its `BodyState` is bit-equal, the
    /// cursors classify, and it is no jumper and no degenerate box.
    pub(crate) const RESTING: u32 = 1 << 0;
    /// The row is held this step: its pairs are skipped and its island is not solved.
    pub(crate) const HELD: u32 = 1 << 1;
    /// The row was held at the previous step (its record survives into this one).
    pub(crate) const PRE_HELD: u32 = 1 << 2;
    /// The row's island froze at the previous step and is not held: a move-in candidate.
    pub(crate) const CAND: u32 = 1 << 3;
    /// The row is a sensor ([`BodyState::is_sensor`]).
    pub(crate) const SENSOR: u32 = 1 << 4;
    /// The row's sleep latch is asleep after the previous step's `end_step`.
    pub(crate) const LATCHED: u32 = 1 << 5;
    /// The row belongs to a record restored this step: its pairs are computed from the kept
    /// source, not the stream's (design 06 B4).
    pub(crate) const RESTORED: u32 = 1 << 6;
    /// The row is a held or candidate row with a sensor pair in the stream (design 08 D-F):
    /// its frames are filled even while it is held.
    pub(crate) const SENSOR_NBR: u32 = 1 << 7;
    /// The row is a member of a held record the prologue did not mark for restore (design 04
    /// T1, ruling W1 of rev 2.1): the tree broadphase may hold it in its sleeper set this step
    /// ([`HeldHint`]). A record the prologue restores is out of the hint, so its pairs are found
    /// by this step's queries, never withheld.
    pub(crate) const SLEEPER: u32 = 1 << 8;
    /// No island: the value of [`island`](Self::island) for a row that has none.
    pub(crate) const NO_ISLAND: u32 = u32::MAX;

    /// Whether the row is held this step: THE predicate the graph's colouring and the solve's
    /// effective inverse mass both read (design 04 D8, ruling W1).
    #[inline]
    pub(crate) fn is_held(self) -> bool {
        self.flags & Self::HELD != 0
    }

    /// Whether the narrowphase's frame fill writes this row's frame (design 08 D-F): a row that
    /// is not held, or a held row with a sensor pair. ANDed with L9's own fill mask (the box
    /// rows).
    #[inline]
    pub(crate) fn fills(self) -> bool {
        self.flags & Self::HELD == 0 || self.flags & Self::SENSOR_NBR != 0
    }
}

/// Where one candidate pair's collision state comes from this step (design 08 D1′).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Route {
    /// Collided from the stream: L9's `pairs_prev` join, as `Off` collides every pair.
    Stream,
    /// Not collided: an endpoint is held. The slot's tag is `PairTag::HELD_SKIP`, its axis
    /// commit `AXIS_NONE`, and it writes no manifold.
    Skip,
    /// Collided from the kept source of a record restored this step (design 06 B4, E2).
    Restore,
}

/// Whether `(a, b)` is a sensor pair from its two rows' flags: THE one sensor predicate of
/// L10 (design 08 D-G). The route, the kept unit and its anchors, the D2 scan, the frame fill
/// and the tree's hint all call it, so a sensor pair is computed every step whatever its
/// endpoints are.
#[inline]
pub(crate) fn sensor_pair(fa: u32, fb: u32) -> bool {
    (fa | fb) & RowCls::SENSOR != 0
}

/// The tree broadphase's sleep hint on a `Sets` step with at least one sleeper (design 04 T1,
/// T2; ruling W1 of rev 2.1), read from the classification the prologue wrote for this gather:
/// * `frozen(r)` — [`RowCls::SLEEPER`]: the row's held record survives the prologue;
/// * `anchor_ok(r)` — [`RowCls::RESTING`] and no sensor: only such a row may be a member of the
///   tree's static or sleeper set, so every pair the tree withholds has two resting, non-sensor
///   endpoints (Invariant V), and a sensor pair is computed every step.
///
/// A `Sets` step with no sleeper reads the tree's `NoHint` instead ([`StepPlan::sleepers`]): the
/// sleeper set dissolves either way, so nothing is withheld and Invariant V has no pair to hold
/// for, while `anchor_ok` would evict every static on a flush step, where nothing rests.
#[derive(Clone, Copy, Debug)]
pub(crate) struct HeldHint<'a> {
    /// The step's per-row classification, indexed by current row.
    cls: &'a [RowCls],
}

impl<'a> HeldHint<'a> {
    /// The hint over `cls`, the classification the prologue wrote for the current gather.
    #[inline]
    pub(crate) fn new(cls: &'a [RowCls]) -> Self {
        Self { cls }
    }
}

impl SleepHint for HeldHint<'_> {
    #[inline]
    fn frozen(&self, r: usize) -> bool {
        self.cls[r].flags & RowCls::SLEEPER != 0
    }

    #[inline]
    fn anchor_ok(&self, r: usize) -> bool {
        let f = self.cls[r].flags;
        f & RowCls::RESTING != 0 && !sensor_pair(f, 0)
    }
}

/// How the narrowphase treats candidate pair `(a, b)` this step (design 08 D1′): one predicate
/// for the parallel chunks and the serial loop.
///
/// With `SETS == false` (the `Off` arm) every pair is collided from the stream, and `cls` is not
/// read. With `SETS == true` a non-sensor pair with a held endpoint is skipped, a non-sensor pair
/// with a restored endpoint is collided from the kept source, and every other pair from the
/// stream; `cls` is this step's per-row classification.
#[inline]
pub(crate) fn np_route<const SETS: bool>(cls: &[RowCls], a: BodyIndex, b: BodyIndex) -> Route {
    if !SETS {
        return Route::Stream;
    }
    let fa = cls[a.0 as usize].flags;
    let fb = cls[b.0 as usize].flags;
    if sensor_pair(fa, fb) {
        return Route::Stream;
    }
    let f = fa | fb;
    if f & RowCls::HELD != 0 {
        Route::Skip
    } else if f & RowCls::RESTORED != 0 {
        Route::Restore
    } else {
        Route::Stream
    }
}

/// L10's inputs to the narrowphase's `Sets` arm, shared read-only by the serial loop and every
/// chunk: the step's per-row classification and the restore pair source (design 06 B4).
#[derive(Clone, Copy, Debug)]
pub(crate) struct NpSets<'a> {
    /// The step's per-row classification.
    pub(crate) cls: &'a [RowCls],
    /// The restore pair source's keys (previous rows, strictly sorted).
    pub(crate) keys: &'a [(BodyIndex, BodyIndex)],
    /// Their tags.
    pub(crate) tags: &'a [PairTag],
    /// Their reuse records.
    pub(crate) reuse: &'a [ReuseRecord],
}

impl NpSets<'static> {
    /// The `Off` arm's inputs: none.
    pub(crate) const OFF: Self = Self { cls: &[], keys: &[], tags: &[], reuse: &[] };
}

/// What the narrowphase's `Sets` arm counted in one step.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct NpCounts {
    /// Pairs skipped for a held endpoint.
    pub(crate) skips: u32,
    /// Pairs computed from the restore source.
    pub(crate) restored: u32,
    /// Of those, the pairs whose join found their kept state.
    pub(crate) restore_hits: u32,
}

/// The admission rule M3 (design 06 B5, 08 B5′) for one box pair's previous-step tag: a `REC`
/// pair iff it hit or settled, a `SEP` pair always, a pair with a contact iff it emitted a
/// manifold and settled; anything else (no contact for a reason other than a separating axis,
/// a contact with no point) refuses the island. The held-skip tag is never admitted (rule 5).
/// One AND per test.
#[inline]
pub(crate) fn admit(tag: PairTag) -> bool {
    if tag.is_held_skip() {
        false
    } else if tag.has(PairTag::REC) {
        tag.bits() & (PairTag::HIT | PairTag::SETTLED) != 0
    } else if tag.has(PairTag::SEP) {
        true
    } else if tag.has(PairTag::SET) {
        tag.has(PairTag::PUSHED | PairTag::SETTLED)
    } else {
        false
    }
}

/// Whether a pair with previous-step tag `tag` carries state a held island must keep (design 06
/// B2): a reuse record, a separating axis or a manifold.
#[inline]
fn kept_class(tag: PairTag) -> bool {
    !tag.is_held_skip() && tag.bits() & (PairTag::REC | PairTag::SEP | PairTag::PUSHED) != 0
}

/// Words in one [`SdfEdit`]'s bits: 4 + 4 + 1 + 1 + 1 + 1.
const SDF_EDIT_WORDS: usize = 12;

/// Every config input a held island's `Off` outputs depend on, as bits (design 04 D5, 06 D-E,
/// 08 D-E′). Any change between two steps flushes every held island, because `Off` would
/// recompute them with different outputs. Compared with `==`: every field is an integer, the
/// floats and the SDF edit list are stored by their bits.
///
/// Not the body-side inputs (pose, velocity, shape, `is_sensor`): those are `BodyState` fields,
/// which R3 compares per row. Not `parallel_narrowphase`, `broadphase` or `parallel_broadphase`
/// (their outputs are identical by L5's theorem and the exact pair set), nor gravity, substeps or
/// the solver constants (a frozen island is not solved, and its integrate is restored).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SleepEpoch {
    /// The step mode.
    mode: u8,
    /// The effective warm start: the colored solver's setup flag AND the latched
    /// `PhysicsConfig::warm_start` (L10 D5b). A change restores every held island: `Off` keeps a
    /// frozen island's records only while warm.
    warm: u8,
    /// `PhysicsConfig::sdf_narrowphase`.
    sdf_kernel: u8,
    /// `PhysicsConfig::contact_reuse` (design 06 D-E).
    reuse: u8,
    /// The SDF edit count, `u32::MAX` for a world with no SDF field.
    sdf_len: u32,
    /// `PhysicsConfig::contact_reuse_distance`'s bits (design 06 D-E).
    tau_bits: u32,
    /// `PhysicsConfig::dt`'s bits (design 08 D-E′: L9's `is_fast` reads it).
    dt_bits: u32,
    /// The live SDF edits by bits; the entries past `sdf_len` are zero.
    sdf_edits: [[u32; SDF_EDIT_WORDS]; MAX_SDF_EDITS],
}

impl SleepEpoch {
    /// The epoch no step has recorded: it differs from every recorded one.
    const UNSET: Self = Self {
        mode: u8::MAX,
        warm: u8::MAX,
        sdf_kernel: u8::MAX,
        reuse: u8::MAX,
        sdf_len: u32::MAX,
        tau_bits: u32::MAX,
        dt_bits: u32::MAX,
        sdf_edits: [[0; SDF_EDIT_WORDS]; MAX_SDF_EDITS],
    };

    /// The epoch of a step run in `mode`, with warm start `warm`, the configuration `cfg` and
    /// the SDF field `field` (`None` on a world without one).
    pub(crate) fn of(
        mode: SleepSkip,
        warm: bool,
        cfg: &PhysicsConfig,
        field: Option<&SdfField>,
    ) -> Self {
        let mut sdf_edits = [[0; SDF_EDIT_WORDS]; MAX_SDF_EDITS];
        let sdf_len = match field {
            Some(field) => {
                let edits = field.edits();
                for (bits, edit) in sdf_edits.iter_mut().zip(edits) {
                    *bits = edit_bits(edit);
                }
                edits.len() as u32
            }
            None => u32::MAX,
        };
        Self {
            mode: mode as u8,
            warm: u8::from(warm),
            sdf_kernel: match cfg.sdf_narrowphase {
                SdfNarrowphaseKernel::Scalar => 0,
                SdfNarrowphaseKernel::Avx2 => 1,
            },
            reuse: u8::from(cfg.contact_reuse),
            sdf_len,
            tau_bits: cfg.contact_reuse_distance.to_bits(),
            dt_bits: cfg.dt.to_bits(),
            sdf_edits,
        }
    }
}

/// One SDF edit as bits. The destructure is exhaustive, so a field added to [`SdfEdit`] fails to
/// compile here instead of escaping the epoch.
fn edit_bits(edit: &SdfEdit) -> [u32; SDF_EDIT_WORDS] {
    let SdfEdit { center, params, kind, op, smoothness, _pad } = *edit;
    [
        center[0].to_bits(),
        center[1].to_bits(),
        center[2].to_bits(),
        center[3].to_bits(),
        params[0].to_bits(),
        params[1].to_bits(),
        params[2].to_bits(),
        params[3].to_bits(),
        kind,
        op,
        smoothness.to_bits(),
        _pad,
    ]
}

/// Whether two gathered body states are bit-equal field by field (design 04 D2 R3). The
/// destructure is exhaustive, so a field added to [`BodyState`] fails to compile here instead of
/// escaping the resting test.
#[inline]
fn bit_equal(a: &BodyState, b: &BodyState) -> bool {
    let BodyState {
        inv_inertia,
        inv_inertia_local,
        position,
        linear_velocity,
        angular_velocity,
        rotation,
        inv_mass,
        restitution,
        friction,
        simulated,
        kinematic,
        is_sensor,
        shape,
    } = a;
    let v = |x: &crate::math::Vec3, y: &crate::math::Vec3| {
        x.x.to_bits() == y.x.to_bits() && x.y.to_bits() == y.y.to_bits() && x.z.to_bits() == y.z.to_bits()
    };
    let m = |x: &crate::math::Mat3, y: &crate::math::Mat3| {
        v(&x.rows[0], &y.rows[0]) && v(&x.rows[1], &y.rows[1]) && v(&x.rows[2], &y.rows[2])
    };
    let shapes = match (shape, &b.shape) {
        (ColliderShape::Sphere { radius: ra }, ColliderShape::Sphere { radius: rb }) => {
            ra.to_bits() == rb.to_bits()
        }
        (ColliderShape::Box { half_extents: ha }, ColliderShape::Box { half_extents: hb }) => {
            v(ha, hb)
        }
        _ => false,
    };
    v(position, &b.position)
        && v(linear_velocity, &b.linear_velocity)
        && v(angular_velocity, &b.angular_velocity)
        && rotation.x.to_bits() == b.rotation.x.to_bits()
        && rotation.y.to_bits() == b.rotation.y.to_bits()
        && rotation.z.to_bits() == b.rotation.z.to_bits()
        && rotation.w.to_bits() == b.rotation.w.to_bits()
        && inv_mass.to_bits() == b.inv_mass.to_bits()
        && restitution.to_bits() == b.restitution.to_bits()
        && friction.to_bits() == b.friction.to_bits()
        && *simulated == b.simulated
        && *kinematic == b.kinematic
        && *is_sensor == b.is_sensor
        && shapes
        && m(inv_inertia, &b.inv_inertia)
        && m(inv_inertia_local, &b.inv_inertia_local)
}

/// Whether a row's collider is a degenerate box (a zero, negative or NaN half-extent): its
/// contact can depend on the hint, so it never rests (design 04 D2).
#[inline]
fn degenerate(b: &BodyState) -> bool {
    match b.shape {
        ColliderShape::Box { half_extents: h } => !(h.x > 0.0 && h.y > 0.0 && h.z > 0.0),
        ColliderShape::Sphere { .. } => false,
    }
}

/// The sleep-skip's counts for the last step whose broadphase ran it (step mode
/// [`SleepSkip::Sets`], or a flush); all zero before the first such step. 48 B.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SleepSkipStats {
    /// Records (held islands) after the step's broadphase.
    pub held_islands: u32,
    /// Rows those records hold.
    pub held_rows: u32,
    /// Kept manifolds those records hold.
    pub held_manifolds: u32,
    /// Candidate pairs the tree broadphase withheld (both endpoints resting, one held).
    pub withheld_pairs: u32,
    /// Stream pairs the narrowphase skipped because an endpoint is held.
    pub held_skipped_pairs: u32,
    /// Stream pairs the narrowphase computed from a restored record's kept source.
    pub restored_pairs: u32,
    /// Stream pairs the narrowphase collided from the stream.
    pub computed: u32,
    /// Records restored this step.
    pub restored: u32,
    /// Records moved in this step.
    pub moved_in: u32,
    /// Rows the tree broadphase released from its sleeper set this step.
    pub released_rows: u32,
    /// Flushes this step (every record restored at once).
    pub flushes: u32,
    /// `1` when the solve took its no-awake fast path this step.
    pub fast_path: u32,
}

const _: () = assert!(
    size_of::<SleepSkipStats>() == 48,
    "SleepSkipStats is twelve u32 counters"
);

/// Why records restored, cumulative since construction: every D-rule and flush kind a gate must
/// observe (design 04 S3 anti-vacuity, "per-rule counters").
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SleepRuleCounts {
    /// D1: a member stopped resting, vanished or became a jumper.
    pub d1_member: u64,
    /// D1a: an anchor stopped resting, vanished or became a jumper.
    pub d1a_anchor: u64,
    /// D8: a member's latch was not asleep.
    pub d8_latch: u64,
    /// D2: a stream pair joined a member to a row that does not rest.
    pub d2_scan: u64,
    /// D3: an Identity step's axis table clear, for an island keeping a box-box manifold.
    pub d3_clear: u64,
    /// D5 flushes: the epoch changed.
    pub d5_epoch: u64,
    /// D6 flushes: `wake_all` was pending.
    pub d6_wake_all: u64,
    /// D7 flushes: a cursor did not classify, or the baseline was stale.
    pub d7_cursor: u64,
    /// D-H flushes: the mode left `Sets` with islands held.
    pub dh_mode: u64,
    /// Candidate islands refused by the D2 scan.
    pub refused_d2: u64,
    /// Candidate islands refused by M1 (not every member admissible).
    pub refused_m1: u64,
    /// Candidate islands refused by M2 (an anchor does not rest or vanished).
    pub refused_m2: u64,
    /// Candidate islands refused by M3 (a box pair's tag, design 08 B5′).
    pub refused_m3: u64,
    /// Kept pairs copied from another record's copy (Invariant K, design 06 B3).
    pub cross_copies: u64,
    /// Held-skip pairs whose copy a move-in looked up in a record restored on the same step, and
    /// so read tombstoned (design 06 B3; review W2 of C3c).
    pub cross_reads_dead: u64,
    /// `REC` pairs admitted on a settled miss (`HIT = 0 ∧ SETTLED = 1`, ruling W3's arm 9).
    pub rec_settled_misses: u64,
    /// `REC` pairs held without a manifold (design 06 anti-vacuity).
    pub rec_without_manifold: u64,
    /// Compactions of the store (A1.0b).
    pub compactions: u64,
    /// Axis keys mirrored by the narrowphase (design 06 D-C).
    pub mirrored: u64,
    /// Mirror passes on a grown or cleared table.
    pub mirror_clears: u64,
    /// Warm keys the solve searched in the restore source (design 06 A3).
    pub restore_rec_searches: u64,
    /// Of those, hits.
    pub restore_rec_hits: u64,
    /// Restored pairs whose join found their kept state (design 06 anti-vacuity).
    pub restore_idx_hits: u64,
}

/// What the colored solve reads and writes of the sleep-skip on its sleeping arm (design 04
/// A6, 06 A1–A3, 08 A1′): built by [`SleepSets::solve_inputs`] for one step.
pub(crate) struct HeldSolve<'a> {
    /// The step's per-row classification (`HELD` → the effective inverse mass), empty when
    /// the sleep-skip did not classify the step.
    pub(crate) cls: &'a [RowCls],
    /// The restore warm source, on a step that restored kept manifolds (P-a's second source).
    pub(crate) restore: Option<&'a WarmRecords>,
    /// The held store.
    pub(crate) held: HeldView<'a>,
    /// The kept slots moved in this step: the capture list.
    pub(crate) capture: core::ops::Range<usize>,
    /// The kept manifolds' warm records, which the capture writes.
    pub(crate) kept_rec: &'a mut ScratchColumn<WarmRecord>,
    /// The live kept slots' warm hit total, which the capture adds to.
    pub(crate) held_warm: &'a mut u32,
    /// Debug builds: `(slot << 32) | j` per captured slot, in capture order (design 08 OQ3).
    pub(crate) sources: &'a [u64],
    /// The step's counts (the solve writes `fast_path`).
    pub(crate) stats: &'a mut SleepSkipStats,
    /// The per-rule counts (the solve adds its restore-source searches and hits).
    pub(crate) rules: &'a mut SleepRuleCounts,
}

/// What the colored broadphase hands the sleep-skip's prologue (design 04 A1).
pub(crate) struct Prologue<'a> {
    /// The step record the broadphase just latched (L10 D9b): the configuration, the mode and the
    /// wake request count the prologue decides from, which every later stage reads too.
    pub(crate) inputs: &'a StepInputs,
    /// The gather's row identity.
    pub(crate) rows: &'a RowIdentity,
    /// This step's gathered bodies.
    pub(crate) bodies: &'a [BodyState],
    /// The previous step's post-solve snapshot, when the gather kept it on this gather.
    pub(crate) baseline: Option<&'a [BodyState]>,
    /// The sleep state as the previous step's solve left it.
    pub(crate) sleep: &'a IslandSleep,
    /// The previous step's graph.
    pub(crate) graph: &'a ConstraintGraph,
    /// The jumper bitset (one bit per current row), valid iff `jumpers_valid`.
    pub(crate) jumpers: &'a [u64],
    /// Whether the bitset was built on this gather.
    pub(crate) jumpers_valid: bool,
    /// How L9's pair carry will classify this step (its cursor, peeked).
    pub(crate) carry: RowRemap<'a>,
    /// The step's effective warm start — the colored solver's setup flag AND the latched
    /// `PhysicsConfig::warm_start` (L10 D5b) — or `None` in a world whose solve is not the
    /// colored one (the graph-only shape), where the sleep-skip never runs.
    pub(crate) warm: Option<bool>,
    /// The latched SDF field the sleep epoch covers: the record's, in a world with the SDF stage
    /// only (design 04 D10).
    pub(crate) field: Option<&'a SdfField>,
}

/// What the colored broadphase hands the sleep-skip's epilogue (design 04 A2).
pub(crate) struct Epilogue<'a> {
    /// This step's gathered bodies.
    pub(crate) bodies: &'a [BodyState],
    /// The gather's row identity.
    pub(crate) rows: &'a RowIdentity,
    /// The previous step's graph.
    pub(crate) graph: &'a ConstraintGraph,
    /// This step's stream (the kind arm's output).
    pub(crate) stream: &'a [(BodyIndex, BodyIndex)],
    /// The previous step's stream.
    pub(crate) pairs_prev: &'a [(BodyIndex, BodyIndex)],
    /// The tags and records the previous narrowphase wrote at `pairs_prev`'s slots, when they
    /// index it.
    pub(crate) written: Option<(&'a [PairTag], &'a [ReuseRecord])>,
    /// The previous step's solver manifolds (still in `Manifolds` at the broadphase).
    pub(crate) stream_prev: &'a [Manifold],
    /// Whether this step's hysteresis table will grow or clear at the logical pair count.
    pub(crate) would_clear: bool,
}

/// What the prologue decided for the epilogue.
#[derive(Clone, Copy, Debug)]
pub(crate) struct StepPlan {
    /// Whether the epilogue runs (the step mode is `Sets`).
    pub(crate) sets: bool,
    /// Whether some row is a [`RowCls::SLEEPER`] this step: only then does the tree read
    /// [`HeldHint`] (design 04 T1, T2), whose sleeper set is otherwise empty.
    pub(crate) sleepers: bool,
    /// Whether the step classifies as Identity against L10's cursor.
    identity: bool,
    /// Whether rows may rest this step (no flush).
    resting_ok: bool,
    /// Whether any row is `PRE_HELD` or `CAND` (the D2 scan runs).
    scan: bool,
    /// Whether the step is a Rows step (a translation and the jumper bits).
    rows_step: bool,
}

impl StepPlan {
    /// The plan of a step the sleep-skip does not run.
    const OFF: Self = Self {
        sets: false,
        sleepers: false,
        identity: false,
        resting_ok: false,
        scan: false,
        rows_step: false,
    };
}

/// `cand` flag: the island was refused this step.
const CAND_REFUSED: u32 = 1 << 31;
/// `cand` flag: the island passed M1 and the D2 scan (move-in pending).
const CAND_ADMIT: u32 = 1 << 30;
/// `cand` flag: some row of the island is a candidate this step.
const CAND_SEEN: u32 = 1 << 29;
/// `cand` count mask: the island's admissible members.
const CAND_COUNT: u32 = CAND_SEEN - 1;

/// The sleep-skip's state (L10), inserted on the colored path beside
/// [`IslandSleep`](crate::resources::IslandSleep). Every per-row datum is a kernel column
/// (principle 0); no field is shared across threads.
///
/// Written by the colored broadphase ([`physics_broadphase_colored`], which records the step
/// mode and runs the prologue and the epilogue), the colored narrowphase (its counts) and the
/// colored solve (the warm capture); every stage reads the classification the broadphase recorded
/// and the configuration it latched into the step record
/// ([`StepInputs`](crate::step_inputs::StepInputs), L10 D9b), never the live configuration, so a
/// configuration write between two stages cannot split a step (design 04 D9).
///
/// [`physics_broadphase_colored`]: crate::systems::physics_broadphase_colored
#[derive(Resource)]
pub struct SleepSets {
    /// This step's per-row classification, indexed by current row (`RowCls`); valid iff
    /// `cls_seq` is the current gather's and `np_sets`.
    row_cls: ScratchColumn<RowCls>,
    /// Previous row → current row, on a Rows step (A1.2).
    inv: ScratchColumn<u32>,
    /// Per island of the previous graph: its admissible member count and flags (A1.3, A2.4).
    cand: ScratchColumn<u32>,
    /// Per kept manifold slot (parallel to the store's `kept`): its warm record, captured by the
    /// solve at the move-in step (design 06 A1, A2).
    kept_rec: ScratchColumn<WarmRecord>,
    /// The warm records of the kept manifolds restored this step, keyed by their previous-step
    /// ordinal, strictly sorted (design 06 A3): the solve's second warm source.
    restore_rec: WarmRecords,
    /// The restore pair source's keys: the restored records' kept pairs in previous-step rows,
    /// `(min, max)`, strictly sorted (design 06 B4, the plan's E2).
    restore_keys: ScratchColumn<(BodyIndex, BodyIndex)>,
    /// Their tags, slot-parallel.
    restore_tags: ScratchColumn<PairTag>,
    /// Their reuse records, slot-parallel (valid where the tag carries `REC`).
    restore_reuse: ScratchColumn<ReuseRecord>,
    /// The sorts' scratch (`restore_sort`, design 08 A3′; the ordinal merge; compaction).
    restore_sort: ScratchColumn<RestoreSort>,
    /// General scratch: the restored records, the move-in's member and pair buckets, and (debug
    /// builds) the `(slot, j)` list of design 08 OQ3.
    scratch: ScratchColumn<u64>,
    /// L10's place in the gather sequence: stamped by every step whose broadphase ran the
    /// sleep-skip, so the held store and the baseline are keyed by that gather (design 04 D9).
    cursor: RemapCursor,
    /// The config inputs of the last step that ran the sleep-skip (design 08 D-E′).
    epoch: SleepEpoch,
    /// The mode the broadphase recorded for this step.
    step_mode: SleepSkip,
    /// Whether the narrowphase runs its `Sets` arm this step (design 08 D-H: the step mode is
    /// `Sets`, or a restore set is non-empty).
    np_sets: bool,
    /// The gather sequence the classification was written on.
    cls_seq: u64,
    /// The gather sequence `restore_rec` was built on (`0`: none).
    restore_rec_seq: u64,
    /// The gather sequence the restore pair source was built on (`0`: none).
    restore_pairs_seq: u64,
    /// The kept slots moved in this step, `[lo, hi)`, which the solve's capture fills.
    capture: (u32, u32),
    /// The gather sequence of `capture`.
    capture_seq: u64,
    /// Σ `kept_rec[s].count` over the live kept slots (the logical `carry_hits` share).
    held_warm: u32,
    /// Whether this step's restore warm records must be drained into the solver's read side
    /// (a D-H flush with the mode `Off`, ruling open question 2).
    drain: bool,
    /// The counts of the last step that ran the sleep-skip.
    stats: SleepSkipStats,
    /// The per-rule counts, cumulative.
    rules: SleepRuleCounts,
}

impl Default for SleepSets {
    /// Hand-written because the columns need their reserved `ComponentId`s.
    #[inline]
    fn default() -> Self {
        Self::with_capacity(0)
    }
}

impl SleepSets {
    /// An empty sleep-skip state, its per-row columns reserved for at least `rows` rows.
    pub fn with_capacity(rows: usize) -> Self {
        register_sleep_sets_layouts();
        let (rec_keys, rec_recs) = sleep_restore_rec_ids();
        let (pair_keys, pair_tags, pair_reuse) = sleep_restore_pair_ids();
        let u32_rows = rows.max(scratch_reserve_rows(size_of::<u32>()));
        Self {
            row_cls: ScratchColumn::new(
                sleep_row_cls_id(),
                rows.max(scratch_reserve_rows(size_of::<RowCls>())),
            ),
            inv: ScratchColumn::new(sleep_inv_id(), u32_rows),
            cand: ScratchColumn::new(sleep_cand_id(), u32_rows),
            kept_rec: ScratchColumn::new(
                sleep_kept_rec_id(),
                scratch_reserve_rows(size_of::<WarmRecord>()),
            ),
            restore_rec: WarmRecords::with_capacity(rec_keys, rec_recs, 0),
            restore_keys: ScratchColumn::new(
                pair_keys,
                scratch_reserve_rows(size_of::<(BodyIndex, BodyIndex)>()),
            ),
            restore_tags: ScratchColumn::new(pair_tags, scratch_reserve_rows(size_of::<PairTag>())),
            restore_reuse: ScratchColumn::new(
                pair_reuse,
                scratch_reserve_rows(size_of::<ReuseRecord>()),
            ),
            restore_sort: ScratchColumn::new(
                sleep_restore_sort_id(),
                scratch_reserve_rows(size_of::<RestoreSort>()),
            ),
            scratch: ScratchColumn::new(sleep_scratch_id(), scratch_reserve_rows(size_of::<u64>())),
            cursor: RemapCursor::default(),
            epoch: SleepEpoch::UNSET,
            step_mode: SleepSkip::Off,
            np_sets: false,
            cls_seq: 0,
            restore_rec_seq: 0,
            restore_pairs_seq: 0,
            capture: (0, 0),
            capture_seq: 0,
            held_warm: 0,
            drain: false,
            stats: SleepSkipStats::default(),
            rules: SleepRuleCounts::default(),
        }
    }

    /// The counts of the last step whose broadphase ran the sleep-skip.
    #[inline]
    pub fn stats(&self) -> SleepSkipStats {
        self.stats
    }

    /// The per-rule restore and refusal counts since construction (the gates' anti-vacuity).
    #[inline]
    pub fn rule_counts(&self) -> SleepRuleCounts {
        self.rules
    }

    /// The mode the last broadphase recorded: [`SleepSkip::Off`] when sleeping was off,
    /// otherwise `PhysicsConfig::sleep_skip` as it stood at the broadphase.
    #[inline]
    pub fn step_mode(&self) -> SleepSkip {
        self.step_mode
    }

    /// Whether row `row` is held this step: its pairs are skipped and its island is not solved.
    /// `false` for a row out of range, and on a step the sleep-skip did not classify.
    #[inline]
    pub fn is_row_held(&self, row: usize) -> bool {
        self.np_sets
            && self.row_cls.as_read_slice().get(row).is_some_and(|c| c.flags & RowCls::HELD != 0)
    }

    /// Whether the narrowphase runs its `Sets` arm this step.
    #[inline]
    pub(crate) fn np_sets(&self) -> bool {
        self.np_sets
    }

    /// This step's per-row classification, read by the narrowphase's `Sets` arm.
    #[inline]
    pub(crate) fn row_cls(&self) -> &[RowCls] {
        self.row_cls.as_read_slice()
    }

    /// This step's classification when `rows`' gather wrote it (the graph, the SDF stage and the
    /// solve read `HELD` through it), else `None`.
    #[inline]
    pub(crate) fn cls_for(&self, rows: &RowIdentity) -> Option<&[RowCls]> {
        (self.np_sets && self.cls_seq == rows.gather_seq()).then(|| self.row_cls.as_read_slice())
    }

    /// The restore pair source of `rows`' gather (keys, tags, reuse records), empty when this
    /// step restored nothing.
    #[inline]
    pub(crate) fn restore_pairs(
        &self,
        rows: &RowIdentity,
    ) -> (&[(BodyIndex, BodyIndex)], &[PairTag], &[ReuseRecord]) {
        if self.restore_pairs_seq == rows.gather_seq() {
            (
                self.restore_keys.as_read_slice(),
                self.restore_tags.as_read_slice(),
                self.restore_reuse.as_read_slice(),
            )
        } else {
            (&[], &[], &[])
        }
    }

    /// The restore warm source to drain into the solver's read side when this step's D-H flush
    /// ran with the mode `Off` (ruling open question 2: drained by L10's own system, never by
    /// the solve). The source is then spent: the solve does not search it.
    #[inline]
    pub(crate) fn take_drain(&mut self) -> Option<&WarmRecords> {
        if core::mem::take(&mut self.drain) {
            self.restore_rec_seq = 0;
            Some(&self.restore_rec)
        } else {
            None
        }
    }

    /// The kept manifolds' warm records, by kept slot (the gate's warm lookups read them).
    #[inline]
    pub(crate) fn kept_records(&self) -> &[WarmRecord] {
        self.kept_rec.as_read_slice()
    }

    /// Records the narrowphase's counts for the step (design 06 D-D D3): of its `stream` pairs,
    /// the ones it skipped, the ones it collided from the restore source (and how many of those
    /// found their kept state), and so the ones it collided from the stream.
    #[inline]
    pub(crate) fn note_np(&mut self, stream: usize, counts: NpCounts) {
        self.stats.held_skipped_pairs = counts.skips;
        self.stats.restored_pairs = counts.restored;
        self.stats.computed = (stream as u32).saturating_sub(counts.skips + counts.restored);
        self.rules.restore_idx_hits += u64::from(counts.restore_hits);
    }

    /// Records the narrowphase's mirror pass (design 06 D-C): keys re-keyed, and whether it
    /// covered every kept pair (a grown or cleared table).
    #[inline]
    pub(crate) fn note_mirror(&mut self, keys: u32, all: bool) {
        self.rules.mirrored += u64::from(keys);
        self.rules.mirror_clears += u64::from(all);
    }

    /// The solve's inputs for `rows`' gather (design 04 A6, 08 A1′): the classification when
    /// this gather's broadphase wrote it, the restore warm source when it built one and did not
    /// drain it, the kept slots it moved in (the capture list) with the column and the total
    /// the capture writes, and (debug builds) the previous-stream index each slot was copied
    /// from, in capture order (design 08 OQ3).
    pub(crate) fn solve_inputs<'a>(
        &'a mut self,
        rows: &RowIdentity,
        held: HeldView<'a>,
    ) -> HeldSolve<'a> {
        let seq = rows.gather_seq();
        let capture = if self.capture_seq == seq {
            self.capture.0 as usize..self.capture.1 as usize
        } else {
            0..0
        };
        let sources: &[u64] = if cfg!(debug_assertions) && !capture.is_empty() {
            self.scratch.as_read_slice()
        } else {
            &[]
        };
        let cls: &[RowCls] = if self.np_sets && self.cls_seq == seq {
            self.row_cls.as_read_slice()
        } else {
            &[]
        };
        let restore = (self.restore_rec_seq == seq && !self.restore_rec.keys().is_empty())
            .then_some(&self.restore_rec);
        HeldSolve {
            cls,
            restore,
            held,
            capture,
            kept_rec: &mut self.kept_rec,
            held_warm: &mut self.held_warm,
            sources,
            stats: &mut self.stats,
            rules: &mut self.rules,
        }
    }

    /// The broadphase prologue (design 04 A1; module docs): the mode, the compaction, the
    /// cursors, the translation, the classification, the per-record rules and the D-H flush.
    /// Returns what the epilogue needs.
    pub(crate) fn prologue(&mut self, p: &Prologue<'_>, held: &mut HeldStore) -> StepPlan {
        let cfg = p.inputs.config();
        let mode = match p.warm {
            Some(_) if cfg.sleeping => cfg.sleep_skip,
            _ => SleepSkip::Off,
        };
        self.step_mode = mode;
        self.np_sets = false;
        self.drain = false;
        if mode == SleepSkip::Off && held.live_records() == 0 {
            // No held island to flush: the step runs as the sleep-skip off, unstamped, so a
            // later switch into `Sets` finds a stale cursor and takes one Reset step (D9).
            return StepPlan::OFF;
        }
        let _zone = boyko_diag::zone!(PHYS_SLEEP_CLASSIFY);
        self.stats = SleepSkipStats::default();
        let n = p.bodies.len();
        let seq = p.rows.gather_seq();

        // A1.0b: compaction of records tombstoned on EARLIER steps (design 06 B3).
        if held.compaction_due() {
            held.compact(&mut self.kept_rec, &mut self.restore_sort);
            self.rules.compactions += 1;
        }

        // A1.1: the cursors. L10's own classifies the step; the mask, the latch and L9's carry
        // must classify alike, and the baseline must be this gather's (R4, D7).
        let remap = self.cursor.remap(p.rows);
        let c = class_of(&remap);
        let rows_step = c == 1;
        let cursors_ok = c != 2
            && class_of(&p.sleep.mask_peek(p.rows)) == c
            && class_of(&p.sleep.latch_peek(p.rows)) == c
            && class_of(&p.carry) == c
            && p.baseline.is_some()
            && (!rows_step || p.jumpers_valid);

        // D5: the epoch; D6: a pending global wake — a `Reset` wake, or a request the step
        // record latched that no solve has served (D9b Decision 5).
        let epoch = SleepEpoch::of(mode, p.warm.unwrap_or(false), cfg, p.field);
        let epoch_changed = epoch != self.epoch;
        self.epoch = epoch;
        let wake_all = p.sleep.wake_pending(p.inputs.wake_requests());
        let flush = mode != SleepSkip::Sets || !cursors_ok || epoch_changed || wake_all;
        if held.live_records() > 0 && flush {
            self.stats.flushes = 1;
            if mode != SleepSkip::Sets {
                self.rules.dh_mode += 1;
            } else if !cursors_ok {
                self.rules.d7_cursor += 1;
            } else if epoch_changed {
                self.rules.d5_epoch += 1;
            } else {
                self.rules.d6_wake_all += 1;
            }
        }

        // A1.2: a Rows step translates every record's row table (and `of_row`); a Reset cannot,
        // so every record is flushed with no row-keyed effect (its members are unknown here).
        match remap {
            RowRemap::Rows(prev_row) => {
                self.build_inv(prev_row, p.rows.prev_rows_len());
                held.translate(self.inv.as_read_slice(), n);
            }
            RowRemap::Identity => {}
            RowRemap::Reset => {
                held.mark_all();
                self.restore_marked(held, None, n, seq);
                held.rebuild_of_row(n);
            }
        }

        // A1.3: every row, every step (ruling W4).
        let resting_ok = !flush;
        let scan = self.classify(p, held.view(), remap, resting_ok, rows_step);

        // A1.4: the per-record rules, or the flush.
        if flush {
            held.mark_all();
        } else {
            self.record_rules(held);
        }

        if mode != SleepSkip::Sets {
            // D-H (design 08): every record restored through A2.5 with its sources, the rows
            // classified RESTORED / SENSOR only; with the mode `Off` the broadphase drains the
            // warm records into the solver.
            self.flush_classify(n);
            let restored = self.restore_marked(held, Some(remap), n, seq);
            self.np_sets = restored > 0;
            self.drain = mode == SleepSkip::Off && restored > 0;
            self.cls_seq = seq;
            self.finish(held, p.rows);
            return StepPlan::OFF;
        }
        // Design 04 T1 (ruling W1 of rev 2.1): the tree's hint is `PRE_HELD` less the records the
        // prologue just marked for restore.
        let sleepers = self.mark_sleepers(held.view());
        StepPlan { sets: true, sleepers, identity: c == 0, resting_ok, scan, rows_step }
    }

    /// Design 04 T1 (ruling W1 of rev 2.1): [`RowCls::SLEEPER`] on every member of a live record
    /// the prologue did not mark for restore — the rows the tree broadphase may hold in its
    /// sleeper set this step. Such a member rests and is no sensor (D1, M1), so the hint's
    /// `anchor_ok` holds for it too. Returns whether any row was marked.
    fn mark_sleepers(&mut self, held: HeldView<'_>) -> bool {
        let mut cls = self.row_cls.build_view();
        let cls = cls.as_mut_slice();
        let mut any = false;
        for rec in held.records().iter().filter(|r| r.live() && r.flags & HeldIsland::RESTORE == 0) {
            for &r in held.rows(rec.members_range()) {
                let c = &mut cls[r as usize];
                debug_assert!(
                    c.flags & RowCls::RESTING != 0 && !sensor_pair(c.flags, 0),
                    "invariant: a member of a surviving record rests and is no sensor (D1, M1)"
                );
                c.flags |= RowCls::SLEEPER;
                any = true;
            }
        }
        any
    }

    /// Records the tree seam's counts for the step (design 04 T3, T4): the pairs withheld after
    /// the release, and the rows released.
    #[inline]
    pub(crate) fn note_tree(&mut self, withheld: usize, released: u32) {
        self.stats.withheld_pairs = withheld as u32;
        self.stats.released_rows = released;
    }

    /// Debug-only: Invariant V (design 04 D11; ruling W1 of rev 2.1) over the withheld pairs —
    /// both endpoints rest and are no sensor, and one is a sleeper (`after_epilogue == false`,
    /// right after the kind arm) or held (`true`, after the epilogue and the release).
    ///
    /// Also: no endpoint is a candidate. A withheld pair's endpoints are static-set members
    /// (statics have no island, so never `CAND`) or sleepers (`SLEEPER` ⊂ `PRE_HELD`, and `CAND`
    /// is written only on a row with no record). So the D2 scan's cross-pair listing (design 06
    /// B3: a pair of a moving-in member and a held one) finds every such pair in the stream, and
    /// never has to walk the withheld list.
    pub(crate) fn debug_withheld(&self, withheld: &[(BodyIndex, BodyIndex)], after_epilogue: bool) {
        if cfg!(debug_assertions) {
            let cls = self.row_cls.as_read_slice();
            let one = if after_epilogue { RowCls::HELD } else { RowCls::SLEEPER };
            for &(a, b) in withheld {
                let (fa, fb) = (cls[a.0 as usize].flags, cls[b.0 as usize].flags);
                assert!(
                    fa & fb & RowCls::RESTING != 0
                        && !sensor_pair(fa, fb)
                        && (fa | fb) & one != 0
                        && (fa | fb) & RowCls::CAND == 0,
                    "invariant V: withheld pair ({}, {}) has flags {fa:#x}, {fb:#x} ({})",
                    a.0,
                    b.0,
                    if after_epilogue { "after the epilogue" } else { "after the kind arm" }
                );
            }
        }
    }

    /// The broadphase epilogue on a `Sets` step (design 04 A2, 06 B3, 08 A3′): D3, the D2 scan,
    /// the restores, the move-ins, `HELD`, the stamp.
    pub(crate) fn epilogue(&mut self, e: &Epilogue<'_>, plan: StepPlan, held: &mut HeldStore) {
        if !plan.sets {
            return;
        }
        let n = e.bodies.len();
        let seq = e.rows.gather_seq();
        // A2.1 (D3): an Identity step whose axis table clears or grows restores every island that
        // keeps a box-box manifold — its hints are gone under `Off` too.
        if plan.identity && e.would_clear {
            let view = held.view();
            let n_records = view.records().len();
            for k in 0..n_records {
                let rec = held.view().records()[k];
                if rec.live() && rec.box_kept > 0 && rec.flags & HeldIsland::RESTORE == 0 {
                    held.mark_restore(k);
                    self.rules.d3_clear += 1;
                }
            }
        }
        // A2.2: the D2 scan, when any row is held or a candidate.
        if plan.scan {
            self.d2_scan(e.stream, held);
        }
        // A2.5a: the restores, with their sources.
        let remap = self.cursor.peek(e.rows);
        self.restore_marked(held, Some(remap), n, seq);
        // A2.4: the move-ins.
        if plan.resting_ok {
            self.move_in(e, held, remap);
        }
        self.set_held(held);
        self.np_sets = true;
        self.cls_seq = seq;
        self.finish(held, e.rows);
        let _ = plan.rows_step;
    }

    /// `inv`: previous row → current row, [`NONE`] for a vanished body.
    fn build_inv(&mut self, prev_row: &[u32], prev_len: usize) {
        let mut inv = self.inv.build_view();
        inv.clear();
        inv.resize(prev_len, NONE);
        let inv = inv.as_mut_slice();
        for (r, &q) in prev_row.iter().enumerate() {
            if q != NO_ROW
                && let Some(slot) = inv.get_mut(q as usize)
            {
                *slot = r as u32;
            }
        }
    }

    /// A1.3: every row's flags (module docs), and each previous island's admissible member
    /// count. Returns whether any row is `PRE_HELD` or `CAND`.
    fn classify(
        &mut self,
        p: &Prologue<'_>,
        held: HeldView<'_>,
        remap: RowRemap<'_>,
        resting_ok: bool,
        rows_step: bool,
    ) -> bool {
        let n = p.bodies.len();
        let latches = p.sleep.latches();
        let baseline = p.baseline.unwrap_or(&[]);
        let mut cand_view = self.cand.build_view();
        cand_view.clear();
        cand_view.resize(p.graph.n_islands() as usize, 0);
        let cand = cand_view.as_mut_slice();
        let mut cls_view = self.row_cls.build_view();
        cls_view.clear();
        cls_view.resize(n, RowCls::default());
        let cls = cls_view.as_mut_slice();
        let mut any = false;
        for (r, (b, out)) in p.bodies.iter().zip(cls.iter_mut()).enumerate() {
            let mut flags = if b.is_sensor { RowCls::SENSOR } else { 0 };
            let record = held.record_of_row(r as u32);
            let mut island = RowCls::NO_ISLAND;
            if record != NONE {
                flags |= RowCls::PRE_HELD;
                island = record;
                any = true;
            }
            let prev = match remap {
                _ if !resting_ok => None,
                RowRemap::Identity => Some(r as u32),
                RowRemap::Rows(prev_row) => Some(prev_row[r]).filter(|&q| q != NO_ROW),
                RowRemap::Reset => None,
            };
            if let Some(q) = prev {
                let qi = q as usize;
                if latches.get(qi).is_some_and(|l| l.asleep) {
                    flags |= RowCls::LATCHED;
                }
                let jumper = rows_step && (p.jumpers[r >> 6] >> (r & 63)) & 1 != 0;
                // R2 first (an inverse-mass or mask test); R3 only when R2 holds.
                if !jumper
                    && !degenerate(b)
                    && baseline.get(qi).is_some_and(|base| {
                        (base.inv_mass == 0.0 || !p.sleep.is_row_awake(qi)) && bit_equal(b, base)
                    })
                {
                    flags |= RowCls::RESTING;
                }
                if record == NONE {
                    let i = p.graph.island_of(q);
                    if i != ConstraintGraph::NO_ISLAND && p.sleep.is_island_frozen(i) {
                        flags |= RowCls::CAND;
                        island = i;
                        any = true;
                        let c = &mut cand[i as usize];
                        *c |= CAND_SEEN;
                        let ok = RowCls::RESTING | RowCls::LATCHED;
                        if flags & ok == ok && !sensor_pair(flags, 0) {
                            *c += 1;
                        }
                    }
                }
            }
            *out = RowCls { island, flags };
        }
        any
    }

    /// A1.4: D1, D1a and D8 per live record: a member that does not rest (it moved, vanished,
    /// became a jumper) or whose latch is not asleep, or an anchor that does not rest, restores
    /// the record.
    fn record_rules(&mut self, held: &mut HeldStore) {
        let n_records = held.view().records().len();
        for k in 0..n_records {
            let rule = {
                let view = held.view();
                let rec = view.records()[k];
                if !rec.live() {
                    continue;
                }
                let cls = self.row_cls.as_read_slice();
                let flags = |r: u32| cls.get(r as usize).map_or(0, |c| c.flags);
                let members = view.rows(rec.members_range());
                if members.iter().any(|&r| flags(r) & RowCls::RESTING == 0) {
                    Some(&mut self.rules.d1_member)
                } else if members.iter().any(|&r| flags(r) & RowCls::LATCHED == 0) {
                    Some(&mut self.rules.d8_latch)
                } else if view.rows(rec.anchors_range()).iter().any(|&r| flags(r) & RowCls::RESTING == 0) {
                    Some(&mut self.rules.d1a_anchor)
                } else {
                    None
                }
            };
            if let Some(count) = rule {
                *count += 1;
                held.mark_restore(k);
            }
        }
    }

    /// A2.2, the D2 scan (design 04; 08 D-F, D-G): per stream pair with a held or candidate
    /// endpoint — a sensor pair marks that endpoint `SENSOR_NBR`; otherwise an endpoint whose
    /// partner does not rest restores its record (held) or refuses its island (candidate).
    fn d2_scan(&mut self, stream: &[(BodyIndex, BodyIndex)], held: &mut HeldStore) {
        let hc = RowCls::PRE_HELD | RowCls::CAND;
        let mut cls_view = self.row_cls.build_view();
        let cls = cls_view.as_mut_slice();
        let mut cand_view = self.cand.build_view();
        let cand = cand_view.as_mut_slice();
        for &(a, b) in stream {
            let (ia, ib) = (a.0 as usize, b.0 as usize);
            let (fa, fb) = (cls[ia].flags, cls[ib].flags);
            if (fa | fb) & hc == 0 {
                continue;
            }
            if sensor_pair(fa, fb) {
                if fa & hc != 0 {
                    cls[ia].flags |= RowCls::SENSOR_NBR;
                }
                if fb & hc != 0 {
                    cls[ib].flags |= RowCls::SENSOR_NBR;
                }
                continue;
            }
            for (x, fx, fo) in [(ia, fa, fb), (ib, fb, fa)] {
                if fx & hc == 0 || fo & RowCls::RESTING != 0 {
                    continue;
                }
                let island = cls[x].island as usize;
                if fx & RowCls::PRE_HELD != 0 {
                    if held.view().records()[island].flags & HeldIsland::RESTORE == 0 {
                        self.rules.d2_scan += 1;
                    }
                    held.mark_restore(island);
                } else if cand[island] & CAND_REFUSED == 0 {
                    cand[island] |= CAND_REFUSED;
                    self.rules.refused_d2 += 1;
                }
            }
        }
    }

    /// The D-H flush's classification (design 08 D-H 3): `SENSOR` from `is_sensor`, every other
    /// flag clear; the restore sets `RESTORED` on the flushed members.
    fn flush_classify(&mut self, n: usize) {
        let mut cls = self.row_cls.build_view();
        let n = n.min(cls.len());
        for c in &mut cls.as_mut_slice()[..n] {
            let flags = if sensor_pair(c.flags, 0) { RowCls::SENSOR } else { 0 };
            *c = RowCls { island: RowCls::NO_ISLAND, flags };
        }
    }

    /// Restores every record marked for restore (A2.5a): `RESTORED` on its present members, its
    /// kept warm hits leave `held_warm`, it is tombstoned, the `order` is filtered, and — when
    /// `remap` names the previous rows — the two restore sources are built (design 06 A3, B4,
    /// 08 A3′), stamped with `seq`. Returns how many records it restored.
    fn restore_marked(
        &mut self,
        held: &mut HeldStore,
        remap: Option<RowRemap<'_>>,
        n: usize,
        seq: u64,
    ) -> u32 {
        let mut restored = 0u32;
        {
            let mut list = self.scratch.build_view();
            list.clear();
            let mut cls = self.row_cls.build_view();
            if cls.len() < n {
                cls.resize(n, RowCls::default());
            }
            let cls = cls.as_mut_slice();
            let kept_rec = self.kept_rec.as_read_slice();
            let n_records = held.view().records().len();
            for k in 0..n_records {
                let rec = held.view().records()[k];
                if !rec.live() || rec.flags & HeldIsland::RESTORE == 0 {
                    continue;
                }
                if remap.is_some() {
                    for &r in held.view().rows(rec.members_range()) {
                        if let Some(c) = cls.get_mut(r as usize) {
                            c.flags |= RowCls::RESTORED;
                        }
                    }
                }
                for s in rec.kept_range() {
                    self.held_warm -= u32::from(kept_rec.get(s).map_or(0, WarmRecord::count));
                }
                held.tombstone(k);
                list.push(k as u64);
                restored += 1;
            }
        }
        if restored == 0 {
            return 0;
        }
        self.stats.restored += restored;
        match remap {
            Some(remap) => self.build_restore_sources(held.view(), remap, seq),
            None => {
                self.restore_rec_seq = 0;
                self.restore_pairs_seq = 0;
            }
        }
        held.filter_order();
        restored
    }

    /// The two restore sources of the records listed in `scratch` (restored this step):
    /// `restore_rec` by previous-step warm key, and the pair source by previous-step pair key
    /// (design 06 A3, B4; 08 A3′). An entry naming a vanished row is skipped (OQ1); a Reset
    /// writes both at length 0 (design 08 O-2, the debug assert below).
    fn build_restore_sources(&mut self, held: HeldView<'_>, remap: RowRemap<'_>, seq: u64) {
        let prev = |r: u32| -> Option<u32> { if r == NONE { None } else { remap.row(r) } };
        let reset = matches!(remap, RowRemap::Reset);
        // The warm source: every restored kept manifold's record, by its previous ordinal.
        {
            let list = self.scratch.as_read_slice();
            let mut sort = self.restore_sort.build_view();
            sort.clear();
            if !reset {
                for &k in list {
                    let rec = held.records()[k as usize];
                    for slot in rec.kept_range() {
                        let Some(m) = held.manifold_present(slot as u32) else { continue };
                        let Some(pa) = prev(m.body_a.0) else { continue };
                        let key = if m.body_b == SDF_SENTINEL {
                            ord(pa, SDF_SENTINEL.0)
                        } else {
                            let Some(pb) = prev(m.body_b.0) else { continue };
                            ord(pa.min(pb), pa.max(pb))
                        };
                        sort.push(RestoreSort::new(key, slot as u32));
                    }
                }
            }
            sort.as_mut_slice().sort_unstable();
        }
        {
            let sorted = self.restore_sort.as_read_slice();
            let kept_rec = self.kept_rec.as_read_slice();
            debug_assert!(
                sorted.windows(2).all(|w| w[0].key < w[1].key),
                "invariant: a kept manifold's key is unique (it unions its dynamic endpoints)"
            );
            self.restore_rec.resize(sorted.len());
            {
                let mut keys = self.restore_rec.keys_mut();
                for (k, e) in keys.as_mut_slice().iter_mut().zip(sorted) {
                    *k = e.key;
                }
            }
            {
                let mut recs = self.restore_rec.recs_mut();
                for (r, e) in recs.as_mut_slice().iter_mut().zip(sorted) {
                    *r = kept_rec.get(e.slot as usize).copied().unwrap_or(WarmRecord::EMPTY);
                }
            }
            self.restore_rec.set_strict(true);
        }
        // The pair source: every restored kept pair, by its previous rows.
        {
            let list = self.scratch.as_read_slice();
            let mut sort = self.restore_sort.build_view();
            sort.clear();
            if !reset {
                for &k in list {
                    let rec = held.records()[k as usize];
                    let rows = held.rows(rec.rows_range());
                    for (q, kp) in held.pairs(rec.pairs_range()).iter().enumerate() {
                        let (Some(pa), Some(pb)) =
                            (prev(rows[kp.la as usize]), prev(rows[kp.lb as usize]))
                        else {
                            continue;
                        };
                        debug_assert!(pa < pb, "invariant: a held pair keeps its previous-step row order");
                        let key = (u64::from(pa) << 32) | u64::from(pb);
                        let mut e = RestoreSort::new(key, rec.pairs_start + q as u32);
                        e.aux = k as u32;
                        sort.push(e);
                    }
                }
            }
            sort.as_mut_slice().sort_unstable();
        }
        {
            let sorted = self.restore_sort.as_read_slice();
            let all_pairs = held.all_pairs();
            let mut keys = self.restore_keys.build_view();
            let mut tags = self.restore_tags.build_view();
            let mut reuse = self.restore_reuse.build_view();
            keys.clear();
            tags.clear();
            reuse.clear();
            for (i, e) in sorted.iter().enumerate() {
                let kp = &all_pairs[e.slot as usize];
                if i > 0 && sorted[i - 1].key == e.key {
                    // Invariant K: both records keep the pair, and the copies are equal.
                    debug_assert_eq!(
                        all_pairs[sorted[i - 1].slot as usize].tag.bits() & PairTag::HIT_INVARIANT,
                        kp.tag.bits() & PairTag::HIT_INVARIANT,
                        "invariant: two records' copies of one pair are equal (Invariant K)"
                    );
                    continue;
                }
                let rec = &held.records()[e.aux as usize];
                keys.push((BodyIndex((e.key >> 32) as u32), BodyIndex(e.key as u32)));
                tags.push(kp.tag);
                reuse.push(held.reuse_of(rec, kp).copied().unwrap_or(ReuseRecord::UNWRITTEN));
            }
        }
        debug_assert!(
            !reset || (self.restore_rec.keys().is_empty() && self.restore_keys.is_empty()),
            "invariant: a Reset flush writes both restore sources at length 0 (design 08 O-2)"
        );
        self.restore_rec_seq = seq;
        self.restore_pairs_seq = seq;
    }

    /// A2.4, the move-ins (design 04; 06 B2, B3, B5; 08 B5′): every candidate island that passed
    /// M1 and the D2 scan is captured — its members, its manifolds from the previous stream, its
    /// kept pairs from the previous pair list (a cross pair from the held record's copy) — and
    /// committed unless M2 (every anchor rests) or M3 (every box pair's tag admits) refuses it;
    /// then the new slots merge into `order`.
    fn move_in(&mut self, e: &Epilogue<'_>, held: &mut HeldStore, remap: RowRemap<'_>) {
        let info = e.graph.island_info();
        let n_islands = e.graph.n_islands() as usize;
        let Some((tags, recs)) = e.written else { return };
        if info.len() != n_islands {
            return;
        }
        // M1: every member of the previous island is present and admissible.
        let mut any = false;
        {
            let mut cand = self.cand.build_view();
            for (c, i) in cand.as_mut_slice().iter_mut().zip(info) {
                if *c & CAND_SEEN == 0 || *c & CAND_REFUSED != 0 {
                    continue;
                }
                if *c & CAND_COUNT == i.members {
                    *c |= CAND_ADMIT;
                    any = true;
                } else {
                    self.rules.refused_m1 += 1;
                }
            }
        }
        if !any {
            return;
        }
        let n = e.bodies.len();
        let inv = self.inv.as_read_slice();
        let to_cur = |q: u32| -> u32 {
            match remap {
                RowRemap::Identity => q,
                RowRemap::Rows(_) => inv.get(q as usize).copied().unwrap_or(NONE),
                RowRemap::Reset => NONE,
            }
        };
        let cand = self.cand.as_read_slice();
        let cls = self.row_cls.as_read_slice();
        // The buckets: members `(island << 32) | row`, then pairs `(island << 32) | slot`.
        let n_members_total = {
            let mut scratch = self.scratch.build_view();
            scratch.clear();
            for (r, c) in cls.iter().enumerate() {
                if c.flags & RowCls::CAND != 0 && cand[c.island as usize] & CAND_ADMIT != 0 {
                    scratch.push((u64::from(c.island) << 32) | r as u64);
                }
            }
            let nm = scratch.len();
            for (j, &(pa, pb)) in e.pairs_prev.iter().enumerate() {
                let ia = e.graph.island_of(pa.0);
                let ib = e.graph.island_of(pb.0);
                let admitted = |i: u32| i != ConstraintGraph::NO_ISLAND && cand[i as usize] & CAND_ADMIT != 0;
                if admitted(ia) {
                    scratch.push((u64::from(ia) << 32) | j as u64);
                }
                if ib != ia && admitted(ib) {
                    scratch.push((u64::from(ib) << 32) | j as u64);
                }
            }
            let s = scratch.as_mut_slice();
            s[..nm].sort_unstable();
            s[nm..].sort_unstable();
            nm
        };
        let first_slot = held.kept_len();
        let first_record = held.view().records().len();
        let buckets = self.scratch.as_read_slice();
        let (members_all, pairs_all) = buckets.split_at(n_members_total);
        let (mut mi, mut pi) = (0usize, 0usize);
        let mut moved = 0u32;
        while mi < members_all.len() {
            let island = (members_all[mi] >> 32) as u32;
            let m_end = mi + members_all[mi..].iter().take_while(|&&x| (x >> 32) as u32 == island).count();
            while pi < pairs_all.len() && ((pairs_all[pi] >> 32) as u32) < island {
                pi += 1;
            }
            let p_end = pi + pairs_all[pi..].iter().take_while(|&&x| (x >> 32) as u32 == island).count();
            let members = &members_all[mi..m_end];
            let pairs = &pairs_all[pi..p_end];
            mi = m_end;
            pi = p_end;
            let ok = capture_island(
                held,
                &mut self.rules,
                e,
                cls,
                (tags, recs),
                island,
                members,
                pairs,
                &to_cur,
                n,
            );
            moved += u32::from(ok);
        }
        self.stats.moved_in = moved;
        if moved == 0 {
            return;
        }
        // The new kept slots' warm records, captured by this step's solve (design 06 A1).
        let kept_len = held.kept_len();
        self.kept_rec.build_view().resize(kept_len, WarmRecord::EMPTY);
        self.capture = (first_slot as u32, kept_len as u32);
        self.capture_seq = e.rows.gather_seq();
        // The `order` merge.
        {
            let view = held.view();
            let mut sort = self.restore_sort.build_view();
            sort.clear();
            for s in first_slot..kept_len {
                sort.push(RestoreSort::new(view.ordinal(s as u32), s as u32));
            }
            sort.as_mut_slice().sort_unstable();
        }
        held.merge_order(self.restore_sort.as_read_slice());
        // Design 08 OQ3: in debug builds, each captured slot's previous-stream index, in capture
        // order, for the solve's `run ∋ j` assertion.
        if cfg!(debug_assertions) {
            let view = held.view();
            let mut list = self.scratch.build_view();
            list.clear();
            for k in first_record..view.records().len() {
                let rec = view.records()[k];
                let root_prev = match remap {
                    RowRemap::Identity => Some(rec.root),
                    RowRemap::Rows(prev_row) => Some(prev_row[rec.root as usize]),
                    RowRemap::Reset => None,
                };
                let Some(root_prev) = root_prev else { continue };
                let island = e.graph.island_of(root_prev);
                for (slot, j) in rec.kept_range().zip(e.graph.island(island).iter()) {
                    list.push(((slot as u64) << 32) | u64::from(j));
                }
            }
        }
    }

    /// `HELD` on every live record's members (they are held this step), each row's island the
    /// record's index.
    fn set_held(&mut self, held: &HeldStore) {
        let view = held.view();
        let mut cls = self.row_cls.build_view();
        let cls = cls.as_mut_slice();
        for (k, rec) in view.records().iter().enumerate() {
            if !rec.live() {
                continue;
            }
            for &r in view.rows(rec.members_range()) {
                let c = &mut cls[r as usize];
                c.flags |= RowCls::HELD;
                c.island = k as u32;
            }
        }
    }

    /// The step's stamp, stats and held-row counter.
    fn finish(&mut self, held: &HeldStore, rows: &RowIdentity) {
        let view = held.view();
        let held_rows: u32 = view.records().iter().filter(|r| r.live()).map(|r| r.n_members).sum();
        self.stats.held_islands = held.live_records();
        self.stats.held_rows = held_rows;
        self.stats.held_manifolds = view.live_kept();
        self.cursor.stamp(rows);
        counter!(PHYS_SLEEP_HELD, u64::from(held_rows));
    }
}

/// L10's axis mirror (design 06 D-C, ruling W3 of rev 2): run by the narrowphase AFTER
/// `begin_frame_synced` — whose grow or clear would erase what it sets — and before either path,
/// serially. Every held pair whose kept tag re-keys ([`PairTag::rekeys`], the one function L9's
/// commit uses) is `set` with the axis its tag carries: all of them when the table grew or
/// cleared, only the pairs whose rows moved on a plain Rows step (`prev_row`), none otherwise.
/// Those are exactly the `set`s `Off`'s computes of the held pairs make that change the table
/// (on any other step `Off` re-sets a key it already holds, an identity), so every lookup and
/// `occupied` equal `Off`'s (linear probing without deletion: a probe's result does not depend
/// on the insertion order of other keys). Returns the keys set and whether every held pair was
/// covered.
pub(crate) fn mirror_held(
    cache: &mut BoxAxisCache,
    held: HeldView<'_>,
    keys: KeyChange,
    prev_row: Option<&[u32]>,
) -> (u32, bool) {
    if held.is_empty() {
        return (0, false);
    }
    let all = keys.cleared || keys.grown;
    if !all && !keys.prefetched {
        return (0, false);
    }
    let moved = |r: u32| prev_row.is_some_and(|p| p[r as usize] != r);
    let mut n = 0u32;
    for rec in held.records().iter().filter(|r| r.live()) {
        let rows = held.rows(rec.rows_range());
        for kp in held.pairs(rec.pairs_range()) {
            debug_assert!(
                !kp.tag.is_held_skip(),
                "invariant: a kept tag is never the held-skip tag (rule 5)"
            );
            if !kp.tag.rekeys() {
                continue;
            }
            let (a, b) = (rows[kp.la as usize], rows[kp.lb as usize]);
            if !all && !(moved(a) || moved(b)) {
                continue;
            }
            let axis = kp.tag.axis().expect("invariant: a re-keying tag carries its axis");
            cache.set(BodyIndex(a), BodyIndex(b), usize::from(axis));
            n += 1;
        }
    }
    (n, all)
}

/// A cursor classification as a small integer: Identity 0, Rows 1, Reset 2.
#[inline]
fn class_of(r: &RowRemap<'_>) -> u8 {
    match r {
        RowRemap::Identity => 0,
        RowRemap::Rows(_) => 1,
        RowRemap::Reset => 2,
    }
}

/// Whether `record` keeps a pair on current rows `{a, b}`, found by translating every kept pair's
/// two row-table indices: the debug oracle of `capture_island`'s cross-copy lookup, which goes
/// through `local_of` and `find_pair` and shares neither (review W2 of C3c). Linear in the
/// record's kept pairs.
fn keeps_pair_on(view: HeldView<'_>, record: &HeldIsland, a: u32, b: u32) -> bool {
    let table = view.rows(record.rows_range());
    view.pairs(record.pairs_range()).iter().any(|kp| {
        let (x, y) = (table[kp.la as usize], table[kp.lb as usize]);
        (x, y) == (a, b) || (x, y) == (b, a)
    })
}

/// Captures one candidate island into the store (A2.4, one island): the members, the kept
/// manifolds (the previous stream's, through the previous graph's island), the kept pairs (the
/// previous pair list's bucket, a cross pair from the held record's copy), the anchors, M2 and
/// M3. Commits the record and returns `true`, or rolls it back and returns `false`.
#[allow(clippy::too_many_arguments)]
fn capture_island(
    held: &mut HeldStore,
    rules: &mut SleepRuleCounts,
    e: &Epilogue<'_>,
    cls: &[RowCls],
    (tags, recs): (&[PairTag], &[ReuseRecord]),
    island: u32,
    members: &[u64],
    pairs: &[u64],
    to_cur: &impl Fn(u32) -> u32,
    n: usize,
) -> bool {
    let mark = held.mark();
    for &m in members {
        held.push_row(m as u32);
    }
    let nm = members.len();
    let is_member = |held: &HeldStore, r: u32| held.rows_since(mark)[..nm].binary_search(&r).is_ok();
    let mut ok = true;
    let mut box_kept = 0u32;
    let mut points = 0u32;
    // The kept manifolds: the previous graph's island, in stream order (so by ordinal).
    for h in e.graph.island(island).iter() {
        debug_assert!(h < HELD_BASE, "invariant: a candidate island was not held at the previous step");
        let m = e.stream_prev[h as usize];
        let ra = to_cur(m.body_a.0);
        // The SDF sentinel is `u32::MAX`, the same bits as `NONE`: test the manifold's kind, never
        // the translated row, for it.
        let sdf = m.body_b == SDF_SENTINEL;
        let rb = if sdf { SDF_SENTINEL.0 } else { to_cur(m.body_b.0) };
        if ra == NONE || (!sdf && rb == NONE) {
            ok = false;
            break;
        }
        for r in [ra, rb] {
            if r != SDF_SENTINEL.0 && !is_member(held, r) {
                held.push_row(r);
            }
        }
        let is_box = |r: u32| r != SDF_SENTINEL.0 && matches!(e.bodies[r as usize].shape, ColliderShape::Box { .. });
        box_kept += u32::from(is_box(ra) && is_box(rb));
        points += u32::from(m.count);
        let mut km = m;
        km.body_a = BodyIndex(ra);
        km.body_b = BodyIndex(rb);
        held.push_kept(KeptManifold { m: km, record: NONE });
    }
    // The kept pairs.
    let mut n_reuse = 0u32;
    if ok {
        for &entry in pairs {
            let j = entry as u32 as usize;
            let (pa, pb) = e.pairs_prev[j];
            let (ra, rb) = (to_cur(pa.0), to_cur(pb.0));
            if ra == NONE || rb == NONE {
                ok = false;
                break;
            }
            let (fa, fb) = (cls[ra as usize].flags, cls[rb as usize].flags);
            // D-G: a sensor pair is never kept, and its partner is no anchor.
            if sensor_pair(fa, fb) {
                continue;
            }
            let mut tag = tags[j];
            let mut reuse = if tag.has(PairTag::REC) { recs.get(j).copied() } else { None };
            if tag.is_held_skip() {
                // Invariant K (design 06 B3): the pair was skipped at the previous step for its
                // held endpoint, whose record keeps its state.
                let y = if fa & RowCls::PRE_HELD != 0 { ra } else { rb };
                let k_y = cls[y as usize].island;
                let view = held.view();
                let rec_y = view.records()[k_y as usize];
                // A record the prologue or A2.5a restored this step is read tombstoned, its table
                // translated but possibly out of order: `local_of` scans it (review W2 of C3c).
                rules.cross_reads_dead += u64::from(!rec_y.live());
                let copy = match (view.local_of(&rec_y, ra), view.local_of(&rec_y, rb)) {
                    (Some(la), Some(lb)) => view.find_pair(&rec_y, la, lb).copied(),
                    _ => None,
                };
                debug_assert!(
                    copy.is_some() || !keeps_pair_on(view, &rec_y, ra, rb),
                    "invariant: the lookup finds the copy of ({ra}, {rb}) its held record keeps \
                     (Invariant K, design 06 B3)"
                );
                match copy {
                    Some(kp) => {
                        tag = kp.tag;
                        reuse = view.reuse_of(&rec_y, &kp).copied();
                        rules.cross_copies += 1;
                    }
                    None => tag = PairTag::NONE,
                }
            }
            if tag.has(PairTag::BOX) && !admit(tag) {
                rules.refused_m3 += 1;
                ok = false;
                break;
            }
            if !kept_class(tag) {
                continue;
            }
            for r in [ra, rb] {
                if !is_member(held, r) {
                    held.push_row(r);
                }
            }
            let rec_slot = if tag.has(PairTag::REC) {
                n_reuse += 1;
                if !tag.has(PairTag::HIT) {
                    rules.rec_settled_misses += 1;
                }
                if !tag.has(PairTag::PUSHED) {
                    rules.rec_without_manifold += 1;
                }
                n_reuse - 1
            } else {
                NONE
            };
            let record = if tag.has(PairTag::REC) {
                Some(reuse.expect("invariant: a REC tag's record was written at its slot"))
            } else {
                None
            };
            held.push_pair(KeptPair { la: ra, lb: rb, m_slot: NONE, r_slot: rec_slot, tag, _p: 0 }, record);
        }
    }
    // M2: the anchors, sorted and deduplicated, each resting.
    let n_anchors = if ok {
        held.sort_rows_since(mark, nm);
        let na = held.dedup_rows_since(mark, nm);
        if held.rows_since(mark)[nm..].iter().any(|&r| cls[r as usize].flags & RowCls::RESTING == 0) {
            rules.refused_m2 += 1;
            ok = false;
        }
        na
    } else {
        0
    };
    if !ok {
        held.rollback(mark);
        return false;
    }
    // Local indices: the row table is final.
    held.localize_since(mark, nm);
    let root = to_cur(e.graph.island_info()[island as usize].root);
    debug_assert!(
        held.rows_since(mark)[..nm].binary_search(&root).is_ok(),
        "invariant: the island's root is one of its members"
    );
    held.commit(
        mark,
        HeldIsland {
            root,
            n_members: nm as u32,
            n_anchors: n_anchors as u32,
            box_kept,
            points,
            ..HeldIsland::default()
        },
        n,
    );
    true
}

#[cfg(test)]
mod tests {
    //! L10 C3b's unit gates (design 06 §7, 08 §7): the admission rule (N24) and the settle bits
    //! (N25) exhaustively, the restore sources against a `Vec` sort, and the grep gate "`&` the
    //! `SENSOR` flag only inside `sensor_pair`" (08 D-G, O-3).

    use boyko_ecs::ecs::core::component::scratch::ScratchColumn;

    use super::{SleepSets, admit, sensor_pair};
    use crate::held_store::tests::{ModelRecord, manifold, move_in};
    use crate::held_store::HeldStore;
    use crate::manifold::SDF_SENTINEL;
    use crate::narrowphase::carry::PairTag;
    use crate::narrowphase::reuse::ReuseRecord;
    use crate::row_identity::RowRemap;
    use crate::scratch_ids::{register_sleep_sets_layouts, sleep_scratch_id};
    use crate::solver::warm_records::ord;

    /// N24: the admission rule, over every `u16` tag, against its statement in design 08 B5′
    /// (and 06 B5): the held-skip tag never; a `REC` tag iff it hit or settled; a `SEP` tag
    /// always; a tag with `SET` iff it emitted a manifold and settled; nothing else.
    #[test]
    fn admit_is_the_b5_rule_on_every_tag() {
        const REC: u16 = 1 << 8;
        const SEP: u16 = 1 << 9;
        const HIT: u16 = 1 << 10;
        const SET: u16 = 1 << 6;
        const SETTLED: u16 = 1 << 7;
        const PUSHED: u16 = 1 << 5;
        for bits in 0..=u16::MAX {
            let tag = PairTag::from_bits(bits);
            let want = if bits == 0x0C00 {
                false
            } else if bits & REC != 0 {
                bits & (HIT | SETTLED) != 0
            } else if bits & SEP != 0 {
                true
            } else if bits & SET != 0 {
                bits & (PUSHED | SETTLED) == PUSHED | SETTLED
            } else {
                false
            };
            assert_eq!(admit(tag), want, "admit({bits:#06x})");
        }
        // The tags the collision paths write, named.
        let miss_settled = PairTag::box_recorded(2, true, false).settled(Some(2));
        let miss_unsettled = PairTag::box_recorded(2, true, false).settled(Some(5));
        let hit = PairTag::box_recorded(2, false, true).settled(None);
        assert!(admit(miss_settled), "a settled record miss is admitted (arm 9)");
        assert!(!admit(miss_unsettled), "an unsettled record miss is refused");
        assert!(admit(hit), "a record hit is admitted");
        assert!(admit(PairTag::box_separated(4, false).settled(None)), "a separated pair is admitted");
        assert!(!admit(PairTag::box_contact(1, true).settled(Some(3))), "an unsettled contact is refused");
        assert!(admit(PairTag::box_contact(1, true).settled(Some(1))), "a settled contact is admitted");
        assert!(!admit(PairTag::HELD_SKIP), "rule 5: the held-skip tag is never admitted");
    }

    /// N25: a record hit and a settled record miss of the same pair agree on every claimed bit
    /// (`HIT_INVARIANT`), for every axis and both emit states; so do a carried-axis hit, a
    /// record-found separation and a full separation on the same axis (design 08 B1′).
    #[test]
    fn the_hit_paths_agree_with_the_settled_misses_on_the_claimed_bits() {
        let inv = |t: PairTag| t.bits() & PairTag::HIT_INVARIANT;
        for axis in 0..15usize {
            for pushed in [false, true] {
                let hit = PairTag::box_recorded(axis, pushed, true).settled(None);
                let miss = PairTag::box_recorded(axis, pushed, false).settled(Some(axis));
                assert_eq!(inv(hit), inv(miss), "REC axis {axis} pushed {pushed}: {hit:?} vs {miss:?}");
                assert!(hit.has(PairTag::SET | PairTag::SETTLED), "a hit is SET and SETTLED");
            }
            let a = axis as u8;
            let full = PairTag::box_separated(a, false).settled(None);
            for other in [PairTag::box_separated(a, true), PairTag::box_separated_by_record(a)] {
                assert_eq!(inv(other.settled(None)), inv(full), "SEP axis {axis}: {other:?}");
            }
            assert!(!full.has(PairTag::SET) && !full.has(PairTag::SETTLED), "a separation sets neither");
        }
    }

    /// The restore sources (design 06 A3, B4; 08 A3′ OQ1, OQ2) against a `Vec` sort: the warm
    /// source holds the restored records' kept manifolds keyed by their previous-row ordinals,
    /// strictly sorted; the pair source their kept pairs by previous rows, sorted, with their
    /// tags; an entry naming a vanished row is dropped; a Reset writes both empty.
    #[test]
    fn the_restore_sources_are_the_sorted_previous_keys() {
        register_sleep_sets_layouts();
        let mut store = HeldStore::with_capacity(0);
        let mut ids: ScratchColumn<u64> = ScratchColumn::new(sleep_scratch_id(), 1 << 10);
        let rec = |members: Vec<u32>, kept: Vec<(u32, u32)>| {
            let kept: Vec<_> = kept
                .into_iter()
                .enumerate()
                .map(|(i, (a, b))| (manifold(a, b, 2, i as u64), i as u64))
                .collect();
            let mut kept = kept;
            kept.sort_unstable_by_key(|(m, _)| ord(m.body_a.0, m.body_b.0));
            let pairs = kept
                .iter()
                .filter(|(m, _)| m.body_b != SDF_SENTINEL)
                .map(|(m, _)| {
                    let tag = PairTag::box_recorded(1, true, false).settled(Some(1));
                    (m.body_a.0, m.body_b.0, tag, Some(ReuseRecord::UNWRITTEN))
                })
                .collect();
            ModelRecord { members, kept, pairs, live: true }
        };
        let n = 32usize;
        move_in(&mut store, &mut ids, &rec(vec![10, 11], vec![(0, 10), (10, 11), (11, SDF_SENTINEL.0)]), n);
        move_in(&mut store, &mut ids, &rec(vec![20], vec![(1, 20)]), n);
        move_in(&mut store, &mut ids, &rec(vec![5, 6, 7], vec![(0, 5), (5, 6), (6, 7)]), n);
        // Rows step: every row at or above 8 moved up by 2 since the previous gather, and row 6
        // vanished (current row 6 is a new body; the previous row 6 has no current row).
        let prev_row: Vec<u32> = (0..n as u32)
            .map(|r| if r >= 8 { r - 2 } else if r == 6 { crate::row_identity::NO_ROW } else { r })
            .collect();
        let mut sets = SleepSets::with_capacity(n);
        sets.kept_rec.build_view().resize(store.kept_len(), crate::solver::warm_records::WarmRecord::EMPTY);
        store.mark_restore(0);
        store.mark_restore(2);
        let restored = sets.restore_marked(&mut store, Some(RowRemap::Rows(&prev_row)), n, 7);
        assert_eq!(restored, 2, "two records restored");
        let prev = |r: u32| prev_row[r as usize];
        // The oracle: record 0 (rows 10, 11 → 8, 9) and record 2 (rows 5, 6, 7; 6 vanished).
        let mut want_warm: Vec<u64> = vec![ord(prev(0), prev(10)), ord(prev(10), prev(11)), ord(prev(11), SDF_SENTINEL.0)];
        want_warm.extend([ord(prev(0), prev(5))]);
        want_warm.sort_unstable();
        assert_eq!(sets.restore_rec.keys(), &want_warm[..], "warm keys: previous rows, sorted, vanished dropped");
        assert!(sets.restore_rec.strict(), "the warm source is strict");
        let mut want_pairs: Vec<(u32, u32)> = vec![(prev(0), prev(10)), (prev(10), prev(11)), (prev(0), prev(5))];
        want_pairs.sort_unstable();
        let got: Vec<(u32, u32)> = sets.restore_keys.as_read_slice().iter().map(|&(a, b)| (a.0, b.0)).collect();
        assert_eq!(got, want_pairs, "pair keys: previous rows, sorted, vanished dropped");
        assert_eq!(sets.restore_tags.len(), want_pairs.len(), "one tag per pair key");
        // A Reset: both sources empty.
        store.mark_restore(1);
        let restored = sets.restore_marked(&mut store, Some(RowRemap::Reset), n, 8);
        assert_eq!(restored, 1, "one record restored");
        assert!(sets.restore_rec.keys().is_empty() && sets.restore_keys.is_empty(), "a Reset writes both empty");
    }

    /// Design 08 D-G, O-3: `sensor_pair` is the only place the `SENSOR` flag is tested; every
    /// other site routes through it. Reads this lane's sources.
    #[test]
    fn the_sensor_flag_is_tested_only_inside_sensor_pair() {
        let needle = format!("& RowCls::{}", "SENSOR");
        let sources = [
            ("sleep_sets.rs", include_str!("sleep_sets.rs")),
            ("systems.rs", include_str!("systems.rs")),
            ("held_store.rs", include_str!("held_store.rs")),
            ("narrowphase/dispatch.rs", include_str!("narrowphase/dispatch.rs")),
            ("narrowphase/reuse.rs", include_str!("narrowphase/reuse.rs")),
            ("solver/colored.rs", include_str!("solver/colored.rs")),
        ];
        let body = format!("(fa | fb) {needle} != 0");
        let mut found = 0;
        for (file, text) in sources {
            for (i, _) in text.match_indices(&needle) {
                let line = text[..i].lines().count();
                let at = &text[text[..i].rfind('\n').map_or(0, |k| k + 1)..];
                let at = at.lines().next().unwrap_or_default().trim();
                assert!(
                    at == body,
                    "{file}:{line}: the SENSOR flag tested outside sensor_pair: `{at}`"
                );
                found += 1;
            }
        }
        assert_eq!(found, 1, "anti-vacuity: sensor_pair's own test is found exactly once");
        assert!(sensor_pair(super::RowCls::SENSOR, 0) && !sensor_pair(0, 0), "the predicate itself");
    }
}
