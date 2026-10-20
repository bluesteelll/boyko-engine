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

use core::fmt::Write as _;
use core::sync::atomic::{AtomicU8, AtomicU32, AtomicU64, Ordering};

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
    /// Feed [`ECS_HANDOFF`](crate::sink::ecs) so an ECS reader can see the log in-frame.
    ///
    /// `false` by default. A title that never displays its own log pays neither the copy nor the
    /// `LogRing` columns, and the ring's `.bss` extent stays untouched — reserved address space
    /// rather than resident memory.
    pub ecs_ring: bool,
    /// Open the destination recorded by [`crate::sink::file::set_path`] at [`enable`].
    ///
    /// A `bool` rather than the path itself, so this struct stays plain `Copy` data with no
    /// handles and no borrowed lifetime — a configuration that owned a path would be one `boot`
    /// could not record without allocating.
    pub file: bool,
    /// Open the destination recorded by [`crate::sink::binary::set_path`] at [`enable`].
    ///
    /// # It had no production route until this field existed
    ///
    /// The binary sink shipped with a format, a dictionary, a destination and — a rung later — an
    /// offline decoder. Nothing on the enable path opened it: the only routes were the request
    /// ring and a direct call from a test. So a shipped configuration could not produce a `.blog`
    /// at all, which makes every gate below it a gate on a file no process writes.
    ///
    /// Separate from [`file`](Self::file) rather than an enum, because a process may legitimately
    /// want both: a text file for a human tailing it and a binary one for the upload.
    pub binary: bool,
    /// Byte cap for the file sink; `0` means uncapped. Reaching it emits `boyko-W0103` once.
    pub file_cap_bytes: u64,
    /// Who drains the rings. See [`SinkMode`].
    pub sink_mode: SinkMode,
}

/// Who holds the consumer role.
///
/// Three variants, because two were not enough. v3 gave the smallest shipping profile `Manual` and
/// a crash sink — and then **nothing drained**: `flush()` returns `NoConsumer` immediately,
/// admission control drops new records rather than overrunning oldest, and within seconds the
/// lanes hold nothing but boot-time records while everything up to the crash is refused. The
/// profile whose only product is a crash log was structurally guaranteed **not to contain the
/// crash**.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum SinkMode {
    /// A resident sink thread drains, parks adaptively, and fans out. The default.
    #[default]
    Thread,
    /// Nothing drains until a host calls [`drain`]. For hermetic tests, CLI tools and the
    /// zero-allocation gate — **not** for a shipping profile, for the reason above.
    Manual,
    /// The ECS drains once per frame on the frame thread — `log_drain_system`'s first duty
    /// (`boyko_ecs::ecs::core::log`), keyed on [`sink_mode`].
    ///
    /// The previous doc said the participation "lands with rung L15". It did not: the mode was
    /// recorded by [`boot`] and read back by NOTHING until a live `shipping-min` host was measured
    /// delivering zero records — the dead-datum class, found by
    /// `boyko_app/tests/log_host_shipping_min.rs` after both ladders were closed.
    Scheduled,
}

/// What a manual [`drain`] did.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DrainResult {
    /// The pass ran. Carries what it moved.
    Ran(crate::lane::DrainStats),
    /// Another consumer holds the role. **Not** a `debug_assert`: a second manual caller is a user
    /// error, not a bug in this crate, and a crate that panicked on it would turn a recoverable
    /// misuse into a crash.
    Busy,
}

/// Recorded by [`boot`], read by the consumer role on every drain.
///
/// A plain `.bss` byte rather than a field threaded through the drain, because the drain's other
/// two callers — a manual `drain()` and the crash drainer — are not on this path and must observe
/// the same answer.
static WANT_ECS_RING: AtomicU8 = AtomicU8::new(0);

/// Boot from a [`LogRuntimePreset`](crate::preset::LogRuntimePreset), applying its WHOLE row.
///
/// # Why a function and not "call `boot(preset.config())`"
///
/// Because `config()` is only part of the row. The preset table also says whether the file sink
/// rotates and which destinations exist, and neither is in `LogConfig` -- `rotates()`'s own doc
/// warned that a preset claiming to rotate without calling `set_rotation` would be "a table that
/// describes a behaviour nobody implements". It was exactly that, because nothing called either.
///
/// This applies all of it in one place: paths, rotation, sinks, and the boot itself. What it
/// cannot invent is the paths, so they are arguments.
///
/// `text` and `binary` are `Option`s rather than a single path, because the two sinks have their
/// own destinations on purpose -- a process may want a text file for a human tailing it and a
/// binary one for the upload, and one path setter serving both would make "which file did that go
/// to" unanswerable.
pub fn boot_preset(preset: crate::preset::LogRuntimePreset, text: Option<&str>, binary: Option<&str>) {
    let cfg = preset.config();
    if cfg.file && let Some(path) = text {
        crate::sink::file::set_path(path);
    }
    if cfg.binary && let Some(path) = binary {
        crate::sink::binary::set_path(path);
    }
    if preset.rotates() {
        // BOTH sinks, and the first draft did only the text one -- which left `Shipping` (the row
        // whose destination IS the binary file) claiming rotation in the table and never rotating.
        // A preset's `rotates()` is a claim about the preset's destination, not about one sink.
        crate::sink::file::set_rotation(crate::preset::ROTATE_AT_BYTES, crate::preset::ROTATE_KEEP);
        crate::sink::binary::set_rotation(
            crate::preset::ROTATE_AT_BYTES,
            crate::preset::ROTATE_KEEP,
        );
    }
    BOOT_PRESET.store(preset as u8 + 1, Ordering::Relaxed);
    // A PRESET ARMS THE ENGINE'S TARGETS, and without this it configures sinks nothing reaches.
    //
    // `CONTROL` is `.bss`-zero, which is `Level::Off` for every target: a process that boots a
    // preset and enables it would open a file, write a header into a ring, and drop every record
    // at gate (c). MEASURED -- the first draft of this function did not arm, and the session
    // header did not reach the `.blog` the preset had just opened.
    //
    // Armed at the COMPILE ceiling, not above it: the runtime axis cannot exceed the compile one,
    // and a preset that tried would be claiming a level the binary cannot emit. A host wanting
    // something narrower calls `set_target_control` after this returns.
    if preset != crate::preset::LogRuntimePreset::Off {
        for (id, _) in crate::target::engine_targets() {
            crate::target::set_target_control(
                id,
                crate::target::TargetControl::new(crate::GLOBAL_CEILING, 0, false),
            );
        }
    }
    boot(cfg);
}

/// The preset [`boot_preset`] recorded, or `None` for a hand-built [`LogConfig`].
#[must_use]
pub fn boot_preset_recorded() -> Option<crate::preset::LogRuntimePreset> {
    crate::preset::LogRuntimePreset::from_raw(BOOT_PRESET.load(Ordering::Relaxed))
}

/// Whether the consumer role should feed [`ECS_HANDOFF`](crate::sink::ecs).
#[inline]
#[must_use]
pub fn ecs_ring_enabled() -> bool {
    WANT_ECS_RING.load(Ordering::Relaxed) != 0
}

/// Recorded by [`boot`], acted on by [`enable`].
static WANT_FILE: AtomicU8 = AtomicU8::new(0);

/// Recorded by [`boot`], acted on by [`enable`]. See [`LogConfig::binary`].
static WANT_BINARY: AtomicU8 = AtomicU8::new(0);

/// The preset [`boot_preset`] recorded, plus one, or `0` for a hand-built [`LogConfig`].
///
/// Plus one so the `.bss`-zero state means "no preset", which is the honest answer for a host that
/// built its own config: printing `runtime_preset=dev` for it would name an axis the host never
/// selected.
static BOOT_PRESET: AtomicU8 = AtomicU8::new(0);

/// The file sink's byte cap, recorded by [`boot`].
static FILE_CAP: AtomicU64 = AtomicU64::new(0);

/// The recorded [`SinkMode`], as its discriminant.
static SINK_MODE: AtomicU8 = AtomicU8::new(0);

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

/// Set by the sink thread's unwind: it died without publishing `Exited`.
static SINK_DIED: AtomicU8 = AtomicU8::new(0);

/// Records a sink thread that unwinds, so an expired [`shutdown`] can name it
/// ([`ShutdownOutcome::SinkDied`]) instead of reading a dead thread as a stalled one.
struct SinkDeathWatch;

impl Drop for SinkDeathWatch {
    fn drop(&mut self) {
        if std::thread::panicking() {
            // Release: the unwind has already dropped the drain token (a later local of
            // `sink_loop`), so a shutdown that reads this with `Acquire` finds the role free.
            SINK_DIED.store(1, Ordering::Release);
        }
    }
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
///
/// # The console is written last, and outside the role
///
/// The console is the one destination with an EXTERNAL consumer: a pipe a parent process may stop
/// reading. The pass therefore stages its console lines and writes them only after the durable
/// sinks hold the pass and the role is released. A consumer that stalls then blocks this thread
/// in that write and nothing else: no durable write waits behind it, and a [`shutdown`] whose join
/// expires can claim the role and close the sinks itself. MEASURED in master's push CI: with the
/// console first and under the role, a one-second pause of the runner's pipe during the census
/// cost the durable file every census row after the stalled line.
fn sink_loop() {
    let _death = SinkDeathWatch;
    // The thread's own stack, for the thread's whole life: no static, no sharing.
    let mut stage = ConsoleStage::new();
    let mut idle: u32 = 0;
    loop {
        let asked_to_stop = SINK_RUNNING.load(Ordering::Acquire) == 0;

        let moved = match crate::drain_owner::try_claim() {
            Some(token) => {
                let stats = deliver(&token, Console::Stage(&mut stage));
                drop(token);
                stats.records > 0
            }
            None => false,
        };
        stage.write_out();

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

/// Claim the consumer role, drain every lane once, and route what it finds.
///
/// Returns `None` when another consumer already holds the role — a refusal, never a steal, because
/// stealing would create the second consumer the token exists to prevent.
///
/// # Why this is a function rather than the sink loop's body
///
/// Three callers need exactly this pass and must not each grow their own: the resident sink
/// thread, a host draining by hand, and — at L15 — `SinkMode::Scheduled`'s in-frame drain. A pass
/// that differed between them would make "was the ECS ring fed" depend on which of the three ran,
/// which is precisely the question the seam's one-frame bound is an answer to.
pub fn drain_once() -> Option<crate::lane::DrainStats> {
    let token = crate::drain_owner::try_claim()?;
    Some(deliver(&token, Console::Direct))
}

/// How a pass delivers its console lines.
enum Console<'a> {
    /// Each line straight to the synchronous channel, in record order -- [`drain_once`]'s callers.
    Direct,
    /// Into the sink thread's stage, written once the pass has released the role.
    Stage(&'a mut ConsoleStage),
    /// Not at all, counted in [`console_lines_skipped`]: a late close, whose console is the
    /// consumer that stalled.
    Skip,
}

/// Rendered console text one sink pass stages before writing it.
///
/// 16 KiB is the census (about 3.4 KiB) four times over; a pass that renders more than that writes
/// the stage under the role and carries on (`ConsoleStage::overflow`), which is the pre-staging
/// behaviour for that pass.
const CONSOLE_STAGE_BYTES: usize = 16 * 1024;

/// The line prefix every ring-borne console line carries.
const CONSOLE_PREFIX: &str = "boyko-log ";

// One rendered line (prefix, body, newline) always fits an empty stage, so an overflow only ever has
// to write the stage.
const _: () = assert!(CONSOLE_PREFIX.len() + crate::record::MAX_RENDERED_BYTES < CONSOLE_STAGE_BYTES);

/// The sink thread's console lines for one pass, on that thread's stack.
struct ConsoleStage {
    buf: [u8; CONSOLE_STAGE_BYTES],
    len: usize,
}

impl ConsoleStage {
    const fn new() -> ConsoleStage {
        ConsoleStage { buf: [0; CONSOLE_STAGE_BYTES], len: 0 }
    }

    /// Stage `prefix body \n`: the bytes [`crate::sync_out::write_oracle_line`] would write.
    fn push(&mut self, prefix: &str, body: &str) {
        let need = prefix.len() + body.len() + 1;
        if self.len + need > CONSOLE_STAGE_BYTES {
            self.overflow();
        }
        let start = self.len;
        let mid = start + prefix.len();
        let end = mid + body.len();
        self.buf[start..mid].copy_from_slice(prefix.as_bytes());
        self.buf[mid..end].copy_from_slice(body.as_bytes());
        self.buf[end] = b'\n';
        self.len = end + 1;
    }

    /// A pass rendered more console text than the stage holds: write what is staged now, under the
    /// role -- the third way [`ShutdownOutcome::StalledInsidePass`] can arise.
    #[cold]
    #[inline(never)]
    fn overflow(&mut self) {
        OVERFLOW_WRITE_IN_FLIGHT.store(true, Ordering::Release);
        self.write_out();
        OVERFLOW_WRITE_IN_FLIGHT.store(false, Ordering::Release);
    }

    /// Write and empty the stage. Called outside the role, after every pass.
    fn write_out(&mut self) {
        if self.len != 0 {
            let _ = crate::sync_out::write_oracle_block(&self.buf[..self.len]);
            self.len = 0;
        }
    }
}

/// Set while an overflowing pass writes its stage under the role. [`report_stalled_exit`] reads
/// it: when it is set, the stalled consumer IS the console, and a report written there would wait
/// behind the very write it is about.
static OVERFLOW_WRITE_IN_FLIGHT: core::sync::atomic::AtomicBool =
    core::sync::atomic::AtomicBool::new(false);

/// One pass over every lane, under a role the caller already holds, routed per sink.
///
/// Per record the order is file, binary, ECS, and the console LAST: each sink's own byte stream is
/// what it always was, and only the order across sinks changed -- so whatever happens to the
/// console's consumer, every durable sink already holds the record.
fn deliver(token: &crate::drain_owner::DrainToken, mut console: Console<'_>) -> crate::lane::DrainStats {
    // Sink lifecycle runs HERE, on the draining thread, before any record moves (L14). The
    // requesting thread posted a byte and returned; the `open` -- a syscall -- happens under the
    // token, which is the process-wide proof that nobody else is writing into the sink being
    // opened or closed.
    //
    // Before the records and not after: a file opened by this pass must receive this pass's
    // output. Starting one pass late is what an operator typing `open` and finding an empty file
    // would report as a bug.
    crate::sink::request::pump(token, FILE_CAP.load(Ordering::Relaxed));
    let to_ecs = ecs_ring_enabled();
    // Snapshotted so the report below carries THIS PASS's refusals. A report of the running total
    // would restate the same thousands every pass, and a reader could not tell a storm that has
    // stopped from one that has not.
    let (lost_before, bytes_before) = crate::sink::ecs::lost();
    // ONE buffer for the whole pass, hoisted out of the per-record closure: `MAX_RENDERED_BYTES`
    // of stack zeroed once instead of once per record.
    let mut line = crate::record::DspBuf::<{ crate::record::MAX_RENDERED_BYTES }>::new();
    let stats = crate::lane::drain(token, |site, _tsc, flags, payload| {
        // A dynamic record carries its target ahead of its values, because a `dyn_*!` site has
        // none at compile time (L10-B). The site's own `target` field is the discriminant, so the
        // split is done once here and everything below reads the resolved pair.
        let (target, payload) = crate::record::split_dynamic_target(site.target, payload);
        // The `Once` register, BEFORE the per-sink filters: a latch is spent when the site emits,
        // whatever a sink then does with the record. Counting delivered records instead would read
        // `fired=0` for every site in a process whose sinks are all off -- the exact silence
        // `W0111` exists to refuse, reproduced inside the mechanism meant to expose it.
        //
        // A no-op for the 29 `Every` rows: the branch is on cold `'static` data this thread
        // already holds, and the emitting thread never sees any of it (L8a-once).
        crate::once_sites::note(site);
        line.clear();
        render_record(&mut line, site, payload);
        let text = line.as_str();
        // Per-sink policy, read ONCE per record and applied per destination (L14). A file
        // capturing everything while a console shows only warnings is the ordinary case, and
        // without a per-sink floor it takes two target ceilings that fight each other.
        //
        // A record with no resolvable target is admitted on state and floor alone: filtering on a
        // target you do not have is filtering on a GUESS, and this record already survived the
        // decision two comments below that losing the message is strictly worse than losing the
        // attribution.
        let admits = |slot: usize| match target {
            Some(t) => crate::sink::slot::accepts(slot, t, site.level),
            None => {
                crate::sink::slot::state(slot) == crate::sink::slot::SinkState::Active
                    && site.level <= crate::sink::slot::floor(slot)
            }
        };
        if admits(crate::sink::slot::SLOT_FILE) {
            crate::sink::file::write_line(token, text.as_bytes());
        }
        // The BINARY sink takes the record BEFORE rendering would matter to it: it writes the
        // payload verbatim and names the site by a dictionary id, which is the entire reason the
        // format exists. It is fed the same `_tsc` and `flags` the ring carried, not a re-read
        // clock -- a second clock read here would put a different time in the two files for one
        // record, and a reader correlating them would find a skew that never happened.
        if admits(crate::sink::slot::SLOT_BINARY) {
            crate::sink::binary::write_record(token, site, _tsc, flags, payload);
        }
        // Counted once per record, HERE and not per destination: `delivered` answers "did this
        // target ever produce anything", and a record that reached two sinks did not happen twice.
        //
        // `None` is a record whose prefix did not survive the ring. It still reaches every sink --
        // losing the message as well as the attribution would be strictly worse -- but it is not
        // charged to a target, because charging it to a guess is how a census starts lying.
        if let Some(t) = target {
            crate::target::count_delivered(t);
        }
        // `to_ecs && target.is_some()`, and the second half is NOT a tidy-up. `FrameMeta.target`
        // is a `u8` over a 256-id space, so EVERY value is a real target and there is no sentinel
        // to spend on "unattributable" -- the same reason `TargetId::INVALID` does not exist. An
        // unattributable record therefore skips the in-frame view, having already reached every
        // byte sink above, rather than being filed under whichever target `255` happens to name.
        if to_ecs && admits(crate::sink::slot::SLOT_ECS) && let Some(t) = target {
            // The byte channel above already has the record, so a refusal here shortens the
            // in-frame view and nothing else. That is why the return value is dropped rather than
            // escalated: `push` has already counted it, on both the ring and the substrate's row.
            let meta = crate::sink::ecs::FrameMeta {
                level: site.level,
                target: t.index() as u8,
                code: site.code,
                flags,
            };
            crate::sink::ecs::push(token, meta, text.as_bytes());
        }
        // THE CONSOLE LAST (see `sink_loop`): every durable sink above already holds this record.
        // The route is matched here and nowhere else, behind the slot test, so a console-off
        // preset never reaches the match.
        if admits(crate::sink::slot::SLOT_CONSOLE) {
            match &mut console {
                Console::Direct => {
                    crate::sync_out::write_oracle_line(CONSOLE_PREFIX, text);
                }
                Console::Stage(stage) => stage.push(CONSOLE_PREFIX, text),
                Console::Skip => {
                    CONSOLE_SKIPPED.fetch_add(1, Ordering::Relaxed);
                }
            }
        }
    });
    let (lost_after, bytes_after) = crate::sink::ecs::lost();
    if lost_after > lost_before {
        crate::sink::ecs::report_overflow(lost_after - lost_before, bytes_after - bytes_before);
    }
    stats
}

/// Render one record: `LEVEL [boyko-Cnnnn ]file:line message`.
///
/// **The message is the format literal with its values interleaved**, which until L6 it was not:
/// this function printed `site.fmt` verbatim and appended a byte count, so every argument any call
/// site ever passed was transported across the ring and thrown away here. See `record.rs`'s header
/// for the measurement and for why the payload became self-describing instead of the site gaining
/// a decoder it could never be given.
///
/// The code is printed **only when the level carries one** — `Info`/`Debug`/`Trace` have none by
/// Decision 7, and a `boyko-B0000` on every third line would be noise that reads like a code.
fn render_record<const N: usize>(
    out: &mut crate::record::DspBuf<N>,
    site: &LogSite,
    payload: &[u8],
) {
    let _ = write!(out, "{} ", site.level.as_str());
    if site.class != 0 {
        // Byte-for-byte the shape the pre-migration `eprintln!`s wrote (`boyko-W1501`), so a
        // reader's grep and this repository's own gates keep matching after the migration.
        let _ = write!(out, "{}-{}{:04} ", site.prefix, site.class as char, site.code);
    }
    let _ = write!(out, "{}:{} ", site.file, site.line);
    if site.fields.is_empty() {
        let mut f = crate::site::LogFormatter::new(out);
        crate::record::render_payload(payload, site.fmt, &mut f);
    } else {
        // The `*_kv!` form (L11b). `fmt` is the whole message and carries no placeholders; the
        // values follow as `name=value`, in declaration order.
        //
        // ⚠️ `LogSite.fields` EXISTED FROM L1 AND NOTHING READ IT. It was written `&[]` by every
        // expansion and consumed by no renderer -- the same shape as `site.decode`, which L6 found
        // carrying a placeholder no drain ever called. A field with a writer and no reader is a
        // feature that cannot be observed to be missing.
        let _ = write!(out, "{}", site.fmt);
        let mut f = crate::site::LogFormatter::new(out);
        crate::record::render_named(payload, site.fields, &mut f);
    }
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
    // A `Manual` or `Scheduled` host has said who drains, so it does not also get a thread — and
    // saying so here rather than at `enable` keeps "what did the host ask for" in one place.
    WANT_SINK.store(
        u8::from(cfg.sink_thread && cfg.sink_mode == SinkMode::Thread),
        Ordering::Relaxed,
    );
    WANT_ECS_RING.store(u8::from(cfg.ecs_ring), Ordering::Relaxed);
    WANT_FILE.store(u8::from(cfg.file), Ordering::Relaxed);
    WANT_BINARY.store(u8::from(cfg.binary), Ordering::Relaxed);
    FILE_CAP.store(cfg.file_cap_bytes, Ordering::Relaxed);
    SINK_MODE.store(cfg.sink_mode as u8, Ordering::Relaxed);
    SINK_STATE.store(SinkState::Booted as u8, Ordering::Release);
}

/// The recorded drain mode.
#[must_use]
pub fn sink_mode() -> SinkMode {
    match SINK_MODE.load(Ordering::Relaxed) {
        1 => SinkMode::Manual,
        2 => SinkMode::Scheduled,
        _ => SinkMode::Thread,
    }
}

/// Run one drain pass by hand, for `SinkMode::Manual`.
///
/// Returns [`DrainResult::Busy`] when another consumer holds the role — a refusal, never a steal,
/// because stealing would create the second consumer the token exists to prevent.
pub fn drain() -> DrainResult {
    match drain_once() {
        Some(stats) => DrainResult::Ran(stats),
        None => DrainResult::Busy,
    }
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
    // CALIBRATION COMES FIRST, and the ordering is load-bearing rather than tidy.
    //
    // The binary sink writes its anchor at `open`, and the anchor carries `ticks_per_ns` so a
    // reader on another machine can turn a delta into a time. Opening a sink before calibrating
    // stamps the UNCALIBRATED 1.0 into the file -- MEASURED: `logdec` read `ticks_per_ns=1` and
    // reported a record 0.2 ms after open as `+85.215ms`. The 20 ms window still lives here and
    // nowhere else; a process that never asks for a log still never pays it.
    boyko_diag::clock::calibrate();
    if WANT_CONSOLE.load(Ordering::Relaxed) != 0 {
        crate::sync_out::set_console_enabled(true);
    }
    if WANT_FILE.load(Ordering::Relaxed) != 0 {
        // A refusal is not a launch failure: the synchronous channel and the rings still work, and
        // the host learns from `sink::file::is_open()` rather than from a missing file it has to
        // notice. Opening here and not at boot is the S13 rule — a syscall the runtime flag has
        // not authorised is exactly what boot may not make.
        crate::sink::file::open(FILE_CAP.load(Ordering::Relaxed));
    }
    if WANT_BINARY.load(Ordering::Relaxed) != 0 {
        // Same rule as the text sink one branch up: the syscall belongs to the enable path, not to
        // boot. A refusal is not a launch failure -- `binary::path_recorded()` and
        // `binary::frames_written()` are how a host learns, rather than by noticing a missing file.
        crate::sink::binary::open();
    }
    install_panic_hook();
    SINK_STATE.store(SinkState::Enabled as u8, Ordering::Release);
    // THE SESSION HEADER, and it is emitted here because here is the first moment it can be.
    //
    // After `calibrate`, so the clock scale it implies is real; after the state moves to `Enabled`,
    // because `info!` is refused before that. G16(d) owes three INDEPENDENT facts -- build profile,
    // runtime preset and ceiling -- and `preset::header` had no caller at all until this line: the
    // function existed, the rung was reported half-shipped, and nothing printed anything.
    crate::preset::header(boot_preset_recorded());
    // A new session: its shutdown has not ended yet.
    SHUTDOWN_OUTCOME.store(0, Ordering::Release);
    if WANT_SINK.load(Ordering::Relaxed) != 0 {
        SINK_RUNNING.store(1, Ordering::Release);
        SINK_PASSES.store(0, Ordering::Release);
        SINK_DIED.store(0, Ordering::Release);
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

/// The hook that was installed before ours, so we chain rather than replace.
///
/// **Chaining is not politeness.** The default hook prints the panic message and the backtrace; a
/// logger that replaced it would silence the one diagnostic that always worked, in exchange for
/// one that only works when it was enabled. A test harness's hook is what makes `#[should_panic]`
/// readable. Both must still run, and ours runs *first* so records are out before anything
/// downstream aborts.
static PREV_HOOK: std::sync::OnceLock<PanicHook> = std::sync::OnceLock::new();

/// A panic hook, in the shape `std::panic::take_hook` returns it.
type PanicHook = Box<dyn Fn(&std::panic::PanicHookInfo<'_>) + Sync + Send>;

/// Times our hook has run.
///
/// Behavioural evidence, because identity is not available: `take_hook` returns a
/// `Box<dyn Fn>` that cannot be compared, so *"is our hook installed"* is not a question the
/// standard library can answer. It has to be asked by observing what happens on a panic.
static HOOK_FIRED: AtomicU8 = AtomicU8::new(0);

/// How many times the panic hook has run, saturating at 255.
#[must_use]
pub fn hook_fired() -> u8 {
    HOOK_FIRED.load(Ordering::Acquire)
}

/// Install the panic hook, at most once per process.
///
/// Called from [`enable`], never from [`boot`]: a process that never asked for diagnostics must
/// not have its panic behaviour changed. It is **never uninstalled** — `set_hook` offers no
/// "restore mine only", and a `disable()` that called `set_hook(prev)` would clobber a hook some
/// other subsystem installed in between.
///
/// Because it is permanent, **the hook does nothing unless diagnostics are `Enabled`.** That is
/// not an optimisation: a hook that drained on every panic would reach into the rings during
/// unrelated panics — including a test harness's `#[should_panic]` cases — and consume records
/// their owners were about to inspect. MEASURED: the first version did exactly that and broke
/// three unrelated tests in this crate.
fn install_panic_hook() {
    if PREV_HOOK.get().is_some() {
        return;
    }
    let prev = std::panic::take_hook();
    if PREV_HOOK.set(prev).is_err() {
        // Another thread won the race and its `take_hook` already ran, so ours is now the hook it
        // captured. Putting it back would double-chain; leave the winner's installation alone.
        return;
    }
    std::panic::set_hook(Box::new(|info| {
        HOOK_FIRED.fetch_add(1, Ordering::Release);
        if state() == SinkState::Enabled {
            // Bounded and returns a value, so a stalled sink cannot turn a panic into a hang —
            // which would replace a diagnosable crash with an undiagnosable one.
            let _ = flush();
        }
        if let Some(prev) = PREV_HOOK.get() {
            prev(info);
        }
    }));
}

/// What a [`flush`] did.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FlushResult {
    /// Everything published before the call has been handed to the destinations.
    Flushed,
    /// **Nothing consumes the rings**, so there is nothing a flush could do. Returned
    /// *immediately*: a caller that waits two seconds to learn this has been told the same thing,
    /// two seconds later, on a path that usually runs while something is already going wrong.
    NoConsumer,
    /// A consumer exists but did not complete a pass within the bound. A defect signal, not an
    /// error to handle — the caller is on its way out either way.
    TimedOut,
}

/// Drain everything published before this call.
///
/// Two shapes, because there are two kinds of consumer:
///
/// - **No sink thread**: this thread claims the role and drains inline. That is the whole flush,
///   and it is synchronous.
/// - **Sink thread running**: wait for it to complete **two** passes. One is not enough — a pass
///   already in flight when `flush` was called may have loaded its horizon before this caller's
///   last record was published, so it can finish without having seen it. Two passes guarantee one
///   that started after the call.
///
/// The wait is bounded and terminates in a **value**, never in a hang. That matters more here than
/// almost anywhere: `flush` is on the crash path.
pub fn flush() -> FlushResult {
    if state() != SinkState::Enabled {
        return FlushResult::NoConsumer;
    }

    if SINK_RUNNING.load(Ordering::Acquire) == 0 {
        // Inline drain, THROUGH THE SINKS. The first draft hand-rolled a lane walk here that
        // rendered every record to the console oracle and nothing else — pre-L14 code that never
        // learned the sinks exist. MEASURED on a `shipping-min` host: a `flush` "delivered" its
        // records to a console that preset turns OFF, and the file the preset promised stayed
        // empty. `drain_once` is the one pass all three consumer shapes share; per-sink policy,
        // the once-register, delivered-counting and the request pump all live inside it, and a
        // flush that skipped any of them would make "flushed" mean less than "drained".
        //
        // A `None` claim means another consumer is draining these same rings right now, so
        // waiting for it is the same answer with more steps.
        let _ = drain_once();
        return FlushResult::Flushed;
    }

    let start = sink_passes();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while std::time::Instant::now() < deadline {
        // Wrapping, because the counter is a `u8` that laps every 256 passes and only differences
        // matter. A subtraction here would be wrong for exactly one window in 256 — the kind of
        // defect that appears once a session and reproduces never.
        if sink_passes().wrapping_sub(start) >= 2 {
            return FlushResult::Flushed;
        }
        std::thread::yield_now();
    }
    FlushResult::TimedOut
}

/// How the first [`shutdown`] of a session ended.
///
/// [`shutdown`] returns a `bool`, and its callers read it that way. The outcome says which way a
/// `false` went, which is the difference between "the durable log is complete and only the console
/// is behind" and "the durable log may lack its census" -- and a host on its exit path can act on
/// the second only if it can tell the two apart.
#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ShutdownOutcome {
    /// The resident sink thread's exit pass delivered everything within the bound, and the sinks
    /// closed after it.
    Joined = 1,
    /// There was no resident sink thread: the calling thread ran the final pass and closed.
    NoThread = 2,
    /// The bound expired with the drain role FREE: the sink thread was outside a pass, blocked
    /// writing its staged console lines to a consumer that stopped reading (or not scheduled at
    /// all). The calling thread delivered the remaining records to the durable sinks -- file,
    /// binary, ECS -- and closed them. Those records did not reach the console
    /// ([`console_lines_skipped`]); the lines the sink thread had staged reach it when its
    /// consumer reads again.
    ClosedAfterStall = 3,
    /// The bound expired with the drain role HELD: the sink is stalled inside a pass -- a durable
    /// write (the disk, a filter driver), a render, or a staged console overflow, which is written
    /// under the role. Nothing could be delivered safely, so the durable file lacks what that pass
    /// had not reached, visibly: its census has no `LOG-CENSUS limiter` line. [`await_exit`] waits
    /// longer and closes the sinks once the thread exits.
    StalledInsidePass = 4,
    /// The sink thread died -- it panicked -- before it could exit. Its unwind released the role,
    /// so the calling thread delivered and closed as in
    /// [`ClosedAfterStall`](Self::ClosedAfterStall); the panic itself was reported by the hook.
    SinkDied = 5,
}

/// `0` until a shutdown has ended this session; then a [`ShutdownOutcome`] discriminant.
///
/// Written once per session, by the FIRST shutdown: a second call finds no thread to stop and
/// would otherwise overwrite a stall with the `NoThread` it reads.
static SHUTDOWN_OUTCOME: AtomicU8 = AtomicU8::new(0);

/// Console lines a late close delivered to the durable sinks and not to the console.
static CONSOLE_SKIPPED: AtomicU32 = AtomicU32::new(0);

/// Record how this session's shutdown ended, unless an earlier call already did.
fn record_outcome(outcome: ShutdownOutcome) {
    // AcqRel on success: the outcome is published after everything the shutdown did, and a
    // reader that sees it (`Acquire` below) sees those effects -- the closed sinks included.
    let _ = SHUTDOWN_OUTCOME.compare_exchange(0, outcome as u8, Ordering::AcqRel, Ordering::Relaxed);
}

/// How this session's first [`shutdown`] ended, or `None` before one has.
#[must_use]
pub fn shutdown_outcome() -> Option<ShutdownOutcome> {
    match SHUTDOWN_OUTCOME.load(Ordering::Acquire) {
        1 => Some(ShutdownOutcome::Joined),
        2 => Some(ShutdownOutcome::NoThread),
        3 => Some(ShutdownOutcome::ClosedAfterStall),
        4 => Some(ShutdownOutcome::StalledInsidePass),
        5 => Some(ShutdownOutcome::SinkDied),
        _ => None,
    }
}

/// Console lines a late close delivered to the durable sinks only
/// ([`ShutdownOutcome::ClosedAfterStall`]); cumulative over the process.
#[must_use]
pub fn console_lines_skipped() -> u32 {
    CONSOLE_SKIPPED.load(Ordering::Relaxed)
}

/// After a [`shutdown`] that returned `false`, wait up to `bound` for the sink thread to exit,
/// then close the sinks. Returns whether it exited.
///
/// For a host that can afford a longer wait than `shutdown`'s 500 ms, on its exit path: a console
/// consumer that stalled for a second still receives the lines the sink thread had staged, and a
/// thread stalled inside a pass ([`ShutdownOutcome::StalledInsidePass`]) still delivers its census
/// to the durable file before the close. Returns `false` at once when no shutdown is in progress,
/// because nothing would ever move the state.
pub fn await_exit(bound: std::time::Duration) -> bool {
    let deadline = std::time::Instant::now() + bound;
    loop {
        match state() {
            SinkState::Exited => {
                close_sinks();
                return true;
            }
            SinkState::Exiting => {}
            _ => return false,
        }
        if std::time::Instant::now() >= deadline {
            return false;
        }
        std::thread::yield_now();
    }
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
/// the caller is on its way out either way. **An expired wait does not give up silently**: it
/// closes the durable sinks itself when the drain role is free, and records what it did either way
/// ([`shutdown_outcome`]); a host that can wait longer calls [`await_exit`].
pub fn shutdown() -> bool {
    // THE CENSUS FIRST, while the state is still `Enabled` and emission is still admitted. It is
    // ring-borne now, so the order is "emit, then deliver, then close" in BOTH arms: the summary
    // must be in the lanes before the final pass that moves it, and the sinks must outlive that
    // pass. Emitting after the swap would work too, but emitting before it keeps one rule —
    // nothing is offered after a consumer has been told to stop.
    close_out();
    if SINK_RUNNING.swap(0, Ordering::AcqRel) == 0 {
        // No resident consumer, so nothing will ever move what the lanes still hold — the census
        // just offered included. One final pass, then the close. MEASURED on a `shipping-min`
        // host before this arm drained at all: a record emitted after the last frame's drain was
        // simply gone. Only when `Enabled`: a never-enabled process has no sinks, and the pass
        // would spend the claim to deliver nowhere.
        if state() == SinkState::Enabled {
            let _ = drain_once();
        }
        close_sinks();
        SINK_STATE.store(SinkState::Exited as u8, Ordering::Release);
        record_outcome(ShutdownOutcome::NoThread);
        return true;
    }
    SINK_STATE.store(SinkState::Exiting as u8, Ordering::Release);
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(500);
    while std::time::Instant::now() < deadline {
        if state() == SinkState::Exited {
            // The thread's exit pass has delivered everything, the census included; the sinks
            // close only now. In the previous shape `close_out` closed the text file BEFORE that
            // pass, so the tail was delivered to a destination that no longer existed.
            close_sinks();
            record_outcome(ShutdownOutcome::Joined);
            return true;
        }
        // Acquire: pairs with the Release in `SinkDeathWatch::drop`. A dead thread will never
        // publish `Exited`, so waiting out the bound for it would only delay the close.
        if SINK_DIED.load(Ordering::Acquire) != 0 {
            break;
        }
        std::thread::yield_now();
    }
    let outcome = close_after_expiry();
    record_outcome(outcome);
    outcome == ShutdownOutcome::Joined
}

/// What an expired [`shutdown`] does instead of giving up: deliver and close when it can, and say
/// which of the two cases it was.
///
/// - **The role is free**: the sink thread is outside a pass -- under [`sink_loop`]'s shape,
///   blocked writing its staged console lines to a consumer that stopped reading, which is the CI
///   case -- or dead. This thread delivers what the lanes still hold, the census included, to every
///   durable sink and closes them, under the one claim. The console is skipped: it is the consumer
///   that stalled. The synchronous channel is refused for the same reason: the stalled sink thread
///   holds stderr's lock, so a sink's own report (the file cap, a lossy rotation) would wait on it
///   without a bound.
/// - **The role is held**: the sink is stalled inside a pass, and nothing can be written safely.
///   Recorded, not hidden.
#[cold]
#[inline(never)]
fn close_after_expiry() -> ShutdownOutcome {
    if state() == SinkState::Exited {
        // The exit pass finished just past the bound.
        close_sinks();
        return ShutdownOutcome::Joined;
    }
    let Some(token) = crate::drain_owner::try_claim() else {
        return ShutdownOutcome::StalledInsidePass;
    };
    // Acquire: pairs with `SinkDeathWatch::drop`'s Release.
    let died = SINK_DIED.load(Ordering::Acquire) != 0;
    {
        let _refused = crate::sync_out::refuse_on_this_thread();
        let _ = deliver(&token, Console::Skip);
        close_sinks_with(&token);
    }
    drop(token);
    if died {
        // Nothing else will ever publish it: the thread that would have is gone.
        SINK_STATE.store(SinkState::Exited as u8, Ordering::Release);
        ShutdownOutcome::SinkDied
    } else {
        ShutdownOutcome::ClosedAfterStall
    }
}

/// Report, on the synchronous channel, a sink thread still stalled inside a pass after a host's
/// [`await_exit`] grace ([`ShutdownOutcome::StalledInsidePass`]): the durable log may lack its
/// census, and this line is the only place left to say so.
///
/// Returns whether the line was written. It is NOT written when the stalled write is the console
/// itself (a staged overflow written under the role): the report would wait behind the write it is
/// about, and on a consumer that never reads again that is a hang on the exit path. The outcome
/// stays readable either way.
#[cold]
#[inline(never)]
pub fn report_stalled_exit() -> bool {
    if OVERFLOW_WRITE_IN_FLIGHT.load(Ordering::Acquire) {
        return false;
    }
    crate::sync_out::write_oracle_line(
        "boyko-log: ",
        "shutdown: the sink thread is still stalled inside a drain pass; the durable log may lack \
         its census (no `LOG-CENSUS limiter` line)",
    )
    .is_some()
}

/// Print the census and close the file, if this process ever opened one.
///
/// Split out because [`shutdown`] and [`disable`] both owe it and a second copy is how the two
/// would come to disagree about whether a session's last lines reached the disk.
fn close_out() {
    crate::census::print();
}

/// Close both file-backed sinks under the token, AFTER the final delivery pass.
///
/// After and not before, because the census is ring-borne (owner-directed 2026-08-20): a close
/// that ran ahead of the last drain would shut the file over the very records that summarize it —
/// MEASURED in the previous shape, where `close_out` closed the text sink and the resident
/// thread's exit pass then delivered the tail to a destination that no longer existed.
///
/// A busy token means another consumer is mid-drain; the sinks are flushed on their next write
/// and the OS closes them at exit, which is a worse outcome than a clean close and a better one
/// than racing a writer. The binary sink is closed here too — it previously had no shutdown-time
/// close at all, only the request-ring route a host had to know to ask for.
fn close_sinks() {
    if let Some(token) = crate::drain_owner::try_claim() {
        close_sinks_with(&token);
    }
}

/// [`close_sinks`] under a role the caller already holds -- the late close's single claim.
fn close_sinks_with(token: &crate::drain_owner::DrainToken) {
    if crate::sink::file::is_open() {
        crate::sink::file::close(token);
    }
    crate::sink::binary::close(token);
}

/// Turn diagnostics off again for a session that no longer wants them.
///
/// Closes the destinations. It does **not** reclaim `.bss` — `.bss` is never freed, and a design
/// that pretended otherwise would be claiming a saving it cannot deliver.
pub fn disable() {
    close_out();
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
        WANT_SINK.store(0, Ordering::Relaxed);
        // The pass counter is process-global too. `enable` zeroes it only when it spawns, so a
        // no-thread test that did not reset it here inherits whatever a sink-thread test left --
        // which is how "no thread was asked for, so no pass may have happened" started failing
        // deterministically the moment a sink-thread test was added beside it.
        SINK_PASSES.store(0, Ordering::Release);
        SHUTDOWN_OUTCOME.store(0, Ordering::Release);
        SINK_DIED.store(0, Ordering::Release);
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
        boot(LogConfig { console: true, sink_thread: false, ecs_ring: false, ..LogConfig::default() });
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
        boot(LogConfig { console: true, sink_thread: false, ecs_ring: false, ..LogConfig::default() });
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
        boot(LogConfig { console: false, sink_thread: false, ecs_ring: false, ..LogConfig::default() });
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
        boot(LogConfig { console: false, sink_thread: true, ecs_ring: false, ..LogConfig::default() });
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
    fn flush_without_a_consumer_answers_immediately() {
        // The property that matters is the LATENCY, not the value: a caller told `NoConsumer`
        // after two seconds has been told the same thing, two seconds later, on a path that
        // usually runs while something is already going wrong.
        let _s = crate::drain_owner::test_serial();
        reset();
        let t0 = std::time::Instant::now();
        assert_eq!(flush(), FlushResult::NoConsumer, "un-booted means nothing consumes");
        assert!(t0.elapsed() < std::time::Duration::from_millis(50), "must not wait to say no");

        boot(LogConfig { console: false, sink_thread: false, ecs_ring: false, ..LogConfig::default() });
        assert_eq!(flush(), FlushResult::NoConsumer, "booted but not enabled is still nothing");
        reset();
    }

    #[test]
    fn flush_drains_inline_when_there_is_no_sink_thread() {
        let _s = crate::drain_owner::test_serial();
        reset();
        boot(LogConfig { console: false, sink_thread: false, ecs_ring: false, ..LogConfig::default() });
        assert!(enable());
        let t0 = std::time::Instant::now();
        assert_eq!(flush(), FlushResult::Flushed);
        assert!(
            t0.elapsed() < std::time::Duration::from_millis(500),
            "an inline drain is synchronous; it must not fall into the sink-thread wait"
        );
        reset();
    }

    #[test]
    fn flush_waits_for_two_sink_passes() {
        // Two, not one: a pass already in flight may have fixed its horizon before this caller's
        // last record was published, so it can finish without ever having seen it.
        let _s = crate::drain_owner::test_serial();
        reset();
        boot(LogConfig { console: false, sink_thread: true, ecs_ring: false, ..LogConfig::default() });
        assert!(enable());
        let before = sink_passes();
        assert_eq!(flush(), FlushResult::Flushed);
        assert!(
            sink_passes().wrapping_sub(before) >= 2,
            "flush returned before two passes had completed"
        );
        assert!(shutdown());
        reset();
    }

    #[test]
    fn the_hook_runs_ours_and_still_chains_to_the_previous_one() {
        // Behavioural, because identity is unavailable: `take_hook` returns an uncomparable
        // `Box<dyn Fn>`. Our counter moving proves ours ran; the panic still being caught and
        // still reaching stderr proves the previous one did too. Replacing rather than chaining
        // would silence the one diagnostic that always worked.
        let _s = crate::drain_owner::test_serial();
        reset();
        boot(LogConfig { console: false, sink_thread: false, ecs_ring: false, ..LogConfig::default() });
        assert!(enable());

        let before = hook_fired();
        let caught = std::panic::catch_unwind(|| panic!("deliberate, hook chain"));
        assert!(caught.is_err(), "the panic must still propagate");
        assert!(hook_fired() > before, "our hook did not run; records would be lost on a crash");
        assert!(PREV_HOOK.get().is_some(), "the previous hook must be retained, not discarded");
        reset();
    }

    #[test]
    fn a_panic_while_diagnostics_are_off_does_not_reach_the_rings() {
        // The hook is permanent by design, so it runs on EVERY panic in the process -- including
        // a test harness's `#[should_panic]` cases. Draining there would consume records their
        // owners were about to inspect. MEASURED: the first version did exactly that and broke
        // three unrelated tests in this crate.
        let _s = crate::drain_owner::test_serial();
        reset();
        let before = crate::lifecycle::sink_passes();
        let caught = std::panic::catch_unwind(|| panic!("deliberate, diagnostics off"));
        assert!(caught.is_err());
        assert_eq!(
            crate::lifecycle::sink_passes(),
            before,
            "a panic with diagnostics off must not touch the rings"
        );
        reset();
    }

    #[test]
    fn enable_is_idempotent_and_refuses_when_there_is_nothing_to_enable() {
        let _s = crate::drain_owner::test_serial();
        reset();
        assert!(!enable(), "enable before boot must refuse rather than half-initialise");
        boot(LogConfig { console: true, sink_thread: false, ecs_ring: false, ..LogConfig::default() });
        assert!(enable());
        assert!(enable(), "a launch flag parsed twice must not calibrate twice");
        disable();
        assert_eq!(state(), SinkState::Booted, "disable returns to Booted, not to NotBooted");
        reset();
    }

    // ── A console consumer that stops reading (ci-fix-1 F1) ─────────────────────────────────
    //
    // Master's push CI went red at `log_host_editor`'s join assertion when the runner's stderr
    // pipe stopped being read for about a second during the census: the sink thread wrote the
    // console FIRST, under the drain role, so every durable write queued behind the blocked line
    // waited too, and `shutdown` gave up after 500 ms without closing anything -- the durable
    // file lost its census.
    // `sync_out::test_stall` reproduces that consumer on the sink thread.

    /// A threaded console + file session with the `log` target armed, so the census reaches both
    /// sinks. Its drop disarms the stall, lets the sink thread exit, closes, and restores the
    /// target, so a red here cannot poison the next test.
    struct ThreadedFileSession {
        path: std::path::PathBuf,
        log_was: crate::target::TargetControl,
    }

    impl ThreadedFileSession {
        fn start(name: &str, file_cap_bytes: u64) -> Self {
            reset();
            // `boot` arms no target and `CONTROL` is `.bss`-Off, so without this the census rows
            // would be refused at the gate and every assertion below would read the admission
            // policy, not the stall.
            let id = <crate::Log as crate::target::LogTarget>::ID;
            let log_was = crate::target::target_control(id);
            crate::target::set_target_control(
                id,
                crate::target::TargetControl::new(crate::GLOBAL_CEILING, 0, false),
            );
            let path = std::env::temp_dir()
                .join(format!("boyko_lifecycle_{name}_{}.log", std::process::id()));
            let _ = std::fs::remove_file(&path);
            assert!(crate::sink::file::set_path(path.to_str().expect("a UTF-8 temp path")));
            boot(LogConfig {
                console: true,
                sink_thread: true,
                file: true,
                file_cap_bytes,
                ..LogConfig::default()
            });
            assert!(enable());
            // The session header reaches the console before any test arms a stall.
            assert_eq!(flush(), FlushResult::Flushed);
            ThreadedFileSession { path, log_was }
        }

        fn text(&self) -> String {
            std::fs::read_to_string(&self.path).unwrap_or_default()
        }
    }

    impl Drop for ThreadedFileSession {
        fn drop(&mut self) {
            crate::sync_out::test_stall::disarm();
            wait_for_exit(std::time::Duration::from_secs(5));
            close_sinks();
            crate::target::set_target_control(<crate::Log as crate::target::LogTarget>::ID, self.log_was);
            let _ = std::fs::remove_file(&self.path);
            reset();
        }
    }

    /// A raw wait on `Exited`, without [`await_exit`]'s close, so a test can observe whether the
    /// late close closed the file itself.
    fn wait_for_exit(bound: std::time::Duration) {
        let deadline = std::time::Instant::now() + bound;
        while state() != SinkState::Exited && std::time::Instant::now() < deadline {
            std::thread::yield_now();
        }
    }

    /// The census rows `text` lacks: one `LOG-CENSUS target=<name> ` per target, then the limiter.
    fn missing_census_rows(text: &str) -> (usize, Vec<String>) {
        let mut want: Vec<String> = crate::target::targets()
            .map(|(_, name)| format!("LOG-CENSUS target={name} "))
            .collect();
        want.push("LOG-CENSUS limiter ".to_owned());
        let total = want.len();
        (total, want.into_iter().filter(|row| !text.contains(row.as_str())).collect())
    }

    /// Emit one console-admitted record and wait until the sink thread is stalled writing it.
    fn stall_the_sink_on_a_record() {
        crate::sync_out::test_stall::arm();
        crate::info!(crate::Log, "the record the stalled console is writing");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !crate::sync_out::test_stall::entered() && std::time::Instant::now() < deadline {
            std::thread::yield_now();
        }
        assert!(crate::sync_out::test_stall::entered(), "fixture: the sink never reached the stalled write");
    }

    #[test]
    fn a_threaded_console_session_delivers_the_whole_census_to_the_file() {
        // The control: no stall, so the 500 ms join holds, and the file check below is shown able
        // to read a complete census before the two stall tests rely on it.
        let _s = crate::drain_owner::test_serial();
        let session = ThreadedFileSession::start("control", 0);
        assert!(shutdown(), "an unstalled threaded shutdown must join within its bound");
        assert_eq!(shutdown_outcome(), Some(ShutdownOutcome::Joined));
        let (total, missing) = missing_census_rows(&session.text());
        assert!(missing.is_empty(), "the durable file lacks {} of {total} census rows: {missing:?}", missing.len());
    }

    #[test]
    fn a_console_stalled_in_the_census_pass_costs_the_file_nothing() {
        let _s = crate::drain_owner::test_serial();
        let session = ThreadedFileSession::start("census_pass", 0);
        crate::sync_out::test_stall::arm();
        let joined = shutdown();
        let (total, missing) = missing_census_rows(&session.text());
        assert!(
            missing.is_empty(),
            "the durable file holds {} of {total} census rows after a console stall: missing {missing:?}",
            total - missing.len()
        );
        assert_eq!(
            shutdown_outcome(),
            Some(ShutdownOutcome::ClosedAfterStall),
            "an expired join with the role free must deliver and close (joined {joined})"
        );
        crate::sync_out::test_stall::disarm();
        wait_for_exit(std::time::Duration::from_secs(5));
        assert_eq!(state(), SinkState::Exited, "the sink thread must exit once its console drains");
        // Only after `Exited`: the sink's exit pass takes `&mut` over the file cell, which this
        // read would race.
        assert!(!crate::sink::file::is_open(), "the late close must close the file it delivered to");
    }

    #[test]
    fn a_console_stalled_in_an_earlier_pass_is_overtaken_by_the_late_close() {
        let _s = crate::drain_owner::test_serial();
        let session = ThreadedFileSession::start("earlier_pass", 0);
        stall_the_sink_on_a_record();
        let skipped_before = console_lines_skipped();
        let joined = shutdown();
        let (total, missing) = missing_census_rows(&session.text());
        assert!(
            missing.is_empty(),
            "the durable file holds {} of {total} census rows after a console stall: missing {missing:?}",
            total - missing.len()
        );
        assert_eq!(
            shutdown_outcome(),
            Some(ShutdownOutcome::ClosedAfterStall),
            "an expired join with the role free must deliver and close (joined {joined})"
        );
        // The whole census went through the late pass, so none of it was written to the console.
        assert!(
            console_lines_skipped() - skipped_before >= total as u32,
            "the late pass must count the console lines it did not write: {} for {total} census rows",
            console_lines_skipped() - skipped_before
        );
        crate::sync_out::test_stall::disarm();
        wait_for_exit(std::time::Duration::from_secs(5));
        assert_eq!(state(), SinkState::Exited, "the sink thread must exit once its console drains");
    }

    #[test]
    fn a_cap_crossed_in_the_late_pass_does_not_wait_on_the_stalled_console() {
        // `file.rs` reports its cap through the synchronous channel, and the stalled sink thread
        // holds stderr's lock: a late pass that reached that channel would wait on it without a
        // bound. 1 KiB holds the header and the stalled record and not the census, so the late
        // pass is the one that crosses the cap.
        let _s = crate::drain_owner::test_serial();
        let _session = ThreadedFileSession::start("cap", 1024);
        stall_the_sink_on_a_record();
        let suppressed_before = crate::sync_out::suppressed_writes();
        let joined = shutdown();
        assert!(
            !crate::sync_out::test_stall::gave_up(),
            "shutdown returned only because the console stall gave up: the late pass reached the \
             synchronous channel and waited on the stalled console (joined {joined})"
        );
        assert_eq!(
            shutdown_outcome(),
            Some(ShutdownOutcome::ClosedAfterStall),
            "an expired join with the role free must deliver and close (joined {joined})"
        );
        assert!(crate::sink::file::state().1, "fixture: the late pass must have crossed the 1 KiB cap");
        assert!(
            crate::sync_out::suppressed_writes() > suppressed_before,
            "the cap report the late pass did not write must be counted"
        );
    }

    #[test]
    fn a_sink_thread_that_died_is_named_and_its_census_still_reaches_the_file() {
        // A dead thread releases the role on its unwind and never publishes `Exited`, so an
        // expired join finds the role free exactly as it does behind a stalled console. Naming
        // that `ClosedAfterStall` would send a reader to look for a console that never stalled.
        let _s = crate::drain_owner::test_serial();
        let session = ThreadedFileSession::start("died", 0);
        crate::sync_out::test_stall::arm_panic();
        crate::info!(crate::Log, "the record the dying sink thread is writing");
        // Until the unwind is over, not merely begun: the panic hook flushes first (bounded at 2 s,
        // and on this thread nothing can complete a pass), and the role is released only after.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while SINK_DIED.load(Ordering::Acquire) == 0 && std::time::Instant::now() < deadline {
            std::thread::yield_now();
        }
        assert_ne!(SINK_DIED.load(Ordering::Acquire), 0, "fixture: the sink thread did not die");
        let joined = shutdown();
        assert_eq!(
            shutdown_outcome(),
            Some(ShutdownOutcome::SinkDied),
            "a sink thread that panicked must be named, not read as a stall (joined {joined})"
        );
        let (total, missing) = missing_census_rows(&session.text());
        assert!(
            missing.is_empty(),
            "the durable file holds {} of {total} census rows after the sink thread died: \
             missing {missing:?}",
            total - missing.len()
        );
        assert_eq!(state(), SinkState::Exited, "nothing else will publish `Exited` for a dead thread");
    }
}
