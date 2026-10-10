//! [`AssetStaging<A>`] — the reserve→decode handoff queue between
//! [`AssetServer::load`](crate::ecs::core::asset::server::AssetServer::load)'s
//! decode step and a later GPU-upload pass (asset-system rung A3a; the
//! upload consumer lands at A3b).

use crate::ecs::core::asset::asset::Asset;
use crate::ecs::core::asset::handle::Handle;
use crate::ecs::core::resources::resource::NonSendResource;

/// One decoded-but-not-yet-uploaded asset: the [`Reserved`](crate::ecs::core::asset::assets::Assets::reserve)
/// row it targets, plus the CPU intermediate
/// [`AssetLoader::decode`](crate::ecs::core::asset::loader::AssetLoader::decode)
/// produced for it.
pub struct Staged<A: Asset> {
    /// The `Reserved` row this entry's consumer must resolve via
    /// [`Assets::fill`](crate::ecs::core::asset::assets::Assets::fill).
    pub handle: Handle<A>,
    /// The decoded CPU intermediate, ready for GPU upload.
    pub cpu: <A as Asset>::Cpu,
}

/// A world-global, per-asset-type queue of decoded assets awaiting GPU
/// upload — the handoff between
/// [`AssetServer::load`](crate::ecs::core::asset::server::AssetServer::load)
/// (which reserves the row and decodes the bytes) and a render-side upload
/// pass (which turns the CPU intermediate into a resident asset and calls
/// [`Assets::fill`](crate::ecs::core::asset::assets::Assets::fill)).
///
/// # `NonSendResource`, not `Resource`
///
/// [`Asset::Cpu`](crate::ecs::core::asset::asset::Asset::Cpu) is bound by
/// `Send` only (not `Send + Sync`, see that trait's doc), so a
/// `Vec<Staged<A>>` is `Send` but not necessarily `Sync` — it cannot satisfy
/// [`Resource`](crate::ecs::core::resources::resource::Resource)'s
/// `Send + Sync` bound in general. Registering it as a [`NonSendResource`]
/// instead (no `Send`/`Sync` bound at all) sidesteps the mismatch: the queue
/// is drained on the main/dispatcher thread by construction — the same
/// single-thread discipline every other `NonSendResource` consumer already
/// follows — so `Sync` access was never needed.
///
/// # Not `#[derive(Default)]`
///
/// A blind `#[derive(Default)]` on a struct generic over `A: Asset` adds a
/// spurious `A: Default` bound to the generated impl (the well-known derive
/// pitfall documented on [`Handle`](crate::ecs::core::asset::handle::Handle)'s
/// own doc for `Clone`/`Copy`/etc.) — even though the only field, `Vec<_>`,
/// is `Default` unconditionally. [`Assets<T>`](crate::ecs::core::asset::assets::Assets)
/// hand-implements `Default` for the identical reason; this type follows the
/// same precedent.
///
/// # Future: a lock-free channel (A5)
///
/// The inner `Vec` is a placeholder for a lock-free MPSC channel once decode
/// moves onto the threadpool (rung A5) — [`push`](Self::push) /
/// [`drain`](Self::drain) / [`is_empty`](Self::is_empty)'s signatures are
/// written to stay unchanged across that swap.
pub struct AssetStaging<A: Asset> {
    queue: Vec<Staged<A>>,
}

impl<A: Asset> Default for AssetStaging<A> {
    fn default() -> Self {
        Self { queue: Vec::new() }
    }
}

impl<A: Asset> AssetStaging<A> {
    /// Queues a decoded asset awaiting upload.
    #[inline]
    pub fn push(&mut self, staged: Staged<A>) {
        self.queue.push(staged);
    }

    /// Drains every queued entry, in FIFO order.
    #[inline]
    pub fn drain(&mut self) -> impl Iterator<Item = Staged<A>> + '_ {
        self.queue.drain(..)
    }

    /// `true` if no decoded asset is awaiting upload.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }
}

impl<A: Asset> NonSendResource for AssetStaging<A> {}

#[cfg(test)]
mod tests {
    use super::*;

    /// An `Asset` whose `Cpu` deliberately has NO `Default` impl — proves
    /// `AssetStaging::<A>::default()` compiles without requiring `A::Cpu:
    /// Default` (plan §A3a unit: default() works without A: Default).
    struct NoDefaultCpu {
        tag: u8,
    }

    struct NoDefaultAsset;
    impl Asset for NoDefaultAsset {
        type Cpu = NoDefaultCpu;
    }

    #[test]
    fn default_is_empty_for_an_asset_with_no_default_impl() {
        let staging = AssetStaging::<NoDefaultAsset>::default();
        assert!(staging.is_empty(), "a freshly defaulted staging queue must be empty");
    }

    /// `push`/`drain` round-trip in FIFO order (plan §A3a unit: push then
    /// drain yields entries in FIFO order).
    #[test]
    fn push_then_drain_yields_entries_in_fifo_order() {
        let mut staging = AssetStaging::<NoDefaultAsset>::default();
        staging.push(Staged { handle: Handle::new(0, 0), cpu: NoDefaultCpu { tag: 1 } });
        staging.push(Staged { handle: Handle::new(1, 0), cpu: NoDefaultCpu { tag: 2 } });
        staging.push(Staged { handle: Handle::new(2, 0), cpu: NoDefaultCpu { tag: 3 } });

        let tags: Vec<u8> = staging.drain().map(|s| s.cpu.tag).collect();

        assert_eq!(tags, vec![1, 2, 3], "drain() must yield entries in push (FIFO) order");
    }

    /// A `Staged` entry's `handle` field survives the push/drain round trip
    /// unchanged — the identity a later `Assets::fill` call resolves against
    /// (plan §A3a unit: drain preserves the staged handle).
    #[test]
    fn drain_preserves_the_staged_handle() {
        let mut staging = AssetStaging::<NoDefaultAsset>::default();
        let handle = Handle::new(7, 3);
        staging.push(Staged { handle, cpu: NoDefaultCpu { tag: 9 } });

        let mut drained = staging.drain();
        let entry = drained.next().expect("one entry was pushed");

        assert_eq!(entry.handle, handle, "drain must not alter the staged handle");
    }

    /// `drain()` empties the queue — a second drain on the same queue yields
    /// nothing (plan §A3a unit: drain empties the queue).
    #[test]
    fn drain_empties_the_queue() {
        let mut staging = AssetStaging::<NoDefaultAsset>::default();
        staging.push(Staged { handle: Handle::new(0, 0), cpu: NoDefaultCpu { tag: 9 } });
        assert!(!staging.is_empty());

        let drained_count = staging.drain().count();

        assert_eq!(drained_count, 1);
        assert!(staging.is_empty(), "drain() must leave the queue empty");
        assert_eq!(staging.drain().count(), 0, "a second drain on an already-emptied queue must yield nothing");
    }

    /// Edge case: `drain()` on a queue that was never pushed to yields an
    /// empty iterator without panicking (plan §A3a unit: drain on an empty
    /// queue).
    #[test]
    fn drain_on_empty_queue_yields_nothing() {
        let mut staging = AssetStaging::<NoDefaultAsset>::default();
        assert_eq!(staging.drain().count(), 0, "draining a never-pushed queue must yield zero entries");
    }
}
