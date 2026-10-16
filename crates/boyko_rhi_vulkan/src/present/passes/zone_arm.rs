//! Render wave PX0 (`px0/cut.md` D-Z2, D-Z3) — the zone arm the POST / FWD brackets record
//! through, and the frame witness the Forward recorder opens.
//!
//! [`ZoneArm`] is `MergeZoneArm`'s shape (`sdf_depth_merge.rs`) keyed by zone id: one arm serves
//! every bracket site of the two PX0 families, so each pass body opens and closes its own zone and
//! one site serves both the Deferred and the VisibilityBuffer recorders. `MergeZoneArm` and
//! `ParticleZoneArm` stay beside it: each is bound to its own family's fixed ids and lives in a
//! file another rung owns, and folding them is a refactor this rung's purpose does not carry.
//!
//! # Arming is a claim about the CALLER
//!
//! A `ZoneArm` built by [`ZoneArm::from_scene`] carries the frame's recorder and ring slot. Its
//! brackets are valid only because the path recorder's witness reset this frame's query pool before
//! the first bracket and seals the slot after the last one. Deferred and VisibilityBuffer have
//! that witness already; Forward gets [`FrameWitness`] here.
//!
//! # Zero cost when off
//!
//! `scene.gpu_zone` is `None` on every shipped, golden and host frame (`GBufferScene::gpu_zone`'s
//! doc), so `begin` returns before recording any command and the stream is byte-identical.

use boyko_rhi::TimestampStage;

use crate::device::DeviceFns;
use crate::ffi::VkCommandBuffer;

#[cfg(feature = "profiling-census")]
use super::super::command_witness::CommandWitness;
use super::super::gpu_zone::{GpuZoneRecorder, zone_begin_stage};
use super::super::scene_types::GBufferScene;

/// The recorder handle a pass body brackets its own zone through.
#[derive(Clone, Copy)]
pub(crate) struct ZoneArm<'a> {
    /// The frame's zone recorder and the ring slot it opened, or `None` when this frame records no
    /// zones.
    zr: Option<(&'a GpuZoneRecorder, usize)>,
    /// The command census, when a gate armed one.
    #[cfg(feature = "profiling-census")]
    cw: Option<&'a CommandWitness>,
}

impl<'a> ZoneArm<'a> {
    /// The arm for a frame whose path recorder's witness reset this frame's pool and seals the
    /// slot after its last bracket.
    #[inline]
    pub(crate) fn from_scene(scene: &GBufferScene<'a>) -> ZoneArm<'a> {
        ZoneArm {
            zr: scene.gpu_zone,
            #[cfg(feature = "profiling-census")]
            cw: scene.vb_cmd_witness,
        }
    }

    /// Opens `zone`'s bracket; returns its pair, or `None` (no command recorded) when unarmed or
    /// the ring slot is full.
    ///
    /// # Safety
    /// Recording is open on `cmd`; `fns` is the live device fn-table; this frame's pool was reset
    /// by the caller's witness before this call and the slot is sealed after it (the arm's
    /// contract).
    #[inline]
    pub(crate) unsafe fn begin(&self, fns: &DeviceFns, cmd: VkCommandBuffer, zone: u16) -> Option<u16> {
        let (rec, ring) = self.zr?;
        let pair = rec.alloc_pair(ring, zone)?;
        // SAFETY: caller contract (recording open, pool reset this frame, sealed after); `pair`
        // came from `alloc_pair` on this slot immediately above.
        let stage = unsafe { rec.record_begin(fns, cmd, ring, pair, zone_begin_stage(zone)) };
        self.mark(zone, stage, true);
        Some(pair)
    }

    /// Closes the bracket [`Self::begin`] opened for `zone` (no-op without a pair).
    ///
    /// # Safety
    /// As [`Self::begin`]; `pair` is the value `begin` returned for this `zone` in this frame.
    #[inline]
    pub(crate) unsafe fn end(&self, fns: &DeviceFns, cmd: VkCommandBuffer, zone: u16, pair: Option<u16>) {
        if let (Some((rec, ring)), Some(pair)) = (self.zr, pair) {
            // SAFETY: caller contract; `pair` is the one `begin` allocated for this zone on this
            // slot.
            let stage = unsafe { rec.record_end(fns, cmd, ring, pair) };
            self.mark(zone, stage, false);
        }
    }

    /// The census half of a BEGIN (`open`) or an END. Compiles to nothing without
    /// `profiling-census`.
    #[inline]
    fn mark(&self, zone: u16, stage: TimestampStage, open: bool) {
        #[cfg(feature = "profiling-census")]
        if let Some(w) = self.cw {
            if open {
                w.open_pair(zone);
                w.timestamp(stage);
            } else {
                w.timestamp(stage);
                w.close_pair(zone);
            }
        }
        #[cfg(not(feature = "profiling-census"))]
        let _ = (zone, stage, open);
    }
}

/// The Forward recorder's frame witness (D-Z3) — the reset / seal pair `GbufWitness::open` /
/// `finish` give the Deferred path, which `record_forward` never had.
///
/// Without it a Forward frame's brackets would write timestamps into an unreset query (an
/// undefined value) and retire unsealed (`NotBracketed`). It mirrors `GbufWitness` in full,
/// census included (critique O1): `open` clears the census's per-frame counters and marks the
/// query reset, so a census build does not accumulate `zone_cmd_*` across Forward frames.
pub(crate) struct FrameWitness<'a> {
    zr: Option<(&'a GpuZoneRecorder, usize)>,
}

impl<'a> FrameWitness<'a> {
    /// Opens the frame's witness and records its pool reset.
    ///
    /// # Safety
    /// Recording must be open on `cmd`, OUTSIDE any render or dynamic-rendering scope
    /// (`VUID-vkCmdResetQueryPool-renderpass`), before any bracket of this frame; `fns` must be
    /// the live device fn-table.
    #[inline]
    pub(crate) unsafe fn open(scene: &GBufferScene<'a>, fns: &DeviceFns, cmd: VkCommandBuffer) -> FrameWitness<'a> {
        #[cfg(feature = "profiling-census")]
        if let Some(w) = scene.vb_cmd_witness {
            w.begin_frame();
        }
        if let Some((rec, ring)) = scene.gpu_zone {
            // SAFETY: caller contract — recording open, outside any scope, before every bracket
            // of this frame; `ring` is the slot `open_frame` claimed for this frame.
            unsafe { rec.record_reset(fns, cmd, ring) };
            #[cfg(feature = "profiling-census")]
            if let Some(w) = scene.vb_cmd_witness {
                w.query_reset();
            }
        }
        FrameWitness { zr: scene.gpu_zone }
    }

    /// Seals the frame's slot — the release edge `retire`'s `Acquire` pairs with. Called after the
    /// frame's last bracket and before `vkEndCommandBuffer`.
    #[inline]
    pub(crate) fn finish(self) {
        if let Some((rec, ring)) = self.zr {
            rec.seal(ring);
        }
    }
}
