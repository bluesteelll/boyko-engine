//! The shared UI-rect draw recorder (`record_ui_rects`) — GUI P5a Rung 5.
//!
//! Generic over [`RhiApi`] so the SAME recording logic serves both the concrete
//! on-screen swapchain path and the trait-encoder golden test (the existing
//! concrete-present / trait-test split). The caller opens a fresh
//! `begin_rendering(LoadOp::Load)` at the FULL target extent FIRST (preserving the
//! composited scene, NOT re-clearing), then calls this; the recorder binds the
//! re-resolved pipeline + bind-group, pushes the ortho (VERTEX stage), sets the
//! full-extent viewport+scissor, and records ONE `draw(6, N, 0, 0)`.

use boyko_rhi::api::RhiApi;
use boyko_rhi::encoder::RhiCommandEncoder;
use boyko_rhi::enums::ShaderStage;
use boyko_rhi::{RenderArea, Viewport};

use crate::ui::plan::UiFramePlan;

/// Records the UI rect pass into an ALREADY-OPEN color target (the caller opened
/// `begin_rendering(LoadOp::Load)` at `full_area` first). Binds `pipeline` +
/// `bind_group` (both RE-RESOLVED by the caller from the current frame index — MF-7,
/// never a cached raw handle), pushes `plan.ortho` to the VERTEX stage, sets the
/// FULL-extent viewport + scissor, and records exactly one instanced draw.
///
/// `full_area` MUST be the full extent of the image the UI pass renders into (the
/// swapchain `VkExtent2D`), matching the `plan.ortho` denominator (Decision 9): a
/// rect at the bottom-right corner then lands at the bottom-right texel. The ortho
/// is read in the VERTEX stage only (the fragment shader reads only the SSBO), so a
/// VERTEX-stage push against the pipeline's VERTEX push range is correct.
///
/// A `plan.instance_count == 0` records nothing (no empty draw).
///
/// # Safety
///
/// The caller guarantees:
/// - `enc` is recording inside a `begin_rendering` scope whose single color
///   attachment's format equals `pipeline`'s `color_formats[0]` (the W2-b contract),
///   at `full_area`;
/// - `pipeline` was created with the UI bind-group layout at set 0 and a
///   VERTEX-stage push range of at least 16 bytes, and `bind_group` is the
///   current-frame ring slot's group bound at set0/binding0 against that layout;
/// - `bind_group`'s backing STORAGE buffer holds at least `plan.instance_count`
///   valid `UiInstance` records (uploaded for THIS frame index before this draw);
/// - `sprite_group` is the set-1 group `pipeline`'s layout declares at index 1 —
///   `RhiContext::ui_sprite_group()`, the SAME accessor the on-screen recorder reads
///   (decision S-D9). It is not optional: `ui_rect.fs` statically uses set 1, so a
///   draw with set 1 unbound is `VUID-vkCmdDraw-None-08600` even for a plain rect.
pub unsafe fn record_ui_rects<A: RhiApi>(
    enc: &mut impl RhiCommandEncoder<A>,
    full_area: &RenderArea,
    plan: &UiFramePlan,
    pipeline: &A::GraphicsPipeline,
    bind_group: &A::BindGroup,
    sprite_group: &A::BindGroup,
) {
    if plan.instance_count == 0 {
        return;
    }

    enc.bind_graphics_pipeline(pipeline);
    enc.bind_descriptor_set(bind_group, pipeline);
    // UI-ADVANCED S3 (S-D3): the sprite lane's set 1, through the GENERIC set-index verb
    // this rung added to `boyko_rhi` — which is what keeps this trait-driven offscreen
    // recorder able to draw a sprite at all, and therefore keeps the sprite path testable
    // on a device with no display.
    enc.bind_descriptor_set_at(1, sprite_group, pipeline);
    // VUID-vkCmdPushConstants-offset-01796: the graphics layout declares its push range
    // over VERTEX | FRAGMENT, and the call must name ALL stages of the overlapping range.
    enc.push_graphics_constants(
        pipeline,
        ShaderStage::VERTEX | ShaderStage::FRAGMENT,
        0,
        plan.ortho.as_bytes(),
    );

    let viewport = Viewport {
        x: full_area.x as f32,
        y: full_area.y as f32,
        width: full_area.width as f32,
        height: full_area.height as f32,
        min_depth: 0.0,
        max_depth: 1.0,
    };
    enc.set_viewport(&viewport);
    enc.set_scissor(full_area);

    // ONE draw: 6 vertices (two triangles per unit quad) × N instances.
    enc.draw(6, plan.instance_count, 0, 0);
}
