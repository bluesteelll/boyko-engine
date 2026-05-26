//! [`SystemSet`] — named group of systems for ordering / membership rules.
//!
//! See Phase 9 plan §5.7 for the trait + id shape and §3 Q9 for the
//! ordering semantics. Wave 4 Step 9 ships only the surface the builder
//! needs to ingest `.in_set(...)` calls; full set-expansion (cross-set
//! ordering, hierarchical inclusion) lands with the auto-sync-point
//! analyzer in Wave 5 Step 14.
//!
//! # `'static + Hash + Eq` rationale
//!
//! Sets are stored in a `HashMap<TypeId, SystemSetId>` inside
//! [`ScheduleBuilder`] keyed by the user's set type. The trait bound
//! collapses to plain `Send + Sync + 'static` because the builder uses
//! [`TypeId::of::<S>()`] for identity — no `Hash` on the user type is
//! required at the ECS layer (the user-facing derive macro lands in
//! Wave 7 Step 21).
//!
//! [`ScheduleBuilder`]: super::schedule_builder::ScheduleBuilder
//! [`TypeId::of::<S>()`]: std::any::TypeId::of

/// Stable handle assigned to each [`SystemSet`] the builder encounters.
///
/// The wrapped `usize` is allocated sequentially by
/// [`ScheduleBuilder::set_id_of`] (Wave 4 Step 9) keyed by the user set's
/// `TypeId`; two `.in_set(MySet)` calls on the same set return the same
/// id. Phase 9 §5.7.
///
/// [`ScheduleBuilder::set_id_of`]: super::schedule_builder::ScheduleBuilder::set_id_of
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
#[repr(transparent)]
pub struct SystemSetId(pub usize);

/// Marker trait for user-defined system grouping types.
///
/// Anything `Send + Sync + 'static` is acceptable; the builder identifies
/// sets by [`TypeId`] rather than by trait-method, so the trait carries
/// no methods today. Wave 7 Step 21 introduces a `#[derive(SystemSet)]`
/// proc-macro that asserts the trait at the user's type definition site;
/// until then users implement it manually for their unit-struct set
/// markers.
///
/// # Example
///
/// ```ignore
/// struct PhysicsSet;
/// impl SystemSet for PhysicsSet {}
///
/// // Later, in the builder:
/// builder.add_system(integrate).in_set(PhysicsSet);
/// ```
///
/// [`TypeId`]: std::any::TypeId
pub trait SystemSet: Send + Sync + 'static {}

#[cfg(test)]
mod tests {
    use super::*;

    /// `SystemSetId` round-trips through equality / hashing — the builder
    /// relies on this for its `HashMap<TypeId, SystemSetId>` keying.
    #[test]
    fn system_set_id_equality_and_hash() {
        let a = SystemSetId(0);
        let b = SystemSetId(0);
        let c = SystemSetId(1);
        assert_eq!(a, b);
        assert_ne!(a, c);

        use std::collections::HashSet;
        let mut set = HashSet::new();
        set.insert(a);
        assert!(set.contains(&b));
        assert!(!set.contains(&c));
    }

    /// Concrete `SystemSet` implementations are accepted as long as the
    /// `Send + Sync + 'static` bound is satisfied. Compile-only check.
    #[test]
    fn concrete_system_set_is_acceptable() {
        struct PhysicsSet;
        impl SystemSet for PhysicsSet {}

        fn assert_impl<S: SystemSet>() {
            let _ = std::marker::PhantomData::<S>;
        }
        assert_impl::<PhysicsSet>();
    }
}
