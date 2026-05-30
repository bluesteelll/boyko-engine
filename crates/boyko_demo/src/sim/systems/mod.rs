//! Simulation systems (plan §6.5).
//!
//! Each system is a plain `fn` with `SystemParam` arguments, registered into the
//! native `Schedule` (and, in a later wave, called by the wasm sequential
//! runner). They are shared, target-agnostic logic; only the dispatch shell
//! differs per platform (plan D10).

pub mod common;
pub mod particles;
