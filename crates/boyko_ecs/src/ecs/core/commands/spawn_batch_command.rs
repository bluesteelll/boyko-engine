//! Phase 12.5 Opt-A2 — `SpawnBatchCommand<B, I>` deferred bulk-spawn
//! command.
//!
//! See `docs/PHASE-12.5-SPAWN-OPTIMIZATIONS-PLAN.md` §5 for the design.
//!
//! # Why this exists
//!
//! `Commands::spawn(bundle)` collapses to **one** `SpawnAtCommand<B>` per
//! entity, so a 10 K-entity wave pays the full per-command dispatch dance
//! 10 000 times (cf. `docs/PHASE-12.5-PROFILE-SPAWN.md` finding #1 —
//! ~30-50 ns/entity overhead from the command queue's outer loop).
//!
//! `SpawnBatchCommand<B, I>` enqueues **one** command for the entire batch:
//! it resolves `B::cached_archetype_id` once, reserves entity IDs in a
//! single atomic `fetch_add(n)`, pre-grows every pool by `n` rows in one
//! capacity check, and runs a tight inline loop over the user-provided
//! iterator inside a single `for_each_component_bytes`-equivalent block.
//!
//! Combined with the Opt-A3 `BundleColumnCache`, the per-entity SparseMap
//! lookup count drops from 4× per component to 0× — the warm path indexes
//! `pools[pool_ids[canonical_idx]]` directly.
//!
//! # Layout (Q-A2.3)
//!
//! `#[repr(C)]` with `start_entity` first. `Entity` is `EntityId(8 B,
//! `#[repr(transparent)]` over `usize`) + `generation: u32` (4 B) + 4 B
//! trailing alignment pad = **16 B total**, not 8 B as an early draft of
//! this comment claimed. The struct's overall byte budget per queue slot:
//!
//! * `I = std::iter::Empty<B>` (ZST): 16 + 4 + 4 + 0 = 24 B payload
//!   + 8 B `CommandMeta` = **32 B per queue slot**.
//! * `I = Range<u32>` (8 B): 16 + 4 + 4 + 8 = 32 B payload + 8 B
//!   `CommandMeta` = **40 B per queue slot**.
//!
//! Adjust by the iterator's actual `size_of::<I>()` for other shapes; the
//! command layout itself is the 24 B prefix.
//!
//! # Invariants (SBO-SEND1, SBO-UNPIN, SBO3, SBO7, SBO9, SBO17b)
//!
//! * **SBO-SEND1**: `Send + Sync` are auto-derived (no hand-written
//!   `unsafe impl`). The trait bounds `B: Bundle (Send + Sync + Unpin +
//!   'static)` and `I: ExactSizeIterator<Item = B> + Send + Sync + Unpin
//!   + 'static` guarantee soundness. Pinned at compile time via
//!   `assert_impl_all!` (production build, NOT `#[cfg(test)]` — I-N5).
//! * **SBO-UNPIN (C-N3)**: `Bundle: Unpin` is a trait supertrait;
//!   `I: Unpin` is a bound on the iterator. Both are required for
//!   `CommandQueue::push`'s bitwise relocation through `write_unaligned`
//!   / `read_unaligned` to be sound.
//! * **SBO3**: a single `EntityCounter::reserve_batch(n)` happens at
//!   enqueue time (worker side). Apply consumes the pre-reserved range;
//!   no further atomic on the apply path.
//! * **SBO7**: iterator state lives inline inside the command's bytes
//!   (not boxed). Drop-glue runs the user iter's `Drop` on un-flushed
//!   queues. Bitwise relocation through the queue is sound because both
//!   `B` and `I` are `Unpin`.
//! * **SBO9**: mid-iterator panic leaves the batch in a partial state —
//!   rows `[0..i)` survive, IDs `[i..n)` leak. ManuallyDrop (B4)
//!   suppresses double-drop with archetype-side ownership.
//! * **SBO17b (I-N1)**: aggregate-worker overshoot is caught at apply
//!   time by a `end_id <= entities_inland.len()` check; on violation the
//!   apply panics with `WorldEntityCapacityExceeded` (propagates through
//!   Opt-A1's outer `catch_unwind`).

#![allow(dead_code)]

use static_assertions::assert_impl_all;

use crate::ecs::core::archetype::archetype::Archetype;
use crate::ecs::core::bundle::Bundle;
use crate::ecs::core::commands::command::Command;
use crate::ecs::core::ecs_master::ecs_master::EcsMaster;
use crate::ecs::core::entity::entity::Entity;
use crate::ecs::identifiers::primitives::{EntityId, InlandPoolId};

/// Maximum component count for a derived `Bundle`. Mirrors the macro's
/// `MAX_BUNDLE_ARITY` and the sibling stack collectors in `spawn_at_command.rs`
/// / `migration_helpers.rs`; sizes the Phase 22.1 `data_pool_ids` stack array.
const MAX_BUNDLE_ARITY: usize = 16;

/// Phase 12.5 Opt-A2 (§5.2): deferred "spawn N entities sharing bundle
/// type `B`, sourced from iterator `I`" command.
///
/// # Layout (24 B + sizeof::<I>())
///
/// `Entity` is `EntityId(8 B) + generation: u32 (4 B) + 4 B pad = 16 B`,
/// so the prefix runs to +24:
///
/// ```text
/// +0  : start_entity: Entity   (16 B  — EntityId(8) + generation(4) + pad(4))
/// +16 : count: u32             (4 B)
/// +20 : _pad: u32              (4 B)
/// +24 : iter: I                (sizeof::<I>(), aligned to align_of::<I>())
/// ```
#[repr(C)]
pub(crate) struct SpawnBatchCommand<B, I>
where
    // Bundle: Send + Sync + Unpin via supertrait (SBO-UNPIN).
    B: Bundle + Send + Sync,
    // C-N3: Unpin bound on the iterator state for SBO7 (bitwise relocation
    // through the queue's byte arena).
    I: ExactSizeIterator<Item = B> + Send + Sync + Unpin + 'static,
{
    /// First reserved entity. The range covered by this command is
    /// `[start_entity.id().0, start_entity.id().0 + count)`. All entities
    /// share `generation = 0` (fresh path, EM1).
    pub(crate) start_entity: Entity,

    /// Number of entities to spawn. Bounded by `MAX_BATCH_HINT = 8_192`
    /// per SBO17 (validated at enqueue time by
    /// `EntityCounter::reserve_batch`).
    pub(crate) count: u32,

    /// Padding to align the `I` field on the natural boundary regardless
    /// of `I`'s alignment requirement.
    pub(crate) _pad: u32,

    /// User-supplied iterator. Drained at apply time; if the queue drops
    /// without apply, the iterator's `Drop` runs via the queue's drop-glue.
    pub(crate) iter: I,
}

// SBO-SEND1 + SBO-UNPIN: pin the auto-derived `Send + Sync + Unpin` at
// production build time using a concrete `derive(Bundle)` stub. I-N5
// pins us OUTSIDE `#[cfg(test)]` — failure mode is build-break, not
// test-fail.
//
// W1 RESOLUTION (plan §5.2): `assert_impl_all!` is a `const _:` item —
// pure compile-time. It does NOT trigger any runtime path (no
// `static_info()` call, no `OnceLock` init), so the lazy
// `ComponentRegistry` / `BundleTypeRegistry` registration is never
// executed unless the pin-test types are spawned — which they are not
// (the module is doc-hidden and unused outside this assertion).
//
// Iter pin: `std::iter::Empty<PinTestBundle>` is the simplest
// `ExactSizeIterator<Item = PinTestBundle> + Send + Sync + Unpin +
// 'static` we can name without dragging in user-side types. The plan's
// `Range<u32>` reference was a layout-size example; the actual iterator
// shape used for the pin must satisfy `Item = B`, so `Empty<B>` is the
// canonical choice.
assert_impl_all!(
    SpawnBatchCommand<
        __private_pin_test::PinTestBundle,
        std::iter::Empty<__private_pin_test::PinTestBundle>,
    >:
    Send, Sync, Unpin
);

/// Phase 12.5 Opt-A2 / W1: doc-hidden bundle stub used solely by the
/// `assert_impl_all!` pin above.
///
/// The `derive(Bundle)` macro emits the required `Send + Sync + Unpin +
/// 'static` impl by construction over a single-field `Component`-derived
/// struct. The stub is never spawned at runtime; the pin is purely a
/// compile-time gate.
#[doc(hidden)]
pub mod __private_pin_test {
    use crate::ecs::core::component::component::Component;
    use crate::ecs::identifiers::primitives::ComponentId;

    /// Pin-test component. Minimal `u8` field so the stub never collides
    /// with a user-registered `ComponentId`. The trait body is a stub —
    /// `component_id` is never invoked at runtime (the bundle is only
    /// named at the `assert_impl_all!` site, which is `const _:` and
    /// does not call any trait method). The constant returned here is
    /// arbitrary; an explicit out-of-range value would panic at runtime
    /// if a future change accidentally called this method.
    #[repr(C)]
    pub struct PinTestComp(pub u8);

    impl Component for PinTestComp {
        fn component_id() -> ComponentId {
            // Never invoked: see the struct doc-comment above. The fixed
            // id keeps the impl total without forcing a `register_layout`
            // side effect. The chosen value is the largest valid slot
            // (`MAX_COMPONENTS - 1 = 511`) so even if the method were
            // somehow reached it would not OOB on the registry array.
            ComponentId(511)
        }
    }

    /// Pin-test bundle. Single `PinTestComp` field — minimal for
    /// `derive(Bundle)` to satisfy the "≥ 1 field" requirement
    /// (`derive(Bundle)` rejects unit / empty structs at compile time).
    ///
    /// Manually implementing `Bundle` here mirrors what the derive
    /// produces, but routes through stub bodies because none of the
    /// trait methods are ever invoked at runtime — only the
    /// `assert_impl_all!` reads the trait bounds.
    pub struct PinTestBundle {
        pub c: PinTestComp,
    }

    impl crate::ecs::core::bundle::bundle::sealed::BundleSealed for PinTestBundle {}

    impl crate::ecs::core::bundle::bundle::Bundle for PinTestBundle {
        fn static_info()
        -> &'static crate::ecs::core::bundle::bundle::BundleStaticInfo {
            // Never invoked. Returning a leaked default keeps the impl
            // total. `assert_impl_all!` is `const _:` — it inspects trait
            // bounds, not call sites.
            unreachable!(
                "PinTestBundle::static_info is a compile-time stub for \
                 assert_impl_all!; should never be invoked at runtime"
            )
        }

        fn cached_archetype_id(
            _world: &mut crate::ecs::core::ecs_master::ecs_master::EcsMaster,
        ) -> crate::ecs::identifiers::primitives::ArchetypeId {
            unreachable!(
                "PinTestBundle::cached_archetype_id is a compile-time stub"
            )
        }

        fn for_each_component_bytes<F: FnMut(ComponentId, &[u8])>(
            self,
            _f: F,
        ) {
            unreachable!(
                "PinTestBundle::for_each_component_bytes is a compile-time stub"
            )
        }
    }
}

impl<B, I> Command for SpawnBatchCommand<B, I>
where
    B: Bundle + Send + Sync,
    I: ExactSizeIterator<Item = B> + Send + Sync + Unpin + 'static,
{
    /// Phase 12.5 Opt-A2 (§5.4): batched apply.
    ///
    /// Sequence (W4 / I-N1 / I-N4 incorporated):
    ///
    /// 1. Resolve the destination archetype once via
    ///    `B::cached_archetype_id` (~3 ns warm; ~1 µs cold first-spawn).
    /// 2. Resolve column ids via the Opt-A3 `BundleColumnCache` — one
    ///    Acquire load if warm, full SparseMap walk + leak if cold.
    /// 3. (W4 hoist) SBO-N + SBO-B2 + arity invariants asserted ONCE
    ///    per batch in debug builds (cheap: ≤ `MAX_BUNDLE_ARITY` ops).
    /// 4. SBO17b runtime guard: `end_id <= entities_inland.len()`. On
    ///    overshoot, hard-panic with `WorldEntityCapacityExceeded` —
    ///    aggregate-worker overshoot becomes observable, not silent UB.
    /// 5. `Archetype::reserve_capacity(n)` — `.expect` per I-N4 (apply
    ///    contract: SBO17 cap-check at enqueue is authoritative).
    /// 6. Per-row loop: pull `bundle = iter.next()`, then run
    ///    `for_each_component_bytes` writing each component via
    ///    `pool_at_unchecked_mut(pool_ids[canonical_idx]).write_at_unchecked_initialized(row, bytes)`.
    /// 7. Bulk-commit units + fill ticks via `slice::fill`-style loops
    ///    (vectorisable; saves N×2 per-row tick writes).
    /// 8. Bulk-register entities via `EntityMaster::register_batch`.
    fn apply(self, world: &mut EcsMaster) {
        let n = self.count as usize;
        if n == 0 {
            return;
        }

        // ── Step 1: resolve archetype (Opt-A3 cached) ──────────────────
        let archetype_id = B::cached_archetype_id(world);
        let archetype_ptr = world
            .archetype_master_mut()
            .archetype_ptr_for(archetype_id)
            .expect("invariant: cached_archetype_id returns a registered id");
        let current_tick = world.current_tick();

        // ── Step 2: resolve column ids via Opt-A3 cache ────────────────
        // Warm-path fast track: try `get_resolved::<B>()` first without
        // constructing the cold-path `&Archetype` reborrow. Only the
        // first-spawn-per-(B, world) cold branch needs the shared view.
        //
        // SAFETY (U1, U2, U14): `archetype_ptr` is write-capable provenance
        //   from `archetype_ptr_for` under `&mut self`; the apply path holds
        //   exclusive `&mut EcsMaster`. The shared borrow inside the cold
        //   arm is scoped to the `resolve_and_cache` call; we re-borrow
        //   `&mut Archetype` later.
        // Phase 12.6 — `bundle_column_cache()` lazily materialises the
        // inner cache on first call per world. Subsequent calls hit the
        // outer OnceLock's Acquire load (~1 ns) plus the same indexed-slot
        // path as before.
        let cache = world.bundle_column_cache();
        let cache_record = if let Some(r) = cache.get_resolved::<B>() {
            r
        } else {
            let archetype_shared: &Archetype = unsafe { &*archetype_ptr };
            cache.resolve_and_cache::<B>(archetype_id, archetype_shared)
        };
        let pool_ids = cache_record.pool_ids;
        // Required components (Feature 1, D5 — batch parity with
        // `SpawnAtCommand::apply` Step 5b): the same record carries the
        // transitively-required entries the bundle does NOT supply
        // (`required_missing`) and their resolved columns (`required_pool_ids`).
        // Both are empty `&'static []` for a require-free bundle — the
        // apply-time 0%-gate (the constructor pass below is skipped entirely).
        let required_missing = cache_record.required_missing;
        let required_pool_ids = cache_record.required_pool_ids;
        // Dense plan D2 — per-canonical-slot dense marker. `0` for a table-only
        // bundle (the apply-time 0%-gate: every dense branch below folds out and
        // the typed-write fast path stays eligible).
        let dense_mask = cache_record.dense_mask;
        let has_dense = dense_mask != 0;

        // ── Step 2.5 (W4): once-per-batch SBO-B2 + SBO-N debug invariants
        // SAFETY (SBO-N + SBO-B2 + B2):
        //   - SBO-N: pools Vec is push-only; warm-path debug_assert
        //     verifies non-decrease vs the install-time snapshot.
        //   - SBO-B2: `pool_ids` is canonical-sorted by `ComponentId.0`;
        //     verified at install AND once per batch here.
        //   - B::component_ids().len() == pool_ids.len() (cache invariant).
        #[cfg(debug_assertions)]
        {
            // SAFETY: we still hold the shared view through the `&mut self`
            //   apply window; no concurrent mutation can occur between the
            //   resolve in Step 2 and the writes in Step 5.
            let pools_len = unsafe { &*archetype_ptr }
                .component_pools()
                .pools_len();
            debug_assert!(
                cache_record.pools_len_at_install as usize <= pools_len,
                "SBO-N violation: pools Vec shrunk after cache install"
            );
            // Dense plan D2: a dense slot holds `DENSE_POOL_SENTINEL`
            // (`usize::MAX`), which is NOT canonical-sorted against the real table
            // pool ids — so the SBO-B2 sortedness check is meaningful only on a
            // table-only bundle (`dense_mask == 0`). The table-only path is
            // unchanged (the assert still runs); the canonical ORDER guarantee
            // for the dense path rides `B::component_ids()` (asserted by the
            // per-row emit-order check below), not `pool_ids`.
            if dense_mask == 0 {
                debug_assert!(
                    pool_ids.is_sorted_by_key(|p| p.0),
                    "SBO-B2 violation: pool_ids must be in canonical-sorted order"
                );
            }
            debug_assert_eq!(pool_ids.len(), B::component_ids().len());
        }

        // ── Step 2.6 (Phase 22.1 D-E): compacted data-column pool ids ──
        // Filter the canonical `pool_ids` down to the DATA (non-ZST)
        // columns, in the same canonical `ComponentId.0` order. The per-row
        // write loop walks only this compacted slice via
        // `for_each_data_component_bytes`, so a ZST tag column costs zero
        // per-row instructions (the dynamic-size memcpy is never reached).
        //
        // Alignment proof (D-E MINOR): the derive's data walk emits entries
        // sorted by `ComponentId` filtered by `size_of::<FieldTy>() == 0`;
        // this slice is the canonical `pool_ids` (also sorted by
        // `ComponentId`) filtered by `layout.size() == 0`. The predicate is
        // identical because `write_at_unchecked_initialized` copies
        // `component_layout.size()` bytes from the SAME component registry
        // the macro's `size_of` reflects. The per-row
        // `debug_assert_eq!(k, data_len)` pins the 1:1 correspondence.
        //
        // No per-row allocation: a fixed `MAX_BUNDLE_ARITY` stack array
        // (matching the sibling collectors) + a runtime length. Built once
        // per batch from `≤ N` pool-layout reads — negligible at 10k rows.
        let component_ids_static: &'static [crate::ecs::identifiers::primitives::ComponentId] =
            B::component_ids();
        let mut data_pool_ids: [InlandPoolId; MAX_BUNDLE_ARITY] =
            [InlandPoolId(0); MAX_BUNDLE_ARITY];
        // Compacted non-ZST component ids, parallel to `data_pool_ids`. Used by
        // (a) the per-row B2 emit-order debug_assert (debug builds) AND (b) the
        // Decision-4 typed-write `perm` build (all builds — `write_row_perm`
        // keys each field's data-column slot on its ComponentId, W3). Cheap:
        // ≤ MAX_BUNDLE_ARITY entries, built once per batch.
        let mut data_component_ids: [crate::ecs::identifiers::primitives::ComponentId;
            MAX_BUNDLE_ARITY] =
            [crate::ecs::identifiers::primitives::ComponentId(0); MAX_BUNDLE_ARITY];
        let mut data_len: usize = 0;
        {
            // SAFETY: the shared `&Archetype` borrow lives only within this
            //   block and is dropped before the `&mut Archetype` reborrow in
            //   Step 3 below; the apply window holds exclusive
            //   `&mut EcsMaster`, so no concurrent mutation occurs.
            let archetype_shared: &Archetype = unsafe { &*archetype_ptr };
            let pools = archetype_shared.component_pools();
            for (canonical_idx, &cid) in component_ids_static.iter().enumerate() {
                // Dense plan D2: a dense slot has NO per-archetype pool — skip it
                // from the TABLE data-column build (its `pool_ids[canonical_idx]`
                // is the `DENSE_POOL_SENTINEL`, never indexed). The dense bytes are
                // routed to `DenseStore` by the row loop's dense branch. For a
                // table-only bundle this test is always false (the 0%-gate).
                if dense_mask & (1u32 << canonical_idx) != 0 {
                    continue;
                }
                let layout_size = pools
                    .get_pool(cid)
                    .expect(
                        "invariant: cached archetype hosts every TABLE component in \
                         B::component_ids() (Bundle / ArchetypeMaster contract; dense \
                         ids skipped above)",
                    )
                    .component_layout()
                    .size();
                if layout_size != 0 {
                    debug_assert!(data_len < MAX_BUNDLE_ARITY);
                    data_pool_ids[data_len] = pool_ids[canonical_idx];
                    data_component_ids[data_len] = cid;
                    data_len += 1;
                }
            }
        }
        let data_pool_ids = &data_pool_ids[..data_len];
        let data_component_ids = &data_component_ids[..data_len];

        // ── Step 2.7 (Decision 4): per-batch typed-write `perm` build ──────
        // `perm[k]` maps declaration field `k` to its canonical data-column
        // slot (`PERM_SKIP` for a ZST field). Built ONCE per batch from the
        // canonical `data_component_ids` (W3 correct-by-construction). For a
        // `HAS_TYPED_WRITE == false` bundle (hand-written impls), the typed
        // arm is compiled out (`if const`) and this build is dead — gated so
        // `write_row_perm`'s `unreachable!` default is never reached.
        let mut perm: [u8; MAX_BUNDLE_ARITY] =
            [crate::ecs::core::bundle::BundleColumnPtrs::PERM_SKIP; MAX_BUNDLE_ARITY];
        // Dense plan D2: `write_row_perm` maps EVERY declaration field (incl. a
        // dense one) onto `data_component_ids`, but a dense field is NOT a
        // data-column id (it has no archetype pool, excluded from
        // `data_component_ids` above), so the perm build would not find it and
        // panic. The dense-bearing batch uses the byte path (gated by `!has_dense`
        // at the row loop), so the perm is dead for it — skip the build. For a
        // table-only typed-write bundle (`!has_dense`) the build runs exactly as
        // before (the 0%-gate).
        if const { B::HAS_TYPED_WRITE } && !has_dense {
            B::write_row_perm(data_component_ids, &mut perm);
        }

        // ── Step 3: ensure entity fast-store capacity (Phase 12.6) ─────
        // Replaces the SBO17b hard-panic guard. The apply path holds
        // `&mut EcsMaster`, so workers are not in flight per SCH7 — and
        // since Phase X.G the store's base is write-once anyway
        // (`InlandStore::ensure` = frontier commit; reallocation does not
        // exist), making the SEND5/SBO16 stable-address provision
        // structural.
        //
        // The `+ MAX_BATCH_HINT` overshoot is kept: it keeps `len` ahead of
        // the reserved-id window so a subsequent batch's worker-side id
        // RESERVATIONS stay below `len` (the bounds oracle), same as the
        // original Phase 12.5 SBO16 ergonomics.
        let start_id = self.start_entity.id().0;
        let end_id = start_id.checked_add(n).expect(
            "invariant SBO17b: enqueue cap-check ensures start + n cannot overflow usize",
        );
        world
            .entity_master_mut()
            .ensure_capacity(end_id + super::super::system::params::entity_counter::MAX_BATCH_HINT);

        // SAFETY (U1, U2, U14): write-capable slab provenance; apply
        //   window holds exclusive `&mut EcsMaster` so no other reader or
        //   writer of this archetype is in flight. The shared `&Archetype`
        //   borrow in Step 2 has already been dropped above.
        let archetype: &mut Archetype = unsafe { &mut *archetype_ptr };

        // ── Step 4: reserve pool capacity (I-N4 .expect) ───────────────
        archetype.reserve_capacity(n).expect(
            "SpawnBatchCommand: pool reserve ceiling (rows) exhausted — committed \
             capacity grows on demand (Phase X.I), so this fires only when the \
             archetype outgrows a pool's reserve_rows (aggregate-worker overshoot \
             or a logic bug in enqueue)",
        );
        let start_row = archetype.current_index;

        // ── Step 5: write rows (Phase 22.1 D-E filtered walk) ──────────
        // The data walk visits ONLY non-ZST columns, so `data_pool_ids[k]`
        // selects the destination pool and `k` advances once per data
        // column. ZST tag columns are never visited here — their bytes are
        // empty and their slots are committed + tick-stamped in Step 6 over
        // ALL pools (the `Added<Tag>` contract is preserved).
        //
        // For a 2-data-only bundle `data_pool_ids == pool_ids` (no ZST
        // filtered out), so this loop body is instruction-identical to the
        // pre-22.1 walk: same indexed load from a small stack array, same
        // deref chain, same memcpy.
        let mut iter = self.iter;
        // Dense plan D2 — the Decision-4 typed-write fast path writes EVERY field
        // (incl. a dense one) through `perm`-indexed `col_ptrs`, but a dense field
        // has NO archetype column, so it is ineligible when the bundle carries a
        // dense component. Fall back to the byte path for a dense-bearing batch
        // (the dense bytes are routed to `DenseStore` per row there). The const is
        // ANDed with the runtime `!has_dense`, so a table-only typed-write bundle
        // keeps the exact pre-dense fast path (the 0%-gate: `has_dense == false`).
        if const { B::HAS_TYPED_WRITE } && !has_dense {
            // ── Decision 4 typed path ──────────────────────────────────────
            // W2 single-provenance: resolve every data column's write base
            // ONCE, under a single `&mut` borrow of the pool bundle that is
            // ENDED before the row loop. The row loop then holds only the raw
            // `*mut u8` bases inside `col_ptrs` and NEVER re-borrows
            // `component_pools_mut()` (CONFIRM-2 / the 14a-F2/9.3c antidote).
            let mut col_ptrs = crate::ecs::core::bundle::BundleColumnPtrs::new();
            archetype
                .component_pools_mut()
                .resolve_column_ptrs(data_pool_ids, &perm, &mut col_ptrs);
            // The `&mut ComponentPoolBundle` borrow above has ended here; from
            // now on only `col_ptrs` (raw bases) is read in the loop.
            for i in 0..n {
                let row = start_row + i;
                let bundle = iter.next().expect(
                    "ExactSizeIterator contract: len() reported n, iter yielded < n",
                );
                // SAFETY (D4 — W1/W2/Q1/W3, see `Bundle::write_row_typed` doc):
                //   - `col_ptrs` bases were resolved once under the &mut that
                //     has ended; the row loop holds only raw bases and does not
                //     re-borrow the pools (W2 single provenance). `B`'s
                //     `I: ... + 'static` bound forbids `iter.next()` from
                //     capturing any world/archetype/pool borrow, and there is no
                //     TLS/DeferredEcsMaster/hook/observer re-entry in this
                //     command, so `next()` cannot invalidate the bases (W1).
                //   - `row < start_row + n <= committed_rows` after
                //     `reserve_capacity` (Q1, debug-asserted per store).
                //   - `perm` keys each field's slot on its ComponentId (W3,
                //     IDENTITY-asserted per store in debug).
                //   - Each slot is uninit (commit happens post-loop, B4); each
                //     field is relocated once via ManuallyDrop::take.
                unsafe {
                    bundle.write_row_typed(&col_ptrs, row);
                }
            }
        } else if !has_dense {
            // ── RETAINED byte path (verbatim; HAS_TYPED_WRITE == false) ─────
            // Dense plan D2: kept BYTE-IDENTICAL behind `!has_dense` so a
            // table-only hand-written-Bundle batch is unchanged (the 0%-gate).
            for i in 0..n {
                let row = start_row + i;
                let bundle = iter.next().expect(
                    "ExactSizeIterator contract: len() reported n, iter yielded < n",
                );
                let mut k = 0usize;
                bundle.for_each_data_component_bytes(|_component_id, bytes| {
                    debug_assert!(k < data_pool_ids.len());
                    debug_assert_eq!(
                        data_component_ids[k], _component_id,
                        "B2/SBO-B2 violation: bundle data-emit order mismatch"
                    );
                    let pool_idx = data_pool_ids[k];
                    // SAFETY (SBO13):
                    //   - `row < start_row + n` and `start_row + n <= the
                    //     pool's committed_rows` after `reserve_capacity`
                    //     succeeded (Phase X.I: Phase B grew every pool).
                    //   - `pool_idx.0 < pools.len()` because `data_pool_ids` is
                    //     a filtered subset of the canonical `pool_ids`
                    //     (SBO-B2), each entry copied straight from a valid
                    //     `pool_ids` slot.
                    //   - Pool is exclusively accessed via `&mut archetype`.
                    //   - `bytes.len()` matches `component_layout.size()` per
                    //     Bundle/macro contract (non-empty by construction).
                    unsafe {
                        archetype
                            .component_pools_mut()
                            .pool_at_unchecked_mut(pool_idx)
                            .write_at_unchecked_initialized(row, bytes);
                    }
                    k += 1;
                });
                // D-E alignment pin: the bundle's data walk emitted exactly the
                // non-ZST columns the per-batch filter counted.
                debug_assert_eq!(k, data_pool_ids.len());
            }
        } else {
            // ── Dense plan D2 — dense-bearing byte path (decision 4) ────────
            // The batch carries ≥1 dense component. Per emitted data component:
            //   * a DENSE component routes its bytes to `DenseStore::insert`
            //     (STORED, op-order-deterministic) — NO per-row hooks/observers
            //     fire (the documented bulk-no-hooks policy, identical to the
            //     table-in-batch policy: a user wanting per-spawn dense hooks uses
            //     `Commands::spawn`, not `spawn_batch` — no table/dense asymmetry);
            //   * a TABLE component takes the same `pool_at_unchecked_mut` write
            //     as the verbatim path, advancing `k`.
            // `archetype` (a `&mut *archetype_ptr` raw reborrow) and
            // `world.dense_registry` are DISJOINT fields, so the closure may hold
            // both: `archetype`'s provenance is the raw slab pointer (not a
            // `&mut world` borrow), and `dense_registry` is reached through the
            // separately-captured `&mut *world`.
            // Dense plan D4: snapshot the world tick before the `&mut world`
            // capture (the closure cannot re-borrow `world`); every batched dense
            // insert is stamped Added at this tick (the bulk-spawn frame's tick).
            let dense_current_tick = world.current_tick();
            let world_ref: &mut EcsMaster = world;
            for i in 0..n {
                let row = start_row + i;
                let bundle = iter.next().expect(
                    "ExactSizeIterator contract: len() reported n, iter yielded < n",
                );
                let mut k = 0usize;
                bundle.for_each_data_component_bytes(|component_id, bytes| {
                    if matches!(
                        crate::ecs::core::component::component_registry::storage_kind(component_id.0),
                        crate::ecs::core::component::component_registry::StorageKind::Dense
                    ) {
                        // Route to the global dense column. `start_id + i` is this
                        // row's entity id (the batch reserved `[start_id, end_id)`).
                        let entity_id =
                            crate::ecs::identifiers::primitives::EntityId(start_id + i);
                        let store = world_ref.dense_registry.store_mut(component_id);
                        store.insert(entity_id, bytes, dense_current_tick);
                        store.mark_arch_present(archetype_id);
                        // No `k += 1` — dense is not a table data column.
                        return;
                    }
                    debug_assert!(k < data_pool_ids.len());
                    debug_assert_eq!(
                        data_component_ids[k], component_id,
                        "B2/SBO-B2 violation: bundle data-emit order mismatch"
                    );
                    let pool_idx = data_pool_ids[k];
                    // SAFETY (SBO13): identical to the verbatim byte path —
                    //   `row < committed_rows` after `reserve_capacity`, `pool_idx`
                    //   is a filtered table-column slot, exclusive `&mut archetype`,
                    //   `bytes.len()` matches the registry layout.
                    unsafe {
                        archetype
                            .component_pools_mut()
                            .pool_at_unchecked_mut(pool_idx)
                            .write_at_unchecked_initialized(row, bytes);
                    }
                    k += 1;
                });
                // The dense-emitted components are NOT in `data_pool_ids`, so `k`
                // counts only the table data columns.
                debug_assert_eq!(k, data_pool_ids.len());
            }
        }

        // ── Step 5b: required-component constructor pass (Feature 1, D5) ──
        // CRITICAL UB FIX: `cold_register_bundle_archetype` resolves the
        // archetype with the required columns ALREADY present, and Step 6's
        // `commit_units_batch` / `fill_ticks_batch` commit + tick-stamp EVERY
        // pool — including the required columns the row loop never wrote. Without
        // this pass those `n` rows would be committed-but-uninitialized:
        // garbage reads + a `drop_in_place` on uninit at teardown (UB).
        //
        // For EACH row in `[start_row, start_row + n)` and EACH required column,
        // construct one value via its capture-free ctor directly into the
        // reserved-but-uncommitted slot. Step 6 then commits + ticks these
        // columns alongside the bundle's own (the batch commit walks all pools),
        // so no per-row commit/fill is done here (mirrors `SpawnAtCommand::apply`
        // Step 5b, looped over n).
        //
        // NOTE (pre-existing gap, NOT fixed here per task scope):
        // `SpawnBatchCommand::apply` fires NO on_add/on_insert hooks or observers
        // for ANY component — not even the bundle's own (there is no
        // flags/`trigger_on_add` block in this command, unlike
        // `SpawnAtCommand::apply` Step 8). Since the batch path fires for nobody,
        // the constructed required columns match the existing behaviour (silent).
        // The "spawn_batch fires no lifecycle hooks" gap is reported as a finding.
        //
        // 0%-gate: `required_missing` is empty for a require-free bundle, so the
        // outer `if` is skipped entirely and this pass costs zero.
        debug_assert_eq!(
            required_missing.len(),
            required_pool_ids.len(),
            "required_missing / required_pool_ids length mismatch",
        );
        if !required_missing.is_empty() {
            for i in 0..n {
                let row = start_row + i;
                for (entry, &pool_idx) in
                    required_missing.iter().zip(required_pool_ids.iter())
                {
                    // SAFETY (mirrors `SpawnAtCommand::apply` Step 5b; Feature 1 D5):
                    //   - `pool_idx.0 < pools.len()` — resolved at cache install
                    //     time against the same archetype (`resolve_required_missing`).
                    //   - `row < start_row + n <= committed_rows` after
                    //     `reserve_capacity(n)` (Phase X.I), and the required pool's
                    //     `len` is still `start_row` (never written by the row loop),
                    //     so `row >= len` for every `i` — the slot is uninit, so
                    //     `construct_at_uninitialized` runs no drop. Step 6's
                    //     `commit_units_batch(start_row, n)` then advances every
                    //     pool's `len` by `n` in lockstep (precondition
                    //     `start_row == len` still holds for the required pools).
                    //   - `&mut archetype` provides exclusive access; no concurrent
                    //     reader of this slot exists.
                    //   - `entry.ctor` writes exactly one value of the pool's
                    //     registered type (the registry paired the ctor with
                    //     `entry.component_id`, and `pool_idx` is that id's column).
                    unsafe {
                        archetype
                            .component_pools_mut()
                            .pool_at_unchecked_mut(pool_idx)
                            .construct_at_uninitialized(row, entry.ctor);
                    }
                }
            }
        }

        // ── Step 6: bulk-commit units + tick init ──────────────────────
        archetype
            .component_pools_mut()
            .commit_units_batch(start_row, n);
        archetype
            .component_pools_mut()
            .fill_ticks_batch(start_row, n, current_tick);

        // ── Step 7: archetype-level bookkeeping ────────────────────────
        // `Range<usize>::map(EntityId)` is `ExactSizeIterator`, so
        // `VmColumn::extend_exact` sizes ONE frontier commit for the whole
        // batch then streams the ids into address-stable slots (F1: no
        // realloc-memcpy spike). `EntityId` is `#[repr(transparent)]` over
        // `usize`, so the map closure compiles down to a no-op.
        archetype
            .entity_ids
            .extend_exact((start_id..start_id + n).map(EntityId));
        archetype.current_index = start_row + n;

        // ── Step 8: bulk-register entities ────────────────────────────
        world.entity_master.register_batch(
            EntityId(start_id),
            archetype_ptr,
            start_row as u32,
            n,
        );
    }
}

