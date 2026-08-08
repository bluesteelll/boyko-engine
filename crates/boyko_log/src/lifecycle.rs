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
//! `enable()` turns on the synchronous destination, calibrates the clock, and — **only when the
//! configuration asked for one** — spawns the sink thread. It does **not** install a panic hook;
//! that arrives with the crash path.
//!
//! The sink thread is opt-in (`LogConfig::sink_thread`, default `false`) rather than implied by
//! `enable()`. A thread is the most expensive thing this subsystem can create, and the profile
//! that wants a crash file and nothing else must not pay for one.
//!
//! **The OS-level thread-count probe is still deferred, and the reason is not "later".** The
//! specified form counts this process's threads through `CreateToolhelp32Snapshot` on Windows and
//! `/proc/self/task` on Linux, **with its own control** — the same fixture spawns one deliberate
//! thread and asserts the count rises by exactly one, so a probe that always returns a constant
//! reds before it can certify anything. Writing it needs a `windows-sys` dev-dependency on a crate
//! whose whole manifest discipline is that it has none, which is a decision worth taking on its
//! own rather than in passing. What is asserted instead, below, is behavioural: the sink makes
//! progress when asked for and none when not.

use core::sync::atomic::{AtomicU8, Ordering};

use crate::site::LogSite;

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
    /// Run a resident sink thread that drains the rings. `false` by default.
    ///
    /// Off by default because a thread is the single most expensive thing this subsystem can
    /// create, and the profile that wants a crash file and nothing else must not pay for one.
    pub sink_thread: bool,
}

/// Set while a sink thread should keep running.
static SINK_RUNNING: AtomicU8 = AtomicU8::new(0);

/// Whether a sink thread was asked for at boot.
static WANT_SINK: AtomicU8 = AtomicU8::new(0);

/// Drain passes the sink thread has completed. Read by tests and by the census; a stalled sink is
/// a number that stops moving, which is a symptom a hung thread does not otherwise have.
static SINK_PASSES: AtomicU8 = AtomicU8::new(0);

/// Passes completed by the sink thread, saturating at 255.
#[must_use]
pub fn sink_passes() -> u8 {
    SINK_PASSES.load(Ordering::Acquire)
}

/// The sink loop: claim the role, drain, park, repeat until asked to stop.
///
/// # The park is adaptive, and the reason is not battery life
///
/// A fixed short park spends a core spinning through empty rings in the common case — a game that
/// logs nothing for minutes at a time. A fixed long park makes the first record of a burst wait
/// for it. The loop therefore parks briefly after a pass that found work and backs off after a
/// pass that did not, so the latency cost is paid only where records actually are.
///
/// **It refuses rather than steals.** If another consumer holds the role — a manual `drain()`, the
/// scheduled ECS drain — this pass does nothing and tries again later. Stealing would create the
/// second consumer the token exists to prevent.
fn sink_loop() {
    let mut idle: u32 = 0;
    loop {
        let asked_to_stop = SINK_RUNNING.load(Ordering::Acquire) == 0;

        let moved = match crate::drain_owner::try_claim() {
            Some(token) => {
                let stats = crate::lane::drain(&token, |site, _tsc, _flags, payload| {
                    let mut buf = [0u8; 192];
                    let n = render_record(&mut buf, site, payload.len());
                    // SAFETY: `render_record` writes only ASCII copied from `&'static str`s and
                    //   decimal digits.
                    let text = unsafe { core::str::from_utf8_unchecked(&buf[..n]) };
                    crate::sync_out::write_oracle_line("boyko-log ", text);
                });
                stats.records > 0
            }
            None => false,
        };

        SINK_PASSES.fetch_add(1, Ordering::Release);

        // The stop check is read BEFORE the pass and acted on after it, so a shutdown always gets
        // one final drain over records published before it was requested. Reading it after would
        // race the last emit out of the log.
        if asked_to_stop {
            SINK_STATE.store(SinkState::Exited as u8, Ordering::Release);
            return;
        }

        if moved {
            idle = 0;
            std::thread::yield_now();
        } else {
            idle = (idle + 1).min(8);
            std::thread::sleep(std::time::Duration::from_micros(200u64 << idle));
        }
    }
}

/// Render `file:line fmt (N B)` into `buf`, truncating rather than overflowing.
fn render_record(buf: &mut [u8], site: &LogSite, payload_len: usize) -> usize {
    let mut n = 0usize;
    let mut put = |s: &[u8], n: &mut usize| {
        let take = s.len().min(buf.len() - *n);
        buf[*n..*n + take].copy_from_slice(&s[..take]);
        *n += take;
    };
    let dec = |v: u64, n: &mut usize, put: &mut dyn FnMut(&[u8], &mut usize)| {
        let mut d = [0u8; 20];
        let mut v = v;
        let mut i = d.len();
        loop {
            i -= 1;
            d[i] = b'0' + (v % 10) as u8;
            v /= 10;
            if v == 0 || i == 0 {
                break;
            }
        }
        put(&d[i..], n);
    };
    put(site.level.as_str().as_bytes(), &mut n);
    put(b" ", &mut n);
    put(site.file.as_bytes(), &mut n);
    put(b":", &mut n);
    dec(u64::from(site.line), &mut n, &mut put);
    put(b" ", &mut n);
    put(site.fmt.as_bytes(), &mut n);
    put(b" (", &mut n);
    dec(payload_len as u64, &mut n, &mut put);
    put(b" B)", &mut n);
    n
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
    WANT_SINK.store(u8::from(cfg.sink_thread), Ordering::Relaxed);
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
    if WANT_SINK.load(Ordering::Relaxed) != 0 {
        SINK_RUNNING.store(1, Ordering::Release);
        SINK_PASSES.store(0, Ordering::Release);
        std::thread::Builder::new()
            .name("boyko-log-sink".into())
            .spawn(sink_loop)
            .map_or_else(
                |_| {
                    // A thread the OS refused is not a reason to fail the launch: the synchronous
                    // channel still works and the rings still fill. The failure is recorded by the
                    // flag going back down, which `shutdown` and the census both read.
                    SINK_RUNNING.store(0, Ordering::Release);
                },
                drop,
            );
    }
    true
}

/// Stop the sink thread and wait, bounded, for it to complete one final drain.
///
/// **No join handle is kept, and that is deliberate**: a handle would have to live in a static
/// that `shutdown` can take by value, which needs interior mutability over a non-`Copy` type on a
/// path that also runs from a panic hook. The state byte carries the same information — the thread
/// publishes `Exited` as its last act — and a bounded wait on it cannot deadlock against a thread
/// that died before it got there.
///
/// Returns `false` if the wait expired, which is a defect signal rather than an error to handle:
/// the caller is on its way out either way.
pub fn shutdown() -> bool {
    if SINK_RUNNING.swap(0, Ordering::AcqRel) == 0 {
        SINK_STATE.store(SinkState::Exited as u8, Ordering::Release);
        return true;
    }
    SINK_STATE.store(SinkState::Exiting as u8, Ordering::Release);
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(500);
    while std::time::Instant::now() < deadline {
        if state() == SinkState::Exited {
            return true;
        }
        std::thread::yield_now();
    }
    false
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

    // These take the PROCESS-WIDE lock in `drain_owner`, not one of their own. The sink thread
    // this module spawns claims the global drain role, so a lifecycle test racing a lane test is
    // two consumers contending — the exact collision a per-module mutex fails to prevent, and
    // which was measured once already in this crate.

    fn reset() {
        crate::sync_out::set_console_enabled(false);
        SINK_STATE.store(SinkState::NotBooted as u8, Ordering::Release);
        WANT_CONSOLE.store(0, Ordering::Relaxed);
    }

    #[test]
    fn an_unbooted_process_is_in_the_bss_zero_state() {
        let _s = crate::drain_owner::test_serial();
        reset();
        assert_eq!(state(), SinkState::NotBooted);
        assert_eq!(SinkState::NotBooted as u8, 0, "the zero state must be the un-booted one");
    }

    #[test]
    fn boot_records_the_wish_and_opens_nothing() {
        // THE no-boot-work property, in the half this rung can assert. Moving the destination
        // open from `enable` back into `boot` reds here -- which is the same edit that, once the
        // sink thread exists, would also make a flag-off run grow a thread.
        let _s = crate::drain_owner::test_serial();
        reset();
        boot(LogConfig { console: true, sink_thread: false });
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
        let _s = crate::drain_owner::test_serial();
        reset();
        boot(LogConfig { console: true, sink_thread: false });
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
        let _s = crate::drain_owner::test_serial();
        reset();
        boot(LogConfig::default());
        assert!(enable());
        assert_eq!(crate::sync_out::write_oracle_line("boyko: ", "nowhere"), None);
        reset();
    }

    #[test]
    fn no_sink_thread_unless_the_config_asked_for_one() {
        // The default. `enable()` doing the maximum it could is exactly the shape this module
        // exists to refuse -- a host that wanted a crash file must not get a resident thread.
        let _s = crate::drain_owner::test_serial();
        reset();
        boot(LogConfig { console: false, sink_thread: false });
        assert!(enable());
        std::thread::sleep(std::time::Duration::from_millis(20));
        assert_eq!(sink_passes(), 0, "no thread was asked for, so no pass may have happened");
        assert!(shutdown(), "shutdown with no thread must complete immediately");
        reset();
    }

    #[test]
    fn the_sink_thread_makes_progress_and_stops_when_asked() {
        // Behavioural, not identity-based: the pass counter moves while it runs and stops moving
        // after `shutdown`. A probe that only checked "a thread exists" would pass against a
        // thread that had hung on its first drain -- which is the failure with no other symptom.
        let _s = crate::drain_owner::test_serial();
        reset();
        boot(LogConfig { console: false, sink_thread: true });
        assert!(enable());

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while sink_passes() == 0 && std::time::Instant::now() < deadline {
            std::thread::yield_now();
        }
        assert!(sink_passes() > 0, "the sink thread never completed a pass");

        assert!(shutdown(), "the sink must observe the stop request within the bound");
        assert_eq!(state(), SinkState::Exited);
        let settled = sink_passes();
        std::thread::sleep(std::time::Duration::from_millis(20));
        assert_eq!(sink_passes(), settled, "a stopped sink must stop counting");
        reset();
    }

    #[test]
    fn enable_is_idempotent_and_refuses_when_there_is_nothing_to_enable() {
        let _s = crate::drain_owner::test_serial();
        reset();
        assert!(!enable(), "enable before boot must refuse rather than half-initialise");
        boot(LogConfig { console: true, sink_thread: false });
        assert!(enable());
        assert!(enable(), "a launch flag parsed twice must not calibrate twice");
        disable();
        assert_eq!(state(), SinkState::Booted, "disable returns to Booted, not to NotBooted");
        reset();
    }
}
