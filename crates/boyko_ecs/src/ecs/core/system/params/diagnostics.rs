//! Shared cold-path panic helpers for the `Res<R>` / `ResMut<R>`
//! `SystemParam` impls.
//!
//! See Phase 8a plan §6.1 for the diagnostic shape. Both helpers are
//! `#[cold]` + `#[inline(never)]` so they live outside the hot path's
//! L1i footprint (principle 3 / I-cache discipline).

use crate::ecs::core::events::event::Event;
use crate::ecs::core::resources::resource::Resource;
use crate::ecs::core::system::filtered_access_set::AccessConflict;

/// Cold-path diagnostic when `Res<R>` / `ResMut<R>::get_param` is invoked
/// against a resource slab that has no live entry for `R`.
///
/// Constructive: names the missing type and points at the canonical fix
/// (`EcsMaster::insert_resource::<R>(...)`).
#[cold]
#[inline(never)]
pub(crate) fn missing_resource_panic<R: Resource>() -> ! {
    panic!(
        "Resource `{}` not registered. \
         Call `EcsMaster::insert_resource::<{}>(...)` before running systems that read it.",
        R::debug_type_name(),
        R::debug_type_name()
    );
}

/// Cold-path diagnostic when `Res<R>::init_access` or `ResMut<R>::init_access`
/// detects an intra-system aliasing conflict against a sibling param's
/// already-registered access.
///
/// Diagnostic code: `boyko-B0002`. Surfaces the conflicting `AccessConflict`
/// fields verbatim so the user sees both the offending pair of param names
/// and the resource id.
#[cold]
#[inline(never)]
pub(crate) fn intra_system_conflict_panic(conflict: AccessConflict) -> ! {
    panic!(
        "error[boyko-B0002]: intra-system access conflict on resource id {}.\n\
         Existing param: {}\n\
         Conflicting param: {}\n\
         Kind: {:?}\n\
         This would be UB at runtime. Remove one of the accesses or use the same mutability.",
        conflict.id, conflict.existing_param, conflict.new_param, conflict.kind
    );
}

/// Cold-path diagnostic when `EventWriter<E>` / `EventReader<E>::init_state`
/// runs before `EcsMaster::preregister_event::<E>` has been called.
///
/// Constructive: names the missing type and points at the canonical fix.
#[cold]
#[inline(never)]
pub(crate) fn event_not_preregistered_panic<E: Event>() -> ! {
    panic!(
        "Event type `{name}` (id={id}) was not preregistered on this EcsMaster. \
         Call `world.preregister_event::<{name}>(EventConfig::default_for(N))` \
         (or `preregister_event_default`) during world setup before adding \
         systems that use EventReader<{name}> / EventWriter<{name}>.",
        name = E::event_name(),
        id = E::event_id(),
    );
}
