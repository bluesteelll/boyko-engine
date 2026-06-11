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
use crate::ecs::identifiers::primitives::EntityId;

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
    ///    per batch in debug builds (cheap: ≤ MAX_BUNDLE_ARITY = 8 ops).
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
            debug_assert!(
                pool_ids.is_sorted_by_key(|p| p.0),
                "SBO-B2 violation: pool_ids must be in canonical-sorted order"
            );
            debug_assert_eq!(pool_ids.len(), B::component_ids().len());
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

        // ── Step 5: write rows ─────────────────────────────────────────
        let mut iter = self.iter;
        let component_ids_static: &'static [crate::ecs::identifiers::primitives::ComponentId] =
            B::component_ids();
        for i in 0..n {
            let row = start_row + i;
            let bundle = iter.next().expect(
                "ExactSizeIterator contract: len() reported n, iter yielded < n",
            );
            let mut canonical_idx = 0usize;
            bundle.for_each_component_bytes(|component_id, bytes| {
                debug_assert!(canonical_idx < pool_ids.len());
                debug_assert_eq!(
                    component_ids_static[canonical_idx], component_id,
                    "B2/SBO-B2 violation: bundle emit order mismatch"
                );
                let pool_idx = pool_ids[canonical_idx];
                // SAFETY (SBO13):
                //   - `row < start_row + n` and `start_row + n <= the
                //     pool's committed_rows` after `reserve_capacity`
                //     succeeded (Phase X.I: Phase B grew every pool).
                //   - `pool_idx.0 < pools.len()` by SBO-B2 (canonical match
                //     with `B::component_ids()`).
                //   - Pool is exclusively accessed via `&mut archetype`.
                //   - `bytes.len()` matches `component_layout.size()` per
                //     Bundle/macro contract.
                unsafe {
                    archetype
                        .component_pools_mut()
                        .pool_at_unchecked_mut(pool_idx)
                        .write_at_unchecked_initialized(row, bytes);
                }
                canonical_idx += 1;
            });
            debug_assert_eq!(canonical_idx, pool_ids.len());
        }

        // ── Step 6: bulk-commit units + tick init ──────────────────────
        archetype
            .component_pools_mut()
            .commit_units_batch(start_row, n);
        archetype
            .component_pools_mut()
            .fill_ticks_batch(start_row, n, current_tick);

        // ── Step 7: archetype-level bookkeeping ────────────────────────
        // `Range<usize>::map(EntityId)` is `TrustedLen + ExactSizeIterator`,
        // so `Vec::extend` fast-paths it to a single `reserve` + bulk
        // `ptr::copy_nonoverlapping`. `EntityId` is `#[repr(transparent)]`
        // over `usize`, so the map closure compiles down to a no-op.
        archetype
            .entity_ids
            .extend((start_id..start_id + n).map(EntityId));
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

