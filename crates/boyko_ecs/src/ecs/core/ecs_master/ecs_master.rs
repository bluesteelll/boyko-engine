use std::cell::UnsafeCell;
use std::ptr::NonNull;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU32, Ordering};

use crate::ecs::core::archetype::archetype::{Archetype, RemoveOutcome};
use crate::ecs::core::archetype::archetype_master::ArchetypeMaster;
use crate::ecs::core::bundle::bundle::Bundle;
use crate::ecs::core::bundle::bundle_column_cache::BundleColumnCache;
use crate::ecs::core::bundle::bundle_type_registry::{BundleTypeId, MAX_BUNDLE_TYPES};
use crate::ecs::core::commands::command::Command;
use crate::ecs::core::commands::command_queue::CommandQueue;
use crate::ecs::core::change_detection::{CHECK_TICK_THRESHOLD, Tick};
use crate::ecs::core::component::component::Component;
use crate::ecs::core::component::component_registry::{self, MAX_COMPONENTS};
use crate::ecs::core::component::hooks::archetype_flags::ArchetypeFlags;
use crate::ecs::core::component::hooks::builder::ComponentHooksBuilder;
use crate::ecs::core::component::hooks::dispatch::{
    trigger_on_add, trigger_on_insert, trigger_on_remove, trigger_on_replace,
};
use crate::ecs::core::component::hooks::scope::{DeferredScopeGuard, hook_drain_depth};
use crate::ecs::core::component::observers::dispatch::{
    fire_on_add_observers, fire_on_insert_observers, fire_on_remove_observers,
    fire_on_replace_observers,
};
use crate::ecs::core::component::observers::{ObserverFn, ObserverId, ObserverKind};
use crate::ecs::core::entity::entity::Entity;
use crate::ecs::core::entity::entity_inland::EntityInland;
use crate::ecs::core::entity::entity_master::EntityMaster;
use crate::ecs::core::events::event::Event;
use crate::ecs::core::events::event_config::EventConfig;
use crate::ecs::core::events::event_dispatcher::EventDispatcher;
use crate::ecs::core::iters::query::data::{Mut, QueryData};
use crate::ecs::core::iters::query::filter::QueryFilter;
use crate::ecs::core::iters::query::query_type_registry::{
    MAX_QUERY_TYPES, QueryTypeId, QueryTypeKey,
};
use crate::ecs::core::iters::query::query_view::QueryView;
use crate::ecs::core::iters::query::state::QueryDataState;
use crate::ecs::core::resources::resource::Resource;
use crate::ecs::core::resources::resources::Resources;
use crate::ecs::core::state::states::States;
use crate::ecs::core::state::transition_record::StateTransitionRecord;
use crate::ecs::core::state::{NextState, State};
use crate::ecs::core::system::{
    into_system::IntoSystem, system::System, unsafe_ecs_cell::UnsafeEcsCell,
};
use crate::ecs::identifiers::primitives::{
    ArchetypeId, ComponentId, EntityId, InlandPoolId, WorldId,
};
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

/// Phase 12.5 Opt-A2 (SBO16 / SBO17 / §1.5): re-export of the per-call
/// `spawn_batch` cap.
///
/// Phase 12.6 — the entity fast-store is no longer pre-extended at
/// world construction. The dispatcher path
/// (`SpawnBatchCommand::apply` and `EcsMaster::spawn_batch`) grows the
/// fast-store on demand via
/// [`EntityMaster::ensure_capacity`](crate::ecs::core::entity::entity_master::EntityMaster::ensure_capacity)
/// before writing rows. The hard SBO17b panic on aggregate-worker
/// overshoot is replaced by lazy growth (the apply path holds
/// `&mut EcsMaster`, so worker reads cannot race a Vec reallocation —
/// SEND5 preserved).
pub(crate) use crate::ecs::core::system::params::entity_counter::MAX_BATCH_HINT;

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

    /// Event dispatcher — dropped after `resources` and before the entity /
    /// archetype subsystems. Event buffers live in their own heap allocations
    /// independent of component storage.
    events: EventDispatcher,

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
    archetype_master: ArchetypeMaster,

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
            events,
            entity_master: EntityMaster::new(),
            archetype_master,
            bundle_archetype_cache: OnceLock::new(),
            bundle_column_cache: OnceLock::new(),
            change_tick: AtomicU32::new(0),
            last_check_tick: Tick::ZERO,
            // Phase 14a: lazy — alloc-free until the first deferred hook push.
            // The reentrancy depth counter is a thread-local (see hooks::scope),
            // so it is not a field here (fixes F2's Tree Borrows UB).
            deferred_hook_queue: CommandQueue::new(),
            query_state_cache: OnceLock::new(),
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
            events,
            entity_master: EntityMaster::with_capacity(entity_capacity),
            archetype_master,
            bundle_archetype_cache: OnceLock::new(),
            bundle_column_cache: OnceLock::new(),
            change_tick: AtomicU32::new(0),
            last_check_tick: Tick::ZERO,
            // Phase 14a: lazy — alloc-free until the first deferred hook push.
            // The reentrancy depth counter is a thread-local (see hooks::scope),
            // so it is not a field here (fixes F2's Tree Borrows UB).
            deferred_hook_queue: CommandQueue::new(),
            query_state_cache: OnceLock::new(),
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

    /// Creates a new archetype with the specified component IDs
    /// Returns the ID of the created archetype
    #[inline]
    pub fn create_archetype(&mut self, component_ids: &[ComponentId]) -> ArchetypeId {
        self.archetype_master.create_archetype(component_ids)
    }

    /// Gets or creates an archetype with the specified component IDs
    /// Returns the ID of the archetype
    #[inline]
    pub fn get_or_create_archetype(&mut self, component_ids: &[ComponentId]) -> ArchetypeId {
        self.archetype_master.get_or_create_archetype(component_ids)
    }

    /// Creates a new entity with components in the specified archetype.
    ///
    /// Takes a borrowed slice of `(ComponentId, &[u8])` pairs for component data —
    /// zero allocation per call on the components argument. Returns the created
    /// entity if successful.
    ///
    /// # Guard pattern (C-007)
    ///
    /// Preconditions are validated **before** `allocate_entity` so that
    /// no `EntityId` is leaked if the archetype lookup fails. Specifically:
    /// 1. `has_archetype(archetype_id)` is checked first.
    /// 2. Only then is `allocate_entity` called.
    /// 3. If `create_entity` fails, `rewind_allocate` undoes the allocation
    ///    (fresh-ID path) so the ID is not silently wasted.
    ///
    /// # W7 choreography (Phase 7)
    ///
    /// 1. Mint write-capable `*mut Archetype` via `archetype_ptr_for` under
    ///    `&mut self`. The raw pointer does not participate in borrow
    ///    checking, so it lives across the subsequent `&mut entity_master`
    ///    call without conflict.
    /// 2. Allocate the entity id.
    /// 3. Reborrow the raw pointer as `&mut Archetype` (scoped tightly to
    ///    the `create_entity` call) to push the new row.
    /// 4. On rejection, rewind the entity-id allocation (C-007 rewind path).
    /// 5. On success, register the entity in the Phase 7 fast store
    ///    (read path of `get_component_raw`, `has_entity`, and friends).
    ///
    /// Audit: C-010 — switched from Vec to &[...].
    pub fn create_entity(
        &mut self,
        archetype_id: ArchetypeId,
        components: &[(ComponentId, &[u8])],
    ) -> EcsResult<Entity> {
        // Phase 14a §3.2 / §8 P1: RAII depth bracket. `Drop` decrements the
        // depth on EVERY exit (Ok / Err / panic), so the early `return Err`
        // paths below (which all PRECEDE the hook-fire point) strand nothing.
        let scope = DeferredScopeGuard::enter();

        // Guard: validate archetype exists BEFORE allocating an EntityId.
        // Previously, allocate_entity() was called first, and if the archetype
        // lookup subsequently failed the ID was permanently leaked (C-007).
        if !self.archetype_master.has_archetype(archetype_id) {
            return Err(EcsError::ArchetypeNotFound(archetype_id));
        }

        // Step 1 of W7: mint write-capable *mut Archetype. The raw pointer is
        // not subject to borrow checking, so it can outlive the &mut borrow
        // on archetype_master that produced it — see U14. F4: this is a
        // FRESH same-frame local (no sibling structural write intervenes before
        // its reborrows below), so it was already legal pre-fix; it is now also
        // interior-mutable (`SharedReadWrite`, F4-rooted) like every slab ptr.
        let archetype_ptr = self.archetype_master
            .archetype_ptr_for(archetype_id)
            .expect("invariant: archetype existed at guard check; single-threaded");

        // Phase 10 INIT3 / Round 2 W4: the world owns the change-detection
        // tick. Read it once here and thread it into `Archetype::create_entity`
        // so the per-row `added`/`changed` ticks land at the correct value.
        // No caller of `EcsMaster::create_entity` needs to know the tick
        // (single source of truth).
        let current_tick = self.current_tick();

        // Step 2 of W7: allocate the entity id (fresh or recycled).
        let entity = self.entity_master.allocate_entity();

        // Step 3 of W7: reborrow archetype_ptr as &mut Archetype inside a
        // tight scope so the &mut reference is dropped before any further
        // entity_master mutation.
        let mut new_unit_index: u32 = 0;
        let pushed = {
            // SAFETY (U14, U1, U2):
            //   - U14: archetype_ptr was just minted via archetype_ptr_for
            //     under &mut self, so the provenance is write-capable; the
            //     bundle slab address is stable; no other live borrow into
            //     this slot exists (single-threaded EcsMaster).
            //   - U1/U2: slab address stable, slab slot lifetime ⊇
            //     EcsMaster lifetime (bundle invariants).
            //   - The reborrow is scoped to this block; once create_entity
            //     returns, the &mut Archetype is dropped before any further
            //     self.entity_master calls.
            let archetype: &mut Archetype = unsafe { &mut *archetype_ptr };
            archetype.create_entity(entity.id(), &mut new_unit_index, components, current_tick)
        };

        if !pushed {
            // Step 4 of W7: archetype rejected the push — signature mismatch,
            // or the pool reserve ceiling (rows). Phase X.I: committed
            // capacity below the ceiling grows on demand inside the pools,
            // so a capacity rejection here means the archetype outgrew a
            // pool's reserve_rows. Undo the allocation so the EntityId is
            // not leaked.
            let rewound = self.entity_master.rewind_allocate(entity);
            if !rewound {
                // rewind_allocate returns false for recycled IDs; fall back
                // to the full deallocate path so the ID returns to the free
                // list.
                self.entity_master.deallocate_entity(entity);
            }
            return Err(EcsError::ArchetypeRejectedEntity { archetype_id });
        }

        // Step 5 of W7: register the entity in the Phase 7 fast store. This
        // is the read path consumed by get_component_raw, has_entity,
        // set_component_raw, and the typed get_component<T> /
        // get_component_mut<T> wrappers.
        self.entity_master.register_entity_with_ptr(entity, archetype_ptr, new_unit_index);

        // Step 6 (Phase 14a §3.2): fire on_add / on_insert hooks. The Step-3
        // `&mut Archetype` was block-scoped (`let pushed = { ... }`) and is
        // dead; only `archetype_ptr` (*mut, Copy) survives — no `world`-derived
        // `&mut` is live, so minting `world_ptr` aliases no reborrow (SAFETY-1).
        //
        // P1 invariant: there is NO fallible step after this fire point — every
        // `return Err` above precedes it, so no deferred command is ever
        // enqueued on an `Err` path (nothing to strand).
        debug_assert!(
            self.archetype_master.has_archetype(archetype_id),
            "P1: no fallible step may follow the hook-fire point in a bracketed body"
        );
        // SAFETY: `archetype_ptr` is write-capable + stable slab provenance;
        //   reading `flags` is one `u16` load (no `&mut` taken).
        let flags = unsafe { (*archetype_ptr).flags };
        if !flags.is_empty() {
            let world_ptr = NonNull::from(&mut *self);
            // Phase 14b: inner gates widen HOOK -> ANY (hook OR observer). Hooks
            // fire first, then observers (per-kind block shape, §5). The two
            // nested `contains` are sub-tests of the already-loaded `flags` u16
            // (no extra load); the `ids` slice is read once per kind.
            if flags.contains(ArchetypeFlags::ON_ADD_ANY) {
                // SAFETY: `archetype_ptr` is a valid `*const Archetype`; the
                //   shared slice is transient and not aliased by a live `&mut`.
                let ids = unsafe { (*archetype_ptr).component_ids.as_slice() };
                if flags.contains(ArchetypeFlags::ON_ADD_HOOK) {
                    for &cid in ids {
                        trigger_on_add(world_ptr, cid, entity);
                    }
                }
                if flags.contains(ArchetypeFlags::ON_ADD_OBSERVER) {
                    for &cid in ids {
                        fire_on_add_observers(world_ptr, cid, entity);
                    }
                }
            }
            if flags.contains(ArchetypeFlags::ON_INSERT_ANY) {
                // SAFETY: same as the on_add slice read above.
                let ids = unsafe { (*archetype_ptr).component_ids.as_slice() };
                if flags.contains(ArchetypeFlags::ON_INSERT_HOOK) {
                    for &cid in ids {
                        trigger_on_insert(world_ptr, cid, entity);
                    }
                }
                if flags.contains(ArchetypeFlags::ON_INSERT_OBSERVER) {
                    for &cid in ids {
                        fire_on_insert_observers(world_ptr, cid, entity);
                    }
                }
            }
        }

        // Direct API: drop the bracket (depth back to 0) then drain on the
        // success path (Q-A1 / §8 P1). On a panic above, `scope`'s `Drop`
        // restores the depth and we do NOT drain (running deferred user code
        // mid-unwind is wrong).
        drop(scope);
        self.drain_deferred_hook_queue();

        Ok(entity)
    }

    /// Phase 11 (plan §6.2): pushes an entity row into the specified
    /// archetype, registering an **already-reserved** `Entity` handle in
    /// the Phase 7 fast store.
    ///
    /// Used by [`SpawnAtCommand::apply`](crate::ecs::core::commands::spawn_at_command::SpawnAtCommand)
    /// after the deferred-spawn path has minted an `Entity` via
    /// [`EntityCounter::reserve_entity`](crate::ecs::core::system::params::entity_counter::EntityCounter::reserve_entity).
    /// Unlike [`create_entity`](Self::create_entity), this function does
    /// NOT mint a fresh `Entity` — it expects the caller to pass the
    /// pre-allocated handle.
    ///
    /// # Pre-conditions (debug-asserted)
    ///
    /// * `archetype_id` is registered.
    /// * `entity.id().0`'s slot in `entities_inland` is currently NULL
    ///   (never registered, never spawned-at). The atomic counter ensures
    ///   uniqueness; double-apply on the same handle is a bug at the
    ///   `SpawnAtCommand` enqueue layer.
    ///
    /// # Behaviour
    ///
    /// 1. Resolves the archetype's write-capable raw pointer.
    /// 2. Resizes `entities_inland` if `entity.id().0` is past the current
    ///    length. Phase 12.6 — single-row growth via `Vec::resize` is the
    ///    canonical lazy path; the dispatcher's `&mut self` borrow
    ///    guarantees workers are not in flight.
    /// 3. Pushes the row into the archetype with the world's current tick
    ///    (same INIT3 contract as `create_entity`).
    /// 4. Registers `(entity, archetype_ptr, unit_index)` in the Phase 7
    ///    fast store via `register_entity_with_ptr`.
    pub fn create_entity_at(
        &mut self,
        entity: Entity,
        archetype_id: ArchetypeId,
        components: &[(ComponentId, &[u8])],
    ) -> EcsResult<Entity> {
        // Phase 14a §3.2 / §8 P1: RAII depth bracket (every `return Err` below
        // precedes the hook-fire point, so they strand nothing).
        let scope = DeferredScopeGuard::enter();

        // Guard: archetype existence is checked BEFORE any state mutation.
        if !self.archetype_master.has_archetype(archetype_id) {
            return Err(EcsError::ArchetypeNotFound(archetype_id));
        }

        // EC7 (debug): slot must be NULL (never registered, never
        // spawned-at) at this point.
        debug_assert!(
            self.entity_master
                .entities_inland
                .get(entity.id().0)
                .is_none_or(|i| i.is_null()),
            "create_entity_at: entity {:?} is already registered (double-apply?)",
            entity
        );

        let archetype_ptr = self
            .archetype_master
            .archetype_ptr_for(archetype_id)
            .expect("invariant: archetype existed at guard check; single-threaded");

        let current_tick = self.current_tick();

        // Phase 12.6 — lazy growth path; Phase X.G — `InlandStore::ensure`
        // extends it on demand under `&mut self` (no worker race per
        // SEND5/SBO16) with zero copies and zero fills.
        let id_raw = entity.id().0;
        self.entity_master.entities_inland.ensure(id_raw + 1);

        let mut new_unit_index: u32 = 0;
        let pushed = {
            // SAFETY (U14, U1, U2, mirrors `create_entity`):
            //   * `archetype_ptr` was just minted via `archetype_ptr_for`
            //     under `&mut self`; provenance is write-capable.
            //   * Bundle slab address is stable; no other live borrow.
            //   * The reborrow is scoped to this block.
            let archetype: &mut Archetype = unsafe { &mut *archetype_ptr };
            archetype.create_entity(entity.id(), &mut new_unit_index, components, current_tick)
        };

        if !pushed {
            return Err(EcsError::ArchetypeRejectedEntity { archetype_id });
        }

        // Register in the Phase 7 fast store. The entity carries its own
        // generation (typically `0` for fresh reserves); we propagate it
        // verbatim through `register_entity_with_ptr`.
        self.entity_master
            .register_entity_with_ptr(entity, archetype_ptr, new_unit_index);

        // Phase 14a §3.2: fire on_add / on_insert hooks (mirrors `create_entity`).
        // The Step-3 `&mut Archetype` was block-scoped and is dead; only
        // `archetype_ptr` survives at the mint (SAFETY-1). P1: no fallible step
        // follows.
        debug_assert!(
            self.archetype_master.has_archetype(archetype_id),
            "P1: no fallible step may follow the hook-fire point in a bracketed body"
        );
        // SAFETY: `archetype_ptr` is write-capable + stable slab provenance.
        let flags = unsafe { (*archetype_ptr).flags };
        if !flags.is_empty() {
            let world_ptr = NonNull::from(&mut *self);
            // Phase 14b: inner gates widen HOOK -> ANY; hooks first, then
            // observers (mirrors `create_entity`, §5).
            if flags.contains(ArchetypeFlags::ON_ADD_ANY) {
                // SAFETY: transient shared slice, not aliased by a live `&mut`.
                let ids = unsafe { (*archetype_ptr).component_ids.as_slice() };
                if flags.contains(ArchetypeFlags::ON_ADD_HOOK) {
                    for &cid in ids {
                        trigger_on_add(world_ptr, cid, entity);
                    }
                }
                if flags.contains(ArchetypeFlags::ON_ADD_OBSERVER) {
                    for &cid in ids {
                        fire_on_add_observers(world_ptr, cid, entity);
                    }
                }
            }
            if flags.contains(ArchetypeFlags::ON_INSERT_ANY) {
                // SAFETY: same as the on_add slice read above.
                let ids = unsafe { (*archetype_ptr).component_ids.as_slice() };
                if flags.contains(ArchetypeFlags::ON_INSERT_HOOK) {
                    for &cid in ids {
                        trigger_on_insert(world_ptr, cid, entity);
                    }
                }
                if flags.contains(ArchetypeFlags::ON_INSERT_OBSERVER) {
                    for &cid in ids {
                        fire_on_insert_observers(world_ptr, cid, entity);
                    }
                }
            }
        }

        drop(scope);
        self.drain_deferred_hook_queue();

        Ok(entity)
    }

    /// Phase 12.5 Opt-A3 (§6.4): `create_entity_at` variant that consumes
    /// pre-resolved `pool_ids` from the per-world
    /// [`BundleColumnCache`](crate::ecs::core::bundle::BundleColumnCache),
    /// bypassing the 4× SparseMap lookup of the legacy path.
    ///
    /// `components` MUST be canonical-sorted by `ComponentId.0` (B1/B2);
    /// `pool_ids[i]` corresponds to `components[i].0`. Caller is
    /// `SpawnAtCommand::apply` post-Opt-A3 wiring.
    ///
    /// # Phase 12.6 — legacy bridge
    ///
    /// `SpawnAtCommand::apply` no longer routes through this method; it
    /// inlines the equivalent write loop to avoid the per-spawn slot-array
    /// rebuild + cross-call hop. Retained as the `EcsMaster`-side primitive
    /// reachable by external benchmarks that model the pre-Phase-12.6
    /// dispatch shape (see
    /// `crates/bench_bevy_vs_boyko/benches/profile_spawn_*.rs`).
    #[allow(dead_code)]
    pub(crate) fn create_entity_at_with_pool_ids(
        &mut self,
        entity: Entity,
        archetype_id: ArchetypeId,
        components: &[(ComponentId, &[u8])],
        pool_ids: &[InlandPoolId],
    ) -> EcsResult<Entity> {
        if !self.archetype_master.has_archetype(archetype_id) {
            return Err(EcsError::ArchetypeNotFound(archetype_id));
        }
        debug_assert!(
            self.entity_master
                .entities_inland
                .get(entity.id().0)
                .is_none_or(|i| i.is_null()),
            "create_entity_at_with_pool_ids: entity {:?} is already registered",
            entity
        );

        let archetype_ptr = self
            .archetype_master
            .archetype_ptr_for(archetype_id)
            .expect("invariant: archetype existed at guard check; single-threaded");
        let current_tick = self.current_tick();

        let id_raw = entity.id().0;
        self.entity_master.entities_inland.ensure(id_raw + 1);

        let mut new_unit_index: u32 = 0;
        let pushed = {
            // SAFETY (U14, U1, U2, mirrors `create_entity_at`):
            //   write-capable provenance under `&mut self`; reborrow
            //   scoped to this block.
            let archetype: &mut Archetype = unsafe { &mut *archetype_ptr };
            archetype.create_entity_with_pool_ids(
                entity.id(),
                &mut new_unit_index,
                components,
                pool_ids,
                current_tick,
            )
        };
        if !pushed {
            return Err(EcsError::ArchetypeRejectedEntity { archetype_id });
        }
        self.entity_master
            .register_entity_with_ptr(entity, archetype_ptr, new_unit_index);
        Ok(entity)
    }

    /// Type-safe wrapper around [`create_entity`] for a single component (Phase 2e — Q-024 follow-up).
    ///
    /// The caller supplies the value by move; this function reads its bytes via
    /// `std::slice::from_raw_parts` and forwards to `create_entity`. No heap
    /// allocation, no `Vec` materialisation, no manual `ComponentId` lookup.
    ///
    /// ```ignore
    /// // Before:
    /// ecs.create_entity(arch_id, &[(Position::component_id(), &pos_bytes)])
    /// // After:
    /// ecs.spawn_one(arch_id, Position { x: 1.0, y: 2.0, z: 3.0 })
    /// ```
    ///
    /// # Drop discipline
    ///
    /// On success, `a` is byte-copied into the pool by `ComponentPool::add`
    /// (`ptr::copy_nonoverlapping`) and the pool's registered `drop_fn` (set up
    /// by `register_layout::<A>`, M-001) becomes the new drop owner. The local
    /// `a` value must NOT run its destructor — `std::mem::forget(a)` suppresses
    /// the local drop only on the Ok path.
    ///
    /// On failure, NO bytes were copied into the pool (the failure modes are
    /// either an early `ArchetypeNotFound` guard or a pool rejection that
    /// rewinds without writing). `a` retains its full identity and runs its
    /// destructor at function-exit scope as usual — no leak, no double-free.
    ///
    /// Mirrors `LegacyQuery::iter_one` (Phase 2d) on the spawn side: bounded 1-arity
    /// API today, generic tuple version is Phase 2e-extension.
    pub fn spawn_one<A: crate::ecs::core::component::component::Component>(
        &mut self,
        archetype_id: ArchetypeId,
        a: A,
    ) -> EcsResult<Entity> {
        // SAFETY: `a` is a valid, fully-initialised `A` living on the caller's
        // stack; we read `size_of::<A>()` bytes out of it as `&[u8]`. The slice
        // borrow is scoped to this call.
        let bytes: &[u8] = unsafe {
            std::slice::from_raw_parts(
                std::ptr::addr_of!(a) as *const u8,
                std::mem::size_of::<A>(),
            )
        };
        let result = self.create_entity(archetype_id, &[(A::component_id(), bytes)]);
        if result.is_ok() {
            // Bytes are now in the pool; pool's drop_fn is the new owner.
            std::mem::forget(a);
        }
        // On Err: no bytes copied; `a` drops normally at scope exit.
        result
    }

    /// Type-safe two-component spawn — see [`spawn_one`] for rationale.
    ///
    /// Mirrors `LegacyQuery::iter_two` (Phase 2d) on the spawn side. Bounded 2-arity.
    ///
    /// # Drop discipline
    ///
    /// Same as [`spawn_one`]: on Ok, both `a` and `b` are byte-copied into
    /// their respective pools and `mem::forget`'d locally so their pool
    /// `drop_fn`s become the new owners. On Err, NEITHER value was copied
    /// (either the archetype guard fired before any copy, or the pool's
    /// `can_push_entity_components` rejected the batch before any pool was
    /// mutated — two-phase commit, C-009), so both values drop normally
    /// at function-exit scope.
    pub fn spawn_two<
        A: crate::ecs::core::component::component::Component,
        B: crate::ecs::core::component::component::Component,
    >(
        &mut self,
        archetype_id: ArchetypeId,
        a: A,
        b: B,
    ) -> EcsResult<Entity> {
        // SAFETY: same rationale as `spawn_one`, applied to both inputs.
        // The two slices view distinct stack locals — no aliasing.
        let bytes_a: &[u8] = unsafe {
            std::slice::from_raw_parts(
                std::ptr::addr_of!(a) as *const u8,
                std::mem::size_of::<A>(),
            )
        };
        let bytes_b: &[u8] = unsafe {
            std::slice::from_raw_parts(
                std::ptr::addr_of!(b) as *const u8,
                std::mem::size_of::<B>(),
            )
        };
        let result = self.create_entity(
            archetype_id,
            &[
                (A::component_id(), bytes_a),
                (B::component_id(), bytes_b),
            ],
        );
        if result.is_ok() {
            std::mem::forget(a);
            std::mem::forget(b);
        }
        result
    }

    /// Cold despawn-hook fire (Phase 14a §3.6 / W1 / §8 P4).
    ///
    /// Fires `on_replace` + `on_remove` for EVERY component of the dying entity,
    /// reading the row PRE-`remove_entity` (Bevy parity, §4.3 firing matrix).
    /// Called by [`Self::delete_entity`] ONLY when the archetype's flag set is
    /// non-empty (some component is hooked), so the ~4 KB id buffer below never
    /// touches `delete_entity`'s prologue — the no-hook hot path keeps its slim
    /// frame (the 0% bench gate). `#[cold] #[inline(never)]` keeps it out of the
    /// hot path's I-cache footprint.
    ///
    /// `archetype_ptr` is the dying entity's `EntityInland::archetype_ptr()`
    /// (write-capable, stable slab provenance); `flags` and `component_ids` are
    /// re-read through it here so the caller carries only the pointer + entity.
    #[cold]
    #[inline(never)]
    fn fire_despawn_hooks(&mut self, entity: Entity, archetype_ptr: *mut Archetype) {
        // Stack buffer (W1): only the touched `[0..n)` prefix is written
        // (`n` ≤ ~32 typical, ≤ MAX_COMPONENTS worst case) — no full memset,
        // no `to_vec()` per-despawn heap alloc. This array lives in THIS cold
        // frame, not `delete_entity`'s.
        let mut id_buf = [ComponentId(0); MAX_COMPONENTS];
        // SAFETY (F1): `archetype_ptr` is the caller's `inland.archetype_ptr()`
        //   — write-capable, stable, interior-mutable (`SharedReadWrite`,
        //   F4-rooted) slab provenance for the EcsMaster's lifetime; it survives
        //   sibling structural writes under TB/SB (whole slab element is
        //   `UnsafeCell`-wrapped). Re-reading `flags` is one `u16` load (no
        //   `&mut` taken).
        let flags = unsafe { (*archetype_ptr).flags };
        let n = {
            // SAFETY (F1): transient SHARED `&Archetype` for the id copy; dropped
            //   at the block close before `world_ptr` is minted, so no `world`-
            //   derived `&mut`/`&` is live across the fire point (SAFETY-1). The
            //   pointer is interior-mutable (`SharedReadWrite`, F4-rooted), so a
            //   prior sibling structural write did not invalidate it.
            let arche = unsafe { &*archetype_ptr };
            let ids = arche.component_ids();
            id_buf[..ids.len()].copy_from_slice(ids);
            ids.len()
            // <-- `&Archetype` drops here.
        };
        // MINT: the shared borrow is dead; no `world`-derived `&mut` is live.
        // The helper takes `&mut self`, so `NonNull::from(&mut *self)` reborrows
        // the dispatcher's exclusive access for the cold fire only.
        let world_ptr = NonNull::from(&mut *self);
        // PRE-DROP (SAFETY-2): on_replace + on_remove for ALL, BEFORE remove.
        // Phase 14b: inner gates widen HOOK -> ANY; per kind, hooks fire first,
        // then observers (§5). The outer `!flags.is_empty()` gate (in
        // `delete_entity`) already covers the observer bits — same `u16` — so it
        // is unchanged; only these inner per-kind tests widen.
        if flags.contains(ArchetypeFlags::ON_REPLACE_ANY) {
            if flags.contains(ArchetypeFlags::ON_REPLACE_HOOK) {
                for &cid in &id_buf[..n] {
                    trigger_on_replace(world_ptr, cid, entity);
                }
            }
            if flags.contains(ArchetypeFlags::ON_REPLACE_OBSERVER) {
                for &cid in &id_buf[..n] {
                    fire_on_replace_observers(world_ptr, cid, entity);
                }
            }
        }
        if flags.contains(ArchetypeFlags::ON_REMOVE_ANY) {
            if flags.contains(ArchetypeFlags::ON_REMOVE_HOOK) {
                for &cid in &id_buf[..n] {
                    trigger_on_remove(world_ptr, cid, entity);
                }
            }
            if flags.contains(ArchetypeFlags::ON_REMOVE_OBSERVER) {
                for &cid in &id_buf[..n] {
                    fire_on_remove_observers(world_ptr, cid, entity);
                }
            }
        }
    }

    /// Deletes an entity and all its components from the system.
    ///
    /// Returns `true` on success, `false` if the entity does not exist or if
    /// archetype removal fails (`RemoveOutcome::PoolFailure`). The explicit
    /// [`RemoveOutcome`] enum (C-006) replaces the previous fragile
    /// `Option<EntityId>`-based logic.
    pub fn delete_entity(&mut self, entity: Entity) -> bool {
        let result = self.delete_entity_core(entity);
        // Direct API: drain on this (post-fire) path. When this method is reached
        // from `DespawnCommand::apply` at depth >= 1, the drain observes
        // `depth != 0` and returns immediately — the outermost owner drains
        // (Q-A1 / C1).
        self.drain_deferred_hook_queue();
        result
    }

    /// Despawns `entity` WITHOUT cascading to its children (Phase 19 W4).
    ///
    /// The opt-out to the default-recursive despawn: the [`Children`] cascade
    /// hook is suppressed for exactly this one removal, so the children survive
    /// — each keeps a now-**dangling** [`ChildOf`] pointing at the freed parent
    /// (a documented footgun; reparent or despawn them explicitly). Equivalent
    /// to Bevy 0.16's `despawn_related`-less single despawn.
    ///
    /// Returns `true` on success, `false` for a stale / never-registered handle
    /// (same contract as [`delete_entity`](Self::delete_entity)).
    ///
    /// [`Children`]: crate::ecs::core::hierarchy::Children
    /// [`ChildOf`]: crate::ecs::core::hierarchy::ChildOf
    pub fn despawn_without_children(&mut self, entity: Entity) -> bool {
        let result = {
            // The guard spans ONLY the hook-fire core, NOT the drain below: the
            // suppress is for THIS entity's cascade hook, and over-suppressing
            // the subsequent drain would wrongly stop unrelated despawns enqueued
            // by other hooks from cascading. Mirrors `DeferredScopeGuard`'s
            // TLS-only discipline (touches no `EcsMaster` field → cannot be
            // frozen by the `&mut self` reborrow).
            let _suppress = crate::ecs::core::hierarchy::commands::CascadeSuppressGuard::enter();
            self.delete_entity_core(entity)
            // <-- `_suppress` drops here, BEFORE the drain.
        };
        self.drain_deferred_hook_queue();
        result
    }

    /// Removal core shared by [`delete_entity`](Self::delete_entity) and
    /// [`despawn_without_children`](Self::despawn_without_children): fires the
    /// pre-remove hooks and releases the row, but does NOT drain the deferred
    /// queue (the caller owns the drain so the suppress window can be scoped
    /// tightly around the fire — Phase 19 W4).
    fn delete_entity_core(&mut self, entity: Entity) -> bool {
        // Phase 14a §3.6 / §8 P1: RAII depth bracket. The two early `return
        // false` paths below PRECEDE the hook-fire point (no command can have
        // been enqueued), so the guard's `Drop` simply restores the depth.
        let scope = DeferredScopeGuard::enter();

        // Resolve the fast inland by value. Copying 16 B releases the
        // entity_master borrow before we dereference the raw archetype_ptr.
        let inland: EntityInland = {
            let Some(slot) = self.entity_master.entities_inland.get(entity.id().0) else {
                return false;
            };
            if slot.is_null() || slot.generation() != entity.generation() {
                return false;
            }
            *slot
        };
        let removed_unit_index = InlandPoolId(inland.unit_index() as usize);

        // Phase 14a §3.6 / W1: PRE-`remove_entity` fire of `on_replace` +
        // `on_remove` for ALL components, reading the dying row. The flags read
        // is one `u16` load (the cheap gate that stays inline here); the
        // ~4 KB `[ComponentId; MAX_COMPONENTS]` id buffer + the trigger loops
        // live in the cold `fire_despawn_hooks` helper, so this hot fn's
        // prologue never reserves that stack slot (§8 P4).
        //
        // SAFETY (F1): `inland.archetype_ptr()` is write-capable, stable,
        //   interior-mutable (`SharedReadWrite`, F4-rooted) slab provenance — it
        //   survives sibling structural writes under TB/SB (whole slab element
        //   is `UnsafeCell`-wrapped). Reading `flags` is one `u16` load.
        let flags = unsafe { (*inland.archetype_ptr()).flags };
        if !flags.is_empty() {
            self.fire_despawn_hooks(entity, inland.archetype_ptr());
        }

        // Re-resolve the `&mut Archetype` and proceed with the removal.
        // SAFETY (U1, U2, U11, U14, F1): archetype_ptr was minted via
        //   archetype_ptr_for under &mut self at registration time; the
        //   bundle slab is heap-stable for the EcsMaster's lifetime and the
        //   pointer is interior-mutable (`SharedReadWrite`, F4-rooted), so it
        //   survives sibling structural writes under TB/SB. Single-threaded
        //   &mut self gives exclusive access and no other live borrow into
        //   this slot exists. Re-resolved AFTER the hooks returned (no live
        //   reborrow during the fire).
        let archetype: &mut Archetype = unsafe { &mut *inland.archetype_ptr() };
        let outcome = archetype.remove_entity(removed_unit_index);

        let result = match outcome {
            RemoveOutcome::Last => {
                self.entity_master.deallocate_entity(entity);
                true
            }
            RemoveOutcome::Swapped { moved_entity: swapped_entity_id } => {
                // The entity that moved into the vacated slot needs its
                // fast-store unit_index updated.
                if let Some(slot) = self.entity_master.entities_inland
                    .get_mut(swapped_entity_id.0)
                {
                    slot.set_unit_index(removed_unit_index.0 as u32);
                }
                self.entity_master.deallocate_entity(entity);
                true
            }
            RemoveOutcome::PoolFailure => false,
        };

        // Drop the bracket (depth back to 0) so the caller's drain runs as the
        // outermost owner.
        drop(scope);
        result
    }

    /// Fast random access read: 3-4 cache lines, ~12-16 ns target.
    ///
    /// Lookup sequence:
    ///   1. `entity_master.entities_inland[entity.id().0]` — 1 line.
    ///   2. Null check + generation check (both fields in the same line as 1).
    ///   3. `(*archetype_ptr).columns[component_id.0]` — 1 line (`columns` at
    ///      offset 0; for `ComponentId.0 < 4` shares the line with the
    ///      archetype deref).
    ///   4. `column.ptr.add(unit_index * stride)` — arithmetic on the
    ///      cached pointer; final line is the component itself.
    ///
    /// Returns `None` for stale entities (generation mismatch), missing
    /// components (column is null), or never-registered entities
    /// (archetype_ptr is null).
    #[inline]
    pub fn get_component_raw(
        &self,
        entity: Entity,
        component_id: ComponentId,
    ) -> Option<*const u8> {
        // Line 1: entity_master.entities_inland[entity.id().0]
        let inland = self.entity_master.entities_inland.get(entity.id().0)?;
        // Null check (dead slot) + generation check (stale handle).
        // Order chosen so the null check covers never-registered IDs first.
        if inland.is_null() {
            return None;
        }
        if inland.generation() != entity.generation() {
            return None;
        }
        // SAFETY (U1, U2, U11, F1): archetype_ptr was minted via the bundle's
        //   `UnsafeCell::raw_get` helper (Step 4 + F4); the slab heap address
        //   is stable for the EcsMaster's lifetime, and the pointer is
        //   interior-mutable (`SharedReadWrite`, F4-rooted) so it survives
        //   sibling structural writes (e.g. a later spawn's `current_index +=
        //   1`) under TB/SB — the whole slab element is `UnsafeCell`-wrapped.
        //   &self gives shared access to the slab.
        let archetype = unsafe { &*inland.archetype_ptr() };

        debug_assert!(component_id.0 < MAX_COMPONENTS);
        // SAFETY (U4): columns is [Column; MAX_COMPONENTS]; bound checked above.
        let column = unsafe { archetype.columns.get_unchecked(component_id.0) };
        if column.ptr.is_null() {
            return None;
        }

        // SAFETY (U5, U6, U10):
        //   - U5: column.ptr / stride are set by refresh_column after add_pool.
        //   - U6: pool buffer pointer is write-once at add_pool (Phase 7 D5
        //     audit table).
        //   - U10: unit_index < archetype.current_index for any alive
        //     entity; multiplication fits because `stride * MAX_ENTITIES`
        //     ≤ pool buffer size, and `unit_index < MAX_ENTITIES`.
        Some(unsafe {
            column.ptr.add(inland.unit_index() as usize * column.stride as usize) as *const u8
        })
    }

    /// Mutable fast random access. `EntityInland` is `Copy`; we copy
    /// 16 B to drop the `EntityMaster` borrow before reborrowing the slab
    /// pointer as `&mut Archetype` (W4 / U14).
    #[inline]
    pub fn get_component_raw_mut(
        &mut self,
        entity: Entity,
        component_id: ComponentId,
    ) -> Option<*mut u8> {
        // Copy the inland by value to release the entity_master borrow.
        let inland: EntityInland = *self.entity_master.entities_inland
            .get(entity.id().0)?;
        if inland.is_null() {
            return None;
        }
        if inland.generation() != entity.generation() {
            return None;
        }
        debug_assert!(component_id.0 < MAX_COMPONENTS);

        // SAFETY (U1, U2, U11, U14, F1):
        //   - U14: archetype_ptr is write-capable provenance (minted via the
        //     bundle's `UnsafeCell::raw_get` helper during create_entity);
        //     single-threaded &mut self gives exclusive access; no other
        //     live borrow into the slot exists.
        //   - F1: interior-mutable (`SharedReadWrite`, F4-rooted) — survives
        //     sibling structural writes under TB/SB (whole slab element is
        //     `UnsafeCell`-wrapped).
        let archetype = unsafe { &mut *inland.archetype_ptr() };

        // SAFETY (U4): same as get_component_raw.
        let column = unsafe { archetype.columns.get_unchecked(component_id.0) };
        if column.ptr.is_null() {
            return None;
        }

        // SAFETY (U5, U6, U10): same as get_component_raw plus
        //   &mut self exclusivity ⇒ the returned *mut points to a uniquely
        //   accessible byte range.
        Some(unsafe {
            column.ptr.add(inland.unit_index() as usize * column.stride as usize)
        })
    }

    /// Fast component write: `~15-18 ns` target. Returns `false`
    /// for stale entities, missing components, or never-registered entities.
    /// On success, byte-copies the provided slice into the component slot.
    ///
    /// `component_bytes.len()` must equal the pool's stride; mismatched
    /// sizes produce undefined behavior in release. Callers should obtain
    /// the slice from a properly-sized `&T` for the target component type
    /// (see [`get_component_mut`] typed wrappers).
    #[inline]
    pub fn set_component_raw(
        &mut self,
        entity: Entity,
        component_id: ComponentId,
        component_bytes: &[u8],
    ) -> bool {
        let Some(dst) = self.get_component_raw_mut(entity, component_id) else {
            return false;
        };
        // Stride is not re-queried here; the size invariant lives at the
        // caller boundary (typed wrappers downcast from `&T` with
        // `size_of::<T>()`). A debug-assertable stride check would require
        // threading the column reference back out of the inner lookup,
        // which defeats the fast-path goal. The pool layer carries the
        // ultimate size guarantee through `Layout`.
        // SAFETY (U5, U6, U10):
        //   - dst is a valid *mut u8 to a byte range of size `stride` for
        //     the target component (U5/U6 — column resolved through the
        //     same fast path as get_component_raw_mut).
        //   - The caller's slice is sized to match by API contract; typed
        //     wrappers enforce this via `size_of::<T>()`.
        //   - Single-threaded &mut self ⇒ no concurrent reader.
        //   - copy_nonoverlapping is sound because the slice and the pool
        //     buffer live in disjoint allocations (slice is a caller-stack
        //     view; the pool buffer lives in the pool's own reservation).
        unsafe {
            std::ptr::copy_nonoverlapping(component_bytes.as_ptr(), dst, component_bytes.len());
        }
        true
    }

    /// Typed read accessor. Returns a shared reference to the
    /// component of type `T` owned by `entity`, or `None` if the entity is
    /// stale, the archetype does not host `T`, or the entity was never
    /// registered.
    #[inline]
    pub fn get_component<T: crate::ecs::core::component::component::Component>(
        &self,
        entity: Entity,
    ) -> Option<&T> {
        let raw = self.get_component_raw(entity, T::component_id())?;
        // SAFETY: the pool was registered with T::component_id(), so the
        //   bytes at `raw` are a valid `T` (M-001 drop-fn / layout guarantee
        //   from the component registry). The lifetime of the returned
        //   reference is bounded by &self.
        Some(unsafe { &*(raw as *const T) })
    }

    /// Typed mutable accessor returning a change-detection-aware [`Mut<T>`]
    /// (Phase 14b W6).
    ///
    /// The direct-API counterpart of querying `Mut<T>` inside a system: writing
    /// through the returned guard (any `DerefMut`, or [`Mut::set_if_neq`]) bumps
    /// the row's `changed_tick`, so a subsequent `Changed<T>` query observes the
    /// write. Returns `None` if the entity is stale (wrong generation), was
    /// never registered, or its archetype does not host `T`.
    ///
    /// # `is_added` / `is_changed` semantics (O4)
    ///
    /// Outside a system there is no `last_run` frame boundary, so this `Mut` is
    /// constructed with `last_run == this_run == current_tick()`. Its
    /// [`Mut::is_added`] / [`Mut::is_changed`] therefore report whether the row
    /// was touched **at the current tick** ("changed relative to the current
    /// tick"), NOT "changed since a previous system run". For frame-delta
    /// semantics, query `Mut<T>` inside a system.
    ///
    /// ## Inside a `Schedule` frame (Bug #56 interaction)
    ///
    /// When called from within a running `Schedule` frame (e.g. an exclusive
    /// `|w: &mut EcsMaster|` system), `current_tick()` is the **apply-window
    /// tick** — one past the frame-start `this_run` that scheduled systems'
    /// `Changed<T>` / `Added<T>` windows are keyed on (the apply-window bump that
    /// makes deferred-command changes observable; see [`Schedule::run`]). A write
    /// made through this guard is therefore observed by a `Changed<T>` /
    /// `Added<T>` reader on the **following** frame (exactly once), like a
    /// deferred-command change — NOT the same frame. For same-frame change
    /// detection within a system, query `Mut<T>` from the system instead (it
    /// stamps at the system's `this_run`).
    ///
    /// [`Schedule::run`]: crate::ecs::core::schedule::schedule::Schedule::run
    #[inline]
    pub fn get_component_mut<T: crate::ecs::core::component::component::Component>(
        &mut self,
        entity: Entity,
    ) -> Option<Mut<'_, T>> {
        // Resolve the inland by value (releases the entity_master borrow before
        // the raw archetype_ptr deref) — same prologue as get_component_raw_mut.
        let inland: EntityInland = *self.entity_master.entities_inland.get(entity.id().0)?;
        if inland.is_null() || inland.generation() != entity.generation() {
            return None;
        }
        let cid = T::component_id();
        debug_assert!(cid.0 < MAX_COMPONENTS);
        let idx = inland.unit_index() as usize;
        let this_run = self.current_tick();

        // SAFETY (OBS-MUT1): `inland.archetype_ptr()` is write-capable, stable,
        //   interior-mutable (`SharedReadWrite`, F4-rooted) slab provenance
        //   (U1/U14/F1); it survives sibling structural writes under TB/SB.
        //   `&mut self` ⇒ exclusive access — no other thread or borrow can read
        //   or write any slot in this archetype for the `Mut`'s lifetime.
        let archetype: &mut Archetype = unsafe { &mut *inland.archetype_ptr() };
        // SAFETY (U4): `cid.0 < MAX_COMPONENTS` (debug-asserted above; the column
        //   table is `[Column; MAX_COMPONENTS]`).
        let column = unsafe { archetype.columns.get_unchecked(cid.0) };
        if column.ptr.is_null() {
            return None;
        }

        // Per-row tick slots come from the COLUMN BASE + idx (NOT the column base
        // alone). `tick_column_base` returns `(added_base, changed_base)`.
        let (added_base, changed_base) = archetype.tick_column_base(cid)?;

        // SAFETY (OBS-MUT2): the row is live (`inland` non-null + generation
        //   match), so `idx < pool.count() <= committed_rows`; both tick bases
        //   are write-once sub-region pointers into the pool's own
        //   `VmReservation` (address-stable for the pool's lifetime — Phase
        //   X.I), and the access stays inside the committed prefix
        //   `[0, committed_rows)` by the bound above.
        //   The `added` read is an eager `Copy` snapshot; `changed_tick` is
        //   offset to this row. The `&mut T` reborrows `column.ptr + idx*stride`,
        //   whose exclusivity rests SOLELY on `&mut self` (OBS-MUT — NOT SCH3:
        //   this is the system-less direct-API path with no conflict graph in
        //   play). The returned `Mut<'_, T>` is tied to `&mut self`, so no
        //   concurrent reader/writer of this row's value or tick can exist.
        let added: Tick = unsafe { *(*added_base.add(idx)).get() };
        let changed_tick: *const UnsafeCell<Tick> = unsafe { changed_base.add(idx) };
        let value: &mut T =
            unsafe { &mut *(column.ptr.add(idx * column.stride as usize) as *mut T) };

        Some(Mut {
            value,
            added,
            changed_tick,
            // O4: no system ran this — there is no frame delta. `last_run ==
            // this_run` makes is_added/is_changed report "newer than
            // (this_run - 1)", i.e. "changed relative to the current tick".
            last_run: this_run,
            this_run,
            deref_mut_called: false,
        })
    }

    /// Fast existence check: 1 cache line, ~5 ns target. Returns `true`
    /// iff the slot for `entity.id()` is live AND its stored generation
    /// matches the handle.
    #[inline]
    pub fn has_entity(&self, entity: Entity) -> bool {
        let Some(inland) = self.entity_master.entities_inland.get(entity.id().0) else {
            return false;
        };
        !inland.is_null() && inland.generation() == entity.generation()
    }

    /// Gets an entity by ID if it exists and is active
    #[inline]
    pub fn get_entity(&self, entity_id: EntityId) -> Option<Entity> {
        self.entity_master.get_entity(entity_id)
    }

    /// Checks if an entity has a specific component.
    ///
    /// Uses the fast inland + column lookup: a null `column.ptr` is the
    /// single source of truth for "archetype does not host this component".
    #[inline]
    pub fn has_component(&self, entity: Entity, component_id: ComponentId) -> bool {
        let Some(inland) = self.entity_master.entities_inland.get(entity.id().0) else {
            return false;
        };
        if inland.is_null() || inland.generation() != entity.generation() {
            return false;
        }
        // SAFETY (U1, U2, U11, F1): archetype_ptr is stable, interior-mutable
        //   (`SharedReadWrite`, F4-rooted) slab provenance — survives sibling
        //   structural writes under TB/SB (whole slab element is
        //   `UnsafeCell`-wrapped).
        let archetype = unsafe { &*inland.archetype_ptr() };
        if component_id.0 >= MAX_COMPONENTS {
            return false;
        }
        // SAFETY (U4): bounded by check above.
        !unsafe { archetype.columns.get_unchecked(component_id.0) }.ptr.is_null()
    }

    /// Gets the archetype ID containing the specified entity.
    ///
    /// Derives the id from the fast inland's slab pointer via
    /// [`Archetype::id`] — no SparseMap traversal.
    #[inline]
    pub fn get_entity_archetype_id(&self, entity: Entity) -> Option<ArchetypeId> {
        let inland = self.entity_master.entities_inland.get(entity.id().0)?;
        if inland.is_null() || inland.generation() != entity.generation() {
            return None;
        }
        // SAFETY (U1, U2, U11, F1): same as get_component_raw — stable,
        //   interior-mutable (`SharedReadWrite`, F4-rooted) slab provenance.
        let archetype = unsafe { &*inland.archetype_ptr() };
        Some(archetype.id())
    }

    /// Gets the total number of active entities in the system
    #[inline]
    pub fn entity_count(&self) -> usize {
        self.entity_master.entity_count()
    }

    /// Gets the number of archetypes in the system
    #[inline]
    pub fn archetype_count(&self) -> usize {
        self.archetype_master.archetype_count()
    }

    /// Gets the number of recycled entity IDs available for reuse
    #[inline]
    pub fn recycled_entity_count(&self) -> usize {
        self.entity_master.recycled_entity_count()
    }

    /// Gets an iterator over all active entities
    #[inline]
    pub fn iter_entities(&self) -> impl Iterator<Item = Entity> + '_ {
        self.entity_master.iter_entities()
    }

    /// Queries entities that have all specified components
    pub fn query_entities(&self, component_ids: &[ComponentId]) -> Vec<Entity> {
        let archetype_ids = self.archetype_master.find_archetypes_with_components(component_ids);
        let mut result = Vec::new();
        
        for archetype_id in archetype_ids {
            if let Some(archetype) = self.archetype_master.get_archetype(archetype_id) {
                // Get all entity IDs from this archetype
                for unit_index in 0..archetype.entity_count() {
                    if let Some(entity_id) = archetype.get_entity_id_at(InlandPoolId(unit_index))
                        && let Some(entity) = self.entity_master.get_entity(entity_id)
                    {
                        result.push(entity);
                    }
                }
            }
        }
        
        result
    }

    /// Gets raw pointers to multiple components for an entity.
    ///
    /// Resolves the inland record once, then walks `component_ids` reading
    /// the cached `Column` table inline. Returns `(ComponentId, *const u8)`
    /// pairs only for components actually hosted by the entity's archetype.
    pub fn get_components_raw(
        &self,
        entity: Entity,
        component_ids: &[ComponentId],
    ) -> Vec<(ComponentId, *const u8)> {
        let mut result = Vec::with_capacity(component_ids.len());
        let Some(inland) = self.entity_master.entities_inland.get(entity.id().0) else {
            return result;
        };
        if inland.is_null() || inland.generation() != entity.generation() {
            return result;
        }
        // SAFETY (U1, U2, U11, F1): archetype_ptr is stable, interior-mutable
        //   (`SharedReadWrite`, F4-rooted) slab provenance — survives sibling
        //   structural writes under TB/SB (whole slab element is
        //   `UnsafeCell`-wrapped).
        let archetype = unsafe { &*inland.archetype_ptr() };
        let unit_index = inland.unit_index() as usize;
        for &component_id in component_ids {
            if component_id.0 >= MAX_COMPONENTS {
                continue;
            }
            // SAFETY (U4): bounded by check above.
            let column = unsafe { archetype.columns.get_unchecked(component_id.0) };
            if column.ptr.is_null() {
                continue;
            }
            // SAFETY (U5, U6, U10): same as get_component_raw.
            let ptr = unsafe { column.ptr.add(unit_index * column.stride as usize) } as *const u8;
            result.push((component_id, ptr));
        }
        result
    }

    /// Gets mutable raw pointers to multiple components for an entity.
    ///
    /// Mutable counterpart of [`get_components_raw`]; the inland is copied
    /// by value (16 B) to release the `entity_master` borrow before the
    /// `archetype_ptr` is reborrowed as `&mut Archetype` (W4 / U14).
    pub fn get_components_raw_mut(
        &mut self,
        entity: Entity,
        component_ids: &[ComponentId],
    ) -> Vec<(ComponentId, *mut u8)> {
        let mut result = Vec::with_capacity(component_ids.len());
        let inland: EntityInland = match self.entity_master.entities_inland
            .get(entity.id().0)
        {
            Some(i) => *i,
            None => return result,
        };
        if inland.is_null() || inland.generation() != entity.generation() {
            return result;
        }
        // SAFETY (U1, U2, U11, U14, F1): write-capable, interior-mutable
        //   (`SharedReadWrite`, F4-rooted) slab provenance under &mut self —
        //   survives sibling structural writes under TB/SB (whole slab element
        //   is `UnsafeCell`-wrapped); no other live borrow into this slot.
        let archetype = unsafe { &mut *inland.archetype_ptr() };
        let unit_index = inland.unit_index() as usize;
        for &component_id in component_ids {
            if component_id.0 >= MAX_COMPONENTS {
                continue;
            }
            // SAFETY (U4): bounded by check above.
            let column = unsafe { archetype.columns.get_unchecked(component_id.0) };
            if column.ptr.is_null() {
                continue;
            }
            // SAFETY (U5, U6, U10): same as get_component_raw_mut.
            let ptr = unsafe { column.ptr.add(unit_index * column.stride as usize) };
            result.push((component_id, ptr));
        }
        result
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

    /// Preregisters event type `E` with a custom config.
    ///
    /// Must be called before the first `send_event::<E>` or `events_of::<E>`.
    /// All write lanes and the reader buffer are allocated here; no allocation
    /// occurs during steady-state `send_event` or `update_events`.
    ///
    /// # Errors
    ///
    /// Forwards errors from [`EventDispatcher::preregister`].
    #[inline]
    pub fn preregister_event<E: Event>(&mut self, cfg: EventConfig) -> EcsResult<()> {
        self.events.preregister::<E>(cfg)
    }

    /// Preregisters event type `E` with default capacity and the dispatcher's
    /// validated `default_thread_count`.
    ///
    /// Equivalent to calling [`preregister_event`] with
    /// `EventConfig::default_for(self.events.default_thread_count())`.
    ///
    /// # Errors
    ///
    /// Forwards errors from [`EventDispatcher::preregister`].
    ///
    /// [`preregister_event`]: EcsMaster::preregister_event
    #[inline]
    pub fn preregister_event_default<E: Event>(&mut self) -> EcsResult<()> {
        let cfg = EventConfig::default_for(self.events.default_thread_count())
            .expect("invariant: default_thread_count was validated at EventDispatcher::new");
        self.events.preregister::<E>(cfg)
    }

    /// Sends a single event of type `E` to the lane for `thread_index`.
    ///
    /// # Errors
    ///
    /// Forwards errors from [`EventDispatcher::send`].
    #[inline]
    pub fn send_event<E: Event>(&self, thread_index: u32, event: E) -> EcsResult<()> {
        self.events.send::<E>(thread_index, event)
    }

    /// Returns the slice of events of type `E` from the previous frame.
    ///
    /// Returns an empty slice if `E` was not registered or if no events were
    /// sent last frame. Slice remains valid until the next `update_events` call.
    #[inline]
    pub fn events_of<E: Event>(&self) -> &[E] {
        self.events.events::<E>()
    }

    /// Advances the frame counter and flattens write lanes into reader buffers.
    ///
    /// Must be called once per frame. After this call, `events_of::<E>()` returns
    /// the events sent during the frame that just ended.
    #[inline]
    pub fn update_events(&mut self) {
        self.events.update_events();
    }

    // ── System execution (Phase 8a Step 8) ──────────────────────────────────

    /// Runs a single [`System`] once, end-to-end.
    ///
    /// Generic over `S: System` so the caller's system value survives across
    /// calls without virtual dispatch. Sequence:
    ///   1. [`System::initialize`] — idempotent two-phase init (state then
    ///      access surface); subsequent calls short-circuit so cross-call
    ///      `&mut S` reuse is supported.
    ///   2. [`UnsafeEcsCell::new_mutable`] — mints a write-capable cell
    ///      bound to the `&mut self` borrow scope.
    ///   3. [`System::run_unsafe`] — invokes the system body.
    ///
    /// Phase 9's scheduler will replace this method with a multi-system
    /// runner that resolves aliasing via the `Access` conflict graph; for
    /// now `&mut EcsMaster` enforces the S1 invariant trivially.
    ///
    /// [`System`]: crate::ecs::core::system::system::System
    /// [`System::initialize`]: crate::ecs::core::system::system::System::initialize
    /// [`System::run_unsafe`]: crate::ecs::core::system::system::System::run_unsafe
    /// [`UnsafeEcsCell::new_mutable`]: crate::ecs::core::system::unsafe_ecs_cell::UnsafeEcsCell::new_mutable
    pub fn run_system_once<S: System>(&mut self, system: &mut S) -> S::Out {
        system.initialize(self);
        // SAFETY (U_C1): `cell` does not outlive the `&mut self` borrow — it
        //   is consumed by `run_unsafe` on the next line and cannot escape.
        let cell = unsafe { UnsafeEcsCell::new_mutable(self) };
        // SAFETY (S1): `&mut self` is exclusive for the entire call ⇒ no
        //   other `System::run_unsafe` is in flight on this `EcsMaster`.
        //   The Phase 9 scheduler will replace this trivial enforcement
        //   with the `Access` conflict graph.
        unsafe { system.run_unsafe(cell) }
    }

    /// Deprecated alias for [`run_system`](EcsMaster::run_system), retained
    /// for Phase 8a callsite compatibility (W3 turbofish form removed —
    /// the closure's param type now infers from its signature).
    ///
    /// ```ignore
    /// // Phase 8a (W3 turbofish — no longer accepted):
    /// // ecs.run_closure_once::<(Res<A>, ResMut<B>), _, _>(|(a, b)| { /* ... */ });
    ///
    /// // Phase 8c (post Step 5 — closure-annotation form):
    /// ecs.run_closure_once(|(a, b): (Res<A>, ResMut<B>)| { /* ... */ });
    /// ```
    ///
    /// New code should call [`run_system`](EcsMaster::run_system) directly;
    /// `run_closure_once` is preserved as a compatibility shim and may be
    /// removed in Phase 9.
    ///
    /// [`run_system`]: EcsMaster::run_system
    #[inline]
    pub fn run_closure_once<F, M, Out>(&mut self, body: F) -> Out
    where
        F: IntoSystem<(), Out, M>,
        F::System: System<Out = Out>,
    {
        self.run_system(body)
    }

    // ── Phase 8c Step 4: `run_system` / `run_cached_system` ──────────────────

    /// Build a one-shot system from any function `F: SystemParamFunction<M>`
    /// (via [`IntoSystem`]), run it once, flush its deferred buffers, and
    /// discard.
    ///
    /// The function is moved in; if you want to amortise the state init
    /// across many invocations, use [`run_cached_system`] with a pre-built
    /// [`FunctionSystem`] hoisted outside your loop. Per-call `run_system`
    /// rebuilds the system on every call (≈ 1 µs cold init + ≤ 30 ns
    /// dispatch + closure body + apply — see plan §1.2 first-call row).
    ///
    /// # Example
    ///
    /// ```ignore
    /// ecs.run_system(|res: Res<MyResource>| {
    ///     println!("{}", res.0);
    /// });
    /// ```
    ///
    /// # Borrow-checker enforced invariants (S1, APP4)
    ///
    /// `&mut self` is exclusive for the entire call; no other `System` can
    /// be in flight on the same world, and no `apply` re-entry into
    /// `run_system` / `run_cached_system` / `run_system_once` is reachable
    /// (Rust's borrow checker rejects the nested `&mut`).
    ///
    /// [`IntoSystem`]: crate::ecs::core::system::into_system::IntoSystem
    /// [`FunctionSystem`]: crate::ecs::core::system::function_system::FunctionSystem
    /// [`run_cached_system`]: EcsMaster::run_cached_system
    pub fn run_system<F, M, Out>(&mut self, system: F) -> Out
    where
        F: IntoSystem<(), Out, M>,
        F::System: System<Out = Out>,
    {
        let mut sys = F::into_system(system);
        self.run_cached_system(&mut sys)
    }

    /// Run a pre-built [`System`] once, flushing its deferred buffers.
    ///
    /// Sequence (plan §17 / §9.5):
    ///   1. [`System::initialize`] — idempotent (FS1). Re-running the same
    ///      cached system pays the init cost only on the first call.
    ///   2. [`UnsafeEcsCell::new_mutable`] — mints the write-capable cell
    ///      bound to the `&mut self` borrow scope.
    ///   3. [`System::run_unsafe`] — body execution under invariant S1.
    ///   4. [`System::apply`] — flushes per-`SystemParam` deferred buffers
    ///      (e.g. `Commands<'s>`'s [`CommandQueue`]) under `&mut self`.
    ///      APP1' — safe method; APP4 — must not re-enter the runner.
    ///
    /// Phase 9's scheduler will replace this method with a multi-system
    /// runner that resolves aliasing via the [`Access`] conflict graph; for
    /// now `&mut EcsMaster` enforces the S1 invariant trivially.
    ///
    /// [`System`]: crate::ecs::core::system::system::System
    /// [`System::initialize`]: crate::ecs::core::system::system::System::initialize
    /// [`System::run_unsafe`]: crate::ecs::core::system::system::System::run_unsafe
    /// [`System::apply`]: crate::ecs::core::system::system::System::apply
    /// [`UnsafeEcsCell::new_mutable`]: crate::ecs::core::system::unsafe_ecs_cell::UnsafeEcsCell::new_mutable
    /// [`CommandQueue`]: crate::ecs::core::commands::command_queue::CommandQueue
    /// [`Access`]: crate::ecs::core::system::access::Access
    pub fn run_cached_system<S>(&mut self, system: &mut S) -> S::Out
    where
        S: System,
    {
        system.initialize(self);
        // SAFETY (U_C1): `cell` does not outlive the `&mut self` borrow — it
        //   is consumed by `run_unsafe` on the next line and cannot escape.
        let cell = unsafe { UnsafeEcsCell::new_mutable(self) };
        // SAFETY (S1): `&mut self` is exclusive for the entire call ⇒ no
        //   other `System::run_unsafe` is in flight on this `EcsMaster`.
        //   The Phase 9 scheduler will replace this trivial enforcement
        //   with the `Access` conflict graph.
        let out = unsafe { system.run_unsafe(cell) };
        // APP1' (Round 3 / O3'): `apply` is a SAFE method; the borrow
        //   checker (still holding `&mut self`) prevents re-entry per APP4.
        system.apply(self);
        // NEW-2: drain the world-resident deferred-hook queue so commands a
        // hook/observer enqueued during `apply` (via `DeferredEcsMaster`) are
        // actually applied. This mirrors `Schedule::run`'s apply-window barrier
        // drain (schedule.rs:560 / :889); without it the single-system runner
        // silently loses nested deferred commands. The drain is depth-0-gated
        // (TLS via `hooks::scope`) and `run_cached_system` is a top-level
        // `&mut self` entry at depth 0 — same self-draining discipline the
        // direct-API methods (`create_entity` / `delete_entity`) use.
        self.drain_deferred_hook_queue();
        out
    }

    /// Run a type-erased read-only **run condition** once on `&mut self`,
    /// returning its `bool` verdict (Phase 16, `PHASE-16-PLAN.md` §5.1).
    ///
    /// Mirrors the [`run_cached_system`](Self::run_cached_system) sequence
    /// but takes a `?Sized` `dyn System<Out = bool>` receiver (so it accepts
    /// a `&mut BoolSystem` via `Box::as_mut`) and DELIBERATELY OMITS the
    /// `apply` step:
    ///
    /// 1. [`System::initialize`] — idempotent (FS1); already ran at build,
    ///    so this is a no-op every frame.
    /// 2. [`UnsafeEcsCell::new_mutable`] — write-capable cell bound to the
    ///    `&mut self` borrow scope.
    /// 3. [`System::run_unsafe`] — the predicate body; returns the `bool`.
    ///
    /// # No `apply` (orchestrator decision, §0-P6a)
    ///
    /// Conditions are pure read-only predicates. A condition that uses
    /// `Commands` / `EventWriter` is a documented logic error; its deferred
    /// commands are DROPPED here (never flushed mid-eval-pass) rather than
    /// applied — flushing structural mutations between two conditions in the
    /// same eval pass would let the second condition observe a half-applied
    /// world. The read-only contract is `debug_assert!`ed at build
    /// (`schedule_builder.rs` Step 1).
    ///
    /// # Change-detection ticks (Phase 16.1)
    ///
    /// This method advances the condition's `(last_run, this_run]` snapshot via
    /// [`System::set_change_ticks`] — but ONLY here, on a frame the condition is
    /// actually evaluated (Bevy "since-last-actual-run" parity). `last_run`
    /// becomes the condition's PREVIOUS `this_run` (frozen across every frame it
    /// was skipped), and `this_run` becomes the caller's frame-start tick
    /// (`Schedule::frame_this_run`). A condition dormant for N frames (gated by
    /// a false set/state condition, or whose members are blocked by
    /// `pred_remaining`) therefore resumes observing ALL changes since its last
    /// actual run — not just since the last frame — so a `Changed<T>` /
    /// `Added<T>` / `Ref<T>` condition no longer silently misses dormant changes
    /// (nor reports always-true). For a condition evaluated every frame the
    /// window is identical to the old frame-start bump.
    ///
    /// [`System::set_change_ticks`]: crate::ecs::core::system::system::System::set_change_ticks
    ///
    /// # Caller precondition
    ///
    /// The dispatcher holds the unique `&mut EcsMaster`, recovered at the
    /// apply-window boundary where `running.count_ones() == 0` — so no
    /// worker holds a live cell copy (the S1 contract). The only call site is
    /// `Schedule::evaluate_ready_conditions` / `set_gate`.
    ///
    /// [`System`]: crate::ecs::core::system::system::System
    /// [`System::initialize`]: crate::ecs::core::system::system::System::initialize
    /// [`System::run_unsafe`]: crate::ecs::core::system::system::System::run_unsafe
    /// [`UnsafeEcsCell::new_mutable`]: crate::ecs::core::system::unsafe_ecs_cell::UnsafeEcsCell::new_mutable
    pub(crate) fn run_condition(
        &mut self,
        condition: &mut dyn System<Out = bool>,
        this_run: Tick,
    ) -> bool {
        // FS1 no-op after build — conditions are initialized once in
        // `ScheduleBuilder::try_build` Step 1, so their `Access` + `Local`
        // state are already live before the first frame.
        condition.initialize(self);
        // Phase 16.1 (Gap #1): advance the condition's tick snapshot ONLY now,
        // on a frame it is actually evaluated. `prev` is the condition's
        // PREVIOUS `this_run` (frozen across skipped frames); the new `this_run`
        // is the dispatcher's `frame_this_run`. This is the single write site
        // for a condition's ticks — there is NO frame-start condition bump.
        let prev = condition.meta().this_run();
        condition.set_change_ticks(prev, this_run);
        // SAFETY (S1 / Phase 16 CR2): `&mut self` is the dispatcher's unique
        //   exclusive borrow on the world, recovered at the apply-window
        //   boundary where `running == 0` (caller-checked) ⇒ no worker holds
        //   a cell copy. `cell` is consumed by `run_unsafe` on the next line
        //   and cannot escape, so no aliasing `UnsafeEcsCell` is minted.
        let cell = unsafe { UnsafeEcsCell::new_mutable(self) };
        // SAFETY (S1): as above — no other `System::run_unsafe` is in flight
        //   on this `EcsMaster` (single-threaded eval at the barrier). The
        //   cell does not outlive this statement.
        unsafe { condition.run_unsafe(cell) }
    }

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

    /// Registers lifecycle hooks for component type `C` at runtime, returning a
    /// chainable [`ComponentHooksBuilder`] (Phase 14a, plan §6.3 / REG).
    ///
    /// ```ignore
    /// world.register_component_hooks::<Health>()
    ///      .on_add(my_on_add)
    ///      .on_remove(my_on_remove);
    /// ```
    ///
    /// The builder commits the accumulated hooks when it is dropped (or
    /// [`finish`](ComponentHooksBuilder::finish)ed). It is the runtime
    /// counterpart of the `#[component(...)]` derive attribute and covers
    /// hand-written `impl Component` / foreign types that the derive cannot
    /// reach.
    ///
    /// # Derive XOR runtime (mutually exclusive)
    ///
    /// A component declares hooks via EITHER `#[component(...)]` OR this runtime
    /// builder — never both. Each `HOOKS` slot is written exactly once. Calling
    /// this method for a type that carries `#[component(...)]` (i.e.
    /// `C::HAS_HOOKS == true`) panics immediately: the derive already installed
    /// the slot, and the two mechanisms must not be mixed.
    ///
    /// # Register-before-use (staleness rule, plan §6.4 / Q-A5; Phase 21 H1)
    ///
    /// Hooks for `C` MUST be registered before `C` first appears in any
    /// archetype **of any world in this process**. An archetype's
    /// [`ArchetypeFlags`] are OR-computed once at construction from the cold
    /// `HOOKS` table; hooks installed *after* an archetype containing `C`
    /// already exists would leave that archetype's flag bit unset and the hook
    /// silently skipped. To make that bug impossible, this method checks the
    /// process-global per-`ComponentId` "ever placed in any archetype" bitmask
    /// (set at every archetype-creation funnel) and **panics** (in release,
    /// not just debug) if the bit is set. The pre-Phase-21 per-world archetype
    /// scan was world-blind: a SECOND world with `C` live would get
    /// silently-skipped hooks because its pre-install archetypes' flags lacked
    /// the bit — the global gate closes that hole. The derive path is
    /// staleness-immune by construction (hooks install inside
    /// `component_id()`, which always precedes the first archetype containing
    /// the component).
    ///
    /// # Multi-world scope (Phase 21)
    ///
    /// Hooks are **process-global per type** — registered once, they fire in
    /// ALL worlds (the `HOOKS` table is `static`). Observers, by contrast, are
    /// **per-world** (`ObserverRegistry` lives on each world's
    /// `ArchetypeMaster`). The asymmetry is by design: hooks are part of a
    /// component type's definition; observers are runtime-mutable per-world
    /// reactions.
    ///
    /// # Panics
    ///
    /// - If `C` declares `#[component(...)]` derive hooks (`C::HAS_HOOKS ==
    ///   true`) — derive and the runtime builder are mutually exclusive.
    /// - If `C` was ever placed in a live archetype of ANY world in this
    ///   process — register hooks before the component is first used.
    #[cold]
    pub fn register_component_hooks<C: Component>(&mut self) -> ComponentHooksBuilder<'_> {
        // Force `C::component_id()`: mints the id and, for a derive-hooked type
        // (`C::HAS_HOOKS == true`), installs those hooks into the slot. A plain
        // `#[derive(Component)]` installs nothing, leaving the slot free for the
        // runtime builder to commit.
        let component_id = C::component_id();

        // Eager derive-XOR-runtime collision check (Wave-5 soundness fix /
        // Change 3): a type carrying `#[component(...)]` already owns its `HOOKS`
        // slot, so the runtime builder must not also write it. Reject at the
        // registration call site — a clearer, earlier error than the builder's
        // `Drop` commit panic (which remains as defense in depth for a
        // hand-`impl Component` with an inconsistent `HAS_HOOKS`).
        if C::HAS_HOOKS {
            register_component_hooks_derive_conflict_panic::<C>();
        }

        // Release-level staleness gate (Q-A5 / W3; Phase 21 H1): a stale
        // `ArchetypeFlags` bit would silently skip the hook, which is too
        // severe a correctness surprise for a feature whose entire value is
        // "the callback fires". The gate is the PROCESS-GLOBAL "ever placed in
        // any archetype" bitmask — matching the process-global scope of the
        // `HOOKS` table itself — because the old per-world archetype scan was
        // blind to other worlds already holding `C` (audit H1). The global
        // subsumes the per-world scan: every archetype of this world was
        // minted through a funnel that set the bit. Cold + one-time; one
        // Relaxed load (the panic is a config-time courtesy, not a soundness
        // fence).
        if component_registry::was_ever_archetyped(component_id.0) {
            register_component_hooks_stale_panic::<C>();
        }

        ComponentHooksBuilder::new(component_id.0)
    }

    // ── Phase 14b: component lifecycle observers (runtime-mutable) ──────────
    //
    // Unlike `register_component_hooks` (write-once per type, staleness-panics
    // if an archetype containing `C` already exists), observers are
    // runtime-mutable: `ArchetypeMaster::add_observer` runs the dynamic
    // add-first archetype walk, raising the `ON_{kind}_OBSERVER` bit on every
    // already-existing archetype containing `C`. There is therefore NO
    // staleness panic — late registration is handled by the walk.

    /// Registers an `on_add` observer for component `C`, returning a stable
    /// [`ObserverId`] for later [`Self::remove_observer`] (Phase 14b).
    ///
    /// The `runner` fires after the per-component `on_add` hook at every
    /// structural op that newly adds `C` to an entity. Observers are
    /// runtime-mutable, so this may be called even after archetypes containing
    /// `C` exist — the dynamic archetype walk raises the flag bit on them.
    #[inline]
    pub fn observe_on_add<C: Component>(&mut self, runner: ObserverFn) -> ObserverId {
        self.archetype_master
            .add_observer(ObserverKind::Add, C::component_id(), runner)
    }

    /// Registers an `on_insert` observer for component `C`, returning a stable
    /// [`ObserverId`] (Phase 14b). See [`Self::observe_on_add`] for semantics.
    #[inline]
    pub fn observe_on_insert<C: Component>(&mut self, runner: ObserverFn) -> ObserverId {
        self.archetype_master
            .add_observer(ObserverKind::Insert, C::component_id(), runner)
    }

    /// Registers an `on_replace` observer for component `C`, returning a stable
    /// [`ObserverId`] (Phase 14b). Fires before an existing `C` value is
    /// overwritten (and, on despawn, for the dying value). See
    /// [`Self::observe_on_add`].
    #[inline]
    pub fn observe_on_replace<C: Component>(&mut self, runner: ObserverFn) -> ObserverId {
        self.archetype_master
            .add_observer(ObserverKind::Replace, C::component_id(), runner)
    }

    /// Registers an `on_remove` observer for component `C`, returning a stable
    /// [`ObserverId`] (Phase 14b). Fires before `C` is removed from an entity
    /// (and, on despawn, for the dying value). See [`Self::observe_on_add`].
    #[inline]
    pub fn observe_on_remove<C: Component>(&mut self, runner: ObserverFn) -> ObserverId {
        self.archetype_master
            .add_observer(ObserverKind::Remove, C::component_id(), runner)
    }

    /// Registers `runner` as a `kind` observer for the component identified by
    /// `cid`, returning a stable [`ObserverId`] (Phase 14b).
    ///
    /// The type-erased sibling of the `observe_on_*::<C>` helpers: prefer those
    /// where the component type is statically known. This form is for callers
    /// that already hold a resolved [`ComponentId`].
    #[inline]
    pub fn add_observer(
        &mut self,
        kind: ObserverKind,
        cid: ComponentId,
        runner: ObserverFn,
    ) -> ObserverId {
        self.archetype_master.add_observer(kind, cid, runner)
    }

    /// Removes the observer with `id`, returning `true` if it was registered
    /// (Phase 14b).
    ///
    /// On removal of the last observer for its `(kind, component)` pair, the
    /// corresponding `ON_{kind}_OBSERVER` archetype flag bits are recomputed
    /// (cleared where no sibling component still observes that kind, hook bits
    /// preserved).
    #[inline]
    pub fn remove_observer(&mut self, id: ObserverId) -> bool {
        self.archetype_master.remove_observer(id)
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

    // ── Phase 17: State facade ──────────────────────────────────────────────

    /// Inserts the three resources that back state type `S` — `State<S> =
    /// initial`, `NextState<S> = Unchanged`, and the per-`S` transition record
    /// — into the world (Phase 17 D7).
    ///
    /// This mutates the world only; it does **not** register the schedule-side
    /// `StateEntry` that drives the transition pass. Use
    /// `ScheduleBuilder::insert_state::<S>(initial)` to both insert the
    /// resources and have the schedule fire the initial `OnEnter` and drain
    /// transitions each frame.
    ///
    /// # Initial-transition interaction
    /// Calling `set_next_state::<S>(..)` (or otherwise queuing a `Pending`)
    /// **before the first `Schedule::run`** suppresses the initial `OnEnter`:
    /// the synthesized `none → initial` transition is overwritten in the same
    /// first pass by the real `initial → requested` transition, so
    /// `on_enter(initial)`-gated systems do NOT run — only `on_enter(requested)`
    /// does. Queue the first transition from *inside* a system (it lands on the
    /// next frame's pass) if you need the initial `OnEnter` to fire first.
    #[cold]
    pub fn insert_state<S: States>(&mut self, initial: S) {
        self.insert_resource(State::new(initial));
        self.insert_resource(NextState::<S>::Unchanged);
        self.insert_resource(StateTransitionRecord::<S>::default());
    }

    /// Inserts the resources backing state type `S` using `S::default()` as the
    /// initial value (Phase 17 D7). Shorthand for `insert_state(S::default())`.
    ///
    /// # Initial-transition interaction
    /// Calling `set_next_state::<S>(..)` (or otherwise queuing a `Pending`)
    /// **before the first `Schedule::run`** suppresses the initial `OnEnter`:
    /// the synthesized `none → initial` transition is overwritten in the same
    /// first pass by the real `initial → requested` transition, so
    /// `on_enter(initial)`-gated systems do NOT run — only `on_enter(requested)`
    /// does. Queue the first transition from *inside* a system (it lands on the
    /// next frame's pass) if you need the initial `OnEnter` to fire first.
    #[cold]
    pub fn init_state<S: States + Default>(&mut self) {
        self.insert_state(S::default());
    }

    /// Returns a shared reference to the current value of state type `S`.
    ///
    /// # Panics
    ///
    /// Panics if `State<S>` was never inserted (via `insert_state` /
    /// `init_state`, or the matching builder methods).
    #[inline]
    pub fn state<S: States>(&self) -> &S {
        self.resource::<State<S>>().get()
    }

    /// Queues a transition of state type `S` to `value`, applied by the next
    /// `Schedule::run`'s transition pass (last-write-wins within a frame).
    ///
    /// Shorthand for `self.resource_mut::<NextState<S>>().set(value)`.
    ///
    /// # Initial-transition interaction
    /// Calling `set_next_state::<S>(..)` (or otherwise queuing a `Pending`)
    /// **before the first `Schedule::run`** suppresses the initial `OnEnter`:
    /// the synthesized `none → initial` transition is overwritten in the same
    /// first pass by the real `initial → requested` transition, so
    /// `on_enter(initial)`-gated systems do NOT run — only `on_enter(requested)`
    /// does. Queue the first transition from *inside* a system (it lands on the
    /// next frame's pass) if you need the initial `OnEnter` to fire first.
    ///
    /// # Panics
    ///
    /// Panics if `NextState<S>` was never inserted (via `insert_state` /
    /// `init_state`, or the matching builder methods).
    #[inline]
    pub fn set_next_state<S: States>(&mut self, value: S) {
        self.resource_mut::<NextState<S>>().set(value);
    }

    // ── Phase 8.5: Bundle archetype-id cache (SBC4) ─────────────────────────
    //
    // Phase 8.5 step-scoped `dead_code` allow on the two helpers below: the
    // first production caller lands in Step 5 (`SpawnCommand::apply`
    // rewrite), routed through the derive-generated `B::cached_archetype_id`
    // (Step 4). Remove the `#[allow(dead_code)]` then.

    /// Resolves the [`ArchetypeId`] for Bundle `B` in this world, lazily
    /// caching on the first call. Subsequent calls hit the cache (~3 ns).
    ///
    /// # Hot path (plan §6.2)
    ///
    /// 1. `B::bundle_type_id()` — single Acquire load on the per-impl
    ///    `OnceLock<BundleStaticInfo>` (~2 ns).
    /// 2. `self.bundle_archetype_cache[id.0].get()` — Acquire load on a
    ///    stable address (~1 ns).
    /// 3. If `Some(arch)`: return.
    /// 4. If `None`: fall into the cold path —
    ///    [`Self::cold_register_bundle_archetype`] resolves via
    ///    [`Self::get_or_create_archetype`] and `OnceLock::set`s the slot
    ///    (~1 µs).
    ///
    /// # Why `&mut self`
    ///
    /// The cold path calls [`Self::get_or_create_archetype`], which requires
    /// `&mut self` (it may register a new archetype). The hot path could be
    /// `&self`-only, but keeping the unified `&mut self` signature lets the
    /// caller (always `SpawnCommand::apply` post-Step 5) match its own
    /// `&mut EcsMaster` receiver. A `&self`-only fast-path accessor would
    /// be a Phase 9 design item (DEFERRED).
    ///
    /// # Visibility
    ///
    /// `pub(crate)` — user code does not call this directly. The
    /// `#[derive(Bundle)]`-generated `cached_archetype_id` (Step 4) is the
    /// blessed entry point; `SpawnCommand::apply` (Step 5) is the only
    /// in-tree caller.
    ///
    /// **Visibility note**: `pub`, not `pub(crate)`. The `#[derive(Bundle)]`
    /// macro in `boyko_macros` emits user-crate code calling this method
    /// from inside the generated `impl Bundle for UserType` block (specifically
    /// from `cached_archetype_id`). Direct user code SHOULD NOT call this —
    /// it is a macro-only API. The blessed surface for user code is
    /// `Commands::spawn(bundle)` (Phase 8.5 Step 5). Same soft-seal pattern
    /// as Bevy's `World::register_bundle_info`.
    #[allow(dead_code)]
    #[inline]
    pub fn bundle_archetype_id_for<B: Bundle>(&mut self) -> ArchetypeId {
        let type_id = B::bundle_type_id();
        debug_assert!(
            type_id.0 < MAX_BUNDLE_TYPES,
            "BundleTypeId out of bounds — saturate-then-panic in register_new should have prevented this"
        );

        if let Some(arch) = self.bundle_archetype_cache()[type_id.0].get() {
            return *arch;
        }

        self.cold_register_bundle_archetype::<B>(type_id)
    }

    /// Cold-path slot installer for [`Self::bundle_archetype_id_for`].
    ///
    /// Computes the canonical component-id list for `B`, registers (or
    /// reuses) the matching archetype, and publishes the result into the
    /// per-world cache slot. Idempotent: if another caller raced ahead and
    /// already populated the slot with an identical id (canonical-sorted
    /// ids + idempotent [`Self::get_or_create_archetype`] = deterministic
    /// `ArchetypeId`), [`OnceLock::set`] returns `Err` which we ignore and
    /// read back the winner's value.
    #[allow(dead_code)]
    #[cold]
    #[inline(never)]
    fn cold_register_bundle_archetype<B: Bundle>(
        &mut self,
        type_id: BundleTypeId,
    ) -> ArchetypeId {
        let ids = B::component_ids();
        let arch = self.get_or_create_archetype(ids);
        // OnceLock::set may race with a concurrent setter (Phase 9). If our
        // set loses, the value already stored is identical because (a)
        // component_ids() returns the same canonical-sorted slice for `B`
        // process-wide, and (b) get_or_create_archetype is idempotent on the
        // same id set within a single world. The Err return carries the
        // rejected value; we drop it and read back the winner's value.
        let cache = self.bundle_archetype_cache();
        let _ = cache[type_id.0].set(arch);
        *cache[type_id.0]
            .get()
            .expect("invariant: OnceLock populated by self or racer in cold path")
    }

    // ── Phase 10 change_tick facade (Wave A Step 2) ─────────────────────────

    /// Returns the world's current change-detection tick.
    ///
    /// Reads [`Self::change_tick`] with `Ordering::Relaxed` — sufficient
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
        loop {
            // Transient shared borrow only for the emptiness test; dropped at
            // the `;`. The twin re-reads `bytes.len()` each turn, so re-entrant
            // appends pushed during a prior `apply_via_raw_twin` are seen here.
            // SAFETY: `world_ptr` is valid + exclusive (`&mut self` at the call
            //   site); the borrow does not escape the `if`.
            if unsafe { (*world_ptr.as_ptr()).deferred_hook_queue.is_empty() } {
                break;
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
    /// ([`EntityMaster::ensure_capacity`](crate::ecs::core::entity::entity_master::EntityMaster::ensure_capacity))
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
    /// # Panics (I-NEW-4 / QV11 / W4 canonical)
    ///
    /// Panics if `D::NEEDS_CHANGE_DETECTION || F::NEEDS_CHANGE_DETECTION`
    /// is true (i.e. if `D` or `F` contains `Ref<T>`, `Mut<T>`, `Added<C>`,
    /// or `Changed<C>`). Change-detection requires `Schedule` context; use
    /// `Query<D, F>` as a SystemParam inside a system body via `Schedule`.
    /// The check is `const`-folded at monomorphisation — zero overhead on
    /// the !NCD path.
    ///
    /// [`FunctionSystem`]: crate::ecs::core::system::function_system::FunctionSystem
    pub fn query<D, F>(&mut self) -> QueryView<'_, D, F>
    where
        D: QueryData + 'static,
        F: QueryFilter + 'static,
    {
        // QV11 / I-NEW-4: change-detection guard. `if const { ... }` const-folds
        // at monomorphisation; the panic site is `#[cold] + #[inline(never)]`
        // and lives outside the hot path's I-cache on !NCD paths.
        if const { D::NEEDS_CHANGE_DETECTION || F::NEEDS_CHANGE_DETECTION } {
            query_change_detection_panic::<D, F>();
        }

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
                state_mut.update(self.archetype_master());
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
                    state_mut.update(self.archetype_master());
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
    /// Also resets the per-world bundle caches ([`Self::bundle_archetype_cache`]
    /// and [`Self::bundle_column_cache`]): both hold `ArchetypeId`s /
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
}

/// Phase 12.5 Track B I-NEW-4 / QV11 / W4 — canonical panic site for
/// `EcsMaster::query<D, F>()` when `D` or `F` carries
/// `NEEDS_CHANGE_DETECTION = true`. `#[cold] + #[inline(never)]` so the
/// site lives outside the hot path's I-cache; the wording is the
/// canonical W4 message, verbatim across plan §0 I-NEW-4, §2.2 QV11,
/// §4.3 doc-comment, and §5 implementation.
#[cold]
#[inline(never)]
fn query_change_detection_panic<D, F>() -> !
where
    D: QueryData + 'static,
    F: QueryFilter + 'static,
{
    panic!(
        "direct API EcsMaster::query<{}, {}>() does not support change-detection \
         filters (D or F has NEEDS_CHANGE_DETECTION = true); use Query<D, F> \
         inside a system body via Schedule",
        std::any::type_name::<D>(),
        std::any::type_name::<F>(),
    );
}

/// Cold-path panic helper for [`EcsMaster::resource`] / [`EcsMaster::resource_mut`].
///
/// Distinct from `params::diagnostics::missing_resource_panic` (which targets
/// the `SystemParam` `get_param` path) — the wording here points at the
/// direct-call API rather than the system runner.
#[cold]
#[inline(never)]
fn missing_resource_panic_facade<R: Resource>() -> ! {
    panic!(
        "Resource `{}` not registered. Call `EcsMaster::insert_resource::<{}>(...)` first.",
        R::debug_type_name(),
        R::debug_type_name()
    );
}

/// Cold-path panic helper for [`EcsMaster::register_component_hooks`] when the
/// release-level staleness gate finds `C` already placed in an archetype (plan
/// §6.4 / Q-A5 / W3; Phase 21 H1 — the gate is process-global across all
/// worlds). Kept off the hot method body via `#[cold] #[inline(never)]`.
#[cold]
#[inline(never)]
fn register_component_hooks_stale_panic<C: Component>() -> ! {
    panic!(
        "register_component_hooks::<{}>() called after {} already appears in a live \
         archetype of some world in this process (hooks are process-global per type, \
         so the gate is too); register hooks before the component is first used in ANY \
         world (the archetype's ArchetypeFlags were computed at construction and would \
         be stale, silently skipping the hook).",
        C::debug_type_name(),
        C::debug_type_name(),
    );
}

/// Cold-path panic helper for [`EcsMaster::register_component_hooks`] when `C`
/// already declares `#[component(...)]` derive hooks (Wave-5 soundness fix /
/// Change 3 — derive XOR runtime). Eager check at the registration call site,
/// kept off the method body via `#[cold] #[inline(never)]`.
#[cold]
#[inline(never)]
fn register_component_hooks_derive_conflict_panic<C: Component>() -> ! {
    panic!(
        "register_component_hooks::<{}>() on a type that declares #[component(...)] \
         derive hooks — use the derive OR the runtime builder, not both.",
        C::debug_type_name(),
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
//   - Direct (non-scheduler) `&mut EcsMaster` callers (`EcsMaster::create_entity`,
//     `EcsMaster::insert_resource`, etc.) inherit the borrow-checker
//     enforcement; no scheduler invariant applies because no worker is in
//     flight at the language level.
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