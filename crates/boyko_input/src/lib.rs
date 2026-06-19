//! `boyko_input` — source-agnostic, rebindable action mapping for boyko-engine.
//!
//! Turns raw keyboard/mouse events from **any** source (native raw-FFI Win32
//! window, egui demo, synthetic test stream) into typed, rebindable **actions**
//! consumed by ECS systems. The engine path depends on NO windowing library
//! (winit/Win32/eframe live behind feature-gated edge adapters only), so the
//! action layer compiles on every target including wasm.
//!
//! # Layers
//! - [`raw`] — the source-agnostic raw layer: canonical physical enums, the
//!   single seam event, the ring buffer + per-frame snapshot, scancode tables.
//! - [`action`] — typed actions ([`Actionlike`](action::actionlike::Actionlike)),
//!   the binding map ([`InputMap`](action::map::InputMap)), the SoA action state
//!   ([`ActionState`](action::state::ActionState)), and the per-frame
//!   aggregation ([`process_actions`](action::process::process_actions)).
//!
//! # Scope
//! This crate now covers the windowing-independent core (I1–I3) **and** ECS
//! integration (I4): [`InputPlugin`](plugin::InputPlugin), the
//! [`update_action_state`](action::process::update_action_state) ingest system,
//! per-`A` generic-resource id minting, and the fixed-step determinism snapshot.
//! `.keys` persistence, runtime rebinding, and the Win32 / egui adapters are
//! added in later rounds (I5–I7); the seams are documented at each module.

// Resolve the crate's own name so the `#[derive(Actionlike)]` macro — which
// emits absolute `::boyko_input::…` paths — works from this crate's own unit
// tests, not only from downstream consumers.
extern crate self as boyko_input;

pub mod action;
pub mod constants;
pub mod plugin;
pub mod prelude;
pub mod raw;

pub use action::actionlike::{ActionKind, Actionlike};
pub use action::map::{
    AxisMode, BindSpec, ClashStrategy, InputMap, InputMapBuilder, InputRef,
};
pub use action::process::{process_actions, update_action_state};
pub use action::state::ActionState;
pub use plugin::{GameplaySet, InputPlugin};
pub use raw::event::RawInputEvent;
pub use raw::keycode::{ButtonState, KeyCode, MouseButton, ScrollDelta};
pub use raw::queue::{PhysicalInput, RawInputQueue};

// Re-export the derive so `boyko_input::Actionlike` (derive) sits next to the
// trait of the same name (the `#[derive(Component)]` re-export pattern).
pub use boyko_macros::Actionlike;
