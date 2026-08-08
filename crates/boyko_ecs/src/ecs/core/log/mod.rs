//! The logging seam — the ECS half of `boyko_log`.
//!
//! # Why this lives inside `boyko_ecs` rather than in `boyko_log`
//!
//! [`LogRing`] is backed by [`VmColumn`](crate::ecs::memory::vm_column::VmColumn), which is
//! `pub(crate)` to this crate, and it must be: the column's soundness argument rests on
//! invariants (`base` is write-once, every mutation takes `&mut self`, cross-thread `&self` reads
//! touch only committed POD) that a foreign crate could not be held to. Putting the ring here is
//! what lets it be engine storage rather than a `Box<[u8]>` side-store — the shape Principle 0
//! forbids **even inside a `Resource`**.
//!
//! The dependency edge therefore runs `boyko_ecs -> boyko_log -> boyko_diag`, never back. The
//! logger knows nothing about the ECS; it writes bytes into a `.bss` ring
//! ([`boyko_log::sink::ecs`]) and one system here copies them out.
//!
//! # What this rung is, and what it is not
//!
//! **Rung L5.** What exists: [`LogLine`], [`LogRing`], [`LogStats`], [`LogPlugin`],
//! [`log_drain_system`], and the manual `Send`/`Sync` impls with their compile-time pins.
//!
//! What does **not** exist yet, by rung: `LogRing::since` / `RingFilter` / `LogRingIter::skipped`,
//! the `frame_epoch` record, `LogCensus`, `DiagCensus` and the `TARGET_STATS` snapshot are **L16**
//! — the ring here is written, not yet read back. The lane-side loss fold into [`LogStats`]
//! (`emitted`, `dropped`, `sampled_out`, …) is **L13a**. Each is absent rather than present and
//! zero: a field that is structurally always zero is indistinguishable from a measurement of
//! zero, and a HUD cannot tell the difference.

pub mod plugin;
pub mod ring;
pub mod stats;

pub use plugin::{LogPlugin, LogSet, log_drain_system};
pub use ring::{ARENA_BYTES, LINE_CAP, LogLine, LogRing};
pub use stats::LogStats;
