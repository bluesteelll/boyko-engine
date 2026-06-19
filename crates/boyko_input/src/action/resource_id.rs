//! Process-global `ResourceId` minting for the **generic** input resources
//! `ActionState<A>` / `InputMap<A>` (plan §7.1, resolves the C1 blocker).
//!
//! # Why not `#[derive(Resource)]`
//!
//! `#[derive(Resource)]` caches a type's `ResourceId` in a
//! `static ID: OnceLock<ResourceId>` declared **inside** the `resource_id()`
//! body. That is sound for a non-generic body (one body per concrete type), but
//! it is **unsound inside a generic `resource_id()`**: per
//! [rust-lang/rust#22991](https://github.com/rust-lang/rust/issues/22991), a
//! `static` declared in a generic function is NOT monomorphised — every
//! instantiation shares one static. Consequence: `ActionState<GameplayAction>`
//! and `ActionState<MenuAction>` would collapse onto the **same** `ResourceId`,
//! reinterpreting bytes of the wrong type — UB / heap corruption. The headline
//! "two independent resources for free" is false under the derive.
//!
//! # Resolution
//!
//! Mirror the proven `TypeId → ResourceId` registry the engine already uses for
//! its own generic resources (`State<S>` / `NextState<S>`, see
//! `boyko_ecs … state::state_resource_registry`): a process-global
//! `OnceLock<Mutex<HashMap<TypeId, ResourceId>>>` keyed by `TypeId::of::<T>()`,
//! minting through the public `register_new` re-export. `ActionState<A>` and
//! `InputMap<A>` hand-implement `Resource` (NOT the derive) and delegate to
//! [`id_for`].
//!
//! `boyko_ecs` keeps `state_resource_registry::resource_id_for` `pub(crate)`, so
//! `boyko_input` cannot reuse it directly; this is the documented
//! zero-`boyko_ecs`-change duplication path (plan §10 / §17 Q1). The only
//! cross-crate entry point used is the public
//! `boyko_ecs::ecs::core::resources::register_new` re-export — the same one
//! `#[derive(Resource)]` emits.
//!
//! # Cost
//!
//! Paid at most once per concrete `T` per process, on the cold registration
//! path (a `Mutex::lock` + `HashMap` probe + one `register_new` mint). Never on
//! the steady-state hot path: `Res<ActionState<A>>::get_param` caches the
//! resolved id in its `ResState` at init, so per-frame reads never touch the
//! map.

use std::any::TypeId;
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use boyko_ecs::ecs::core::resources::register_new;
use boyko_ecs::ecs::core::resources::resource::Resource;
use boyko_ecs::ecs::identifiers::primitives::ResourceId;

/// Process-global registry mapping `TypeId::of::<T>()` to the `ResourceId`
/// minted for the generic input resource `T`.
///
/// Replaces the per-impl `static SLOT` pattern, which collapses across
/// monomorphisations inside a generic `resource_id()` body (see the module
/// doc-comment for the rust#22991 rationale).
static REGISTRY: OnceLock<Mutex<HashMap<TypeId, ResourceId>>> = OnceLock::new();

/// Returns the process-global [`ResourceId`] for the generic input resource
/// `T`, minting it on first call.
///
/// The get-or-mint is atomic under the registry `Mutex`: the map is probed, a
/// fresh id minted via [`register_new`] **only if absent**, inserted, and
/// returned — all inside one lock. Concurrent callers for the same `T`
/// therefore observe the same id (the second caller blocks on the lock, then
/// finds the entry the first caller inserted).
///
/// `register_new::<T>()` mints from the global resource-id counter and stores
/// the type's `ResourceInfo`; it does not re-enter `T::resource_id()`, so there
/// is no recursion through this function.
///
/// # Panics
///
/// Propagates [`register_new`]'s panics (resource-slab exhaustion, or a
/// Component/Resource clash for `T`). Panics if the registry `Mutex` is poisoned
/// by a previous panicking caller.
pub fn id_for<T: Resource>() -> ResourceId {
    let registry = REGISTRY.get_or_init(|| Mutex::new(HashMap::new()));
    let mut map = registry
        .lock()
        .expect("invariant: boyko_input resource registry mutex poisoned");
    let key = TypeId::of::<T>();
    if let Some(&id) = map.get(&key) {
        return id;
    }
    // `register_new` mints a fresh raw id and records `T`'s `ResourceInfo`; it
    // does not call `T::resource_id()`, so this is not re-entrant.
    let id = ResourceId::new(register_new::<T>());
    map.insert(key, id);
    id
}
