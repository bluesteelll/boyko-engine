use std::cell::UnsafeCell;
use std::ptr::NonNull;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU32, Ordering};

use crate::ecs::core::archetype::archetype_master::ArchetypeMaster;
use crate::ecs::core::bundle::bundle::Bundle;
use crate::ecs::core::bundle::bundle_column_cache::BundleColumnCache;
use crate::ecs::core::bundle::bundle_type_registry::MAX_BUNDLE_TYPES;
use crate::ecs::core::commands::command::Command;
use crate::ecs::core::commands::command_queue::CommandQueue;
use crate::ecs::core::change_detection::{CHECK_TICK_THRESHOLD, Tick};
use crate::ecs::core::component::hooks::scope::{DeferredScopeGuard, hook_drain_depth};
use crate::ecs::core::component::observers::entity_store::EntityObserverStore;
use crate::ecs::core::component::observers::trigger::TriggerRegistry;
use crate::ecs::core::entity::entity::Entity;
use crate::ecs::core::entity::entity_master::EntityMaster;
use crate::ecs::core::events::event_dispatcher::EventDispatcher;
use crate::ecs::core::iters::query::data::QueryData;
use crate::ecs::core::iters::query::filter::QueryFilter;
use crate::ecs::core::iters::query::query_type_registry::{
    MAX_QUERY_TYPES, QueryTypeId, QueryTypeKey,
};
use crate::ecs::core::iters::query::query_view::QueryView;
use crate::ecs::core::iters::query::state::QueryDataState;
use crate::ecs::core::resources::nonsend_resources::NonSendResources;
use crate::ecs::core::resources::resources::Resources;
use crate::ecs::core::system::params::entity_counter::MAX_BATCH_HINT;
use crate::ecs::core::system::unsafe_ecs_cell::UnsafeEcsCell;
use crate::ecs::identifiers::primitives::{ArchetypeId, EntityId, WorldId};
use crate::ecs::error::{EcsError, EcsResult};

/// Phase 9 Round 2 W3 — entity-master capacity hint used by the
/// dispatcher-side `ensure_capacity` growth path. Structural mutation runs
/// only on the dispatcher inside the apply window (SCH7), but
/// `entity_master.entities_inland` is read by worker `&self` paths
/// (e.g. `get_component_raw` from inside a system body); a reallocation
/// during a concurrent read would dangle the worker's reference. Growing
/// the fast-store to `n + MAX_BATCH_HINT` on the dispatcher BEFORE the
/// apply window opens preserves the SEND5 / SBO16 invariant.
///
/// 64 000 ≈ 16 MB at the current `EntityInland` size (~32 B incl. swap-side
/// vectors), which fits comfortably under the L3 budget of every supported
/// target and matches the plan's "MAX_ENTITIES_HINT" knob in §11.4 W3.
///
/// Phase 12.6 — `EcsMaster::new` no longer pre-extends the entity vectors
/// to `MAX_ENTITIES_HINT + MAX_BATCH_HINT`. The 480 µs eager memset that
/// dominated `EcsMaster::new` is gone; the dispatcher's `spawn_batch` /
/// `Commands::spawn_batch` apply path drives growth on demand via
/// [`EntityMaster::ensure_capacity`]. Single-entity paths
/// (`create_entity`, `register_entity_with_ptr`) already grow lazily via
/// `Vec::resize` and need no additional plumbing.
#[allow(dead_code)] // reserved for future ensure_capacity overshoot tracking
const MAX_ENTITIES_HINT: usize = 64_000;



/// Main ECS manager that coordinates entities, archetypes, memory, and events.
///
/// # Field order (drop order — Phase 8a C5 RESOLUTION)
///
/// Fields are dropped in declaration order:
/// `resources → events → entity_master → archetype_master → …`.
///
/// `resources: Resources` is the **first** field so it drops first. A
/// `Resource`'s `Drop` impl runs while every other subsystem is still alive;
/// if user code violates the [`Resource`] contract and touches the world from
/// `Drop`, the world is still fully valid. The most-defensive position
/// prevents the worst case from being UB.
///
/// `events: EventDispatcher` drops next. Event buffers live in their own
/// heap allocations and do not reference component storage.
///
/// Component storage is owned per-pool since Phase X.I: every
/// `ComponentPool` carries its own `VmReservation`, released by the pool's
/// own `Drop` strictly after its rows are dropped. (The historical shared
/// `Box<Arena>` field — and the whole C-001 / Phase 3a raw-provenance
/// story around it — was retired in Phase X.J.) `query_state_cache` stays
/// declared LAST so cached query state always drops after the archetype
/// subsystem it may point into.
///
/// [`Resource`]: crate::ecs::core::resources::Resource
pub struct EcsMaster {
    /// Process-unique world identifier (Phase 21), minted at construction.
    ///
    /// Read by [`Schedule::run`]'s world-binding gate — a `Schedule` records
    /// the id of the world it was built on and panics when handed a different
    /// one. Declared first for readability; `WorldId` is `Copy` with no drop
    /// glue, so the Phase 8a C5 drop-order contract (`resources` drops first
    /// among Drop-bearing fields) is unaffected.
    ///
    /// [`Schedule::run`]: crate::ecs::core::schedule::schedule::Schedule::run
    world_id: WorldId,

    /// World-global resources slab.
    ///
    /// Dropped first per the Phase 8a C5 drop-order resolution. Public facade
    /// methods (`insert_resource`, `remove_resource`, `resource`,
    /// `resource_mut`) are deferred to Step 9; this minimal field addition
    /// unblocks Step 7's `Res<R>` / `ResMut<R>` `get_param` via
    /// `UnsafeEcsCell::resources()` / `resources_mut()`.
    pub(crate) resources: Resources,

    /// Phase 4 Seam 2 (D6 / CR-A / P5) — world-global **non-`Send`** resource
    /// slab. LAZY (`Option<Box<NonSendResources>>`): `None` until the first
    /// `insert_non_send_resource`, so a world that never homes a NonSend
    /// resource pays ZERO allocation (the 0%-gate) and ZERO `EcsMaster::new`
    /// cost.
    ///
    /// **Drop-order (C5)**: declared immediately AFTER `resources`, so
    /// `resources` still drops first; the NonSend slab drops next, both
    /// before the entity / archetype subsystems.
    ///
    /// **SEND1 (SEND10 / CR-A — FIX-6 / FSC-I1)**: `EcsMaster: Send` is forced
    /// by the blanket `unsafe impl Send/Sync for EcsMaster` REGARDLESS of this
    /// field, so type erasure of the slot (raw `*mut u8` + drop fn + `TypeId`,
    /// no inline `R`) is NOT what makes it sound — it only means the field does
    /// not add a fresh `!Send` auto-trait obligation. The `!Send` payload is
    /// sound ONLY by the runtime CpuExclusive-routing discipline: a NonSend
    /// `SystemParam` declares universal access → `CpuExclusive` →
    /// `runs_on_dispatcher()` → solo when `running == 0`, so the value is only
    /// ever touched single-threaded on the dispatcher, reachable through the
    /// `unsafe` `NonSendRes`/`NonSendResMut::get_param` accessors. There is no
    /// compile-time tripwire enforcing the routing — the behavioral test
    /// `nonsend_system_runs_on_dispatcher_and_observes_resource` is the guard.
    /// See the SEND10 bullet on the `unsafe impl Send for EcsMaster`.
    pub(crate) nonsend_resources: Option<Box<NonSendResources>>,

    /// Event dispatcher — dropped after `resources` and before the entity /
    /// archetype subsystems. Event buffers live in their own heap allocations
    /// independent of component storage.
    ///
    /// `pub(crate)` so the `event_api` sibling module's `impl EcsMaster` half can
    /// touch the field directly, as the pre-split single-file `impl` did. Still
    /// opaque out-of-crate; the public [`Self::events`] / [`Self::events_mut`]
    /// accessors are unchanged.
    pub(crate) events: EventDispatcher,

    /// Entity management system.
    ///
    /// Phase 11 Round 3 (C-N1): `pub(crate)` so
    /// [`crate::ecs::core::system::unsafe_ecs_cell::UnsafeEcsCell::entity_counter`]
    /// can project `next_id_atomic()` without going through the `&self`
    /// borrow that the public accessor would impose. The field is still
    /// opaque to out-of-crate consumers — the only worker-side view is
    /// the [`crate::ecs::core::system::params::entity_counter::EntityCounter<'s>`]
    /// newtype (EM6).
    pub(crate) entity_master: EntityMaster,

    /// Archetype management system.
    ///
    /// `pub(crate)` so the topic-grouped `impl EcsMaster` halves in sibling
    /// modules (`component_api`, `observer_api`, `entity_api`, …) can touch the
    /// field directly, exactly as the pre-split single-file `impl` did. Still
    /// opaque to out-of-crate consumers; the public accessors
    /// [`Self::archetype_master`] / [`Self::archetype_master_mut`] are unchanged.
    pub(crate) archetype_master: ArchetypeMaster,

    /// Dense (non-fragmenting) storage subsystem (Dense plan D2). Owns one
    /// [`DenseStore`](crate::ecs::core::component::dense::DenseStore) per dense
    /// `ComponentId`, created lazily on first insert. A `DenseStore` owns a
    /// `ComponentPool` (a raw VM reservation) and is therefore `!Send`, so it
    /// cannot live in the `Send` `Resource` slab — it lives here, single-threaded
    /// behind `&mut EcsMaster` on the structural path exactly like
    /// `archetype_master`.
    ///
    /// **0%-gate**: a world that defines no dense component never creates a store
    /// — the registry is alloc-free until the first dense insert, and the
    /// despawn-path membership walk over its (empty) id list runs zero turns.
    ///
    /// **Drop-order**: declared after `archetype_master`. A `DenseStore` owns its
    /// rows outright (its own `Drop` runs each live component's `drop_fn`) and
    /// holds no pointer into the archetype slab, so its drop position relative to
    /// the archetype subsystem is not a correctness hazard.
    ///
    /// `pub(crate)` so the structural-op routing (spawn / insert / remove /
    /// despawn / clone) can mutate the stores directly.
    pub(crate) dense_registry: crate::ecs::core::component::dense::DenseRegistry,

    /// Per-bundle-type `ArchetypeId` cache (Phase 8.5 SBC5). Indexed by
    /// `BundleTypeId.0`.
    ///
    /// Phase 12.6 — wrapped in `OnceLock` for lazy allocation. The 24 KB
    /// inner array (`Box<[OnceLock<ArchetypeId>; MAX_BUNDLE_TYPES]>`) is
    /// only materialised on the first `bundle_archetype_id_for::<B>()`
    /// call, removing ~30-50 µs of eager `core::array::from_fn` work
    /// from `EcsMaster::new`. Steady-state warm-path cost is unchanged
    /// (one Acquire load on the outer lock once it is initialised, then
    /// the same indexed slot Acquire load as before).
    ///
    /// **Field slot (C6 pin)**: declared after `archetype_master`. The field
    /// holds only `OnceLock<ArchetypeId>` values — no resource ownership and
    /// no `Drop` side-effects — so the drop position is informational only
    /// and does not interact with the Phase 8a C5 drop-order contract.
    ///
    /// Access via [`Self::bundle_archetype_cache`].
    #[allow(dead_code)]
    bundle_archetype_cache: OnceLock<Box<[OnceLock<ArchetypeId>; MAX_BUNDLE_TYPES]>>,

    /// Phase 12.5 Opt-A3 (§6.2): per-world cache of resolved
    /// `(BundleTypeId, ArchetypeId, &'static [InlandPoolId])` records.
    ///
    /// Phase 12.6 — wrapped in `OnceLock` for lazy allocation. The inner
    /// `BundleColumnCache` allocation (≤ 48 KB) is only materialised on
    /// the first spawn-path apply, removing ~30-50 µs of eager
    /// `Vec::extend((0..MAX_BUNDLE_TYPES).map(...))` from `EcsMaster::new`.
    /// Warm-path apply cost is unchanged once the inner cache is
    /// initialised; the spawn-path code reads through
    /// [`Self::bundle_column_cache`] which performs one Acquire load on
    /// the outer lock followed by the same indexed slot lookup as before.
    ///
    /// Drop-order note: declared after `bundle_archetype_cache` and
    /// before `change_tick`. The `&'static [InlandPoolId]` slices leaked
    /// inside `BundleColumnRecord` are deliberately not freed on world
    /// drop — bounded by SBO6 (one slice per `(BundleTypeId, ArchetypeId)`
    /// per world).
    ///
    /// Access via [`Self::bundle_column_cache`].
    pub(crate) bundle_column_cache: OnceLock<BundleColumnCache>,

    /// Phase 10 monotonic per-frame counter for change detection.
    ///
    /// Bumped once per [`Schedule::run`] via `fetch_add(1, Relaxed)` (Wave D
    /// Step 13 integration). The new value becomes the dispatcher-wide
    /// `this_run`; every system's previous `this_run` becomes its new
    /// `last_run` via `System::set_change_ticks`.
    ///
    /// # Round 2 O2 — NOT `CachePadded`
    ///
    /// The atomic is touched at most a handful of times per frame
    /// (`fetch_add` at frame start, `load` inside `EcsMaster::create_entity`).
    /// False-sharing risk against neighbouring fields is essentially zero
    /// at that contention level; `CachePadded` would burn 60 B for no
    /// measurable gain (plan §11.5 audit).
    ///
    /// Wave A: declared `pub(crate)` so the dispatcher (Wave D Step 13)
    /// can call `fetch_add` directly. Outside the crate, read through
    /// [`Self::current_tick`] only.
    pub(crate) change_tick: AtomicU32,

    /// Last [`Tick`] at which `check_ticks` scanned the world. Initialised
    /// to [`Tick::ZERO`]; updated by `Schedule::run` (Wave D Step 13) after
    /// each [`run_check_ticks_scan`] call via [`Self::set_last_check_tick`].
    /// Read every frame by [`Self::should_run_check_ticks`] to decide when
    /// the next clamp scan must fire (plan §2.7 WRAP1-WRAP2).
    ///
    /// [`run_check_ticks_scan`]: crate::ecs::core::change_detection::run_check_ticks_scan
    pub(crate) last_check_tick: Tick,

    /// Phase 14a (plan §2.3) — world-resident channel a hook's
    /// `DeferredCommands` enqueues into. Reachable via `&mut EcsMaster` (which
    /// the outermost apply holds), unlike the borrow-frozen per-system
    /// `Commands` queue. Lazy: `CommandQueue::new()` is allocation-free
    /// (command_queue.rs:87) until first push, so a world whose hooks never
    /// enqueue pays zero allocation.
    ///
    /// **Drop-order**: declared late (plan §2.3), mirroring the
    /// `query_state_cache` C5 placement. Stored commands are `Command: Send +
    /// 'static`, so they cannot borrow component storage — the drop order
    /// relative to the pools is not a correctness hazard.
    ///
    /// `pub(crate)` so `DeferredCommands` (hooks module) can `push` and
    /// `drain_deferred_hook_queue` can derive the raw twin.
    pub(crate) deferred_hook_queue: CommandQueue,

    /// Phase 12.5 Track B (QC5 / QC6 / C5) — per-`(D, F)` cached
    /// `QueryState` slot array.
    ///
    /// Each slot holds a type-erased `(NonNull<()>, fn(NonNull<()>))` pair:
    /// the cached `NonNull<UnsafeCell<QueryDataState<D, F>>>` plus a
    /// monomorphised drop glue pointer.
    ///
    /// Phase 12.6 — wrapped in `OnceLock` for lazy allocation. The inner
    /// `QueryStateCache` allocation (≤ 32 KB) is only materialised on
    /// the first `EcsMaster::query::<D, F>()` call, removing ~30-50 µs
    /// of eager `Vec::extend((0..MAX_QUERY_TYPES).map(...))` from
    /// `EcsMaster::new`. Worlds that never call `query` (e.g. pure
    /// `Schedule::run`-driven workloads) skip this allocation entirely.
    ///
    /// **Field slot (C5 fix)**: declared LAST. Rust drops fields in
    /// declaration order, so this field is dropped after the archetype
    /// subsystem (whose pools own the component reservations). Inverts the
    /// failure mode for any future `D::State` / `F::State` impl carrying
    /// storage-derived raw pointers from silent miscompile to immediate Miri
    /// trip: state pointers freed before the backing reservation would now
    /// trigger a Miri use-after-free instead of silently using the freed
    /// allocation.
    ///
    /// If no `query` call ever populates the outer lock, `OnceLock::drop`
    /// is a no-op — the inner `QueryStateCache::drop` (which walks every
    /// slot and invokes per-slot drop glue) never runs and the field is
    /// drop-cost-free.
    ///
    /// `pub(crate)` so `query` / `query_cold_init` can index without going
    /// through a public accessor.
    pub(crate) query_state_cache: OnceLock<QueryStateCache>,

    /// Feature 2 — per-world entity-targeted observer store (lazy `Option<Box>`).
    ///
    /// Holds only POD (`SparseMap<u32>` handles + fn-ptr-as-`usize` entries), so
    /// it carries no storage-derived pointer and its drop order relative to the
    /// pools is not a hazard. `pub(crate)` so the cold `fire_entity_observers` /
    /// `fire_entity_triggers` dispatch can re-derive `&world.entity_observers`.
    pub(crate) entity_observers: EntityObserverStore,

    /// Feature 2 — per-world global custom-trigger observer registry (lazy
    /// `Option<Box>`). fn-ptr-only payload; `pub(crate)` so the cold trigger
    /// walk can re-derive `&world.triggers`.
    pub(crate) triggers: TriggerRegistry,
}

/// Per-slot payload stored in [`QueryStateCache`]: a type-erased pointer
/// to the cached `Box<UnsafeCell<QueryDataState<D, F>>>` plus a
/// monomorphised drop glue function pointer. The drop fn is installed at
/// `query_cold_init` time so `QueryStateCache::drop` can reclaim the
/// leaked Box without knowing the concrete `(D, F)` pair.
pub(crate) type QueryCacheSlot = (NonNull<()>, fn(NonNull<()>));

/// Phase 12.5 Track B — per-world cache of `QueryState<D, F>` slots indexed
/// by `QueryTypeId`.
///
/// Each slot stores a [`QueryCacheSlot`] — type-erased pointer to a
/// heap-allocated `UnsafeCell<QueryDataState<D, F>>` plus a per-type drop
/// glue function pointer. Eagerly allocated to `MAX_QUERY_TYPES` slots at
/// world construction.
///
/// # Memory footprint (§10.3)
///
/// ≤ 32 KB at `MAX_QUERY_TYPES = 1024` (1024 × ≤ 32 B per slot pinned by
/// the `oncelock_query_slot_size_assumptions` tripwire). ≤ 128 KB with the
/// `big_query_table` feature (4096 slots).
///
/// # Drop
///
/// Walks every slot; if a slot holds a `Some((typed_ptr, drop_fn))`,
/// invokes `drop_fn(typed_ptr)`. The drop fn reconstructs the original
/// `Box<UnsafeCell<QueryDataState<D, F>>>` via `Box::from_raw` and lets it
/// drop normally (running the embedded `QueryDataState::Drop` glue).
pub(crate) struct QueryStateCache {
    slots: Box<[OnceLock<QueryCacheSlot>]>,
}

// SAFETY (QC9):
//   - The slot tuples hold `NonNull<()>` (an opaque address into the world's
//     own heap allocations — guarded by `&mut EcsMaster` for mutation) and a
//     `fn(NonNull<()>)` (function pointer, trivially `Send + Sync`).
//   - `OnceLock<T>` is `Send + Sync` whenever `T: Send + Sync`.
//   - Slot CAS soundness is provided by `OnceLock`'s internal atomics.
//   - The pointee is a `Box<UnsafeCell<QueryDataState<D, F>>>` whose
//     concrete `D::State` / `F::State` carry `Send + Sync + 'static` per
//     `QueryData::State` / `QueryFilter::State` trait bounds.
unsafe impl Send for QueryStateCache {}
// SAFETY: same composition as `Send`; the cache offers no `&self` mutation
// path (slot writes go through `&mut self` via `EcsMaster::query_cold_init`).
unsafe impl Sync for QueryStateCache {}

impl QueryStateCache {
    /// Allocates the per-world cache eagerly, with every slot in the
    /// `OnceLock::new()` (empty) state.
    ///
    /// Per the plan's C3 fix (Round 2): allocate via `Vec::with_capacity` +
    /// `extend` + `into_boxed_slice` instead of
    /// `Box::new(core::array::from_fn(...))`. The latter constructs the
    /// array on the stack first and copies it into the heap; for a 32 KB
    /// array this would risk stack overflow on small-stack threads and
    /// thrash L1 unnecessarily.
    #[inline]
    fn new() -> Self {
        let mut v: Vec<OnceLock<QueryCacheSlot>> =
            Vec::with_capacity(MAX_QUERY_TYPES);
        v.extend((0..MAX_QUERY_TYPES).map(|_| OnceLock::new()));
        let slots: Box<[OnceLock<QueryCacheSlot>]> = v.into_boxed_slice();
        debug_assert_eq!(slots.len(), MAX_QUERY_TYPES);
        Self { slots }
    }

    /// Returns the slot for `id`. Bounds checked in debug builds via the
    /// `MAX_QUERY_TYPES` saturation on the minter side.
    #[inline]
    fn slot(&self, id: QueryTypeId) -> &OnceLock<QueryCacheSlot> {
        debug_assert!(id.0 < MAX_QUERY_TYPES, "QueryTypeId out of bounds");
        // SAFETY: `id.0 < MAX_QUERY_TYPES` is enforced by the minter's
        //   saturate-then-panic discipline; the slot array was sized to
        //   `MAX_QUERY_TYPES` at construction time.
        unsafe { self.slots.get_unchecked(id.0) }
    }
}

impl Drop for QueryStateCache {
    fn drop(&mut self) {
        for slot in self.slots.iter() {
            if let Some(&(typed_ptr, drop_fn)) = slot.get() {
                // SAFETY (QC7): the slot was populated by
                //   `EcsMaster::query_cold_init` with a monomorphised
                //   `drop_fn` for the concrete `(D, F)` pair; the drop fn
                //   reconstructs the original
                //   `Box<UnsafeCell<QueryDataState<D, F>>>` via
                //   `Box::from_raw` and lets it drop normally. The slot is
                //   consumed exactly once (by this Drop impl) over the
                //   world's lifetime — no double-free.
                drop_fn(typed_ptr);
            }
        }
    }
}

impl EcsMaster {
    /// Creates a new empty EcsMaster.
    ///
    /// # Phase 12.6 — lazy allocation
    ///
    /// The heavy per-world allocations (`entities_inland` fast-store memset,
    /// `bundle_archetype_cache`, `bundle_column_cache`, `query_state_cache`)
    /// are all deferred to first-use. (Phase X.D removed the parallel
    /// `sparse_to_active` fast-store.)
    ///
    /// # Phase X.I — per-pool reserve-lazy backing store
    ///
    /// Component storage is acquired per `ComponentPool`, RESERVE-ONLY:
    /// each pool holds one virtual-address reservation with ZERO initial
    /// commit — no commit syscall, no commit charge. The first row commits
    /// the first slab at the frontier; subsequent growth is one syscall per
    /// slab, never an O(N) move. Miri / wasm32 / exotic targets fall back
    /// to one eager global-allocator allocation per pool (small D2 fallback
    /// ceilings). (Phase X.J retired the historical shared Arena.)
    pub fn new() -> Self {
        let archetype_master = ArchetypeMaster::new();
        // EventDispatcher::new(1) validates 1 ∈ 1..=64 — never fails.
        let events = EventDispatcher::new(1)
            .expect("invariant: default thread_count=1 is always valid");
        // Phase 12.6 — entity fast-store starts empty. Growth is driven by:
        //   * single-row paths (`register_entity_with_ptr`, `create_entity_at`)
        //     which `Vec::resize` on demand under `&mut self`.
        //   * batch paths (`EcsMaster::spawn_batch`, `SpawnBatchCommand::apply`)
        //     which call `EntityMaster::ensure_capacity` BEFORE the apply
        //     window (dispatcher-only, no worker reads in flight).
        Self {
            world_id: WorldId::mint(),
            resources: Resources::new(),
            // Phase 4 — lazy: alloc-free until the first NonSend resource insert.
            nonsend_resources: None,
            events,
            entity_master: EntityMaster::new(),
            archetype_master,
            // Dense plan D2 — alloc-free until the first dense insert (0%-gate).
            dense_registry: crate::ecs::core::component::dense::DenseRegistry::new(),
            bundle_archetype_cache: OnceLock::new(),
            bundle_column_cache: OnceLock::new(),
            change_tick: AtomicU32::new(0),
            last_check_tick: Tick::ZERO,
            // Phase 14a: lazy — alloc-free until the first deferred hook push.
            // The reentrancy depth counter is a thread-local (see hooks::scope),
            // so it is not a field here (fixes F2's Tree Borrows UB).
            deferred_hook_queue: CommandQueue::new(),
            query_state_cache: OnceLock::new(),
            // Feature 2: lazy — alloc-free until the first entity observer /
            // custom-trigger observer registration.
            entity_observers: EntityObserverStore::new(),
            triggers: TriggerRegistry::new(),
        }
    }

    /// Creates a new EcsMaster with pre-allocated capacity.
    ///
    /// Phase 12.6 — the per-world cache arrays
    /// (`bundle_archetype_cache`, `bundle_column_cache`,
    /// `query_state_cache`) remain lazy. `entity_capacity` reserves but
    /// does NOT memset the entity fast-store vectors; the actual memset
    /// happens on the first dispatcher growth call. Callers that need
    /// the fast-store pre-extended (e.g. test fixtures asserting the
    /// SBO17 strong-form contract) should call `ensure_capacity`
    /// explicitly after construction.
    pub fn with_capacity(entity_capacity: usize, archetype_capacity: usize) -> Self {
        let archetype_master = ArchetypeMaster::with_capacity(archetype_capacity);
        // EventDispatcher::new(1) validates 1 ∈ 1..=64 — never fails.
        let events = EventDispatcher::new(1)
            .expect("invariant: default thread_count=1 is always valid");
        Self {
            world_id: WorldId::mint(),
            resources: Resources::new(),
            // Phase 4 — lazy: alloc-free until the first NonSend resource insert.
            nonsend_resources: None,
            events,
            entity_master: EntityMaster::with_capacity(entity_capacity),
            archetype_master,
            // Dense plan D2 — alloc-free until the first dense insert (0%-gate).
            dense_registry: crate::ecs::core::component::dense::DenseRegistry::new(),
            bundle_archetype_cache: OnceLock::new(),
            bundle_column_cache: OnceLock::new(),
            change_tick: AtomicU32::new(0),
            last_check_tick: Tick::ZERO,
            // Phase 14a: lazy — alloc-free until the first deferred hook push.
            // The reentrancy depth counter is a thread-local (see hooks::scope),
            // so it is not a field here (fixes F2's Tree Borrows UB).
            deferred_hook_queue: CommandQueue::new(),
            query_state_cache: OnceLock::new(),
            // Feature 2: lazy — alloc-free until the first entity observer /
            // custom-trigger observer registration.
            entity_observers: EntityObserverStore::new(),
            triggers: TriggerRegistry::new(),
        }
    }

    // ── Phase 12.6 lazy cache accessors ─────────────────────────────────────

    /// Returns the per-world bundle-archetype-id cache, materialising the
    /// inner 24 KB array on first call.
    ///
    /// Hot path: one Acquire load on the outer `OnceLock`, then the
    /// indexed slot Acquire load that was already on the spawn warm path.
    /// Cold path (first call per world): one `Box<[OnceLock<ArchetypeId>;
    /// MAX_BUNDLE_TYPES]>` heap allocation (~30-50 µs) — amortised across
    /// the world's lifetime.
    ///
    /// `#[inline]` so cross-crate callers see the body and avoid a
    /// function-call hop on the warm path.
    #[inline]
    pub(crate) fn bundle_archetype_cache(
        &self,
    ) -> &[OnceLock<ArchetypeId>; MAX_BUNDLE_TYPES] {
        self.bundle_archetype_cache
            .get_or_init(|| Box::new(core::array::from_fn(|_| OnceLock::new())))
    }

    /// Returns the per-world bundle-column cache, materialising the
    /// inner ~48 KB slot array on first call.
    ///
    /// Hot path: one Acquire load on the outer `OnceLock`. Cold path
    /// (first call per world): `BundleColumnCache::new` allocation
    /// (~30-50 µs).
    ///
    /// `#[inline]` so the spawn-batch / spawn-at apply hot path inlines
    /// through the accessor without a call hop.
    #[inline]
    pub(crate) fn bundle_column_cache(&self) -> &BundleColumnCache {
        self.bundle_column_cache.get_or_init(BundleColumnCache::new)
    }

    /// Returns the per-world query-state cache, materialising the inner
    /// ~32 KB slot array on first call.
    ///
    /// Hot path: one Acquire load on the outer `OnceLock`. Cold path
    /// (first call per world): `QueryStateCache::new` allocation
    /// (~30-50 µs).
    #[inline]
    pub(crate) fn query_state_cache(&self) -> &QueryStateCache {
        self.query_state_cache.get_or_init(QueryStateCache::new)
    }

    /// Returns this world's process-unique [`WorldId`] (Phase 21).
    ///
    /// Minted once at construction and never reused within a process. A
    /// [`Schedule`](crate::ecs::core::schedule::schedule::Schedule) is bound
    /// to the world it was built on via this id; `Schedule::run` panics on a
    /// mismatch.
    #[inline]
    pub fn world_id(&self) -> WorldId {
        self.world_id
    }

    /// Gets a reference to the EntityMaster
    #[inline]
    pub fn entity_master(&self) -> &EntityMaster {
        &self.entity_master
    }

    /// Gets a mutable reference to the EntityMaster
    #[inline]
    pub fn entity_master_mut(&mut self) -> &mut EntityMaster {
        &mut self.entity_master
    }

    /// Gets a reference to the ArchetypeMaster
    #[inline]
    pub fn archetype_master(&self) -> &ArchetypeMaster {
        &self.archetype_master
    }

    /// Gets a mutable reference to the ArchetypeMaster
    #[inline]
    pub fn archetype_master_mut(&mut self) -> &mut ArchetypeMaster {
        &mut self.archetype_master
    }

    /// Returns a shared reference to the dense storage subsystem (Dense plan
    /// D3). The query path reads it to resolve a dense term's global
    /// `DenseStore` (column geometry + `e2s` membership oracle) once at fetch
    /// construction. Empty for a table-only world (the 0%-gate).
    #[inline]
    pub fn dense_registry(&self) -> &crate::ecs::core::component::dense::DenseRegistry {
        &self.dense_registry
    }

    /// Returns a mutable reference to the dense storage subsystem (Dense plan
    /// D3). The structural-op routing already mutates the stores through
    /// `self.dense_registry` directly; this accessor exposes the same surface
    /// to external structural callers (e.g. the physics build phase).
    #[inline]
    pub fn dense_registry_mut(&mut self) -> &mut crate::ecs::core::component::dense::DenseRegistry {
        &mut self.dense_registry
    }

    // ── Event dispatch proxy methods (Phase 6) ──────────────────────────────

    /// Returns a shared reference to the event dispatcher.
    #[inline]
    pub fn events(&self) -> &EventDispatcher {
        &self.events
    }

    /// Returns a mutable reference to the event dispatcher.
    #[inline]
    pub fn events_mut(&mut self) -> &mut EventDispatcher {
        &mut self.events
    }

    // ── Phase 10 change_tick facade (Wave A Step 2) ─────────────────────────

    /// Returns the world's current change-detection tick.
    ///
    /// Reads `Self::change_tick` with `Ordering::Relaxed` — sufficient
    /// per plan §8.1: the per-row tick writes that consume this value are
    /// synchronised via the Phase 9 conflict graph (SCH3), not via the
    /// atomic. The fetch returns a single `u32` (no compound state).
    ///
    /// Wave B's `Archetype::create_entity` reads this via the canonical
    /// path: `EcsMaster::create_entity` (Round 2 W4 INIT3 — the tick is
    /// owned by the world, not threaded through user APIs).
    #[inline]
    pub fn current_tick(&self) -> Tick {
        Tick::new(self.change_tick.load(Ordering::Relaxed))
    }

    /// Atomically increments [`Self::change_tick`] and returns the NEW value.
    ///
    /// Used by `Schedule::run` at frame start (Wave D Step 13) — the
    /// single dispatcher-owned bump site. `fetch_add(1, Relaxed)` returns
    /// the PREVIOUS value, so we wrap-add 1 to obtain the new `this_run`.
    ///
    /// Visibility is `pub(crate)` — only the scheduler is permitted to
    /// bump; user code reads via [`Self::current_tick`] only.
    #[inline]
    pub(crate) fn bump_change_tick(&self) -> Tick {
        let prev = self.change_tick.fetch_add(1, Ordering::Relaxed);
        Tick::new(prev.wrapping_add(1))
    }

    // ── Phase 14a deferred-hook plumbing (plan §2.2 / §2.3 / §8 P1 / §8 P2) ──

    /// Drains the world-resident deferred-hook queue at the OUTERMOST apply
    /// boundary (plan §8 P2 / Q-A1). Re-entrant: hooks fired during the drain
    /// enqueue into the SAME queue; the loop's transient `is_empty()` re-reads
    /// `bytes.len()` so re-entrant appends are picked up.
    ///
    /// The depth gate is the whole correctness story: a nested call (inside a
    /// `CommandQueue::apply` at `depth > 0`) returns immediately, so the single
    /// outermost owner drains exactly once after the per-system apply returns
    /// (the C1 single-`catch_unwind` proof). The depth lives in a thread-local
    /// (see [`hooks::scope`]); the bracket guards are wired into the three
    /// direct-API methods + the two schedule `system.apply` sites.
    ///
    /// [`hooks::scope`]: crate::ecs::core::component::hooks::scope
    pub(crate) fn drain_deferred_hook_queue(&mut self) {
        // Q-A1 gate (via the thread-local depth): only the outermost owner
        // drains. A nested call (inside an `apply_via_raw_twin` that routes a
        // hook-enqueued command through a self-draining direct-API method)
        // observes `depth >= 1` and returns immediately.
        if hook_drain_depth() != 0 {
            return;
        }
        // SAFETY-7 (W4): hooks + their deferred commands run OUTSIDE the
        // system-body allocation-discipline window. Verified: InSystemRunGuard
        // wraps only `run_unsafe` (tls.rs:152-159; schedule.rs created/dropped
        // before `apply`). If a future refactor moved `apply` inside the guard,
        // this tripwire fires.
        debug_assert!(
            !boyko_threadpool::is_in_system_run(),
            "SAFETY-7: hook drain must run with IN_SYSTEM_RUN == false"
        );

        // F1 fix: bracket the drain's OWN walk (depth 0 -> 1). Any command we
        // apply that routes through a self-draining direct-API method
        // (`delete_entity` / `create_entity` / `create_entity_at`) now sees
        // `depth >= 1` and no-ops its own nested drain, so the in-flight queue
        // is applied exactly once. Re-entrant appends during the walk are still
        // picked up by the `while !is_empty()` loop below (the guard holds
        // depth 1 throughout). The guard touches only TLS — it caches no
        // `NonNull<EcsMaster>`, so it cannot be frozen by the `&mut *self`
        // reborrow on the next line (F2 invariant).
        let _scope = DeferredScopeGuard::enter();

        let world_ptr: NonNull<EcsMaster> = NonNull::from(&mut *self);
        // W4 / C1 — cross-level cascade backstop. The `LINKED_DESPAWN` cascade
        // recurses through THIS flat queue, not the call stack: each cascade level
        // enqueues despawns that surface as the NEXT drain turn. A *cyclic*
        // `LINKED_DESPAWN` graph terminates naturally (a re-entered despawn of an
        // already-freed entity is a generation-checked no-op in
        // `delete_entity_core`, so the live set strictly shrinks per real
        // despawn), so this counter is a BLUNT backstop against a *pathological*
        // non-terminating re-enqueue — not the primary cycle-termination
        // mechanism. The bound lives HERE (the flat drain) because the per-hook
        // RAII depth guard could never accumulate across turns (every cascade
        // level fired at depth 1). `turns` is a loop-local `usize` reset per
        // outermost drain — ZERO cost on the 0%-gate (a register increment only
        // when a turn actually runs) and relation-agnostic (no `LINKED_DESPAWN`
        // const-fold dependence; the loop is not in a hook body).
        let mut turns = 0usize;
        loop {
            // Transient shared borrow only for the emptiness test; dropped at
            // the `;`. The twin re-reads `bytes.len()` each turn, so re-entrant
            // appends pushed during a prior `apply_via_raw_twin` are seen here.
            // SAFETY: `world_ptr` is valid + exclusive (`&mut self` at the call
            //   site); the borrow does not escape the `if`.
            if unsafe { (*world_ptr.as_ptr()).deferred_hook_queue.is_empty() } {
                break;
            }
            turns += 1;
            if turns > crate::ecs::constants::MAX_HOOK_DRAIN_TURNS {
                drain_runaway_panic();
            }
            // SAFETY (SAFETY-5 / SAFETY-6): full catch + recovery semantics; no
            //   `&mut`-into-queue is held across the per-command `&mut *world`
            //   (proven in `apply_via_raw_twin`'s SAFETY contract). The world
            //   pointer stays exclusively ours for the call.
            unsafe { CommandQueue::apply_via_raw_twin(world_ptr); }
        }
    }

    /// Returns `true` iff `current_tick - last_check_tick >= CHECK_TICK_THRESHOLD`.
    ///
    /// Called every frame from `Schedule::run` (plan §2.7 WRAP2). Returning
    /// `true` means the dispatcher should next invoke the cold-path
    /// [`run_check_ticks_scan`](crate::ecs::core::change_detection::run_check_ticks_scan)
    /// to clamp aged-out per-row ticks against [`MAX_CHANGE_AGE`].
    ///
    /// `wrapping_sub` is the correct elapsed computation under the §9.3
    /// wraparound discipline: stored ticks stay within `MAX_CHANGE_AGE` of
    /// the current tick by construction, so the unsigned subtraction yields
    /// the true elapsed count (mod `u32::MAX`, which `MAX_CHANGE_AGE +
    /// CHECK_TICK_THRESHOLD < u32::MAX` keeps faithful).
    ///
    /// [`MAX_CHANGE_AGE`]: crate::ecs::core::change_detection::MAX_CHANGE_AGE
    #[inline]
    pub(crate) fn should_run_check_ticks(&self) -> bool {
        let current = self.current_tick();
        let elapsed = current.get().wrapping_sub(self.last_check_tick.get());
        elapsed >= CHECK_TICK_THRESHOLD
    }

    /// Margin-aware variant of [`Self::should_run_check_ticks`] for the
    /// App-level all-schedule clamp pass (Phase 20 ★C1 / D8).
    ///
    /// Returns `true` iff `current_tick - last_check_tick >=
    /// CHECK_TICK_THRESHOLD - margin`, i.e. it fires `margin` ticks EARLIER
    /// than the per-schedule internal blocks in `Schedule::run`. Both paths
    /// read the SAME counter, but the App checks at frame start (before any
    /// bump) while each internal block checks after its own frame-start bump
    /// — without the margin the first internal block to bump would win most
    /// threshold crossings, clamp only its own systems, reset the shared
    /// counter, and starve the SIBLING schedule's clamp (the
    /// `Tick::is_newer_than` wraparound class). The margin guarantees the App
    /// pass crosses its earlier threshold at a frame start strictly before
    /// any internal block can reach the full threshold mid-frame, as long as
    /// one frame consumes fewer than `margin` ticks (`debug_assert`ed in
    /// `App::update_with_delta`).
    ///
    /// The canonical `margin` is
    /// [`CHECK_TICK_PREEMPT_MARGIN`](crate::ecs::core::change_detection::CHECK_TICK_PREEMPT_MARGIN).
    #[inline]
    pub(crate) fn should_run_check_ticks_with_margin(&self, margin: u32) -> bool {
        debug_assert!(
            margin < CHECK_TICK_THRESHOLD,
            "invariant: the preempt margin must be a strict sliver of CHECK_TICK_THRESHOLD"
        );
        let current = self.current_tick();
        let elapsed = current.get().wrapping_sub(self.last_check_tick.get());
        elapsed >= CHECK_TICK_THRESHOLD - margin
    }

    /// Records that the world's stored ticks have just been clamped against
    /// [`Tick`] = `tick`.
    ///
    /// Called by `Schedule::run` after [`run_check_ticks_scan`] returns
    /// (plan §2.7 WRAP1). Resets the wraparound counter so the next scan
    /// fires another `CHECK_TICK_THRESHOLD` ticks later.
    ///
    /// Visibility is `pub(crate)` — only the scheduler / change_detection
    /// machinery is permitted to update this.
    ///
    /// [`run_check_ticks_scan`]: crate::ecs::core::change_detection::run_check_ticks_scan
    #[inline]
    pub(crate) fn set_last_check_tick(&mut self, tick: Tick) {
        self.last_check_tick = tick;
    }

    /// Phase 12.5 Track B Opt-B1 — direct query API.
    ///
    /// Returns a [`QueryView<'_, D, F>`] handle exposing `iter`, `iter_mut`,
    /// `single`, `single_mut`, `get`, `get_mut`, `par_iter`, `par_iter_mut`.
    /// Bypasses the [`FunctionSystem`] wrapper used by the in-system
    /// [`Query<D, F>`](crate::ecs::core::iters::query::query::Query)
    /// SystemParam — no `FilteredAccessSet` allocation, no per-call
    /// `QueryDataState::new`, no apply pass. Aliasing is gated at the type
    /// level by `&mut self`.
    ///
    /// # Cost
    ///
    /// * First call for a given `(D, F)` pair: ~1 µs cold cost
    ///   (`QueryDataState::new` + `OnceLock::set`).
    /// * Subsequent calls: ~5 ns cache hit (per plan §6.1 breakdown — one
    ///   per-impl `OnceLock::get` + one slot `OnceLock::get` + a
    ///   `state.update(master)` warm short-circuit).
    ///
    /// # Compile errors (I-NEW-4 / QV11 / W4 canonical)
    ///
    /// Fails to compile if `D::NEEDS_CHANGE_DETECTION ||
    /// F::NEEDS_CHANGE_DETECTION` is true (i.e. if `D` or `F` contains
    /// `Ref<T>`, `Mut<T>`, `Added<C>`, or `Changed<C>`). Change-detection
    /// requires `Schedule` context; use `Query<D, F>` as a SystemParam inside
    /// a system body via `Schedule`. The check is a `const`-block assertion
    /// evaluated at monomorphisation — it produces a compile error at the
    /// offending call site and const-folds to nothing on the !NCD path.
    ///
    /// [`FunctionSystem`]: crate::ecs::core::system::function_system::FunctionSystem
    pub fn query<D, F>(&mut self) -> QueryView<'_, D, F>
    where
        D: QueryData + 'static,
        F: QueryFilter + 'static,
    {
        // QV11 / I-NEW-4 / W4: change-detection guard. The inline `const {}`
        // block is the CODEGEN-time trigger — it fires at monomorphisation
        // (build / test), producing a compile error at the call site with no
        // trait-bound surface (the EnableTag const-reject idiom). The
        // CHECK-time trigger for `trybuild` `compile_fail` is the public
        // `assert_query_no_change_detection::<D, F>()` in a `const ITEM`
        // context — a generic-fn `const {}` block is NOT evaluated under a
        // metadata-only `cargo check` of an external caller. On the !NCD path
        // the assert is `assert!(true)` and const-folds to nothing, so the hot
        // path is byte-identical and the cold panic-fn reference is gone.
        const { eval_query_no_change_detection::<D, F>() };

        let type_id = <(D, F) as QueryTypeKey>::query_type_id();
        debug_assert!(
            type_id.0 < MAX_QUERY_TYPES,
            "QueryTypeId out of bounds — register_new's saturate-then-panic \
             discipline should have prevented this"
        );

        let slot = self.query_state_cache().slot(type_id);

        if let Some(&(typed_ptr, _drop_fn)) = slot.get() {
            // Hot path: cache hit. Reborrow the cached state under `&mut
            // self`'s exclusive provenance and refresh against the world's
            // archetype master.
            let cell_ptr: NonNull<UnsafeCell<QueryDataState<D, F>>> = typed_ptr.cast();

            // SAFETY (QV6 / I-NEW-3):
            //   - `cell_ptr` was minted from `Box::leak` in
            //     `query_cold_init` and never freed until `Drop`.
            //   - `&mut self` is unique for `'_`; the `&mut` retag below
            //     derives from `self`'s unique provenance, NOT from the
            //     raw `Box::leak + as_mut` of Round 2.
            //   - `UnsafeCell::get` returns `*mut`; we reborrow as `&mut`
            //     for the `state.update(master)` call only and drop the
            //     `&mut` before constructing the `QueryView`.
            unsafe {
                let state_mut: &mut QueryDataState<D, F> =
                    &mut *(*cell_ptr.as_ptr()).get();
                // Phase 22.1 Area A (P2): this is the slot-exclusive mint
                // funnel (`&mut self`). Free the retired term list (if any)
                // here, where no resolve on this slot can be in flight — fast
                // path is one Relaxed null-load + a predicted-not-taken
                // branch (off the row loop; 50-systems bench gated).
                state_mut.term_scratch.reclaim_retired();
                // Dense plan D3: route a dense-include query through the
                // dense-seed path (const-folded to the plain `update` for a
                // no-dense query — the 0%-gate). `dense_registry()` and
                // `archetype_master()` are both `&self` reborrows; `state_mut`'s
                // provenance is the raw `cell_ptr`, independent of `self`.
                state_mut.update_with_world(self.archetype_master(), self.dense_registry());
            }

            // SAFETY (QV1, QV7, U_C1):
            //   - `new_mutable` is sound because `&mut self` enforces world
            //     exclusivity for `'_` (the returned view's lifetime).
            //   - `from_parts` carries the contract that both `world` and
            //     `state` descend from the same `&mut self` borrow.
            let world = unsafe { UnsafeEcsCell::new_mutable(self) };
            return unsafe { QueryView::from_parts(world, cell_ptr) };
        }

        // Cache miss: cold init path.
        self.query_cold_init::<D, F>(type_id)
    }

    /// Relations W1 — the value-carrying twin of [`Self::query`].
    ///
    /// Builds (or reuses) the SAME type-keyed `(D, F)` query as
    /// `query::<D, F>()` — the matched-archetype set is value-INDEPENDENT (a
    /// [`RelatedTo<R>`](crate::ecs::core::iters::query::relation::RelatedTo)
    /// bounds to `R`-hosting archetypes regardless of the runtime target) — then
    /// seeds the cached `filter_state` from the passed `filter` value via the
    /// [`QueryFilter::seed_state`] seam. Only the per-row `filter_fetch`
    /// comparison depends on the runtime value, so the cached set is reused
    /// wholesale.
    ///
    /// For a value-less filter the seam is a no-op (the 0%-gate), so
    /// `query_filtered(With::<C>::default())` is equivalent to `query::<_,
    /// With<C>>()`. The intended use is a runtime-valued filter:
    ///
    /// ```ignore
    /// for t in world
    ///     .query_filtered::<&Transform, _>(RelatedTo::<ChildOf>::new(parent))
    ///     .iter()
    /// { /* per-row match: source's ChildOf FK target == parent */ }
    /// ```
    ///
    /// # Cost
    ///
    /// Identical to [`Self::query`] plus one `seed_state` call (a single field
    /// write for `RelatedTo`, const-folded to nothing for value-less filters).
    ///
    /// # Compile errors
    ///
    /// Same change-detection reject as [`Self::query`] (`D` / `F` carrying
    /// `Ref`/`Mut`/`Added`/`Changed` is a compile error).
    ///
    /// [`QueryFilter::seed_state`]: crate::ecs::core::iters::query::filter::QueryFilter::seed_state
    pub fn query_filtered<D, F>(&mut self, filter: F) -> QueryView<'_, D, F>
    where
        D: QueryData + 'static,
        F: QueryFilter + 'static,
    {
        let mut view = self.query::<D, F>();
        view.seed_filter(&filter);
        view
    }

    /// Cold-path initialiser for [`Self::query`]. Allocates the
    /// `QueryDataState<D, F>` once for `(D, F)` per world; subsequent
    /// `query` calls hit the cache.
    ///
    /// `#[cold] + #[inline(never)]`: runs at most once per `(D, F)` per
    /// world; isolating it keeps the warm path's hot loop tight.
    #[cold]
    #[inline(never)]
    fn query_cold_init<D, F>(&mut self, type_id: QueryTypeId) -> QueryView<'_, D, F>
    where
        D: QueryData + 'static,
        F: QueryFilter + 'static,
    {
        let state = QueryDataState::<D, F>::new(self);
        let cell = Box::new(UnsafeCell::new(state));
        // SAFETY: `Box::leak` produces a `&'static mut UnsafeCell<...>`
        //   from a `Box`; `NonNull::from` is infallible. The `'static`
        //   lifetime is narrowed by the cache's drop fn pointer back to
        //   the world's lifetime (the slot's drop in `QueryStateCache::drop`
        //   reconstructs the `Box`).
        let cell_ptr: NonNull<UnsafeCell<QueryDataState<D, F>>> =
            NonNull::from(Box::leak(cell));
        let type_erased: NonNull<()> = cell_ptr.cast();

        // Monomorphised drop glue. The fn pointer is stable for the
        // process lifetime; `Box::from_raw` reconstructs the original Box
        // and lets it drop normally.
        let drop_fn: fn(NonNull<()>) = |p: NonNull<()>| {
            let typed: NonNull<UnsafeCell<QueryDataState<D, F>>> = p.cast();
            // SAFETY (QC7): invoked from `QueryStateCache::drop`; the
            //   pointer was minted from `Box::leak` on a
            //   `Box<UnsafeCell<QueryDataState<D, F>>>` in this same
            //   function for the same `(D, F)`; reconstructing the Box
            //   resumes ownership and runs the embedded drop glue.
            unsafe { drop(Box::from_raw(typed.as_ptr())); }
        };

        let slot = self.query_state_cache().slot(type_id);
        match slot.set((type_erased, drop_fn)) {
            Ok(()) => {
                // SAFETY: same contract as the cache-hit path in `query`.
                unsafe {
                    let state_mut: &mut QueryDataState<D, F> =
                        &mut *(*cell_ptr.as_ptr()).get();
                    // Dense plan D3: route a dense-include query through the
                    // dense-seed path (const-folds to plain `update` otherwise).
                    state_mut.update_with_world(self.archetype_master(), self.dense_registry());
                }
                // SAFETY (QV1, U_C1): see `query` cache-hit path.
                let world = unsafe { UnsafeEcsCell::new_mutable(self) };
                // SAFETY (QV1): `from_parts` contract upheld — `world` and
                //   `cell_ptr` both descend from `&mut self`.
                unsafe { QueryView::from_parts(world, cell_ptr) }
            }
            Err(_) => {
                // `OnceLock::set` raced under `&mut self` — structurally
                // impossible because the cache mutation path is gated by
                // an exclusive borrow. If we hit this branch in production,
                // a contributor has broken the invariant.
                //
                // SAFETY (cleanup): reclaim Box ownership before panic so
                //   the leaked allocation is dropped on the unwind.
                unsafe { drop(Box::from_raw(cell_ptr.as_ptr())); }
                debug_assert!(
                    false,
                    "OnceLock::set raced under &mut self — impossible"
                );
                panic!("invariant violated: query_state_cache slot raced under &mut self");
            }
        }
    }

    /// Clears all entities and archetypes from the system.
    ///
    /// Also resets the per-world bundle caches (`Self::bundle_archetype_cache`
    /// and `Self::bundle_column_cache`): both hold `ArchetypeId`s /
    /// `InlandPoolId`s resolved against the pre-clear archetype registry, which
    /// `archetype_master.clear()` discards (`next_archetype_id` rolls back to
    /// `ArchetypeId(1)`). Without the reset, respawning a bundle type that was
    /// used before the clear would read a stale id — either unregistered
    /// (panic: "cached_archetype_id returned an unregistered id" class) or
    /// aliased to a *different* post-clear archetype that happens to reuse the
    /// numeric id (silent wrong-archetype writes).
    ///
    /// The query-state cache needs no reset: `ArchetypeMaster::clear()` bumps
    /// `structural_generation`, and `QueryDataState::update` (run on every
    /// `query()` call) performs a full rebuild on mismatch.
    pub fn clear(&mut self) {
        self.entity_master.clear();
        self.archetype_master.clear();
        // Replace (not mutate) the OnceLock wrappers: the next access through
        // `bundle_archetype_cache()` / `bundle_column_cache()` lazily
        // re-materialises an empty cache, exactly like a fresh world
        // (`EcsMaster::new` initialises the same fields the same way — which
        // is also why a SECOND world in the same process never observes
        // another world's cached ids). The warm read path is untouched: same
        // accessors, same Acquire loads, zero added compares — `clear()` is
        // the only writer and it holds `&mut self`.
        //
        // The `&'static [InlandPoolId]` slices leaked by the old
        // `BundleColumnRecord`s stay alive by design: records are `Copy` and
        // the slices may have escaped as `'static` borrows, so reclaiming
        // them would be unsound. The leak per clear() is bounded by the same
        // SBO6 bound that already applies per world
        // (`MAX_BUNDLE_TYPES × MAX_BUNDLE_ARITY × 4 B` worst case).
        self.bundle_archetype_cache = OnceLock::new();
        self.bundle_column_cache = OnceLock::new();
    }
    // ── Phase 12.5 Opt-A2 — direct `spawn_batch` path (§5.5) ───────────────

    /// Phase 12.5 Opt-A2 (§5.5): dispatcher-only direct bulk-spawn.
    ///
    /// `&mut self` precludes concurrent worker access by Rust's borrow
    /// checker; this method is intended for **setup-time** use (fixture
    /// builds, integration tests, world bootstrap). For worker-side bulk
    /// spawns, use [`crate::ecs::core::system::params::commands::Commands::spawn_batch`].
    ///
    /// # Returns
    ///
    /// `Vec<Entity>` of length `n` on success. **W3 documented**: the
    /// direct path returns `Vec<Entity>` for caller ergonomics; this is a
    /// setup-time heap allocation, not a hot-path allocation. The queued
    /// path (`Commands::spawn_batch`) does NOT allocate.
    ///
    /// Typical use: `let players = ecs.spawn_batch(...)?;` at world setup.
    ///
    /// # Errors
    ///
    /// * [`EcsError::SpawnBatchExceedsCapacity`] if `iter.len() > MAX_BATCH_HINT`.
    ///
    /// Phase 12.6 — `WorldEntityCapacityExceeded` is no longer reachable
    /// on this path. The Phase 12.5 SBO17 strong-form check (Relaxed
    /// pre-load + capacity comparison) backed a fixed pre-sized
    /// fast-store; the lazy-growth replacement
    /// (`EntityMaster::ensure_capacity`)
    /// expands the fast-store on demand under `&mut self`, so the
    /// capacity guard inside `SpawnBatchCommand::apply` grows instead of
    /// panicking. Memory exhaustion will surface as an OOM from
    /// `Vec::resize` before this method can produce a logically
    /// undersized world.
    pub fn spawn_batch<B, I>(&mut self, iter: I) -> EcsResult<Vec<Entity>>
    where
        B: Bundle + Send + Sync,
        I: IntoIterator<Item = B>,
        I::IntoIter: ExactSizeIterator + Send + Sync + Unpin + 'static,
    {
        let iter = iter.into_iter();
        let n = iter.len();
        if n == 0 {
            return Ok(Vec::new());
        }

        // PRE-CHECK: MAX_BATCH_HINT cap. Mirrors `reserve_batch`'s own
        // gate but short-circuits before touching the counter at all.
        if n > MAX_BATCH_HINT {
            return Err(EcsError::SpawnBatchExceedsCapacity {
                requested: n,
                max: MAX_BATCH_HINT,
            });
        }

        // Route through `EntityMaster` (C-N2: never poke `next_entity_id`
        // directly — EM6 preserved). With Phase 12.6 lazy growth, no
        // capacity pre-check is needed — `SpawnBatchCommand::apply`
        // grows the fast-store via `ensure_capacity` before writing.
        let range = self.entity_master.reserve_batch(n)?;
        let start_entity = Entity::new(EntityId(range.start), 0);

        // Build an equivalent SpawnBatchCommand and apply inline. The
        // pre-checks above guarantee SBO17b's runtime guard inside
        // `apply` will NOT fire — the apply runs the same code path as
        // the queued command but is panic-free for the direct caller.
        let cmd = crate::ecs::core::commands::spawn_batch_command::SpawnBatchCommand::<
            B,
            I::IntoIter,
        > {
            start_entity,
            count: n as u32,
            _pad: 0,
            iter,
        };
        cmd.apply(self);

        // Materialise the entity-id list for the W3 ergonomic return.
        let mut result = Vec::with_capacity(n);
        for i in 0..n {
            result.push(Entity::new(EntityId(range.start + i), 0));
        }
        Ok(result)
    }

}

/// Phase 12.5 Track B I-NEW-4 / QV11 / W4 — the change-detection reject for
/// [`EcsMaster::query<D, F>()`](EcsMaster::query), shared by both trigger sites.
///
/// A `const fn` so it can be forced from a `const ITEM` context (the check-time
/// trigger) as well as an inline `const {}` block (the codegen-time trigger).
/// The assert fires when `D` or `F` carries `NEEDS_CHANGE_DETECTION = true`
/// (`Ref<T>`, `Mut<T>`, `Added<C>`, or `Changed<C>`). The message is a
/// `&'static str` literal — the offending `(D, F)` is identified by the call
/// site span, not by `type_name` interpolation (which is not `const`).
const fn eval_query_no_change_detection<D, F>()
where
    D: QueryData + 'static,
    F: QueryFilter + 'static,
{
    assert!(
        !(D::NEEDS_CHANGE_DETECTION || F::NEEDS_CHANGE_DETECTION),
        "EcsMaster::query<D, F>() does not support change-detection filters \
         (D or F has NEEDS_CHANGE_DETECTION = true, i.e. Ref/Mut/Added/Changed); \
         use Query<D, F> as a SystemParam inside a system body via Schedule."
    );
}

/// Check-time trigger for the [`EcsMaster::query`] change-detection reject
/// (I-NEW-4 / QV11 / W4).
///
/// A `pub const fn` so an external `trybuild` `compile_fail` test can force the
/// assert in a `const ITEM` context:
///
/// ```ignore
/// const _: () = assert_query_no_change_detection::<Ref<'_, P>, ()>();
/// ```
///
/// A `const fn` call inside a `const _: () = ...` item is eagerly
/// const-evaluated even under a metadata-only `cargo check` — unlike the inline
/// generic-fn `const {}` block in [`EcsMaster::query`], which fires only at
/// codegen. Both triggers are required: neither alone covers every build path
/// (the Phase-12.5 "const must be in a forcing context" lesson, mirrored from
/// `QueryDataState::assert_query_shape`).
pub const fn assert_query_no_change_detection<D, F>()
where
    D: QueryData + 'static,
    F: QueryFilter + 'static,
{
    eval_query_no_change_detection::<D, F>();
}

/// Cold-path panic helper for [`EcsMaster::resource`] / [`EcsMaster::resource_mut`].
///
/// Distinct from `params::diagnostics::missing_resource_panic` (which targets
/// the `SystemParam` `get_param` path) — the wording here points at the
/// direct-call API rather than the system runner.
/// Cold-path panic for the W4 / C1 deferred-hook drain backstop: the re-entrant
/// drain exceeded [`MAX_HOOK_DRAIN_TURNS`] turns, i.e. a hook re-enqueues
/// unboundedly (a pathological non-terminating cascade — a relation that
/// resurrects entities, or a malformed hook). A well-formed cyclic
/// `LINKED_DESPAWN` graph terminates via the already-dead-no-op long before this,
/// so reaching it is a bug, surfaced loudly. Kept off the drain-loop body via
/// `#[cold] #[inline(never)]`.
///
/// [`MAX_HOOK_DRAIN_TURNS`]: crate::ecs::constants::MAX_HOOK_DRAIN_TURNS
#[cold]
#[inline(never)]
fn drain_runaway_panic() -> ! {
    panic!(
        "boyko-ecs: deferred-hook drain exceeded MAX_HOOK_DRAIN_TURNS ({}) — a hook \
         re-enqueues unboundedly (non-terminating LINKED_DESPAWN cascade / resurrecting \
         relation / malformed hook).",
        crate::ecs::constants::MAX_HOOK_DRAIN_TURNS
    );
}

impl Default for EcsMaster {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

// SAFETY (SEND1 — Phase 9 §2.4, §9.1, §9.2): `EcsMaster` becomes `Send + Sync`
// under the Phase 9 contract:
//
//   - Structural allocation is bounded by the ALLOC1 discipline (§2.7):
//     the scheduler invokes it only on the dispatcher thread inside the
//     apply window (§5.4.5.1, SCH7), with all workers drained. (The
//     historical shared `arena: Box<Arena>` field — the original
//     `!Send + !Sync` interior — was retired in Phase X.J; the manual
//     impls remain authoritative for the raw-pointer-bearing subsystems.)
//   - `resources` (`Resources`), `events` (`EventDispatcher`),
//     `entity_master` (`EntityMaster`), `archetype_master` (`ArchetypeMaster`),
//     and `bundle_archetype_cache` (`Box<[OnceLock<ArchetypeId>; _]>`) are
//     each independently `Send + Sync` per SEND3-SEND9. Since Phase 14b
//     `archetype_master` also owns the `ObserverRegistry` (only
//     `Option<Box<[[Vec<ObserverEntry>; MAX_COMPONENTS]; 4]>>` + a `u64`; the
//     entries are fn-ptr POD, unconditionally `Send + Sync`), which is
//     `Send + Sync` by construction with no `unsafe impl` (SEND6).
//   - The apply-window barrier (SCH7 + Round 2 C4) guarantees the dispatcher's
//     `&mut EcsMaster` never aliases any worker-held `UnsafeEcsCell` read; the
//     `ConflictGraph` (SCH3) prevents intra-frame aliasing between concurrently
//     running systems.
//   - F3 (kernel-memory audit): `dense_registry` owns `DenseStore`s whose
//     `s2e` field is a `VmColumn<EntityId>` — `!Send`/`!Sync` by auto-trait
//     (the `NonNull` base + `VmReservation` inside), absorbed by THIS blanket
//     impl, so the argument is carried here. The invariants mirror SEND10 on
//     `Archetype.entity_ids`: the column's `base` is write-once (set in the
//     `&mut`-only cold `grow_to`, stable thereafter), every mutation runs
//     under `&mut DenseStore` reached only through the dispatcher-serialized
//     structural paths (insert/remove routing, `DenseBuildView` — itself
//     deliberately `!Send`), and concurrent worker reads (`s2e()` slice / the
//     `DenseQueryIter` cached base pointer) touch only committed
//     plain-old-data memory below `len` with no interior mutability, gated by
//     the same SCH3/SCH7 discipline the former `Vec<EntityId>` relied on.
//     (`DenseSolveView` carries its OWN `unsafe impl Send/Sync` in
//     `dense/views.rs`; it caches raw pointers and is unaffected by the
//     auto-trait flip.)
//   - Direct (non-scheduler) `&mut EcsMaster` callers (`EcsMaster::create_entity`,
//     `EcsMaster::insert_resource`, etc.) inherit the borrow-checker
//     enforcement; no scheduler invariant applies because no worker is in
//     flight at the language level.
//   - SEND10 (Phase 4 Seam 2 / CR-A — FIX-6 / FSC-I1): `EcsMaster: Send` is
//     forced by THIS blanket `unsafe impl` REGARDLESS of what
//     `nonsend_resources` (`Option<Box<NonSendResources>>`) holds — the impl is
//     unconditional, so type erasure is NOT what makes the field sound. (Erasure
//     of the slot — raw `*mut u8` + drop fn + `TypeId`, never an inline `R` —
//     only means the field does not by ITSELF re-introduce a `!Send` auto-trait
//     obligation that the impl would have to override; it provides no run-time
//     guarantee.) The actual soundness of the `!Send` payload rests ENTIRELY on
//     the runtime CpuExclusive-routing discipline: every NonSend `SystemParam`
//     declares universal access (CR-B) → resolves `SystemKind::CpuExclusive` →
//     `runs_on_dispatcher()` runs it solo on the dispatcher thread when
//     `running == 0`, so the `!Send` value is constructed / projected / dropped
//     single-threaded, never concurrently with a worker. That payload is
//     reachable only through the `unsafe` `NonSendRes`/`NonSendResMut::get_param`
//     accessors (via `UnsafeEcsCell::nonsend_resources[_mut]`), whose SAFETY
//     contract holds ONLY on-dispatcher. There is NO compile-time tripwire that
//     enforces the routing — it is guarded by the behavioral test
//     `nonsend_system_runs_on_dispatcher_and_observes_resource`. SEND1 is
//     unchanged.
//
//     Phase 5 Option C amendment (NSND-THREAD): a hand-written out-of-crate
//     System (the `boyko_render` `GpuSystem`) reaches the `!Send` payload NOT
//     through a public `UnsafeEcsCell` accessor — the Wave-C
//     `pub unsafe fn UnsafeEcsCell::nonsend_resource_mut` was DELETED because it
//     was reachable on the concurrent worker path (C1) and its `'w` return
//     lifetime allowed two aliasing `&mut R` (M1). It now reaches it ONLY through
//     `DispatcherToken::nonsend_resource_mut`, a capability mintable solely on the
//     dispatcher-solo path (scheduler `running == 0` + `run_system_once`), with a
//     `&mut self`-tied return lifetime that makes a second `&mut R` un-aliasable.
//     A DEBUG-ONLY thread tripwire (M2) — `NonSendResources::owning_thread`,
//     stamped at the first `insert_non_send_resource` and asserted by every
//     projection (`UnsafeEcsCell::nonsend_resources[_mut]` +
//     `DispatcherToken`) — catches a wrong-thread touch loud in debug. The
//     routing is still behaviorally (not compile-time) enforced; M2 is the
//     debug backstop. NSND-THREAD caller obligation: callers of
//     `insert_non_send_resource` and `Schedule::run` MUST keep the world on a
//     single owning (dispatcher) thread for the `!Send` slab's lifetime.
unsafe impl Send for EcsMaster {}
unsafe impl Sync for EcsMaster {}

/// Compile-time gate for the Phase 9 SEND1 contract. Forces a type-system
/// failure if either auto-trait is lost on `EcsMaster` (e.g. by a future
/// field addition that re-introduces a `!Send` / `!Sync` interior).
const _: fn() = || {
    fn assert_send_sync<T: Send + Sync>() {}
    // NOTE: this gate passes via the manual `unsafe impl Send/Sync for EcsMaster`
    // above; it does NOT independently prove field-level `Send + Sync` (a manual
    // `unsafe impl` satisfies the bound regardless of whether an interior field is
    // `!Send`). The per-field justification lives in SEND1 (this module) and SEND6
    // (`archetype_master.rs`, covering the Phase 14b `ObserverRegistry`).
    assert_send_sync::<EcsMaster>();
};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ecs::core::component::component_registry;
    // Imported explicitly (not via `super::*`): the topic-split moved the
    // `EcsMaster` methods that used these at module scope into sibling files, so
    // the module-level `use`s were trimmed as lib-unused. The tests below still
    // reference them, hence the direct imports here.
    use crate::ecs::core::system::into_system::IntoSystem;
    use crate::ecs::error::EcsError;
    use crate::ecs::identifiers::primitives::{ComponentId, EntityId};

    // Define test components with their IDs.
    //
    // Each test module owns its own ComponentId range to avoid inter-test
    // pollution through the global `OnceLock<ComponentLayout>` registry —
    // `OnceLock::set` fixes the first registration and silently ignores
    // subsequent ones, so two test modules registering different types under
    // the same ID end up with a layout mismatch (see audit C-003 — Phase 1b).
    //   ecs_master  : 100-109
    //   query       : 200-209
    //   archetype_master : 300-309
    //   archetype (unit) : 400-409
    const POSITION_ID: ComponentId = ComponentId(100);
    const VELOCITY_ID: ComponentId = ComponentId(101);
    const HEALTH_ID: ComponentId = ComponentId(102);

    #[repr(C)]
    struct Position { x: f32, y: f32, z: f32 }

    #[repr(C)]
    struct Velocity { x: f32, y: f32, z: f32 }

    #[repr(C)]
    struct Health { value: i32 }

    // Component impls mirror what `#[derive(Component)]` generates — needed
    // because Phase 2e `spawn_one` / `spawn_two` are bounded by `Component`,
    // and the test types must satisfy that bound to exercise the spawn path.
    use crate::ecs::core::component::component::Component;

    impl Component for Position {
        fn component_id() -> ComponentId { POSITION_ID }
    }
    impl Component for Velocity {
        fn component_id() -> ComponentId { VELOCITY_ID }
    }
    impl Component for Health {
        fn component_id() -> ComponentId { HEALTH_ID }
    }

    fn register_test_components() {
        // Register components in the global registry
        component_registry::register_layout::<Position>(POSITION_ID.0);
        component_registry::register_layout::<Velocity>(VELOCITY_ID.0);
        component_registry::register_layout::<Health>(HEALTH_ID.0);
    }

    #[test]
    fn test_ecs_master_creation() {
        register_test_components();
        
        let ecs = EcsMaster::new();
        assert_eq!(ecs.entity_count(), 0);
        assert_eq!(ecs.archetype_count(), 0);
    }

    #[test]
    fn test_entity_creation_and_deletion() {
        register_test_components();
        
        let mut ecs = EcsMaster::new();
        
        // Create an archetype
        let archetype_id = ecs.create_archetype(&[POSITION_ID, VELOCITY_ID]);
        
        // Create entities
        let pos = Position { x: 1.0, y: 2.0, z: 3.0 };
        let vel = Velocity { x: 4.0, y: 5.0, z: 6.0 };
        
        let pos_bytes = unsafe {
            std::slice::from_raw_parts(&pos as *const _ as *const u8, std::mem::size_of::<Position>())
        };
        let vel_bytes = unsafe {
            std::slice::from_raw_parts(&vel as *const _ as *const u8, std::mem::size_of::<Velocity>())
        };
        
        let entity1 = ecs.create_entity(archetype_id, &[
            (POSITION_ID, pos_bytes),
            (VELOCITY_ID, vel_bytes),
        ]).unwrap();
        
        assert_eq!(ecs.entity_count(), 1);
        assert!(ecs.has_entity(entity1));
        
        // Delete entity
        assert!(ecs.delete_entity(entity1));
        assert_eq!(ecs.entity_count(), 0);
        assert!(!ecs.has_entity(entity1));
    }

    #[test]
    fn test_query_entities() {
        register_test_components();

        let mut ecs = EcsMaster::new();

        // Create archetypes
        let arch1 = ecs.create_archetype(&[POSITION_ID, VELOCITY_ID]);
        let arch2 = ecs.create_archetype(&[POSITION_ID, HEALTH_ID]);

        // Create entities (simplified - using dummy data)
        let dummy_bytes = [0u8; 64];

        let _entity1 = ecs.create_entity(arch1, &[
            (POSITION_ID, &dummy_bytes[..12]),
            (VELOCITY_ID, &dummy_bytes[..12]),
        ]).unwrap();

        let _entity2 = ecs.create_entity(arch2, &[
            (POSITION_ID, &dummy_bytes[..12]),
            (HEALTH_ID, &dummy_bytes[..4]),
        ]).unwrap();

        // Query entities with Position
        let entities_with_position = ecs.query_entities(&[POSITION_ID]);
        assert_eq!(entities_with_position.len(), 2);

        // Query entities with Position and Velocity
        let entities_with_pos_vel = ecs.query_entities(&[POSITION_ID, VELOCITY_ID]);
        assert_eq!(entities_with_pos_vel.len(), 1);
    }

    // C-007 guard tests: validate that create_entity never leaks EntityIds.
    //
    // The guard sequence is:
    //   1. has_archetype() checked BEFORE allocate_entity()
    //   2. If archetype not found → bail! (no EntityId consumed)
    //   3. On post-allocation failure → rewind_allocate() undoes fresh-ID
    //      allocation, or deallocate_entity() recycles an existing one.

    /// Creating an entity in a non-existent archetype must fail and must not
    /// consume an EntityId from the allocator.
    #[test]
    fn test_create_entity_nonexistent_archetype_no_id_leak() {
        register_test_components();

        let mut ecs = EcsMaster::new();

        let dummy_bytes = [0u8; 12];

        // Attempt to create an entity in archetype 999 (never created).
        let result = ecs.create_entity(ArchetypeId(999), &[(POSITION_ID, &dummy_bytes)]);
        // C-019: caller can pattern-match on the concrete EcsError variant
        // (not just `is_err`) — the whole point of switching off `anyhow`.
        assert!(
            matches!(result, Err(EcsError::ArchetypeNotFound(ArchetypeId(999)))),
            "expected Err(ArchetypeNotFound(ArchetypeId(999))), got {:?}",
            result
        );

        // No EntityId must have been allocated: next fresh id stays at 0.
        assert_eq!(ecs.entity_master().next_entity_id(), EntityId(0),
            "EntityId must not be consumed when the guard fires");

        // No active entities and no recycled slots.
        assert_eq!(ecs.entity_count(), 0);
        assert_eq!(ecs.recycled_entity_count(), 0);
    }

    /// Consecutive failed guard calls must not accumulate leaked EntityIds.
    #[test]
    fn test_repeated_guard_failures_do_not_leak_ids() {
        register_test_components();

        let mut ecs = EcsMaster::new();
        let dummy_bytes = [0u8; 12];

        for _ in 0..5 {
            let _ = ecs.create_entity(ArchetypeId(42), &[(POSITION_ID, &dummy_bytes)]);
        }

        // After 5 failed guard calls the fresh-id counter must still be 0.
        assert_eq!(ecs.entity_master().next_entity_id(), EntityId(0));
        assert_eq!(ecs.entity_count(), 0);
        assert_eq!(ecs.recycled_entity_count(), 0);
    }

    /// A successful create_entity followed by a delete_entity returns the
    /// EntityId to the free list. A subsequent create_entity in a bad
    /// archetype must NOT consume that recycled slot.
    #[test]
    fn test_guard_does_not_consume_recycled_slot() {
        register_test_components();

        let mut ecs = EcsMaster::new();
        let arch = ecs.create_archetype(&[POSITION_ID, VELOCITY_ID]);

        let pos_bytes = [0u8; 12];
        let vel_bytes = [0u8; 12];

        // Create and immediately delete an entity.
        let entity = ecs.create_entity(arch, &[
            (POSITION_ID, &pos_bytes),
            (VELOCITY_ID, &vel_bytes),
        ]).unwrap();
        assert!(ecs.delete_entity(entity));
        assert_eq!(ecs.recycled_entity_count(), 1);

        // A guard-failing call must not touch the free list.
        let _ = ecs.create_entity(ArchetypeId(999), &[(POSITION_ID, &pos_bytes)]);
        assert_eq!(ecs.recycled_entity_count(), 1,
            "free list must not be consumed when guard fires before allocate_entity");
        assert_eq!(ecs.entity_count(), 0);
    }

    /// `rewind_allocate` is the internal mechanism backing the C-007 guard.
    /// Exercise it directly through entity_master() to verify the invariant:
    /// rewinding a fresh (non-registered) entity decrements next_entity_id.
    #[test]
    fn test_rewind_allocate_restores_fresh_id() {
        register_test_components();

        let mut ecs = EcsMaster::new();
        let entity_master = ecs.entity_master_mut();

        // Allocate a fresh entity without registering it.
        let entity = entity_master.allocate_entity();
        assert_eq!(entity.id(), EntityId(0));
        assert_eq!(entity_master.next_entity_id(), EntityId(1));

        // Rewind must succeed and restore next_entity_id to 0.
        let rewound = entity_master.rewind_allocate(entity);
        assert!(rewound, "fresh-ID rewind must succeed");
        assert_eq!(entity_master.next_entity_id(), EntityId(0),
            "next_entity_id must be restored after rewind");
        assert_eq!(entity_master.entity_count(), 0);
    }

    /// After a successful create_entity in a valid archetype the entity count
    /// must be 1 and the EntityId must be stable across the rewind path
    /// (i.e., the rewind path is never taken when creation succeeds).
    #[test]
    fn test_successful_create_entity_no_rewind() {
        register_test_components();

        let mut ecs = EcsMaster::new();
        let arch = ecs.create_archetype(&[POSITION_ID, VELOCITY_ID]);

        let pos = Position { x: 1.0, y: 0.0, z: 0.0 };
        let vel = Velocity { x: 0.0, y: 1.0, z: 0.0 };
        let pos_bytes = unsafe {
            std::slice::from_raw_parts(&pos as *const _ as *const u8, std::mem::size_of::<Position>())
        };
        let vel_bytes = unsafe {
            std::slice::from_raw_parts(&vel as *const _ as *const u8, std::mem::size_of::<Velocity>())
        };

        let entity = ecs.create_entity(arch, &[
            (POSITION_ID, pos_bytes),
            (VELOCITY_ID, vel_bytes),
        ]).unwrap();

        assert!(ecs.has_entity(entity));
        assert_eq!(ecs.entity_count(), 1);
        // next_entity_id was advanced to 1 and not rewound.
        assert_eq!(ecs.entity_master().next_entity_id(), EntityId(1));
        assert_eq!(ecs.recycled_entity_count(), 0);
    }

    // --- Phase 2e: spawn_one / spawn_two ergonomic wrappers ---

    /// `spawn_one` is equivalent to a 1-component `create_entity` call with
    /// auto-derived `ComponentId` and zero-alloc byte slicing.
    #[test]
    fn spawn_one_creates_entity_with_component() {
        register_test_components();

        let mut ecs = EcsMaster::new();
        let arch = ecs.create_archetype(&[POSITION_ID]);

        let entity = ecs.spawn_one(arch, Position { x: 1.5, y: 2.5, z: 3.5 })
            .expect("spawn_one in valid archetype must succeed");

        assert!(ecs.has_entity(entity), "spawned entity must be reachable");
        assert_eq!(ecs.entity_count(), 1);
    }

    /// `spawn_two` packs two components in archetype-defined order; result
    /// must be a fully-formed entity.
    #[test]
    fn spawn_two_creates_entity_with_both_components() {
        register_test_components();

        let mut ecs = EcsMaster::new();
        let arch = ecs.create_archetype(&[POSITION_ID, VELOCITY_ID]);

        let entity = ecs.spawn_two(
            arch,
            Position { x: 10.0, y: 20.0, z: 30.0 },
            Velocity { x: 1.0, y: 2.0, z: 3.0 },
        ).expect("spawn_two in valid archetype must succeed");

        assert!(ecs.has_entity(entity));
        assert_eq!(ecs.entity_count(), 1);
    }

    /// `spawn_one` must propagate `ArchetypeNotFound` for a bogus archetype id
    /// AND must not consume an EntityId from the allocator (C-007 guard
    /// behaviour carries through the wrapper).
    #[test]
    fn spawn_one_unknown_archetype_returns_err_no_leak() {
        register_test_components();
        let mut ecs = EcsMaster::new();
        let result = ecs.spawn_one(ArchetypeId(999), Position { x: 1.0, y: 2.0, z: 3.0 });
        assert!(
            matches!(result, Err(EcsError::ArchetypeNotFound(ArchetypeId(999)))),
            "spawn_one must propagate the typed error variant unchanged"
        );
        assert_eq!(ecs.entity_master().next_entity_id(), EntityId(0),
            "no EntityId must be consumed when the archetype guard fires");
        assert_eq!(ecs.entity_count(), 0);
    }

    // --- Phase 8a Step 8: `run_system_once` / `run_closure_once` smoke tests ---

    /// Test resource used by the `run_closure_once` smoke tests. Lives inside
    /// the `tests` module so its `ResourceId` is reserved on first use without
    /// colliding with other test modules.
    struct SystemTestRes(u32);

    impl crate::ecs::core::resources::resource::Resource for SystemTestRes {
        fn resource_id() -> crate::ecs::identifiers::primitives::ResourceId {
            use crate::ecs::core::resources::resource_registry::register_new;
            use crate::ecs::identifiers::primitives::ResourceId;
            use std::sync::OnceLock;
            static ID: OnceLock<ResourceId> = OnceLock::new();
            *ID.get_or_init(|| ResourceId(register_new::<Self>()))
        }
    }

    /// `run_closure_once(|| ...)` runs an empty closure and propagates
    /// its return value. Phase 8c Step 5: turbofish dropped — the closure's
    /// (zero-)param surface infers from its signature.
    #[test]
    fn run_system_once_with_empty_closure_runs_once() {
        let mut ecs = EcsMaster::new();
        let out: u32 = ecs.run_closure_once(|| 42);
        assert_eq!(out, 42, "run_closure_once must propagate the closure's output");
    }

    /// `run_closure_once(|r: Res<TestRes>| ...)` reads back a resource that
    /// was inserted via the `pub(crate)` `resources` field (the public
    /// `insert_resource` facade lands in Step 9).
    #[test]
    fn run_closure_once_with_res_reads_value() {
        use crate::ecs::core::system::Res;
        use std::sync::Arc;
        use std::sync::atomic::{AtomicU32, Ordering};

        let mut ecs = EcsMaster::new();
        ecs.resources.insert(SystemTestRes(123));

        // The closure capture must be `Send + Sync` because the `System`
        // trait bound transitively requires it on the closure. `AtomicU32`
        // behind `Arc` satisfies the bound and serves as a probe channel
        // from inside the closure back to the outer test.
        let observed = Arc::new(AtomicU32::new(0));
        let probe = Arc::clone(&observed);
        // Phase 8c Step 5: turbofish replaced by closure-arg annotation.
        // r: Res<SystemTestRes>; r.0: &SystemTestRes; r.0.0: u32 (the inner
        // newtype field, accessed via auto-deref through the shared borrow).
        ecs.run_closure_once(move |r: Res<SystemTestRes>| {
            probe.store(r.0.0, Ordering::Relaxed);
        });
        assert_eq!(
            observed.load(Ordering::Relaxed),
            123,
            "Res<R> must round-trip the inserted value"
        );
    }

    // --- Phase 8c Step 4: `run_system` / `run_cached_system` smoke tests ---

    /// Phase 8c Step 4 test resource. A fresh `TypeId` -> fresh dynamic
    /// `ResourceId` via `register_new::<Self>()`; no collision with the
    /// `SystemTestRes` slot used by the Phase 8a smoke tests above.
    struct Step4Res(u32);

    impl crate::ecs::core::resources::resource::Resource for Step4Res {
        fn resource_id() -> crate::ecs::identifiers::primitives::ResourceId {
            use crate::ecs::core::resources::resource_registry::register_new;
            use crate::ecs::identifiers::primitives::ResourceId;
            use std::sync::OnceLock;
            static ID: OnceLock<ResourceId> = OnceLock::new();
            *ID.get_or_init(|| ResourceId(register_new::<Self>()))
        }
    }

    /// `run_system(|| { ... })` — arity-0 closure, no params, no return.
    /// The headline ergonomic claim: no turbofish, no `<P>` to spell out.
    #[test]
    fn run_system_arity_0_no_param_runs_closure() {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicU32, Ordering};

        let mut ecs = EcsMaster::new();

        // Probe channel — `AtomicU32` behind `Arc` satisfies the
        // `Send + Sync + 'static` bound transitively required by the
        // closure captures (System: Send + Sync + 'static).
        let observed = Arc::new(AtomicU32::new(0));
        let probe = Arc::clone(&observed);
        ecs.run_system(move || {
            probe.store(7, Ordering::Relaxed);
        });
        assert_eq!(
            observed.load(Ordering::Relaxed),
            7,
            "run_system must execute the arity-0 closure body"
        );
    }

    /// `run_system(|res: Res<R>| -> u32 { ... })` — arity-1 with `Res<R>`
    /// and a non-unit return. Verifies that the `IntoSystem` dispatch
    /// propagates the body's output back through `FunctionSystem`.
    #[test]
    fn run_system_arity_1_res_returns_value() {
        use crate::ecs::core::system::Res;

        let mut ecs = EcsMaster::new();
        ecs.resources.insert(Step4Res(531));

        // `res.0` is the `&Step4Res` (pub(crate) field on `Res<'w, R>`);
        // `.0` on the newtype yields the inner `u32`. Mirrors the
        // `r.0.0` pattern used by the Phase 8a smoke tests above.
        let out: u32 = ecs.run_system(|res: Res<Step4Res>| -> u32 { res.0.0 });
        assert_eq!(
            out, 531,
            "run_system must round-trip a Res<R> read into the return value"
        );
    }

    /// Phase 9 SEND1 — compile-time gate that `EcsMaster` and
    /// `UnsafeEcsCell<'_>` are both `Send + Sync`. The const assertion at
    /// module scope provides a sharper error site if the contract is ever
    /// broken by a field-level change; this test gives `cargo test` a
    /// human-visible green tick for the same condition.
    #[test]
    fn ecs_master_and_cell_are_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<EcsMaster>();
        assert_send_sync::<crate::ecs::core::system::unsafe_ecs_cell::UnsafeEcsCell<'static>>();
    }

    /// `run_cached_system(&mut sys)` invoked twice on the same
    /// `FunctionSystem` reuses the cached state (FS1 idempotent
    /// `initialize`); the second call must observe the post-first-call
    /// state of the world without rebuilding the system.
    #[test]
    fn run_cached_system_reused_twice_reads_updated_resource() {
        use crate::ecs::core::system::Res;
        use crate::ecs::core::system::function_system::FunctionSystem;
        use std::sync::Arc;
        use std::sync::atomic::{AtomicU32, Ordering};

        let mut ecs = EcsMaster::new();
        ecs.resources.insert(Step4Res(100));

        // Build the FunctionSystem once via `IntoSystem::into_system` —
        // exactly the same construction `run_system` performs internally,
        // hoisted so the state survives across two `run_cached_system`
        // calls.
        let observed = Arc::new(AtomicU32::new(0));
        let probe = Arc::clone(&observed);
        let body = move |res: Res<Step4Res>| {
            probe.store(res.0.0, Ordering::Relaxed);
        };
        // `into_system` produces a `FunctionSystem<F, Marker>` (turbofish
        // here is on the IntoSystem trait, not the closure body).
        let mut sys: FunctionSystem<_, _> = IntoSystem::into_system(body);

        // First call: cold init + run + apply. State transitions from
        // None -> Some(_).
        ecs.run_cached_system(&mut sys);
        assert_eq!(observed.load(Ordering::Relaxed), 100);

        // Mutate the resource in between via the public facade. This is
        // safe because `run_cached_system` returned `&mut ecs` back to us.
        ecs.resource_mut::<Step4Res>().0 = 200;

        // Second call: re-init is a no-op (FS1). The cached state and
        // SystemMeta are reused; the body observes the updated value.
        ecs.run_cached_system(&mut sys);
        assert_eq!(
            observed.load(Ordering::Relaxed),
            200,
            "run_cached_system must reuse cached state and read fresh world data"
        );
    }

    /// Phase 16 — `run_condition(&mut dyn System<Out=bool>)` returns the
    /// condition's bool verdict. A `|| true` condition returns `true`, a
    /// `|| false` returns `false`. (Plan §10 `run_condition_returns_bool`.)
    #[test]
    fn run_condition_returns_constant_verdict() {
        use crate::ecs::core::system::System;
        use crate::ecs::core::system::function_system::FunctionSystem;

        let mut ecs = EcsMaster::new();

        let mut yes: FunctionSystem<_, _> = IntoSystem::into_system(|| true);
        yes.initialize(&mut ecs);
        let this_run = ecs.current_tick();
        assert!(
            ecs.run_condition(&mut yes, this_run),
            "`|| true` condition returns true"
        );

        let mut no: FunctionSystem<_, _> = IntoSystem::into_system(|| false);
        no.initialize(&mut ecs);
        let this_run = ecs.current_tick();
        assert!(
            !ecs.run_condition(&mut no, this_run),
            "`|| false` condition returns false"
        );
    }

    /// Phase 16 — a `fn(Res<R>) -> bool` condition run via `run_condition`
    /// reads the resource and returns the value-derived verdict. Mutating the
    /// resource between calls flips the verdict (cached-system state reuse).
    #[test]
    fn run_condition_reads_resource_value() {
        use crate::ecs::core::system::Res;
        use crate::ecs::core::system::System;
        use crate::ecs::core::system::function_system::FunctionSystem;

        let mut ecs = EcsMaster::new();
        ecs.resources.insert(Step4Res(5));

        let mut cond: FunctionSystem<_, _> =
            IntoSystem::into_system(|r: Res<Step4Res>| r.0.0 == 5);
        cond.initialize(&mut ecs);
        let this_run = ecs.current_tick();
        assert!(
            ecs.run_condition(&mut cond, this_run),
            "resource == 5 ⇒ true"
        );

        ecs.resource_mut::<Step4Res>().0 = 7;
        let this_run = ecs.current_tick();
        assert!(
            !ecs.run_condition(&mut cond, this_run),
            "after mutation resource != 5 ⇒ false (cached-system state reused)"
        );
    }

    /// Phase 12.5 Track B C3 / C5 — smoke test for the
    /// `query_state_cache` drop ordering invariant.
    ///
    /// Rust drops struct fields in declaration order — `archetype_master`
    /// (whose pools own the component reservations since Phase X.I; the
    /// shared arena was retired in Phase X.J) is declared BEFORE
    /// `query_state_cache`, so the storage drops FIRST and the cache drops
    /// AFTER. This pin exercises the basic `EcsMaster::drop()` path after a
    /// `query` cold-init has populated the cache slot; a future regression
    /// that reordered the fields and dropped the cache before the storage
    /// would surface here (additional Miri coverage lives in
    /// `tests/miri_phase12_5_track_b.rs::miri_query_cache_drops_after_arena_with_arena_derived_d_state`).
    ///
    /// The full synthetic-`D::State` drop-order recorder described in the
    /// plan §11.4 outline is deferred to Phase 13 — implementing a
    /// `QueryData` trait fixture that records drop order in a `thread_local!`
    /// requires routing through the entire trait surface for a fixture
    /// that today exercises a code path the existing Miri test already
    /// guards. The current pin documents the invariant; Phase 13 can lift
    /// it to a full ordering recorder if production code adds an
    /// arena-derived `D::State`.
    #[test]
    fn query_state_cache_drops_after_arena() {
        register_test_components();
        let mut ecs = EcsMaster::new();
        let arch = ecs.create_archetype(&[POSITION_ID]);
        let pos = Position { x: 1.0, y: 2.0, z: 3.0 };
        // SAFETY: `Position` is `#[repr(C)]` POD; bytes are valid for the
        //   duration of this call.
        let bytes = unsafe {
            std::slice::from_raw_parts(
                &pos as *const _ as *const u8,
                std::mem::size_of::<Position>(),
            )
        };
        ecs.create_entity(arch, &[(POSITION_ID, bytes)])
            .expect("create_entity must succeed in test fixture");

        // Populate the cache slot for `<&Position, ()>` via the direct
        // API. The cache will hold a `Box<UnsafeCell<QueryDataState<...>>>`
        // for the duration of `ecs`.
        {
            let view = ecs.query::<&Position, ()>();
            let _ = view.iter().count();
        }

        // Drop the world. Field-order on EcsMaster places
        // `query_state_cache` AFTER `archetype_master` — Rust's
        // declaration-order drop semantics therefore reclaim the cache slot
        // AFTER the component storage has dropped. A regression that
        // reordered the fields would either (a) trigger Miri inside the
        // existing
        // `miri_query_cache_drops_after_arena_with_arena_derived_d_state`
        // test, or (b) surface as a use-after-free in any future
        // storage-derived `D::State` impl.
        drop(ecs);
    }
}