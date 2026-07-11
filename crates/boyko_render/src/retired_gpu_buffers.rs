//! [`RetiredGpuBuffers`] — the fence-gated device-free queue for a GROWN GPU mirror's
//! superseded buffer (asset-streaming plan F7 §4/§7.4).
//!
//! Grow-and-defer-old (F7) replaces a GPU-mirrored SSBO with a larger one in place
//! (no `vkDeviceWaitIdle`, no device→device copy) and routes the OLD buffer here,
//! stamped with the same `submission_epoch + FRAMES_IN_FLIGHT` fence horizon the F6
//! `DeferredFree`/[`OrphanedMeshGpu`](crate::mesh_assets::OrphanedMeshGpu) queues use —
//! `retire_deferred_frees` drains this queue on the SAME `epoch` gate as every other F6
//! device-free. `BoundBuffer` is `!Send` (an RHI device handle), so this queue cannot
//! live in the `Send` `DeferredFree`/`FreeEntry` (`boyko_scene`, which cannot depend on
//! the Vulkan backend) — a dedicated `!Send` `NonSendResource` is therefore type-forced,
//! mirroring `OrphanedMeshGpu`'s shape exactly. Asset-streaming plan F7-hwrt (task#11)
//! adds a SECOND, `hwrt`-gated lane (`tlases`) for a superseded PERSISTENT TLAS (the
//! RT-leg instance-family grow's AS-side counterpart) — `PersistentTlas` is `!Send` too
//! (it owns a `BoundAccelStruct`), so both lanes share this one queue's `!Send` contract.

use boyko_ecs::ecs::core::resources::resource::NonSendResource;
use boyko_rhi::RhiDevice;
use boyko_rhi_vulkan::device::VulkanContext;
use boyko_rhi_vulkan::memory::BoundBuffer;
#[cfg(feature = "hwrt")]
use boyko_rhi_vulkan::accel_build::{PersistentTlas, destroy_persistent_tlas};

/// One retired buffer awaiting its fence horizon: the `!Send` device handle + the F6-style
/// `retire_frame` stamp (`submission_epoch` at replacement time `+ RETIRE_DELAY`).
struct RetiredBuffer {
    buf: BoundBuffer,
    retire_frame: u64,
}

/// HW-RT rung (asset-streaming plan F7-hwrt, task#11): one retired PERSISTENT TLAS
/// awaiting its fence horizon — the AS-side counterpart of [`RetiredBuffer`]. A SEPARATE
/// lane (not folded into [`RetiredGpuBuffers::entries`]) because a [`PersistentTlas`]
/// bundles THREE owned resources (the AS handle + its backing buffer + its build scratch)
/// that must be freed AS-FIRST via [`destroy_persistent_tlas`], not a single `BoundBuffer`
/// destroy.
#[cfg(feature = "hwrt")]
struct RetiredTlas {
    tlas: PersistentTlas,
    retire_frame: u64,
}

/// The fence-gated queue of superseded GPU-mirror buffers awaiting device-free
/// (asset-streaming plan F7 §4). A genuine `Vec` (not a `[Option<BoundBuffer>; FIF]` slot
/// array): a multi-grow-per-FIF-window (F7 §8 FIX-F) can enqueue more than one entry before
/// any of them reaches its horizon, and each carries its OWN `retire_frame` from its OWN
/// replacement epoch — a fixed-size slot array would let a second grow overwrite a still-
/// pending first one. This is the sanctioned GPU-teardown-queue exception (identical in
/// kind to [`OrphanedMeshGpu`](crate::mesh_assets::OrphanedMeshGpu)'s `orphans: Vec<_>`),
/// not a parallel gameplay data store (Principle 0).
#[derive(Default)]
pub struct RetiredGpuBuffers {
    entries: Vec<RetiredBuffer>,
    /// HW-RT rung (task#11): the TLAS-side retire lane, parallel to `entries` — a per-slot
    /// TLAS grow (`gpu_scene::tlas::TlasResources::grow_slot`) routes the superseded
    /// [`PersistentTlas`] here instead of `entries` (it is not a `BoundBuffer`). Drained by
    /// [`Self::drain_ready`]/[`Self::drain_all`] on the SAME fence-gate as `entries`.
    #[cfg(feature = "hwrt")]
    tlases: Vec<RetiredTlas>,
}

impl NonSendResource for RetiredGpuBuffers {}

impl RetiredGpuBuffers {
    /// Queues `buf` for device-free once the host observes `epoch >= retire_frame` — the
    /// caller stamps `retire_frame` as `submission_epoch_at_replacement + RETIRE_DELAY`
    /// (mirrors [`DeferredFree::push`](boyko_scene::DeferredFree::push) /
    /// [`OrphanedMeshGpu::push`](crate::mesh_assets::OrphanedMeshGpu::push)).
    #[inline]
    pub fn push(&mut self, buf: BoundBuffer, retire_frame: u64) {
        self.entries.push(RetiredBuffer { buf, retire_frame });
    }

    /// Queues `t` (a superseded PERSISTENT TLAS — the RT-leg counterpart of [`Self::push`],
    /// asset-streaming plan F7-hwrt, task#11) for device-free once the host observes
    /// `epoch >= retire_frame`.
    ///
    /// # Why `retire_frame = epoch + RETIRE_DELAY` is safe for a per-frame-REBUILT TLAS
    ///
    /// `gpu_scene::tlas::TlasResources` rebuilds `tlas[s].accel` into every frame slot `s`
    /// uses (unlike a write-once buffer, this AS is re-recorded-into via
    /// `cmd_build_acceleration_structures` and re-traced by the resolve's `rayQuery` EVERY
    /// time slot `s` is used) — but a grow that REPLACES it only ever fires for slot `s`
    /// INSIDE that slot's own fenced window: the caller (`TlasResources::grow_slot`) runs
    /// after `wait_frame_in_flight(s)` for THIS frame, so slot `s`'s LAST submission (its
    /// pre-grow build + trace) has already completed. The SIBLING in-flight frame traces
    /// `tlas[1 - s].accel`, a DIFFERENT `PersistentTlas` entirely — it never references the
    /// one being retired here. Immediately after the replacement, `repoint_tlas_accel` (the
    /// SAME fenced frame) rewrites every resolve-family descriptor set's AS binding away
    /// from the old handle, so no descriptor set anywhere still points at it either — the
    /// OLD `tlas.accel`'s only possible GPU references were slot `s`'s own already-fence-
    /// waited submissions. `retire_frame = epoch + RETIRE_DELAY` (== `epoch +
    /// FRAMES_IN_FLIGHT`) is therefore safe with slack to spare, exactly like every other
    /// F6/F7 fence-gated retire.
    #[cfg(feature = "hwrt")]
    #[inline]
    pub fn push_tlas(&mut self, t: PersistentTlas, retire_frame: u64) {
        self.tlases.push(RetiredTlas { tlas: t, retire_frame });
    }

    /// `true` iff no retired buffer/TLAS is awaiting teardown — the O(1) golden early-out
    /// (a scene whose GPU mirrors never outgrow their boot capacity never pushes here).
    /// F7-hwrt (task#11) extends this to the `tlases` lane: a growth-only frame with an
    /// empty `entries` but a pending retired TLAS must still report non-empty, or
    /// `retire_deferred_frees`'s early-out (which reads only this fn) would leak it —
    /// mirrors the F7 C2 fix's "extend, don't narrow" reasoning for the SAME early-out.
    #[inline]
    pub fn is_empty(&self) -> bool {
        #[cfg(feature = "hwrt")]
        {
            self.entries.is_empty() && self.tlases.is_empty()
        }
        #[cfg(not(feature = "hwrt"))]
        {
            self.entries.is_empty()
        }
    }

    /// Destroys every entry whose `retire_frame <= epoch`, retaining the rest. Uses a
    /// `swap_remove` scan (O(1) per removal) rather than `Vec::remove` (O(n) shift) — free
    /// order is irrelevant here, mirroring the F6 `OrphanedMeshGpu::drain_ready` shape
    /// (asset-streaming plan F7 O5).
    ///
    /// # Safety
    ///
    /// The caller has waited THIS `epoch`'s fence (via `Renderer::wait_frame_in_flight`,
    /// the same F6 fence-gate precondition `retire_deferred_frees`'s other drains rely on)
    /// — every submit that could reference an entry with `retire_frame <= epoch` is
    /// GPU-complete. `ctx` must be the live context every queued buffer was created on.
    pub unsafe fn drain_ready(&mut self, epoch: u64, ctx: &VulkanContext) {
        let mut i = 0;
        while i < self.entries.len() {
            if self.entries[i].retire_frame > epoch {
                i += 1;
                continue;
            }
            // `swap_remove` moves the last element into slot `i`, which must still be
            // checked this pass — do not advance `i` here.
            let entry = self.entries.swap_remove(i);
            // SAFETY: `entry.retire_frame <= epoch` (checked above) — the caller's fence-
            // wait contract (this fn's `# Safety`) guarantees every submit that could
            // reference `entry.buf` is GPU-complete; `swap_remove` yields it exactly once,
            // so the by-value destroy frees it exactly once.
            unsafe { ctx.destroy_buffer(entry.buf) };
        }

        // HW-RT rung (task#11): the TLAS lane, same scan shape, SAME fence-gate contract.
        #[cfg(feature = "hwrt")]
        {
            let mut i = 0;
            while i < self.tlases.len() {
                if self.tlases[i].retire_frame > epoch {
                    i += 1;
                    continue;
                }
                let entry = self.tlases.swap_remove(i);
                // SAFETY: `entry.retire_frame <= epoch` (checked above) — the caller's
                // fence-wait contract (this fn's `# Safety`) guarantees every submit that
                // could reference `entry.tlas` is GPU-complete (see `push_tlas`'s doc for
                // the per-slot rebuild proof); `swap_remove` yields it exactly once, so the
                // by-value destroy (AS-first-then-backing-then-scratch) frees it exactly
                // once.
                unsafe { destroy_persistent_tlas(ctx, entry.tlas) };
            }
        }
    }

    /// Destroys EVERY remaining entry regardless of its horizon — the teardown drain
    /// (asset-streaming plan F7 O1), mirroring `OrphanedMeshGpu`'s shutdown discipline.
    ///
    /// # Safety
    ///
    /// The caller has made the device idle (e.g. the renderer's `Drop` `vkDeviceWaitIdle`)
    /// so no in-flight submission references any queued buffer; `ctx` is the live context
    /// every entry was created on.
    pub unsafe fn drain_all(&mut self, ctx: &VulkanContext) {
        for entry in self.entries.drain(..) {
            // SAFETY: per this fn's contract the device is idle, so no submission
            // references `entry.buf`; `Vec::drain` yields each entry exactly once, so the
            // by-value destroy frees it exactly once.
            unsafe { ctx.destroy_buffer(entry.buf) };
        }

        // HW-RT rung (task#11): the TLAS lane, same contract.
        #[cfg(feature = "hwrt")]
        for entry in self.tlases.drain(..) {
            // SAFETY: per this fn's contract the device is idle, so no submission
            // references `entry.tlas`; `Vec::drain` yields each entry exactly once, so the
            // by-value destroy (AS-first-then-backing-then-scratch) frees it exactly once.
            unsafe { destroy_persistent_tlas(ctx, entry.tlas) };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A device-inert `BoundBuffer` — mirrors `asset_streaming_f5_validation.rs`'s
    /// `dummy_mesh_gpu` idiom (`BoundBuffer`'s fields are all `pub`): no test in this
    /// module ever calls a Vulkan function on it. `drain_ready`/`drain_all` — the only
    /// methods that WOULD destroy a buffer — need a live `&VulkanContext`, which has no
    /// testable constructor outside a real device boot (the same constraint
    /// `asset_refcount.rs`'s churn-stress test documents for `retire_deferred_frees`),
    /// so this module tests `push`/`is_empty` directly against the real type, and tests
    /// the selection ALGORITHM (which entries a given epoch would drain) via a small
    /// verbatim mirror of `drain_ready`'s scan (see `SelectionModel` below) — the
    /// "small extracted helper" the F7 test plan allows when a method cannot be split
    /// from its device call. `offset` is repurposed here as a plain identity tag
    /// (never a real memory offset).
    fn dummy_buffer(id: u64) -> BoundBuffer {
        BoundBuffer { buffer: boyko_rhi_vulkan::ffi::VkBuffer::NULL, offset: id, size: 0, mapped: None }
    }

    #[test]
    fn new_queue_is_empty() {
        let q = RetiredGpuBuffers::default();
        assert!(q.is_empty(), "a freshly-constructed queue holds no entries");
    }

    #[test]
    fn push_flips_is_empty_to_false() {
        let mut q = RetiredGpuBuffers::default();
        q.push(dummy_buffer(1), 10);
        assert!(!q.is_empty(), "a single push must flip is_empty to false");
    }

    #[test]
    fn pushing_multiple_entries_keeps_the_queue_not_empty() {
        let mut q = RetiredGpuBuffers::default();
        for i in 0..5 {
            q.push(dummy_buffer(i), i * 2);
        }
        assert!(!q.is_empty(), "multiple pushes must not spuriously report empty");
    }

    // ════════════════════════════════════════════════════════════════════
    // `drain_ready`'s selection algorithm — an ORACLE MODEL, not the real
    // method (see this module's doc for why `drain_ready` itself is untestable
    // without a device). `SelectionModel` is a byte-for-byte copy of
    // `drain_ready`'s swap_remove scan, operating on a bare `retire_frame: u64`
    // instead of a full `RetiredBuffer` — it proves the SELECTION PREDICATE
    // (which entries drain at a given epoch) is correct, exhaustively over many
    // random pushes/epochs, without touching a device.
    // ════════════════════════════════════════════════════════════════════

    /// Mirrors `RetiredGpuBuffers::drain_ready`'s scan verbatim (same
    /// swap_remove-without-advancing-`i`-on-a-hit shape), operating on a
    /// `Vec<u64>` of bare `retire_frame`s.
    struct SelectionModel {
        entries: Vec<u64>,
    }
    impl SelectionModel {
        fn drain_ready(&mut self, epoch: u64) -> Vec<u64> {
            let mut drained = Vec::new();
            let mut i = 0;
            while i < self.entries.len() {
                if self.entries[i] > epoch {
                    i += 1;
                    continue;
                }
                drained.push(self.entries.swap_remove(i));
            }
            drained
        }
    }

    fn sorted(mut v: Vec<u64>) -> Vec<u64> {
        v.sort_unstable();
        v
    }

    #[test]
    fn oracle_model_drains_nothing_below_every_horizon() {
        let mut m = SelectionModel { entries: vec![10, 20, 30] };
        let drained = m.drain_ready(5);
        assert!(drained.is_empty(), "epoch below every retire_frame must drain nothing");
        assert_eq!(sorted(m.entries), vec![10, 20, 30], "no entry may be consumed");
    }

    #[test]
    fn oracle_model_drains_exactly_at_the_boundary() {
        let mut m = SelectionModel { entries: vec![10] };
        let drained = m.drain_ready(10);
        assert_eq!(drained, vec![10], "epoch == retire_frame must drain (inclusive boundary)");
        assert!(m.entries.is_empty());
    }

    #[test]
    fn oracle_model_drains_past_the_horizon() {
        let mut m = SelectionModel { entries: vec![10] };
        let drained = m.drain_ready(999);
        assert_eq!(drained, vec![10], "epoch > retire_frame must still drain the entry");
    }

    #[test]
    fn oracle_model_on_an_empty_queue_drains_nothing() {
        let mut m = SelectionModel { entries: Vec::new() };
        assert!(m.drain_ready(1_000_000).is_empty());
    }

    #[test]
    fn oracle_model_mixed_horizons_drain_only_the_ready_subset() {
        let mut m = SelectionModel { entries: vec![5, 20, 5, 100, 5] };
        let drained = m.drain_ready(5);
        assert_eq!(sorted(drained), vec![5, 5, 5], "only the three ready (<=5) entries must drain");
        assert_eq!(sorted(m.entries), vec![20, 100], "the two not-yet-ready entries must remain queued");
    }

    /// A tiny deterministic xorshift32 PRNG — matches the reproducible-churn idiom
    /// `asset_refcount.rs`'s own tests use (no new `proptest`/`rand` dev-dependency
    /// for this crate, which has neither today).
    struct Xorshift32(u32);
    impl Xorshift32 {
        fn next_u32(&mut self) -> u32 {
            let mut x = self.0;
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            self.0 = x;
            x
        }
    }

    /// Oracle property: for MANY random `(retire_frames, epoch)` draws, the scan
    /// must select EXACTLY the entries with `retire_frame <= epoch` — each exactly
    /// once, none skipped, order-irrelevant (a `swap_remove` scan reorders) — and
    /// leave exactly the complement queued. 512 trials, comfortably past the
    /// ">100 cases" bar for a property test over this input domain (mirrors
    /// `asset_refs.rs`'s `drain_ready_matches_the_retire_frame_le_epoch_model`
    /// proptest, without adding a `proptest` dev-dependency to this crate).
    #[test]
    fn oracle_model_selects_exactly_retire_frame_le_epoch_over_many_random_trials() {
        let mut rng = Xorshift32(0xF00D_CAFE);
        for trial in 0..512 {
            let n = (rng.next_u32() as usize) % 64;
            let entries: Vec<u64> = (0..n).map(|_| u64::from(rng.next_u32() % 50)).collect();
            let epoch = u64::from(rng.next_u32() % 50);

            let expected_drained = sorted(entries.iter().copied().filter(|&rf| rf <= epoch).collect());
            let expected_remaining = sorted(entries.iter().copied().filter(|&rf| rf > epoch).collect());

            let mut m = SelectionModel { entries: entries.clone() };
            let drained = m.drain_ready(epoch);

            assert_eq!(
                sorted(drained),
                expected_drained,
                "trial {trial} (entries={entries:?}, epoch={epoch}): drained set must equal the \
                 retire_frame<=epoch filter"
            );
            assert_eq!(
                sorted(m.entries),
                expected_remaining,
                "trial {trial} (entries={entries:?}, epoch={epoch}): remaining set must equal the \
                 retire_frame>epoch complement"
            );
        }
    }
}
