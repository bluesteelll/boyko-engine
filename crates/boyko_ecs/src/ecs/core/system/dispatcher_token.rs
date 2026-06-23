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
use crate::ecs::core::entity::entity::Entity;
use crate::ecs::core::resources::nonsend_resources::nonsend_id;
use crate::ecs::core::resources::resource::{NonSendResource, Resource};
use crate::ecs::core::system::unsafe_ecs_cell::UnsafeEcsCell;
use crate::ecs::identifiers::primitives::{ArchetypeId, ComponentId};

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

    /// Projects a read-only [`WorldView`] of the whole ECS world.
    ///
    /// Tied to `&self` (NOT `'w`) — the M1 discipline: a `WorldView` cannot
    /// coexist with [`nonsend_resource_mut`](Self::nonsend_resource_mut)
    /// (`&mut self`) or [`into_cell`](Self::into_cell) (by value); borrowck
    /// forbids holding a `&self` and a `&mut self`/by-value borrow of the token
    /// at once. Read-only by design: there is no `world_mut` (it would alias the
    /// dispatcher's post-token apply reborrow).
    ///
    /// Does NOT hand out a `&EcsMaster`. A held struct-wide shared reference
    /// would freeze the F4-rooted `SharedReadWrite` slab cells and conflict with
    /// the `Box`-of-slab dealloc on `EcsMaster` drop (BUG-MIGRATE-TB-1). The
    /// returned [`WorldView`] re-exposes only the already-TB-safe `&self` read
    /// primitives, each doing its own narrow projection.
    #[inline]
    pub fn world(&self) -> WorldView<'_> {
        WorldView {
            ptr: self.ptr as *const EcsMaster,
            _marker: PhantomData,
            #[cfg(debug_assertions)]
            owning_thread: self.owning_thread,
        }
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

/// A read-only, dispatcher-solo view of the ECS world, projected from a
/// [`DispatcherToken`] via [`world`](DispatcherToken::world).
///
/// Tied to the token's `&self` borrow (`'v`). Holds a raw `*const EcsMaster`
/// and NEVER forms a struct-wide `&EcsMaster` (BUG-MIGRATE-TB-1): every accessor
/// forwards to a `&self` method on `EcsMaster` that does its own narrow
/// projection (slab access for `resource`; `addr_of!((*p).columns)` for
/// `get_component_raw`; the archetype walk for `query_entities_buf`). The handle
/// adds no wide-reference provenance: each `(*self.ptr).method()` autoref
/// reborrows only for the duration of that single call.
///
/// # Not `Copy` / not `Clone`, `!Send` / `!Sync`
///
/// The `'v = &self` tie makes a `WorldView` mutually exclusive with the token's
/// `&mut self` / by-value projections at borrowck (the M1 discipline). The raw
/// pointer makes it auto-`!Send`/`!Sync` so it can never cross to a worker. It is
/// deliberately not `Copy`/`Clone` so it cannot be stashed past the `&self`
/// borrow's intent (defensive; borrowck already binds `'v`).
///
/// In a release build the only field is the `*const EcsMaster` (the M2
/// `owning_thread` tripwire is `#[cfg(debug_assertions)]`-gated out), so the view
/// is pointer-sized; the forwarders compile to a direct call of the narrow
/// `&self` method.
pub struct WorldView<'v> {
    /// Narrow-projected per access; NEVER bound as a `&EcsMaster`.
    ptr: *const EcsMaster,
    /// Ties `'v` to the token's `&self`; makes the view `!Send`/`!Sync`.
    _marker: PhantomData<&'v EcsMaster>,
    /// Debug-only M2 tripwire copied from the token: the minting (dispatcher)
    /// thread. Zero release cost.
    #[cfg(debug_assertions)]
    owning_thread: std::thread::ThreadId,
}

impl WorldView<'_> {
    /// Debug-only M2 tripwire shared by every forwarder.
    #[cfg(debug_assertions)]
    #[inline]
    fn assert_owning_thread(&self, method: &str) {
        debug_assert_eq!(
            std::thread::current().id(),
            self.owning_thread,
            "invariant M2: WorldView::{method} called off the owning (dispatcher) \
             thread — the world must be read only on the thread that minted the token",
        );
    }

    /// [`EcsMaster::resource`] — panicking resource read. Narrow-projected.
    ///
    /// # Panics
    ///
    /// Panics if no resource of type `R` has been inserted (see
    /// [`EcsMaster::resource`]). Use [`try_resource`](Self::try_resource) for the
    /// non-panicking variant.
    #[inline]
    pub fn resource<R: Resource>(&self) -> &R {
        #[cfg(debug_assertions)]
        self.assert_owning_thread("resource");
        // SAFETY (Option C + BUG-MIGRATE-TB-1):
        //  Threading axis: `self.ptr` was minted from a DispatcherToken's
        //   `&'w mut EcsMaster` on the dispatcher-solo path (`new`'s pub(crate)
        //   unsafe gate, `running == 0`), so NO other thread holds a concurrent
        //   &mut/cell access — the read is single-threaded and aliasing-free, the
        //   same basis as the TB-green `nonsend_resource` read. EcsMaster is
        //   interior-mutable (query_state_cache/bundle_column_cache write through
        //   &self); we do NOT claim otherwise.
        //  Single-threaded TB axis: we form NO held struct-wide `&EcsMaster`. The
        //   `(*self.ptr).resource::<R>()` autoref reborrows only for the one call
        //   to a &self method that does its own narrow projection (the resources
        //   slab access), so no F4-rooted slab cell is frozen across the EcsMaster
        //   drop. The returned reference is tied to `&self` ⊆ 'v ⊆ the token's
        //   &self (M1) — it cannot alias a &mut projected from the token.
        unsafe { (*self.ptr).resource::<R>() }
    }

    /// [`EcsMaster::try_resource`] — non-panicking resource read. Narrow-projected.
    #[inline]
    pub fn try_resource<R: Resource>(&self) -> Option<&R> {
        #[cfg(debug_assertions)]
        self.assert_owning_thread("try_resource");
        // SAFETY (Option C + BUG-MIGRATE-TB-1):
        //  Threading axis: dispatcher-solo mint (`running == 0`) ⇒ no concurrent
        //   &mut/cell access; the read is single-threaded and aliasing-free, the
        //   same basis as `nonsend_resource`. EcsMaster is interior-mutable
        //   through &self; we do NOT claim otherwise.
        //  Single-threaded TB axis: no held struct-wide `&EcsMaster`. The
        //   `(*self.ptr).try_resource::<R>()` autoref reborrows only for the one
        //   call to a &self method doing its own narrow projection (resources slab
        //   access), so no F4-rooted slab cell is frozen across the EcsMaster drop.
        //   The returned reference is tied to `&self` ⊆ 'v ⊆ the token's &self (M1).
        unsafe { (*self.ptr).try_resource::<R>() }
    }

    /// [`EcsMaster::get_component_raw`] — per-entity column read. Narrow-projected
    /// (BUG-MIGRATE-TB-1-correct: the callee reads `columns` through
    /// `addr_of!`, never `&Archetype`).
    #[inline]
    pub fn get_component_raw(
        &self,
        entity: Entity,
        component_id: ComponentId,
    ) -> Option<*const u8> {
        #[cfg(debug_assertions)]
        self.assert_owning_thread("get_component_raw");
        // SAFETY (Option C + BUG-MIGRATE-TB-1):
        //  Threading axis: dispatcher-solo mint (`running == 0`) ⇒ no concurrent
        //   &mut/cell access; the read is single-threaded and aliasing-free.
        //   EcsMaster is interior-mutable through &self; we do NOT claim otherwise.
        //  Single-threaded TB axis: no held struct-wide `&EcsMaster`. The
        //   `(*self.ptr).get_component_raw(..)` autoref reborrows only for the one
        //   call to the &self method, which projects the single `Column` through
        //   `addr_of!((*p).columns)` — exactly the BUG-MIGRATE-TB-1 discipline, so
        //   no F4-rooted slab cell is frozen across the EcsMaster drop. The return
        //   is an owned `*const u8`, not tied to `&self`.
        unsafe { (*self.ptr).get_component_raw(entity, component_id) }
    }

    /// [`EcsMaster::query_entities_buf`] — allocation-free archetype walk writing
    /// matching entities into `out`, reusing `arch_scratch`. Narrow-projected.
    #[inline]
    pub fn query_entities_buf(
        &self,
        component_ids: &[ComponentId],
        out: &mut Vec<Entity>,
        arch_scratch: &mut Vec<ArchetypeId>,
    ) {
        #[cfg(debug_assertions)]
        self.assert_owning_thread("query_entities_buf");
        // SAFETY (Option C + BUG-MIGRATE-TB-1):
        //  Threading axis: dispatcher-solo mint (`running == 0`) ⇒ no concurrent
        //   &mut/cell access; the walk is single-threaded and aliasing-free.
        //   EcsMaster is interior-mutable through &self; we do NOT claim otherwise.
        //  Single-threaded TB axis: no held struct-wide `&EcsMaster`. The
        //   `(*self.ptr).query_entities_buf(..)` autoref reborrows only for the one
        //   call to the &self method (the archetype-master walk, already TB-audited
        //   for the F4 slab discipline), so no slab cell is frozen across the
        //   EcsMaster drop. Output goes into the caller's `out`/`arch_scratch`.
        unsafe { (*self.ptr).query_entities_buf(component_ids, out, arch_scratch) }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::OnceLock;

    use super::*;
    use crate::ecs::core::component::component::Component;
    use crate::ecs::core::component::component_registry;
    use crate::ecs::core::resources::register_new;
    use crate::ecs::core::resources::resource::Resource;
    use crate::ecs::identifiers::primitives::ResourceId;

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

    /// `Send + Sync` test resource for the `WorldView::resource` forwarder.
    #[derive(Debug, PartialEq)]
    struct ViewResource(u32);
    impl Resource for ViewResource {
        fn resource_id() -> ResourceId {
            static ID: OnceLock<ResourceId> = OnceLock::new();
            *ID.get_or_init(|| ResourceId(register_new::<Self>()))
        }
    }

    /// Test component for the `WorldView::get_component_raw` / `query_entities_buf`
    /// forwarders. Manual `Component` impl mirrors what `#[derive(Component)]`
    /// generates; the id is high enough to avoid colliding with other in-crate
    /// test components in the same test binary.
    #[repr(C)]
    #[derive(Clone, Copy, PartialEq, Debug)]
    struct ViewComponent {
        x: u32,
        y: u32,
    }
    impl Component for ViewComponent {
        fn component_id() -> ComponentId {
            static ID: OnceLock<ComponentId> = OnceLock::new();
            // In-range (< MAX_COMPONENTS == 512) and in a band no other in-crate
            // lib test occupies (498/499 are free; 495-497 are component_set's), so
            // it neither trips the registry cap nor aliases another test component
            // in the shared lib-test binary.
            const VIEW_CID: ComponentId = ComponentId(498);
            *ID.get_or_init(|| {
                component_registry::register_layout::<ViewComponent>(VIEW_CID.0);
                VIEW_CID
            })
        }
    }

    /// `WorldView` round-trips a resource read, a per-entity column read, and an
    /// allocation-free archetype walk, all through the dispatcher token's `&self`
    /// projection. Dropping `ecs` at scope end exercises the BUG-MIGRATE-TB-1
    /// freeze-on-drop scenario the forwarders' narrow projection must survive.
    #[test]
    fn world_view_reads_resource_and_component() {
        let mut ecs = EcsMaster::new();
        ecs.insert_resource(ViewResource(42));

        let cid = ViewComponent::component_id();
        let arch = ecs.get_or_create_archetype(&[cid]);
        let entity = ecs
            .spawn_one(arch, ViewComponent { x: 7, y: 9 })
            .expect("spawn must succeed");

        // SAFETY (Option C): `run_system_once`-equivalent — `&mut ecs` is
        //   exclusive for the whole test, so `running == 0` at the language
        //   level (no worker). The view is consumed before `ecs` is touched
        //   mutably again.
        let token = unsafe { DispatcherToken::new(&mut ecs) };
        let world = token.world();

        assert_eq!(
            world.resource::<ViewResource>(),
            &ViewResource(42),
            "resource forwarder reads the inserted value"
        );
        assert_eq!(
            world.try_resource::<ViewResource>(),
            Some(&ViewResource(42)),
            "try_resource forwarder reads the inserted value"
        );

        let raw = world
            .get_component_raw(entity, cid)
            .expect("entity hosts the component");
        // SAFETY: `raw` points at the live `ViewComponent` row written by
        //   `spawn_one`; the column byte range is valid for the read while the
        //   `&self`-tied `world` view (hence `ecs`) is alive.
        let got = unsafe { *(raw as *const ViewComponent) };
        assert_eq!(got, ViewComponent { x: 7, y: 9 });

        let mut out = Vec::new();
        let mut arch_scratch = Vec::new();
        world.query_entities_buf(&[cid], &mut out, &mut arch_scratch);
        assert_eq!(out, vec![entity], "the walk finds the one matching entity");
    }

    /// Exercises the #31 borrow-split SHAPE at runtime: a `WorldView` read borrow
    /// (a closure consuming the view by value, gathering into a reused buffer)
    /// that ENDS before a subsequent `&mut self` token projection runs — the same
    /// gather-then-`!Send`-upload sequencing
    /// `UiUploadSystem::host_upload_frame_from_world` relies on. If the read borrow
    /// outlived the gather it would conflict with the `&mut self` projection
    /// (exactly the case the `world_then_mut_aliases_rejected` compile-fail pins);
    /// here it does not, so the split runs cleanly.
    #[test]
    fn world_read_then_mut_split_has_no_borrow_conflict() {
        let mut ecs = EcsMaster::new();
        ecs.insert_resource(ViewResource(3));
        ecs.insert_non_send_resource(NonSendCounter {
            value: 100,
            _not_send: std::ptr::null(),
        });

        // SAFETY (Option C): exclusive `&mut ecs` for the whole test ⇒ no worker.
        let mut token = unsafe { DispatcherToken::new(&mut ecs) };

        // Phase 1 — world-read gather: the closure takes the `WorldView` BY VALUE,
        // fills a reused buffer, and the view borrow ends with the closure (mirrors
        // `gather_nodes(world, node_buf)`).
        let mut gathered: Vec<u32> = Vec::new();
        let gather = |w: WorldView<'_>, out: &mut Vec<u32>| {
            out.clear();
            out.push(w.resource::<ViewResource>().0);
        };
        gather(token.world(), &mut gathered);
        assert_eq!(gathered, vec![3], "the gather read the resource through the view");

        // Phase 2 — the `&mut self` projection runs only AFTER the read borrow ended
        // (mirrors the `!Send` upload borrow going live post-gather). No conflict.
        let c = token
            .nonsend_resource_mut::<NonSendCounter>()
            .expect("present");
        c.value += gathered[0];
        assert_eq!(c.value, 103, "the post-gather mutable projection observed the gather");
    }
}
