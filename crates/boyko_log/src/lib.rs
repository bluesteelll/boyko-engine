//! In-house structured logging for boyko-engine.
//!
//! # What this crate is for
//!
//! One logging system for the engine **and** for games built on it, with a cost model stated
//! rather than hoped for:
//!
//! | Configuration | What a call site costs |
//! |---|---|
//! | above the compile ceiling | **nothing** — the site and its argument expressions are deleted |
//! | under the ceiling, target `Off` | one `.bss` byte load and one predicted branch |
//! | enabled | the load, the branch, and the record |
//!
//! The middle row is the honest one and it is why there are two axes rather than one. A runtime
//! flag has to be *read* in order to be a flag, so no runtime setting reaches zero per-site cost;
//! only the compile-time ceiling does, by removing the site. Keeping both means a **shipped**
//! binary can still be asked for a log, which a compile-only design cannot offer.
//!
//! # No third-party facade
//!
//! `log` and `tracing` are not dependencies and will not become them. Their level filter is a
//! runtime one, so a `trace!` in a shipping build still evaluates its gate; and a Cargo feature
//! ceiling is *additive and unified*, so one crate in the graph enabling `max_level_trace`
//! re-enables it for everyone. The compile ceiling here comes from a build profile, which has no
//! such failure mode.
//!
//! # State of this crate
//!
//! **Rungs L0 through L6**, and the list is re-measured against the tree at each rung rather than
//! carried forward — this paragraph claimed "L4 … which this crate skipped" while `sink/file.rs`,
//! `rate.rs` and `census.rs` were all in the directory beside it.
//!
//! What exists: [`Level`], [`LogTarget`], [`TargetId`], [`TargetControl`], the engine target
//! table, the control array and its epoch counter, the five macros with their full three-gate
//! expansion, the per-site `static`, the **tagged** payload encoding ([`LogValue`], [`ValueTag`],
//! [`LogArgs`], [`dsp!`]) and its one walker [`render_payload`], the per-lane SPSC ring and the
//! producer path [`emit_impl`] (L0/L1); the diagnostic-code registry ([`codes`]) (L2); the
//! synchronous channel ([`sync_out`]), the CAS'd consumer role ([`drain_owner`]), the drain walk,
//! the opt-in sink thread, `flush`/`shutdown`, the chained panic hook and the `boot`/`enable`
//! split ([`lifecycle`]) (L3); the file sink and its byte cap, the rate limiter and the
//! `LOG-CENSUS` ([`sink::file`], [`rate`], [`census`]) (L4); the sink→ECS transport [`sink::ecs`],
//! whose consumer lives in `boyko_ecs` (L5); and the engine's own emitters, in `boyko_ecs` and
//! `boyko_threadpool` rather than here (L6).
//!
//! What does **not** exist yet, by rung: dynamic targets are L10, `LogPod` L11b, sampling L12, the
//! binary sink and `logdec` L13b, runtime sink control L14, and the crash path L15. Each is
//! **absent rather than stubbed**: a step that returns without doing anything is indistinguishable
//! from a working one at every call site, which is the failure mode this crate's gates exist to
//! make impossible — and which this crate nevertheless shipped once, in the record decoder that
//! L6 replaced. See `record.rs`'s header.
//!
//! **Nothing here runs at process start.** The control array is `.bss`-zero, which already means
//! "every target off"; no initialiser touches it, no clock is calibrated, no thread is spawned.
//! A process that never enables logging never writes a byte of this crate's state — `boot` records
//! the configuration and returns, and `enable` is what acts on it.

// The logger must never write to stdout: stdout belongs to the game, or to whatever tool is
// piping it. The console sink (a later rung) writes to stderr through its own handle. This is a
// permanent policy for this crate, not a scaffold, which is why `print_stderr` is NOT denied
// beside it -- denying something a later rung must do would only teach the next reader that the
// `deny` list is negotiable.
#![deny(clippy::print_stdout)]
#![deny(clippy::dbg_macro)]

pub mod census;
pub mod control;
pub mod preset;
pub mod codes;
pub mod drain_owner;
pub mod lane;
pub mod level;
pub mod lifecycle;
mod macros;
pub mod once_sites;
/// Test-only record observation for the crates that emit into this logger.
///
/// Behind the `test-probe` feature, which emitting crates enable in `[dev-dependencies]` only —
/// so it reaches their test targets and never a shipping build. See the module header for why a
/// `#[cfg(test)]` module could not have served, and for what an observer does and does not prove.
#[cfg(feature = "test-probe")]
pub mod probe;
pub mod rate;
pub mod record;
pub mod sample;
pub mod sink;
pub mod site;
pub mod sync_out;
pub mod target;

pub use codes::{
    CODE_IDX_EXHAUSTED, CodeStatus, DIAGNOSTICS, DiagInfo, ErrorCode, OnceSite, PanicCode,
    RatePolicy, WarnCode, explain,
};
pub use lane::{LANE_ARRAY_LEN, emit_impl, emit_impl_dyn};
pub use level::Level;
pub use record::{
    DspBuf, LogArgs, LogValue, MAX_RECORD_BYTES, MAX_STR_BYTES, ValueTag, flags, render_payload,
};
pub use site::{LogFormatter, LogSite};
pub use target::{
    DYN_BAND_LEN, DYN_BAND_START, ENGINE_BAND_END, LogTarget, MAX_TARGETS, TargetControl, TargetId,
    control_epoch, runtime_ceiling, set_target_control, set_target_level, target_control,
};

// The engine target table's marker types, re-exported at the root so a call site reads
// `boyko_log::info!(boyko_log::Ecs, …)` rather than naming the module.
pub use target::{
    App, Assets, ChangeDetect, Components, Ecs, Events, Fontbake, GpuColumns, Host, Image, Input,
    Log, MathSdf, Memory, Physics, Profiling, Query, Render, Rhi, RhiVulkan, Scene, Schedule,
    Serialize, ShaderDsl, Threadpool, Ui,
};

/// The compile-time severity ceiling for this build. **A site above it does not exist.**
///
/// Derived from `boyko_diag::profile::LOG_CEILING`, which is where the one `BOYKO_PROFILE` build
/// script in the workspace will put it at the joint rung. Two properties matter and both are
/// consequences of it being declared *here*:
///
/// - The emission macros write `$crate::GLOBAL_CEILING`, so the `env!` that will eventually
///   produce it is expanded in **this** crate — never in a caller crate, where no
///   `cargo:rerun-if-env-changed` directive exists and a profile change would therefore not
///   trigger a rebuild.
/// - It is a `const` mapped by a `const fn` from a `const`, so it folds at every call site and
///   gate (b) is decided by the compiler.
pub const GLOBAL_CEILING: Level = Level::from_raw(boyko_diag::profile::LOG_CEILING);

#[cfg(test)]
mod tests {
    use super::*;

    /// The default build must admit every level.
    ///
    /// Const-decidable, and kept anyway — the distinction that matters is not "can the compiler
    /// fold it" but "can the input change without the compiler objecting". Editing
    /// `boyko_diag::profile::LOG_CEILING` compiles cleanly and **silently deletes every `debug!`
    /// and `trace!` in the engine**, which is the correct outcome for a shipping profile and a
    /// catastrophic one for `dev`. This is the only pin on that value in the workspace: the
    /// substrate deliberately does not carry a second copy.
    ///
    /// **J1 obligation.** Once `boyko_diag/build.rs` makes the ceiling profile-dependent, this
    /// test must become profile-aware or be replaced by the build script's own input/output gate.
    /// Left as-is it would fail every non-`dev` CI leg for the right reason and the wrong cause.
    #[test]
    fn the_dev_profile_admits_every_level() {
        assert_eq!(GLOBAL_CEILING, Level::Trace);
    }
}
