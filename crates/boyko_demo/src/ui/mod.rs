//! egui control panel for the demo (plan §7 / Wave 4).
//!
//! Pure egui: the panel reads/writes a borrowed [`SimParams`](crate::sim::resources::SimParams)
//! and reads a borrowed [`FrameStats`](crate::sim::resources::FrameStats). It owns
//! no ECS or GPU state — the app shell ([`crate::app`]) owns those and passes
//! mutable/shared references in. Keeping the panel dependency-light makes it the
//! single place to evolve the controls without touching the sim or render layers.

pub mod panel;
