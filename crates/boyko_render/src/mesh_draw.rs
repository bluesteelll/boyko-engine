//! The ECS-native bucketed instance gather + per-mesh draw batches (mesh foundation
//! M3).
//!
//! This is the Principle-0 heart of M3: the instanced draw is driven by the
//! [`MeshHandle`](boyko_scene::render_caps::MeshHandle) +
//! [`InstanceModelCol`](crate::instance_model::InstanceModelCol) of SPAWNED ENTITIES
//! — read through an ECS [`Query`] — NOT by a test-built buffer. The gather buckets
//! every visible instance by its mesh id into ONE contiguous instance ring (so each
//! mesh draws ALL its instances in a single `vkCmdDrawIndexed`), and emits a
//! [`DrawBatch`] per non-empty mesh carrying that bucket's `base_instance` offset
//! into the ring.
//!
//! # The algorithm (Decision 7 — count → prefix-sum → scatter, sort INDICES not records)
//!
//! 1. **Count.** Walk the query once touching only the small `MeshHandle` key
//!    (cache-dense), incrementing `counts[mesh_id]`.
//! 2. **Prefix-sum.** `offsets[m] = Σ counts[0..m]` — `offsets[m]` is mesh `m`'s
//!    `base_instance` (its contiguous bucket's start in the ring). Emit a `DrawBatch`
//!    per non-empty mesh.
//! 3. **Scatter.** Walk the query a second time, writing each row's 48-byte
//!    [`InstanceModelCol`] into `ring[offsets[m] + cursors[m]++]` — contiguous per
//!    bucket, no overlap.
//!
//! Alloc-free after warmup: every lane is a
//! [`ScratchColumn<T>`](boyko_ecs::ecs::core::component::scratch::ScratchColumn) — the
//! same `ComponentPool`-backed, VM-native transient-scratch primitive the kernel's own
//! solver scratch uses, NOT `std::Vec` (Principle 0) — `clear()`ed + re-filled every
//! frame (the backing reservation persists). Each lane's ceiling is
//! [`pool_reserve_rows`](boyko_ecs::ecs::constants::pool_reserve_rows) of its element
//! size — the same VA-reservation-class (address-space-only, lazy-commit) ceiling every
//! other kernel column uses, so there is no fixed `MAX_MESHES`/instance-count cap in
//! practice. The scratch is a reused [`Resource`], NOT an ad-hoc buffer (the
//! [`UiRenderScratch`](crate::ui::UiRenderScratch) precedent, Principle 5).
//!
//! # The per-instance mesh-id lane (M3 → HW-RT, TLAS-readiness)
//!
//! Alongside the affine [`ring`](MeshRenderScratch::ring) the gather scatters a
//! PARALLEL [`mesh_ids`](MeshRenderScratch::mesh_ids) lane: `mesh_ids[i]` is ring
//! instance `i`'s `MeshHandle.0` — which is also its BLAS index (the mesh BLAS is
//! keyed by the same `Assets<MeshGpu>` handle). This makes the instance ring DIRECTLY
//! consumable by a future TLAS builder — instance `i` maps to (`ring[i]` = its 3×4
//! world affine, `mesh_ids[i]` = its BLAS) in O(1), with no need to reconstruct the
//! mapping by range-searching the per-mesh [`batches`](MeshRenderScratch::batches).
//! The lane is valid for DYNAMIC rows too (interpolation rewrites the affine on-GPU,
//! never the mesh identity). It is a host-side scratch column the acceleration-structure
//! builder reads; the RASTER draw does NOT read it (it reads mesh identity from each
//! batch's contiguous `base_instance` range), so the lane costs the raster path
//! nothing but one scatter store per instance.

use boyko_ecs::ecs::constants::pool_reserve_rows;
use boyko_ecs::ecs::core::asset::{Assets, register_asset_layout};
use boyko_ecs::ecs::core::component::scratch::ScratchColumn;
use boyko_ecs::ecs::core::iters::query::Query;
use boyko_ecs::ecs::core::iters::query::filter_enable::Enabled;
use boyko_ecs::ecs::core::system::{NonSendRes, ResMut};
#[cfg(feature = "hwrt")]
use boyko_ecs::ecs::core::system::Res;
use boyko_macros::Resource;
use boyko_rhi::enums::IndexType;
use boyko_scene::render_caps::{MeshHandle, RenderEnabled};
use bytemuck::Zeroable;

use crate::gpu_transform3d::GpuTransform3D;
use crate::instance_model::InstanceModelCol;
use crate::mesh::MeshGpu;
use crate::mesh_assets::MeshAssetsExt;

/// One per-mesh instanced draw (mesh foundation M3) — the consumer issues exactly ONE
/// `vkCmdDrawIndexed(index_count, instance_count, 0, 0, base_instance)` per batch
/// (Principle-1 one-draw-per-mesh).
///
/// `#[repr(C)]`, `Copy` — a small POD the recorder reads to bind the mesh's buffers
/// and place its instance range. The `base_instance` is this mesh's prefix-sum offset
/// into the shared instance ring (NONZERO for every mesh after the first — the C1
/// proof); the VS reads `instances[base_instance + SV_InstanceID]`.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DrawBatch {
    /// The mesh this batch draws (`MeshHandle.0`) — the consumer resolves it to GPU
    /// buffers via the world's `Assets<MeshGpu>` table
    /// ([`MeshAssetsExt`](crate::mesh_assets::MeshAssetsExt)).
    pub mesh_id: u32,
    /// The mesh's index count (`vkCmdDrawIndexed`'s `index_count`), copied from the
    /// asset table at gather time so the recorder reads it without a second lookup.
    pub index_count: u32,
    /// The mesh's bound index width (O3 mixed `Uint16`/`Uint32`), copied from the
    /// registry at gather time.
    pub index_type: IndexType,
    /// This mesh's bucket start in the shared instance ring — the
    /// `vkCmdDrawIndexed`'s `base_instance`. The prefix-sum of every prior non-empty
    /// mesh's instance count, so it is NONZERO for every mesh after the first.
    pub base_instance: u32,
    /// The number of instances of this mesh (`vkCmdDrawIndexed`'s `instance_count`) —
    /// the bucket's length, `ring[base_instance .. base_instance + instance_count]`.
    pub instance_count: u32,
}

/// The reused per-frame mesh-render scratch (Principle-0 storage — a [`Resource`],
/// NOT a side store; the [`UiRenderScratch`](crate::ui::UiRenderScratch) precedent).
///
/// Every lane is a [`ScratchColumn<T>`] — the same `ComponentPool`-backed,
/// address-stable, VM-native transient-scratch primitive the kernel's own solver
/// scratch uses — NOT a `std::Vec` (Principle 0: durable/bulk per-frame data lives
/// on the engine's own storage). Cleared-not-reallocated each frame (Principle 5):
/// a steady-state frame only `clear()`s + re-fills + scatters in place through each
/// lane's [`ScratchBuildView`](boyko_ecs::ecs::core::component::scratch::ScratchBuildView),
/// so there is ZERO steady-state heap allocation. Each lane's reservation ceiling is
/// [`pool_reserve_rows`] of its element size — the SAME VA-reservation-class ceiling
/// (address space only, lazy-commit, effectively unbounded in practice) every other
/// `ComponentPool`-backed kernel column uses, so this preserves the "no fixed
/// `MAX_MESHES`/instance-count ceiling" property without a bespoke growth scheme.
#[derive(Resource)]
pub struct MeshRenderScratch {
    /// `counts[m]` — the number of visible instances of mesh `m` (pass 1). Length ==
    /// the per-mesh-lane length (≥ mesh count); `clear()` + re-fill with zeros.
    counts: ScratchColumn<u32>,
    /// `offsets[m]` — mesh `m`'s `base_instance` (the prefix-sum of `counts`). Reused.
    offsets: ScratchColumn<u32>,
    /// `cursors[m]` — the scatter write-head within mesh `m`'s bucket (pass 2),
    /// `0..counts[m]`. A separate lane so `offsets` stays the immutable bucket base.
    cursors: ScratchColumn<u32>,
    /// The emitted [`DrawBatch`]es (one per non-empty mesh), in mesh-id order;
    /// `clear()` + sequential `push`, never a fresh allocation.
    pub batches: ScratchColumn<DrawBatch>,
    /// The UNIFIED contiguous instance ring (refined-B) — EVERY visible instance's
    /// 48-byte [`InstanceModelCol`], STATIC and interpolated alike, in one
    /// draw-ordered buffer. Static rows are CPU-scattered here at gather time; dynamic
    /// (interpolated) rows' slots are left as-scattered on the CPU (stale bytes,
    /// overwritten on-GPU) and filled by the interp compute pre-pass via
    /// [`pair_out_slot`](Self::pair_out_slot). The renderer uploads
    /// [`as_read_slice()`](ScratchColumn::as_read_slice) into ONE shared instance SSBO
    /// bound once for the whole batch list; on an interp frame the compute overwrites
    /// only the dynamic slots before the raster VS reads. `ring.len()` == the TOTAL
    /// drawable count. `clear()` + scatter, backing reservation persists.
    pub ring: ScratchColumn<InstanceModelCol>,
    /// The parallel per-instance MESH-ID (BLAS-index) lane (M3 → HW-RT): `mesh_ids[i]`
    /// is [`ring`](Self::ring) instance `i`'s `MeshHandle.0`, scattered in lock-step with
    /// `ring` (`mesh_ids.len() == ring.len()`, every slot written exactly once). Makes the
    /// instance ring directly TLAS-consumable — instance `i` → (`ring[i]` affine,
    /// `mesh_ids[i]` BLAS) — without range-searching [`batches`](Self::batches). Valid for
    /// dynamic rows (mesh identity is interpolation-invariant). Host-side only (the
    /// AS builder reads it; the raster draw does not). `clear()` + scatter, backing
    /// reservation persists.
    pub mesh_ids: ScratchColumn<u32>,
    /// The contiguous interpolation-PAIR ring (Pillar B B1) — the 96-byte
    /// [`GpuTransform3D`] of EVERY DYNAMIC (interpolated) instance, in gather order.
    /// `pair_ring.len()` == the dynamic instance count (NOT the total — static rows
    /// contribute no pair). The B2 interpolation compute pre-pass reads this slice as
    /// its `TransformPair` input SSBO. Populated by
    /// [`gather_mixed_into`](Self::gather_mixed_into); `clear()` + scatter, backing
    /// reservation persists.
    pub pair_ring: ScratchColumn<GpuTransform3D>,
    /// The parallel SoA lane to [`pair_ring`](Self::pair_ring): `pair_out_slot[d]` is
    /// dynamic instance `d`'s gather-assigned offset into the unified
    /// [`ring`](Self::ring) — where the interp compute must scatter its interpolated
    /// model column (the shader's `OutSlot` binding). A SEPARATE lane, NOT a widened
    /// 96-byte pair record, keeping the pair ring's dense std430 layout intact
    /// (Principle 0). `pair_out_slot.len()` == `pair_ring.len()` == the dynamic count.
    /// `clear()` + scatter, backing reservation persists.
    pub pair_out_slot: ScratchColumn<u32>,
    /// HW-RT Rung 3b: the PREVIOUS-frame instance ring — the 48-byte
    /// [`InstanceModelCol`] each drawable had LAST frame, scattered INDEX-ALIGNED with
    /// [`ring`](Self::ring) (`prev_ring[i]` is `ring[i]`'s prev-frame model). Filled from
    /// the entity's [`PrevInstanceModelCol`](crate::instance_model::PrevInstanceModelCol)
    /// dense column in the SAME mesh-bucketed scatter as `ring`; a drawable with NO
    /// prev column (`None`) falls back to its CURRENT row (camera-only motion, safe).
    /// The gbuffer MV VS reads it as `StructuredBuffer<InstanceModelCol> prev_instances`
    /// (binding 1) so a moving mesh's per-object motion vector is `cur_world −
    /// prev_world`. Written ONLY when the temporal denoiser is on (the raster MV variant);
    /// on every other frame it is filled but the prev-instance ring is bound by no set.
    /// `clear()` + scatter, backing reservation persists (like `ring`). Host-side only.
    #[cfg(feature = "hwrt")]
    pub prev_ring: ScratchColumn<InstanceModelCol>,
    /// HW-RT Rung 3b: the prev-ring scatter's write-head lane — a private twin of
    /// `cursors` reset to 0 so [`gather_prev_ring_into`](Self::gather_prev_ring_into) can
    /// re-derive each drawable's ring slot (`offsets[m] + prev_cursors[m]++`) IDENTICALLY
    /// to the `ring` scatter, without disturbing `cursors`. Reused across frames
    /// (`fit_len` re-zeros it), never a fresh allocation on the hot path.
    #[cfg(feature = "hwrt")]
    prev_cursors: ScratchColumn<u32>,
}

/// Constructs every lane fresh: registers ONE [`boyko_ecs`] `ComponentId` per
/// DISTINCT element type (memoized process-wide by [`register_asset_layout`] — no
/// `World`/registry handle needed here, mirroring the asset store's F1
/// `AssetBacking::register_layout` convention) and sizes each lane's backing
/// `ComponentPool` at [`pool_reserve_rows`] of its element size — the kernel's
/// standard VA-reservation-class ceiling (address space only, lazy-commit), so
/// `MeshRenderScratch::default()` stays a valid zero-argument constructor callable
/// from `insert_resource(MeshRenderScratch::default())` / test setup exactly as
/// before.
impl Default for MeshRenderScratch {
    fn default() -> Self {
        let u32_id = register_asset_layout::<u32>(None);
        let batch_id = register_asset_layout::<DrawBatch>(None);
        let model_id = register_asset_layout::<InstanceModelCol>(None);
        let pair_id = register_asset_layout::<GpuTransform3D>(None);

        let u32_rows = pool_reserve_rows(std::mem::size_of::<u32>());
        let batch_rows = pool_reserve_rows(std::mem::size_of::<DrawBatch>());
        let model_rows = pool_reserve_rows(std::mem::size_of::<InstanceModelCol>());
        let pair_rows = pool_reserve_rows(std::mem::size_of::<GpuTransform3D>());

        Self {
            counts: ScratchColumn::new(u32_id, u32_rows),
            offsets: ScratchColumn::new(u32_id, u32_rows),
            cursors: ScratchColumn::new(u32_id, u32_rows),
            batches: ScratchColumn::new(batch_id, batch_rows),
            ring: ScratchColumn::new(model_id, model_rows),
            mesh_ids: ScratchColumn::new(u32_id, u32_rows),
            pair_ring: ScratchColumn::new(pair_id, pair_rows),
            pair_out_slot: ScratchColumn::new(u32_id, u32_rows),
            #[cfg(feature = "hwrt")]
            prev_ring: ScratchColumn::new(model_id, model_rows),
            #[cfg(feature = "hwrt")]
            prev_cursors: ScratchColumn::new(u32_id, u32_rows),
        }
    }
}

/// Grows `col` to exactly `min_len` live elements (a fresh `clear()` + fill-push):
/// every slot in `[0, min_len)` holds `fill` until the caller's scatter overwrites
/// it. Unlike the old `Vec`-backed POW2-reserve dance, there is no separate
/// capacity-growth step to amortize — `col`'s backing `ComponentPool` already
/// reserves a [`pool_reserve_rows`]-class VA ceiling at construction, so growing
/// `len` within it is a cheap page-commit at worst (never a realloc/move; alloc-free
/// once the working set's pages are resident from a prior frame).
#[inline]
fn fit_len(col: &mut ScratchColumn<u32>, min_len: usize, fill: u32) {
    let mut view = col.build_view();
    view.clear();
    for _ in 0..min_len {
        view.push(fill);
    }
}

impl MeshRenderScratch {
    /// The number of distinct meshes with at least one visible instance this frame —
    /// `batches.len()` after a [`gather_mixed_into`](Self::gather_mixed_into). The Principle-1
    /// one-draw-per-mesh count.
    #[inline]
    pub fn batch_count(&self) -> usize {
        self.batches.len()
    }

    /// The total number of scattered instances (== the ring length) — `Σ
    /// batch.instance_count` across every batch. Includes BOTH static and
    /// interpolated instances (refined-B — one unified ring).
    #[inline]
    pub fn instance_count(&self) -> usize {
        self.ring.len()
    }

    /// The number of DYNAMIC (interpolated) instances this frame — the interp compute
    /// dispatch bound (`ceil(dynamic_count / LOCAL_SIZE_X)` groups) and the `pair_ring`
    /// / `pair_out_slot` length. `0` on a pure-static scene (interp OFF — no dispatch,
    /// byte-identical to R3).
    #[inline]
    pub fn dynamic_count(&self) -> usize {
        self.pair_ring.len()
    }

    /// The UNIFIED gather core (refined-B, Decision 7): ONE count → prefix-sum →
    /// scatter over ALL drawables — static and interpolated alike — into ONE
    /// draw-ordered output [`ring`](Self::ring), recording each interpolated row's
    /// 96-byte pair + its ring offset into the parallel
    /// [`pair_ring`](Self::pair_ring) / [`pair_out_slot`](Self::pair_out_slot) lanes.
    ///
    /// This REPLACES the old two-gather split (`gather_into` affine + `gather_pairs_into`
    /// pair), which each cleared+owned the shared `batches` over DISJOINT populations —
    /// a last-writer-wins race that dropped drawables (the R5 review P0). One gather over
    /// all rows produces ONE batch list into ONE ring, so no population can clobber
    /// another's batches.
    ///
    /// `mesh_count` is the registry's mesh count (sizes the per-mesh lanes, O2); `meta`
    /// resolves a mesh id to its `(index_count, index_type)` for the emitted batch, or
    /// `None` if the mesh no longer resolves (asset-streaming plan F6 FIX-2/FIX-C1: a
    /// carrier whose mesh retired THIS frame, between `validate_asset_refs`'s
    /// best-effort disable and this gather). A non-resolvable mesh's instances are
    /// EXCLUDED from the ring/`mesh_ids`/prev-ring BY CONSTRUCTION (not merely
    /// un-batched — `bucket_lanes_mixed` zeroes `counts[m]` for such a mesh, so the
    /// scatter's `counts[m] == 0` skip covers it too): the hwrt TLAS packer
    /// must never see a retired mesh's id in `mesh_ids`, so `count == ring.len()`
    /// holds exactly, with no tail drop and no freed-BLAS read; `iter_input` is an
    /// ITERATOR FACTORY the gather invokes TWICE (once to count, once
    /// to scatter) — each call returns a FRESH iterator over the same
    /// `(mesh_id, &InstanceModelCol, Option<&GpuTransform3D>)` source. The `Option` keys
    /// the row's kind: `None` ⇒ STATIC (the affine is real, CPU-scattered into `ring`),
    /// `Some(pair)` ⇒ DYNAMIC (the affine is a placeholder — the interp compute
    /// overwrites this ring slot on-GPU — and the pair + its assigned ring slot are
    /// recorded for the compute dispatch).
    ///
    /// The factory (`Fn() -> I`, not a re-iteration `&mut dyn FnMut` callback) keeps both
    /// passes FULLY MONOMORPHIC — zero virtual dispatch on the per-instance hot path
    /// (P-002/P4). An ECS [`Query`] iterator is not `Clone`, but it does not need to be:
    /// `Query::iter` borrows `&self`, so the factory simply re-runs `q.iter()` per pass
    /// (the system wrapper [`gather_mesh_draws`] passes
    /// `|| q.iter().map(|(h, c, g)| (h.0, c, g))`; a unit test passes a slice map). The
    /// two iterators observe the SAME rows in the SAME order (the gather is over row
    /// VALUES — mesh id + affine + pair — plus a stable per-mesh cursor, not the global
    /// row order), so the second pass's `offsets[m] + cursors[m]` assigns each row the
    /// SAME slot the count pass reserved for it.
    ///
    /// After the call: `batches` holds one [`DrawBatch`] per non-empty, resolvable
    /// mesh (mesh-id order, correct prefix-sum `base_instance`s); `ring` holds ONLY
    /// resolvable meshes' instances, contiguously (`ring.len() == Σ instance_count`
    /// over resolvable meshes — a non-resolvable mesh's instances are excluded
    /// entirely, not merely un-batched, per FIX-C1 above); `pair_ring` /
    /// `pair_out_slot` hold the dynamic rows' pairs + ring slots (`len() == the
    /// dynamic count`, in gather order, non-resolvable rows excluded the same way).
    ///
    /// `debug_assert!`s catch an out-of-range `mesh_id` (a gather over a handle the
    /// registry never minted — a bundle/asset-binding bug) and pin the SoA-lane
    /// invariants (`pair_ring.len() == pair_out_slot.len()`; every recorded slot is in
    /// range of the ring).
    pub fn gather_mixed_into<'a, M, F, I>(&mut self, mesh_count: usize, meta: M, iter_input: F)
    where
        M: FnMut(u32) -> Option<(u32, IndexType)>,
        F: Fn() -> I,
        I: Iterator<Item = (u32, &'a InstanceModelCol, Option<&'a GpuTransform3D>)>,
    {
        // Shared count → prefix-sum → batch-emit over the lanes; `bucket_lanes` touches
        // only the small `mesh_id` key (the record tuple is never read on pass 1).
        let total = self.bucket_lanes_mixed(mesh_count, meta, &iter_input);

        // Grow `ring`/`mesh_ids` to exactly `total` live elements: a clear + fill-push
        // loop, byte-equivalent to the old `Vec::resize(total, fill)` — every slot in
        // `[0, total)` is written here, then overwritten exactly once by the scatter
        // below (no slot is ever READ between the two writes), so the fill value is
        // never observed. `ScratchColumn` has no bulk "grow to N" op; a push-loop over
        // its already-reserved (`pool_reserve_rows`-class) backing is the equivalent —
        // never a realloc, at worst a page-commit already amortized from a prior frame.
        {
            let mut ring_view = self.ring.build_view();
            ring_view.clear();
            for _ in 0..total {
                ring_view.push(InstanceModelCol::zeroed());
            }
        }
        // The per-instance mesh-id lane is scattered in lock-step with `ring` (every slot
        // written once, so the `0` fill is fully overwritten).
        {
            let mut mesh_ids_view = self.mesh_ids.build_view();
            mesh_ids_view.clear();
            for _ in 0..total {
                mesh_ids_view.push(0u32);
            }
        }
        // The pair lanes are re-filled by `push` (their length is the dynamic count,
        // not `total`); `clear()` keeps the backing reservation (Principle 5).
        self.pair_ring.build_view().clear();
        self.pair_out_slot.build_view().clear();
        {
            // Disjoint field-projection borrows off `&mut self` / `&self` — each view
            // borrows only its own field, so all seven coexist (the same discipline the
            // prior `mem::take` dance achieved, without needing `ScratchColumn: Default`).
            let mut ring_view = self.ring.build_view();
            let ring = ring_view.as_mut_slice();
            let mut mesh_ids_view = self.mesh_ids.build_view();
            let mesh_ids = mesh_ids_view.as_mut_slice();
            let mut pair_ring_view = self.pair_ring.build_view();
            let mut pair_out_slot_view = self.pair_out_slot.build_view();
            let offsets = self.offsets.as_read_slice();
            let mut cursors_view = self.cursors.build_view();
            let cursors = cursors_view.as_mut_slice();
            let counts = self.counts.as_read_slice();
            for (mesh_id, col, maybe_pair) in iter_input() {
                let m = mesh_id as usize;
                // FIX-C1 (asset-streaming plan F6): a non-resolvable mesh's instances
                // are EXCLUDED from the ring/`mesh_ids` here — not scattered into a
                // reserved-but-unused slot. `bucket_lanes_mixed` already zeroed this
                // mesh's count/offset span (`counts[m] == 0` iff the mesh had no
                // instances OR its `meta` resolved to `None` this gather — the two
                // are indistinguishable here, and don't need to be: both mean "skip"),
                // so `total` (== `ring.len()`, sized above) already excludes them;
                // this skip is what keeps a retired mesh's id out of `mesh_ids`
                // entirely, so the hwrt TLAS packer (`BlasAddr[MeshIds[i]]`) never
                // reads a freed BLAS device address.
                if counts[m] == 0 {
                    continue;
                }
                let slot = offsets[m] + cursors[m];
                cursors[m] += 1;
                // STATIC rows carry the real model column; DYNAMIC rows carry a
                // placeholder (the interp compute overwrites `ring[slot]` on-GPU). Either
                // way the CPU writes the whole ring (the data-race note: static slots are
                // never in `pair_out_slot`, so never GPU-touched — no conflict).
                ring[slot as usize] = *col;
                // The BLAS-id lane: the mesh identity is the same whether the row is static
                // or interpolated (interpolation touches only the affine), so it is written
                // unconditionally for every slot (M3 → HW-RT TLAS-readiness).
                mesh_ids[slot as usize] = mesh_id;
                if let Some(pair) = maybe_pair {
                    pair_ring_view.push(*pair);
                    pair_out_slot_view.push(slot);
                }
            }
        }
        debug_assert_eq!(
            self.ring.len(),
            total as usize,
            "invariant: the unified ring holds exactly Σ instance_count instances"
        );
        debug_assert_eq!(
            self.mesh_ids.len(),
            self.ring.len(),
            "invariant: the per-instance mesh-id lane is parallel to the ring (one id per instance)"
        );
        debug_assert_eq!(
            self.pair_ring.len(),
            self.pair_out_slot.len(),
            "invariant: the pair ring and its out-slot lane are parallel (one entry per dynamic row)"
        );
        debug_assert!(
            self.pair_out_slot
                .as_read_slice()
                .iter()
                .all(|&s| (s as usize) < self.ring.len()),
            "invariant: every dynamic out-slot indexes the unified ring in range"
        );
    }

    /// The count → resolve → prefix-sum → batch-emit core of the unified gather
    /// (Decision 7; asset-streaming plan F6 FIX-C1 adds the resolve step).
    ///
    /// Fills `counts` (pass 1); then, per mesh (pass 2, AT MOST `mesh_count` calls
    /// to `meta`, NEVER per-instance): resolves `meta(m)` for every mesh with a
    /// non-zero count, and — on `None` — ZEROES `counts[m]` BEFORE the
    /// prefix-sum runs, so a non-resolvable mesh contributes NOTHING to
    /// `offsets`/`running` (its instances are excluded from the ring by
    /// construction, not merely un-batched — see
    /// [`gather_mixed_into`](Self::gather_mixed_into)'s scatter, which consults
    /// `counts[m] == 0` per instance: the zeroing here IS the resolvability
    /// signal, so no separate lane is needed). Fills `offsets` (each mesh's
    /// `base_instance`), zeroed `cursors` (the scatter write-heads the caller
    /// advances), and `batches` (one [`DrawBatch`] per resolvable non-empty mesh,
    /// mesh-id order). Returns `Σ instance_count` over RESOLVABLE meshes only —
    /// the ring length the caller sizes its scatter to.
    ///
    /// `iter_input` is invoked ONCE here (the count pass); the caller invokes it a
    /// second time for the scatter. Only the small `mesh_id` key is touched — neither
    /// the affine nor the `Option` pair is read on pass 1. The resolve pass is
    /// O(mesh_count), not O(instance_count) — `meta` is never called per-instance,
    /// keeping the per-instance scatter a plain array index (`counts[m] == 0`).
    fn bucket_lanes_mixed<'a, M, F, I>(
        &mut self,
        mesh_count: usize,
        mut meta: M,
        iter_input: &F,
    ) -> u32
    where
        M: FnMut(u32) -> Option<(u32, IndexType)>,
        F: Fn() -> I,
        I: Iterator<Item = (u32, &'a InstanceModelCol, Option<&'a GpuTransform3D>)>,
    {
        // --- Pass 1: count per mesh (touches only the small MeshHandle key). ---
        fit_len(&mut self.counts, mesh_count, 0);
        {
            let mut counts_view = self.counts.build_view();
            let counts = counts_view.as_mut_slice();
            for (mesh_id, _col, _pair) in iter_input() {
                debug_assert!(
                    (mesh_id as usize) < mesh_count,
                    "invariant: a gathered mesh_id is in range of the registry"
                );
                counts[mesh_id as usize] += 1;
            }
        }

        // --- Resolve + prefix-sum: offsets[m] = Σ counts[0..m] = mesh m's
        // base_instance (over RESOLVABLE meshes only). `meta` is called ONCE per
        // non-empty mesh (never per-instance, asset-streaming plan F6 FIX-C1) —
        // `None` zeroes `counts[m]` BEFORE it feeds `running`/`offsets`, so a
        // non-resolvable mesh's instances are excluded from the ring by
        // construction (the zeroed `counts[m]` IS the scatter's per-instance
        // exclusion signal — `counts[m] == 0`, no separate lane needed). Emits a
        // DrawBatch per resolvable non-empty mesh (in mesh-id order) — the SAME
        // `meta` call drives both the batch emission and the exclusion, so there
        // is exactly one skip mechanism.
        fit_len(&mut self.offsets, mesh_count, 0);
        fit_len(&mut self.cursors, mesh_count, 0);
        self.batches.build_view().clear();
        let mut running: u32 = 0;
        // Disjoint field-projection borrows: `offsets`/`counts` (both `ScratchColumn<u32>`)
        // and `batches` (`ScratchColumn<DrawBatch>`) are three DISTINCT fields of `self`, so
        // their views coexist for the loop's duration exactly as the prior direct
        // `self.offsets[m]` / `self.counts[m]` / `self.batches.push` field accesses did.
        let mut offsets_view = self.offsets.build_view();
        let offsets = offsets_view.as_mut_slice();
        let mut counts_view = self.counts.build_view();
        let counts = counts_view.as_mut_slice();
        let mut batches_view = self.batches.build_view();
        for m in 0..mesh_count {
            offsets[m] = running;
            // A mesh with zero instances this frame needs no `meta` call — its
            // resolvability is moot (nothing would be excluded either way).
            let resolved = if counts[m] != 0 { meta(m as u32) } else { None };
            if resolved.is_none() {
                counts[m] = 0;
            }
            let c = counts[m];
            if let Some((index_count, index_type)) = resolved {
                batches_view.push(DrawBatch {
                    mesh_id: m as u32,
                    index_count,
                    index_type,
                    base_instance: running,
                    instance_count: c,
                });
            }
            running += c;
        }
        running
    }

    /// HW-RT Rung 3b: scatters the PREVIOUS-frame instance ring INDEX-ALIGNED with
    /// [`ring`](Self::ring), re-using the offsets [`gather_mixed_into`](Self::gather_mixed_into)
    /// just computed. Call IMMEDIATELY after `gather_mixed_into` on the SAME frame, with a
    /// factory (`Fn() -> J`) that yields the SAME `(mesh_id, ...)` rows in the SAME order as
    /// the affine gather's factory (the system passes `q.iter()` over the identical query),
    /// each carrying `Option<&PrevInstanceModelCol>` — `Some(prev)` for a row with a prev
    /// column, `None` for a row without one.
    ///
    /// # Index-alignment guarantee
    ///
    /// The slot arithmetic is `offsets[m] + prev_cursors[m]++` — the SAME `offsets` (this
    /// frame's prefix-sum) and the SAME per-mesh cursor advance the `ring` scatter used,
    /// over the SAME query iteration order, INCLUDING the SAME `counts[m] == 0`
    /// per-instance skip (asset-streaming plan F6 FIX-C1) — a row whose mesh `gather_mixed_into`
    /// excluded from `ring` is excluded here too. So `prev_ring[slot]` lands in the SAME
    /// slot `ring[slot]` did for that drawable — guaranteed by construction, not by luck. A
    /// `prev_cursors` lane (reset to 0 here) keeps `cursors` untouched.
    ///
    /// # Fallback (camera-only motion)
    ///
    /// A row with NO prev column (`None`) writes its CURRENT model (from `curr`) into
    /// `prev_ring[slot]`, so `prev_world == cur_world` for that pixel — the motion vector
    /// carries only the CAMERA's inter-frame delta (safe: no per-object ghost, the
    /// disocclusion-clean seed). `prev_ring.len() == ring.len()`, every slot written once.
    ///
    /// `curr` and `prev` are read as raw `[[f32; 4]; 3]` rows: the caller supplies the
    /// current [`InstanceModelCol`] (for the fallback) and the optional prev column's rows
    /// (both are the same 48-byte layout).
    #[cfg(feature = "hwrt")]
    pub fn gather_prev_ring_into<'a, F, J>(&mut self, iter_input: F)
    where
        F: Fn() -> J,
        J: Iterator<
            Item = (
                u32,
                &'a InstanceModelCol,
                Option<&'a crate::instance_model::PrevInstanceModelCol>,
            ),
        >,
    {
        let total = self.ring.len();
        // A fresh cursor lane sized to the current mesh-lane length, reset to 0 (reuses the
        // backing reservation — no per-frame alloc). `offsets.len()` is the mesh count the
        // affine gather just fitted.
        fit_len(&mut self.prev_cursors, self.offsets.len(), 0);

        // Grow `prev_ring` to exactly `total` — same clear + fill-push equivalence as
        // `gather_mixed_into`'s `ring` grow (every slot is written once here, then
        // overwritten exactly once by the scatter below).
        {
            let mut prev_ring_view = self.prev_ring.build_view();
            prev_ring_view.clear();
            for _ in 0..total {
                prev_ring_view.push(InstanceModelCol::zeroed());
            }
        }
        {
            let mut prev_ring_view = self.prev_ring.build_view();
            let prev_ring = prev_ring_view.as_mut_slice();
            let offsets = self.offsets.as_read_slice();
            let mut cursors_view = self.prev_cursors.build_view();
            let cursors = cursors_view.as_mut_slice();
            let counts = self.counts.as_read_slice();
            for (mesh_id, curr, maybe_prev) in iter_input() {
                let m = mesh_id as usize;
                // FIX-C1 (asset-streaming plan F6): mirror `gather_mixed_into`'s skip
                // EXACTLY — a non-resolvable mesh's instances never occupied a `ring`
                // slot, so they must not occupy a `prev_ring` slot either. The
                // index-alignment guarantee this fn depends on requires visiting the
                // SAME rows, in the SAME order, with the SAME skip as the affine gather.
                if counts[m] == 0 {
                    continue;
                }
                let slot = (offsets[m] + cursors[m]) as usize;
                cursors[m] += 1;
                // A row with a prev column carries LAST frame's affine; a row without one
                // falls back to its CURRENT affine (camera-only motion — no per-object ghost).
                prev_ring[slot] = match maybe_prev {
                    Some(prev) => InstanceModelCol { rows: prev.rows },
                    None => *curr,
                };
            }
        }
        debug_assert_eq!(
            self.prev_ring.len(),
            self.ring.len(),
            "invariant: the prev-instance ring is index-aligned with the current ring"
        );
    }
}

/// The ECS-native M3 gather SYSTEM: buckets every visible
/// `(MeshHandle, InstanceModelCol)` entity into per-mesh [`DrawBatch`]es + the shared
/// instance [`ring`](MeshRenderScratch::ring), reusing the [`MeshRenderScratch`]
/// resource (Principle 0 — instances from spawned entities via the query, not an
/// ad-hoc buffer).
///
/// The query is filtered on `Enabled<RenderEnabled>` (the `Visibility::Hidden` gate),
/// so a hidden row never enters a bucket. The world's [`Assets<MeshGpu>`] (a `NonSend`
/// resource) supplies the mesh count (sizes the lanes, O2) + each batch's `(index_count,
/// index_type)`.
///
/// # Static + interpolated in ONE gather (refined-B — the R5 review P0 fix)
///
/// The query's `Option<&GpuTransform3D>` term keys each row's kind WITHOUT dropping it:
/// a STATIC drawable (no interpolation pair) yields `None` and its real
/// [`InstanceModelCol`] is CPU-scattered into the unified [`ring`](MeshRenderScratch::ring);
/// an INTERPOLATED drawable yields `Some(pair)`, its 96-byte pair + its ring slot are
/// recorded into the [`pair_ring`](MeshRenderScratch::pair_ring) /
/// [`pair_out_slot`](MeshRenderScratch::pair_out_slot) lanes for the interp compute, and
/// its ring slot is left as placeholder bytes (the compute overwrites it on-GPU). ONE
/// gather over ALL drawables ⇒ ONE batch list ⇒ no population can clobber another's
/// batches (the last-writer-wins drop the old two-gather split caused). `Option<&Dense>`
/// yields `None` on per-row dense absence (kernel W1), and the gather uses `iter()` only,
/// never the dense `.get()` fast path — so no dense null-deref (follow-up #14).
///
/// # 0%-gate
///
/// A world with no `InstanceModelCol` column yields zero matching rows, so the gather
/// emits zero batches + an empty ring — the recorder then takes the legacy
/// (empty-slice) draw, byte-identical to the pre-M3 stream. A world with `InstanceModelCol`
/// rows but NO `GpuTransform3D` (a pure-static scene, e.g. room.rs) takes the `None`
/// branch for every row: `pair_ring` / `pair_out_slot` stay empty, `ring` is scattered
/// exactly as the pre-R5 affine gather did, and `dynamic_count() == 0` ⇒ no interp
/// dispatch ⇒ byte-identical to R3.
///
/// # Two passes over the query
///
/// The Decision-7 count + scatter each iterate the query once. The query is
/// re-iterable (`Query::iter` borrows `&self`-style state), so the gather passes an
/// iterator FACTORY (`|| q.iter().map(..)`) that is re-run per pass — both passes read
/// the SAME rows in the SAME order, fully monomorphically (no per-instance virtual
/// dispatch), so the second pass assigns each row the SAME slot the count reserved.
///
/// # HW-RT Rung 3b — the prev-instance ring (temporal motion vectors)
///
/// Under `feature = "hwrt"` the query gains an `Option<&PrevInstanceModelCol>` term (an
/// `Option`, so it never restricts archetype matching — a scene without the column yields
/// `None` for every row). Immediately after the unified gather the system re-scatters that
/// prev column INDEX-ALIGNED with the ring via
/// [`gather_prev_ring_into`](MeshRenderScratch::gather_prev_ring_into) (reusing the SAME
/// offsets over the SAME query order — alignment guaranteed by construction). A row without
/// a prev column falls back to its current affine (camera-only motion). The prev ring is
/// bound by the gbuffer MV pipeline only when the temporal denoiser is on; otherwise it is
/// filled but read by no set (the byte-identical OFF path). A `not(hwrt)` build compiles the
/// pre-Rung-3b system verbatim.
#[cfg(not(feature = "hwrt"))]
#[allow(clippy::needless_pass_by_value)]
pub fn gather_mesh_draws(
    q: Query<
        (&MeshHandle, &InstanceModelCol, Option<&GpuTransform3D>),
        Enabled<RenderEnabled>,
    >,
    mesh_assets: NonSendRes<Assets<MeshGpu>>,
    mut scratch: ResMut<MeshRenderScratch>,
) {
    // asset-streaming plan F5: `high_water()`, not `len()` — a live `MeshHandle.0`
    // can exceed the live COUNT once a hole exists (a freed-then-not-yet-reused
    // slot); `high_water()` is the true index ceiling the bucket lanes must size to.
    let mesh_count = mesh_assets.high_water();
    scratch.gather_mixed_into(
        mesh_count,
        // INVARIANT (asset-streaming plan F6 FIX-2): never dereference a non-Loaded
        // slot. `try_get` + `None` lets `gather_mixed_into` skip ONLY this bucket's
        // batch (see its doc) — a graceful, construction-guaranteed skip that does
        // not depend on `validate_asset_refs` (a best-effort net) having caught this
        // mesh's retire in time.
        |mesh_id| {
            let m = mesh_assets.try_get(MeshHandle(mesh_id))?;
            Some((m.index_count, m.index_type))
        },
        // slot resolved by index; staleness is caught by validate_asset_refs earlier this frame (apply→validate→gather)
        || q.iter().map(|(h, col, pair)| (h.0, col, pair)),
    );
}

/// The HW-RT Rung 3b variant of [`gather_mesh_draws`] (see that fn's docs): identical
/// bucketed gather PLUS the prev-instance ring scatter. The affine gather reads only
/// `(MeshHandle, InstanceModelCol, GpuTransform3D)` — the prev term is read solely by the
/// second, index-aligned scatter, so the ring / mesh-id / pair lanes are byte-identical to
/// the non-hwrt gather (the OFF path never diverges).
#[cfg(feature = "hwrt")]
#[allow(clippy::needless_pass_by_value)]
// `clippy::type_complexity`: this IS the ECS query contract — the 4-term tuple + the
// `Enabled<RenderEnabled>` filter is the system's `SystemParam` signature, which the scheduler
// reads to derive access. Factoring it into a `type` alias would only hide the access set from a
// reader (and the alias could not carry the elided lifetime cleanly). The 3-term non-hwrt variant
// stays under the threshold; the 4th (`Option<&PrevInstanceModelCol>`) tips it only under hwrt.
#[allow(clippy::type_complexity)]
pub fn gather_mesh_draws(
    q: Query<
        (
            &MeshHandle,
            &InstanceModelCol,
            Option<&GpuTransform3D>,
            Option<&crate::instance_model::PrevInstanceModelCol>,
        ),
        Enabled<RenderEnabled>,
    >,
    mesh_assets: NonSendRes<Assets<MeshGpu>>,
    mut scratch: ResMut<MeshRenderScratch>,
    denoise: Res<crate::ShadowDenoiseConfig>,
) {
    // asset-streaming plan F5: `high_water()`, not `len()` — see the non-hwrt
    // variant's comment above.
    let mesh_count = mesh_assets.high_water();
    scratch.gather_mixed_into(
        mesh_count,
        // INVARIANT (asset-streaming plan F6 FIX-2): never dereference a non-Loaded
        // slot — see the non-hwrt variant's comment above.
        |mesh_id| {
            let m = mesh_assets.try_get(MeshHandle(mesh_id))?;
            Some((m.index_count, m.index_type))
        },
        // slot resolved by index; staleness is caught by validate_asset_refs earlier this frame (apply→validate→gather)
        || q.iter().map(|(h, col, pair, _prev)| (h.0, col, pair)),
    );
    // The prev-instance ring is bound ONLY by the temporal-MV raster pipeline, so the O(N)
    // prev-scatter is pure waste on a temporal-OFF frame (the default `mode ∈ {None, Spatial}`).
    // Gate it on the temporal predicate so an OFF hwrt frame pays ZERO — Principle 1 (no OFF-path
    // overhead), the owner's static-where-dynamic-costs directive. The config is always present
    // here (`EnginePlugins` adds `ShadowDenoisePlugin` alongside this system). When ON, the
    // re-scatter reuses the SAME offsets over the SAME query order ⇒ index-aligned with `ring`.
    if denoise.temporal_enabled() {
        // slot resolved by index; staleness is caught by validate_asset_refs earlier this frame (apply→validate→gather)
        scratch.gather_prev_ring_into(|| q.iter().map(|(h, col, _pair, prev)| (h.0, col, prev)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gpu_transform3d::TrsPacked;

    /// A distinct-per-instance affine so a misplaced scatter is detectable by value.
    /// The translation encodes `(mesh_id, ordinal)` so the test can prove WHICH
    /// instance landed in WHICH ring slot.
    fn affine(mesh_id: u32, ordinal: u32) -> InstanceModelCol {
        let t = [mesh_id as f32, ordinal as f32, 0.0];
        InstanceModelCol {
            rows: [
                [1.0, 0.0, 0.0, t[0]],
                [0.0, 1.0, 0.0, t[1]],
                [0.0, 0.0, 1.0, t[2]],
            ],
        }
    }

    /// A fake registry `meta`: every mesh `m` has `index_count = 6 * (m + 1)` and
    /// alternating index width (mesh 0 → Uint16, mesh 1 → Uint32, …) to exercise the
    /// O3 mixed-width batch carry. Always `Some` — these existing suites exercise the
    /// "every mesh resolves" path; the F6 FIX-2 `None` (skipped-bucket) path is a
    /// dedicated new suite (see the tester's list).
    fn meta(mesh_id: u32) -> Option<(u32, IndexType)> {
        let width = if mesh_id.is_multiple_of(2) {
            IndexType::Uint16
        } else {
            IndexType::Uint32
        };
        Some((6 * (mesh_id + 1), width))
    }

    /// A distinct-per-instance interpolation pair so a misplaced scatter is
    /// detectable by value: `curr.pos` encodes `(mesh_id, ordinal)` and `prev.pos`
    /// encodes them shifted, so a swapped prev/curr or a wrong slot is caught.
    fn pair(mesh_id: u32, ordinal: u32) -> GpuTransform3D {
        let trs = |bias: f32| TrsPacked {
            pos: [mesh_id as f32 + bias, ordinal as f32 + bias, bias, 0.0],
            rot: [0.0, 0.0, 0.0, 1.0],
            scale: [1.0, 1.0, 1.0, 0.0],
        };
        GpuTransform3D {
            prev: trs(-100.0),
            curr: trs(0.0),
        }
    }

    /// A STATIC input row (no interpolation pair) for the unified gather.
    #[inline]
    fn stat(mesh_id: u32, col: &InstanceModelCol) -> (u32, &InstanceModelCol, Option<&GpuTransform3D>) {
        (mesh_id, col, None)
    }

    /// The C1 nonzero-`base_instance` proof + the Principle-1 one-draw-per-mesh guard,
    /// CPU-side, over the UNIFIED gather with ALL-STATIC rows: two meshes (A=mesh 0 with
    /// 3 instances, B=mesh 1 with 2 instances) bucket into one batch each; mesh B's
    /// `base_instance` is NONZERO and equals mesh A's instance count; each bucket's
    /// instances are contiguous + non-overlapping in the ring; `Σ instance_count ==
    /// total`; and — because every row is static — the pair lanes stay EMPTY (interp
    /// OFF, byte-identical to R3).
    #[test]
    fn bucketing_two_meshes_nonzero_base_contiguous() {
        let mesh_count = 2;
        // Interleave the two meshes' instances so the scatter (not the input order)
        // is what produces contiguous buckets: A, B, A, B, A.
        let a0 = affine(0, 0);
        let a1 = affine(0, 1);
        let a2 = affine(0, 2);
        let b0 = affine(1, 0);
        let b1 = affine(1, 1);
        let inputs = [
            stat(0, &a0),
            stat(1, &b0),
            stat(0, &a1),
            stat(1, &b1),
            stat(0, &a2),
        ];

        let mut scratch = MeshRenderScratch::default();
        scratch.gather_mixed_into(mesh_count, meta, || inputs.iter().copied());

        // One batch per distinct mesh (Principle 1: one draw per mesh).
        assert_eq!(scratch.batch_count(), 2, "two distinct meshes => two batches");

        // Batch 0 = mesh A: base 0, 3 instances, Uint16 width, 6 indices.
        let ba = scratch.batches.as_read_slice()[0];
        assert_eq!(ba.mesh_id, 0);
        assert_eq!(ba.base_instance, 0, "mesh A is the first bucket => base 0");
        assert_eq!(ba.instance_count, 3);
        assert_eq!(ba.index_count, 6);
        assert_eq!(ba.index_type, IndexType::Uint16);

        // Batch 1 = mesh B: base == count(A) == 3 (NONZERO — the C1 proof), 2
        // instances, Uint32 width, 12 indices (O3 mixed width).
        let bb = scratch.batches.as_read_slice()[1];
        assert_eq!(bb.mesh_id, 1);
        assert_eq!(bb.base_instance, 3, "mesh B's base == count(A) == 3 (NONZERO)");
        assert_eq!(bb.instance_count, 2);
        assert_eq!(bb.index_count, 12);
        assert_eq!(bb.index_type, IndexType::Uint32);

        // Σ instance_count == total inputs.
        let total: u32 = scratch.batches.as_read_slice().iter().map(|b| b.instance_count).sum();
        assert_eq!(total, 5);
        assert_eq!(scratch.instance_count(), 5, "the ring holds every instance");
        // All-static ⇒ interp OFF (the pair lanes are empty ⇒ no dispatch).
        assert_eq!(scratch.dynamic_count(), 0, "an all-static gather arms no interp");
        assert!(scratch.pair_out_slot.is_empty());

        // The ring: mesh A's 3 instances contiguous at [0..3), mesh B's 2 at [3..5).
        // Each slot holds the EXPECTED instance (its translation encodes (mesh, ord)),
        // proving the scatter placed each row in its own bucket with no overlap.
        for ord in 0..3u32 {
            let slot = ba.base_instance + ord;
            assert_eq!(
                scratch.ring.as_read_slice()[slot as usize], affine(0, ord),
                "mesh A instance {ord} at ring slot {slot}"
            );
        }
        for ord in 0..2u32 {
            let slot = bb.base_instance + ord;
            assert_eq!(
                scratch.ring.as_read_slice()[slot as usize], affine(1, ord),
                "mesh B instance {ord} at ring slot {slot}"
            );
        }
    }

    /// An empty gather (no visible instances) emits zero batches + an empty ring (the
    /// recorder's legacy 0%-gate path) + empty pair lanes.
    #[test]
    fn bucketing_empty_yields_no_batches() {
        let mut scratch = MeshRenderScratch::default();
        let inputs: Vec<(u32, &InstanceModelCol, Option<&GpuTransform3D>)> = Vec::new();
        scratch.gather_mixed_into(3, meta, || inputs.iter().copied());
        assert_eq!(scratch.batch_count(), 0);
        assert_eq!(scratch.instance_count(), 0);
        assert_eq!(scratch.dynamic_count(), 0);
    }

    /// A gap mesh (mesh 1 has zero instances; meshes 0 and 2 have some) emits NO batch
    /// for the empty mesh, and mesh 2's base_instance skips mesh 1's (zero) bucket —
    /// the prefix-sum is over the actual counts, not the mesh index.
    #[test]
    fn bucketing_skips_empty_mesh_in_the_middle() {
        let a0 = affine(0, 0);
        let a1 = affine(0, 1);
        let c0 = affine(2, 0);
        let inputs = [stat(0, &a0), stat(2, &c0), stat(0, &a1)];

        let mut scratch = MeshRenderScratch::default();
        scratch.gather_mixed_into(3, meta, || inputs.iter().copied());

        assert_eq!(scratch.batch_count(), 2, "mesh 1 is empty => only 2 batches");
        let batches = scratch.batches.as_read_slice();
        assert_eq!(batches[0].mesh_id, 0);
        assert_eq!(batches[0].base_instance, 0);
        assert_eq!(batches[0].instance_count, 2);
        // Mesh 2's base == count(0) + count(1) == 2 + 0 == 2.
        assert_eq!(batches[1].mesh_id, 2);
        assert_eq!(batches[1].base_instance, 2);
        assert_eq!(batches[1].instance_count, 1);
        assert_eq!(scratch.instance_count(), 3);
    }

    /// FIX-C1 regression guard (asset-streaming plan F6, hwrt GPU-UAF closure): a
    /// `meta` closure returning `None` for a mesh with a NONZERO instance count in
    /// the MIDDLE of the scene (a retired-slot hole — its carriers are still
    /// gathered by the query this frame, between `validate_asset_refs`'s
    /// best-effort disable and this gather) must EXCLUDE that mesh's instances
    /// from the ring / `mesh_ids` ENTIRELY (not merely leave them un-batched —
    /// `bucket_lanes_mixed` zeroes `counts[m]` for a non-resolvable mesh, and the
    /// scatter's `counts[m] == 0` skip excludes it): the hwrt TLAS
    /// packer reads `BlasAddr[MeshIds[i]]` for every `i` in the FULL ring, so a
    /// retired mesh's id surviving in `mesh_ids` would read a freed BLAS device
    /// address. Sibling of [`bucketing_skips_empty_mesh_in_the_middle`] above,
    /// which only covers the `count == 0` skip (a DIFFERENT code path — that
    /// test never reaches the `meta` call at all, since `bucket_lanes_mixed`'s
    /// `self.counts[m] != 0` guard short-circuits first).
    #[test]
    fn none_mid_scene_meta_excludes_its_instances_from_the_ring() {
        let mesh_count = 3;
        // mesh 0: 2 instances, mesh 1 (the retired hole, NONZERO count): 3
        // instances, mesh 2: 2 instances. Interleaved so the scatter order, not
        // input order, drives the ring layout.
        let a0 = affine(0, 0);
        let a1 = affine(0, 1);
        let b0 = affine(1, 0);
        let b1 = affine(1, 1);
        let b2 = affine(1, 2);
        let c0 = affine(2, 0);
        let c1 = affine(2, 1);
        let inputs = [
            stat(0, &a0),
            stat(1, &b0),
            stat(0, &a1),
            stat(1, &b1),
            stat(2, &c0),
            stat(1, &b2),
            stat(2, &c1),
        ];

        // FIX-C1 scenario: mesh 1's meta returns None (a retired-slot hole) —
        // everything else (the query rows, the mesh_count) is unchanged.
        let meta_hole = |mesh_id: u32| if mesh_id == 1 { None } else { meta(mesh_id) };
        let mut holed = MeshRenderScratch::default();
        holed.gather_mixed_into(mesh_count, meta_hole, || inputs.iter().copied());

        assert_eq!(holed.batch_count(), 2, "mesh 1's None must skip its batch");
        assert!(
            holed.batches.as_read_slice().iter().all(|b| b.mesh_id != 1),
            "no batch may reference the mesh whose meta returned None"
        );

        // THE KEY GATE (FIX-C1): the ring holds ONLY the surviving (resolvable)
        // instances — count == ring.len() == Σ surviving, NOT the pre-hole total.
        assert_eq!(
            holed.instance_count(),
            4,
            "the retired mesh's 3 instances must be EXCLUDED, not merely un-batched: \
             2 (mesh 0) + 2 (mesh 2) == 4, not 2 + 3 + 2 == 7"
        );

        // No retired mesh_id ever appears in the TLAS-consumable mesh-id lane —
        // the exact hazard FIX-C1 closes (a stale id would let the hwrt packer
        // read `BlasAddr[1]`, a freed BLAS device address).
        assert!(
            holed.mesh_ids.as_read_slice().iter().all(|&id| id != 1),
            "the retired mesh's id must never appear in mesh_ids"
        );

        // The surviving batches are CONTIGUOUS (no gap where the excluded mesh's
        // instances used to be) — mesh 2 starts exactly where mesh 0 ends.
        let batches = holed.batches.as_read_slice();
        let mesh0 = *batches.iter().find(|b| b.mesh_id == 0).expect("mesh0 batch present");
        let mesh2 = *batches.iter().find(|b| b.mesh_id == 2).expect("mesh2 batch present");
        assert_eq!(mesh0.base_instance, 0, "mesh 0's base_instance is unaffected (it precedes the hole)");
        assert_eq!(mesh0.instance_count, 2);
        assert_eq!(
            mesh2.base_instance,
            mesh0.base_instance + mesh0.instance_count,
            "mesh 2's batch is CONTIGUOUS with mesh 0's — no gap for the excluded mesh"
        );
        assert_eq!(mesh2.instance_count, 2);

        // The ring itself is exactly [a0, a1, c0, c1] — no reserved-but-empty slots.
        let ring = holed.ring.as_read_slice();
        assert_eq!(ring[0], a0);
        assert_eq!(ring[1], a1);
        assert_eq!(ring[2], c0);
        assert_eq!(ring[3], c1);
    }

    /// Re-running the gather REUSES the scratch's capacity (Principle 5): after a
    /// large frame and then a smaller one, the smaller frame produces the correct
    /// (smaller) result without losing the reserved capacity.
    #[test]
    fn gather_reuses_capacity_across_frames() {
        let mut scratch = MeshRenderScratch::default();

        // Frame 1: 5 instances across 2 meshes.
        let big: Vec<InstanceModelCol> = (0..5).map(|i| affine(i % 2, i)).collect();
        let big_inputs: Vec<(u32, &InstanceModelCol, Option<&GpuTransform3D>)> =
            big.iter().enumerate().map(|(i, c)| ((i as u32) % 2, c, None)).collect();
        scratch.gather_mixed_into(2, meta, || big_inputs.iter().copied());
        assert_eq!(scratch.instance_count(), 5);
        let ring_cap_after_big = scratch.ring.capacity();

        // Frame 2: 1 instance of mesh 0.
        let small = affine(0, 0);
        let small_inputs = [stat(0, &small)];
        scratch.gather_mixed_into(2, meta, || small_inputs.iter().copied());
        assert_eq!(scratch.batch_count(), 1);
        assert_eq!(scratch.instance_count(), 1);
        // The capacity did not shrink — the smaller frame reused the big frame's ring.
        assert!(
            scratch.ring.capacity() >= ring_cap_after_big,
            "the ring retains its reserved capacity across a smaller frame"
        );
    }

    // ════════════════════════════════════════════════════════════════════════
    // Refined-B — the UNIFIED static + interpolated gather (one output ring).
    // ════════════════════════════════════════════════════════════════════════

    /// The direct R5 review P0 witness: a MIXED scene (1 static floor mesh + 1
    /// interpolated cube mesh) yields TWO batches with NO drawable dropped, the whole
    /// ring is covered contiguously `[0, total)` with no gap, the STATIC row's ring slot
    /// holds its real affine VERBATIM, and the DYNAMIC row's pair + ring slot land in the
    /// parallel `pair_ring` / `pair_out_slot` lanes (the interp compute's OutSlot input).
    #[test]
    fn mixed_static_and_interp_scene_drops_no_drawable() {
        let mesh_count = 2;
        // mesh 0 = the static floor (2 instances); mesh 1 = the interpolated cube
        // (1 instance carrying a pair). Interleave so the scatter, not the input
        // order, produces contiguous buckets: floor, cube, floor.
        let floor0 = affine(0, 0);
        let floor1 = affine(0, 1);
        // The cube's CPU ring slot is a placeholder (overwritten on-GPU); give it a
        // recognizable value so we can prove the slot was reserved, not skipped.
        let cube_placeholder = affine(1, 0);
        let cube_pair = pair(1, 0);
        let inputs = [
            (0u32, &floor0, None),
            (1u32, &cube_placeholder, Some(&cube_pair)),
            (0u32, &floor1, None),
        ];

        let mut scratch = MeshRenderScratch::default();
        scratch.gather_mixed_into(mesh_count, meta, || inputs.iter().copied());

        // TWO batches — the floor batch is NOT dropped by the cube's presence (the P0
        // regression dropped exactly this).
        assert_eq!(scratch.batch_count(), 2, "static + interp => two batches, none dropped");
        let batches = scratch.batches.as_read_slice();
        let bf = batches[0];
        let bc = batches[1];
        assert_eq!((bf.mesh_id, bf.base_instance, bf.instance_count), (0, 0, 2), "floor batch");
        assert_eq!((bc.mesh_id, bc.base_instance, bc.instance_count), (1, 2, 1), "cube batch (NONZERO base)");

        // base_instance / instance_count cover the whole bound ring [0, 3) with no gap.
        let total: u32 = batches.iter().map(|b| b.instance_count).sum();
        assert_eq!(total, 3);
        assert_eq!(scratch.instance_count(), 3, "ring covers every drawable, no drop");
        let mut covered = [false; 3];
        for b in batches {
            for s in b.base_instance..b.base_instance + b.instance_count {
                assert!(!covered[s as usize], "no ring slot double-covered");
                covered[s as usize] = true;
            }
        }
        assert!(covered.iter().all(|&c| c), "the two batches cover [0, 3) contiguously");

        // The STATIC floor rows hold their real affines verbatim (not GPU-touched).
        let ring = scratch.ring.as_read_slice();
        assert_eq!(ring[0], affine(0, 0), "floor slot 0 is the real affine");
        assert_eq!(ring[1], affine(0, 1), "floor slot 1 is the real affine");

        // The DYNAMIC cube: exactly one pair, its out-slot == the cube's ring slot (2),
        // and that slot is NOT a static slot (never CPU-authoritative).
        assert_eq!(scratch.dynamic_count(), 1, "exactly one interpolated instance");
        assert_eq!(scratch.pair_ring.as_read_slice()[0], cube_pair, "the cube's pair was recorded");
        assert_eq!(
            scratch.pair_out_slot.as_read_slice()[0], bc.base_instance,
            "the cube's out-slot is its gather-assigned ring slot (2)"
        );
    }

    /// The dynamic row's ring slot placeholder is inert: a scene of ONLY interpolated
    /// bodies still assigns each a ring slot (the compute writes them all on-GPU),
    /// `dynamic_count() == the total`, and every out-slot is a distinct in-range index.
    #[test]
    fn all_interp_scene_assigns_every_slot() {
        let mesh_count = 2;
        let p_a = pair(0, 0);
        let p_b0 = pair(1, 0);
        let p_b1 = pair(1, 1);
        let ph = affine(9, 9); // one shared placeholder — the CPU bytes are overwritten.
        let inputs = [
            (0u32, &ph, Some(&p_a)),
            (1u32, &ph, Some(&p_b0)),
            (1u32, &ph, Some(&p_b1)),
        ];

        let mut scratch = MeshRenderScratch::default();
        scratch.gather_mixed_into(mesh_count, meta, || inputs.iter().copied());

        assert_eq!(scratch.instance_count(), 3, "the ring reserves a slot per drawable");
        assert_eq!(scratch.dynamic_count(), 3, "every row is interpolated");
        assert_eq!(scratch.pair_ring.len(), scratch.pair_out_slot.len());
        // Every out-slot is a distinct index in [0, 3).
        let mut slots: Vec<u32> = scratch.pair_out_slot.as_read_slice().to_vec();
        slots.sort_unstable();
        assert_eq!(slots, vec![0, 1, 2], "the three out-slots partition the ring");
    }

    /// The unified ring and the pair lanes reuse their capacity across successive
    /// gathers on ONE reused scratch (Principle 5): a mixed gather then a smaller
    /// gather each produce the correct result, and the pair lane keeps its capacity.
    #[test]
    fn unified_ring_reuses_capacity_across_frames() {
        let mut scratch = MeshRenderScratch::default();

        // Frame 1: 2 static + 1 interp.
        let s0 = affine(0, 0);
        let s1 = affine(0, 1);
        let ph = affine(1, 0);
        let p = pair(1, 0);
        let f1 = [(0u32, &s0, None), (0u32, &s1, None), (1u32, &ph, Some(&p))];
        scratch.gather_mixed_into(2, meta, || f1.iter().copied());
        assert_eq!(scratch.instance_count(), 3);
        assert_eq!(scratch.dynamic_count(), 1);
        let pair_cap = scratch.pair_ring.capacity();

        // Frame 2: 1 static of mesh 0 (no interp) — the pair lane empties but keeps cap.
        let only = affine(0, 0);
        let f2 = [stat(0, &only)];
        scratch.gather_mixed_into(2, meta, || f2.iter().copied());
        assert_eq!(scratch.instance_count(), 1);
        assert_eq!(scratch.dynamic_count(), 0, "a static frame clears the pair lanes");
        assert!(
            scratch.pair_ring.capacity() >= pair_cap,
            "the pair ring retains its reserved capacity across a static frame"
        );
    }

    /// M3 → HW-RT: the per-instance mesh-id (BLAS-id) lane is parallel to the ring and
    /// agrees with the per-mesh batches — every ring slot in a batch's `base_instance`
    /// range carries that batch's `mesh_id`. Crucially the lane is keyed off the input
    /// MESH KEY, not the ring affine: a DYNAMIC row's ring affine is a placeholder (the
    /// interp compute overwrites it on-GPU), yet its mesh-id entry is still correct — so a
    /// TLAS builder reading (`ring[i]`, `mesh_ids[i]`) maps every instance, static or
    /// interpolated, to the right BLAS.
    #[test]
    fn mesh_id_lane_is_parallel_to_ring_and_matches_batches() {
        let mesh_count = 2;
        let a00 = affine(0, 0);
        let a01 = affine(0, 1);
        let a10 = affine(1, 0);
        // The DYNAMIC row's ring affine is a deliberately WRONG-encoded placeholder
        // (translation encodes mesh 9), proving the mesh-id lane is set from the mesh KEY,
        // never re-derived from the placeholder affine.
        let ph = affine(9, 9);
        let p = pair(1, 0);
        let inputs = [
            (0u32, &a00, None),
            (1u32, &a10, None),
            (0u32, &a01, None),
            (1u32, &ph, Some(&p)),
        ];

        let mut scratch = MeshRenderScratch::default();
        scratch.gather_mixed_into(mesh_count, meta, || inputs.iter().copied());

        // The lane is parallel to the ring (one id per instance).
        assert_eq!(scratch.mesh_ids.len(), scratch.ring.len());
        assert_eq!(scratch.mesh_ids.len(), 4);

        // Every slot in each batch's range carries that batch's mesh_id.
        let mesh_ids = scratch.mesh_ids.as_read_slice();
        for b in scratch.batches.as_read_slice() {
            let start = b.base_instance as usize;
            let end = start + b.instance_count as usize;
            for (slot, &id) in mesh_ids.iter().enumerate().take(end).skip(start) {
                assert_eq!(
                    id, b.mesh_id,
                    "ring slot {slot} must carry its batch's mesh_id {}",
                    b.mesh_id
                );
            }
        }
        // Concretely: mesh 0 fills [0,2), mesh 1 fills [2,4).
        assert_eq!(mesh_ids, [0u32, 0, 1, 1]);

        // The DYNAMIC row: its ring affine is the WRONG-encoded placeholder (x == 9), but
        // its mesh-id entry is the correct BLAS id (1). The interp out-slot points at that
        // same slot, and the lane there reads 1.
        let dyn_slot = scratch.pair_out_slot.as_read_slice()[0] as usize;
        assert_eq!(mesh_ids[dyn_slot], 1, "the dynamic row maps to BLAS 1");
        let ring = scratch.ring.as_read_slice();
        assert_eq!(
            ring[dyn_slot].rows[0][3], 9.0,
            "the dynamic row's ring affine is the placeholder (proves the lane is key-derived)"
        );

        // For STATIC rows the ring affine's encoded mesh id agrees with the lane.
        for (slot, &mid) in mesh_ids.iter().enumerate() {
            if slot == dyn_slot {
                continue;
            }
            assert_eq!(
                ring[slot].rows[0][3] as u32, mid,
                "a static row's affine-encoded mesh id matches the lane"
            );
        }
    }
}
