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

// Profiling rung 15 — the one line every crate that declares a zone must write, and this is a GAME.
//
// `User` is not a formality here, it is the whole point of the two-region split: a game's zones
// draw from the user id range and the user ring region, so a runaway game scope costs the game's
// samples and never the engine's. The partition is keyed on the DECLARING CRATE, not on which macro
// a site used, so writing it once at the crate root is what makes every `declare_zone!` below a user
// zone — including one added later by someone who never read this comment.
//
// It is also load-bearing rather than decorative: `declare_zone!` expands to `crate::
// __BOYKO_ZONE_PARTITION`, so a crate that declares a zone without this line does not compile. That
// is the intended outcome — an unpartitioned zone has no region to be isolated in.
boyko_diag::profiling_partition!(User);

pub mod app;
pub mod render;
pub mod sim;
pub mod ui;
