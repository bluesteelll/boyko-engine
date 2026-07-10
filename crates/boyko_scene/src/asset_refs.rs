//! Asset refcount lifetime plumbing (asset-streaming plan F2 §1/§3).
//!
//! [`MeshHandle`](crate::render_caps::MeshHandle) /
//! [`MaterialHandle`](crate::render_caps::MaterialHandle) carrier hooks push a
//! [`RefDelta`] into [`RefcountDeltas`] whenever a carrier attaches/detaches
//! from an entity; a `boyko_render` apply system drains the buffer each
//! frame, folds each delta into the matching `Assets<T>` table (`inc_ref` /
//! `dec_ref`), and enqueues any resulting retire into [`DeferredFree`]. This
//! module owns the two resources + their POD payload types — it is reachable
//! from BOTH the hook side (this crate) and the apply-system side
//! (`boyko_render`, which depends on `boyko_scene`).
//!
//! # `AssetRefKind` vs. `boyko_ecs`'s `AssetKind` (F1) — one representation,
//! two owners, by necessity
//!
//! `boyko_ecs::ecs::core::asset::assets::AssetKind` wraps a `ComponentId`
//! minted by `T::register_layout()` — constructing one requires the concrete
//! asset type (`MeshGpu`/`MaterialGpu`) in scope. Those types live in
//! `boyko_render`, which `boyko_scene` must not depend on (wrong direction —
//! `boyko_render` depends on `boyko_scene`, never the reverse). So a carrier
//! hook defined HERE cannot mint a `boyko_ecs::AssetKind`. [`AssetRefKind`] is
//! the small, hook-local routing tag this crate CAN construct (it only needs
//! to know "mesh or material", not the concrete GPU type); the apply system
//! matches on it to pick which `Assets<T>` table to call.
//!
//! Since `Assets::dec_ref`'s `RetireTicket`
//! already carries the store's own `AssetKind`, the apply system has both tags
//! available when it enqueues a [`FreeEntry`] — it stamps the [`AssetRefKind`]
//! it already matched on (the same value the `RefDelta` carried in), not the
//! store's `AssetKind` (which would require threading a second, foreign type
//! through this crate's queue for no benefit).

use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_macros::Resource;

/// Which `Assets<T>` table a [`RefDelta`] / [`FreeEntry`] routes to — the
/// hook-local counterpart of `boyko_ecs`'s internal `AssetKind` (see the
/// module doc for why the two are distinct types).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssetRefKind {
    /// Routes to `Assets<MeshGpu>` (`boyko_render`).
    Mesh,
    /// Routes to `Assets<MaterialGpu>` (`boyko_render`).
    Material,
}

/// A single refcount delta pushed by a carrier hook (asset-streaming plan F2
/// §1, gen-checked as of F5 §Decision 3). POD, `Send`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RefDelta {
    /// The entity whose carrier pushed this delta — needed so the `+1` apply
    /// path (asset-streaming plan F5) can stamp the carrier's
    /// `MeshRefGen`/`MaterialRefGen` lane via `Commands::entity(entity)`. On
    /// despawn the entity is gone by apply time (the delta is still valid —
    /// only the lane re-sync becomes a moot no-op, never reached for a `-1`).
    pub entity: Entity,
    /// Which store `slot` addresses.
    pub kind: AssetRefKind,
    /// The dense row the delta targets — a carrier's raw index
    /// (`MeshHandle(u32)` / `MaterialHandle(u16)` widened to `u32`).
    pub slot: u32,
    /// The carrier's BIND-time generation, captured by `on_replace` from the
    /// sibling `MeshRefGen`/`MaterialRefGen` lane while the row is still live
    /// (asset-streaming plan F5 §Decision 3) — `apply_refcount_deltas` passes
    /// this to `Assets::dec_ref`'s gen-check. Unused (set to
    /// `boyko_ecs::ecs::core::asset::GEN_UNSYNCED`) on a `+1` delta: the
    /// attach path re-derives the CURRENT generation directly from the store,
    /// not from this field.
    pub gen_: u32,
    /// `+1` on attach (`on_insert`), `-1` on detach (`on_replace`, which
    /// fires on BOTH an in-place overwrite and a genuine removal/despawn —
    /// see `render_caps.rs`'s hook wiring for why `on_remove` is NOT also
    /// wired to push a delta).
    pub delta: i32,
}

impl RefDelta {
    /// Constructs a delta. `const fn` — the hook call sites build one and
    /// push it in the same statement, no runtime cost beyond the `Vec::push`.
    ///
    /// `gen_` (not `gen`) — `gen` is a reserved keyword as of the 2024 edition.
    #[inline]
    pub const fn new(entity: Entity, kind: AssetRefKind, slot: u32, gen_: u32, delta: i32) -> Self {
        Self { entity, kind, slot, gen_, delta }
    }
}

/// World-global queue of [`RefDelta`]s awaiting the per-frame apply system
/// (asset-streaming plan F2 §1). A carrier hook only ever
/// [`push`](Self::push)es; the apply system
/// [`drain`](Self::drain)s it once per frame, in push (FIFO) order — mirrors
/// `boyko_ecs`'s `AssetStaging<A>` handoff-queue shape.
///
/// A plain [`Resource`] (`Send + Sync`): [`RefDelta`] is POD, so the buffer
/// carries no `!Sync` payload the way `AssetStaging<A>` does for a decoded
/// CPU intermediate.
#[derive(Default, Resource)]
pub struct RefcountDeltas {
    deltas: Vec<RefDelta>,
}

impl RefcountDeltas {
    /// Queues a delta. Called ONLY from a carrier hook body (the sanctioned
    /// `on_insert`/`on_replace`-pushes-to-a-resource pattern via
    /// `DeferredEcsMaster::resource_mut`).
    #[inline]
    pub fn push(&mut self, delta: RefDelta) {
        self.deltas.push(delta);
    }

    /// Drains every queued delta, in FIFO order, for the apply system to fold
    /// into the matching `Assets<T>` table.
    #[inline]
    pub fn drain(&mut self) -> impl Iterator<Item = RefDelta> + '_ {
        self.deltas.drain(..)
    }

    /// `true` if no delta is awaiting the apply system.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.deltas.is_empty()
    }
}

/// A row awaiting the fence-gated device-free pass (asset-streaming plan F3:
/// GPU-mirror growth's per-buffer routing shares this shape; F6 wires the
/// actual device teardown). F2 only defines the queue and enqueues from an
/// `Assets::dec_ref`-returned `RetireTicket` — nothing drains it yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FreeEntry {
    /// Which store `slot` addresses.
    pub kind: AssetRefKind,
    /// The dense row awaiting retire.
    pub slot: u32,
    /// The monotonic renderer frame at/after which the retire is fence-safe
    /// (F6 sets the real `frame_index + FIF` gate; F2 stamps a placeholder
    /// `0` — see [`DeferredFree::push`]).
    pub retire_frame: u64,
}

/// World-global queue of rows retired-but-not-yet-freed (asset-streaming plan
/// F2 §3 / F6). F2 defines the queue and enqueues on every
/// `Assets::dec_ref`-returned `RetireTicket`; nothing drains it until F6's
/// fence-gated `retire_deferred_frees` lands.
///
/// A plain [`Resource`] (`Send + Sync`): [`FreeEntry`] is POD.
#[derive(Default, Resource)]
pub struct DeferredFree {
    entries: Vec<FreeEntry>,
}

impl DeferredFree {
    /// Enqueues a row awaiting retire.
    #[inline]
    pub fn push(&mut self, entry: FreeEntry) {
        self.entries.push(entry);
    }

    /// Every currently-queued entry, in enqueue order (test/inspection
    /// surface — F6 replaces this with the real drain-by-`retire_frame`
    /// pass).
    #[inline]
    pub fn entries(&self) -> &[FreeEntry] {
        &self.entries
    }

    /// `true` if no row is awaiting retire.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Moves every entry with `retire_frame <= epoch` out of the queue and
    /// into `out` (cleared first), preserving enqueue (FIFO) order; entries
    /// not yet fence-safe stay queued (asset-streaming plan F6: the
    /// fence-gate). `out` is a caller-owned, host-parked scratch buffer
    /// reused every frame — zero steady-state allocation on the churn-free
    /// (golden) path, where this is called on an already-empty queue and
    /// returns immediately.
    pub fn drain_ready(&mut self, epoch: u64, out: &mut Vec<FreeEntry>) {
        out.clear();
        let mut i = 0;
        while i < self.entries.len() {
            if self.entries[i].retire_frame <= epoch {
                out.push(self.entries[i]);
                // `FreeEntry` is `Copy` and the queue is churn-small (bounded
                // by refcount zero-crossings per fence window) — an
                // order-preserving `remove` (not `swap_remove`) keeps `out`
                // in FIFO order, which the mesh/material retire passes rely
                // on for no particular reason today but is cheap to keep.
                self.entries.remove(i);
            } else {
                i += 1;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refcount_deltas_push_then_drain_yields_fifo_order() {
        use boyko_ecs::ecs::identifiers::primitives::EntityId;

        let e = Entity::new(EntityId(0), 0);
        let mut deltas = RefcountDeltas::default();
        deltas.push(RefDelta::new(e, AssetRefKind::Mesh, 1, 0, 1));
        deltas.push(RefDelta::new(e, AssetRefKind::Material, 2, 0, -1));

        let drained: Vec<RefDelta> = deltas.drain().collect();

        assert_eq!(drained.len(), 2, "both pushed deltas must drain");
        assert_eq!(drained[0].slot, 1, "drain preserves FIFO push order");
        assert_eq!(drained[1].slot, 2);
        assert!(deltas.is_empty(), "drain must empty the queue");
    }

    #[test]
    fn deferred_free_push_records_the_entry() {
        let mut free = DeferredFree::default();
        assert!(free.is_empty());

        free.push(FreeEntry { kind: AssetRefKind::Mesh, slot: 3, retire_frame: 0 });

        assert!(!free.is_empty());
        assert_eq!(free.entries().len(), 1);
        assert_eq!(free.entries()[0].slot, 3);
    }

    // ════════════════════════════════════════════════════════════════════
    // `DeferredFree::drain_ready` — the F6 fence-gate. A row stamped
    // `retire_frame = N` must NEVER drain for any epoch `M < N`, MUST drain
    // at `M == N` (the fence-gate proof's own boundary — `retire_deferred_frees`
    // never calls this with `M < N` for a real row, but the horizon check
    // itself must be `<=`, not `<`, or a real caller would wait one frame
    // longer than the proof requires), and enqueue order (FIFO) is preserved
    // in `out`.
    // ════════════════════════════════════════════════════════════════════

    fn entry(slot: u32, retire_frame: u64) -> FreeEntry {
        FreeEntry { kind: AssetRefKind::Mesh, slot, retire_frame }
    }

    /// A row stamped `retire_frame = N` must NOT drain for any `epoch < N` —
    /// the CPU-side half of the fence-gate proof (`retire_deferred_frees`'s
    /// doc): draining early would free a resource a submit made before the
    /// horizon could still reference.
    #[test]
    fn drain_ready_does_not_drain_an_entry_below_its_retire_frame() {
        let mut free = DeferredFree::default();
        free.push(entry(7, 10));
        let mut out = Vec::new();

        free.drain_ready(9, &mut out);

        assert!(out.is_empty(), "epoch 9 < retire_frame 10 must not drain the entry");
        assert_eq!(free.entries().len(), 1, "the entry must remain queued");
        assert_eq!(free.entries()[0].slot, 7);
    }

    /// A row stamped `retire_frame = N` MUST drain the instant `epoch == N` —
    /// the fence-gate's own boundary is inclusive (`<=`), not exclusive.
    #[test]
    fn drain_ready_drains_an_entry_exactly_at_its_retire_frame() {
        let mut free = DeferredFree::default();
        free.push(entry(7, 10));
        let mut out = Vec::new();

        free.drain_ready(10, &mut out);

        assert_eq!(out.len(), 1, "epoch == retire_frame must drain the entry");
        assert_eq!(out[0].slot, 7);
        assert!(free.is_empty(), "a drained entry must leave the queue");
    }

    /// A row stamped `retire_frame = N` also drains for any `epoch > N` (a
    /// missed/late drain call, e.g. a frame-index gap, must not strand it
    /// forever — the horizon is a MINIMUM, not an exact match).
    #[test]
    fn drain_ready_drains_an_entry_past_its_retire_frame() {
        let mut free = DeferredFree::default();
        free.push(entry(7, 10));
        let mut out = Vec::new();

        free.drain_ready(999, &mut out);

        assert_eq!(out.len(), 1, "epoch > retire_frame must still drain the entry");
        assert!(free.is_empty());
    }

    /// Mixed horizons: only the ready entries drain, FIFO (enqueue) order is
    /// preserved in `out`, and the not-yet-ready entries stay queued in their
    /// original relative order — `retire_deferred_frees`'s single drain call
    /// per frame must never reorder or skip a still-pending row.
    #[test]
    fn drain_ready_preserves_fifo_order_and_leaves_not_ready_entries_queued() {
        let mut free = DeferredFree::default();
        // Enqueue order: slot 1 (ready), slot 2 (not ready), slot 3 (ready),
        // slot 4 (not ready), slot 5 (ready).
        free.push(entry(1, 5));
        free.push(entry(2, 20));
        free.push(entry(3, 5));
        free.push(entry(4, 100));
        free.push(entry(5, 5));
        let mut out = Vec::new();

        free.drain_ready(5, &mut out);

        assert_eq!(
            out.iter().map(|e| e.slot).collect::<Vec<_>>(),
            vec![1, 3, 5],
            "drained entries must preserve FIFO (enqueue) order"
        );
        assert_eq!(
            free.entries().iter().map(|e| e.slot).collect::<Vec<_>>(),
            vec![2, 4],
            "not-yet-ready entries must remain queued in their original relative order"
        );
    }

    /// `out` is cleared BEFORE being populated — a caller-owned, host-parked
    /// scratch buffer reused every frame must never leak a PRIOR frame's
    /// drained entries into the current frame's result.
    #[test]
    fn drain_ready_clears_out_before_populating() {
        let mut free = DeferredFree::default();
        free.push(entry(9, 1));
        let mut out = vec![entry(999, 0), entry(998, 0)];

        free.drain_ready(1, &mut out);

        assert_eq!(
            out.iter().map(|e| e.slot).collect::<Vec<_>>(),
            vec![9],
            "out must hold ONLY this call's drained entries, not a prior frame's leftovers"
        );
    }

    /// Edge case: draining an already-empty queue is a no-op that also
    /// clears `out` — the O(1) golden-scene early-out path
    /// `retire_deferred_frees` relies on (a scene that never lets an asset's
    /// refcount reach zero never enqueues anything).
    #[test]
    fn drain_ready_on_an_empty_queue_clears_out_and_does_nothing() {
        let mut free = DeferredFree::default();
        let mut out = vec![entry(1, 0)];

        free.drain_ready(1_000_000, &mut out);

        assert!(out.is_empty(), "draining an empty queue must still clear a stale `out`");
        assert!(free.is_empty());
    }

    /// proptest oracle: for ANY sequence of `(slot, retire_frame)` pushes and
    /// ANY probe `epoch`, `drain_ready` must (1) drain exactly the entries
    /// with `retire_frame <= epoch`, in their original enqueue order, and (2)
    /// leave exactly the entries with `retire_frame > epoch` queued, also in
    /// their original relative order. The fence-gate's core safety property —
    /// no entry ever drains before its horizon — proven over a wide,
    /// randomized span of horizons and epochs, not just the hand-picked cases
    /// above.
    mod drain_ready_proptest {
        use proptest::prelude::*;

        use super::*;

        proptest! {
            #![proptest_config(ProptestConfig::with_cases(256))]
            #[test]
            fn drain_ready_matches_the_retire_frame_le_epoch_model(
                retire_frames in proptest::collection::vec(0u64..50, 0..64),
                epoch in 0u64..50,
            ) {
                let mut free = DeferredFree::default();
                for (i, &rf) in retire_frames.iter().enumerate() {
                    free.push(entry(i as u32, rf));
                }

                let expected_drained: Vec<u32> = retire_frames
                    .iter()
                    .enumerate()
                    .filter(|&(_, &rf)| rf <= epoch)
                    .map(|(i, _)| i as u32)
                    .collect();
                let expected_remaining: Vec<u32> = retire_frames
                    .iter()
                    .enumerate()
                    .filter(|&(_, &rf)| rf > epoch)
                    .map(|(i, _)| i as u32)
                    .collect();

                let mut out = Vec::new();
                free.drain_ready(epoch, &mut out);

                prop_assert_eq!(
                    out.iter().map(|e| e.slot).collect::<Vec<_>>(),
                    expected_drained,
                    "drained slots must be exactly {{retire_frame <= epoch}}, in enqueue order"
                );
                prop_assert_eq!(
                    free.entries().iter().map(|e| e.slot).collect::<Vec<_>>(),
                    expected_remaining,
                    "remaining slots must be exactly {{retire_frame > epoch}}, in enqueue order"
                );
            }
        }
    }
}
