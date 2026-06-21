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
pub mod instance;
pub mod pack;
pub mod plan;

pub use draw::record_ui_rects;
pub use instance::{
    premultiply_rgba8, UiInstance, UiOrtho, FLAG_BORDER_ANY, FLAG_CLIP_PRESENT, UI_INSTANCE_SIZE,
};
pub use pack::{pack_ui_instance, PackInput, UiRenderGeneration, UiRenderScratch};
pub use plan::UiFramePlan;

/// Frames-in-flight for the UI render ring — one host-mapped STORAGE ring slot + one
/// bind-group per slot (Decision 7). MUST equal the swapchain `Renderer`'s
/// `FRAMES_IN_FLIGHT` so the UI ring slot a frame writes/binds matches the
/// swapchain's in-flight fence for that `frame_index`.
pub const FRAMES_IN_FLIGHT: usize = 2;
