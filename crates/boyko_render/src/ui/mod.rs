//! GUI P5a — the in-house UI-rect render capability (instanced rounded-rect SDF).
//!
//! Rasterizes every laid-out UI node as a crisp, anti-aliased, optionally-rounded,
//! optionally-bordered rectangle on the in-house Vulkan path, reading ONLY ECS
//! columns (`ComputedRect`, `StackIndex`, `ComputedClip`, `UiBackground`) at zero
//! per-frame heap allocation and (steady-state) one draw call.
//!
//! # Module layout
//!
//! - [`instance`] — the std430 [`UiInstance`] GPU record + [`UiOrtho`] push block +
//!   the no-bytemuck POD byte views + the premultiply helper.
//! - [`pack`] — the CPU pack ([`pack_ui_instance`]) + the reused [`UiRenderScratch`]
//!   (with the in-place stable z-sort) + the [`UiRenderGeneration`] O(1) change gate.
//! - [`plan`] — the POD by-value cross-frame handoff [`UiFramePlan`] (Decision 9:
//!   borrows no RHI handle; the recorder re-resolves device handles by `frame_index`).
//! - [`draw`] — the shared, `RhiApi`-generic [`record_ui_rects`] draw recorder (one
//!   `draw(6, N, 0, 0)` into an already-open `LoadOp::Load` full-extent scope).
//!
//! # The combination is GPU-proven (Rung 0.5)
//!
//! The never-before-exercised path — a GRAPHICS pipeline binding a STORAGE buffer at
//! `set0/binding0` visible at VERTEX|FRAGMENT, read by `SV_InstanceID` in BOTH stages
//! — is validated by the `ssbo_graphics_probe` GPU golden (RTX 3060, validation
//! clean) BEFORE this module's SDF/blend complexity, per Decision 2.
//!
//! # Frames-in-flight
//!
//! The on-screen path double-buffers: one persistent-mapped grow-only STORAGE ring +
//! one bind-group PER [`FRAMES_IN_FLIGHT`] slot, each created once, selected by
//! `frame_index` (Decision 7). The ring + pipeline + bind-groups are owned by a
//! first-class `RhiContext` UI capability with wired `Drop` (Decision 8) — a named
//! owner, NOT a side store (Principle 0).
//!
//! # Scope of THIS commit — FOUNDATION ONLY (Rungs 0–2 + the draw recorder)
//!
//! What is shipped here (CPU-side + GPU asset foundation, end-to-end-render NOT yet
//! wired):
//! - [`UiInstance`] / [`UiOrtho`] — the std430 POD record + ortho push block, with
//!   the compile-time layout oracle and the no-bytemuck byte views.
//! - [`pack_ui_instance`] / [`UiRenderScratch`] / [`UiRenderGeneration`] — the CPU
//!   pack, the reused zero-alloc scratch + in-place stable z-sort, the O(1) gate.
//! - [`UiFramePlan`] — the POD by-value cross-frame handoff CARRIER (sound by
//!   construction: it borrows no RHI handle; see [`plan`]).
//! - [`record_ui_rects`] — the shared, `RhiApi`-generic one-draw recorder.
//! - The two HLSL shaders + their dxc-compiled `.spv` assets (not yet embedded).
//!
//! What is DEFERRED to a follow-up commit (Rungs 3–5; tracked, not done here):
//! - The `RhiContext` UI capability `ui_setup` / `ui_upload` / `ui_handles` and the
//!   owned `UiRenderResources` sub-owner (Decision 8) — the per-FIF host-mapped
//!   STORAGE ring, the per-FIF bind-groups, the grow-on-overflow path (Decision 7),
//!   and the wired `Drop` / `destroy_all` teardown.
//! - `UiUploadSystem` (Rung 4) — the dispatcher-solo `GpuSystem`-shaped pack→sort→
//!   upload that stashes the [`UiFramePlan`].
//! - The swapchain wiring (Rung 5 step 13) — the second `begin_rendering(LoadOp::
//!   Load)` at the full swapchain extent in `present_sampled` and the `record_ui_
//!   rects` call.
//!
//! Because the ring/upload-system/swapchain integration is deferred, the FULL
//! cross-frame `!Send` handoff MECHANISM (the dispatcher-solo projection of
//! `RhiContext` via `nonsend_resource_mut`, the by-`frame_index` re-resolution in the
//! swapchain recorder) is NOT exercised yet and MUST be re-audited end-to-end (and
//! Miri-TB exercised, per the Phase 9.1 / 14a lessons) when Rungs 3–5 land. The
//! [`UiFramePlan`] carrier delivered here is the sound half (POD, no borrowed handle);
//! the soundness of the mechanism around it is established only once that mechanism
//! exists.

pub mod draw;
pub mod gather;
pub mod instance;
pub mod pack;
pub mod plan;
pub(crate) mod resources;
pub mod upload;

pub use draw::record_ui_rects;
pub use gather::{gather_ui_nodes, probe_component, ui_render_discovery, UiGatherScratch};
pub use instance::{
    premultiply_rgba8, UiInstance, UiOrtho, FLAG_BORDER_ANY, FLAG_CLIP_PRESENT, FLAG_TEXT,
    FLAG_TEXTURED, FLAG_TILED, UI_INSTANCE_SIZE, UI_SLOT_BITS, UI_SLOT_MASK, UI_SLOT_SHIFT,
    UI_TILE_BITS, UI_TILE_MASK, UI_TILE_MAX, UI_TILE_X_SHIFT, UI_TILE_Y_SHIFT,
};
pub use pack::{
    emit_ui_node_records, pack_ui_image_instance, pack_ui_instance, pack_ui_nine_slice_instance,
    pack_ui_sub_record, ui_nine_slice_tiles, ui_nine_slice_tiles_axis, ui_node_sub_codes,
    PackInput, UiImageInput, UiNineSliceInput, UiRenderGeneration, UiRenderScratch, UI_IMAGE_SUB,
    UI_MAX_SUBS_PER_NODE, UI_NINE_SLICE_CENTER_SUB, UI_NINE_SLICE_MODE_COUNT,
    UI_NINE_SLICE_MODE_TILE, UI_NINE_SLICE_REGIONS, UI_NINE_SLICE_SUB_BASE, UI_RECORDS_PER_NODE,
};
pub use resources::UiSamplerMode;
pub use plan::UiFramePlan;
pub use upload::{UiNode, UiUploadSystem, UI_MAX_NODES, UI_STAGING_ROWS};

/// Frames-in-flight for the UI render ring — one host-mapped STORAGE ring slot + one
/// bind-group per slot (Decision 7). MUST equal the swapchain `Renderer`'s
/// `FRAMES_IN_FLIGHT` so the UI ring slot a frame writes/binds matches the
/// swapchain's in-flight fence for that `frame_index`.
pub const FRAMES_IN_FLIGHT: usize = 2;

/// A 4-byte-aligned wrapper around a committed SPIR-V byte blob so its address is a
/// valid `*const u32` and it re-views as the `&[u32]` word stream
/// [`RhiDevice::create_shader_module`](boyko_rhi::RhiDevice::create_shader_module)
/// requires. Mirrors the `gpu_system::SpirvBlob` trick (a bare `include_bytes!` is
/// only `align(1)`; SPIR-V needs 4-byte alignment).
#[repr(C, align(4))]
struct SpirvBlob<const N: usize>([u8; N]);

impl<const N: usize> SpirvBlob<N> {
    /// Re-views the blob as its SPIR-V `u32` word stream after a magic-number check.
    fn as_words(&self) -> &[u32] {
        const { assert!(N.is_multiple_of(4), "SPIR-V byte length must be a multiple of 4") };
        const { assert!(N >= 4, "SPIR-V blob must hold at least the magic word") };
        // Release-present magic check: a misplaced / non-SPIR-V file fails loud here
        // (`0x07230203`, little-endian bytes `03 02 23 07`) rather than handing a
        // corrupt word stream to `vkCreateShaderModule`. Validated once at setup.
        assert_eq!(
            [self.0[0], self.0[1], self.0[2], self.0[3]],
            [0x03, 0x02, 0x23, 0x07],
            "SPIR-V blob does not start with the magic number 0x07230203 \
             (corrupt, wrong-endian, or not a .spv file)"
        );
        // SAFETY: the `align(4)` wrapper makes `self.0`'s address a valid `*const
        // u32`; `N` is a 4-byte multiple (const-asserted), so the blob is exactly
        // `N / 4` whole `u32` words; the `&self` borrow keeps the `'static` blob
        // alive for the slice's lifetime; any byte pattern is a valid `u32`.
        unsafe { core::slice::from_raw_parts(self.0.as_ptr().cast::<u32>(), N / 4) }
    }
}

/// The committed `ui_rect.vs.spv` (vertexless quad, a VERTEX-stage SSBO transform
/// read, and the ortho push constant). The `const N` byte length must match the file
/// on disk (a mismatch is a compile error).
///
/// RE-BLESSED 2368 → 2408 at UI-ADVANCED S2 (the D1 widening): the `UiInstance`
/// mirror gained the `uv` member (stride 64 → 80 B), which the VS declares but never
/// reads — the byte move is the struct declaration alone. Diff read before the
/// re-bless; the four S-D6 image hashes reproduced the 64 B build's exactly.
static UI_RECT_VS_SPV: SpirvBlob<2408> = SpirvBlob(*include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/shaders/ui_rect.vs.spv"
)));

/// The committed `ui_rect.fs.spv` (`sdRoundedBox` + `fwidth` AA + uniform border +
/// flag-gated clip, premultiplied out; FRAGMENT-stage SSBO read).
///
/// RE-BLESSED 7060 → 7136 at UI-ADVANCED S2 (the D1 widening): the mirror gained
/// `uv` (stride 64 → 80 B) and the `FLAG_TEXT` branch now reads `inst.uv` instead of
/// the retired `corner_radius` alias — the whole semantic delta of the rung. Diff
/// read before the re-bless; the four S-D6 image hashes reproduced the 64 B build's
/// exactly (G2-3).
///
/// RE-BLESSED 7136 → 8760 at UI-ADVANCED S3 (the sprite lane): the stage gained the
/// set-1 bindless `Texture2D g_sprites[]` declaration, the UI's own set-0/binding-3
/// `SamplerState`, the `FLAG_TEXTURED` + `UI_SLOT_{SHIFT,MASK}` constants and the
/// `NonUniformResourceIndex` sprite branch — the rung's whole semantic delta, and the
/// first non-uniform descriptor index in this shader (SR3). The generated-HLSL diff was
/// read before the re-bless. Its SIBLING `ui_rect.vs.spv` did NOT move (2408 → 2408,
/// byte-identical): the VS's only S3 edit is a comment inside the shared struct mirror,
/// and DXC's output is measurably indifferent to it.
///
/// RE-BLESSED 8760 → 9120 at UI-ADVANCED S5 (the tiled lane, S-D15): the stage gained the
/// `FLAG_TILED` + `UI_TILE_{X_SHIFT, Y_SHIFT, MASK}` constants and the seventh eDSL leaf
/// `ui_tile_uv`, into which the sprite branch's WHOLE UV computation moved — the line was
/// previously an inline `lerp` in the generator's `main` template, where nothing compared
/// it to the committed copy. The untiled arm still spells the same `lerp(uv.xy, uv.zw, t)`
/// intrinsic on the same operands (S-D15 (4) / `Cf::vec2_lerp`), which is why all SIX
/// committed image pins (four S2 + one S3 + one S4) reproduced byte-for-byte across this
/// re-bless rather than needing one. The VS again did NOT move (2408 → 2408): its only S5
/// edit is one comment line in the shared struct mirror. *(S-D15 (4) said "only the FS blob
/// moves" — true of the `.spv` and NOT of the `.hlsl`: the tile bits land in the SHARED
/// mirror span, so the VS SOURCE moves and `ui_rect_edsl_sync`'s VS half covers it.)*
static UI_RECT_FS_SPV: SpirvBlob<9120> = SpirvBlob(*include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/shaders/ui_rect.fs.spv"
)));

/// The committed UI vertex SPIR-V as a `u32` word stream, ready for
/// [`RhiContext::ui_setup`](crate::RhiContext::ui_setup).
#[inline]
pub fn ui_rect_vs_spirv() -> &'static [u32] {
    UI_RECT_VS_SPV.as_words()
}

/// The committed UI fragment SPIR-V as a `u32` word stream, ready for
/// [`RhiContext::ui_setup`](crate::RhiContext::ui_setup).
#[inline]
pub fn ui_rect_fs_spirv() -> &'static [u32] {
    UI_RECT_FS_SPV.as_words()
}
