//! Data binding (GUI P4): push a change-gated ECS source field into a widget's
//! inline text/value sink via a reflection-free codegen accessor table.
//!
//! * [`bindable`] — the [`Bindable`] codegen trait (`#[derive(Bindable)]`).
//! * [`components`] — `BindText` / `BindValue` (topology) + `UiTextBuffer` /
//!   `UiValue` (sinks) + `TemplateId`.
//! * [`bind_system`] — `ui_bind_discovery` (0%-gate) + `ui_bind_apply`
//!   (tick-gated, set-if-changed).

pub mod bind_system;
pub mod bindable;
pub mod components;

pub use bind_system::{ui_bind_apply, ui_bind_discovery, UiBindScratch};
pub use bindable::Bindable;
pub use components::{
    BindText, BindValue, TemplateId, UiTextBuffer, UiValue, NO_FIELD,
};
