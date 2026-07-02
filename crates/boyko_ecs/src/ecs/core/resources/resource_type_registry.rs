//! First-class kernel registry that interns a `TypeId → ResourceId` mapping for
//! **generic** resource types whose `resource_id()` body is itself generic.
//!
//! # Who uses this
//!
//! Any resource whose `Resource::resource_id()` is implemented on a *generic*
//! type reaches its stable [`ResourceId`] through [`resource_id_for`]:
//!
//! * `State<S>` / `NextState<S>` / `StateTransitionRecord<S>` — the Phase 17
//!   state resources (`crate::ecs::core::state`).
//! * `ActionState<A>` / `InputMap<A>` — the `boyko_input` action resources.
//!
//! Non-generic resources keep using `#[derive(Resource)]`, which caches the id
//! in a per-type `static OnceLock<ResourceId>` inside a *monomorphic* body —
//! that is sound because the body is one-per-concrete-type. This registry is the
//! canonical replacement for the derive **only on the generic path**, where that
//! `static` idiom is unsound (see below).
//!
//! # Why a `TypeId → ResourceId` HashMap instead of a per-impl `static SLOT`
//!
//! `#[derive(Resource)]` (and the equivalent hand-rolled idiom) caches a
//! type's `ResourceId` in a `static ID: OnceLock<ResourceId>` declared inside
//! the `resource_id()` body. That works for a **non-generic** body (one body
//! per concrete type), but it is **unsound inside a generic `resource_id()`**:
//! per [rust-lang/rust#22991](https://github.com/rust-lang/rust/issues/22991)
//! and [rust-lang/rfcs#2130](https://github.com/rust-lang/rfcs/pull/2130),
//! a `static` declared in a generic function is NOT monomorphised — every
//! instantiation shares one static. Consequence for `State<S>`: every distinct
//! `S` would collapse to the SAME `ResourceId`, so `State<AppState>` and
//! `State<MenuState>` would silently alias one resource slot — reinterpreting
//! the bytes of the wrong type (UB / heap corruption). The same trap collapses
//! `ActionState<GameplayAction>` and `ActionState<MenuAction>`.
//!
//! This is exactly the trap the query-type registry
//! (`crate::ecs::core::iters::query::query_type_registry`) already solved for
//! `(D, F)` pairs. We reuse that proven pattern verbatim: a process-global
//! `OnceLock<Mutex<HashMap<TypeId, ResourceId>>>` keyed by
//! `TypeId::of::<T>()`, where `T` is the concrete generic resource.
//!
//! # Cost
//!
//! Paid at most once per concrete `T` per process, on the cold registration
//! path (a `Mutex::lock` + `HashMap` probe + a single
//! [`resource_registry::register_new`] mint). Never on the steady-state hot
//! path: `Res<T>::get_param` caches the resolved `ResourceId` in `ResState<T>`
//! at init, so every per-frame read goes through the cached id with zero map
//! traffic.

use std::any::TypeId;
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use crate::ecs::core::resources::resource::Resource;
use crate::ecs::core::resources::resource_registry;
use crate::ecs::identifiers::primitives::ResourceId;

/// Process-global registry mapping `TypeId::of::<T>()` to the `ResourceId`
/// minted for the generic resource `T`.
///
/// Replaces the per-impl `static SLOT` pattern, which collapses across
/// monomorphisations inside a generic `resource_id()` body (see the module
/// doc-comment for the rust#22991 / rfcs#2130 rationale).
static REGISTRY: OnceLock<Mutex<HashMap<TypeId, ResourceId>>> = OnceLock::new();

/// Returns the process-global [`ResourceId`] for the generic resource `T`,
/// minting it on first call.
///
/// The get-or-mint is atomic under the registry `Mutex`: the map is probed,
/// a fresh id minted via [`resource_registry::register_new`] **only if
/// absent**, inserted, and returned — all inside one lock. Concurrent callers
/// for the same `T` therefore observe the same id (the second caller blocks on
/// the lock, then finds the entry the first caller inserted).
///
/// `register_new::<T>()` does not re-enter `T::resource_id()` — it mints from
/// the global `NEXT_RESOURCE_ID` counter and stores `ResourceInfo::new_static`
/// — so there is no recursion through this function.
///
/// All generic resources (state `State<S>` / `NextState<S>` /
/// `StateTransitionRecord<S>`, input `ActionState<A>` / `InputMap<A>`) share
/// this one map, so their ids come from a single `TypeId`-keyed space over the
/// global resource-id counter — distinct `TypeId`s always mint distinct ids.
///
/// # Panics
///
/// Propagates [`resource_registry::register_new`]'s panics (resource-slab
/// exhaustion at `RESOURCE_SLOT_COUNT`, or a Component/Resource clash for `T`).
/// Panics if the registry `Mutex` is poisoned by a previous panicking caller.
pub fn resource_id_for<T: Resource>() -> ResourceId {
    let registry = REGISTRY.get_or_init(|| Mutex::new(HashMap::new()));
    let mut map = registry
        .lock()
        .expect("invariant: resource type registry mutex poisoned");
    let key = TypeId::of::<T>();
    if let Some(&id) = map.get(&key) {
        return id;
    }
    // `register_new` mints from `NEXT_RESOURCE_ID` and stores
    // `ResourceInfo::new_static::<T>()`; it does not call `T::resource_id()`,
    // so this is not re-entrant.
    let id = ResourceId::new(resource_registry::register_new::<T>());
    map.insert(key, id);
    id
}

#[cfg(test)]
mod tests {
    use crate::ecs::core::resources::resource::Resource;
    use crate::ecs::core::state::next_state::NextState;
    use crate::ecs::core::state::state::State;
    use crate::ecs::core::state::states::States;
    use crate::ecs::core::state::transition_record::StateTransitionRecord;

    // Two DISTINCT state types. The whole point of the rust#22991 guard is that
    // a generic-body `static` would collapse `State<A>` and `State<B>` onto one
    // `ResourceId` — these two types must mint different ids. The variants are
    // never *constructed* (the test only references the types at the type level,
    // via `State::<StateA>::resource_id()`), hence `#[allow(dead_code)]`.
    #[derive(Clone, Copy, PartialEq, Eq, Hash)]
    #[allow(dead_code)]
    enum StateA {
        X,
    }
    impl States for StateA {}

    #[derive(Clone, Copy, PartialEq, Eq, Hash)]
    #[allow(dead_code)]
    enum StateB {
        Y,
    }
    impl States for StateB {}

    /// THE rust#22991 regression guard (plan §9 unit:
    /// `state_resource_ids_distinct_per_type`).
    ///
    /// A `static ID: OnceLock<ResourceId>` inside the generic `resource_id()`
    /// body would NOT be monomorphised — every instantiation would share one
    /// static, collapsing `State<StateA>` and `State<StateB>` onto the SAME id
    /// (silent aliasing of two resource slots). The `TypeId`-keyed registry
    /// prevents that. This test FAILS if the collapsing static is ever
    /// reintroduced.
    ///
    /// It asserts three independence axes:
    /// 1. `State<A>` vs `State<B>` — distinct `S` ⇒ distinct ids (the core
    ///    rust#22991 case).
    /// 2. `State<A>` vs `NextState<A>` — the two resources for one `S` are
    ///    distinct slots (they must never alias).
    /// 3. `State<A>` vs `StateTransitionRecord<A>` — all three per-`S`
    ///    resources occupy distinct slots.
    #[test]
    fn state_resource_ids_distinct_per_type() {
        // Axis 1: distinct state types ⇒ distinct ids (the rust#22991 case).
        assert!(
            State::<StateA>::resource_id() != State::<StateB>::resource_id(),
            "State<StateA> and State<StateB> must mint DISTINCT ResourceIds \
             (rust#22991 collapse guard: a generic-body `static` would alias them)"
        );

        // Axis 2: State<A> vs NextState<A> — distinct resources for one S.
        assert!(
            State::<StateA>::resource_id() != NextState::<StateA>::resource_id(),
            "State<StateA> and NextState<StateA> must occupy DISTINCT slots"
        );

        // Axis 3: State<A> vs StateTransitionRecord<A>.
        assert!(
            State::<StateA>::resource_id()
                != StateTransitionRecord::<StateA>::resource_id(),
            "State<StateA> and StateTransitionRecord<StateA> must occupy DISTINCT slots"
        );

        // Bonus: the other two pairings for one S are also distinct (full
        // 3-way disjointness, the property D4 relies on).
        assert!(
            NextState::<StateA>::resource_id()
                != StateTransitionRecord::<StateA>::resource_id(),
            "NextState<StateA> and StateTransitionRecord<StateA> must occupy DISTINCT slots"
        );
    }

    /// `resource_id()` is stable per type across repeated calls — the registry
    /// caches and returns the same id (idempotent minting). A flaky/non-cached
    /// implementation would mint a fresh id each call and exhaust the slab.
    #[test]
    fn state_resource_id_stable_across_calls() {
        let first = State::<StateA>::resource_id();
        let second = State::<StateA>::resource_id();
        assert!(
            first == second,
            "resource_id() must return the same id on every call for a given type"
        );
    }
}
