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
//! Alloc-free after warmup: every `Vec` is `clear()`ed + re-filled (capacity
//! persists); the per-mesh lanes grow POW2 keyed off the registry's mesh count (O2 —
//! no fixed `MAX_MESHES` ceiling), and the ring grows POW2 keyed off the live
//! instance count. The scratch is a reused [`Resource`], NOT an ad-hoc `Vec` (the
//! [`UiRenderScratch`](crate::ui::UiRenderScratch) precedent, Principle 5).

use boyko_ecs::ecs::core::iters::query::Query;
use boyko_ecs::ecs::core::iters::query::filter_enable::Enabled;
use boyko_ecs::ecs::core::system::{NonSendRes, ResMut};
use boyko_macros::Resource;
use boyko_rhi::enums::IndexType;
use boyko_scene::render_caps::{MeshHandle, RenderEnabled};
use bytemuck::Zeroable;

use crate::gpu_transform3d::GpuTransform3D;
use crate::instance_model::InstanceModelCol;
use crate::mesh_registry::MeshRegistry;

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
    /// buffers via the [`MeshRegistry`].
    pub mesh_id: u32,
    /// The mesh's index count (`vkCmdDrawIndexed`'s `index_count`), copied from the
    /// registry at gather time so the recorder reads it without a second lookup.
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
/// Cleared-not-reallocated each frame (Principle 5): a steady-state frame only
/// `clear()`s + re-fills + scatters in place, so there is ZERO steady-state
/// allocation. The per-mesh lanes (`counts`/`offsets`/`cursors`) grow POW2 keyed off
/// the registry's mesh count (O2 — no fixed `MAX_MESHES`); the instance `ring` grows
/// POW2 keyed off the live instance count.
#[derive(Resource, Default)]
pub struct MeshRenderScratch {
    /// `counts[m]` — the number of visible instances of mesh `m` (pass 1). Length ==
    /// the per-mesh-lane capacity (≥ mesh count); `clear()` + re-fill with zeros.
    counts: Vec<u32>,
    /// `offsets[m]` — mesh `m`'s `base_instance` (the prefix-sum of `counts`). Reused.
    offsets: Vec<u32>,
    /// `cursors[m]` — the scatter write-head within mesh `m`'s bucket (pass 2),
    /// `0..counts[m]`. A separate lane so `offsets` stays the immutable bucket base.
    cursors: Vec<u32>,
    /// The emitted [`DrawBatch`]es (one per non-empty mesh), in mesh-id order;
    /// `clear()` + `extend`, never `Vec::new`.
    pub batches: Vec<DrawBatch>,
    /// The contiguous instance ring — every visible instance's 48-byte
    /// [`InstanceModelCol`] scattered into its mesh's bucket. The renderer uploads
    /// this slice into ONE shared instance SSBO bound once for the whole batch list.
    /// `clear()` + scatter, capacity persists.
    pub ring: Vec<InstanceModelCol>,
    /// The contiguous interpolation-PAIR ring (Pillar B B1) — every visible
    /// instance's 96-byte [`GpuTransform3D`] scattered into its mesh's bucket, in the
    /// SAME batch order as [`ring`](Self::ring). The B2 interpolation compute pre-pass
    /// reads this slice as its `TransformPair` input SSBO; its per-instance model
    /// output lands in the [`ring`](Self::ring) layout, so the interpolated instances
    /// are already draw-ordered. Populated by
    /// [`gather_pairs_into`](Self::gather_pairs_into) (the sibling of
    /// [`gather_into`](Self::gather_into)); `clear()` + scatter, capacity persists.
    pub pair_ring: Vec<GpuTransform3D>,
}

/// Grows `v` to at least `min_len` using POW2 capacity steps (O2 — no fixed ceiling),
/// then sets its length to exactly `min_len` (the extra capacity stays reserved). The
/// `[min_len .. ]` tail is left at `fill` after a grow; existing `[.. old_len]` is NOT
/// reset here (the caller zeroes lanes explicitly where it matters). Alloc-free once
/// the capacity covers `min_len`.
#[inline]
fn fit_len(v: &mut Vec<u32>, min_len: usize, fill: u32) {
    if v.capacity() < min_len {
        // POW2 reserve keyed off the requested length (O2): reserve up to the next
        // power of two so repeated small growths amortize, never realloc per frame.
        let target = min_len.next_power_of_two();
        v.reserve(target - v.len());
    }
    v.clear();
    v.resize(min_len, fill);
}

impl MeshRenderScratch {
    /// The number of distinct meshes with at least one visible instance this frame —
    /// `batches.len()` after a [`gather_into`](Self::gather_into). The Principle-1
    /// one-draw-per-mesh count.
    #[inline]
    pub fn batch_count(&self) -> usize {
        self.batches.len()
    }

    /// The total number of scattered instances (== the ring length) — `Σ
    /// batch.instance_count` across every batch.
    #[inline]
    pub fn instance_count(&self) -> usize {
        self.ring.len()
    }

    /// The TESTABLE gather core (Decision 7): count → prefix-sum → scatter, into the
    /// reused scratch. `mesh_count` is the registry's mesh count (sizes the per-mesh
    /// lanes, O2); `meta` resolves a mesh id to its `(index_count, index_type)` for the
    /// emitted batch; `iter_input` is an ITERATOR FACTORY the gather invokes TWICE (once
    /// to count, once to scatter) — each call returns a FRESH iterator over the same
    /// `(mesh_id, &InstanceModelCol)` source.
    ///
    /// The factory (`Fn() -> I`, not a re-iteration `&mut dyn FnMut` callback) keeps both
    /// passes FULLY MONOMORPHIC — zero virtual dispatch on the per-instance hot path
    /// (P-002/P4). An ECS [`Query`] iterator is not `Clone`, but it does not need to be:
    /// `Query::iter` borrows `&self`, so the factory simply re-runs `q.iter()` per pass,
    /// yielding a brand-new iterator each time (the system wrapper [`gather_mesh_draws`]
    /// passes `|| q.iter().map(|(h, c)| (h.0, c))`; a unit test passes `||
    /// slice.iter().copied()`). The two iterators observe the SAME rows (the gather is
    /// over row VALUES — mesh id + affine — not row order), so a stable two-pass walk
    /// yields contiguous, correctly-offset buckets. Keeping the inputs behind the factory
    /// (not building an ECS world or touching a GPU) makes the bucketing unit-testable in
    /// isolation.
    ///
    /// After the call: `batches` holds one [`DrawBatch`] per non-empty mesh in mesh-id
    /// order with the correct prefix-sum `base_instance`s, and `ring` holds each mesh's
    /// instances contiguously (no overlap, `Σ instance_count == the input count`).
    ///
    /// `debug_assert!`s catch an out-of-range `mesh_id` (a gather over a handle the
    /// registry never minted — a bundle/asset-binding bug).
    pub fn gather_into<'a, M, F, I>(&mut self, mesh_count: usize, meta: M, iter_input: F)
    where
        M: FnMut(u32) -> (u32, IndexType),
        F: Fn() -> I,
        I: Iterator<Item = (u32, &'a InstanceModelCol)>,
    {
        // Shared count → prefix-sum → batch-emit over the lanes; then scatter the
        // 48-byte records into `ring`. `ring` is temporarily taken so the closure can
        // borrow `&self.offsets`/`&mut self.cursors` disjointly from it.
        let total = self.bucket_lanes(mesh_count, meta, &iter_input);
        let mut ring = std::mem::take(&mut self.ring);
        ring.clear();
        ring.resize(total as usize, InstanceModelCol::zeroed());
        {
            let offsets = &self.offsets;
            let cursors = &mut self.cursors;
            for (mesh_id, col) in iter_input() {
                let m = mesh_id as usize;
                let slot = offsets[m] + cursors[m];
                cursors[m] += 1;
                ring[slot as usize] = *col;
            }
        }
        debug_assert_eq!(
            ring.len(),
            total as usize,
            "invariant: the ring holds exactly Σ instance_count instances"
        );
        self.ring = ring;
    }

    /// The pair-emitting sibling of [`gather_into`](Self::gather_into) (Pillar B B1):
    /// identical count → prefix-sum → batch structure, but scatters the 96-byte
    /// [`GpuTransform3D`] interpolation pairs into [`pair_ring`](Self::pair_ring)
    /// instead of the 48-byte affines into [`ring`](Self::ring).
    ///
    /// The buckets are keyed on the SAME `mesh_id` and emitted in the SAME order, so
    /// `pair_ring[base_instance .. base_instance + instance_count]` is mesh `m`'s
    /// contiguous pair bucket — draw-ordered, mirroring the `ring` layout. Feeding
    /// this ring through the B2 interpolation compute pre-pass yields per-instance
    /// model columns already in draw order (the interp output lands draw-ready).
    ///
    /// The 48-byte `InstanceModelCol` [`ring`](Self::ring) is UNTOUCHED (it is the
    /// interpolation-OFF path); a frame runs EITHER this pair gather (interp on) OR
    /// [`gather_into`](Self::gather_into) (interp off), never both — but both leave the
    /// [`batches`](Self::batches) invariant identical, so a switch is transparent to
    /// the recorder.
    ///
    /// `iter_input` is the same twice-invoked iterator factory as
    /// [`gather_into`](Self::gather_into), yielding `(mesh_id, &GpuTransform3D)`.
    pub fn gather_pairs_into<'a, M, F, I>(&mut self, mesh_count: usize, meta: M, iter_input: F)
    where
        M: FnMut(u32) -> (u32, IndexType),
        F: Fn() -> I,
        I: Iterator<Item = (u32, &'a GpuTransform3D)>,
    {
        let total = self.bucket_lanes(mesh_count, meta, &iter_input);
        let mut pair_ring = std::mem::take(&mut self.pair_ring);
        pair_ring.clear();
        pair_ring.resize(total as usize, GpuTransform3D::zeroed());
        {
            let offsets = &self.offsets;
            let cursors = &mut self.cursors;
            for (mesh_id, pair) in iter_input() {
                let m = mesh_id as usize;
                let slot = offsets[m] + cursors[m];
                cursors[m] += 1;
                pair_ring[slot as usize] = *pair;
            }
        }
        debug_assert_eq!(
            pair_ring.len(),
            total as usize,
            "invariant: the pair ring holds exactly Σ instance_count pairs"
        );
        self.pair_ring = pair_ring;
    }

    /// The shared count → prefix-sum → batch-emit core of both gather paths (Decision
    /// 7, factored so the 48-byte affine and the 96-byte pair paths dedup it).
    ///
    /// Fills `counts` (pass 1), `offsets` (each mesh's `base_instance`), zeroed
    /// `cursors` (the scatter write-heads the caller advances), and `batches` (one
    /// [`DrawBatch`] per non-empty mesh, mesh-id order). Returns `Σ instance_count` —
    /// the ring length the caller sizes its scatter to.
    ///
    /// `iter_input` is invoked ONCE here (the count pass); the caller invokes it a
    /// second time for the scatter. Only the small `mesh_id` key is touched — the
    /// record type `T` is never read, so this is generic over the two record shapes.
    fn bucket_lanes<'a, M, F, I, T: 'a>(
        &mut self,
        mesh_count: usize,
        mut meta: M,
        iter_input: &F,
    ) -> u32
    where
        M: FnMut(u32) -> (u32, IndexType),
        F: Fn() -> I,
        I: Iterator<Item = (u32, &'a T)>,
    {
        // --- Pass 1: count per mesh (touches only the small MeshHandle key). ---
        fit_len(&mut self.counts, mesh_count, 0);
        {
            let counts = &mut self.counts;
            for (mesh_id, _rec) in iter_input() {
                debug_assert!(
                    (mesh_id as usize) < mesh_count,
                    "invariant: a gathered mesh_id is in range of the registry"
                );
                counts[mesh_id as usize] += 1;
            }
        }

        // --- Prefix-sum: offsets[m] = Σ counts[0..m] = mesh m's base_instance. Emit a
        // DrawBatch per non-empty mesh (in mesh-id order). ---
        fit_len(&mut self.offsets, mesh_count, 0);
        fit_len(&mut self.cursors, mesh_count, 0);
        self.batches.clear();
        let mut running: u32 = 0;
        for m in 0..mesh_count {
            self.offsets[m] = running;
            let c = self.counts[m];
            if c != 0 {
                let (index_count, index_type) = meta(m as u32);
                self.batches.push(DrawBatch {
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
}

/// The ECS-native M3 gather SYSTEM: buckets every visible
/// `(MeshHandle, InstanceModelCol)` entity into per-mesh [`DrawBatch`]es + the shared
/// instance [`ring`](MeshRenderScratch::ring), reusing the [`MeshRenderScratch`]
/// resource (Principle 0 — instances from spawned entities via the query, not an
/// ad-hoc buffer).
///
/// The query is filtered on `Enabled<RenderEnabled>` (the `Visibility::Hidden` gate),
/// so a hidden row never enters a bucket. The [`MeshRegistry`] (a `NonSend` resource)
/// supplies the mesh count (sizes the lanes, O2) + each batch's `(index_count,
/// index_type)`.
///
/// # 0%-gate
///
/// A world with no `InstanceModelCol` column yields zero matching rows, so the gather
/// emits zero batches + an empty ring — the recorder then takes the legacy
/// (empty-slice) draw, byte-identical to the pre-M3 stream.
///
/// # Two passes over the query
///
/// The Decision-7 count + scatter each iterate the query once. The query is
/// re-iterable (`Query::iter` borrows `&self`-style state), so the gather passes an
/// iterator FACTORY (`|| q.iter().map(..)`) that is re-run per pass — both passes read
/// the SAME rows, fully monomorphically (no per-instance virtual dispatch). The gather is
/// over the row VALUES (mesh id + affine), not the row order, so a stable two-pass walk
/// yields contiguous, correctly-offset buckets.
#[allow(clippy::needless_pass_by_value)]
pub fn gather_mesh_draws(
    q: Query<(&MeshHandle, &InstanceModelCol), Enabled<RenderEnabled>>,
    registry: NonSendRes<MeshRegistry>,
    mut scratch: ResMut<MeshRenderScratch>,
) {
    let mesh_count = registry.len();
    scratch.gather_into(
        mesh_count,
        |mesh_id| {
            let m = registry.get(MeshHandle(mesh_id));
            (m.index_count, m.index_type)
        },
        || q.iter().map(|(h, col)| (h.0, col)),
    );
}

/// The pair-emitting sibling of [`gather_mesh_draws`] (Pillar B B1): buckets every
/// visible `(MeshHandle, GpuTransform3D)` entity into per-mesh
/// [`DrawBatch`](DrawBatch)es + the shared 96-byte interpolation-pair ring
/// ([`pair_ring`](MeshRenderScratch::pair_ring)), reusing the same
/// [`MeshRenderScratch`] and the shared count/prefix-sum core.
///
/// This is the INTERPOLATION-ON gather: the pairs it scatters (draw-ordered) feed the
/// B2 compute pre-pass, whose per-instance model output lands in the `ring` layout.
/// A frame runs EITHER this OR [`gather_mesh_draws`] (the 48-byte interp-off path),
/// never both.
///
/// The query is a mixed table + dense join (`MeshHandle` is a table column,
/// `GpuTransform3D` the dense column), filtered on `Enabled<RenderEnabled>` (the
/// `Visibility::Hidden` gate) — a hidden row never enters a bucket, exactly as the
/// affine gather does. The [`MeshRegistry`] supplies the mesh count + each batch's
/// `(index_count, index_type)`.
///
/// # 0%-gate
///
/// A world with no `GpuTransform3D` column yields zero matching rows, so the gather
/// emits zero batches + an empty pair ring.
#[allow(clippy::needless_pass_by_value)]
pub fn gather_mesh_draw_pairs(
    q: Query<(&MeshHandle, &GpuTransform3D), Enabled<RenderEnabled>>,
    registry: NonSendRes<MeshRegistry>,
    mut scratch: ResMut<MeshRenderScratch>,
) {
    let mesh_count = registry.len();
    scratch.gather_pairs_into(
        mesh_count,
        |mesh_id| {
            let m = registry.get(MeshHandle(mesh_id));
            (m.index_count, m.index_type)
        },
        || q.iter().map(|(h, pair)| (h.0, pair)),
    );
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
    /// O3 mixed-width batch carry.
    fn meta(mesh_id: u32) -> (u32, IndexType) {
        let width = if mesh_id.is_multiple_of(2) {
            IndexType::Uint16
        } else {
            IndexType::Uint32
        };
        (6 * (mesh_id + 1), width)
    }

    /// The C1 nonzero-`base_instance` proof + the Principle-1 one-draw-per-mesh guard,
    /// CPU-side: two meshes (A=mesh 0 with 3 instances, B=mesh 1 with 2 instances)
    /// bucket into one batch each; mesh B's `base_instance` is NONZERO and equals
    /// mesh A's instance count; each bucket's instances are contiguous + non-overlapping
    /// in the ring; `Σ instance_count == total`.
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
        let inputs: Vec<(u32, &InstanceModelCol)> =
            vec![(0, &a0), (1, &b0), (0, &a1), (1, &b1), (0, &a2)];

        let mut scratch = MeshRenderScratch::default();
        scratch.gather_into(mesh_count, meta, || inputs.iter().copied());

        // One batch per distinct mesh (Principle 1: one draw per mesh).
        assert_eq!(scratch.batch_count(), 2, "two distinct meshes => two batches");

        // Batch 0 = mesh A: base 0, 3 instances, Uint16 width, 6 indices.
        let ba = scratch.batches[0];
        assert_eq!(ba.mesh_id, 0);
        assert_eq!(ba.base_instance, 0, "mesh A is the first bucket => base 0");
        assert_eq!(ba.instance_count, 3);
        assert_eq!(ba.index_count, 6);
        assert_eq!(ba.index_type, IndexType::Uint16);

        // Batch 1 = mesh B: base == count(A) == 3 (NONZERO — the C1 proof), 2
        // instances, Uint32 width, 12 indices (O3 mixed width).
        let bb = scratch.batches[1];
        assert_eq!(bb.mesh_id, 1);
        assert_eq!(bb.base_instance, 3, "mesh B's base == count(A) == 3 (NONZERO)");
        assert_eq!(bb.instance_count, 2);
        assert_eq!(bb.index_count, 12);
        assert_eq!(bb.index_type, IndexType::Uint32);

        // Σ instance_count == total inputs.
        let total: u32 = scratch.batches.iter().map(|b| b.instance_count).sum();
        assert_eq!(total, 5);
        assert_eq!(scratch.instance_count(), 5, "the ring holds every instance");

        // The ring: mesh A's 3 instances contiguous at [0..3), mesh B's 2 at [3..5).
        // Each slot holds the EXPECTED instance (its translation encodes (mesh, ord)),
        // proving the scatter placed each row in its own bucket with no overlap.
        for ord in 0..3u32 {
            let slot = ba.base_instance + ord;
            assert_eq!(
                scratch.ring[slot as usize], affine(0, ord),
                "mesh A instance {ord} at ring slot {slot}"
            );
        }
        for ord in 0..2u32 {
            let slot = bb.base_instance + ord;
            assert_eq!(
                scratch.ring[slot as usize], affine(1, ord),
                "mesh B instance {ord} at ring slot {slot}"
            );
        }
    }

    /// An empty gather (no visible instances) emits zero batches + an empty ring (the
    /// recorder's legacy 0%-gate path).
    #[test]
    fn bucketing_empty_yields_no_batches() {
        let mut scratch = MeshRenderScratch::default();
        let inputs: Vec<(u32, &InstanceModelCol)> = Vec::new();
        scratch.gather_into(3, meta, || inputs.iter().copied());
        assert_eq!(scratch.batch_count(), 0);
        assert_eq!(scratch.instance_count(), 0);
    }

    /// A gap mesh (mesh 1 has zero instances; meshes 0 and 2 have some) emits NO batch
    /// for the empty mesh, and mesh 2's base_instance skips mesh 1's (zero) bucket —
    /// the prefix-sum is over the actual counts, not the mesh index.
    #[test]
    fn bucketing_skips_empty_mesh_in_the_middle() {
        let a0 = affine(0, 0);
        let a1 = affine(0, 1);
        let c0 = affine(2, 0);
        let inputs: Vec<(u32, &InstanceModelCol)> = vec![(0, &a0), (2, &c0), (0, &a1)];

        let mut scratch = MeshRenderScratch::default();
        scratch.gather_into(3, meta, || inputs.iter().copied());

        assert_eq!(scratch.batch_count(), 2, "mesh 1 is empty => only 2 batches");
        assert_eq!(scratch.batches[0].mesh_id, 0);
        assert_eq!(scratch.batches[0].base_instance, 0);
        assert_eq!(scratch.batches[0].instance_count, 2);
        // Mesh 2's base == count(0) + count(1) == 2 + 0 == 2.
        assert_eq!(scratch.batches[1].mesh_id, 2);
        assert_eq!(scratch.batches[1].base_instance, 2);
        assert_eq!(scratch.batches[1].instance_count, 1);
        assert_eq!(scratch.instance_count(), 3);
    }

    /// Re-running the gather REUSES the scratch's capacity (Principle 5): after a
    /// large frame and then a smaller one, the smaller frame produces the correct
    /// (smaller) result without losing the reserved capacity.
    #[test]
    fn gather_reuses_capacity_across_frames() {
        let mut scratch = MeshRenderScratch::default();

        // Frame 1: 5 instances across 2 meshes.
        let big: Vec<InstanceModelCol> = (0..5).map(|i| affine(i % 2, i)).collect();
        let big_inputs: Vec<(u32, &InstanceModelCol)> =
            big.iter().enumerate().map(|(i, c)| ((i as u32) % 2, c)).collect();
        scratch.gather_into(2, meta, || big_inputs.iter().copied());
        assert_eq!(scratch.instance_count(), 5);
        let ring_cap_after_big = scratch.ring.capacity();

        // Frame 2: 1 instance of mesh 0.
        let small = affine(0, 0);
        let small_inputs: Vec<(u32, &InstanceModelCol)> = vec![(0, &small)];
        scratch.gather_into(2, meta, || small_inputs.iter().copied());
        assert_eq!(scratch.batch_count(), 1);
        assert_eq!(scratch.instance_count(), 1);
        // The capacity did not shrink — the smaller frame reused the big frame's ring.
        assert!(
            scratch.ring.capacity() >= ring_cap_after_big,
            "the ring retains its reserved capacity across a smaller frame"
        );
    }

    // ════════════════════════════════════════════════════════════════════════
    // Pillar B B1 — the pair-emitting gather (96-byte GpuTransform3D ring).
    // ════════════════════════════════════════════════════════════════════════

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

    /// The pair gather mirrors the affine gather's bucketing: two meshes bucket into
    /// one batch each, mesh B's `base_instance` is NONZERO and equals mesh A's count,
    /// each bucket's PAIRS are contiguous + non-overlapping in `pair_ring`, and the
    /// batch metadata is byte-identical to the affine path (the recorder is agnostic).
    #[test]
    fn pair_bucketing_two_meshes_nonzero_base_contiguous() {
        let mesh_count = 2;
        // Interleave so the scatter (not input order) produces contiguous buckets.
        let a0 = pair(0, 0);
        let a1 = pair(0, 1);
        let a2 = pair(0, 2);
        let b0 = pair(1, 0);
        let b1 = pair(1, 1);
        let inputs: Vec<(u32, &GpuTransform3D)> =
            vec![(0, &a0), (1, &b0), (0, &a1), (1, &b1), (0, &a2)];

        let mut scratch = MeshRenderScratch::default();
        scratch.gather_pairs_into(mesh_count, meta, || inputs.iter().copied());

        // Batch metadata identical to the affine path (mesh-A base 0 / 3, mesh-B base
        // 3 / 2, alternating width).
        assert_eq!(scratch.batch_count(), 2, "two distinct meshes => two batches");
        let ba = scratch.batches[0];
        assert_eq!((ba.mesh_id, ba.base_instance, ba.instance_count), (0, 0, 3));
        assert_eq!((ba.index_count, ba.index_type), (6, IndexType::Uint16));
        let bb = scratch.batches[1];
        assert_eq!(
            (bb.mesh_id, bb.base_instance, bb.instance_count),
            (1, 3, 2),
            "mesh B's base == count(A) == 3 (NONZERO — draw-ordered like the affine ring)"
        );
        assert_eq!((bb.index_count, bb.index_type), (12, IndexType::Uint32));

        // The pair ring holds every pair, contiguous per bucket, each slot the
        // EXPECTED pair (prev + curr both correct — no swap, no overlap).
        assert_eq!(scratch.pair_ring.len(), 5, "the pair ring holds every pair");
        for ord in 0..3u32 {
            let slot = (ba.base_instance + ord) as usize;
            assert_eq!(scratch.pair_ring[slot], pair(0, ord), "mesh A pair {ord}");
        }
        for ord in 0..2u32 {
            let slot = (bb.base_instance + ord) as usize;
            assert_eq!(scratch.pair_ring[slot], pair(1, ord), "mesh B pair {ord}");
        }
    }

    /// The pair gather over an empty input emits zero batches + an empty pair ring
    /// (the interp-off / no-instance 0%-gate).
    #[test]
    fn pair_bucketing_empty_yields_no_batches() {
        let mut scratch = MeshRenderScratch::default();
        let inputs: Vec<(u32, &GpuTransform3D)> = Vec::new();
        scratch.gather_pairs_into(3, meta, || inputs.iter().copied());
        assert_eq!(scratch.batch_count(), 0);
        assert_eq!(scratch.pair_ring.len(), 0);
    }

    /// A gap mesh: mesh 1 has zero pairs, so mesh 2's `base_instance` skips its
    /// (zero) bucket — the prefix-sum is over the actual counts, identical to the
    /// affine path's `bucketing_skips_empty_mesh_in_the_middle`.
    #[test]
    fn pair_bucketing_skips_empty_mesh_in_the_middle() {
        let a0 = pair(0, 0);
        let a1 = pair(0, 1);
        let c0 = pair(2, 0);
        let inputs: Vec<(u32, &GpuTransform3D)> = vec![(0, &a0), (2, &c0), (0, &a1)];

        let mut scratch = MeshRenderScratch::default();
        scratch.gather_pairs_into(3, meta, || inputs.iter().copied());

        assert_eq!(scratch.batch_count(), 2, "mesh 1 is empty => only 2 batches");
        assert_eq!(
            (scratch.batches[0].mesh_id, scratch.batches[0].base_instance),
            (0, 0)
        );
        // Mesh 2's base == count(0) + count(1) == 2 + 0 == 2.
        assert_eq!(
            (scratch.batches[1].mesh_id, scratch.batches[1].base_instance),
            (2, 2)
        );
        assert_eq!(scratch.pair_ring[2], pair(2, 0), "mesh 2's lone pair at slot 2");
    }

    /// The affine ring and the pair ring share the SAME lanes without corrupting each
    /// other across successive gathers on ONE reused scratch: a pair gather then an
    /// affine gather (or vice-versa) each produces the correct ring, and the OTHER
    /// ring keeps its capacity (Principle 5 — both lanes persist).
    #[test]
    fn pair_and_affine_rings_coexist_on_one_scratch() {
        let mut scratch = MeshRenderScratch::default();

        // Pair gather: 3 pairs across 2 meshes.
        let p: Vec<GpuTransform3D> = (0..3).map(|i| pair(i % 2, i)).collect();
        let p_in: Vec<(u32, &GpuTransform3D)> =
            p.iter().enumerate().map(|(i, r)| ((i as u32) % 2, r)).collect();
        scratch.gather_pairs_into(2, meta, || p_in.iter().copied());
        assert_eq!(scratch.pair_ring.len(), 3);
        let pair_cap = scratch.pair_ring.capacity();

        // Affine gather next on the SAME scratch: 2 affines of mesh 0.
        let a0 = affine(0, 0);
        let a1 = affine(0, 1);
        let a_in: Vec<(u32, &InstanceModelCol)> = vec![(0, &a0), (0, &a1)];
        scratch.gather_into(2, meta, || a_in.iter().copied());
        assert_eq!(scratch.instance_count(), 2, "affine ring correct after a pair gather");
        // The pair ring retained its reserved capacity (the lanes are independent).
        assert!(
            scratch.pair_ring.capacity() >= pair_cap,
            "the pair ring retains its capacity across an affine gather"
        );
    }
}
