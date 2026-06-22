//! Canonical node bundles.
//!
//! [`UiNodeBundle`] is the always-present node base. Bundling exactly the two
//! components every laid-out node carries lets the common node hit the Phase-8.5
//! static archetype cache as a single 2-component unit in one spawn — the
//! `ui!` macro spawns this base whenever a node's component set contains both
//! `UiLayout` and `ComputedRect`.

use boyko_macros::Bundle;

use crate::binding::{UiTextBuffer, UiValue};
use crate::components::{
    Bar, Button, ComputedRect, ContentSize, UiBackground, UiGrid, UiImage, UiLayout,
};
use crate::interaction::{Focusable, Interaction, OnClick};
use crate::text::UiText;

/// The always-present node base: `UiLayout` (primary layout input) + the
/// `ComputedRect` output every laid-out node carries.
///
/// Hits the Phase-8.5 static archetype cache as a single 2-component unit (one
/// per-world `OnceLock<ArchetypeId>` slot). The `ui!` macro spawns this base
/// when a node's component set contains BOTH `UiLayout` and `ComputedRect`
/// (set-based recognition); otherwise it spawns `UiLayout` and injects
/// `ComputedRect::default()`.
///
/// NOTE: this is the node BASE, not its final archetype. The hierarchy hooks
/// migrate archetypes on linking — a child gains `ChildOf`, a parent gains
/// `Children` — so the final archetype is `UiNodeBundle (+ opts) (+ ChildOf)
/// (+ Children)`.
#[derive(Bundle)]
pub struct UiNodeBundle {
    /// Primary layout input.
    pub layout: UiLayout,
    /// Resolved screen-space rectangle (the renderer's only geometry input).
    pub rect: ComputedRect,
}

// ───────────────────────── GUI P6a widget presets ─────────────────────────
//
// Rust-only ergonomic presets (C1): each is a `#[derive(Bundle)]` NAMED struct
// that expands to the SAME component set the canonical authorable form spawns, so
// `cmds.spawn(ButtonBundle { .. })` hits the Phase-8.5 static archetype cache as
// one unit. These are NOT `ui!`/`.ui` type-names — in `ui!`/`.ui` a widget is
// authored as its explicit component list (markers + style + layout), which is
// what the `.ui`≡`ui!`≡hand-spawn equivalence gate compares. The bundles are the
// hand-spawn convenience layer over that same set.

// The `#[derive(Bundle)]` collects each FIELD as one component (it calls
// `T::component_id()` per field — no nested-bundle flattening), so the widget
// presets list FLAT components (`layout` + `rect` + the widget's set) rather than
// embedding `UiNodeBundle`. Every preset still carries `UiLayout` + `ComputedRect`
// (the node base) so it hits the layout solver and never trips `missing_rect`.

/// A styled container: a node with a [`UiBackground`] fill. The simplest widget —
/// no marker (a Panel is "a node with a background"). Add a `UiSpacing` for
/// padding/borders separately.
#[derive(Bundle)]
pub struct PanelBundle {
    /// Primary layout input.
    pub layout: UiLayout,
    /// Resolved screen-space rectangle (the renderer's geometry input).
    pub rect: ComputedRect,
    /// Visual fill / border.
    pub background: UiBackground,
}

/// A text label: the text STYLE ([`UiText`]) + its inline buffer
/// ([`UiTextBuffer`]) + the measured [`ContentSize`] an `Auto` leaf hugs. The
/// buffer is filled by a `BindText`/direct write; P5b shapes + emits it.
#[derive(Bundle)]
pub struct LabelBundle {
    /// Primary layout input.
    pub layout: UiLayout,
    /// Resolved screen-space rectangle.
    pub rect: ComputedRect,
    /// Text style (color, size, font, align).
    pub text: UiText,
    /// Inline render-facing text buffer (the bind sink).
    pub buffer: UiTextBuffer,
    /// Leaf intrinsic size (the measure seam).
    pub content: ContentSize,
}

/// An interactive button: a styled panel + the interaction state + focusability +
/// an action, tagged with the [`Button`] marker.
#[derive(Bundle)]
pub struct ButtonBundle {
    /// Primary layout input.
    pub layout: UiLayout,
    /// Resolved screen-space rectangle.
    pub rect: ComputedRect,
    /// Visual fill / border.
    pub background: UiBackground,
    /// Button identity marker.
    pub marker: Button,
    /// Per-node interaction state (written by `ui_focus_system`).
    pub interaction: Interaction,
    /// Keyboard focusability + tab order.
    pub focusable: Focusable,
    /// Action emitted on a release-up click.
    pub on_click: OnClick,
}

/// A progress/health bar TRACK: a styled panel carrying the [`Bar`] marker + the
/// [`UiValue`] fraction sink. The FILL child (a node with
/// [`BarFill`](crate::components::BarFill) + its own `UiLayout`/`UiBackground`) is
/// spawned separately and parented to the track; `ui_bar_apply` drives the fill's
/// main-axis `Unit::Pct` from `UiValue`.
#[derive(Bundle)]
pub struct BarBundle {
    /// Primary layout input.
    pub layout: UiLayout,
    /// Resolved screen-space rectangle.
    pub rect: ComputedRect,
    /// Track fill / border.
    pub background: UiBackground,
    /// Bar-track identity marker.
    pub marker: Bar,
    /// The `0..1` fraction sink (the P4 `BindValue` target).
    pub value: UiValue,
}

/// An image node: a [`UiImage`] fill + a [`UiLayout`]. Layout-complete and
/// authorable in P6a; the P5a pack path learns `UiImage` in a follow-up (a
/// transparent-tint default renders nothing until then).
#[derive(Bundle)]
pub struct ImageBundle {
    /// Primary layout input.
    pub layout: UiLayout,
    /// Resolved screen-space rectangle.
    pub rect: ComputedRect,
    /// Image fill (texture handle + UV + tint).
    pub image: UiImage,
}

/// A grid container: a [`UiLayout`]`{ layout_type: Grid }` + the [`UiGrid`] track
/// config. Children parented to it are placed into uniform `columns × rows` cells
/// by the layout solver.
#[derive(Bundle)]
pub struct GridBundle {
    /// Primary layout input — set `layout.layout_type = LayoutType::Grid`.
    pub layout: UiLayout,
    /// Resolved screen-space rectangle.
    pub rect: ComputedRect,
    /// Uniform grid track config (columns / rows).
    pub grid: UiGrid,
}
