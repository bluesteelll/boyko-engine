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
//! **Rung L0.** What exists: [`Level`], [`LogTarget`], [`TargetId`], [`TargetControl`], the engine
//! target table, the control array and its epoch counter, and the five macros with their full
//! three-gate expansion. What does **not** exist yet: the record encoding, the lanes, the sink,
//! and therefore any output at all. The enabled arm of every macro calls
//! [`__l0_no_sink_yet`], which evaluates the arguments and drops them.
//!
//! **Nothing here runs at process start.** The control array is `.bss`-zero, which already means
//! "every target off"; no initialiser touches it, no clock is calibrated, no thread is spawned.
//! A process that never enables logging never writes a byte of this crate's state.

// The logger must never write to stdout: stdout belongs to the game, or to whatever tool is
// piping it. The console sink (a later rung) writes to stderr through its own handle. This is a
// permanent policy for this crate, not a scaffold, which is why `print_stderr` is NOT denied
// beside it -- denying something a later rung must do would only teach the next reader that the
// `deny` list is negotiable.
#![deny(clippy::print_stdout)]
#![deny(clippy::dbg_macro)]

pub mod level;
mod macros;
pub mod target;

pub use level::Level;
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

/// Rung L0's stand-in for the emission path: evaluates the arguments, then drops them.
///
/// **This is not an API.** It exists so the macros' enabled arm has a body while there is no
/// record encoding and no sink, and so the side-effect gates can observe that arguments *are*
/// evaluated when the chain passes and are *not* when it does not. Rung L1 replaces every call
/// with `emit_impl`, and this function is deleted in the same commit.
///
/// The name is deliberately unpleasant. A stub that reads like a finished call is a stub that
/// ships.
#[doc(hidden)]
#[inline]
pub fn __l0_no_sink_yet<A>(_fmt: &'static str, _args: A) {}

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
