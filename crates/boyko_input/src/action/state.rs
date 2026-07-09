//! [`ActionState<A>`] — the per-action result of input processing (plan §6,
//! Decision 5).
//!
//! Structure-of-Arrays: the hot "is action N pressed/just_pressed?" query is one
//! [`BitSet256`] bit test (branchless, one cache line). Analog values live in
//! separate dense arrays so a button-only game never touches `axis2` (hot/cold
//! split). The value arrays are allocated **once** at build (`with_count`),
//! never per frame.
//!
//! # Fixed-step determinism (plan §7.3, C3)
//! Alongside the live (Main-facing) sets, `ActionState` holds a **frame-stable
//! fixed snapshot** (`fixed_*`) that the Fixed schedule reads. Each Main frame
//! [`freeze_fixed_snapshot`](ActionState::freeze_fixed_snapshot) **overwrites**
//! the frozen levels (a level is sampled) and **OR-accumulates** the frozen
//! edges (sticky until consumed). Because the frame order is fixed-loop-first
//! then Main, an edge frozen on frame N is carried forward across any 0-substep
//! frame N+1 and is consumed by the first fixed batch that runs after it;
//! [`clear_fixed_edges`](ActionState::clear_fixed_edges) — driven at the start
//! of the next Main, gated on `FixedTime::steps_this_frame > 0` — clears it once
//! that batch has run. The snapshot is otherwise identical for every substep of
//! one fixed batch (no per-substep mutation, no substep index needed), so a
//! 0-substep frame never misses a press (sticky edge) and an N-substep batch
//! never double-counts it (cleared after the consuming batch).

use core::marker::PhantomData;

use boyko_ecs::ecs::core::resources::resource::Resource;
use boyko_ecs::ecs::core::resources::resource_id_for;
use boyko_ecs::ecs::identifiers::primitives::ResourceId;
use boyko_utils::bit_mask::bit_set_256::BitSet256;

use crate::action::actionlike::{ActionKind, Actionlike};
use crate::constants::MAX_ACTIONS;

/// The processed state of every action, queried by gameplay systems.
///
/// Built once with [`ActionState::with_count`]; mutated each frame by
/// [`process_actions`](super::process::process_actions). The hot bitsets are
/// `4 × 32 B = 128 B` (2 cache lines); the value arrays are cold and untouched
/// by button-only queries.
#[repr(C)]
pub struct ActionState<A: Actionlike> {
    // --- hot: edge/level bitsets, no per-frame alloc ---
    /// Bit `i` = action `i` held this frame (level).
    pressed: BitSet256,
    /// Bit `i` = action `i` rising edge this frame.
    just_pressed: BitSet256,
    /// Bit `i` = action `i` falling edge this frame.
    just_released: BitSet256,
    /// Bit `i` = action `i` clash-suppressed or user-`consume`d this frame.
    consumed: BitSet256,
    // --- frame-stable snapshot the FIXED loop reads (C3, plan §7.3) ---
    /// Level, frozen for the whole of the next frame's fixed loop.
    fixed_pressed: BitSet256,
    /// Rising edge, frame-stable; single-consume owned by ingest (C3).
    fixed_just_pressed: BitSet256,
    /// Falling edge, frame-stable.
    fixed_just_released: BitSet256,
    // --- cold values: allocated ONCE at build, never per frame ---
    /// Analog value for Button (`0..1`) / Axis1D (`-1..1`); `len == COUNT`.
    button_value: Box<[f32]>,
    /// Deadzoned + clamped 2D value for Axis2D; `len == COUNT`.
    axis2: Box<[[f32; 2]]>,
    /// Frame-stable mirror of `button_value` for the fixed loop; `len == COUNT`.
    fixed_value: Box<[f32]>,
    /// Frame-stable mirror of `axis2` for the fixed loop; `len == COUNT`.
    fixed_axis2: Box<[[f32; 2]]>,
    // `fn() -> A`, not `A`: the marker owns no `A`, so `ActionState<A>` is
    // unconditionally `Send + Sync` (required by `Resource`) regardless of
    // whether `A: Send + Sync` — `Actionlike` does not demand those bounds.
    _pd: PhantomData<fn() -> A>,
}

impl<A: Actionlike> ActionState<A> {
    /// Allocates the value arrays for `count` actions (cold path, build-time).
    ///
    /// `count` is `A::COUNT`; passed explicitly so the constructor is testable
    /// without a full enum. Panics in debug if `count > MAX_ACTIONS`.
    pub fn with_count(count: usize) -> Self {
        debug_assert!(
            count <= MAX_ACTIONS,
            "action count {count} exceeds BitSet256 capacity {MAX_ACTIONS}"
        );
        Self {
            pressed: BitSet256::new(),
            just_pressed: BitSet256::new(),
            just_released: BitSet256::new(),
            consumed: BitSet256::new(),
            fixed_pressed: BitSet256::new(),
            fixed_just_pressed: BitSet256::new(),
            fixed_just_released: BitSet256::new(),
            button_value: vec![0.0f32; count].into_boxed_slice(),
            axis2: vec![[0.0f32; 2]; count].into_boxed_slice(),
            fixed_value: vec![0.0f32; count].into_boxed_slice(),
            fixed_axis2: vec![[0.0f32; 2]; count].into_boxed_slice(),
            _pd: PhantomData,
        }
    }

    /// Allocates sized to `A::COUNT` (the common path).
    #[inline]
    pub fn new() -> Self {
        Self::with_count(A::COUNT)
    }

    /// Clears all per-frame edge/level/value state at the start of a frame.
    ///
    /// Called by [`process_actions`](super::process::process_actions) before it
    /// re-aggregates from the fresh [`PhysicalInput`](crate::raw::queue::PhysicalInput)
    /// snapshot. Edges are recomputed from scratch each frame (event-stream
    /// authoritative, plan §7.3), so a full reset is correct and cheap (a few
    /// `u64` stores + a `memset` of the value arrays).
    #[inline]
    pub fn begin_frame(&mut self) {
        self.pressed = BitSet256::new();
        self.just_pressed = BitSet256::new();
        self.just_released = BitSet256::new();
        self.consumed = BitSet256::new();
        for v in self.button_value.iter_mut() {
            *v = 0.0;
        }
        for v in self.axis2.iter_mut() {
            *v = [0.0; 2];
        }
    }

    // --- consumption (hot path) ---

    /// Returns `true` if `a` is held this frame and not consumed.
    #[inline]
    pub fn pressed(&self, a: A) -> bool {
        let i = a.index();
        self.pressed.get(i) && !self.consumed.get(i)
    }

    /// Returns `true` if `a` had a rising edge this frame and is not consumed.
    #[inline]
    pub fn just_pressed(&self, a: A) -> bool {
        let i = a.index();
        self.just_pressed.get(i) && !self.consumed.get(i)
    }

    /// Returns `true` if `a` had a falling edge this frame and is not consumed.
    #[inline]
    pub fn just_released(&self, a: A) -> bool {
        let i = a.index();
        self.just_released.get(i) && !self.consumed.get(i)
    }

    /// The analog value of `a` (Button `0..1` / Axis1D `-1..1`), `0.0` if
    /// consumed. For an Axis2D action use [`ActionState::axis2`].
    #[inline]
    pub fn value(&self, a: A) -> f32 {
        let i = a.index();
        if self.consumed.get(i) {
            return 0.0;
        }
        self.button_value[i]
    }

    /// The deadzoned + clamped 2D value of an Axis2D action, `[0,0]` if
    /// consumed.
    #[inline]
    pub fn axis2(&self, a: A) -> [f32; 2] {
        let i = a.index();
        if self.consumed.get(i) {
            return [0.0; 2];
        }
        self.axis2[i]
    }

    /// Marks `a` handled this frame: clears its edges (in BOTH the Main-facing
    /// sets and the frozen fixed snapshot) and sets its `consumed` bit so later
    /// queries see it as inactive (plan §7.3 O3 semantics).
    ///
    /// Clearing the fixed snapshot's edge bit too keeps the fixed view
    /// self-consistent: a `consume` from a Main system is observed identically by
    /// every substep of the next frame's fixed loop; a `consume` from a Fixed
    /// system masks the action for the remaining substeps of the current frame
    /// (documented, rarely used). Clearing the *live* `just_pressed`/
    /// `just_released` bit also stops the action being re-frozen by the next
    /// [`freeze_fixed_snapshot`](ActionState::freeze_fixed_snapshot) (the frozen
    /// edges are now OR-accumulated, sticky-until-consumed — see that method), so
    /// a consumed press never leaks into a later fixed batch.
    #[inline]
    pub fn consume(&mut self, a: A) {
        let i = a.index();
        self.just_pressed.clear(i);
        self.just_released.clear(i);
        self.fixed_just_pressed.clear(i);
        self.fixed_just_released.clear(i);
        self.consumed.set(i);
    }

    // --- frame-stable fixed snapshot (C3, plan §7.3) ---

    /// Freezes the current Main-facing state into the fixed snapshot the Fixed
    /// schedule reads. Called once per frame at the end of
    /// [`update_action_state`](super::process::update_action_state).
    ///
    /// # Sticky-until-consumed edges (C3, plan §7.3)
    ///
    /// Levels (`fixed_pressed`, `fixed_value`, `fixed_axis2`) are **overwritten**
    /// with the current frame's value — a level is sampled, not latched. Edges
    /// (`fixed_just_pressed`, `fixed_just_released`) are **OR-accumulated** onto
    /// the existing frozen bits, NOT overwritten:
    ///
    /// ```text
    /// fixed_just_pressed  |= just_pressed
    /// fixed_just_released |= just_released
    /// ```
    ///
    /// The frame order is Fixed-loop-first, then Main (`App::update_with_delta`).
    /// A press observed on frame N's Main is frozen here; if frame N+1's fixed
    /// loop runs **0 substeps** (a sub-timestep render frame, common above 64 Hz)
    /// no substep consumes it. Overwriting would then destroy the edge before any
    /// substep saw it (the original BUG-I4-C3 no-miss failure). OR-accumulating
    /// carries the edge forward across 0-substep frames; it is cleared only after
    /// a fixed batch consumes it, by
    /// [`clear_fixed_edges`](ActionState::clear_fixed_edges) at the start of the
    /// next Main (gated on `steps_this_frame > 0`). The result: a press is
    /// `fixed_just_pressed` for exactly the one fixed batch that first runs after
    /// it — never lost across 0-substep frames (no-miss), never counted in two
    /// batches (no-double-count).
    ///
    /// Allocation-free: the OR-accumulate drains a 32-byte stack copy of each
    /// edge set (the same pattern as the `consumed` suppression below), so the
    /// cost is O(actions-edged-this-frame), typically 0–2.
    #[inline]
    pub fn freeze_fixed_snapshot(&mut self) {
        // Levels: overwrite (a level is sampled each frame, not latched).
        self.fixed_pressed = self.pressed;
        self.fixed_value.copy_from_slice(&self.button_value);
        self.fixed_axis2.copy_from_slice(&self.axis2);

        // Edges: OR-accumulate (sticky until a fixed batch consumes them). The
        // pop-drain ORs each newly-edged bit in without a `union_with` method on
        // the public `BitSet256` surface; on the common no-input frame both
        // scratch copies are empty and neither loop iterates.
        let mut rising = self.just_pressed;
        while let Some(bit) = rising.pop_lowest_set_bit() {
            self.fixed_just_pressed.set(bit as usize);
        }
        let mut falling = self.just_released;
        while let Some(bit) = falling.pop_lowest_set_bit() {
            self.fixed_just_released.set(bit as usize);
        }

        // Suppress consumed actions in the frozen view up front, so the fixed
        // accessors are a single bit test (no per-read `consumed` mask). On the
        // common path `consumed` is empty, so this loop iterates zero times.
        // `pop_lowest_set_bit` drains a scratch copy, so `self.consumed` is
        // untouched (the Main-facing accessors still mask through it this frame).
        let mut consumed = self.consumed;
        while let Some(bit) = consumed.pop_lowest_set_bit() {
            let i = bit as usize;
            self.fixed_pressed.clear(i);
            self.fixed_just_pressed.clear(i);
            self.fixed_just_released.clear(i);
            self.fixed_value[i] = 0.0;
            self.fixed_axis2[i] = [0.0; 2];
        }
    }

    /// Clears the accumulated frozen edge bits (`fixed_just_pressed`,
    /// `fixed_just_released`) — the clear-on-consume half of the sticky-edge
    /// model (C3, plan §7.3).
    ///
    /// Called by [`clear_consumed_fixed_edges`](super::process::clear_consumed_fixed_edges)
    /// at the start of `CoreSchedule::Main`, AFTER the frame's fixed loop has
    /// run and BEFORE [`update_action_state`](super::process::update_action_state)
    /// OR-accumulates the next batch — but ONLY when the fixed loop ran ≥ 1
    /// substep this frame (`FixedTime::steps_this_frame > 0`). A 0-substep frame
    /// skips the clear, so the frozen edge persists to the next frame and the
    /// press is never lost (no-miss). Once a batch has consumed it, clearing here
    /// guarantees the next batch does not re-observe it (no-double-count).
    ///
    /// The frozen levels (`fixed_pressed`/value/axis2) are NOT touched: a level
    /// is re-sampled wholesale by every [`freeze_fixed_snapshot`], so it carries
    /// no stale state.
    #[inline]
    pub fn clear_fixed_edges(&mut self) {
        self.fixed_just_pressed = BitSet256::new();
        self.fixed_just_released = BitSet256::new();
    }

    /// Fixed-loop view: is `a` held? (frame-stable level, plan §7.3).
    ///
    /// A Fixed system reads this to "fire once per substep while held".
    #[inline]
    pub fn fixed_pressed(&self, a: A) -> bool {
        self.fixed_pressed.get(a.index())
    }

    /// Fixed-loop view: did `a` have a rising edge this frame? (frame-stable).
    ///
    /// Guaranteed `true` on every substep of exactly one frame per physical
    /// press, then `false`. A Fixed system wanting "fire once per press" reads
    /// this and acts idempotently per frame (the standard fixed-step input
    /// contract, plan §7.3 step 3).
    #[inline]
    pub fn fixed_just_pressed(&self, a: A) -> bool {
        self.fixed_just_pressed.get(a.index())
    }

    /// Fixed-loop view: did `a` have a falling edge this frame? (frame-stable).
    #[inline]
    pub fn fixed_just_released(&self, a: A) -> bool {
        self.fixed_just_released.get(a.index())
    }

    /// Fixed-loop view of the analog value (Button `0..1` / Axis1D `-1..1`).
    #[inline]
    pub fn fixed_value(&self, a: A) -> f32 {
        self.fixed_value[a.index()]
    }

    /// Fixed-loop view of the deadzoned + clamped 2D value (Axis2D).
    #[inline]
    pub fn fixed_axis2(&self, a: A) -> [f32; 2] {
        self.fixed_axis2[a.index()]
    }

    // --- writer surface used by `process_actions` (crate-internal) ---

    /// Sets the level + value for a button/axis1d action from an aggregated
    /// binding result. `held` drives the `pressed` bit; `value` is the analog
    /// magnitude.
    #[inline]
    pub(crate) fn write_button(&mut self, index: usize, held: bool, value: f32) {
        if held {
            self.pressed.set(index);
        }
        self.button_value[index] = value;
    }

    /// Sets the 2D value for an Axis2D action; marks it `pressed` if non-zero.
    #[inline]
    pub(crate) fn write_axis2(&mut self, index: usize, value: [f32; 2]) {
        if value[0] != 0.0 || value[1] != 0.0 {
            self.pressed.set(index);
        }
        self.axis2[index] = value;
    }

    /// Records a rising edge for `index`.
    #[inline]
    pub(crate) fn set_just_pressed(&mut self, index: usize) {
        self.just_pressed.set(index);
    }

    /// Records a falling edge for `index`.
    #[inline]
    pub(crate) fn set_just_released(&mut self, index: usize) {
        self.just_released.set(index);
    }

    /// Marks `index` consumed (clash suppression — plan Decision 8).
    #[inline]
    pub(crate) fn set_consumed(&mut self, index: usize) {
        self.consumed.set(index);
    }

    /// Read-back of the raw `pressed` bit (ignores `consumed`), for the clash
    /// pass and for `kind`-dispatched aggregation bookkeeping.
    #[inline]
    pub(crate) fn raw_pressed(&self, index: usize) -> bool {
        self.pressed.get(index)
    }

    // --- UI as an action SOURCE (GUI P4 Decision 9) ---------------------------

    /// UI-source rising edge for action `index`: ORs the **live** rising edge and
    /// sets the **level** bit (so a held UI button reads `pressed`), value `1.0`.
    ///
    /// This is the sanctioned UI→action path — symmetric to the device-side
    /// [`RawInputQueue::push_raw`](crate::raw::queue::RawInputQueue::push_raw) but
    /// on the *processed* side (UI is a post-processing action source, opposite
    /// direction from device adapters). It writes ONLY the live edge + level; it
    /// does NOT touch the fixed snapshot directly — the schedule ordering (GUI P4
    /// Decision 10) runs the UI dispatch before
    /// [`freeze_fixed_snapshot`](Self::freeze_fixed_snapshot), so the existing
    /// OR-accumulate carries the UI edge into the fixed loop with no second
    /// writer.
    ///
    /// `index < A::COUNT` is the caller's contract (the `OnClick`/`OnHover`/
    /// `OnSubmit` dense action index); a debug build asserts it.
    #[inline]
    pub fn ui_press(&mut self, index: usize) {
        debug_assert!(
            index < self.button_value.len(),
            "ui_press index {index} out of range (A::COUNT = {})",
            self.button_value.len()
        );
        if index >= self.button_value.len() {
            return;
        }
        self.just_pressed.set(index);
        self.pressed.set(index);
        self.button_value[index] = 1.0;
    }

    /// UI-source analog value for a Button/Axis1D action `index` (sliders, drag
    /// handles). Sets the level bit when `value != 0.0` and writes the analog
    /// magnitude into the live `button_value`. No edge is implied (a slider drag
    /// is a level, not a press). Same fixed-snapshot composition as
    /// [`ui_press`](Self::ui_press).
    #[inline]
    pub fn ui_set_value(&mut self, index: usize, value: f32) {
        debug_assert!(
            index < self.button_value.len(),
            "ui_set_value index {index} out of range (A::COUNT = {})",
            self.button_value.len()
        );
        if index >= self.button_value.len() {
            return;
        }
        if value != 0.0 {
            self.pressed.set(index);
        }
        self.button_value[index] = value;
    }
}

impl<A: Actionlike> Default for ActionState<A> {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

// NOT `#[derive(Resource)]`: the derive caches the id in a `static` inside the
// generic `resource_id()` body, which collapses every `A` onto one id
// (rust#22991). Mint through the shared kernel `TypeId`-keyed registry
// `boyko_ecs::…::resource_type_registry` (published for reuse — the same map
// `State<S>` uses).
impl<A: Actionlike> Resource for ActionState<A> {
    #[inline]
    fn resource_id() -> ResourceId {
        resource_id_for::<Self>()
    }
}

/// Applies the per-kind output convention to an aggregated scalar:
/// Button clamps to `0..1`, Axis1D clamps to `-1..1`. Axis2D is handled by the
/// 2D path. Pulled out so the aggregation in
/// [`process_actions`](super::process::process_actions) stays compact.
#[inline]
pub(crate) fn clamp_scalar(kind: ActionKind, raw: f32) -> f32 {
    match kind {
        ActionKind::Button => raw.clamp(0.0, 1.0),
        ActionKind::Axis1D => raw.clamp(-1.0, 1.0),
        // An Axis2D action has no scalar value; callers use the 2D path.
        ActionKind::Axis2D => 0.0,
    }
}
