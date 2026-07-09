//! The `boyko_input` prelude — the common surface for game code.
//!
//! `use boyko_input::prelude::*;`

pub use crate::action::actionlike::{ActionKind, Actionlike};
pub use crate::action::map::{
    AxisMode, BindSpec, ClashStrategy, InputMap, InputMapBuilder, InputRef,
};
pub use crate::action::names::{register_action_names, resolve_action_name};
pub use crate::action::process::{process_actions, update_action_state};
pub use crate::action::rebind::{RebindOutcome, RebindSession};
pub use crate::action::state::ActionState;
pub use crate::persist::{keys_to_string, load_keys, save_keys, ParseReport};
pub use crate::plugin::{GameplaySet, InputPlugin};
pub use crate::raw::event::RawInputEvent;
pub use crate::raw::keycode::{ButtonState, KeyCode, MouseButton, ScrollDelta};
pub use crate::raw::queue::{PhysicalInput, RawInputQueue};

// The derive is re-exported so `use boyko_input::prelude::*;` brings it in
// alongside the trait it implements.
pub use boyko_macros::Actionlike;
