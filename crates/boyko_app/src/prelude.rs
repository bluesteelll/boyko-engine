//! Common `boyko_app` imports, collapsed into one glob:
//! `use boyko_app::prelude::*;` — kept minimal for R2 (App, AppExit,
//! EnginePlugins).

pub use boyko_ecs::{App, AppExit};

pub use crate::plugins::EnginePlugins;
