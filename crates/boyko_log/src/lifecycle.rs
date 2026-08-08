//! The boot/enable split: what a process pays before anyone asks for a log.
//!
//! # `boot()` is a pure struct-fill. `enable()` does the work.
//!
//! The predecessor made `boot()` a no-op **only when the compile ceiling was `Off`** — so a `dev`
//! or `shipping` binary that nobody had asked for diagnostics still spawned a sink thread and
//! installed a process-global panic hook at start-up. That is the weaker half of the rule this
//! module implements:
//!
//! - **`boot(cfg)` spawns nothing, installs nothing, calibrates nothing, in ANY profile.** It
//!   records a configuration and moves a state byte. A process that boots and never enables has
//!   one extra `AtomicU8` written and nothing else.
//! - **`enable()` does all of it** — at launch, before the game loop, on the host thread, where a
//!   syscall and a calibration window are free of both hot-path and frame-time concerns.
//!
//! The distinction matters because it is the difference between *"diagnostics are off"* and
//! *"diagnostics are off and cost nothing"*. A flag that has to be read is still a flag; a thread
//! that was never spawned is genuinely absent.
//!
//! # What is here and what is not
//!
//! `enable()` currently turns on the synchronous destination and calibrates the clock. **It does
//! not spawn a sink thread or install a panic hook, because neither exists yet** — the drain loop
//! is the rest of this rung. That is stated rather than left implicit, because the no-boot-work
//! gate below can only assert what there is to assert: it proves the console destination and the
//! clock stay untouched across `boot()`, and it will gain the thread and hook legs in the same
//! commit that gives them something to be true about.
//!
//! **The OS-level probe is deferred, and the reason is not "later".** The specified form counts
//! this process's threads through `CreateToolhelp32Snapshot` on Windows and `/proc/self/task` on
//! Linux, **with its own control** — the same fixture spawns one deliberate thread and asserts the
//! count rises by exactly one, so a probe that always returns a constant reds before it can
//! certify anything. That control is the whole value, and it needs a thread to count. It lands
//! with the sink thread.

use core::sync::atomic::{AtomicU8, Ordering};

/// Where the logging subsystem is in its lifecycle.
#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SinkState {
    /// Nothing has been configured. **The `.bss`-zero state**, and therefore the state of every
    /// process that never calls [`boot`].
    NotBooted = 0,
    /// A configuration has been recorded. Still no thread, no hook, no destination.
    Booted = 1,
    /// Diagnostics are on.
    Enabled = 2,
    /// A shutdown is in progress; emission is refused and the remaining records are drained.
    Exiting = 3,
    /// Shutdown completed.
    Exited = 4,
}

impl SinkState {
    const fn from_raw(raw: u8) -> SinkState {
        match raw {
            0 => SinkState::NotBooted,
            1 => SinkState::Booted,
            2 => SinkState::Enabled,
            3 => SinkState::Exiting,
            _ => SinkState::Exited,
        }
    }
}

// `NotBooted` must be zero: it is what makes an un-booted process correct without an initialiser,
// and it is the same argument `Level::Off == 0` makes for the control array.
const _: () = assert!(SinkState::NotBooted as u8 == 0);

static SINK_STATE: AtomicU8 = AtomicU8::new(SinkState::NotBooted as u8);

/// Whether a console destination should exist once diagnostics are enabled.
///
/// Recorded by [`boot`] and acted on by [`enable`] — the split is the point. `.bss`-false, so an
/// un-booted process has recorded nothing.
static WANT_CONSOLE: AtomicU8 = AtomicU8::new(0);

/// What a host asks for at boot.
///
/// Deliberately plain data with no handles: a configuration that owns a file or a thread cannot be
/// recorded without doing the work, which is exactly what [`boot`] must not do.
#[derive(Clone, Copy, Debug, Default)]
pub struct LogConfig {
    /// Write synchronous lines to the console. `false` by default, which is the shipped default.
    pub console: bool,
}

/// The current lifecycle state.
#[inline]
#[must_use]
pub fn state() -> SinkState {
    SinkState::from_raw(SINK_STATE.load(Ordering::Acquire))
}

/// Record a configuration. **Spawns nothing, installs nothing, calibrates nothing.**
///
/// Two atomic stores and a return. Everything a host might expect this to do — the thread, the
/// hook, the destination, the clock — happens in [`enable`].
pub fn boot(cfg: LogConfig) {
    WANT_CONSOLE.store(u8::from(cfg.console), Ordering::Relaxed);
    SINK_STATE.store(SinkState::Booted as u8, Ordering::Release);
}

/// Turn diagnostics on: open the destinations and calibrate the clock.
///
/// Runs at launch, before the game loop, on the host thread. Idempotent — a second call is a
/// no-op, because a launch flag parsed twice must not calibrate twice.
///
/// Returns `false` when there is nothing to enable, which is the case a host must be able to
/// distinguish from success: [`boot`] was never called, or a shutdown has already begun.
pub fn enable() -> bool {
    let cur = state();
    if cur == SinkState::Enabled {
        return true;
    }
    if cur != SinkState::Booted {
        return false;
    }
    if WANT_CONSOLE.load(Ordering::Relaxed) != 0 {
        crate::sync_out::set_console_enabled(true);
    }
    // The 20 ms calibration window lives here and nowhere else. Doing it at boot would make every
    // process pay it, including one that never asks for a log.
    boyko_diag::clock::calibrate();
    SINK_STATE.store(SinkState::Enabled as u8, Ordering::Release);
    true
}

/// Turn diagnostics off again for a session that no longer wants them.
///
/// Closes the destinations. It does **not** reclaim `.bss` — `.bss` is never freed, and a design
/// that pretended otherwise would be claiming a saving it cannot deliver.
pub fn disable() {
    crate::sync_out::set_console_enabled(false);
    SINK_STATE.store(SinkState::Booted as u8, Ordering::Release);
}

#[cfg(test)]
mod tests {
    use super::*;

    // The lifecycle is process-global, so these take it in turn. Letting them race would make a
    // "boot enabled nothing" assertion pass or fail on which test got there first.
    #[allow(clippy::disallowed_types)]
    static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn reset() {
        crate::sync_out::set_console_enabled(false);
        SINK_STATE.store(SinkState::NotBooted as u8, Ordering::Release);
        WANT_CONSOLE.store(0, Ordering::Relaxed);
    }

    #[test]
    fn an_unbooted_process_is_in_the_bss_zero_state() {
        let _s = SERIAL.lock().expect("serial");
        reset();
        assert_eq!(state(), SinkState::NotBooted);
        assert_eq!(SinkState::NotBooted as u8, 0, "the zero state must be the un-booted one");
    }

    #[test]
    fn boot_records_the_wish_and_opens_nothing() {
        // THE no-boot-work property, in the half this rung can assert. Moving the destination
        // open from `enable` back into `boot` reds here -- which is the same edit that, once the
        // sink thread exists, would also make a flag-off run grow a thread.
        let _s = SERIAL.lock().expect("serial");
        reset();
        boot(LogConfig { console: true });
        assert_eq!(state(), SinkState::Booted);
        assert_eq!(
            crate::sync_out::write_oracle_line("boyko: ", "must not be written"),
            None,
            "boot() must not open a destination, even one the config asked for"
        );
        reset();
    }

    #[test]
    fn enable_opens_what_boot_only_recorded() {
        // The other side of the same property: without this, "boot opens nothing" is satisfied by
        // an enable that also opens nothing.
        let _s = SERIAL.lock().expect("serial");
        reset();
        boot(LogConfig { console: true });
        assert!(enable());
        assert_eq!(state(), SinkState::Enabled);
        assert!(
            crate::sync_out::write_oracle_line("boyko-test: ", "enabled").is_some(),
            "enable() must open the destination boot() recorded"
        );
        disable();
        reset();
    }

    #[test]
    fn a_config_that_asked_for_nothing_opens_nothing_even_when_enabled() {
        // The shipped default. `console: false` is not "enable it quietly"; it is "there is no
        // synchronous destination", which is what makes the fallback paths inert rather than
        // writing to a stream nothing collects.
        let _s = SERIAL.lock().expect("serial");
        reset();
        boot(LogConfig::default());
        assert!(enable());
        assert_eq!(crate::sync_out::write_oracle_line("boyko: ", "nowhere"), None);
        reset();
    }

    #[test]
    fn enable_is_idempotent_and_refuses_when_there_is_nothing_to_enable() {
        let _s = SERIAL.lock().expect("serial");
        reset();
        assert!(!enable(), "enable before boot must refuse rather than half-initialise");
        boot(LogConfig { console: true });
        assert!(enable());
        assert!(enable(), "a launch flag parsed twice must not calibrate twice");
        disable();
        assert_eq!(state(), SinkState::Booted, "disable returns to Booted, not to NotBooted");
        reset();
    }
}
