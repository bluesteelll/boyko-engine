//! [`LogStats`] — the counters the ECS seam itself produces.

use std::sync::OnceLock;

use crate::ecs::core::resources::register_new;
use crate::ecs::core::resources::resource::Resource;
use crate::ecs::identifiers::primitives::ResourceId;

/// Monotonic counters from the logging seam. `Copy` POD; zero per-frame allocation.
///
/// # One field, and the reason there is only one
///
/// The specified game-facing surface has eleven: `emitted`, `dropped`, `dropped_bytes`,
/// `suppressed`, `unlaned_dropped`, `sampled_out`, `handoff_lost`, `codes_unindexed`,
/// `lanes_claimed`, `lanes_retired`, `lanes_leaked`. Ten of them are folds of state this rung does
/// not own: the lane-side loss fold over `boyko_diag::loss` is L13a, the rate limiter's
/// `suppressed` is L4, `sampled_out` is L12, `codes_unindexed` is L11a. Declaring them now would
/// put ten fields in a `Resource` that read `0` forever, and **a value that is structurally always
/// zero is indistinguishable from a measurement of zero** — a HUD showing `emitted: 0` while the
/// log streams is worse than a HUD that does not offer the number yet.
///
/// So each field arrives with the rung that can fill it. The set this rung fills is pinned by a
/// test, so the next rung's addition is a deliberate edit rather than a silent one.
///
/// # What a game may NOT do with these numbers
///
/// Gameplay may not branch on them. They are lower bounds under drop, schedule-dependent,
/// non-deterministic across machines, and therefore break replay. Display and telemetry only;
/// gameplay counters belong in the game's own components, which is Principle 0's answer.
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct LogStats {
    /// Formatted lines that reached the byte sinks but not the in-frame view — the handoff ring
    /// refused them for want of space.
    ///
    /// Charged as `boyko_diag::LossClass::Sink` rather than `Overflow`, because the record is not
    /// lost: only this view of it is short. Cumulative for the process, mirroring the ring's own
    /// counter rather than accumulating a second, divergent copy.
    pub handoff_lost: u64,
}

// Hand-implemented rather than `#[derive(Resource)]`: `boyko-macros` is a dev-dependency of
// `boyko-ecs`, so its derives are unavailable in normal builds.
impl Resource for LogStats {
    #[inline]
    fn resource_id() -> ResourceId {
        static ID: OnceLock<ResourceId> = OnceLock::new();
        *ID.get_or_init(|| ResourceId(register_new::<Self>()))
    }
}
