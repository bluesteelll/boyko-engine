//! HW-RT rung R0 — the GPU timestamp bracket collector for the software-ray passes.
//!
//! [`TimestampCollector`] holds a per-frame-in-flight ring of [`VulkanQueryPool`]s (one
//! `2 * PASS_COUNT`-query pool per slot) and exposes the reset + begin/end brackets the
//! on-screen recorder ([`crate::present`]'s `record_gbuffer`) wraps around the four
//! software-ray passes. It is threaded onto the scene as `Option<&TimestampCollector>`
//! ([`crate::present::scene_types::GBufferScene::gpu_timing`]): `None` on every
//! golden/host frame emits ZERO reset/write commands, so the recorded command stream is
//! BYTE-IDENTICAL to the pre-R0 path.
//!
//! # Recording model
//!
//! Writing a timestamp mutates GPU query memory, NOT the Rust struct, so the collector is
//! shared `&`-read-only during recording (the frame top resets `(0, 2*PASS_COUNT)`, then
//! each pass writes its begin/end via [`TimestampCollector::write_begin`] /
//! [`TimestampCollector::write_end`]). All accumulation (median / p95) is HOST-side in the
//! offline harness AFTER `read_query_pool_ns` — never here.
//!
//! # Lifecycle (per measured frame)
//!
//! `reset_frame(pool)` at the frame top (outside any render / dynamic-rendering scope) →
//! per pass `write_begin(TopOfPipe)` before its first cmd + `write_end(BottomOfPipe)` after
//! its last → submit(fence) → `wait_fence` → `RhiDevice::read_query_pool_ns`.

use boyko_rhi::TimestampStage;

use crate::device::DeviceFns;
use crate::ffi::VkCommandBuffer;
use crate::rhi_impl::VulkanQueryPool;

use super::FRAMES_IN_FLIGHT;

/// The four software-ray passes the R0 harness brackets, in query-pair-slot order (the
/// begin query for pass `p` is `2 * p`, its end query `2 * p + 1`).
///
/// `#[repr(u32)]` so the discriminant IS the pair slot index — a `write_begin(pass)` writes
/// query `2 * pass.slot()` and `write_end(pass)` writes `2 * pass.slot() + 1`.
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimedPass {
    /// The SDFDDGI probe-update dispatch (the probe field-march + blend).
    DdgiUpdate = 0,
    /// The deferred resolve dispatch — INCLUDING the inline SDF soft-shadow march (R0
    /// brackets passes, not shader sections, so this is the whole resolve, not the march
    /// alone).
    DeferredResolve = 1,
    /// The CSM cascade depth pass (the layered depth loop).
    CsmDepth = 2,
    /// The sparse spot/point atlas depth pass (the layered depth loop).
    PunctualDepth = 3,
}

impl TimedPass {
    /// The pair slot index — the begin query is `2 * slot`, the end query `2 * slot + 1`.
    #[inline]
    pub const fn slot(self) -> u32 {
        self as u32
    }
}

/// The number of bracketed software-ray passes (`DdgiUpdate`, `DeferredResolve`, `CsmDepth`,
/// `PunctualDepth`). Each needs a begin+end query, so a pool holds `2 * PASS_COUNT` queries.
pub const PASS_COUNT: u32 = 4;

/// A per-frame-in-flight ring of TIMESTAMP query pools bracketing the four software-ray
/// passes (HW-RT rung R0).
///
/// The offline harness fences each frame, so a single pool would suffice; the collector
/// rings `[VulkanQueryPool; FRAMES_IN_FLIGHT]` indexed by the renderer's `fi` for
/// future-pipelining safety at no cost (the same lock-free per-frame discipline every ringed
/// G-buffer resource follows). The harness OWNS the pools (creates + destroys them); this
/// struct only borrows them into the recording brackets.
pub struct TimestampCollector {
    /// One `2 * PASS_COUNT`-query TIMESTAMP pool per in-flight frame, indexed by `fi`.
    pools: [VulkanQueryPool; FRAMES_IN_FLIGHT],
}

impl TimestampCollector {
    /// Builds a collector from the per-frame pools (each created with `2 * PASS_COUNT`
    /// queries). The caller (the offline harness) owns the pools' lifetime — it creates them
    /// via `RhiDevice::create_query_pool` and destroys them after `wait_idle`.
    #[inline]
    pub fn new(pools: [VulkanQueryPool; FRAMES_IN_FLIGHT]) -> Self {
        Self { pools }
    }

    /// This frame's query pool (indexed by the renderer's frame-in-flight slot `fi`). The
    /// harness reads it after `wait_fence` via `RhiDevice::read_query_pool_ns`.
    #[inline]
    pub fn pool(&self, fi: usize) -> &VulkanQueryPool {
        debug_assert!(fi < FRAMES_IN_FLIGHT, "invariant: fi must be a valid frame-in-flight slot");
        &self.pools[fi]
    }

    /// Consumes the collector, yielding its owned per-frame pools back to the caller for
    /// destruction (`RhiDevice::destroy_query_pool`). The collector holds no GPU objects of its
    /// own — only the pool values — so this is the teardown seam (after `wait_idle`).
    #[inline]
    pub fn into_pools(self) -> [VulkanQueryPool; FRAMES_IN_FLIGHT] {
        self.pools
    }

    /// Resets ALL `2 * PASS_COUNT` queries of frame `fi`'s pool (HW-RT rung R0). MUST be
    /// recorded at the frame top, OUTSIDE any render / dynamic-rendering scope
    /// (`VUID-vkCmdResetQueryPool-renderpass`) — a TIMESTAMP query is undefined until reset.
    ///
    /// # Safety
    /// `cmd` must be recordable (recording open) and `fns` must be the live device fn-table;
    /// the reset MUST NOT be inside a render pass. Records `vkCmdResetQueryPool`.
    #[inline]
    pub unsafe fn reset_frame(&self, fns: &DeviceFns, cmd: VkCommandBuffer, fi: usize) {
        let pool = self.pool(fi);
        // SAFETY: `cmd` is recordable + outside any rendering scope (caller contract); `fns` is
        // the live device fn-table; `pool.pool` is a live TIMESTAMP pool with `pool.count ==
        // 2 * PASS_COUNT` queries, so `[0..2*PASS_COUNT)` is exactly in bounds.
        unsafe { (fns.cmd_reset_query_pool)(cmd, pool.pool, 0, 2 * PASS_COUNT) };
    }

    /// Writes the BEGIN timestamp (`TopOfPipe`) for `pass` into frame `fi`'s pool (query
    /// `2 * pass.slot()`). Records it before the pass's first command.
    ///
    /// # Safety
    /// `cmd` must be recordable and `fns` the live device fn-table; the pool's queries were
    /// reset this frame ([`Self::reset_frame`]). Records `vkCmdWriteTimestamp` at
    /// `TOP_OF_PIPE`.
    #[inline]
    pub unsafe fn write_begin(
        &self,
        fns: &DeviceFns,
        cmd: VkCommandBuffer,
        fi: usize,
        pass: TimedPass,
    ) {
        // SAFETY: caller contract (recordable `cmd`, live `fns`, pool reset this frame).
        unsafe { self.write(fns, cmd, fi, TimestampStage::TopOfPipe, 2 * pass.slot()) };
    }

    /// Writes the END timestamp (`BottomOfPipe`) for `pass` into frame `fi`'s pool (query
    /// `2 * pass.slot() + 1`). Records it after the pass's last command.
    ///
    /// # Safety
    /// `cmd` must be recordable and `fns` the live device fn-table; the pool's queries were
    /// reset this frame ([`Self::reset_frame`]). Records `vkCmdWriteTimestamp` at
    /// `BOTTOM_OF_PIPE`.
    #[inline]
    pub unsafe fn write_end(
        &self,
        fns: &DeviceFns,
        cmd: VkCommandBuffer,
        fi: usize,
        pass: TimedPass,
    ) {
        // SAFETY: caller contract (recordable `cmd`, live `fns`, pool reset this frame).
        unsafe { self.write(fns, cmd, fi, TimestampStage::BottomOfPipe, 2 * pass.slot() + 1) };
    }

    /// The shared `vkCmdWriteTimestamp` helper: writes query `index` of frame `fi`'s pool at
    /// `stage`.
    ///
    /// # Safety
    /// See [`Self::write_begin`] / [`Self::write_end`].
    #[inline]
    unsafe fn write(
        &self,
        fns: &DeviceFns,
        cmd: VkCommandBuffer,
        fi: usize,
        stage: TimestampStage,
        index: u32,
    ) {
        let pool = self.pool(fi);
        debug_assert!(index < pool.count, "invariant: timestamp index must be in the pool");
        // Map the agnostic stage to a `VkPipelineStageFlagBits` via an identity cast — the
        // `TimestampStage` discriminants equal the `VK_PIPELINE_STAGE_*` bits (asserted in
        // `abi_guard.rs`).
        let vk_stage = stage.as_i32() as u32;
        // SAFETY: `cmd` is recordable (caller contract); `fns` is the live device fn-table;
        // `pool.pool` is a live TIMESTAMP pool; `index < pool.count` (asserted) and was reset
        // this frame; `vk_stage` is a single valid pipeline-stage bit (TOP/BOTTOM).
        unsafe { (fns.cmd_write_timestamp)(cmd, vk_stage, pool.pool, index) };
    }
}
