//! `boyko_demo` — interactive showcase for the boyko-engine ECS.
//!
//! The crate is library-shaped so the simulation can be exercised headlessly by
//! integration tests (`tests/sim_smoke.rs`) without opening a window: the `sim`
//! module is pure `boyko_ecs` (no GPU), and the `render`/`app` modules are the
//! GPU/eframe shell. The binary entry point lives in `main.rs` and is a thin
//! wrapper over [`app::DemoApp`].
//!
//! Wave 3 (plan §10) wires the ECS into the renderer: a real `boyko_ecs` world +
//! `Schedule::run` + `par_iter_mut` integration drives a zero-copy `for_each_chunk`
//! SoA->GPU upload into the instanced renderer.

pub mod app;
pub mod render;
pub mod sim;
