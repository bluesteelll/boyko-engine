//! The [`RhiCommandEncoder`] operational trait: the hot command-recording path.
//!
//! No `dyn`, no per-command allocation. Every method monomorphizes to a direct
//! `(fns.cmd_*)` indirect call — byte-identical codegen to the current inherent
//! Vulkan methods (plan O4). `pipeline_barrier` is **buffer-only** in Phase 1
//! (plan D3): explicit caller-side barriers per plan §5.5, no auto-tracking.

use crate::api::RhiApi;
use crate::descriptor::BarrierDesc;
use crate::enums::ShaderStage;
use crate::error::RhiError;

/// Records commands into a one-time-submit command buffer.
///
/// The encoder owns its command pool + command buffer + descriptor pool + the
/// fixed compute descriptor set (built once at `create_command_encoder`, per
/// plan Q1 — no per-record descriptor-set rebuild).
pub trait RhiCommandEncoder<A: RhiApi> {
    /// One unified per-backend error type (plan D4); bound is `From<RhiError>`
    /// only — see [`crate::device::RhiDevice::Error`].
    type Error: core::fmt::Debug + From<RhiError>;

    /// Resets and begins recording (one-time-submit).
    fn begin(&mut self) -> Result<(), Self::Error>;

    /// Ends recording.
    fn end(&mut self) -> Result<(), Self::Error>;

    /// Binds a compute pipeline for subsequent dispatches.
    fn bind_compute_pipeline(&mut self, pipeline: &A::ComputePipeline);

    /// Binds `buffer` as the storage buffer at `(set, binding)`.
    ///
    /// The encoder updates its cached descriptor set only when the bound buffer
    /// differs from the last binding (plan Q1).
    fn bind_storage_buffer(&mut self, buffer: &A::Buffer, set: u32, binding: u32);

    /// Records a push-constant update for `stage` at byte `offset`.
    fn push_constants(&mut self, stage: ShaderStage, offset: u32, bytes: &[u8]);

    /// Records a compute dispatch of `gx * gy * gz` workgroups.
    fn dispatch(&mut self, gx: u32, gy: u32, gz: u32);

    /// Records an explicit buffer pipeline barrier (plan §5.5; no auto-tracking).
    fn pipeline_barrier(&mut self, barrier: &BarrierDesc<A>);

    // ===== DEFERRED SEAM (Phase 6+) =====

    /// Records an indirect compute dispatch reading group counts from `buffer` at
    /// byte `offset`. Seam: Phase 6+.
    ///
    /// The default body is a no-op marked `#[cold] #[inline(never)]` so it never
    /// touches the hot recording path's I-cache when not overridden; a backend
    /// fills it in when indirect dispatch lands.
    #[cold]
    #[inline(never)]
    fn dispatch_indirect(&mut self, _buffer: &A::Buffer, _offset: u64) {
        // Phase-6+ seam: no foundation code calls this.
    }
}
