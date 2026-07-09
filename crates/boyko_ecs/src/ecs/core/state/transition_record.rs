//! The per-`S` transition record + the monomorphised transition-apply
//! function and its type-erased schedule entry (Phase 17 D4, §5.1).

use crate::ecs::core::ecs_master::ecs_master::EcsMaster;
use crate::ecs::core::resources::resource::Resource;
use crate::ecs::core::resources::resource_type_registry::resource_id_for;
use crate::ecs::core::state::next_state::NextState;
use crate::ecs::core::state::state::State;
use crate::ecs::core::state::states::States;
use crate::ecs::identifiers::primitives::ResourceId;

/// A single state transition observed by the transition pass.
///
/// `exited` is `None` only for the synthesized initial transition
/// (`none → initial`, D7); `on_exit` reads `exited.as_ref() == Some(&target)`,
/// so it is naturally false for the synthetic case.
///
/// # `Clone`, not `Copy`
///
/// The plan (§4) describes this as `#[derive(Clone, Copy)]`, but [`States`]
/// bounds only `Clone`, not `Copy`. Deriving `Copy` here would require
/// `S: Copy`, which the `States` bound does not provide, so we derive `Clone`
/// only. The record holds the value by clone where needed; the data is a tiny
/// POD either way.
///
/// # Visibility (L1 fallback, plan §9 / §13-OQ4)
///
/// This type is `pub` rather than `pub(crate)`. It appears in the public
/// `impl FnMut(Res<StateTransitionRecord<S>>) -> bool` bound of the
/// `on_enter` / `on_exit` / `on_transition` conditions; keeping it
/// `pub(crate)` triggers the `private_interfaces` lint. It is an opaque POD
/// with no public mutators (fields are `pub(crate)`), so exposing the name is
/// harmless — Bevy likewise keeps its transition types public.
#[derive(Clone)]
pub struct Transition<S: States> {
    /// The state that was exited; `None` for the synthesized initial transition.
    pub(crate) exited: Option<S>,
    /// The state that was entered.
    pub(crate) entered: S,
}

/// Per-`S` record of the last transition observed *this frame*, stored as a
/// world-global [`Resource`].
///
/// Written only by the transition pass (the sole writer), which clears
/// `transition` to `None` at its own start each frame; read (shared) by
/// `on_enter`/`on_exit`/`on_transition`. The `Option` read is **not**
/// tick-based — `recorded_tick` is a defensive belt-and-suspenders stamp, not
/// a change-detection window.
///
/// Derives `Clone` only (not `Copy`) for the same reason as [`Transition<S>`].
/// `pub` for the same L1-fallback reason as [`Transition<S>`] (it is the
/// `Res<…>` param type in the public condition bounds); an opaque POD whose
/// fields are private and whose only public method is the read-only
/// `current`.
///
/// [`Resource`]: crate::ecs::core::resources::resource::Resource
#[derive(Clone)]
pub struct StateTransitionRecord<S: States> {
    /// The transition recorded this frame, or `None` on a no-transition frame
    /// (cleared at the start of every transition pass).
    transition: Option<Transition<S>>,
    /// Frame tick the `transition` was recorded for. Defensive guard, NOT a
    /// change-detection tick. Written by the pass (`apply_state_transition` /
    /// `Schedule::run_state_transitions`); read ONLY by the `debug_assert!`s in
    /// [`apply_state_transition`], which are compiled out in release — so this
    /// field is intentionally write-only there. The `dead_code` allow is kept
    /// for that release configuration, NOT because the writers are dormant.
    #[allow(dead_code)]
    recorded_tick: u32,
}

impl<S: States> StateTransitionRecord<S> {
    /// Returns the transition recorded this frame, or `None` if no transition
    /// fired this frame.
    #[inline]
    pub(crate) fn current(&self) -> Option<&Transition<S>> {
        self.transition.as_ref()
    }
}

impl<S: States> Default for StateTransitionRecord<S> {
    #[inline]
    fn default() -> Self {
        Self {
            transition: None,
            recorded_tick: 0,
        }
    }
}

impl<S: States> Resource for StateTransitionRecord<S> {
    // See `State<S>::resource_id` — minted through the `TypeId`-keyed registry
    // to avoid the rust#22991 generic-`static` collapse.
    #[inline]
    fn resource_id() -> ResourceId {
        resource_id_for::<StateTransitionRecord<S>>()
    }
}

/// Runs the per-`S` transition logic once (the monomorphised body behind a
/// [`StateEntry::apply`] fn-pointer). Implements plan §5.1.
///
/// `fire_initial` is the per-entry one-shot flag: on the first `Schedule::run`
/// it synthesizes a `none → initial` transition (D7); thereafter it is `false`.
///
/// # Borrow discipline (§12)
///
/// Every resource access is a fresh `world.resource{_mut}::<…>()` call scoped
/// to its own statement, so no two conflicting borrows of `world` are ever
/// live at once. Where both a `State<S>` read and a `&mut Record` write are
/// needed, the state value is cloned into a local *first*, then the `&mut
/// Record` is taken — never bound across a `State` read. This is what lets the
/// single three-arg fn compile with no `unsafe`.
pub(crate) fn apply_state_transition<S: States>(
    world: &mut EcsMaster,
    this_run: u32,
    fire_initial: bool,
) {
    // 1. Clear the record FIRST so a no-transition frame leaves
    //    `transition == None`.
    {
        let rec = world.resource_mut::<StateTransitionRecord<S>>();
        rec.transition = None;
        rec.recorded_tick = this_run;
    }

    // 2. Synthesized initial transition (D7). Falls through to step 3 so an
    //    initial + an immediately-queued Pending both apply (last-write-wins).
    if fire_initial {
        let initial = world.resource::<State<S>>().get().clone();
        let rec = world.resource_mut::<StateTransitionRecord<S>>();
        rec.transition = Some(Transition {
            exited: None,
            entered: initial,
        });
    }

    // 3. Drain `NextState` (`take` resets it to `Unchanged` in this same pass).
    let request = std::mem::take(world.resource_mut::<NextState<S>>());
    let requested = match request {
        NextState::Unchanged => return,
        NextState::Pending(value) => value,
    };

    // Clone the current value into a local BEFORE taking `&mut Record`.
    let current = world.resource::<State<S>>().get().clone();
    if requested == current {
        // D6 identity transition → no-op (record already cleared in step 1).
        return;
    }

    {
        let rec = world.resource_mut::<StateTransitionRecord<S>>();
        rec.transition = Some(Transition {
            exited: Some(current),
            entered: requested.clone(),
        });
        rec.recorded_tick = this_run;
    }

    *world.resource_mut::<State<S>>() = State::new(requested);

    // §9 invariants: the slab now holds the entered value and the request is
    // drained. `requested` was moved into `State<S>`; compare the slab against
    // the recorded `entered` (the same value, by construction).
    debug_assert!(
        Some(world.resource::<State<S>>().get())
            == world
                .resource::<StateTransitionRecord<S>>()
                .current()
                .map(|t| &t.entered),
        "invariant: State<S> equals the recorded `entered` after a real transition"
    );
    debug_assert!(
        matches!(world.resource::<NextState<S>>(), NextState::Unchanged),
        "invariant: NextState<S> is Unchanged after draining"
    );
}

/// Type-erased schedule-side registration for one registered state type `S`.
///
/// `apply` is the monomorphised [`apply_state_transition::<S>`] coerced to a
/// plain `fn` pointer (a safe reified-fn coercion — no `unsafe`), so the
/// `Schedule` stays non-generic over `S` and can hold a `Vec<StateEntry>` for
/// arbitrarily many state types.
pub(crate) struct StateEntry {
    /// Monomorphised transition-apply, erased to a fn pointer. Signature:
    /// `fn(&mut EcsMaster, this_run, fire_initial)`.
    pub(crate) apply: fn(&mut EcsMaster, u32, bool),
    /// `true` until the first run fires the synthesized initial `OnEnter` (D7).
    pub(crate) pending_initial: bool,
    /// Diagnostics: the state type name (`type_name::<S>()`), captured by the
    /// builder for future diagnostic surfacing (e.g. duplicate-registration or
    /// transition-trace messages). Written but not yet read in-crate this phase
    /// — the plan (§4.2) specifies it as a diagnostics slot, so the `dead_code`
    /// allow reflects "reserved diagnostics field", not a dormant code path.
    #[allow(dead_code)]
    pub(crate) type_name: &'static str,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, Copy, PartialEq, Eq, Hash)]
    enum TestState {
        A,
        B,
    }
    impl States for TestState {}

    /// Informational `size_of` on `StateTransitionRecord<TestState>` (plan §9
    /// unit: `transition_record_layout_sanity`). NOT a hard cap (§4.1) — this
    /// only documents the POD footprint and guards against an accidental
    /// blow-up (e.g. a stray `Vec`/`Box` creeping into the record). For a
    /// fieldless 2-variant enum `S`, the record is `Option<Transition<S>> +
    /// u32`, comfortably small.
    #[test]
    fn transition_record_layout_sanity() {
        let size = std::mem::size_of::<StateTransitionRecord<TestState>>();
        assert!(
            size <= 64,
            "StateTransitionRecord<TestState> should be a small POD (got {size} bytes); \
             a blow-up implies a stray heap field crept in"
        );
    }

    /// The record's default has no live transition and its `current()` accessor
    /// returns `None` — the steady-state "no transition this frame" reading.
    #[test]
    fn transition_record_default_current_is_none() {
        let rec = StateTransitionRecord::<TestState>::default();
        assert!(
            rec.current().is_none(),
            "a default (cleared) record reports no current transition"
        );
    }

    /// A record carrying a real transition exposes both endpoints through
    /// `current()`. Drives the `Transition` field reads the conditions rely on.
    #[test]
    fn transition_record_current_exposes_endpoints() {
        let rec = StateTransitionRecord::<TestState> {
            transition: Some(Transition {
                exited: Some(TestState::A),
                entered: TestState::B,
            }),
            recorded_tick: 0,
        };
        let t = rec.current().expect("a recorded transition must be visible");
        assert!(
            t.exited.as_ref() == Some(&TestState::A) && t.entered == TestState::B,
            "current() must expose the recorded exited/entered endpoints"
        );
    }

    // ── apply_state_transition direct-drive tests (the Miri-testable core) ────
    //
    // These drive the transition algorithm (§5.1) DIRECTLY on a real
    // `EcsMaster`, with NO thread pool / `Schedule::run` / `Scope::spawn`. The
    // function holds `&mut world` for its whole body (a single owned borrow), so
    // it is fully Miri-testable: this is the path the plan §9 / the tester brief
    // calls out as the one that "holds `&mut world`, no pool". They run under
    // both regular `cargo test` AND `cargo +nightly miri test --lib`.

    /// `fire_initial = true` synthesizes a `none → initial` transition: the
    /// record carries `exited: None, entered: <current>` and the state is
    /// unchanged (no `NextState` request). Validates D7 step 2.
    #[test]
    fn apply_synthesizes_initial_transition() {
        let mut world = EcsMaster::new();
        world.insert_state::<TestState>(TestState::A);

        let tick = world.current_tick().get();
        apply_state_transition::<TestState>(&mut world, tick, true);

        let rec = world.resource::<StateTransitionRecord<TestState>>();
        let t = rec.current().expect("initial transition must be recorded");
        assert!(t.exited.is_none(), "synthesized initial has no `exited`");
        assert!(t.entered == TestState::A, "synthesized initial enters the initial value");
        assert!(*world.state::<TestState>() == TestState::A, "state stays at the initial value");
    }

    /// A pending `Pending(B)` against current `A` records a real
    /// `A → B` transition, swaps `State` to `B`, and drains `NextState` to
    /// `Unchanged`. Validates D-algorithm step 3 (the real-transition branch).
    #[test]
    fn apply_real_transition_swaps_state_and_records() {
        let mut world = EcsMaster::new();
        world.insert_state::<TestState>(TestState::A);
        world.set_next_state::<TestState>(TestState::B);

        let tick = world.current_tick().get();
        apply_state_transition::<TestState>(&mut world, tick, false);

        assert!(*world.state::<TestState>() == TestState::B, "state swapped A→B");
        let rec = world.resource::<StateTransitionRecord<TestState>>();
        let t = rec.current().expect("a real transition must be recorded");
        assert!(t.exited.as_ref() == Some(&TestState::A), "exited = A");
        assert!(t.entered == TestState::B, "entered = B");
        assert!(
            matches!(world.resource::<NextState<TestState>>(), NextState::Unchanged),
            "NextState drained to Unchanged"
        );
    }

    /// A pending `Pending(A)` against current `A` (identity) records NO
    /// transition, leaves `State` at `A`, and drains `NextState`. Validates D6.
    #[test]
    fn apply_identity_transition_is_noop() {
        let mut world = EcsMaster::new();
        world.insert_state::<TestState>(TestState::A);
        world.set_next_state::<TestState>(TestState::A); // identity

        let tick = world.current_tick().get();
        apply_state_transition::<TestState>(&mut world, tick, false);

        assert!(*world.state::<TestState>() == TestState::A, "state unchanged");
        assert!(
            world
                .resource::<StateTransitionRecord<TestState>>()
                .current()
                .is_none(),
            "identity transition records nothing (D6)"
        );
        assert!(
            matches!(world.resource::<NextState<TestState>>(), NextState::Unchanged),
            "identity request still drained to Unchanged"
        );
    }

    /// With no `fire_initial` and no pending request, the pass clears the record
    /// (to `None`) and returns without touching `State`. Validates step 1.
    #[test]
    fn apply_no_request_clears_record_only() {
        let mut world = EcsMaster::new();
        world.insert_state::<TestState>(TestState::B);
        // Pre-seed a stale record to confirm step 1 clears it.
        {
            let rec = world.resource_mut::<StateTransitionRecord<TestState>>();
            rec.transition = Some(Transition {
                exited: Some(TestState::A),
                entered: TestState::B,
            });
        }

        let tick = world.current_tick().get();
        apply_state_transition::<TestState>(&mut world, tick, false);

        assert!(
            world
                .resource::<StateTransitionRecord<TestState>>()
                .current()
                .is_none(),
            "a no-request frame clears the record back to None (step 1)"
        );
        assert!(*world.state::<TestState>() == TestState::B, "state untouched");
    }

    /// `fire_initial = true` AND a pending `Pending(B)` queued before frame 1:
    /// step 2 records the initial enter, then step 3 OVERWRITES it with the real
    /// `A → B` transition and swaps the state (D7 last-write-wins on frame 1).
    /// The recorded `entered` is `B` (not the initial `A`), and `exited` is
    /// `Some(A)` — so `on_enter(initial)` would be suppressed.
    #[test]
    fn apply_initial_plus_pending_overwrites_with_real_transition() {
        let mut world = EcsMaster::new();
        world.insert_state::<TestState>(TestState::A);
        world.set_next_state::<TestState>(TestState::B);

        let tick = world.current_tick().get();
        apply_state_transition::<TestState>(&mut world, tick, true);

        assert!(*world.state::<TestState>() == TestState::B, "state swapped to the requested B");
        let rec = world.resource::<StateTransitionRecord<TestState>>();
        let t = rec.current().expect("the real transition must be recorded");
        assert!(
            t.exited.as_ref() == Some(&TestState::A) && t.entered == TestState::B,
            "the initial enter(A) is overwritten by the real A→B (D7 same-pass override)"
        );
    }
}
