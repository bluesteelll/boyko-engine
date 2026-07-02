//! Load-direction world writer (Phase S2 — the `CopyIntoWorld` core).
//!
//! Spec: `docs/SERIALIZATION-PLAN.md` §3.11 (LOAD) + §5 (W4). This module is the
//! `boyko_ecs`-side half of the loader: it owns the `pub(crate)` write primitives
//! (`Archetype::reserve_capacity`, `ComponentPool::{write_at_unchecked_initialized,
//! reserved_row_ptr, commit_units, fill_ticks, construct_at_uninitialized,
//! drop_at, pop_entity_no_drop}`, `EntityMaster::{reserve_batch, ensure_capacity,
//! register_batch}`) that the file-format parser in the `boyko_serialize` crate
//! cannot reach (they are crate-private). The parser (`boyko_serialize::load`)
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
//! * **W4 append base (`start_row`)**: a load targets a freshly-created archetype
//!   (`start_row == 0`) OR APPENDS to one that a prior block dedup'd onto (when two
//!   saved blocks collapse to the same id set after `Ignore` columns are dropped —
//!   e.g. a parent `{Tag, ChildOf, Children}` collapses onto a child `{Tag,
//!   ChildOf}` because `Children` is `Ignore`). `start_row` is the archetype's
//!   current row count; `reserve_capacity` grows additively (`count + n`) and every
//!   write/commit is offset by `start_row`, so the whole-column blit writes
//!   `buffer_ptr_mut().add(start_row * stride)` for `n * stride` bytes into the
//!   freshly-reserved tail and never overruns into a co-located tick sub-region nor
//!   overwrites a pre-existing row.
//! * **Incremental commit + rollback (mirrors `materialize.rs` `CloneRowGuard`)**:
//!   columns are committed as they complete (a blit/construct column in one
//!   `commit_units(0, n)`; a decode column row-by-row). On the FIRST decode `Err`
//!   the `ArchetypeLoadGuard` drops every committed row of every committed pool
//!   via `drop_at` + `pop_entity_no_drop` and leaves the fresh archetype empty
//!   (`current_index == 0`, `entity_ids` empty) — the entity batch is never
//!   registered, so `entity_master` is untouched on the rollback path.
//! * **No cached `NonNull<EcsMaster>` across a structural op**: the loader holds a
//!   raw `*mut Archetype` (slab-stable provenance) and confines every
//!   `&mut Archetype` reborrow to a tight scope, exactly as `materialize.rs` does.

use crate::ecs::core::archetype::archetype::Archetype;
use crate::ecs::core::component::component_registry::{self, LoadMapEntitiesFn, RequiredCtor};
use crate::ecs::core::ecs_master::ecs_master::EcsMaster;
use crate::ecs::core::entity::entity::Entity;
use crate::ecs::core::serialize::{DecodeError, DeserializeFn, LoadCursor, LoadEntityMap};
use crate::ecs::core::system::params::entity_counter::MAX_BATCH_HINT;
use crate::ecs::identifiers::primitives::{ArchetypeId, ComponentId, EntityId};

/// A failure surfaced by the load-direction WRITER ([`load_archetype`]).
///
/// The writer ingests file-derived row counts and per-element decoder output, both
/// of which can be hostile (C3). Two distinct failure classes can occur:
///
/// * **`Decode`** — a per-element `deserialize_fn` rejected a malformed/truncated
///   column run; the partially-loaded archetype was rolled back to empty.
/// * **`CapacityExceeded`** — the file's archetype row count (`n`, possibly summed
///   across dedup-collapsed blocks appending onto one running archetype) exceeds a
///   hosted pool's reserve ceiling. This was previously an `.expect()` PANIC at the
///   `reserve_capacity` call; it is now a LOUD, release-level `Err` (C2) because the
///   per-block load-side guard cannot see the ADDITIVE pool `len` when two file
///   blocks collapse onto the same archetype (`e1 <= ceiling`, `e2 <= ceiling`, but
///   `e1 + e2 > ceiling`). No row is written before the reserve, so the world is
///   untouched on this path.
///
/// COLD — produced only on the `boyko_serialize::load_world` path (the C1 0%-gate;
/// never a per-frame allocation/iteration site).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[non_exhaustive]
pub enum LoadWriteError {
    /// A per-element `deserialize_fn` (or the entity-remap pass) rejected the
    /// stream. Carries the underlying [`DecodeError`]; the archetype was rolled
    /// back to empty.
    Decode(DecodeError),
    /// The file declares more rows for this archetype than a hosted
    /// [`ComponentPool`](crate::ecs::memory::component_pool::ComponentPool) can
    /// hold (its reserve ceiling). Carries the offending column's [`ComponentId`]
    /// and the rejected additive row request (`current_len + n`).
    CapacityExceeded {
        /// The column whose pool ceiling was exceeded.
        component: ComponentId,
        /// The additive row count the reserve would have grown the pool to.
        requested: usize,
    },
}

impl From<DecodeError> for LoadWriteError {
    #[inline]
    fn from(e: DecodeError) -> Self {
        LoadWriteError::Decode(e)
    }
}

/// The destination [`ComponentId`] of a load-column instruction (every variant
/// names exactly one). Used to identify the offending column on a reserve failure.
#[inline]
fn load_column_id(col: &LoadColumn<'_>) -> ComponentId {
    match col {
        LoadColumn::Blit { component_id, .. }
        | LoadColumn::Decode { component_id, .. }
        | LoadColumn::Construct { component_id, .. } => *component_id,
    }
}

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
/// (plan §3.11 step 4). Creates (or dedups to) the archetype, reserves `n` MORE
/// rows (additive), writes every column at the append base `start_row` (0 for a
/// fresh archetype; the existing row count when a prior block dedup'd to this same
/// archetype after `Ignore` columns were dropped), commits + stamps ticks, allocates
/// and registers a fresh entity batch, and records the saved→fresh mapping into
/// `map`.
///
/// `ids` is the canonical (sorted) component-id set of the fresh archetype.
/// `columns` carries one [`LoadColumn`] per id in `ids` that this archetype stores
/// (the parser excludes a no-data, no-ctor column from BOTH `ids` and `columns`, so
/// the archetype never contains an uninitializable pool — that component is counted
/// `types_defaulted` by the caller). `saved_entity_ids` are the file's per-row saved
/// `EntityId`s (used only to populate `map`; the fresh ids are independent).
///
/// On success returns the number of entities loaded (`n`). On a malformed
/// `Decode` column it rolls the fresh archetype back to empty and returns
/// [`LoadWriteError::Decode`] — the world is left consistent (the archetype exists
/// but holds no rows, `entity_master` untouched). When the file declares more rows
/// than a hosted pool can hold (a forged or block-collapse-summed `entity_count`)
/// it returns [`LoadWriteError::CapacityExceeded`] BEFORE any row is written (C2 —
/// formerly an `.expect()` panic at the reserve site).
///
/// # Panics
///
/// `panic`s only on an internal invariant violation (a bug): an archetype id the
/// master just created not resolving, or a column id with no pool in the fresh
/// archetype. Both are `expect`-guarded.
///
/// The "column id with no pool" `expect` is UNREACHABLE from any file input: the
/// only ids whose freshly-created archetype lacks a pool are enable tags
/// (`StorageKind::Bitset`, filtered out of the signature), and the file-format
/// parser (`boyko_serialize::load`) excludes a bitset-classified id from `ids` /
/// `columns` in pass 1 (the W1 hardening), so every id reaching here has a pool. A
/// fire would mean the parser passed a bitset id, i.e. a caller-side bug.
pub fn load_archetype(
    world: &mut EcsMaster,
    ids: &[ComponentId],
    columns: &[LoadColumn<'_>],
    saved_entity_ids: &[EntityId],
    map: &mut LoadEntityMap,
) -> Result<usize, LoadWriteError> {
    let n = saved_entity_ids.len();
    let current_tick = world.current_tick();

    // ── Create (or dedup to) the archetype + mint slab-stable provenance ───────
    // `create_archetype` DEDUPS against an existing archetype with the same id set.
    // Two file blocks can resolve to the SAME running archetype after `Ignore`
    // columns are dropped on load (e.g. a parent saved as `{Tag, ChildOf, Children}`
    // collapses onto a child saved as `{Tag, ChildOf}` because `Children` is
    // `Ignore`/skipped). So this archetype is "fresh" only on the FIRST block that
    // reaches it; a subsequent block APPENDS at the current row head. The W4 anchor
    // is therefore relaxed from "start_row == 0" to "start_row == the archetype's
    // current row count" — `reserve_capacity` already grows additively (count + n),
    // and `commit_units` extends the tail.
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

    // The append base: the archetype's existing row count (0 for a fresh archetype,
    // > 0 when a prior block already loaded rows into this same dedup'd archetype).
    // SAFETY: `archetype_ptr` is write-capable slab provenance under `&mut
    //   EcsMaster`; this reborrow only reads `current_index` and is dropped here.
    let start_row = unsafe { (*archetype_ptr).current_index };

    // ── Reserve pool capacity for n MORE rows (additive: count + n) ────────────
    // `reserve_capacity` is ADDITIVE (`Err` iff some pool's `len + n` exceeds its
    // reserve ceiling). The load-side per-block guard cannot see `len` when two file
    // blocks dedup-collapse onto ONE running archetype, so this is the SINGLE
    // authoritative gate (C2): propagate a LOUD `Err` instead of panicking. No row is
    // written before this point, so the archetype/world is untouched on failure.
    {
        // SAFETY: `archetype_ptr` is write-capable slab provenance under
        //   `&mut EcsMaster`; this reborrow is confined to the reserve call.
        let archetype: &mut Archetype = unsafe { &mut *archetype_ptr };
        if archetype.reserve_capacity(n).is_err() {
            // Identify the offending column for the diagnostic: the first column id
            // whose pool cannot hold `n` MORE rows (the additive ceiling check). A
            // column whose pool is absent (impossible here — every present id has a
            // pool) is skipped.
            let requested = start_row.saturating_add(n);
            let pools = archetype.component_pools();
            let component = columns
                .iter()
                .map(load_column_id)
                .find(|&cid| pools.get_pool(cid).is_some_and(|pool| !pool.can_reserve(n)))
                .unwrap_or_else(|| {
                    // Unreachable in practice (some pool rejected the reserve), but
                    // stay total: name the first column rather than panic.
                    columns.first().map(load_column_id).unwrap_or(ComponentId(0))
                });
            return Err(LoadWriteError::CapacityExceeded {
                component,
                requested,
            });
        }
    }

    let mut guard = ArchetypeLoadGuard::new(archetype_ptr, columns.len());

    // ── Per column: blit / decode / construct, committing as each completes ────
    for col in columns {
        match col {
            LoadColumn::Blit { component_id, bytes } => {
                // SAFETY (W4 / W7):
                //   * `bytes.len() == n * stride` (parser-validated); the pool
                //     reserved `n` MORE rows (additive) so its data sub-region holds
                //     `(start_row + n) * stride` committed bytes.
                //   * The blit writes `buffer_ptr_mut().add(start_row * stride)` for
                //     `n * stride` bytes — into the freshly-reserved tail, never
                //     overwriting the `start_row` pre-existing rows nor overrunning
                //     into the co-located tick sub-region.
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
                    // SAFETY: `dst` is the freshly-reserved tail of the pool — exactly
                    //   `n * stride` bytes starting at row `start_row`; `src` is the
                    //   in-file POB image, all-bits valid; disjoint allocations.
                    unsafe {
                        std::ptr::copy_nonoverlapping(
                            bytes.as_ptr(),
                            pool.buffer_ptr_mut().add(start_row * stride),
                            bytes.len(),
                        );
                    }
                }
                pool.commit_units(start_row, n);
                pool.fill_ticks(start_row, n, current_tick);
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
                    // Absolute pool row for this column element: the append base plus
                    // the per-column row index.
                    let pool_row = start_row + row;
                    // Derive the dst `*mut u8` in a TIGHT scope, then drop the
                    // `&mut Archetype` BEFORE calling the (panic-prone, fallible)
                    // user `deserialize_fn`. At the call NO `&mut Archetype` is live
                    // — only `dst: *mut u8` (no TB protector across the call) — so on
                    // a panic the guard's `Drop` is the sole `&mut Archetype`
                    // accessor (the F2 / W5 anchor).
                    // SAFETY: `pool_row < start_row + n <= committed_rows` after
                    //   `reserve_capacity`; the slot is reserved-uninit; the returned
                    //   `*mut u8` carries the pool's reservation provenance, aligned
                    //   for this type.
                    let dst_ptr = unsafe {
                        let archetype: &mut Archetype = &mut *archetype_ptr;
                        let pool = archetype
                            .component_pools_mut()
                            .get_pool_mut(*component_id)
                            .expect("invariant: a Decode column id has a pool");
                        pool.reserved_row_ptr(pool_row)
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
                        return Err(LoadWriteError::Decode(e));
                    }

                    // Commit + stamp this row immediately so the rollback path can
                    // drop it via `drop_at` (which needs a committed `len`). A
                    // confined `&mut Archetype` reborrow; neither op can panic.
                    // SAFETY: `pool_row == pool.count()` (rows extend the tail in
                    //   order from the append base); `commit_units(pool_row, 1)` makes
                    //   the just-written slot live; `fill_ticks` then stamps it.
                    unsafe {
                        let archetype: &mut Archetype = &mut *archetype_ptr;
                        let pool = archetype
                            .component_pools_mut()
                            .get_pool_mut(*component_id)
                            .expect("invariant: a Decode column id has a pool");
                        pool.commit_units(pool_row, 1);
                        pool.fill_ticks(pool_row, 1, current_tick);
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
                // SAFETY: `start_row + row < start_row + n <= committed_rows`; each
                //   slot is reserved-uninit; `ctor` writes one value of the pool's
                //   registered type (registry-paired) without dropping the uninit
                //   dst. `&mut` confined here.
                let archetype: &mut Archetype = unsafe { &mut *archetype_ptr };
                let pool = archetype
                    .component_pools_mut()
                    .get_pool_mut(*component_id)
                    .expect("invariant: a Construct column id has a pool");
                for row in 0..n {
                    // SAFETY: as documented above; `start_row + row < committed_rows`.
                    unsafe { pool.construct_at_uninitialized(start_row + row, *ctor) };
                }
                pool.commit_units(start_row, n);
                pool.fill_ticks(start_row, n, current_tick);
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
    // The fresh ids land at rows `[start_row, start_row + n)` (appended to any
    // pre-existing rows from a prior block that dedup'd to this same archetype).
    // SAFETY: `archetype_ptr` is write-capable slab provenance; `&mut` confined;
    //   neither op can panic.
    unsafe {
        let archetype: &mut Archetype = &mut *archetype_ptr;
        archetype
            .entity_ids
            .extend((start_id..start_id + n).map(EntityId));
        archetype.current_index = start_row + n;
    }

    // The archetype + pools are now fully consistent; disarm before touching the
    // entity master (W5 — past this point a panic would no longer roll back the
    // archetype, but registration is infallible).
    guard.disarm();

    // Each fresh entity's `unit_index` is its absolute archetype row: `start_row`
    // is the append base (0 for a fresh archetype; the prior row count when this
    // block appended to a dedup'd archetype).
    world.entity_master.register_batch(
        EntityId(start_id),
        archetype_ptr,
        start_row as u32,
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

/// Dense plan D4 — loads ONE dense store's compacted snapshot into the world.
///
/// The `boyko_serialize` file-format parser resolves a `DenseStoreBlock` into
/// `(component_id, member_bytes, saved_s2e)` and calls this crate-private driver
/// (it owns `EcsMaster::dense_registry_mut` + the `DenseStore` write primitives the
/// downstream crate cannot reach). For each saved member it remaps the saved
/// owning `EntityId` to its freshly-allocated `EntityId` via `map` (the same
/// machinery `ChildOf`/`#[entities]` use), then inserts the value into a fresh
/// `DenseStore` — rebuilding `e2s` / `s2e` / `live` / `free` and stamping the
/// slot's `added` + `changed` ticks at the load tick (the membership is Added on
/// load, mirroring the table blit's `fill_ticks`).
///
/// `member_bytes` is the compacted live column (`member_count * stride` bytes, slot
/// order); `saved_s2e` is the per-slot saved `EntityId.0` (same length). A POB store
/// is a contiguous blit-in-slot-order; the per-member `insert` copies each member's
/// `stride` bytes, which for a POD dense component (the physics-body case) is a plain
/// memcpy. COLD — reached only from `boyko_serialize::load_world`.
///
/// # Fresh-world-load invariant
///
/// MUST be called against a freshly-created world (the loader's W4 contract): the
/// target `DenseStore` is assumed EMPTY on entry. Merge-load into a populated world
/// is unsupported — a remapped fresh id could collide with an existing member and
/// silently corrupt the store. A `debug_assert!(store.is_empty())` trips loudly on a
/// future merge-load mistake; it is safe today only because every loaded entity
/// received a brand-new id.
///
/// # Errors
///
/// [`DecodeError::UnmappedEntity`] if a saved owning id is absent from `map` (a
/// corrupt / foreign file — never a silent dangling membership).
///
/// # Panics
///
/// Debug-asserts `member_bytes.len() == member_count * stride` (the parser
/// validated it) and `store.is_empty()` (the fresh-world-load invariant); a release
/// mismatch is impossible from the parser's own save into a fresh world.
pub fn load_dense_store(
    world: &mut EcsMaster,
    component_id: ComponentId,
    member_bytes: &[u8],
    saved_s2e: &[u64],
    map: &LoadEntityMap,
) -> Result<usize, DecodeError> {
    let member_count = saved_s2e.len();
    if member_count == 0 {
        return Ok(0);
    }
    let current_tick = world.current_tick();
    let stride = member_bytes
        .len()
        .checked_div(member_count)
        .filter(|s| s * member_count == member_bytes.len())
        .unwrap_or(0);
    debug_assert!(
        stride * member_count == member_bytes.len(),
        "load_dense_store: member_bytes.len() ({}) != member_count ({}) * stride ({})",
        member_bytes.len(),
        member_count,
        stride
    );

    // Resolve every (fresh entity, its archetype) FIRST — `entity_archetype_id`
    // borrows `&world`, which cannot coexist with the `&mut DenseStore` borrow
    // below. The fresh entities are already live (the archetype loads ran before
    // this dense pass), so each resolves; an unmapped saved id is a loud
    // `DecodeError` (C4 — never a silent dangling membership).
    let mut resolved: Vec<(Entity, Option<ArchetypeId>)> = Vec::with_capacity(member_count);
    for &saved_id in saved_s2e {
        let fresh = map
            .get(saved_id as usize)
            .ok_or(DecodeError::UnmappedEntity)?;
        let arch = world.entity_archetype_id(fresh);
        resolved.push((fresh, arch));
    }

    let store = world.dense_registry_mut().store_mut(component_id);
    // FRESH-WORLD-LOAD invariant: the loader's contract (W4) is to load into a
    // freshly-created world, so the target dense store MUST be empty here. A future
    // "merge load" into a populated world would let a remapped fresh id collide with
    // an existing member's slot/`e2s` entry and silently corrupt the store; this
    // trips loudly in debug instead. Safe today only because every loaded entity got
    // a brand-new id, so no remapped id can already be a member of this store.
    debug_assert!(
        store.is_empty(),
        "load_dense_store: target dense store for {component_id:?} is not empty \
         (fresh-world-load contract violated — merge load is unsupported)"
    );
    for (i, &(fresh, arch)) in resolved.iter().enumerate() {
        let lo = i * stride;
        let bytes = &member_bytes[lo..lo + stride];
        // `insert` rebuilds e2s/s2e/live/free for this slot and stamps both ticks
        // (the membership is Added on load — the load-tick contract).
        store.insert(fresh.id(), bytes, current_tick);
        // Seed the candidate-archetype bitset so a pure-dense / mixed query finds
        // the restored member (matches the structural insert path's
        // `mark_arch_present`). A fresh entity with no archetype (shouldn't occur —
        // every loaded entity has one) is simply not seeded.
        if let Some(arch) = arch {
            store.mark_arch_present(arch);
        }
    }
    Ok(member_count)
}

/// Dense plan D4 (v1.1) — loads ONE OWNING (`SerializeViaFn`) dense store's
/// compacted snapshot into the world, decoding each member with its per-element
/// `deserialize_fn`.
///
/// The dense analogue of the table [`LoadColumn::Decode`] path: where
/// [`load_dense_store`] blits a POB member's `stride` bytes in place, an OWNING
/// dense member (a `Vec`/`String`/bit-restricted/entity-bearing dense component)
/// carries a VARIABLE-LENGTH encoded run on disk (the saver ran `serialize_fn` per
/// member into one shared cursor — `save.rs` dense ViaFn path), so `data`'s total
/// length is NOT `member_count * stride`. This driver reads `member_count` elements
/// sequentially from ONE [`LoadCursor`] over the whole `data` run (each element's
/// length is self-describing — length-prefixed heap fields + fixed leaf widths),
/// reconstructing each value into a reused scratch buffer, then BYTE-MOVING it into
/// a fresh [`DenseStore`] slot via [`DenseStore::insert`].
///
/// Saved owning ids are remapped through `map` exactly like [`load_dense_store`].
///
/// # Scratch-buffer byte-move (no double-drop)
///
/// `DenseStore::insert` COPIES the passed `&[u8]` into the slot (a bitwise move of
/// the reconstructed value). So each member is decoded into a heap scratch buffer of
/// the component's exact [`Layout`](std::alloc::Layout) (size + align — the
/// `DeserializeFn` contract requires `dst` aligned to `align_of::<C>()`), the
/// `stride` scratch bytes are handed to `insert` (which moves them into the store),
/// and the scratch allocation is then FREED AS RAW BYTES — never dropped: on the
/// `Ok` path the value's ownership already moved into the store (dropping the
/// scratch would double-free its heap fields), and on the `Err` path the scratch is
/// left uninitialized (dropping it would be UB). One scratch buffer is allocated
/// once and reused across all members; it is freed exactly once at the end (or on
/// the error return) regardless of outcome.
///
/// # Fresh-world-load invariant / Errors / Panics
///
/// Identical to [`load_dense_store`]: MUST target a freshly-created world (the
/// target store is `debug_assert!`-empty); [`DecodeError::UnmappedEntity`] on a
/// saved id absent from `map`; a malformed/short encoded run surfaces as the
/// [`DecodeError`] its `deserialize_fn` returns (never UB). On any error the
/// members inserted so far stay in the store, but `load_world` propagates the error
/// and the caller discards the partially-loaded world — matching the POB dense path.
/// COLD — reached only from `boyko_serialize::load_world`.
pub fn load_dense_store_via_fn(
    world: &mut EcsMaster,
    component_id: ComponentId,
    deserialize_fn: DeserializeFn,
    member_data: &[u8],
    saved_s2e: &[u64],
    map: &LoadEntityMap,
) -> Result<usize, DecodeError> {
    let member_count = saved_s2e.len();
    if member_count == 0 {
        return Ok(0);
    }
    let current_tick = world.current_tick();

    // Resolve every (fresh entity, its archetype) FIRST — `entity_archetype_id`
    // borrows `&world`, which cannot coexist with the `&mut DenseStore` borrow
    // below (identical to `load_dense_store`). An unmapped saved id is a loud
    // `DecodeError` before any value is decoded or inserted (C4).
    let mut resolved: Vec<(Entity, Option<ArchetypeId>)> = Vec::with_capacity(member_count);
    for &saved_id in saved_s2e {
        let fresh = map
            .get(saved_id as usize)
            .ok_or(DecodeError::UnmappedEntity)?;
        let arch = world.entity_archetype_id(fresh);
        resolved.push((fresh, arch));
    }

    let store = world.dense_registry_mut().store_mut(component_id);
    // FRESH-WORLD-LOAD invariant — see `load_dense_store`.
    debug_assert!(
        store.is_empty(),
        "load_dense_store_via_fn: target dense store for {component_id:?} is not empty \
         (fresh-world-load contract violated — merge load is unsupported)"
    );

    let layout = store.component_layout();
    let stride = layout.size();

    // One reused scratch buffer of the component's exact layout. A ZST dense
    // component (`stride == 0`, e.g. a bit-restricted marker) needs no allocation:
    // `deserialize_fn` writes nothing to `dst` and `insert` copies zero bytes, so a
    // dangling-but-aligned pointer is a valid `dst` for a zero-sized `C`.
    let scratch: *mut u8 = if stride == 0 {
        // A non-null, well-aligned dangling pointer for the ZST case (the layout's
        // align is a valid dangling address for a zero-sized value).
        layout.align() as *mut u8
    } else {
        // SAFETY: `layout` has non-zero size (checked) and a valid power-of-two
        //   align (from a registered component). `alloc` returns either a block
        //   valid for `layout` or null (handled below).
        unsafe { std::alloc::alloc(layout) }
    };
    if scratch.is_null() {
        // Allocation failure: mirror `std`'s convention (a raw-alloc null is an OOM
        // abort site), but stay total on the load path — surface a decode error
        // rather than deref a null. No value was decoded, so nothing to drop.
        return Err(DecodeError::UnexpectedEof);
    }

    let mut cursor = LoadCursor::new(member_data);
    let mut inserted = 0usize;
    for &(fresh, arch) in &resolved {
        // SAFETY (registry `DeserializeFn` contract):
        //   * `scratch` points at writable, uninitialized space of `stride ==
        //     size_of::<C>()` bytes aligned to `align_of::<C>()` (the `layout`
        //     alloc, or the aligned dangling ptr for a ZST).
        //   * On `Ok` the scratch holds an initialized `C` written exactly once; on
        //     `Err` it is left uninitialized and we free it as RAW bytes (below)
        //     without dropping — no UB.
        //   * No `&mut DenseStore` is live across this call (only `store` is
        //     borrowed, and `deserialize_fn` touches only `scratch` + `cursor`).
        let decoded = unsafe { deserialize_fn(&mut cursor, scratch) };
        if let Err(e) = decoded {
            // The scratch is uninitialized (the failed decode left it untouched).
            // Free it as raw bytes — never drop (the value was never constructed).
            // SAFETY: `scratch` came from `alloc(layout)` with the SAME `layout`
            //   (only reached for `stride != 0`; a ZST decode cannot fail on `dst`).
            if stride != 0 {
                unsafe { std::alloc::dealloc(scratch, layout) };
            }
            return Err(e);
        }

        // BYTE-MOVE the reconstructed value into a fresh slot. `insert`
        // `copy_nonoverlapping`s exactly `stride` bytes, moving ownership of the
        // value (and any heap it owns) into the store; the scratch bytes are now a
        // stale bitwise copy that MUST NOT be dropped.
        // SAFETY: `scratch` holds an initialized, `stride`-sized `C` (the `Ok`
        //   above); the slice borrows the live scratch for the `insert` call only.
        let value_bytes = unsafe { std::slice::from_raw_parts(scratch, stride) };
        store.insert(fresh.id(), value_bytes, current_tick);
        if let Some(arch) = arch {
            store.mark_arch_present(arch);
        }
        inserted += 1;
    }

    // Free the scratch allocation ONCE, as raw bytes — every decoded value was moved
    // into the store, so no drop runs here (a drop would double-free the moved-out
    // heap fields).
    // SAFETY: `scratch` came from `alloc(layout)` with the SAME `layout`; the last
    //   value it held was byte-moved into the store, so freeing the raw block frees
    //   no live value. Skipped for a ZST (no allocation was made).
    if stride != 0 {
        unsafe { std::alloc::dealloc(scratch, layout) };
    }

    debug_assert_eq!(
        inserted, member_count,
        "load_dense_store_via_fn: inserted count must equal member_count"
    );
    Ok(member_count)
}

/// S2.5 — the entity-remap pass (plan §3.11 step 5 + §5 C4). Runs AFTER every
/// archetype has loaded (a separate whole-world pass): rewrites every saved
/// `Entity` reference inside a remappable component to its freshly-allocated
/// `Entity`, translated through `map` (saved id → fresh, populated by
/// [`load_archetype`]).
///
/// A column is remappable iff its
/// [`SerializeInfo::map_entities_fn`](component_registry::SerializeInfo::map_entities_fn)
/// is `Some` — `ChildOf` (the hand-written v1 relationship remap) and any
/// `#[entities]`-annotated derived component. A plain `Entity` field WITHOUT
/// `#[entities]` is NOT remapped (the C4 explicit-opt-in decision — it keeps its
/// raw saved id). Every other component pays nothing (its slot is unset, so it is
/// never visited through the fn-ptr).
///
/// An unmapped saved id (referenced but absent from `map`) is a [`DecodeError`]
/// (`UnmappedEntity`) — the C4 loud error, never a silent dangling reference. The
/// pass mutates the loaded rows in place; on an error the world is left with those
/// rows holding their (partially-remapped) saved ids, but the caller surfaces the
/// error so no consumer observes a dangling reference as valid.
///
/// # F2 / W5 borrow discipline (critical)
///
/// The remap is a PURE per-row mutation with NO structural op during the walk.
/// Mirroring [`load_archetype`]'s `Decode` loop, it NEVER holds a `&mut Archetype`
/// (nor a cached `NonNull`) across the fn-ptr call: it snapshots the live archetype
/// ids first (the immutable iter borrow ends before any write), then per archetype
/// re-derives a slab-stable `*mut Archetype`, and per ROW derives the dst `*mut u8`
/// in a tight scope that DROPS the `&mut Archetype` reborrow BEFORE invoking the
/// (panic-prone, user-authored) `map_entities_fn`. At the call only `dst: *mut u8`
/// is live (no TB protector spans it).
///
/// # Panics
///
/// Never panics on its own; a panic inside a user `map_entities_fn` (a derive-
/// generated remap is panic-free by construction) unwinds through the caller.
pub fn remap_loaded_entities(
    world: &mut EcsMaster,
    map: &LoadEntityMap,
) -> Result<(), DecodeError> {
    // Snapshot the live archetype ids under a SHORT immutable borrow that ends
    // before any write — never iterate-and-mutate (W5). The slab is fixed (no
    // archetype is created during the remap pass), so each id stays resolvable.
    let archetype_ids: Vec<ArchetypeId> = world
        .archetype_master()
        .iter_archetypes()
        .map(|a| a.id())
        .collect();

    for archetype_id in archetype_ids {
        // Re-derive a slab-stable, write-capable pointer per archetype (mirrors
        // `load_archetype`). `archetype_ptr_for` mints provenance under
        // `&mut EcsMaster`; the slab is never relocated, so it stays valid across
        // the per-row reborrows below.
        let Some(archetype_ptr) = world.archetype_master_mut().archetype_ptr_for(archetype_id)
        else {
            continue;
        };

        // Gather the remappable columns of this archetype FIRST (one tight borrow),
        // so the per-row write loop below holds no archetype-wide borrow while it
        // re-derives row pointers. Each entry is `(component_id, map_entities_fn)`.
        let remap_columns: Vec<(ComponentId, LoadMapEntitiesFn)> = {
            // SAFETY: `archetype_ptr` is write-capable slab provenance under
            //   `&mut EcsMaster`; this shared reborrow only READS the component-id
            //   set and is dropped at the end of this block (before any write).
            let archetype: &Archetype = unsafe { &*archetype_ptr };
            archetype
                .component_ids()
                .iter()
                .filter_map(|&cid| {
                    component_registry::get_serialize_info(cid.0)
                        .and_then(|info| info.map_entities_fn)
                        .map(|f| (cid, f))
                })
                .collect()
        };

        for (cid, map_entities_fn) in remap_columns {
            // Resolve this pool's live row count + stride in a tight scope, then drop
            // the borrow before the write loop.
            let (row_count, stride) = {
                // SAFETY: as above — a shared reborrow that only reads the pool's
                //   `count` / layout; dropped before the write loop.
                let archetype: &Archetype = unsafe { &*archetype_ptr };
                match archetype.component_pools().get_pool(cid) {
                    Some(pool) => (pool.count(), pool.component_layout().size()),
                    None => continue,
                }
            };

            for row in 0..row_count {
                // Derive the dst `*mut u8` for this LIVE row in a TIGHT scope, then
                // drop the `&mut Archetype` BEFORE calling the (panic-prone, fallible)
                // user `map_entities_fn` — exactly the F2 / W5 anchor `load_archetype`
                // uses for `deserialize_fn`. At the call NO `&mut Archetype` is live;
                // only `dst: *mut u8` (no TB protector across the call).
                // SAFETY: `archetype_ptr` is write-capable slab provenance under
                //   `&mut EcsMaster`; this reborrow + `buffer_ptr_mut` derivation is
                //   confined to this scope. `row < row_count == pool.count()`, so the
                //   row is a LIVE, initialized value of this column's type, and
                //   `buffer_ptr_mut().add(row * stride)` lies inside the pool's data
                //   sub-region (stride 0 is a ZST column — never remappable here, but
                //   `add(0)` is still valid).
                let dst_ptr = unsafe {
                    let archetype: &mut Archetype = &mut *archetype_ptr;
                    let pool = archetype
                        .component_pools_mut()
                        .get_pool_mut(cid)
                        .expect("invariant: a remappable column has a pool");
                    pool.buffer_ptr_mut().add(row * stride)
                };

                // SAFETY (`LoadMapEntitiesFn` contract):
                //   * `dst_ptr` points at a live, initialized value of `cid`'s type
                //     (a row written by the load + committed via `commit_units`).
                //   * `map` is a shared, non-aliased reference for the call's
                //     duration (single-threaded `&mut EcsMaster`).
                //   * NO `&mut Archetype` is live across this call (dropped above), so
                //     a panic inside the fn has the caller's frame as the sole
                //     `&mut EcsMaster` accessor (F2 / W5).
                // An unmapped saved id surfaces as `Err(UnmappedEntity)` (C4 loud).
                unsafe { map_entities_fn(dst_ptr, map)? };
            }
        }
    }

    Ok(())
}
