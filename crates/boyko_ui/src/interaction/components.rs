//! Pointer-interaction components — the per-node interaction state and opt-in
//! pointer/focus inputs (GUI P4).
//!
//! All POD `Copy` (`Send + Sync`) like the layout components. The tick-bearing
//! [`Interaction`] column is the `Changed`-gate source for `ui_dispatch_system`;
//! the EnableTag bits (`UiHovered`/`UiPressed`/`UiFocused`) are the O(1) filter
//! surface for downstream render/styling systems (Decision 1).

use boyko_macros::Component;

/// Pointer interaction state, recomputed each frame by `ui_focus_system`.
///
/// Tick-bearing column (drives `Changed<Interaction>`). Written set-if-changed,
/// so a still frame bumps no tick (Decision 1). `#[repr(u8)]`.
#[repr(u8)]
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Interaction {
    /// Cursor not over this node (or occluded / invisible).
    #[default]
    None,
    /// Cursor over the node, button not pressed on it.
    Hovered,
    /// Button pressed on this node (held).
    Pressed,
}

/// Cursor position relative to the node, normalized to `(-0.5..0.5)^2`, center
/// `[0, 0]`. OPT-IN (sliders, drag handles). 12 B.
///
/// Written set-if-changed only on the hovered node; on a leave
/// (`cursor_over = false`) `normalized` is reset to the canonical `[0.0, 0.0]`
/// BEFORE the equality compare, so a leave does not leave residual bytes that
/// defeat the set-if-changed gate (Decision 1 minor fix).
#[repr(C)]
#[derive(Component, Clone, Copy, Debug, Default, PartialEq)]
pub struct RelativeCursorPosition {
    /// Whether the cursor is currently over this node.
    pub cursor_over: bool,
    /// Normalized cursor position; canonical `[0.0, 0.0]` when `!cursor_over`.
    pub normalized: [f32; 2],
}

/// Hit-test propagation policy. OPT-IN, default [`FocusPolicy::Pass`].
///
/// `Block` stops HOVER RESOLUTION (no node painted below it can become the
/// hovered node); it does NOT skip the unconditional reset pass — a node
/// occluded by a `Block` node this frame is still reset to
/// [`Interaction::None`] (Decision, focus step 3b). `#[repr(u8)]`.
#[repr(u8)]
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FocusPolicy {
    /// Occlude nodes painted below: stop hover resolution at this node.
    Block,
    /// Pass through: a node painted below may also become hovered.
    #[default]
    Pass,
}

/// Marks a node keyboard-focusable and carries its linear tab order. OPT-IN.
///
/// Cross-root tab order is a total order: (root enumeration order, then
/// `tab_index`, then `Entity`) — see `ui_focus_system` step 7.
/// `#[repr(transparent)]`.
#[repr(transparent)]
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Focusable {
    /// Linear tab order key within the total cross-root order.
    pub tab_index: u32,
}
