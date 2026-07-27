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

// === VB-P1d — the VisibilityBuffer froxel light-cull GPU-timestamp bench collector. ===
//
// A SEPARATE, small collector from [`TimestampCollector`] (not a widened `PASS_COUNT`):
// [`TimestampCollector`]'s own offline harness (`engine_grand_showcase_512_gpu_pass_cost`)
// reads ALL `PASS_COUNT` (begin,end) pairs with `VK_QUERY_RESULT_WAIT_BIT`, which BLOCKS
// FOREVER on any pair its recorder never wrote this frame (see [`TimestampCollector::write`]'s
// own precondition doc). Widening the shared `PASS_COUNT`/`TimedPass` to also cover the VB
// passes would make that EXISTING harness request extra pairs while `record_gbuffer` writes
// only `PASS_COUNT` of them — an instant deadlock. A dedicated
// `VbTimestampCollector`/`VbTimedPass`/`VB_PASS_COUNT` keeps the two rungs' query-pool sizing
// independent — VB-P1e H0 grew `VB_PASS_COUNT` 2 → 3 without touching `PASS_COUNT` at all.

/// The three `record_vb` dispatches the VB-P1d/VB-P1e bench brackets, in query-pair-slot order
/// (the begin query for pass `p` is `2 * p`, its end query `2 * p + 1`).
///
/// `#[repr(u32)]` so the discriminant IS the pair slot index — mirrors [`TimedPass`]'s own
/// shape.
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VbTimedPass {
    /// VB-P1e H0: the light-cull's alloc-counter `cmd_fill_buffer` plus its graph-derived
    /// TRANSFER→COMPUTE barrier — the FIRST HALF of what VB-P1d bracketed as one `LightCull`
    /// pair. Bracketed UNCONDITIONALLY whenever the bench collector is armed — even on a frame
    /// where the froxel arm itself is not boot-built (`scene.cluster_cull.is_none()`), so this
    /// pair is ALWAYS written (near-zero ns then) and the readback never blocks on an unwritten
    /// query regardless of which leg (flat vs froxel) this boot resolved. Exists to attribute
    /// §1.2's ~13.9 us fixed cull cost (fill+barrier vs dispatch ramp) instead of assuming it.
    CullReset = 0,
    /// VB-P1e H0: the cull dispatch itself (`cluster_cull.comp.hlsl`) — the SECOND HALF of
    /// VB-P1d's `LightCull` pair. Same unconditional-write shape as [`Self::CullReset`].
    CullDispatch = 1,
    /// The `record_vb` lit-producer dispatch — whichever of the THREE mutually-exclusive
    /// producers this frame selects: `vb_shade_split` (when `scene.path_vb_split()`, which
    /// DISPLACES both others), else `vb_shade` (material-classified, when
    /// `scene.vb_use_classified`), else the fused `vb_resolve`. Bracketed identically in all
    /// three branches — same "derived barriers + bind + dispatch" extent — so exactly one
    /// begin/end pair is written per mesh-leg frame whichever branch runs.
    ///
    /// The split arm's bracket was added by **VB-SV0 rung S5**, whose paired A/B measures the
    /// split tail (`.spv` matrix row 7) alongside the fused one (row 1). Before it, a split
    /// frame reset-but-never-wrote this pair and the `VK_QUERY_RESULT_WAIT_BIT` readback would
    /// block forever; that hazard is now closed at the recorder rather than by a caller-side
    /// precondition. VB-P1d's `!mesh_geo_shade_split` assertion survives as a SCOPE statement
    /// (its break-even number is defined against the fused/classified tail), not as a hang guard.
    VbShade = 2,
}

impl VbTimedPass {
    /// The pair slot index — the begin query is `2 * slot`, the end query `2 * slot + 1`.
    #[inline]
    pub const fn slot(self) -> u32 {
        self as u32
    }
}

/// The number of bracketed VB-P1d/VB-P1e passes (`CullReset`, `CullDispatch`, `VbShade`). Each
/// needs a begin+end query, so a pool holds `2 * VB_PASS_COUNT` queries.
pub const VB_PASS_COUNT: u32 = 3;

/// A per-frame-in-flight ring of TIMESTAMP query pools bracketing the VB-P1d froxel
/// cull/shade cost bench — the `record_vb` sibling of [`TimestampCollector`].
///
/// Threaded onto the scene as `Option<&VbTimestampCollector>`
/// ([`crate::present::scene_types::GBufferScene::vb_gpu_timing`]): `None` on every
/// golden/host/interactive frame emits ZERO reset/write commands, so the recorded command
/// stream is BYTE-IDENTICAL to the pre-VB-P1d path.
pub struct VbTimestampCollector {
    /// One `2 * VB_PASS_COUNT`-query TIMESTAMP pool per in-flight frame, indexed by `fi`.
    pools: [VulkanQueryPool; FRAMES_IN_FLIGHT],
}

impl VbTimestampCollector {
    /// Builds a collector from the per-frame pools (each created with `2 * VB_PASS_COUNT`
    /// queries). The caller owns the pools' lifetime — created via
    /// `RhiDevice::create_query_pool` and destroyed via `RhiDevice::destroy_query_pool` after
    /// `wait_idle`.
    #[inline]
    pub fn new(pools: [VulkanQueryPool; FRAMES_IN_FLIGHT]) -> Self {
        Self { pools }
    }

    /// This frame's query pool (indexed by the renderer's frame-in-flight slot `fi`). The
    /// caller reads it after the frame's GPU work completes via `RhiDevice::read_query_pool_ns`.
    #[inline]
    pub fn pool(&self, fi: usize) -> &VulkanQueryPool {
        debug_assert!(fi < FRAMES_IN_FLIGHT, "invariant: fi must be a valid frame-in-flight slot");
        &self.pools[fi]
    }

    /// Consumes the collector, yielding its owned per-frame pools back to the caller for
    /// destruction (`RhiDevice::destroy_query_pool`).
    #[inline]
    pub fn into_pools(self) -> [VulkanQueryPool; FRAMES_IN_FLIGHT] {
        self.pools
    }

    /// Resets ALL `2 * VB_PASS_COUNT` queries of frame `fi`'s pool. MUST be recorded at the
    /// frame top, OUTSIDE any render / dynamic-rendering scope
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
        // 2 * VB_PASS_COUNT` queries, so `[0..2*VB_PASS_COUNT)` is exactly in bounds.
        unsafe { (fns.cmd_reset_query_pool)(cmd, pool.pool, 0, 2 * VB_PASS_COUNT) };
    }

    /// Writes the BEGIN timestamp (`TopOfPipe`) for `pass` into frame `fi`'s pool (query
    /// `2 * pass.slot()`). Records it before the pass's first command.
    ///
    /// # Safety
    /// `cmd` must be recordable and `fns` the live device fn-table; the pool's queries were
    /// reset this frame ([`Self::reset_frame`]). Records `vkCmdWriteTimestamp` at
    /// `TOP_OF_PIPE`.
    #[inline]
    pub unsafe fn write_begin(&self, fns: &DeviceFns, cmd: VkCommandBuffer, fi: usize, pass: VbTimedPass) {
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
    pub unsafe fn write_end(&self, fns: &DeviceFns, cmd: VkCommandBuffer, fi: usize, pass: VbTimedPass) {
        // SAFETY: caller contract (recordable `cmd`, live `fns`, pool reset this frame).
        unsafe { self.write(fns, cmd, fi, TimestampStage::BottomOfPipe, 2 * pass.slot() + 1) };
    }

    /// The shared `vkCmdWriteTimestamp` helper: writes query `index` of frame `fi`'s pool at
    /// `stage`.
    ///
    /// # Safety
    /// See [`Self::write_begin`] / [`Self::write_end`].
    #[inline]
    unsafe fn write(&self, fns: &DeviceFns, cmd: VkCommandBuffer, fi: usize, stage: TimestampStage, index: u32) {
        let pool = self.pool(fi);
        debug_assert!(index < pool.count, "invariant: timestamp index must be in the pool");
        let vk_stage = stage.as_i32() as u32;
        // SAFETY: `cmd` is recordable (caller contract); `fns` is the live device fn-table;
        // `pool.pool` is a live TIMESTAMP pool; `index < pool.count` (asserted) and was reset
        // this frame; `vk_stage` is a single valid pipeline-stage bit (TOP/BOTTOM).
        unsafe { (fns.cmd_write_timestamp)(cmd, vk_stage, pool.pool, index) };
    }
}

// === VB-SV0 rung S1.5 — the DEFERRED marcher GPU-timestamp bench collector. ===
//
// A THIRD dedicated collector, for the same reason [`VbTimestampCollector`] is a second one
// (see its block comment): every `read_query_pool_ns` reader asks for ALL of its collector's
// (begin,end) pairs with `VK_QUERY_RESULT_WAIT_BIT`, which BLOCKS FOREVER on a pair its
// recorder never wrote this frame. `VbTimedPass`'s three pairs are written by `record_vb`,
// which a **Deferred** frame never runs; `TimedPass`'s four are written by passes S1.5's
// fixture does not arm (DDGI / CSM / punctual). Widening either would deadlock the other
// rung's harness on the very first frame. A one-pass collector with its own pool sizing keeps
// all three independent — the pattern this file already commits to.

/// The single `record_gbuffer` dispatch the VB-SV0 S1.5 bench brackets.
///
/// `#[repr(u32)]` so the discriminant IS the pair slot index — mirrors [`TimedPass`]'s and
/// [`VbTimedPass`]'s shape.
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sv0TimedPass {
    /// The Deferred fine-marcher dispatch (`sdf_gbuffer_composite.hlsl`) — the pass that
    /// carries BOTH `pc.lighting_flags`-gated arms S1.5 performs its interleaved paired A/B
    /// over: the `own_pixel` SDF-hit arm and the `!own_pixel` raster-owned arm that writes
    /// `gMaterial.RG = (mesh_shadow, mesh_ao)`.
    ///
    /// Bracketed inside the recorder's `if let Some(marcher_pass)` arm, i.e. on exactly the
    /// frames the dispatch is recorded. A render path that does not dispatch the marcher
    /// (`!scene.path_has_marcher()`) therefore leaves this pair UNWRITTEN, which would hang the
    /// `WAIT`-bit readback — the caller must only arm this collector on a marcher-carrying path
    /// (`boyko_app::runner`'s S1.5 block asserts it before reading).
    Marcher = 0,
}

impl Sv0TimedPass {
    /// The pair slot index — the begin query is `2 * slot`, the end query `2 * slot + 1`.
    #[inline]
    pub const fn slot(self) -> u32 {
        self as u32
    }
}

/// The number of bracketed VB-SV0 S1.5 passes ([`Sv0TimedPass::Marcher`]). Each needs a
/// begin+end query, so a pool holds `2 * SV0_PASS_COUNT` queries.
pub const SV0_PASS_COUNT: u32 = 1;

/// A per-frame-in-flight ring of TIMESTAMP query pools bracketing the Deferred fine-marcher
/// dispatch (VB-SV0 rung S1.5) — the `record_gbuffer` marcher sibling of
/// [`VbTimestampCollector`].
///
/// Threaded onto the scene as `Option<&Sv0TimestampCollector>`
/// ([`crate::present::scene_types::GBufferScene::sv0_gpu_timing`]): `None` on every
/// golden/host/interactive frame emits ZERO reset/write commands, so the recorded command
/// stream is BYTE-IDENTICAL to the pre-S1.5 path.
pub struct Sv0TimestampCollector {
    /// One `2 * SV0_PASS_COUNT`-query TIMESTAMP pool per in-flight frame, indexed by `fi`.
    pools: [VulkanQueryPool; FRAMES_IN_FLIGHT],
}

impl Sv0TimestampCollector {
    /// Builds a collector from the per-frame pools (each created with `2 * SV0_PASS_COUNT`
    /// queries). The caller owns the pools' lifetime — created via
    /// `RhiDevice::create_query_pool` and destroyed via `RhiDevice::destroy_query_pool` after
    /// `wait_idle`.
    #[inline]
    pub fn new(pools: [VulkanQueryPool; FRAMES_IN_FLIGHT]) -> Self {
        Self { pools }
    }

    /// This frame's query pool (indexed by the renderer's frame-in-flight slot `fi`). The
    /// caller reads it after the frame's GPU work completes via `RhiDevice::read_query_pool_ns`.
    #[inline]
    pub fn pool(&self, fi: usize) -> &VulkanQueryPool {
        debug_assert!(fi < FRAMES_IN_FLIGHT, "invariant: fi must be a valid frame-in-flight slot");
        &self.pools[fi]
    }

    /// Consumes the collector, yielding its owned per-frame pools back to the caller for
    /// destruction (`RhiDevice::destroy_query_pool`).
    #[inline]
    pub fn into_pools(self) -> [VulkanQueryPool; FRAMES_IN_FLIGHT] {
        self.pools
    }

    /// Resets ALL `2 * SV0_PASS_COUNT` queries of frame `fi`'s pool. MUST be recorded at the
    /// frame top, OUTSIDE any render / dynamic-rendering scope
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
        // 2 * SV0_PASS_COUNT` queries, so `[0..2*SV0_PASS_COUNT)` is exactly in bounds.
        unsafe { (fns.cmd_reset_query_pool)(cmd, pool.pool, 0, 2 * SV0_PASS_COUNT) };
    }

    /// Writes the BEGIN timestamp (`TopOfPipe`) for `pass` into frame `fi`'s pool (query
    /// `2 * pass.slot()`). Records it before the pass's first command.
    ///
    /// # Safety
    /// `cmd` must be recordable and `fns` the live device fn-table; the pool's queries were
    /// reset this frame ([`Self::reset_frame`]). Records `vkCmdWriteTimestamp` at
    /// `TOP_OF_PIPE`.
    #[inline]
    pub unsafe fn write_begin(&self, fns: &DeviceFns, cmd: VkCommandBuffer, fi: usize, pass: Sv0TimedPass) {
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
    pub unsafe fn write_end(&self, fns: &DeviceFns, cmd: VkCommandBuffer, fi: usize, pass: Sv0TimedPass) {
        // SAFETY: caller contract (recordable `cmd`, live `fns`, pool reset this frame).
        unsafe { self.write(fns, cmd, fi, TimestampStage::BottomOfPipe, 2 * pass.slot() + 1) };
    }

    /// The shared `vkCmdWriteTimestamp` helper: writes query `index` of frame `fi`'s pool at
    /// `stage`.
    ///
    /// # Safety
    /// See [`Self::write_begin`] / [`Self::write_end`].
    #[inline]
    unsafe fn write(&self, fns: &DeviceFns, cmd: VkCommandBuffer, fi: usize, stage: TimestampStage, index: u32) {
        let pool = self.pool(fi);
        debug_assert!(index < pool.count, "invariant: timestamp index must be in the pool");
        let vk_stage = stage.as_i32() as u32;
        // SAFETY: `cmd` is recordable (caller contract); `fns` is the live device fn-table;
        // `pool.pool` is a live TIMESTAMP pool; `index < pool.count` (asserted) and was reset
        // this frame; `vk_stage` is a single valid pipeline-stage bit (TOP/BOTTOM).
        unsafe { (fns.cmd_write_timestamp)(cmd, vk_stage, pool.pool, index) };
    }
}
