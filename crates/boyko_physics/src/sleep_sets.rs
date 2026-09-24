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
//! # What this commit (C2a) lands: plumbing under `Off`
//!
//! * the [`SleepSets`] resource, inserted on the colored path only, with the per-row
//!   classification column (`RowCls`, flags only), the step mode the broadphase records, the
//!   narrowphase's arm, and the counters ([`SleepSkipStats`]);
//! * the step's sleep epoch type (`SleepEpoch`, design 08 D-E′), not yet computed;
//! * the one routing predicate of both narrowphase paths ([`np_route`], design 08 D1′) and the
//!   one sensor predicate ([`sensor_pair`], D-G), with the narrowphase's `Sets` arm compiled but
//!   never selected: nothing is held before C3b, so [`SleepSets::np_sets`] is always `false`
//!   and every pair routes to the stream, exactly as before.
//!
//! No value moves: a world with sleeping off runs [`SleepSkip::Off`] whatever the mode says, and
//! a sleeping world in `Sets` holds nothing yet.

use boyko_ecs::ecs::core::component::scratch::ScratchColumn;
use boyko_macros::Resource;
use boyko_sdf_math::{MAX_SDF_EDITS, SdfEdit};

use crate::manifold::BodyIndex;
use crate::profiling::{PHYS_SLEEP_CLASSIFY, PHYS_SLEEP_HELD, counter};
use crate::resources::{PhysicsConfig, SdfNarrowphaseKernel, SleepSkip};
use crate::row_identity::{RemapCursor, RowIdentity};
use crate::scratch_ids::{register_sleep_sets_layouts, scratch_reserve_rows, sleep_row_cls_id};
use crate::sdf_query::SdfField;

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
    /// [`SENSOR_NBR`](Self::SENSOR_NBR).
    pub(crate) flags: u32,
}

const _: () = assert!(
    size_of::<RowCls>() == 8 && align_of::<RowCls>() == 4,
    "RowCls is the 8 B, align 4 per-row classification element"
);

// The flags whose first reader is L10 C3b's broadphase prologue (A1.3) or epilogue (A2). The bit
// layout is fixed here, with the ones the narrowphase's route already reads, so it cannot drift.
#[expect(dead_code, reason = "L10 C3b's broadphase prologue and epilogue are the first readers")]
impl RowCls {
    /// The row's inputs are bit-equal to the previous step's (R1–R4 of design 04 D2): it has a
    /// previous row, that row was immovable or not awake, its `BodyState` is bit-equal, the
    /// cursors classify, and it is no jumper and no degenerate box.
    pub(crate) const RESTING: u32 = 1 << 0;
    /// The row was held at the previous step (its record survives into this one).
    pub(crate) const PRE_HELD: u32 = 1 << 2;
    /// The row's island froze at the previous step and is not held: a move-in candidate.
    pub(crate) const CAND: u32 = 1 << 3;
    /// The row's sleep latch is asleep after the previous step's `end_step`.
    pub(crate) const LATCHED: u32 = 1 << 5;
    /// The row is a held or candidate row with a sensor pair in the stream (design 08 D-F):
    /// its frames are filled even while it is held.
    pub(crate) const SENSOR_NBR: u32 = 1 << 7;
    /// No island: the value of [`island`](Self::island) for a row that has none.
    pub(crate) const NO_ISLAND: u32 = u32::MAX;
}

impl RowCls {
    /// The row is held this step: its pairs are skipped and its island is not solved.
    pub(crate) const HELD: u32 = 1 << 1;
    /// The row is a sensor ([`BodyState::is_sensor`](crate::resources::BodyState)).
    pub(crate) const SENSOR: u32 = 1 << 4;
    /// The row belongs to a record restored this step: its pairs are computed from the kept
    /// source, not the stream's (design 06 B4).
    pub(crate) const RESTORED: u32 = 1 << 6;
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
/// L10 (design 08 D-G). The route, the kept unit and its anchors, the D2 scan and the frame
/// fill all call it, so a sensor pair is computed every step whatever its endpoints are.
#[inline]
pub(crate) fn sensor_pair(fa: u32, fb: u32) -> bool {
    (fa | fb) & RowCls::SENSOR != 0
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
    /// Whether warm-starting is enabled.
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
}

#[expect(dead_code, reason = "L10 C3b's broadphase prologue (A1.0, D5) computes and compares it")]
impl SleepEpoch {
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
#[expect(dead_code, reason = "L10 C3b's broadphase prologue (A1.0, D5) computes the epoch")]
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

/// The sleep-skip's counts for the last step whose broadphase ran it (step mode
/// [`SleepSkip::Sets`]); all zero before the first such step. 48 B.
///
/// Every field is `0` until L10 C3b holds islands.
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

/// The sleep-skip's state (L10), inserted on the colored path beside
/// [`IslandSleep`](crate::resources::IslandSleep). Every per-row datum is a kernel column
/// (principle 0); no field is shared across threads.
///
/// Written by the colored broadphase ([`physics_broadphase_colored`], which records the step
/// mode) and the colored narrowphase ([`physics_narrowphase_colored`], which reads the arm the
/// broadphase chose); later stages read the step mode it recorded, never the configuration, so
/// a configuration write between two stages cannot split a step (design 04 D9).
///
/// [`physics_broadphase_colored`]: crate::systems::physics_broadphase_colored
/// [`physics_narrowphase_colored`]: crate::systems::physics_narrowphase_colored
#[derive(Resource)]
pub struct SleepSets {
    /// This step's per-row classification, indexed by current row (`RowCls`). Empty until L10
    /// C3b's broadphase prologue writes it.
    row_cls: ScratchColumn<RowCls>,
    /// L10's place in the gather sequence: stamped by every step whose broadphase ran the
    /// sleep-skip, so the held store and the baseline are keyed by that gather (design 04 D9).
    cursor: RemapCursor,
    /// The config inputs of the last step that ran the sleep-skip (design 08 D-E′).
    #[expect(dead_code, reason = "L10 C3b's broadphase prologue (A1.0, D5) is the first reader")]
    epoch: SleepEpoch,
    /// The mode the broadphase recorded for this step: `Off` when sleeping is off, else
    /// `PhysicsConfig::sleep_skip`.
    step_mode: SleepSkip,
    /// Whether the narrowphase runs its `Sets` arm this step (design 08 D-H: the step mode is
    /// `Sets`, or a restore set is non-empty). Always `false` before L10 C3b, which writes the
    /// per-row classification that arm reads.
    np_sets: bool,
    /// Bumped whenever the held set changes, for the solve's consistency check (design 04).
    #[expect(dead_code, reason = "L10 C3b's broadphase epilogue (A2.5) bumps it")]
    flags_gen: u64,
    /// The counts of the last step that ran the sleep-skip.
    stats: SleepSkipStats,
}

impl Default for SleepSets {
    /// Hand-written because the per-row column needs its reserved `ComponentId`.
    #[inline]
    fn default() -> Self {
        Self::with_capacity(0)
    }
}

impl SleepSets {
    /// An empty sleep-skip state, its per-row column reserved for at least `rows` rows.
    pub fn with_capacity(rows: usize) -> Self {
        register_sleep_sets_layouts();
        Self {
            row_cls: ScratchColumn::new(
                sleep_row_cls_id(),
                rows.max(scratch_reserve_rows(size_of::<RowCls>())),
            ),
            cursor: RemapCursor::default(),
            epoch: SleepEpoch::UNSET,
            step_mode: SleepSkip::Off,
            np_sets: false,
            flags_gen: 0,
            stats: SleepSkipStats::default(),
        }
    }

    /// The counts of the last step whose broadphase ran the sleep-skip.
    #[inline]
    pub fn stats(&self) -> SleepSkipStats {
        self.stats
    }

    /// The mode the last broadphase recorded: [`SleepSkip::Off`] when sleeping was off,
    /// otherwise `PhysicsConfig::sleep_skip` as it stood at the broadphase.
    #[inline]
    pub fn step_mode(&self) -> SleepSkip {
        self.step_mode
    }

    /// Whether row `row` is held this step: its pairs are skipped and its island is not solved.
    /// `false` for a row out of range, and for every row before L10 C3b holds islands.
    #[inline]
    pub fn is_row_held(&self, row: usize) -> bool {
        self.row_cls.as_read_slice().get(row).is_some_and(|c| c.flags & RowCls::HELD != 0)
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

    /// The broadphase prologue (design 04 A1): records the step mode and, on a step that runs
    /// the sleep-skip, opens its classification.
    ///
    /// A1.0: the mode is `Off` when sleeping is off, else `cfg.sleep_skip`. An `Off` step returns
    /// without stamping, so a later switch into `Sets` finds a stale cursor and takes one Reset
    /// step (D9). C2a stops there: with nothing held, the classification writes no row and the
    /// narrowphase keeps its `Off` arm. The prologue's span and the held-row counter are emitted
    /// once per step that runs it.
    pub(crate) fn open_step(&mut self, cfg: &PhysicsConfig, rows: &RowIdentity) {
        let mode = if cfg.sleeping { cfg.sleep_skip } else { SleepSkip::Off };
        self.step_mode = mode;
        self.np_sets = false;
        if mode == SleepSkip::Off {
            return;
        }
        let _zone = boyko_diag::zone!(PHYS_SLEEP_CLASSIFY);
        self.stats = SleepSkipStats::default();
        // Nothing is held, so the (empty) held state is keyed by this gather as by any other.
        self.cursor.stamp(rows);
        counter!(PHYS_SLEEP_HELD, u64::from(self.stats.held_rows));
    }

    /// Records the narrowphase's held-skip count for the step (design 06 D-D D3).
    #[inline]
    pub(crate) fn note_held_skips(&mut self, skips: u32) {
        self.stats.held_skipped_pairs = skips;
    }
}
