//! [`SystemSet`] — named group of systems for ordering / membership rules.
//!
//! See Phase 9 plan §5.7 for the trait + id shape and §3 Q9 for the
//! ordering semantics. Wave 4 Step 9 ships only the surface the builder
//! needs to ingest `.in_set(...)` calls; full set-expansion (cross-set
//! ordering, hierarchical inclusion) lands with the auto-sync-point
//! analyzer in Wave 5 Step 14.
//!
//! # `'static` rationale
//!
//! Sets are stored in a `HashMap<(TypeId, u32), SystemSetId>` inside
//! [`ScheduleBuilder`] keyed by the user's set type plus its
//! `set_discriminant()`. The trait bound collapses to plain
//! `Send + Sync + 'static` because the builder uses [`TypeId::of::<S>()`]
//! (and the value's discriminant) for identity — no `Hash` on the user
//! type is required at the ECS layer.
//!
//! [`ScheduleBuilder`]: super::schedule_builder::ScheduleBuilder
//! [`TypeId::of::<S>()`]: std::any::TypeId::of

/// Stable handle assigned to each [`SystemSet`] the builder encounters.
///
/// The wrapped `usize` is allocated sequentially by
/// `ScheduleBuilder::set_id_of_value` keyed by the pair
/// `(TypeId::of::<S>(), set.set_discriminant())`; two `.in_set(MySet)`
/// calls on the same set (or the same enum variant) return the same id.
/// Phase 9 §5.7, Phase 15 §13.1 R3-A.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
#[repr(transparent)]
pub struct SystemSetId(pub usize);

/// Marker trait for user-defined system grouping types.
///
/// Anything `Send + Sync + 'static` is acceptable; the builder identifies
/// sets by the key `(TypeId::of::<Self>(), set_discriminant(self))` and
/// interns that key into a sequential [`SystemSetId`]. Unit-struct sets
/// share the default discriminant `0`, so each distinct type is one set;
/// `#[derive(SystemSet)]` on an enum overrides [`set_discriminant`] so
/// each variant becomes a distinct set.
///
/// Both methods are **defaulted**, so hand-written `impl SystemSet for X {}`
/// keeps compiling unchanged (Phase 15 R10).
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
/// [`set_discriminant`]: SystemSet::set_discriminant
pub trait SystemSet: Send + Sync + 'static {
    /// Distinguishes variants of an enum set. Unit-struct sets use the
    /// default (`0`); `#[derive(SystemSet)]` on an enum overrides this to
    /// return the variant index. Set identity is the pair
    /// `(TypeId::of::<Self>(), set_discriminant(self))`.
    #[inline]
    fn set_discriminant(&self) -> u32 {
        0
    }

    /// Human-readable name for diagnostics (cycle / empty-set messages).
    /// Defaults to the fully-qualified type name; the enum derive overrides
    /// it to `"Type::Variant"` so variants of one enum stay distinguishable.
    #[inline]
    fn set_name(&self) -> &'static str {
        core::any::type_name::<Self>()
    }
}

#[cfg(test)]
mod tests {
    // Test oracle model: a std `HashSet` is the REFERENCE hash/eq consumer the
    // `SystemSetId` derive is verified against (the builder's set-intern map has
    // the same requirement). Compiled out of every shipping build.
    #![allow(clippy::disallowed_types)]

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
