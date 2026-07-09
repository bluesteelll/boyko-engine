//! The opt-in [`StateTransitionSet`] ordering hook (Phase 17 D10).

use crate::ecs::core::schedule::system_set::SystemSet;

/// A unit [`SystemSet`] users MAY opt into to order their enter/exit systems
/// relative to other systems (Phase 17 D10).
///
/// It is **not** auto-wired: the transition pass already runs before the
/// executor loop, so every `on_enter`/`on_exit`-gated system observes the
/// correct transition record regardless of its topological position — a
/// built-in ordering set is not needed for correctness. This marker exists
/// purely as the building block for the *relative* order of state systems vs
/// ordinary systems, expressed through the Phase-15 `.before`/`.after`/
/// `.in_set` machinery:
///
/// ```ignore
/// builder
///     .add_system(spawn_level)
///     .run_if(on_enter(AppState::InGame))
///     .in_set(StateTransitionSet);
/// builder.configure_set(StateTransitionSet).before(GameplaySet);
/// ```
///
/// Zero cost if unused. Auto-wiring it (forcing all enter/exit systems before
/// everything) is deliberately deferred — it would duplicate Phase 15, impose
/// a global ordering edge, and perturb every state-using schedule's conflict
/// graph.
///
/// [`SystemSet`]: crate::ecs::core::schedule::system_set::SystemSet
pub struct StateTransitionSet;

impl SystemSet for StateTransitionSet {}
