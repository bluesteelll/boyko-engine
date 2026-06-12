//! In-crate single-component `Bundle` machinery (Phase 22 D5/D7).
//!
//! Two items live here:
//!
//! * [`impl_self_bundle!`] — the hand-written, in-crate mirror of the
//!   `#[derive(Component)]` single-component `Bundle` emission
//!   (`boyko_macros/src/lib.rs::component_self_bundle_codegen`), for engine
//!   components defined in library `src/` where the derive is unavailable
//!   (`boyko-macros` is a dev-dependency). Applied to `ChildOf` / `Children`
//!   in `hierarchy/bundles.rs`; the Phase-19 `ChildOfBundle` /
//!   `ChildrenBundle` newtypes are deleted.
//! * [`EmptyBundle`] — the zero-component `Bundle` backing
//!   `Commands::spawn_empty` (plan D5). Hand-written, `pub(crate)`, **zero
//!   unsafe** (there are no bytes to erase); `derive(Bundle)` deliberately
//!   keeps its ≥1-field rule for users.
//!
//! # SAFETY accounting note (Phase 19 precedent, plan unsafe table)
//!
//! The byte-erasure `slice::from_raw_parts` inside `impl_self_bundle!` is a
//! mechanical reproduction of the derive output — soundness-identical to
//! every `#[derive(Component)]`-emitted Bundle in downstream crates — not
//! novel hand-authored unsafe. Keep the macro body in lock-step with
//! `component_self_bundle_codegen`.

use std::sync::OnceLock;

use crate::ecs::core::bundle::bundle::{Bundle, BundleStaticInfo, sealed::BundleSealed};
use crate::ecs::core::bundle::bundle_type_registry;
use crate::ecs::core::ecs_master::ecs_master::EcsMaster;
use crate::ecs::identifiers::primitives::{ArchetypeId, ComponentId};

/// Emits `impl BundleSealed + Bundle` for a `Component` type where the whole
/// `self` is the one component — the exact mirror of the
/// `#[derive(Component)]` Bundle emission (Phase 22 D7).
///
/// Mirrored elements, in derive order: the named bound const-assert (readable
/// diagnostic), the per-type concrete `static INFO: OnceLock<BundleStaticInfo>`
/// (O3 / SBC-D5), a 1-element id slice (trivially canonical — B1),
/// `cached_archetype_id` via the per-world cache helper (SBC4), and the
/// `ManuallyDrop`-upfront (B4) + pointer-based byte-erasure (C5)
/// `for_each_component_bytes`.
macro_rules! impl_self_bundle {
    ($ty:ty) => {
        const _: () = {
            // Single-component bundle emission requires Send + Sync + Unpin.
            // (Mirror of the derive's named const-assert — same symbol, same
            // readable diagnostic if an engine component ever regresses.)
            const fn _boyko_component_as_bundle_requires_send_sync_unpin<
                T: Send + Sync + Unpin,
            >() {}
            _boyko_component_as_bundle_requires_send_sync_unpin::<$ty>();
        };

        impl $crate::ecs::core::bundle::bundle::sealed::BundleSealed for $ty {}

        impl $crate::ecs::core::bundle::bundle::Bundle for $ty {
            fn static_info()
            -> &'static $crate::ecs::core::bundle::bundle::BundleStaticInfo {
                static INFO: ::std::sync::OnceLock<
                    $crate::ecs::core::bundle::bundle::BundleStaticInfo,
                > = ::std::sync::OnceLock::new();
                INFO.get_or_init(|| {
                    // B1: a 1-element slice is trivially canonical. Leak
                    // bounded by SBC8 (once per type per process).
                    let leaked: &'static [
                        $crate::ecs::identifiers::primitives::ComponentId;
                        1
                    ] = ::std::boxed::Box::leak(::std::boxed::Box::new([
                        <$ty as $crate::ecs::core::component::component::Component>::component_id(),
                    ]));
                    $crate::ecs::core::bundle::bundle::BundleStaticInfo {
                        type_id: $crate::ecs::core::bundle::bundle_type_registry::register_new(),
                        component_ids: leaked.as_slice(),
                    }
                })
            }

            #[inline]
            fn cached_archetype_id(
                world: &mut $crate::ecs::core::ecs_master::ecs_master::EcsMaster,
            ) -> $crate::ecs::identifiers::primitives::ArchetypeId {
                world.bundle_archetype_id_for::<Self>()
            }

            fn for_each_component_bytes<F>(self, mut f: F)
            where
                F: ::std::ops::FnMut(
                    $crate::ecs::identifiers::primitives::ComponentId,
                    &[u8],
                ),
            {
                // B4: ManuallyDrop the whole value UPFRONT, before the
                // callback runs — a callback panic suppresses Drop (leak,
                // never double-drop with archetype-side ownership).
                let this = ::std::mem::ManuallyDrop::new(self);
                let id = <$ty as $crate::ecs::core::component::component::Component>::component_id();
                let ptr = &raw const *this as *const u8;
                let len = ::std::mem::size_of::<$ty>();
                // SAFETY (C5 byte-erasure, mechanical mirror of the
                //   `#[derive(Component)]` Bundle emission — see the module
                //   SAFETY-accounting note):
                //   (i)   `ptr` derives from `&raw const *ManuallyDrop<Self>`
                //         over a live stack local — valid for
                //         `len = size_of::<Self>()` bytes for this call.
                //   (ii)  `len` is exactly `size_of::<Self>()` — no over-read;
                //         for a ZST this is a valid zero-length slice over a
                //         non-null, u8-aligned pointer.
                //   (iii) The materialized slice is shared/immutable and the
                //         only live borrow of `this`; on callback success
                //         ownership of the bytes transfers to the archetype,
                //         on panic the ManuallyDrop suppresses Drop.
                let bytes: &[u8] =
                    unsafe { ::std::slice::from_raw_parts(ptr, len) };
                f(id, bytes);
            }
        }
    };
}

pub(crate) use impl_self_bundle;

/// Zero-component bundle backing [`Commands::spawn_empty`] (Phase 22 D5).
///
/// Spawning it lands the entity in the empty archetype
/// (`get_or_create_archetype(&[])`), resolved lazily through the ordinary
/// static-bundle-cache path: `EmptyBundle` owns its own [`BundleTypeId`], so
/// warm `spawn_empty` is the same sub-ns cached lookup as any bundle (SBC4).
///
/// Zero unsafe: `component_ids()` is a static empty slice (no leak) and
/// [`Bundle::for_each_component_bytes`] is a no-op — there are no bytes to
/// erase.
///
/// [`Commands::spawn_empty`]: crate::ecs::core::system::params::commands::Commands::spawn_empty
/// [`BundleTypeId`]: crate::ecs::core::bundle::BundleTypeId
pub(crate) struct EmptyBundle;

impl BundleSealed for EmptyBundle {}

impl Bundle for EmptyBundle {
    fn static_info() -> &'static BundleStaticInfo {
        static INFO: OnceLock<BundleStaticInfo> = OnceLock::new();
        INFO.get_or_init(|| BundleStaticInfo {
            type_id: bundle_type_registry::register_new(),
            // B1 holds vacuously; a static empty slice — no Box::leak needed.
            component_ids: &[],
        })
    }

    #[inline]
    fn cached_archetype_id(world: &mut EcsMaster) -> ArchetypeId {
        world.bundle_archetype_id_for::<Self>()
    }

    #[inline]
    fn for_each_component_bytes<F>(self, _f: F)
    where
        F: FnMut(ComponentId, &[u8]),
    {
        // Zero components — nothing to emit (B2/B4 hold vacuously).
    }
}
