//! Action-emitting interaction components (GUI P4 Decisions 2/3).
//!
//! Each carries a dense `Actionlike::index()` as a raw `u16`, resolved at
//! authoring/parse time — NOT a generic `OnClick<A>`. The dispatch system writes
//! the action by `usize` index via `ActionState::ui_press`, so it never
//! monomorphizes per action enum and the components are authorable from `.ui`
//! text (an integer is the reflection-free common denominator). All
//! `#[repr(transparent)]` POD `Copy`.

use boyko_macros::Component;

/// The sentinel "no action" index for an action-emitting component whose action
/// is unresolved (e.g. a `.ui` action-name that did not resolve). The dispatch
/// system treats it as "fire nothing".
pub const NO_ACTION: u16 = u16::MAX;

/// Emit action `index` (dense `Actionlike::index()`) on a release-up click over
/// this node (Decisions 2/3): press-inside → release-inside-same-node.
/// Reflection-free (an integer). `#[repr(transparent)]`.
#[repr(transparent)]
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub struct OnClick(pub u16);

/// Emit action `index` on hover-enter (`None` → `Hovered`). OPT-IN.
/// `#[repr(transparent)]`.
#[repr(transparent)]
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub struct OnHover(pub u16);

/// Emit action `index` on a submit edge (Enter while this node is focused).
/// OPT-IN. `#[repr(transparent)]`.
#[repr(transparent)]
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub struct OnSubmit(pub u16);
