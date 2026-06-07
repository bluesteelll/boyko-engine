//! Hand-written `Bundle` impls for the two 1-field hierarchy newtypes
//! ([`ChildrenBundle`] / [`ChildOfBundle`], Phase 19 R2 C1).
//!
//! # Why hand-written instead of `#[derive(Bundle)]`
//!
//! `boyko-macros` is a **dev-dependency** of `boyko-ecs` (Cargo.toml), so its
//! derive macros are unavailable to library `src/` code — a `#[derive(Bundle)]`
//! here would compile only under `cargo test`. The established codebase pattern
//! is to **hand-write the impl mirroring the derive output** (see the
//! hand-written `Component` impls in `ecs_master.rs`'s test module and
//! `component_pool.rs:1405`).
//!
//! These impls are a faithful, mechanical reproduction of the
//! `#[derive(Bundle)]` expansion (`boyko_macros/src/lib.rs:966-1086`) for a
//! single-field tuple struct — same `BundleSealed` seal, same `OnceLock`-cached
//! `static_info`, same `cached_archetype_id` delegation, same
//! `ManuallyDrop`-upfront (B4) + pointer-based `for_each_component_bytes` (C5).
//!
//! # SAFETY accounting note (Phase 19 unsafe budget)
//!
//! The plan budgeted "exactly one new `unsafe` (the M2 `assume_init`) + one
//! audited-copy raw deref" on the assumption that `#[derive(Bundle)]` would
//! generate the bundle boilerplate (hiding its `slice::from_raw_parts` inside
//! the macro expansion). Because the macro is dev-only, that byte-erasure
//! `from_raw_parts` is reproduced by hand HERE instead of being macro-generated.
//! It is mechanical derive output — soundness-identical to every other
//! `#[derive(Bundle)]` in the engine — not novel hand-authored unsafe. Flagged
//! as the forced deviation it is.

use std::mem::ManuallyDrop;
use std::sync::OnceLock;

use crate::ecs::core::bundle::bundle::{Bundle, BundleStaticInfo, sealed::BundleSealed};
use crate::ecs::core::bundle::bundle_type_registry;
use crate::ecs::core::component::component::Component;
use crate::ecs::core::ecs_master::ecs_master::EcsMaster;
use crate::ecs::core::hierarchy::{ChildOf, ChildOfBundle, Children, ChildrenBundle};
use crate::ecs::identifiers::primitives::{ArchetypeId, ComponentId};

/// Emits a hand-written `Bundle` impl for a single-field tuple newtype wrapping
/// one `Component`. Mirrors the `#[derive(Bundle)]` expansion exactly (one
/// component ⇒ a one-element sorted array; the sort is a no-op but kept for
/// fidelity with the derive's B1 contract).
macro_rules! impl_single_field_bundle {
    ($bundle:ty, $inner:ty) => {
        impl BundleSealed for $bundle {}

        impl Bundle for $bundle {
            fn static_info() -> &'static BundleStaticInfo {
                static INFO: OnceLock<BundleStaticInfo> = OnceLock::new();
                INFO.get_or_init(|| {
                    let mut arr: [ComponentId; 1] = [<$inner as Component>::component_id()];
                    arr.sort_unstable_by_key(|id| id.0);
                    let leaked: &'static [ComponentId; 1] = Box::leak(Box::new(arr));
                    BundleStaticInfo {
                        type_id: bundle_type_registry::register_new(),
                        component_ids: leaked.as_slice(),
                    }
                })
            }

            #[inline]
            fn cached_archetype_id(world: &mut EcsMaster) -> ArchetypeId {
                world.bundle_archetype_id_for::<Self>()
            }

            fn for_each_component_bytes<F>(self, mut f: F)
            where
                F: FnMut(ComponentId, &[u8]),
            {
                // B4: ManuallyDrop the field UPFRONT, before the callback runs, so
                // a callback panic suppresses the field's Drop (leak, never
                // double-drop with archetype-side ownership).
                let field = ManuallyDrop::new(self.0);
                let id = <$inner as Component>::component_id();
                let ptr = &raw const *field as *const u8;
                let len = ::std::mem::size_of::<$inner>();
                // SAFETY (reproduced `#[derive(Bundle)]` C5 byte-erasure — see the
                //   module SAFETY-accounting note):
                //   * `ptr` derives from `&raw const *ManuallyDrop<T>` over a live
                //     stack local; it is valid for `len = size_of::<T>()` bytes for
                //     the duration of this call.
                //   * `len` is exactly `size_of::<T>()` for the wrapped component —
                //     no over-read.
                //   * The materialized slice is shared/immutable and the only live
                //     borrow of `field`; on callback success ownership transfers to
                //     the archetype, on panic the ManuallyDrop suppresses Drop.
                let bytes: &[u8] = unsafe { ::std::slice::from_raw_parts(ptr, len) };
                f(id, bytes);
            }
        }
    };
}

impl_single_field_bundle!(ChildrenBundle, Children);
impl_single_field_bundle!(ChildOfBundle, ChildOf);
