//! [`ActionState<A>`] — the per-action result of input processing (plan §6,
//! Decision 5).
//!
//! Structure-of-Arrays: the hot "is action N pressed/just_pressed?" query is one
//! [`BitSet256`] bit test (branchless, one cache line). Analog values live in
//! separate dense arrays so a button-only game never touches `axis2` (hot/cold
//! split). The value arrays are allocated **once** at build (`with_count`),
//! never per frame.
//!
//! # I4 seam (fixed-step determinism, plan §7.3)
//! The C3 frame-stable *fixed snapshot* (`fixed_*` mirrors read by the Fixed
//! schedule) and the `Resource` impl via the TypeId registry are added in I4.
//! This module ships only the windowing-independent core: the live sets, the
//! value arrays, the consumption accessors, and the per-frame `begin_frame`
//! reset that [`process_actions`](super::process::process_actions) writes into.

use core::marker::PhantomData;

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
    // --- cold values: allocated ONCE at build, never per frame ---
    /// Analog value for Button (`0..1`) / Axis1D (`-1..1`); `len == COUNT`.
    button_value: Box<[f32]>,
    /// Deadzoned + clamped 2D value for Axis2D; `len == COUNT`.
    axis2: Box<[[f32; 2]]>,
    _pd: PhantomData<A>,
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
            button_value: vec![0.0f32; count].into_boxed_slice(),
            axis2: vec![[0.0f32; 2]; count].into_boxed_slice(),
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

    /// Marks `a` handled this frame: clears its edges and sets its `consumed`
    /// bit so later queries see it as inactive (plan §7.3 O3 semantics; the
    /// fixed-snapshot interaction is wired in I4).
    #[inline]
    pub fn consume(&mut self, a: A) {
        let i = a.index();
        self.just_pressed.clear(i);
        self.just_released.clear(i);
        self.consumed.set(i);
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
}

impl<A: Actionlike> Default for ActionState<A> {
    #[inline]
    fn default() -> Self {
        Self::new()
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
