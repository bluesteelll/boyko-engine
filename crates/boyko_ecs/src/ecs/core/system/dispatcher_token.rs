//! [`DispatcherToken`] — the dispatcher-only capability to reach `!Send`
//! resources (Phase 5 Option C — the soundness rework of the Wave C raw-cell
//! projection).
//!
//! # Why a token (Option C)
//!
//! A hand-written out-of-crate `System` (the `boyko_render` `GpuSystem`) must
//! reach a concrete `!Send` resource (its RHI context) WITHOUT routing through
//! the `NonSendResMut` `SystemParam` — the param's `init_access` side effect
//! (`mark_universal`) would promote the system to `SystemKind::CpuExclusive`,
//! contradicting the `GpuCompute` marker (MF-5).
//!
//! Wave C exposed a `pub unsafe fn UnsafeEcsCell::nonsend_resource_mut`. That
//! accessor was reachable on the CONCURRENT worker path (any system that holds
//! a cell copy could call it from a worker thread), and its `'w` return
//! lifetime let two back-to-back calls hand out two live `&mut R` aliases. Both
//! are real UB paths (C1 = worker reachability of the `!Send` projection; M1 =
//! the aliasing `'w` lifetime).
//!
//! `DispatcherToken` closes both by ENFORCEMENT:
//!
//! * **C1** — the token is minted ONLY by the scheduler on the dispatcher-solo
//!   path (and by [`EcsMaster::run_system_once`], which holds `&mut EcsMaster`
//!   exclusively, so `running == 0` at the language level). A worker never sees
//!   one — the `!Send` projection is structurally unreachable from a worker.
//! * **M1** — [`nonsend_resource_mut`](DispatcherToken::nonsend_resource_mut)
//!   ties the returned `&mut R` to `&mut self`, NOT to `'w`. A second call
//!   cannot alias the first: the borrow checker forbids holding two `&mut self`
//!   borrows of the token.
//! * **M2** — a debug-only `owning_thread` stamp tripwires any projection from
//!   the wrong thread (`assert_eq!`), catching a routing mistake in debug long
//!   before it can be UB in release.
//!
//! `DispatcherToken` is generic over [`NonSendResource`] and names NO graphics
//! type — `boyko_ecs` stays graphics-pure.

use std::marker::PhantomData;

use crate::ecs::core::ecs_master::ecs_master::EcsMaster;
use crate::ecs::core::resources::nonsend_resources::nonsend_id;
use crate::ecs::core::resources::resource::NonSendResource;
use crate::ecs::core::system::unsafe_ecs_cell::UnsafeEcsCell;

/// A dispatcher-only capability handle on `EcsMaster`, the sole route a
/// hand-written out-of-crate [`System`] uses to reach a `!Send` resource
/// (Phase 5 Option C / MF-5).
///
/// Minted by the scheduler on the dispatcher-solo path (and by
/// [`EcsMaster::run_system_once`]) — never handed to a worker. Passed to
/// [`System::run_dispatcher`] by value, so a system body that needs `!Send`
/// access overrides that method and projects through this token; CPU systems
/// use the default forwarder and never see it.
///
/// # Not `Copy` / not `Clone`
///
/// Deliberately NEITHER. The borrowck M1 fix depends on it: a `Copy` token
/// would let a system mint two independent handles, each yielding a `&mut R`,
/// re-opening the aliasing hole that the `&mut self` receiver of
/// [`nonsend_resource_mut`](Self::nonsend_resource_mut) closes.
///
/// [`System`]: super::system::System
/// [`System::run_dispatcher`]: super::system::System::run_dispatcher
/// [`EcsMaster::run_system_once`]: crate::ecs::core::ecs_master::ecs_master::EcsMaster::run_system_once
pub struct DispatcherToken<'w> {
    /// Raw pointer to the underlying `EcsMaster`. Lifetime enforced by the
    /// `PhantomData<&'w mut EcsMaster>` below — the token may not outlive the
    /// borrow that produced it.
    ptr: *mut EcsMaster,
    /// Carries `&'w mut EcsMaster` variance + the unique-borrow marker, so the
    /// token cannot escape the dispatcher's reborrow scope.
    _marker: PhantomData<&'w mut EcsMaster>,
    /// Debug-only tripwire: the thread that minted the token. Every projection
    /// `assert_eq!`s the current thread against it (M2). Zero release cost.
    #[cfg(debug_assertions)]
    owning_thread: std::thread::ThreadId,
}

impl<'w> DispatcherToken<'w> {
    /// Mints a dispatcher token from `&mut EcsMaster`. Dispatcher-only.
    ///
    /// # Safety
    ///
    /// (Option C — the dispatcher-solo mint contract.)
    ///
    /// * The caller MUST be the scheduler on the dispatcher-solo path
    ///   (`running == 0`, no worker live), or [`EcsMaster::run_system_once`]
    ///   (which holds `&mut EcsMaster` exclusively ⇒ `running == 0` at the
    ///   language level). The token's whole soundness story is "no worker
    ///   aliases the `!Send` payload it projects" — minting it anywhere a
    ///   worker could be live breaks that.
    /// * The returned token must not outlive `'w` (enforced by `PhantomData`).
    ///
    /// [`EcsMaster::run_system_once`]: crate::ecs::core::ecs_master::ecs_master::EcsMaster::run_system_once
    #[inline]
    pub(crate) unsafe fn new(world: &'w mut EcsMaster) -> Self {
        Self {
            ptr: world as *mut EcsMaster,
            _marker: PhantomData,
            #[cfg(debug_assertions)]
            owning_thread: std::thread::current().id(),
        }
    }

    /// Projects an exclusive borrow of the `!Send` resource of type `R` from the
    /// world's NonSend slab, or `None` if it was never inserted.
    ///
    /// The returned `&mut R` is tied to `&mut self`, NOT to `'w` — this is the
    /// M1 fix. A second `nonsend_resource_mut` call cannot alias the first: the
    /// borrow checker forbids holding two `&mut self` borrows of the token, so
    /// the prior `&mut R` must be dropped before the next projection.
    ///
    /// # Safety
    ///
    /// (Option C — the apply-window single-thread-touch invariant.)
    ///
    /// * The token is mintable ONLY by the dispatcher at `running == 0` (see
    ///   `new`), so no worker holds an aliasing cell — the `!Send`
    ///   payload `R` is touched single-threaded on its owning thread, the
    ///   external-synchronisation contract `!Send` types need.
    /// * `&mut self` guarantees the returned `&mut R` is the UNIQUE live
    ///   projection through this token (M1).
    ///
    pub fn nonsend_resource_mut<R: NonSendResource>(&mut self) -> Option<&mut R> {
        #[cfg(debug_assertions)]
        debug_assert_eq!(
            std::thread::current().id(),
            self.owning_thread,
            "invariant M2: DispatcherToken::nonsend_resource_mut called off the \
             owning (dispatcher) thread — the !Send payload must be touched only \
             on the thread that minted the token"
        );
        // SAFETY (Option C): the token is mintable only by the dispatcher at
        //   `running == 0` (the `new` contract), so no worker holds an aliasing
        //   cell; the `!Send` `R` is touched single-threaded on its owning
        //   thread. `&mut self` makes the returned `&mut R` the unique live
        //   projection (M1). `self.ptr` carries write-capable provenance (minted
        //   from `&mut EcsMaster`), is valid for `'w >= self`, and is projected
        //   directly through `*self.ptr` onto the `nonsend_resources` field with
        //   no intermediate `&mut EcsMaster` reborrow that would downgrade the
        //   tag stack.
        let slab = unsafe { (*self.ptr).nonsend_resources.as_deref_mut() }?;
        let ptr = slab.get_mut_ptr_by_id(nonsend_id::<R>())?;
        // SAFETY (Option C): `get_mut_ptr_by_id` returns `Some` only for a
        //   populated slot whose bytes form a valid `R` (the id minted by
        //   `nonsend_id::<R>()` is type-bound to `R`, N1), with write-capable
        //   provenance. The reborrow's lifetime is tied to `&mut self`, so it is
        //   the unique live `&mut R` (M1).
        Some(unsafe { &mut *(ptr as *mut R) })
    }

    /// Projects a shared borrow of the `!Send` resource of type `R`, or `None`
    /// if it was never inserted. The read twin of
    /// [`nonsend_resource_mut`](Self::nonsend_resource_mut).
    ///
    /// # Safety
    ///
    /// Same single-thread-touch invariant as
    /// [`nonsend_resource_mut`](Self::nonsend_resource_mut): the token is
    /// dispatcher-only, so the `!Send` `R` is read single-threaded on its owning
    /// thread. The returned `&R` is tied to `&self`.
    pub fn nonsend_resource<R: NonSendResource>(&self) -> Option<&R> {
        #[cfg(debug_assertions)]
        debug_assert_eq!(
            std::thread::current().id(),
            self.owning_thread,
            "invariant M2: DispatcherToken::nonsend_resource called off the owning \
             (dispatcher) thread"
        );
        // SAFETY (Option C): dispatcher-only mint ⇒ no worker aliases the slab;
        //   the `!Send` `R` is read single-threaded on its owning thread.
        //   `self.ptr` is valid for `'w >= self`; the `&` projects directly
        //   through `*self.ptr` onto the `nonsend_resources` field, and the
        //   returned reference is tied to `&self`.
        let slab = unsafe { (*self.ptr).nonsend_resources.as_deref() }?;
        let ptr = slab.get_ptr_by_id(nonsend_id::<R>())?;
        // SAFETY (Option C): `get_ptr_by_id` returns `Some` only for a populated,
        //   R-typed slot (N1); the cast + reborrow are sound and tied to `&self`.
        Some(unsafe { &*(ptr as *const R) })
    }

    /// Reconstructs an [`UnsafeEcsCell`] from the token, for the default
    /// [`System::run_dispatcher`] forwarder to `run_unsafe`.
    ///
    /// # Safety
    ///
    /// * The token was minted on the dispatcher-solo path (`new`'s contract),
    ///   so the cell's S1 contract (no other `run_unsafe` in flight) holds —
    ///   the same `&mut EcsMaster` provenance that minted the token backs the
    ///   cell.
    ///
    /// [`System::run_dispatcher`]: super::system::System::run_dispatcher
    #[inline]
    pub(crate) unsafe fn into_cell(self) -> UnsafeEcsCell<'w> {
        // SAFETY: `self.ptr` was minted from `&'w mut EcsMaster` (write-capable
        //   provenance) on the dispatcher-solo path, so reconstructing a mutable
        //   cell over the same pointer upholds U_C1 (lifetime `'w`) and the
        //   dispatcher-solo S1 contract. We go through the live `&mut *self.ptr`
        //   reborrow so the cell is minted via the blessed `new_mutable` path.
        unsafe { UnsafeEcsCell::new_mutable(&mut *self.ptr) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `!Send` test resource with a mutable counter (raw pointer interior keeps
    /// it `!Send`).
    struct NonSendCounter {
        value: u32,
        _not_send: *const u8,
    }
    impl NonSendResource for NonSendCounter {}

    /// `DispatcherToken::nonsend_resource_mut` round-trips a write into the
    /// `!Send` slab.
    #[test]
    fn nonsend_resource_mut_round_trips_a_write() {
        let mut ecs = EcsMaster::new();
        ecs.insert_non_send_resource(NonSendCounter {
            value: 10,
            _not_send: std::ptr::null(),
        });

        // SAFETY (Option C): `run_system_once`-equivalent — `&mut ecs` is
        //   exclusive for the whole test, so `running == 0` at the language
        //   level (no worker). The token is consumed before `ecs` is touched
        //   again.
        let mut token = unsafe { DispatcherToken::new(&mut ecs) };
        {
            let c = token
                .nonsend_resource_mut::<NonSendCounter>()
                .expect("inserted resource must project");
            assert_eq!(c.value, 10, "initial value round-trips");
            c.value += 5;
        }
        // A second, sequential projection observes the write (the first borrow
        // ended at the block close).
        let c = token
            .nonsend_resource_mut::<NonSendCounter>()
            .expect("still present");
        assert_eq!(c.value, 15, "the mutation persisted in the slab");
    }

    /// The read twin returns the stored value.
    #[test]
    fn nonsend_resource_reads() {
        let mut ecs = EcsMaster::new();
        ecs.insert_non_send_resource(NonSendCounter {
            value: 7,
            _not_send: std::ptr::null(),
        });
        // SAFETY (Option C): exclusive `&mut ecs`, no worker live.
        let token = unsafe { DispatcherToken::new(&mut ecs) };
        let c = token
            .nonsend_resource::<NonSendCounter>()
            .expect("present");
        assert_eq!(c.value, 7);
    }

    /// A missing resource projects `None` rather than panicking.
    #[test]
    fn missing_resource_projects_none() {
        let mut ecs = EcsMaster::new();
        // SAFETY (Option C): exclusive `&mut ecs`, no worker live.
        let mut token = unsafe { DispatcherToken::new(&mut ecs) };
        assert!(token.nonsend_resource_mut::<NonSendCounter>().is_none());
    }
}
