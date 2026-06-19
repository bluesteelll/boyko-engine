//! The `boyko_input` prelude — the common surface for game code.
//!
//! `use boyko_input::prelude::*;`

pub use crate::action::actionlike::{ActionKind, Actionlike};
pub use crate::action::map::{
    AxisMode, BindSpec, ClashStrategy, InputMap, InputMapBuilder, InputRef,
};
pub use crate::action::process::process_actions;
pub use crate::action::state::ActionState;
pub use crate::raw::event::RawInputEvent;
pub use crate::raw::keycode::{ButtonState, KeyCode, MouseButton, ScrollDelta};
pub use crate::raw::queue::{PhysicalInput, RawInputQueue};

// The derive is re-exported so `use boyko_input::prelude::*;` brings it in
// alongside the trait it implements.
pub use boyko_macros::Actionlike;
