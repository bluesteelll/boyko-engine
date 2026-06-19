//! [`process_actions`] — aggregate the physical snapshot into action state
//! (plan §12 step 3–4).
//!
//! Per frame, for each action, iterate its flat binding slice sequentially and
//! aggregate: buttons OR (held) / max (value); axes sum → deadzone → clamp;
//! WASD-style 2D composites normalize diagonals. Then run the clash pass
//! (Decision 8). Edges are derived from the **event-stream** physical edge bits
//! (`keys_just_pressed`/`keys_just_released`), so a same-frame down+up tap
//! survives (W4).
//!
//! **Zero per-frame heap allocation:** all work is over the fixed binding arena,
//! the fixed-size `ActionState` arrays, and a stack-resident clash candidate
//! set. No `Vec`, no `Box`, no `HashMap`, no `dyn`.

use crate::action::actionlike::{ActionKind, Actionlike};
use crate::action::clash::{resolve_prioritize_longest, CandidateSet};
use crate::action::map::{AxisMode, BindSpec, ClashStrategy, InputMap, InputRef};
use crate::action::state::{clamp_scalar, ActionState};
use crate::raw::keycode::KeyCode;
use crate::raw::queue::PhysicalInput;

/// Aggregates `physical` into `state` according to `map` (the per-frame hot
/// path). Clears the previous frame's action state first, then re-derives.
///
/// Generic over `A: Actionlike`; monomorphizes to a tight, allocation-free
/// loop. The central binding dispatch is a `match` jump table.
pub fn process_actions<A: Actionlike>(
    physical: &PhysicalInput,
    map: &InputMap<A>,
    state: &mut ActionState<A>,
) {
    state.begin_frame();

    let count = map.action_count();
    debug_assert!(count <= A::COUNT, "map declares more actions than the enum");

    // Stack-resident clash candidates (no heap).
    let mut candidates = CandidateSet::new();

    for action_index in 0..count {
        let kind = action_kind::<A>(action_index);
        let mut acc = Accum::new();

        for spec in map.bindings_at(action_index) {
            acc.aggregate(physical, spec);

            // Collect active button/chord bindings for the clash pass.
            if matches!(map.clash(), ClashStrategy::PrioritizeLongest)
                && spec.clash_len() > 0
                && binding_active(physical, spec)
            {
                candidates.push_active(action_index, spec);
            }
        }

        match kind {
            ActionKind::Button => {
                state.write_button(action_index, acc.held, clamp_scalar(kind, acc.scalar_max));
            }
            ActionKind::Axis1D => {
                let v = apply_deadzone_1d(acc.axis_sum[0], binding_deadzone_1d(map, action_index));
                state.write_button(action_index, v != 0.0, clamp_scalar(kind, v));
            }
            ActionKind::Axis2D => {
                let v = finalize_axis2(map, action_index, acc.axis_sum);
                state.write_axis2(action_index, v);
            }
        }

        if acc.rising {
            state.set_just_pressed(action_index);
        }
        if acc.falling {
            state.set_just_released(action_index);
        }
    }

    // Clash pass: suppress strict-subset actions (Ctrl+S vs S).
    if matches!(map.clash(), ClashStrategy::PrioritizeLongest) {
        resolve_prioritize_longest(&candidates, |action| {
            // Only suppress if the longer (superset) binding's action is itself
            // still active; `set_consumed` masks all reads of the subset action.
            if state.raw_pressed(action) {
                state.set_consumed(action);
            }
        });
    }
}

/// Returns the `ActionKind` for a dense action index by reconstructing `A`.
#[inline]
fn action_kind<A: Actionlike>(index: usize) -> ActionKind {
    match A::from_index(index) {
        Some(a) => a.kind(),
        // Indices are always `< A::COUNT`; reconstruction cannot fail.
        None => ActionKind::Button,
    }
}

/// Per-action aggregation accumulators, folded across one action's bindings.
///
/// Grouping these into a struct keeps [`Accum::aggregate`] a two-argument call
/// (the kind is applied by the caller at finalization, so it is not needed
/// per-binding) — cleaner than threading six `&mut` outparams.
struct Accum {
    /// Button / Axis1D magnitude (OR/max across bindings).
    scalar_max: f32,
    /// Axis1D/Axis2D pre-deadzone component sum.
    axis_sum: [f32; 2],
    /// Any binding currently active.
    held: bool,
    /// Any binding had a rising edge this frame.
    rising: bool,
    /// Any binding had a falling edge this frame.
    falling: bool,
}

impl Accum {
    #[inline]
    fn new() -> Self {
        Self {
            scalar_max: 0.0,
            axis_sum: [0.0; 2],
            held: false,
            rising: false,
            falling: false,
        }
    }

    /// Folds one binding into the accumulators.
    #[inline]
    fn aggregate(&mut self, physical: &PhysicalInput, spec: &BindSpec) {
        match spec {
            BindSpec::Key(code) => {
                if key_held(physical, *code) {
                    self.held = true;
                    self.scalar_max = self.scalar_max.max(1.0);
                }
                self.rising |= key_rising(physical, *code);
                self.falling |= key_falling(physical, *code);
            }
            BindSpec::Mouse(button) => {
                if physical.mouse_held(*button) {
                    self.held = true;
                    self.scalar_max = self.scalar_max.max(1.0);
                }
                self.rising |= mouse_rising(physical, *button);
                self.falling |= mouse_falling(physical, *button);
            }
            BindSpec::Chord { keys, len } => {
                let n = *len as usize;
                let all_held = n > 0 && keys[..n].iter().all(|k| key_held(physical, *k));
                if all_held {
                    self.held = true;
                    self.scalar_max = self.scalar_max.max(1.0);
                    // Chord rising: all keys held now AND at least one had a
                    // rising edge (the last key completed the chord) — W4.
                    self.rising |= keys[..n].iter().any(|k| key_rising(physical, *k));
                }
                // Chord falling: a member just released.
                if n > 0 {
                    self.falling |= keys[..n].iter().any(|k| key_falling(physical, *k));
                }
            }
            BindSpec::Axis1 { neg, pos, dz: _ } => {
                let p = leg(physical, *pos);
                let ng = leg(physical, *neg);
                let v = p - ng;
                self.axis_sum[0] += v;
                self.held |= v != 0.0;
                self.rising |=
                    input_ref_rising(physical, *pos) || input_ref_rising(physical, *neg);
                self.falling |=
                    input_ref_falling(physical, *pos) || input_ref_falling(physical, *neg);
            }
            BindSpec::Axis2 {
                up,
                down,
                left,
                right,
                dz: _,
                mode: _,
            } => {
                let x = leg(physical, *right) - leg(physical, *left);
                let y = leg(physical, *up) - leg(physical, *down);
                self.axis_sum[0] += x;
                self.axis_sum[1] += y;
                self.held |= x != 0.0 || y != 0.0;
                let legs = [*up, *down, *left, *right];
                self.rising |= legs.iter().any(|r| input_ref_rising(physical, *r));
                self.falling |= legs.iter().any(|r| input_ref_falling(physical, *r));
            }
            // Reserved gamepad seam / explicit unbind contribute nothing in v1.
            BindSpec::Stick | BindSpec::None => {}
        }
    }
}

/// Finalizes an Axis2D action: deadzone then optional diagonal normalization
/// then clamp to the unit square legs.
#[inline]
fn finalize_axis2<A: Actionlike>(
    map: &InputMap<A>,
    action_index: usize,
    sum: [f32; 2],
) -> [f32; 2] {
    let (dz, mode) = axis2_params(map, action_index);
    let mut v = sum;

    // Deadzone on magnitude.
    let mag = (v[0] * v[0] + v[1] * v[1]).sqrt();
    if mag <= dz || mag == 0.0 {
        return [0.0, 0.0];
    }

    if matches!(mode, AxisMode::DigitalNormalized) && mag > 1.0 {
        // Diagonal normalization: cap magnitude at 1 (WASD diagonal == straight).
        v[0] /= mag;
        v[1] /= mag;
    }

    [v[0].clamp(-1.0, 1.0), v[1].clamp(-1.0, 1.0)]
}

/// Extracts `(dz, mode)` from the first Axis2 binding of an action (the
/// composite carries its own deadzone/mode).
#[inline]
fn axis2_params<A: Actionlike>(map: &InputMap<A>, action_index: usize) -> (f32, AxisMode) {
    for spec in map.bindings_at(action_index) {
        if let BindSpec::Axis2 { dz, mode, .. } = spec {
            return (*dz, *mode);
        }
    }
    (0.0, AxisMode::DigitalNormalized)
}

/// Extracts the deadzone of the first Axis1 binding of an action.
#[inline]
fn binding_deadzone_1d<A: Actionlike>(map: &InputMap<A>, action_index: usize) -> f32 {
    for spec in map.bindings_at(action_index) {
        if let BindSpec::Axis1 { dz, .. } = spec {
            return *dz;
        }
    }
    0.0
}

/// Applies a symmetric 1D deadzone, then leaves the value for the caller to
/// clamp.
#[inline]
fn apply_deadzone_1d(v: f32, dz: f32) -> f32 {
    if v.abs() <= dz {
        0.0
    } else {
        v
    }
}

/// Whether a binding currently contributes any held input (for clash candidacy).
#[inline]
fn binding_active(physical: &PhysicalInput, spec: &BindSpec) -> bool {
    match spec {
        BindSpec::Key(code) => key_held(physical, *code),
        BindSpec::Mouse(button) => physical.mouse_held(*button),
        BindSpec::Chord { keys, len } => {
            let n = *len as usize;
            n > 0 && keys[..n].iter().all(|k| key_held(physical, *k))
        }
        BindSpec::Axis1 { .. } | BindSpec::Axis2 { .. } | BindSpec::Stick | BindSpec::None => false,
    }
}

// --- physical-level helpers ---

#[inline]
fn key_held(physical: &PhysicalInput, code: KeyCode) -> bool {
    code.dense_index()
        .is_some_and(|i| physical.keys_pressed.get(i))
}

#[inline]
fn key_rising(physical: &PhysicalInput, code: KeyCode) -> bool {
    code.dense_index()
        .is_some_and(|i| physical.keys_just_pressed.get(i))
}

#[inline]
fn key_falling(physical: &PhysicalInput, code: KeyCode) -> bool {
    code.dense_index()
        .is_some_and(|i| physical.keys_just_released.get(i))
}

#[inline]
fn input_ref_held(physical: &PhysicalInput, r: InputRef) -> bool {
    match r {
        InputRef::Key(code) => key_held(physical, code),
        InputRef::Mouse(button) => physical.mouse_held(button),
    }
}

#[inline]
fn input_ref_rising(physical: &PhysicalInput, r: InputRef) -> bool {
    match r {
        InputRef::Key(code) => key_rising(physical, code),
        InputRef::Mouse(button) => mouse_rising(physical, button),
    }
}

#[inline]
fn input_ref_falling(physical: &PhysicalInput, r: InputRef) -> bool {
    match r {
        InputRef::Key(code) => key_falling(physical, code),
        InputRef::Mouse(button) => mouse_falling(physical, button),
    }
}

#[inline]
fn leg(physical: &PhysicalInput, r: InputRef) -> f32 {
    if input_ref_held(physical, r) {
        1.0
    } else {
        0.0
    }
}

#[inline]
fn mouse_rising(physical: &PhysicalInput, button: crate::raw::keycode::MouseButton) -> bool {
    match button.dense_index() {
        Some(i) => (physical.mouse_just_pressed >> i) & 1 == 1,
        None => false,
    }
}

#[inline]
fn mouse_falling(physical: &PhysicalInput, button: crate::raw::keycode::MouseButton) -> bool {
    match button.dense_index() {
        Some(i) => (physical.mouse_just_released >> i) & 1 == 1,
        None => false,
    }
}
