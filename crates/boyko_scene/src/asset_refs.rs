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
/// §1). POD, `Send` — no generation lane (F5 adds one for the gen-checked
/// apply; F2's apply is ungated, matching the F2 scope note in
/// `docs/ASSET-STREAMING-PLAN.md`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RefDelta {
    /// Which store `slot` addresses.
    pub kind: AssetRefKind,
    /// The dense row the delta targets — a carrier's raw index
    /// (`MeshHandle(u32)` / `MaterialHandle(u16)` widened to `u32`).
    pub slot: u32,
    /// `+1` on attach (`on_insert`), `-1` on detach (`on_replace`, which
    /// fires on BOTH an in-place overwrite and a genuine removal/despawn —
    /// see `render_caps.rs`'s hook wiring for why `on_remove` is NOT also
    /// wired to push a delta).
    pub delta: i32,
}

impl RefDelta {
    /// Constructs a delta. `const fn` — the hook call sites build one and
    /// push it in the same statement, no runtime cost beyond the `Vec::push`.
    #[inline]
    pub const fn new(kind: AssetRefKind, slot: u32, delta: i32) -> Self {
        Self { kind, slot, delta }
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refcount_deltas_push_then_drain_yields_fifo_order() {
        let mut deltas = RefcountDeltas::default();
        deltas.push(RefDelta::new(AssetRefKind::Mesh, 1, 1));
        deltas.push(RefDelta::new(AssetRefKind::Material, 2, -1));

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
}
