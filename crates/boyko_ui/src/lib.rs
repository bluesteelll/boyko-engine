//! boyko_ui — ECS-native UI. P1: layout components + the in-house layout systems.
//!
//! Widgets are entities; layout inputs/outputs are components; the tree is
//! `ChildOf`/`Children` (Phase 19); layout is two systems over the ECS. There is
//! no parallel data system — props/outputs are ECS columns and the per-frame
//! scratch is a `Resource`-owned engine buffer (frame-transient, reset every
//! frame).
//!
//! # The two systems
//!
//! * [`ui_layout_discovery`](layout::ui_layout_discovery) — a normal scheduled
//!   `FunctionSystem`. SystemParams supply the `(last_run, this_run]` window that
//!   `Changed`/`Added` require, so this is where change detection lives. It sets a
//!   per-frame `dirty` flag in [`LayoutScratch`](resources::LayoutScratch).
//! * [`ui_layout_apply`](layout::ui_layout_apply) — an exclusive system
//!   (`&mut EcsMaster`, the only form that expresses nested parent↔child mutable
//!   row access without unsafe aliasing). When `dirty` (or the viewport resized)
//!   it re-lays-out the root subtrees.
//!
//! Schedule them in that order, after all structural/prop-mutation systems (the
//! `Children`/`ChildOf` consistency window).
//!
//! # GUI P5b text (host-driven scheduling — Decision T5-B)
//!
//! [`ui_text_measure_system`](text::ui_text_measure_system) MUST be registered
//! `.before(ui_layout_discovery)` so the same-frame relayout sees the new
//! [`ContentSize`](components::ContentSize) (the layout lists `Changed<ContentSize>` as
//! a relayout trigger and reads it as the leaf intrinsic size). Like the layout pair,
//! the ORDER is the host's responsibility (P5b ships the system, not an App schedule);
//! the CPU-boundary correctness of the seam is unit-proven (`measure_one` matches a
//! shaped run; an `Auto` leaf hugs the measured size).

pub mod anchor;
pub mod binding;
pub mod bundles;
pub mod components;
pub mod interaction;
pub mod layout;
pub mod plugin;
pub mod reload;
pub mod resources;
pub mod text;
pub mod units;
pub mod widgets;
pub mod world;

/// The `ui!` authoring macro (P2): write a UI entity tree as a literal nested
/// block. See [`boyko_macros::ui`] for the grammar and expansion contract.
pub use boyko_macros::ui;

/// The `#[derive(Bindable)]` macro (P4): generate the reflection-free per-field
/// accessor + the type-erased `BindAccessor` registration. Re-exported next to
/// the [`Bindable`](binding::Bindable) trait (the `ui!`/`Component` derive
/// re-export pattern).
pub use boyko_macros::Bindable;

/// The `.ui` text format (P3): [`parse_ui`](text::parse_ui),
/// [`spawn_ui_tree`](text::spawn_ui_tree), [`serialize_ui`](text::serialize_ui),
/// and the runtime hot-reload [`UiPlugin`](plugin::UiPlugin).
pub use plugin::UiPlugin;
pub use text::{parse_ui, serialize_ui, spawn_ui_tree, UI_FORMAT_VERSION};

/// GUI P5b text rendering: the [`UiText`](text::UiText) style component +
/// [`FontTable`](text::FontTable) resource, the glyph emitter, the measure system, and
/// the `.bfont` loader. Text RIDES the P5a instanced-quad path (one pipeline, one
/// draw, premultiplied blend).
pub use text::{
    emit_glyphs, measure_one, read_bfont, ui_text_measure_system, BakedFont, FontEntry, FontId,
    FontTable, GlyphInstance, ShapedExtent, ShapedGlyph, TextAlign, TextEmitScratch, TextNode,
    UiText,
};

/// Crate-local convenience re-exports (no engine-wide prelude).
pub mod prelude {
    pub use crate::binding::{
        ui_bind_apply, ui_bind_discovery, BindText, BindValue, Bindable, TemplateId, UiBindScratch,
        UiTextBuffer, UiValue, NO_FIELD,
    };
    pub use crate::anchor::{resolve_anchor_origin, AnchorOrigin};
    pub use crate::bundles::{
        BarBundle, ButtonBundle, GridBundle, ImageBundle, LabelBundle, PanelBundle, UiNodeBundle,
    };
    pub use crate::components::{
        AnchorEdge, Bar, BarFill, Button, ComputedClip, ComputedRect, ContentSize, StackIndex,
        UiAbsolute, UiAlign, UiAnchor, UiBackground, UiGrid, UiImage, UiLayout, UiName, UiRoot,
        UiSpacing,
    };
    pub use crate::interaction::{
        register_bindable, ui_dispatch_system, ui_focus_system, ui_refreeze_fixed_snapshot,
        FocusPolicy, Focusable, Interaction, OnClick, OnHover, OnSubmit, RelativeCursorPosition,
        UiBindSet, UiBindingPlugin, UiInputFocus, UiInteractionConfig, UiInteractionPlugin,
        UiInteractionScratch, UiPointerState, NO_ACTION,
    };
    pub use crate::layout::{ui_layout_apply, ui_layout_discovery};
    pub use crate::plugin::UiPlugin;
    pub use crate::resources::{LayoutScratch, UiSafeArea, UiViewport};
    pub use crate::widgets::{
        ui_bar_apply, ui_bar_discovery, UiBarScratch, UiWidgetSet, UiWidgetsPlugin,
    };
    pub use crate::text::{
        emit_glyphs, measure_one, parse_ui, read_bfont, serialize_ui, spawn_ui_tree,
        ui_text_measure_system, BakedFont, FontId, FontTable, GlyphInstance, TextAlign,
        TextEmitScratch, TextNode, UiText, UI_FORMAT_VERSION,
    };
    pub use crate::units::{AlignCross, AlignMain, LayoutType, PositionType, Unit};
    pub use crate::world::{
        project_world_to_screen, ui_world_project_system, ui_world_visibility_system,
        HoveredWorldEntity, ProjectedPoint, UiWorldAnchor, UiWorldCulled, UiWorldHidden,
        UiWorldProjection, UiWorldScratch, WorldScaleMode, WorldTarget,
    };

    pub use boyko_macros::{ui, Bindable};
}
