//! The `SystemParam` trait — typed parameter slot for a system function.
//!
//! See Phase 8a plan §13 for the design (C2 + C4 + M4 resolution). This
//! module hosts only the trait definition; concrete impls live in
//! `params` and the later step files (`Res`/`ResMut` in
//! Step 7, query params in Phase 8b).
//!
//! # Why `unsafe trait`
//!
//! Implementations must uphold the SP1, SP2, SP4 invariants documented in
//! plan §10. None of those can be enforced by the type system alone:
//!
//! * **SP1** — the access surface declared in [`init_access`] is the
//!   complete and honest summary of every read/write the param will
//!   perform in [`get_param`].
//! * **SP2** — [`get_param`] is only called after the scheduler (or
//!   [`run_system_once`] in Phase 8a) has resolved aliasing against
//!   sibling params and other systems.
//! * **SP4** — [`init_state`] performs no structural mutation of the
//!   world (no archetype/resource registrations). Debug-asserted by
//!   `FunctionSystem::initialize` via `archetype_generation()`
//!   (Phase 8c Step 4).
//!
//! [`init_access`]: SystemParam::init_access
//! [`init_state`]: SystemParam::init_state
//! [`get_param`]: SystemParam::get_param
//! [`run_system_once`]: crate::ecs::core::ecs_master::ecs_master::EcsMaster

use crate::ecs::core::archetype::archetype::Archetype;
use crate::ecs::core::ecs_master::ecs_master::EcsMaster;
use crate::ecs::core::system::filtered_access_set::FilteredAccessSet;
use crate::ecs::core::system::system_meta::SystemMeta;
use crate::ecs::core::system::unsafe_ecs_cell::UnsafeEcsCell;

/// Typed parameter for a system function.
///
/// A `SystemParam` describes how to fetch a borrowed view of `EcsMaster`
/// state once per system invocation. Concrete impls cover global
/// resources (`Res`/`ResMut`, Step 7), entity queries (Phase 8b
/// `Query`), deferred mutations (Phase 8d `Commands`), per-system locals
/// (`Local<T>`), and tuples thereof (see `super::params::tuple_impl`).
///
/// # Trait shape (C2 RESOLUTION)
///
/// Every concrete impl is parameterised over the param's outer lifetime,
/// e.g. `unsafe impl<'a, R: Resource> SystemParam for Res<'a, R>`. This
/// generic blanket satisfies the [`Item<'w, 's>: SystemParam<State = ...>`](Self::Item)
/// bound for all `'w` / `'s`; no `lifetimeless::Res` shim is required.
///
/// # GAT `Item<'w, 's>` — two lifetimes (§13.1)
///
/// `'w` is the world-access scope (lives for the duration of a single
/// `get_param` call); `'s` is the state scope (lives for the duration
/// of the system's stored `State`). Phase 8b `Query<'w, 's, D, F>` needs
/// both — its `QueryState` lives in the system's state slot but its
/// returned iter borrows from the world. A single-lifetime GAT collapses
/// the two and forces redundant coercions; Bevy adopts the same shape.
///
/// # `Send + Sync + 'static` on `Self::State` (§13.3)
///
/// Required so the containing system can be migrated across worker
/// threads under the Phase 9 scheduler. All Phase 8a `State` types
/// satisfy this trivially (e.g. `ResState<R>` is `Copy + Send + Sync`).
///
/// # Safety
///
/// Implementations MUST uphold:
///
/// 1. **SP1** — every read/write that [`get_param`](Self::get_param) will
///    perform on `world` is declared via the [`FilteredAccessSet`]
///    accumulator passed to [`init_access`](Self::init_access).
/// 2. **SP2** — [`get_param`](Self::get_param) assumes the caller has
///    already resolved aliasing per the declared access; calling it in
///    violation of that contract is UB.
/// 3. **SP4** — [`init_state`](Self::init_state) performs no structural
///    mutation of the world.
///
pub unsafe trait SystemParam: Sized {
    /// Whether this param can enqueue work for [`apply`](Self::apply).
    ///
    /// `true` iff the param's `apply` may mutate the world. Today exactly one
    /// impl in the tree answers `true` — [`Commands`], the only one that
    /// overrides `apply` with a body; every other param inherits the trait's
    /// no-op and answers `false`. A tuple ORs its members.
    ///
    /// # What reads it (KE17 D3)
    ///
    /// [`FunctionSystem::has_deferred`] forwards this const, and
    /// `ScheduleBuilder::try_build` folds it into the schedule's `may_defer`
    /// bitset. The apply-window barrier exists only to give a successor
    /// visibility of its predecessor's deferred commands
    /// (`schedule_builder.rs`'s sync-point doc block states this in writing),
    /// so a system whose whole param chain answers `false` carries nothing the
    /// barrier is protecting and can be retired the instant it finishes. The
    /// measured cost of not knowing this is 8.1 % of a frame on the engine's
    /// own Main shape — see `docs/scheduler/KE17-APPLY-WINDOW-MEASUREMENT.md`.
    ///
    /// # Why no default
    ///
    /// The same reasoning
    /// [`System::set_change_ticks`](super::system::System::set_change_ticks)
    /// gives for having no default body, at the point where it decides
    /// soundness rather than freshness. A param whose `apply` does real work
    /// while silently declaring `false` would let a successor observe the
    /// world before that work landed — the exact silent-wrong-read class the
    /// barrier is there to prevent. A defaulted const makes that omission
    /// compile; a required one makes it `E0046`. The cost is one line per
    /// impl.
    ///
    /// [`Commands`]: super::params::commands::Commands
    /// [`FunctionSystem::has_deferred`]: super::function_system::FunctionSystem
    const HAS_DEFERRED: bool;

    /// Long-lived state owned by the containing system.
    ///
    /// `Send + Sync + 'static` so the containing system can migrate
    /// across worker threads under the Phase 9 scheduler. See §13.3.
    type State: Send + Sync + 'static;

    /// The borrowed view delivered to the system body per run.
    ///
    /// `'w` is the world-access scope, `'s` is the state scope (see
    /// §13.1). The `SystemParam<State = Self::State>` bound makes tuples
    /// of params nest cleanly — `(A, B)::Item<'w, 's>` is
    /// `(A::Item<'w, 's>, B::Item<'w, 's>)` which itself implements
    /// `SystemParam` via the tuple-impl macro.
    type Item<'w, 's>: SystemParam<State = Self::State>;

    /// Initialises the per-system state.
    ///
    /// Called once during system construction, after the param tuple is
    /// known but before any `get_param` invocation. The `&mut EcsMaster`
    /// is provided for state types that need to pre-allocate (e.g. Phase
    /// 8b `QueryState` matches existing archetypes during construction).
    ///
    /// # Safety contract (SP4)
    ///
    /// Implementations MUST NOT mutate the world's structural shape — no
    /// new archetype or resource registrations may be performed here.
    /// Debug-asserted via `archetype_generation()` comparison in
    /// `FunctionSystem::initialize` (Phase 8c Step 4).
    fn init_state(world: &mut EcsMaster, system_meta: &mut SystemMeta) -> Self::State;

    /// Declares this param's access surface.
    ///
    /// Called once per system after [`init_state`](Self::init_state),
    /// before any [`get_param`](Self::get_param) call. Implementations
    /// MUST declare every read and write they will perform via
    /// `access_set.add_resource_read/write/component_read/write`. An
    /// `Err` returned from one of those calls indicates an intra-system
    /// conflict with a sibling param and MUST be turned into a panic
    /// with the B0002 diagnostic (see [`AccessConflict`]).
    ///
    /// Implementations with no access — e.g. `Local<T>`, `Commands`,
    /// `EventReader` — provide an explicit empty body (this method has **no**
    /// default impl).
    ///
    /// [`AccessConflict`]: super::filtered_access_set::AccessConflict
    fn init_access(
        state: &Self::State,
        system_meta: &mut SystemMeta,
        access_set: &mut FilteredAccessSet,
        world: &mut EcsMaster,
    );

    /// Produces the per-invocation [`Item<'w, 's>`](Self::Item).
    ///
    /// # Safety
    ///
    /// SP1 + SP2. The caller asserts that:
    ///
    /// * The [`Access`](super::access::Access) declared during
    ///   [`init_access`](Self::init_access) is the complete and honest
    ///   summary of this param's reads and writes (SP1).
    /// * Either the Phase 9 scheduler or
    ///   [`EcsMaster::run_system_once`] (Phase 8a) has resolved
    ///   aliasing such that no other reference produced by a sibling
    ///   param or other system aliases the borrow this call mints (SP2).
    ///
    /// Violating either invariant is UB.
    ///
    /// [`EcsMaster::run_system_once`]: crate::ecs::core::ecs_master::ecs_master::EcsMaster
    unsafe fn get_param<'w, 's>(
        state: &'s mut Self::State,
        system_meta: &SystemMeta,
        world: UnsafeEcsCell<'w>,
    ) -> Self::Item<'w, 's>;

    /// Hook for deferred mutations after the system body returns.
    ///
    /// Phase 8d `Commands` overrides this to drain its command queue.
    /// Default no-op.
    ///
    /// An impl that overrides this with a non-empty body MUST declare
    /// [`HAS_DEFERRED = true`](Self::HAS_DEFERRED); the two are one statement
    /// wearing two spellings, and only the const is machine-readable.
    #[inline]
    fn apply(_state: &mut Self::State, _system_meta: &SystemMeta, _world: &mut EcsMaster) {}

    /// Hook called by Phase 8b `Query` when a new archetype is added
    /// after `init_state`. Default no-op.
    #[inline]
    fn new_archetype(
        _state: &mut Self::State,
        _system_meta: &mut SystemMeta,
        _archetype: &Archetype,
    ) {
    }
}
