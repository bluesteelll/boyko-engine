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
    pub fn gather_into<'a, M, F, I>(&mut self, mesh_count: usize, mut meta: M, iter_input: F)
    where
        M: FnMut(u32) -> (u32, IndexType),
        F: Fn() -> I,
        I: Iterator<Item = (u32, &'a InstanceModelCol)>,
    {
        // --- Pass 1: count per mesh (touches only the small MeshHandle key). ---
        fit_len(&mut self.counts, mesh_count, 0);
        {
            let counts = &mut self.counts;
            for (mesh_id, _col) in iter_input() {
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

        // --- Pass 2: scatter each instance's 48-byte affine into ring[offsets[m] +
        // cursors[m]++] — contiguous per bucket, non-overlapping. ---
        self.ring.clear();
        self.ring.resize(running as usize, InstanceModelCol::zeroed());
        {
            let offsets = &self.offsets;
            let cursors = &mut self.cursors;
            let ring = &mut self.ring;
            for (mesh_id, col) in iter_input() {
                let m = mesh_id as usize;
                let slot = offsets[m] + cursors[m];
                cursors[m] += 1;
                ring[slot as usize] = *col;
            }
        }
        debug_assert_eq!(
            self.ring.len(),
            running as usize,
            "invariant: the ring holds exactly Σ instance_count instances"
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
