//! Resource & NonSend-resource facade on [`EcsMaster`] (mechanical split).
//!
//! Insert / remove / get / contains for `Resource` and `NonSendResource`.
//! Extracted verbatim from `ecs_master.rs`.

use crate::ecs::core::resources::nonsend_resources::NonSendResources;
use crate::ecs::core::resources::resource::{NonSendResource, Resource};
use crate::ecs::core::ecs_master::ecs_master::EcsMaster;

impl EcsMaster {
    // ── Resources facade (Phase 8a Step 9) ───────────────────────────────────

    /// Inserts (or replaces) the world-global resource of type `R`.
    ///
    /// Cold path. Forwards to [`Resources::insert`]; see its docs for the
    /// clear-bit-first replace protocol (R4) that guards against panic-in-drop
    /// UB on the old value.
    ///
    /// [`Resources::insert`]: crate::ecs::core::resources::resources::Resources::insert
    #[cold]
    pub fn insert_resource<R: Resource>(&mut self, value: R) {
        self.resources.insert(value);
    }

    /// Removes the resource of type `R` from the world, returning the typed
    /// value if it was present.
    ///
    /// Cold path. Forwards to [`Resources::remove`]; see invariant R5 for the
    /// clear-bit-before-`Box::from_raw` ordering.
    ///
    /// [`Resources::remove`]: crate::ecs::core::resources::resources::Resources::remove
    #[cold]
    pub fn remove_resource<R: Resource>(&mut self) -> Option<R> {
        self.resources.remove::<R>()
    }

    /// Returns `true` iff the world currently holds a resource of type `R`.
    #[inline]
    pub fn contains_resource<R: Resource>(&self) -> bool {
        self.resources.contains::<R>()
    }

    /// Returns a shared reference to the resource of type `R`.
    ///
    /// # Panics
    ///
    /// Panics if no resource of type `R` has been inserted. Use
    /// [`try_resource`] for the non-panicking variant.
    ///
    /// [`try_resource`]: EcsMaster::try_resource
    #[inline]
    pub fn resource<R: Resource>(&self) -> &R {
        match self.resources.get_ptr::<R>() {
            Some(ptr) => {
                // SAFETY (R2): `get_ptr` returned `Some` ⇒ the slot is populated
                //   and the bytes at `ptr` form a valid `R` (the slot was
                //   inserted via `insert_resource::<R>` with this same TypeId
                //   binding; the cached `ResourceId` in the registry guarantees
                //   the type tag). The lifetime of the returned reference is
                //   tied to `&self`, so the pointer cannot outlive the borrow.
                unsafe { &*ptr }
            }
            None => missing_resource_panic_facade::<R>(),
        }
    }

    /// Returns an exclusive reference to the resource of type `R`.
    ///
    /// # Panics
    ///
    /// Panics if no resource of type `R` has been inserted. Use
    /// [`try_resource_mut`] for the non-panicking variant.
    ///
    /// [`try_resource_mut`]: EcsMaster::try_resource_mut
    #[inline]
    pub fn resource_mut<R: Resource>(&mut self) -> &mut R {
        match self.resources.get_mut_ptr::<R>() {
            Some(ptr) => {
                // SAFETY (R2, R4): `get_mut_ptr` returned `Some` ⇒ the slot is
                //   populated and the bytes at `ptr` form a valid `R`. `&mut
                //   self` gives exclusive access to the resources slab, so the
                //   `&mut R` produced here cannot alias any other reference
                //   into the same slot for the duration of the borrow.
                unsafe { &mut *ptr }
            }
            None => missing_resource_panic_facade::<R>(),
        }
    }

    /// Returns a shared reference to the resource of type `R`, or `None` if
    /// the resource has not been inserted. Non-panicking counterpart of
    /// [`resource`].
    ///
    /// [`resource`]: EcsMaster::resource
    #[inline]
    pub fn try_resource<R: Resource>(&self) -> Option<&R> {
        // SAFETY (R2): same as `resource` — `get_ptr` returns `Some` only when
        //   the slot is populated and holds a valid `R`. Lifetime is tied to
        //   `&self`.
        self.resources.get_ptr::<R>().map(|p| unsafe { &*p })
    }

    /// Returns an exclusive reference to the resource of type `R`, or `None`
    /// if the resource has not been inserted. Non-panicking counterpart of
    /// [`resource_mut`].
    ///
    /// [`resource_mut`]: EcsMaster::resource_mut
    #[inline]
    pub fn try_resource_mut<R: Resource>(&mut self) -> Option<&mut R> {
        // SAFETY (R2, R4): same as `resource_mut` — `get_mut_ptr` returns
        //   `Some` only when the slot is populated and holds a valid `R`.
        //   `&mut self` gives exclusive access for the returned borrow.
        self.resources.get_mut_ptr::<R>().map(|p| unsafe { &mut *p })
    }

    // ── NonSend resources facade (Phase 4 Seam 2 — D6 / CR-A) ─────────────────

    /// Inserts (or replaces) the world-global **non-`Send`** resource of type
    /// `R`, lazily materialising the NonSend slab on first call (P5 — zero
    /// allocation until then).
    ///
    /// Cold path. `R` is `!Send`, so this MUST be called on `R`'s owning
    /// thread — the typical caller is the dispatcher during setup. The world
    /// itself stays `Send + Sync` (the slab erases types to a raw pointer +
    /// drop fn + `TypeId`; SEND10).
    ///
    /// # Caller obligation (NSND-THREAD — Phase 5 Option C)
    ///
    /// The thread that makes the FIRST `insert_non_send_resource` call becomes
    /// the slab's OWNING thread (stamped in debug as
    /// [`NonSendResources::owning_thread`]). Every subsequent insert, projection
    /// (`NonSendRes` / `NonSendResMut` / `DispatcherToken`), and drop of a
    /// `!Send` value MUST happen on that same thread. In practice this is the
    /// dispatcher thread that also drives [`Schedule::run`]. A wrong-thread touch
    /// is UB in release; in debug the M2 tripwire (`debug_assert_eq!`) catches it.
    ///
    /// [`NonSendResources::owning_thread`]: crate::ecs::core::resources::nonsend_resources::NonSendResources
    /// [`Schedule::run`]: crate::ecs::core::schedule::schedule::Schedule::run
    #[cold]
    pub fn insert_non_send_resource<R: NonSendResource>(&mut self, value: R) {
        self.nonsend_resources
            .get_or_insert_with(|| Box::new(NonSendResources::new()))
            .insert(value);
    }

    /// Removes the non-`Send` resource of type `R`, returning it if present.
    ///
    /// Cold path; runs on `R`'s owning thread (the returned `R` is `!Send`).
    /// Returns `None` if the NonSend slab was never materialised or the slot
    /// is empty.
    #[cold]
    pub fn remove_non_send_resource<R: NonSendResource>(&mut self) -> Option<R> {
        self.nonsend_resources.as_mut()?.remove::<R>()
    }

    /// Returns `true` iff the world currently holds a non-`Send` resource of
    /// type `R`.
    #[inline]
    pub fn contains_non_send_resource<R: NonSendResource>(&self) -> bool {
        self.nonsend_resources
            .as_ref()
            .is_some_and(|slab| slab.contains::<R>())
    }

    /// Returns a shared reference to the non-`Send` resource of type `R`.
    ///
    /// # Panics
    /// Panics if no non-`Send` resource of type `R` has been inserted. Use
    /// [`try_non_send_resource`](Self::try_non_send_resource) for the
    /// non-panicking variant.
    #[inline]
    pub fn non_send_resource<R: NonSendResource>(&self) -> &R {
        match self.try_non_send_resource::<R>() {
            Some(r) => r,
            None => missing_non_send_resource_panic::<R>(),
        }
    }

    /// Returns an exclusive reference to the non-`Send` resource of type `R`.
    ///
    /// # Panics
    /// Panics if no non-`Send` resource of type `R` has been inserted. Use
    /// [`try_non_send_resource_mut`](Self::try_non_send_resource_mut) for the
    /// non-panicking variant.
    #[inline]
    pub fn non_send_resource_mut<R: NonSendResource>(&mut self) -> &mut R {
        match self.try_non_send_resource_mut::<R>() {
            Some(r) => r,
            None => missing_non_send_resource_panic::<R>(),
        }
    }

    /// Returns a shared reference to the non-`Send` resource of type `R`, or
    /// `None` if it has not been inserted. Non-panicking counterpart of
    /// [`non_send_resource`](Self::non_send_resource).
    #[inline]
    pub fn try_non_send_resource<R: NonSendResource>(&self) -> Option<&R> {
        let slab = self.nonsend_resources.as_ref()?;
        // SAFETY (N2): `get_ptr` returns `Some` only when the slot is
        //   populated and the bytes form a valid `R` (the id is type-bound to
        //   `R` inside the slab); the `&self` borrow bounds the lifetime. `R`
        //   is `!Send`, but the direct-API caller is on the owning thread.
        slab.get_ptr::<R>().map(|p| unsafe { &*p })
    }

    /// Returns an exclusive reference to the non-`Send` resource of type `R`,
    /// or `None` if it has not been inserted. Non-panicking counterpart of
    /// [`non_send_resource_mut`](Self::non_send_resource_mut).
    #[inline]
    pub fn try_non_send_resource_mut<R: NonSendResource>(&mut self) -> Option<&mut R> {
        let slab = self.nonsend_resources.as_mut()?;
        // SAFETY (N2): same as `try_non_send_resource`; `&mut self` gives
        //   exclusive access for the returned borrow.
        slab.get_mut_ptr::<R>().map(|p| unsafe { &mut *p })
    }

}

#[cold]
#[inline(never)]
fn missing_resource_panic_facade<R: Resource>() -> ! {
    panic!(
        "Resource `{}` not registered. Call `EcsMaster::insert_resource::<{}>(...)` first.",
        R::debug_type_name(),
        R::debug_type_name()
    );
}

/// Cold-path panic helper for [`EcsMaster::non_send_resource`] /
/// [`EcsMaster::non_send_resource_mut`] (Phase 4 Seam 2). Mirrors
/// [`missing_resource_panic_facade`] for the NonSend slab.
#[cold]
#[inline(never)]
fn missing_non_send_resource_panic<R: NonSendResource>() -> ! {
    let name = std::any::type_name::<R>();
    panic!(
        "NonSend resource `{name}` not registered. \
         Call `EcsMaster::insert_non_send_resource::<{name}>(...)` first."
    );
}
