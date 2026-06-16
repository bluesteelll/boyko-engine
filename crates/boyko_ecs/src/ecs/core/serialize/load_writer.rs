//! Load-direction world writer (Phase S2 — the `CopyIntoWorld` core).
//!
//! Spec: `docs/SERIALIZATION-PLAN.md` §3.11 (LOAD) + §5 (W4). This module is the
//! `boyko_ecs`-side half of the loader: it owns the `pub(crate)` write primitives
//! (`Archetype::reserve_capacity`, `ComponentPool::{write_at_unchecked_initialized,
//! reserved_row_ptr, commit_units, fill_ticks, construct_at_uninitialized,
//! drop_at, pop_entity_no_drop}`, `EntityMaster::{reserve_batch, ensure_capacity,
//! register_batch}`) that the file-format parser in the `boyko_serialize` crate
//! cannot reach (they are crate-private). The parser ([`boyko_serialize::load`])
//! resolves the file into a per-archetype set of [`LoadColumn`] instructions and
//! calls [`load_archetype`]; this module performs the
//! create → reserve → write → commit → fill → register sequence the clone
//! `materialize.rs` template mirrors.
//!
//! # Why the driver lives here (crate-boundary decision, C1 0%-gate)
//!
//! Identical to why `clone/materialize.rs` lives in core: the row-write API is
//! `pub(crate)`. Exposing the driver as a single cold `pub fn` (called ONLY from
//! `boyko_serialize::load_world`) keeps every spawn / iter / schedule path
//! untouched — `load_archetype` is grep-provably off the per-frame path.
//!
//! # Scope (Phase S2)
//!
//! Non-Entity-bearing worlds only. Entity fields are loaded with their RAW saved
//! ids (the [`Wire`](super::wire::Wire) `Entity` codec); the saved→fresh remap
//! (`map_entities_fn`) is deferred to S2.5. [`LoadEntityMap`] is populated here so
//! the map is ready for that pass.
//!
//! # Soundness anchors
//!
//! * **W4 start_row == 0**: every load targets a FRESHLY-created archetype, so the
//!   whole-column blit writes `buffer_ptr_mut().add(0)` for `count * stride` bytes
//!   and never overruns into a co-located tick sub-region.
//! * **Incremental commit + rollback (mirrors `materialize.rs` `CloneRowGuard`)**:
//!   columns are committed as they complete (a blit/construct column in one
//!   `commit_units(0, n)`; a decode column row-by-row). On the FIRST decode `Err`
//!   the [`ArchetypeLoadGuard`] drops every committed row of every committed pool
//!   via `drop_at` + `pop_entity_no_drop` and leaves the fresh archetype empty
//!   (`current_index == 0`, `entity_ids` empty) — the entity batch is never
//!   registered, so `entity_master` is untouched on the rollback path.
//! * **No cached `NonNull<EcsMaster>` across a structural op**: the loader holds a
//!   raw `*mut Archetype` (slab-stable provenance) and confines every
//!   `&mut Archetype` reborrow to a tight scope, exactly as `materialize.rs` does.

use crate::ecs::core::archetype::archetype::Archetype;
use crate::ecs::core::component::component_registry::RequiredCtor;
use crate::ecs::core::ecs_master::ecs_master::EcsMaster;
use crate::ecs::core::entity::entity::Entity;
use crate::ecs::core::serialize::{DecodeError, DeserializeFn, LoadCursor, LoadEntityMap};
use crate::ecs::core::system::params::entity_counter::MAX_BATCH_HINT;
use crate::ecs::identifiers::primitives::{ComponentId, EntityId};

/// One column's load instruction, resolved by the file-format parser before the
/// world write (plan §3.11 step 4). Each variant names the destination
/// [`ComponentId`] (a column of the freshly-created archetype) and how its `n`
/// rows are materialized.
pub enum LoadColumn<'a> {
    /// `PlainOldBytes`, fingerprint OK: one `copy_nonoverlapping` of the in-file
    /// column image (`bytes.len() == n * stride`) into the fresh pool's data
    /// sub-region, then a single `commit_units(0, n)`.
    Blit {
        /// Destination column id.
        component_id: ComponentId,
        /// The in-file POB column image — exactly `n * stride` all-bits-valid
        /// bytes (the parser validated `byte_len == n * stride` and the
        /// `layout_fingerprint`).
        bytes: &'a [u8],
    },
    /// `SerializeViaFn`: loop the `n` rows calling `deserialize_fn(&mut LoadCursor,
    /// reserved_row_ptr(i))` into each uninit slot, committing row-by-row; on the
    /// FIRST `Err` the whole archetype rolls back.
    Decode {
        /// Destination column id.
        component_id: ComponentId,
        /// The per-element decoder installed for this component's runtime type.
        deserialize_fn: DeserializeFn,
        /// The in-file encoded column run (the cursor reads `n` elements out of
        /// it; a short/hostile run surfaces as a [`DecodeError`] → rollback).
        bytes: &'a [u8],
    },
    /// A column carrying no data (a ViaFn column with `byte_len == 0` / no decoder,
    /// or a skipped/`Ignore` component) that the running archetype nonetheless
    /// includes: default-construct each of the `n` rows via its capture-free
    /// `#[require]` ctor so the entity is valid (never a read of garbage).
    Construct {
        /// Destination column id.
        component_id: ComponentId,
        /// The capture-free constructor paired with `component_id` (resolved by
        /// the parser via `required_ctor_for`).
        ctor: RequiredCtor,
    },
}

/// RAII rollback guard for a partially-loaded fresh archetype (mirrors
/// `materialize.rs::CloneRowGuard`, W5).
///
/// Holds ONLY archetype-local state: the raw fresh-archetype pointer and the
/// committed `(ComponentId, row_count)` pairs in commit order. On unwind (a decode
/// `Err` triggers an explicit `rollback()`; a panic in a `deserialize_fn` triggers
/// `Drop`) it drops every committed row of every committed pool and resets each
/// pool's `len` to 0. It NEVER touches `entity_master` — the entity batch is
/// registered only AFTER full success, so on the rollback path no entity was ever
/// mapped. No world pointer is cached.
struct ArchetypeLoadGuard {
    /// Slab-stable, write-capable provenance for the fresh archetype. Never moved
    /// while the guard is live (no archetype-slab growth occurs mid-load of one
    /// archetype). `null` once disarmed.
    archetype_ptr: *mut Archetype,
    /// Committed columns in commit order: `(id, rows_committed)`. On rollback each
    /// pool's committed rows are dropped from the tail down and uncommitted.
    /// Bounded by the archetype's column count (≤ `MAX_COMPONENTS`).
    committed: Vec<(ComponentId, usize)>,
    /// The IN-PROGRESS decode column (id + rows committed so far), or `None` outside
    /// a decode loop. A `deserialize_fn` that PANICS mid-column (rather than
    /// returning `Err`) skips the explicit `note_committed`, so the `Drop` path
    /// rolls back this pending column too — without it those row-by-row-committed
    /// decoded values would survive in a pool whose archetype reports 0 entities
    /// (an inconsistent archetype + a leak). Cleared when the column completes.
    pending: Option<(ComponentId, usize)>,
    /// `true` until the caller disarms after full success.
    armed: bool,
}

impl ArchetypeLoadGuard {
    #[inline]
    fn new(archetype_ptr: *mut Archetype, column_capacity: usize) -> Self {
        Self {
            archetype_ptr,
            committed: Vec::with_capacity(column_capacity),
            pending: None,
            armed: true,
        }
    }

    /// Records that `id`'s pool has `rows` committed rows at `[0, rows)`.
    #[inline]
    fn note_committed(&mut self, id: ComponentId, rows: usize) {
        self.committed.push((id, rows));
    }

    /// Marks `id` as the in-progress decode column with `rows` rows committed so far
    /// (updated per row). `Drop` (panic path) rolls this back alongside `committed`.
    #[inline]
    fn set_pending(&mut self, id: ComponentId, rows: usize) {
        self.pending = Some((id, rows));
    }

    /// Clears the in-progress marker once a decode column completes (its full
    /// `n`-row commit is recorded via `note_committed` by the caller).
    #[inline]
    fn clear_pending(&mut self) {
        self.pending = None;
    }

    /// Disarms the guard after the archetype bookkeeping is consistent and the
    /// entity batch is about to be registered. After this, `Drop` is a no-op.
    #[inline]
    fn disarm(&mut self) {
        self.armed = false;
    }

    /// Drops every committed row of every committed pool and resets each pool's
    /// `len` to 0, leaving the fresh archetype empty. Shared by the explicit
    /// decode-`Err` path and the `Drop` (panic) path.
    fn rollback_committed(&mut self) {
        // SAFETY (W5):
        //   * `archetype_ptr` is write-capable, slab-stable, interior-mutable
        //     provenance minted under `&mut EcsMaster`, never moved while the guard
        //     is live.
        //   * Each `(id, rows)` in `committed` records a pool whose rows `[0, rows)`
        //     were committed via `commit_units` (so `rows == pool.count()` for that
        //     pool — every committed column committed the SAME row prefix). Dropping
        //     from the tail (`drop_at(len - 1)` then `pop_entity_no_drop`) runs each
        //     value's `drop_fn` exactly once and ends with the pool empty.
        //   * Single-threaded `&mut EcsMaster` is held by the calling frame, so no
        //     concurrent reader/writer exists.
        //   * `entity_master` is NOT touched: the entity batch is registered only
        //     after `disarm`, so on this path no entity was ever mapped.
        let archetype: &mut Archetype = unsafe { &mut *self.archetype_ptr };
        // The in-progress decode column (if any) is rolled back FIRST: its pool's
        // `len` is the partial commit count, distinct from the fully-committed
        // columns. (On the explicit `Err` path the caller records it into `committed`
        // and clears `pending`, so this is empty there; on the panic path it carries
        // the partial column.)
        let drop_pool_rows = |archetype: &mut Archetype, id: ComponentId, rows: usize| {
            if let Some(pool) = archetype.component_pools_mut().get_pool_mut(id) {
                for _ in 0..rows {
                    let last = pool.count() - 1;
                    // SAFETY: `last < pool.count()`; the slot holds a value committed
                    //   via `commit_units`. `drop_at` runs its `drop_fn` once;
                    //   `pop_entity_no_drop` then uncommits the now-dead slot.
                    unsafe { pool.drop_at(last) };
                    pool.pop_entity_no_drop();
                }
            }
        };
        if let Some((id, rows)) = self.pending.take() {
            drop_pool_rows(archetype, id, rows);
        }
        for &(id, rows) in &self.committed {
            drop_pool_rows(archetype, id, rows);
        }
        self.committed.clear();
        // The fresh archetype's `entity_ids` / `current_index` were never advanced
        // (that happens only after every column commits), so the archetype is now
        // exactly as `create_archetype` left it.
    }
}

impl Drop for ArchetypeLoadGuard {
    fn drop(&mut self) {
        if self.armed {
            self.rollback_committed();
        }
    }
}

/// Loads ONE archetype's `n` entities from the resolved `columns` into the world
/// (plan §3.11 step 4). Always creates a FRESH archetype (W4 — `start_row == 0`),
/// reserves `n` rows, writes every column, commits + stamps ticks, allocates and
/// registers a fresh entity batch, and records the saved→fresh mapping into `map`.
///
/// `ids` is the canonical (sorted) component-id set of the fresh archetype.
/// `columns` carries one [`LoadColumn`] per id in `ids` that this archetype stores
/// (the parser excludes a no-data, no-ctor column from BOTH `ids` and `columns`, so
/// the archetype never contains an uninitializable pool — that component is counted
/// `types_defaulted` by the caller). `saved_entity_ids` are the file's per-row saved
/// `EntityId`s (used only to populate `map`; the fresh ids are independent).
///
/// On success returns the number of entities loaded (`n`). On a malformed
/// `Decode` column it rolls the fresh archetype back to empty and returns the
/// [`DecodeError`] — the world is left consistent (the archetype exists but holds
/// no rows, `entity_master` untouched).
///
/// # Panics
///
/// `panic`s only on an internal invariant violation (a bug): an archetype id the
/// master just created not resolving, or a column id with no pool in the fresh
/// archetype. Both are `expect`-guarded.
pub fn load_archetype(
    world: &mut EcsMaster,
    ids: &[ComponentId],
    columns: &[LoadColumn<'_>],
    saved_entity_ids: &[EntityId],
    map: &mut LoadEntityMap,
) -> Result<usize, DecodeError> {
    let n = saved_entity_ids.len();
    let current_tick = world.current_tick();

    // ── Create the fresh archetype (W4) + mint slab-stable provenance ──────────
    let archetype_id = world.create_archetype(ids);
    let archetype_ptr = world
        .archetype_master_mut()
        .archetype_ptr_for(archetype_id)
        .expect("invariant: archetype just created is live");

    if n == 0 {
        // An empty archetype: nothing to write, no entities to allocate. The
        // archetype still exists (round-trip-inspectable), matching the saver
        // (which emits a block per live archetype; an empty one cannot occur in a
        // v1 save, but the loader stays total).
        return Ok(0);
    }

    // ── Reserve pool capacity for n rows (the fresh archetype starts at len 0) ──
    {
        // SAFETY: `archetype_ptr` is write-capable slab provenance under
        //   `&mut EcsMaster`; this reborrow is confined to the reserve call.
        let archetype: &mut Archetype = unsafe { &mut *archetype_ptr };
        archetype.reserve_capacity(n).expect(
            "load: fresh archetype pool reserve ceiling (rows) exhausted — committed \
             capacity grows on demand (Phase X.I)",
        );
    }

    let mut guard = ArchetypeLoadGuard::new(archetype_ptr, columns.len());

    // ── Per column: blit / decode / construct, committing as each completes ────
    for col in columns {
        match col {
            LoadColumn::Blit { component_id, bytes } => {
                // SAFETY (W4 / W7):
                //   * `bytes.len() == n * stride` (parser-validated); the fresh
                //     pool reserved `n` rows so its data sub-region holds exactly
                //     `n * stride` bytes starting at `buffer_ptr_mut()`.
                //   * `start_row == 0` (W4), so the blit writes
                //     `buffer_ptr_mut().add(0)` for `n * stride` bytes and never
                //     overruns into the co-located tick sub-region.
                //   * Source (file bytes) and dest (the pool reservation) are
                //     disjoint allocations; every byte of a POB type is
                //     all-bits-valid (C3) so the copied image is a valid column.
                //   * `&mut Archetype` / `&mut pool` confined to this block.
                let archetype: &mut Archetype = unsafe { &mut *archetype_ptr };
                let pool = archetype
                    .component_pools_mut()
                    .get_pool_mut(*component_id)
                    .expect("invariant: a Blit column id has a pool in the fresh archetype");
                let stride = pool.component_layout().size();
                debug_assert_eq!(
                    bytes.len(),
                    n.checked_mul(stride).expect("n * stride overflow"),
                    "load Blit: bytes.len() != n * stride"
                );
                if stride != 0 {
                    // SAFETY: `dst` is the fresh reserved pool region of exactly
                    //   `n * stride` bytes; `src` is the in-file POB image, all-bits
                    //   valid; disjoint allocations.
                    unsafe {
                        std::ptr::copy_nonoverlapping(
                            bytes.as_ptr(),
                            pool.buffer_ptr_mut(),
                            bytes.len(),
                        );
                    }
                }
                pool.commit_units(0, n);
                pool.fill_ticks(0, n, current_tick);
                guard.note_committed(*component_id, n);
            }

            LoadColumn::Decode {
                component_id,
                deserialize_fn,
                bytes,
            } => {
                // One cursor over the whole encoded column run; the per-element
                // decoder advances it. A short/hostile run surfaces as a
                // `DecodeError` on some row → rollback. Mark this column pending so a
                // PANIC in `deserialize_fn` (not just an `Err`) also rolls back the
                // rows committed so far (the Drop path).
                let mut cursor = LoadCursor::new(bytes);
                guard.set_pending(*component_id, 0);
                for row in 0..n {
                    // Derive the dst `*mut u8` in a TIGHT scope, then drop the
                    // `&mut Archetype` BEFORE calling the (panic-prone, fallible)
                    // user `deserialize_fn`. At the call NO `&mut Archetype` is live
                    // — only `dst: *mut u8` (no TB protector across the call) — so on
                    // a panic the guard's `Drop` is the sole `&mut Archetype`
                    // accessor (the F2 / W5 anchor).
                    // SAFETY: `row < n <= committed_rows` after `reserve_capacity`;
                    //   the slot is reserved-uninit; the returned `*mut u8` carries
                    //   the pool's reservation provenance, aligned for this type.
                    let dst_ptr = unsafe {
                        let archetype: &mut Archetype = &mut *archetype_ptr;
                        let pool = archetype
                            .component_pools_mut()
                            .get_pool_mut(*component_id)
                            .expect("invariant: a Decode column id has a pool");
                        pool.reserved_row_ptr(row)
                    };

                    // SAFETY (registry `DeserializeFn` contract):
                    //   * `dst_ptr` is writable, uninitialized space of
                    //     `>= size_of::<C>()` bytes aligned to `align_of::<C>()`.
                    //   * On `Ok` the slot holds an initialized `C` written exactly
                    //     once (no drop of the uninit prior contents); on `Err` (or
                    //     a panic) the slot is left uninit and the rollback below must
                    //     NOT count this row as committed.
                    //   * NO `&mut Archetype` is live across this call (dropped above).
                    let decoded = unsafe { deserialize_fn(&mut cursor, dst_ptr) };
                    if let Err(e) = decoded {
                        // Rows `[0, row)` of THIS column are already committed (tracked
                        // in `pending`); `row` is uninit (the failed decode left it
                        // untouched). Roll the whole archetype back to empty (the
                        // pending partial column + every prior full column) and bail.
                        guard.rollback_committed();
                        guard.disarm();
                        return Err(e);
                    }

                    // Commit + stamp this row immediately so the rollback path can
                    // drop it via `drop_at` (which needs a committed `len`). A
                    // confined `&mut Archetype` reborrow; neither op can panic.
                    // SAFETY: `row == pool.count()` (rows extend the tail in order);
                    //   `commit_units(row, 1)` makes the just-written slot live;
                    //   `fill_ticks` then stamps it.
                    unsafe {
                        let archetype: &mut Archetype = &mut *archetype_ptr;
                        let pool = archetype
                            .component_pools_mut()
                            .get_pool_mut(*component_id)
                            .expect("invariant: a Decode column id has a pool");
                        pool.commit_units(row, 1);
                        pool.fill_ticks(row, 1, current_tick);
                    }
                    // Advance the pending count to include the row just committed.
                    guard.set_pending(*component_id, row + 1);
                }
                // Column complete: promote the pending partial into a full committed
                // record and clear the in-progress marker.
                guard.clear_pending();
                guard.note_committed(*component_id, n);
            }

            LoadColumn::Construct { component_id, ctor } => {
                // Default-construct each row via the capture-free ctor, then commit
                // the whole column. The ctor (a derive-generated free fn) does not
                // panic by design; the `&mut Archetype` reborrow is confined.
                // SAFETY: `row < n <= committed_rows`; each slot is reserved-uninit;
                //   `ctor` writes one value of the pool's registered type (registry-
                //   paired) without dropping the uninit dst. `&mut` confined here.
                let archetype: &mut Archetype = unsafe { &mut *archetype_ptr };
                let pool = archetype
                    .component_pools_mut()
                    .get_pool_mut(*component_id)
                    .expect("invariant: a Construct column id has a pool");
                for row in 0..n {
                    // SAFETY: as documented on the block above; `row < committed_rows`.
                    unsafe { pool.construct_at_uninitialized(row, *ctor) };
                }
                pool.commit_units(0, n);
                pool.fill_ticks(0, n, current_tick);
                guard.note_committed(*component_id, n);
            }
        }
    }

    // ── All columns committed: allocate + register the fresh entity batch ──────
    // Reserve `n` fresh ids (NOT the saved ids — the saved→fresh remap is recorded
    // into `map` for the S2.5 pass). `reserve_batch` caps a single call at
    // `MAX_BATCH_HINT`, so a larger archetype is reserved in chained calls; the
    // counter is monotonic, so the chained reservations form ONE contiguous range
    // (the second call's `start` equals the first call's `end`). `ensure_capacity`
    // then grows the fast store so the single `register_batch` is in bounds.
    let start_id = reserve_contiguous_ids(world, n);
    world
        .entity_master_mut()
        .ensure_capacity(start_id.checked_add(n).expect("start_id + n overflow"));

    // ── Archetype bookkeeping: push the fresh entity ids, advance the row head ─
    // SAFETY: `archetype_ptr` is write-capable slab provenance; `&mut` confined;
    //   neither op can panic.
    unsafe {
        let archetype: &mut Archetype = &mut *archetype_ptr;
        archetype
            .entity_ids
            .extend((start_id..start_id + n).map(EntityId));
        archetype.current_index = n;
    }

    // The archetype + pools are now fully consistent; disarm before touching the
    // entity master (W5 — past this point a panic would no longer roll back the
    // archetype, but registration is infallible).
    guard.disarm();

    world.entity_master.register_batch(
        EntityId(start_id),
        archetype_ptr,
        0,
        n,
    );

    // ── Record the saved→fresh mapping for the S2.5 remap pass ─────────────────
    for (i, saved) in saved_entity_ids.iter().enumerate() {
        let fresh = Entity::new(EntityId(start_id + i), 0);
        map.insert(saved.0, fresh);
    }

    Ok(n)
}

/// Reserves `n` contiguous fresh entity ids, chaining `EntityMaster::reserve_batch`
/// past its per-call `MAX_BATCH_HINT` cap. Returns the first id of the range.
///
/// The world counter is monotonic (`fetch_add`), so back-to-back reservations
/// yield a contiguous `[start, start + n)` window: the first call returns
/// `start..start+k`, the next `start+k..start+k+k'`, etc. — the loader registers
/// the whole window as one `register_batch`. `n > 0` is a precondition (the caller
/// early-returns on an empty archetype).
fn reserve_contiguous_ids(world: &mut EcsMaster, n: usize) -> usize {
    debug_assert!(n > 0, "reserve_contiguous_ids: n must be > 0");
    let first = world
        .entity_master_mut()
        .reserve_batch(n.min(MAX_BATCH_HINT))
        .expect("invariant: n.min(MAX_BATCH_HINT) <= MAX_BATCH_HINT");
    let start = first.start;
    let mut got = first.len();
    while got < n {
        let chunk = (n - got).min(MAX_BATCH_HINT);
        let next = world
            .entity_master_mut()
            .reserve_batch(chunk)
            .expect("invariant: chunk <= MAX_BATCH_HINT");
        debug_assert_eq!(
            next.start,
            start + got,
            "reserve_contiguous_ids: counter is not monotonic-contiguous"
        );
        got += next.len();
    }
    start
}
