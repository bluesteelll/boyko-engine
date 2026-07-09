//! The [`States`] marker trait (Phase 17 D1).

/// Marker trait for application/game state types.
///
/// A `States` type is the value carried by a [`State<S>`] resource and queued
/// in a [`NextState<S>`] resource. It is typically a fieldless `enum`:
///
/// ```ignore
/// #[derive(Clone, Copy, PartialEq, Eq, Hash, Default)]
/// enum AppState { #[default] Menu, InGame, Paused }
/// impl States for AppState {}
/// ```
///
/// # Bounds
///
/// * `PartialEq + Eq` — the transition pass compares the queued value against
///   the current one (`next != current`).
/// * `Clone` — the value is moved `Pending(S) → State<S>` and captured by value
///   into the `in_state`/`on_enter`/`on_exit`/`on_transition` condition
///   closures.
/// * `Hash` — reserved for the deferred computed/sub-state map keys (§11);
///   zero-cost here (never hashed this phase), kept so adding them later is
///   non-breaking. Mirrors Bevy's `States: Hash`.
/// * `Send + Sync + 'static` — `State<S>`/`NextState<S>` are [`Resource`]s
///   carried across the parallel scheduler's workers.
///
/// # No derive
///
/// There is intentionally no `#[derive(States)]`: a plain state type
/// enumerates nothing, so the derive would add only a bound check with no
/// codegen. Hand-write `impl States for MyEnum {}`. A future derive would
/// emit exactly that, additively.
///
/// [`State<S>`]: crate::ecs::core::state::state::State
/// [`NextState<S>`]: crate::ecs::core::state::next_state::NextState
/// [`Resource`]: crate::ecs::core::resources::resource::Resource
pub trait States: Send + Sync + Sized + Clone + PartialEq + Eq + std::hash::Hash + 'static {}
