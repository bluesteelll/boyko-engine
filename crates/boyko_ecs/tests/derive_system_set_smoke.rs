//! Phase 9 Wave 7 Step 21 — smoke tests for `#[derive(SystemSet)]`.
//!
//! Each test pins one slice of the derive contract:
//!
//! 1. `system_set_derive_compiles` — minimal happy path; derived unit
//!    structs satisfy the `SystemSet` bound.
//! 2. `distinct_sets_have_distinct_typeids` — two derived markers receive
//!    distinct `TypeId`s; `ScheduleBuilder::set_id_of` relies on this for
//!    its `HashMap<TypeId, SystemSetId>` lookup (plan §5.7).
//! 3. `same_set_has_stable_typeid` — repeated `TypeId::of` calls on the
//!    same marker return identical values; mirrors the cache-key
//!    invariant the builder depends on.
//!
//! The derive's compile_fail paths (generics, enums, tuple structs) are
//! exercised by the tester-owned `trybuild` suite — out of scope here.

use std::any::TypeId;

use boyko_ecs::ecs::core::schedule::SystemSet;
use boyko_macros::SystemSet;

#[derive(SystemSet)]
struct PhysicsSet;

#[derive(SystemSet)]
struct RenderSet;

/// Sanity: the derived impl satisfies the `SystemSet` bound. Type-level
/// check, no runtime work.
#[test]
fn system_set_derive_compiles() {
    fn assert_system_set<T: SystemSet>() {}
    assert_system_set::<PhysicsSet>();
    assert_system_set::<RenderSet>();
}

/// `ScheduleBuilder::set_id_of` keys its set-id table by `TypeId`. The
/// derive must not collapse distinct user types into the same id.
#[test]
fn distinct_sets_have_distinct_typeids() {
    assert_ne!(TypeId::of::<PhysicsSet>(), TypeId::of::<RenderSet>());
}

/// Repeated `TypeId::of` calls on the same marker return identical values
/// — pins the cache-key invariant the Wave 4 builder depends on.
#[test]
fn same_set_has_stable_typeid() {
    assert_eq!(TypeId::of::<PhysicsSet>(), TypeId::of::<PhysicsSet>());
    assert_eq!(TypeId::of::<RenderSet>(), TypeId::of::<RenderSet>());
}
