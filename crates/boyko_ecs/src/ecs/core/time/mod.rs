//! Phase 20 — the engine clock family: [`Time`] (virtual frame clock),
//! [`FixedTime`] (fixed-timestep accumulator), and [`fixed_advance`] (the
//! unified fixed-loop driver).
//!
//! Two separate resources, no generic-`Time` swap (plan D2): frame systems
//! read `Res<Time>`, fixed-schedule systems read `Res<FixedTime>` — the type
//! IS the documentation of which clock a system marches to. All accumulator
//! math is integer-nanosecond [`Duration`](std::time::Duration) arithmetic,
//! so step counts and `FixedTime::elapsed` are bit-deterministic for a given
//! raw-delta sequence (plan D3/D4).
//!
//! [`fixed_advance`] is the SAME code path on native, wasm, and Miri: it has
//! no pool, no `Instant`, no platform branch — `App::update_with_delta` calls
//! it with `|w| fixed.run(w)`, a pool-less runner calls it with a sequential
//! step closure (plan D3, the unified-path rule).

pub mod fixed_loop;
pub mod fixed_time;
#[allow(clippy::module_inception)]
pub mod time;

pub use fixed_loop::fixed_advance;
pub use fixed_time::FixedTime;
pub use time::Time;
