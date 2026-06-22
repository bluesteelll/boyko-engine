//! Pointer + keyboard interaction (GUI P4): hit-test, `Interaction` edges, and
//! action dispatch.
//!
//! * [`components`] — `Interaction` / `RelativeCursorPosition` / `FocusPolicy` /
//!   `Focusable`.
//! * [`action`] — `OnClick` / `OnHover` / `OnSubmit` (dense action indices).
//! * [`focus`] — `ui_focus_system` (the exclusive hit-test + interaction writer)
//!   and its resources (`UiPointerState` / `UiInputFocus` / `UiInteractionConfig`
//!   / `UiInteractionScratch`).
//! * [`dispatch`] — `ui_dispatch_system<A>` (Interaction edge → `ActionState`).

pub mod action;
pub mod components;
pub mod dispatch;
pub mod focus;
pub mod plugin;

pub use action::{OnClick, OnHover, OnSubmit, NO_ACTION};
pub use components::{FocusPolicy, Focusable, Interaction, RelativeCursorPosition};
pub use dispatch::{ui_dispatch_system, ui_refreeze_fixed_snapshot};
pub use focus::{
    ui_focus_system, PointerSlot, UiInputFocus, UiInteractionConfig, UiInteractionScratch,
    UiPointerState, MAX_POINTERS,
};
pub use plugin::{register_bindable, UiBindSet, UiBindingPlugin, UiInteractionPlugin};
