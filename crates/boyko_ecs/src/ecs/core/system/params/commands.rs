//! `Commands<'s>` — deferred world-mutation buffer SystemParam.
//!
//! Phase 8d Step 7 / plan §13 (extended by Phase 11 §5.6). `Commands<'s>`
//! borrows a per-system [`CommandQueue`] (the SystemParam's `State`) and
//! carries an [`EntityCounter<'s>`] projecting the world's atomic
//! `next_entity_id` (Round 3 C-N1, plan §5.5). It exposes the user-facing
//! enqueue surface: `spawn(bundle).insert(extra).id()`, `entity(id)`,
//! `despawn(id)`, `add(cmd)`. The queue is flushed via
//! [`SystemParam::apply`] after the system body returns (invariant
//! **APP3** — drain order is deterministic).
//!
//! # Why no declared access (SP1)
//!
//! `Commands` declares NO component / resource reads or writes during
//! [`init_access`]. Buffering is a pure append-only stack operation against
//! `&'s mut CommandQueue`; `EntityCounter::reserve_entity` is a single
//! atomic RMW (conflict-free per EM6 + EVT1 precedent); the flush in
//! [`apply`] uses `&mut EcsMaster` exclusively (CQ7), so no aliasing arises
//! with sibling params during the system body.
//!
//! # Per-invocation lifecycle (W-N3, plan §8.7)
//!
//! Phase 8c `IntoSystem::FunctionSystem` calls [`SystemParam::get_param`]
//! **once per system invocation each frame**. The returned `Commands<'s>`
//! value is dropped at the end of the system body — the contained
//! [`EntityCounter<'s>`]'s pointer never outlives `'w`. Cross-frame
//! staleness is impossible: each frame re-mints the counter from a fresh
//! `UnsafeEcsCell<'w>` reborrow.
//!
//! [`apply`]: SystemParam::apply
//! [`init_access`]: SystemParam::init_access

// `Commands` is wired into `FunctionSystem` by Phase 8c Step 4
// (`EcsMaster::run_system`). The lib build does not exercise the path
// until Phase 8.5 Step 7 lands integration tests in
// `crates/boyko_ecs/tests/derive_bundle_smoke.rs`; until then the
// user-facing API is dead-code from the library's standpoint. Mirrors the
// existing suppression on Step 6's `command_queue.rs`.
#![allow(dead_code)]

use crate::ecs::core::bundle::{Bundle, EmptyBundle};
use crate::ecs::core::commands::Command;
use crate::ecs::core::commands::command_queue::CommandQueue;
use crate::ecs::core::commands::despawn_command::DespawnCommand;
use crate::ecs::core::commands::send_event_command::SendEventCommand;
use crate::ecs::core::commands::spawn_at_command::SpawnAtCommand;
use crate::ecs::core::commands::spawn_batch_command::SpawnBatchCommand;
use crate::ecs::core::commands::spawn_batch_iter::SpawnBatchIter;
use crate::ecs::core::ecs_master::ecs_master::EcsMaster;
use crate::ecs::core::entity::entity::Entity;
use crate::ecs::core::events::event::Event;
use crate::ecs::core::system::filtered_access_set::FilteredAccessSet;
use crate::ecs::core::system::params::entity_commands::EntityCommands;
use crate::ecs::core::system::params::entity_counter::EntityCounter;
use crate::ecs::core::system::system_meta::SystemMeta;
use crate::ecs::core::system::system_param::SystemParam;
use crate::ecs::core::system::unsafe_ecs_cell::UnsafeEcsCell;
use crate::ecs::error::EcsResult;
use crate::ecs::identifiers::primitives::EntityId;

/// Deferred world-mutation buffer borrowed for one system invocation.
///
/// Constructed by the [`SystemParam`] machinery — user code does not
/// instantiate it directly. The lifetime `'s` is the system's state scope
/// (the [`CommandQueue`] lives in the system's cached state slot).
///
/// # Enqueue API
///
/// * [`spawn`](Self::spawn) — pre-allocate an [`Entity`] via the atomic
///   counter and enqueue a [`SpawnAtCommand<B>`]. Returns
///   [`EntityCommands<'_, 's>`] for `.insert(...).insert(...).id()`
///   chaining. The destination archetype id is resolved lazily on apply
///   via `B::cached_archetype_id` (Phase 8.5 SBC4).
/// * [`entity`](Self::entity) — return a [`EntityCommands<'_, 's>`] handle
///   for an existing entity (per-entity chaining over an already-live id).
/// * [`despawn`](Self::despawn) — enqueue a [`DespawnCommand`] for an
///   existing entity (convenience wrapper for `entity(id).despawn()`).
/// * [`reserve_entity`](Self::reserve_entity) — atomically reserve a fresh
///   ID without enqueueing anything; used by power users assembling
///   spawn-and-relate plans manually.
/// * [`add`](Self::add) — enqueue an arbitrary user-defined [`Command`].
/// * [`send_event`](Self::send_event) — enqueue a deferred
///   `EventDispatcher::send`.
///
/// # Layout (plan §11.7)
///
/// 16 B total: `(&'s mut CommandQueue, EntityCounter<'s>)`. One cache line.
///
/// # `!Send + !Sync`
///
/// `Commands<'s>` carries `&'s mut CommandQueue`, which is `!Sync` for the
/// lifetime `'s` (CQ-SEND2). The owning [`CommandQueue`] itself is `Send`
/// (CQ-SEND1). The contained [`EntityCounter<'s>`] is `Send + Sync` on its
/// own — but `Commands<'s>` inherits the `!Sync` from the queue field.
pub struct Commands<'s> {
    /// Exclusive borrow of the system's per-call queue. The system's
    /// cached `State` (a `CommandQueue`) is the storage backing this
    /// borrow; the reborrow is established by [`SystemParam::get_param`].
    pub(crate) queue: &'s mut CommandQueue,

    /// Phase 11 (Round 3 C-N1, EM6): worker-safe projection of the world's
    /// atomic `next_entity_id`. The newtype encapsulates a raw pointer
    /// whose destination type is `AtomicUsize` — no compile-time path
    /// leads to any other `EntityMaster` field through `Commands`.
    pub(crate) entity_counter: EntityCounter<'s>,
}

// Plan §11.7: `Commands<'s>` is exactly 16 B (one cache line). Compile-time
// guard so a future field addition is caught at the assertion site rather
// than as a perf surprise.
// It holds a `&mut CommandQueue` plus a pointer-width `EntityCounter`, so the
// 16-byte size encodes the 64-bit ABI; gated to 64-bit (the engine's supported
// platform) — see CLAUDE.md target platform.
#[cfg(target_pointer_width = "64")]
const _: () = assert!(core::mem::size_of::<Commands<'static>>() == 16);

impl<'s> Commands<'s> {
    /// Enqueues a user-defined [`Command`] for deferred apply.
    ///
    /// Cost: `~20 ns` per push (D1) — two `write_unaligned` calls plus a
    /// possible `Vec::reserve` growth (amortised).
    #[inline]
    pub fn add<C: Command>(&mut self, cmd: C) {
        self.queue.push(cmd);
    }

    /// Pre-allocates an [`Entity`] via the world's atomic counter and
    /// enqueues a [`SpawnAtCommand<B>`] (Phase 11 §5.6 / plan Q9). Returns
    /// an [`EntityCommands<'_, 's>`] handle for chaining.
    ///
    /// The destination [`ArchetypeId`] is resolved on the apply path via
    /// [`Bundle::cached_archetype_id`] — there is no per-callsite
    /// pre-resolution and no `archetype_id` argument. Hot path on apply:
    /// `~3 ns` cache hit (SBC4); cold path on first spawn of `B` in this
    /// world: `~1 µs` via `ArchetypeMaster::get_or_create_archetype`.
    ///
    /// # Cost
    ///
    /// `~28 ns` single-thread / up to `~78 ns` under 8-worker contention
    /// (plan §10.1 / §10.5): `EntityCounter::reserve_entity` (~10 ns) +
    /// `CommandQueue::push` (~18 ns) + EntityCommands construction (free).
    ///
    /// # Example
    ///
    /// ```ignore
    /// use boyko_macros::Bundle;
    ///
    /// #[derive(Bundle)]
    /// struct PlayerBundle { pos: Position, vel: Velocity }
    ///
    /// let id = commands.spawn(PlayerBundle {
    ///     pos: Position(0),
    ///     vel: Velocity(1),
    /// })
    /// .insert(HealthBundle { hp: 100 })
    /// .id();
    /// ```
    ///
    /// [`ArchetypeId`]: crate::ecs::identifiers::primitives::ArchetypeId
    /// [`Bundle::cached_archetype_id`]: crate::ecs::core::bundle::Bundle::cached_archetype_id
    #[inline]
    pub fn spawn<B: Bundle>(&mut self, bundle: B) -> EntityCommands<'_, 's> {
        let entity = self.entity_counter.reserve_entity();
        self.queue.push(SpawnAtCommand { entity, bundle });
        EntityCommands::new(entity, self)
    }

    /// Pre-allocates an [`Entity`] and enqueues a spawn with **zero
    /// components** (Phase 22 D5). Returns an [`EntityCommands<'_, 's>`]
    /// handle for chaining (`.insert(...)`, `.id()`, ...).
    ///
    /// The entity lands in the empty archetype
    /// (`get_or_create_archetype(&[])`), created lazily on the first empty
    /// spawn per world. Resolution goes through the ordinary static bundle
    /// cache ([`EmptyBundle`] owns its own `BundleTypeId`), so the warm path
    /// costs the same sub-ns cached lookup as any bundle spawn (SBC4).
    ///
    /// Tag-only and component-less entities are first-class: the result is
    /// invisible to every component query (the empty signature matches only
    /// zero-required-component queries) until components or tags are added.
    #[inline]
    pub fn spawn_empty(&mut self) -> EntityCommands<'_, 's> {
        self.spawn(EmptyBundle)
    }

    /// Returns an [`EntityCommands<'_, 's>`] handle for an existing entity
    /// (Phase 11 §5.4).
    ///
    /// No validation at the call site — `entity` may be stale (debug_assert
    /// fires inside `InsertCommand::apply` / `RemoveCommand::apply`; release
    /// silently no-ops per EC8).
    #[inline]
    pub fn entity(&mut self, entity: Entity) -> EntityCommands<'_, 's> {
        EntityCommands::new(entity, self)
    }

    /// Reserves a fresh [`Entity`] without enqueueing any command
    /// (Phase 11 §5.6 escape hatch).
    ///
    /// Use this when you need an Entity ID to thread through user code
    /// before deciding what to spawn (e.g. constructing a relation between
    /// two not-yet-spawned entities). The caller is responsible for
    /// eventually enqueueing a [`SpawnAtCommand`] for this id — if the
    /// queue drops without an apply for this id, the id leaks (one ID per
    /// missed apply; counter marches forward monotonically per EM4).
    #[inline]
    pub fn reserve_entity(&self) -> Entity {
        self.entity_counter.reserve_entity()
    }

    /// Convenience wrapper for `entity(id).despawn()` (Phase 11 §5.4).
    ///
    /// Equivalent to enqueueing a [`DespawnCommand`] directly. Cost
    /// ~18 ns (single `CommandQueue::push`).
    #[inline]
    pub fn despawn(&mut self, entity: Entity) {
        self.queue.push(DespawnCommand { entity });
    }

    /// Adds `child` as a child of `parent` by inserting
    /// [`ChildOf`](crate::ecs::core::hierarchy::ChildOf) on the child (Phase 19).
    ///
    /// Equivalent to `commands.entity(parent).add_child(child)`. The whole
    /// relationship is driven by `ChildOf` insertion — user code never writes
    /// `Children` directly.
    #[inline]
    pub fn add_child(&mut self, parent: Entity, child: Entity) {
        self.queue.push(crate::ecs::core::commands::insert_command::InsertCommand {
            entity: child,
            bundle: crate::ecs::core::hierarchy::ChildOf(parent),
        });
    }

    /// Phase 12.5 Opt-A2 (§5.2): enqueues a [`SpawnBatchCommand<B, I>`]
    /// covering `iter.len()` entities sharing bundle type `B`.
    ///
    /// Returns a [`SpawnBatchIter<'_, 's, B>`] yielding the reserved
    /// `Entity` IDs. Entities are not yet alive at the return point —
    /// they become observable after the next `CommandQueue::apply`.
    ///
    /// # Errors
    ///
    /// Returns `Err(EcsError::SpawnBatchExceedsCapacity)` if
    /// `iter.len() > MAX_BATCH_HINT` (8 192). The bundle iterator is
    /// dropped on `Err`; the counter is NOT advanced.
    ///
    /// Larger requests must be chunked by the caller:
    ///
    /// ```ignore
    /// for chunk in (0..70_000).step_by(MAX_BATCH_HINT - 1) {
    ///     let end = (chunk + MAX_BATCH_HINT - 1).min(70_000);
    ///     commands.spawn_batch((chunk..end).map(|i| MyBundle::new(i)))
    ///         .expect("chunk size ≤ MAX_BATCH_HINT")
    ///         .for_each(drop);
    /// }
    /// ```
    ///
    /// # Drop semantics (SBO8b — I-N2)
    ///
    /// Dropping the returned `SpawnBatchIter` without iterating does NOT
    /// cancel the spawn. The `SpawnBatchCommand` is already enqueued; the
    /// entities are spawned at the next apply regardless. Drop simply
    /// discards the unread Entity IDs (counter has already advanced).
    ///
    /// # Panic safety (SBO9)
    ///
    /// If the bundle iterator panics on row `i`, rows `[0..i)` survive;
    /// rows `[i..n)` are not spawned and their reserved IDs leak.
    /// `ManuallyDrop` (B4) suppresses double-drop.
    ///
    /// # Aggregate-worker overshoot (SBO17b — I-N1)
    ///
    /// If multiple workers near steady-state simultaneously call
    /// `spawn_batch(MAX_BATCH_HINT)`, the per-world counter may advance
    /// past the pre-sized fast-store. Apply will hard-panic with
    /// `WorldEntityCapacityExceeded` — observable failure, not silent UB.
    #[inline]
    pub fn spawn_batch<B, I>(
        &mut self,
        iter: I,
    ) -> EcsResult<SpawnBatchIter<'_, 's, B>>
    where
        B: Bundle + Send + Sync,
        I: IntoIterator<Item = B>,
        I::IntoIter: ExactSizeIterator + Send + Sync + Unpin + 'static,
    {
        let iter = iter.into_iter();
        let n = iter.len();
        // SBO17 cap-check + atomic reserve in one routine.
        let range = self.entity_counter.reserve_batch(n)?;
        let start_entity = Entity::new(EntityId(range.start), 0);
        self.queue.push(SpawnBatchCommand::<B, I::IntoIter> {
            start_entity,
            count: n as u32,
            _pad: 0,
            iter,
        });
        Ok(SpawnBatchIter::new(range))
    }

    /// Enqueues a [`SendEventCommand<E>`] that forwards `event` to
    /// [`EventDispatcher::send_event`] at apply time (Phase 9 EVT2).
    ///
    /// Because the apply path always runs on the dispatcher thread under
    /// `&mut EcsMaster`, the event lands on lane `worker_count` (the
    /// dispatcher's reserved lane — plan §2.8). Workers that need to send
    /// events directly (without going through the queue) should call
    /// [`EcsMaster::events`]`.send_event::<E>(event)` from inside the
    /// system body; the TLS routing then targets the worker's own lane.
    ///
    /// Cost: `~18 ns` — one [`CommandQueue::push`] (two `write_unaligned`
    /// calls + amortised arena growth). The actual `EventDispatcher::send`
    /// runs on the apply path.
    ///
    /// # Errors
    ///
    /// This call is infallible at enqueue time. The inner
    /// [`EventDispatcher::send_event`] result is dropped on apply (the
    /// `Command::apply` driver has no error channel). If you must surface
    /// `EventNotRegistered` / `EventBufferFull` synchronously, call
    /// `world.events().send_event::<E>(event)` directly from an exclusive
    /// system that receives `&mut EcsMaster`.
    ///
    /// [`EventDispatcher::send_event`]: crate::ecs::core::events::event_dispatcher::EventDispatcher::send_event
    /// [`EcsMaster::events`]: crate::ecs::core::ecs_master::ecs_master::EcsMaster::events
    /// [`CommandQueue::push`]: crate::ecs::core::commands::command_queue::CommandQueue::push
    #[inline]
    pub fn send_event<E: Event>(&mut self, event: E) {
        self.queue.push(SendEventCommand { event });
    }
}

// SAFETY (SP1, SP2, SP4 — Phase 11 §5.6 augmented):
//   - SP1: `init_access` declares NO reads / writes. `Commands` is a pure
//     append-only buffer; the contained `EntityCounter` access is
//     conflict-free (EM6 + EVT1 precedent — only an `AtomicUsize` is
//     reachable through the carried pointer type, and atomic RMW from
//     `&self` is data-race-free).
//   - SP2: per Phase 8c IntoSystem, `get_param` runs PER SYSTEM INVOCATION
//     each frame (W-N3 / plan §8.7). The `EntityCounter`'s pointer is
//     re-minted fresh every call; `Commands<'s>::Item<'w, 's>` is dropped
//     at the end of the system body, so the pointer never outlives `'w`.
//   - SP4: `init_state` constructs a fresh `CommandQueue` — no world
//     mutation, no archetype / resource registry change.
unsafe impl SystemParam for Commands<'_> {
    type State = CommandQueue;
    type Item<'w, 's> = Commands<'s>;

    #[inline]
    fn init_state(_world: &mut EcsMaster, _system_meta: &mut SystemMeta) -> Self::State {
        CommandQueue::new()
    }

    #[inline]
    fn init_access(
        _state: &Self::State,
        _system_meta: &mut SystemMeta,
        _access_set: &mut FilteredAccessSet,
        _world: &mut EcsMaster,
    ) {
        // SP1: `Commands` declares NO component / resource access. The
        // queue is a per-system append-only buffer; the deferred apply
        // (via `Self::apply`) holds `&mut EcsMaster` exclusively (CQ7).
        // The `EntityCounter` channel is conflict-free per EM6.
    }

    #[inline]
    unsafe fn get_param<'w, 's>(
        state: &'s mut Self::State,
        _system_meta: &SystemMeta,
        world: UnsafeEcsCell<'w>,
    ) -> Self::Item<'w, 's> {
        // SAFETY (SP2 / APP2 / EM5 / EM6 / U_C2 / W-N3):
        //   - `state: &'s mut Self::State` is exclusive for `'s` by the
        //     trait contract (the system holds the only reference to its
        //     own cached state slot during a single `run_unsafe` call).
        //   - `world.entity_counter()` mints a fresh `EntityCounter<'_>`
        //     whose internal pointer is valid for `'w`. We re-tag the
        //     lifetime to `'s` via PhantomData; sound because `'w >= 's`
        //     per the Phase 8c IntoSystem contract (plan §8.7 —
        //     `get_param` runs once per system invocation; `'s` never
        //     outlives `'w`).
        //   - The `EntityCounter`'s contract (plan §5.5) restricts
        //     reachable state to the atomic counter only — EM6 is
        //     type-enforced through the destination pointer type.
        let entity_counter = unsafe { world.entity_counter::<'s>() };
        Commands { queue: state, entity_counter }
    }

    /// Flushes the queued commands against `world` (APP3).
    ///
    /// Called by the System's outer `apply` driver after the body returns.
    /// Panic recovery is handled inside [`CommandQueue::apply`] (C5 + W3'
    /// semantics — the panicker is skipped, survivors re-absorbed for the
    /// next apply).
    #[inline]
    fn apply(state: &mut Self::State, _system_meta: &SystemMeta, world: &mut EcsMaster) {
        state.apply(world);
    }
}

// Phase 8.5 Step 5: the Phase 8d in-file smoke test
// `commands_spawn_then_apply_creates_entity` and its companion
// `commands_is_system_param` were removed because they exercised the old
// two-arg `commands.spawn(archetype_id, (A, B))` surface plus an ad-hoc
// `impl Bundle for (A, B)` tuple impl that Step 2 deleted. Step 7 of
// Phase 8.5 lands fresh smoke tests in
// `crates/boyko_ecs/tests/derive_bundle_smoke.rs` covering the new
// `commands.spawn(MyBundle { ... })` surface end-to-end via the
// `#[derive(Bundle)]` derive macro (Step 4).
