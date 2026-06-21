//! GUI P5b — the text-measure system (`ui_text_measure_system`).
//!
//! Writes each text node's [`ContentSize`](crate::components::ContentSize) (the P1
//! Auto-sizing seam) from the SHAPED run, set-if-changed and change-gated on
//! `Or<(Changed<UiTextBuffer>, Changed<UiText>)>` so a static label costs nothing
//! (Phase-10 0%-overhead). It MUST be scheduled BEFORE
//! [`ui_layout_discovery`](crate::layout::ui_layout_discovery) so the same-frame
//! relayout sees the new `ContentSize` (the layout lists `Changed<ContentSize>` as a
//! relayout trigger, and reads `ContentSize` as the leaf intrinsic size when a node
//! has no relative children).
//!
//! Measure uses the SAME [`shape_into`](super::shape::shape_into) core as the emitter,
//! so the size it reports is byte-identical to the run the emitter lays down. The
//! shaping wrap width is the node's `ComputedRect.w` when set (a re-measure after a
//! width is known re-wraps); for the FIRST measure (before layout) the width is
//! whatever the previous frame's rect carried (0 ⇒ unwrapped single-line intrinsic
//! width, the natural `Auto` hug).
//!
//! Alloc-free: shaping streams glyphs to a no-op sink that only tracks the extent (the
//! shaper returns the extent directly), so the measure allocates nothing per frame.
//!
//! # Scheduling contract (Decision T5-B — host-driven, deliberately)
//!
//! P5b stays HOST-DRIVEN (matching P5a Decision T5-B): `boyko_ui` SHIPS the system but
//! does NOT own an App/Schedule for the UI systems, so registering the measure→layout
//! order is the HOST's responsibility. The host MUST register
//! `ui_text_measure_system` `.before(ui_layout_discovery)` (the same way it orders
//! `ui_layout_discovery` `.before(ui_layout_apply)`); a host that registers them out of
//! order gets a one-frame-stale layout. The CPU-boundary correctness of the seam is
//! proven without a scheduler by the `text_measure` unit tests: `measure_one`'s extent
//! equals the shaped run, and an `Auto` leaf node hugs the measured `ContentSize`
//! through the layout's leaf intrinsic-size fallback.

use boyko_ecs::ecs::core::iters::query::filter::{Changed, Or};
use boyko_ecs::ecs::core::iters::query::query::Query;
use boyko_ecs::ecs::core::system::Res;

use crate::binding::UiTextBuffer;
use crate::components::{ComputedRect, ContentSize};

use super::components::UiText;
use super::font::FontTable;
use super::shape::shape_into;

/// The change-gated measure system (GUI P5b). For every node whose `UiTextBuffer` or
/// `UiText` changed this frame, shapes the content into the node's font and writes the
/// run extent to `ContentSize` set-if-changed (so a same-value re-measure bumps no
/// tick — a quiet steady state for value-bound labels whose displayed value did not
/// actually change). Reads `ComputedRect.w` as the wrap width (0 ⇒ single line).
///
/// Scheduled BEFORE the layout discovery pass.
//
// `clippy::type_complexity`: the `Query<.., Or<(Changed<…>, …)>>` change-set type IS
// the SystemParam signature (resolved positionally), so it cannot be a `type` alias
// without losing the SystemParam impl. Allowed (mirrors `ui_layout_discovery`).
#[allow(clippy::type_complexity)]
pub fn ui_text_measure_system(
    mut nodes: Query<
        (&UiText, &UiTextBuffer, &ComputedRect, &mut ContentSize),
        Or<(Changed<UiTextBuffer>, Changed<UiText>)>,
    >,
    fonts: Res<FontTable>,
) {
    // No font loaded yet ⇒ nothing measurable; leave ContentSize untouched.
    if fonts.is_empty() {
        return;
    }
    for (text, buffer, rect, content_size) in nodes.iter_mut() {
        let measured = measure_one(text, buffer.as_str(), rect, &fonts);
        // Set-if-changed: a re-measure that produced the same size bumps no tick, so a
        // value-bound label whose text is unchanged stays quiet for the relayout gate.
        if (content_size.width - measured.width).abs() > f32::EPSILON
            || (content_size.height - measured.height).abs() > f32::EPSILON
        {
            content_size.width = measured.width;
            content_size.height = measured.height;
        }
    }
}

/// The shaped run extent for one node (logical px) — the testable measure core (no
/// world / Query). Returns a zero size when the font is unloaded, the size is
/// non-positive, or the content is empty (so an empty label hugs to nothing).
pub fn measure_one(
    text: &UiText,
    content: &str,
    rect: &ComputedRect,
    fonts: &FontTable,
) -> ContentSize {
    if content.is_empty() || text.size_px <= 0.0 {
        return ContentSize::default();
    }
    let Some(font) = fonts.entry(text.font) else {
        return ContentSize::default();
    };
    // The wrap width is the node's known content width (0 ⇒ unwrapped, the natural
    // single-line intrinsic width an `Auto` node hugs on the first measure).
    let extent = shape_into(content, font, text.size_px, rect.w, text.align, |_g| {});
    ContentSize {
        width: extent.width,
        height: extent.height,
    }
}
