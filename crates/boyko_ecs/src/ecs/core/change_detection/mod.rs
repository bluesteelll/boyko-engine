//! Phase 10 change detection foundations: [`Tick`] type, wraparound
//! constants, and the [`run_check_ticks_scan`] clamp scan.
//!
//! See [`docs/PHASE-10-CHANGE-DETECTION-PLAN.md`] for the full design.
//!
//! # Wave layout
//!
//! * Wave A (Steps 1-4) — [`Tick`] + wraparound constants.
//! * Wave B (Steps 5-6) — per-pool tick storage + `Archetype::create_entity`
//!   threading.
//! * Wave C (Steps 7-10) — `Added` / `Changed` filters and `Ref` / `Mut`
//!   `QueryData` impls.
//! * Wave D (Steps 12-14) — [`run_check_ticks_scan`] cold path, scheduler
//!   tick bump, per-system `set_change_ticks` dispatch, and
//!   `FunctionSystem` / `ExclusiveFunctionSystem` constructor refit.

pub mod tick;

pub(crate) mod check_ticks;

pub use tick::{CHECK_TICK_PREEMPT_MARGIN, CHECK_TICK_THRESHOLD, MAX_CHANGE_AGE, Tick};

pub(crate) use check_ticks::run_check_ticks_scan;
