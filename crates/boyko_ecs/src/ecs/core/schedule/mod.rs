//! Phase 9 schedule subsystem.
//!
//! Wave 3 Step 8 lands the schedule module skeleton plus [`SystemBox`].
//! Wave 4 Step 9 / Step 10 layer the user-facing [`ScheduleBuilder`] +
//! the cycle detection (Tarjan SCC) + topological sort (Kahn's) +
//! [`ConflictGraph`] build phase. The hot-path executor (`Schedule::run`
//! body) lands in Wave 5 Step 12.
//!
//! See Phase 9 plan §5 for the full design and §14 Step 9 / Step 10 for
//! the acceptance criteria.
//!
//! [`SystemBox`]: system_box::SystemBox
//! [`ScheduleBuilder`]: schedule_builder::ScheduleBuilder
//! [`ConflictGraph`]: conflict_graph::ConflictGraph

pub(crate) mod bitset_intersects;
pub(crate) mod conflict_graph;
pub(crate) mod executor_scratch;
pub(crate) mod ordering;
#[allow(clippy::module_inception)]
pub mod schedule;
pub mod schedule_builder;
pub mod system_config;
pub(crate) mod system_descriptor;
pub mod system_set;

// Internal — `SystemBox` is consumed only by other submodules.
pub(crate) mod system_box;

pub use schedule::Schedule;
pub use schedule_builder::{
    ConfigureSet, MAX_SYSTEMS_PER_SCHEDULE, ScheduleBuildError, ScheduleBuilder,
};
pub use system_config::SystemConfig;
pub use system_set::{SystemSet, SystemSetId};

// Future submodules (wired in later Phase 9 waves):
//   pub mod apply_deferred; // Wave 5 Step 14 (sync-point analyzer)
