//! The host's terminal-exit reporters — the three records that must survive a run nobody asked
//! to log.
//!
//! # Why this module exists at all, measured rather than assumed *(L8b)*
//!
//! Every other site this rung migrates becomes a plain `warn!`/`info!` and is done. These three
//! cannot, and the reason is a property of the logging design rather than of these call sites:
//!
//! With `BOYKO_LOG` unset — the default for every run of this engine that is not itself a logging
//! experiment — `boot_and_enable_logging_from_env` records the configuration and **returns before
//! `enable()`**. `CONTROL` stays `.bss`-zero, so every target's runtime ceiling is `Off`, so the
//! third gate in `error!`'s expansion is false and the site is one predicted branch and a return.
//! Not a dropped record. Not a counted loss. **Nothing.**
//!
//! That is specified, not accidental: `logging/sink-lifecycle`'s Decision 25 states it in as many
//! words — *"a flag-off run of any other preset configures nothing either, because `enable()`
//! never ran and no sink slot was ever opened"* — and `tests/log_host_reachable.rs` pins it,
//! asserting `flush() == NoConsumer` after a full `EnginePlugins` build with the flag absent, on
//! the argument that a thread spawned there *"would be a cost no launch flag authorised"*.
//!
//! **The three conditions below are not diagnostics.** They are the process saying why it is
//! about to stop. Migrating them naively would mean an engine that exits with no output at all
//! unless the operator had already suspected they would need a log — which is exactly backwards,
//! because the run that needed the explanation is the one that is now over.
//!
//! # The shape, and where it comes from
//!
//! Not invented here. `boyko_threadpool::worker::abort_on_task_panic` hit the identical problem at
//! L6 and resolved it identically, with the reason written beside it: *"Diagnostics were never
//! enabled, so the record above is in a ring nothing will ever read and the abort decision would
//! be INVISIBLE — strictly worse than the unconditional `eprintln!` this replaced."* This module
//! is that pattern factored, so the engine has **one** copy of it rather than a fourth, fifth and
//! sixth.
//!
//! `flush()` rather than `lifecycle::state()`, and the difference is load-bearing in both
//! directions:
//!
//! * when there **is** a consumer, the record is still sitting in a lane ring, and this host
//!   drops the sink thread's `JoinHandle` at spawn — so `return AppExit(true)` two lines later
//!   races the drain. `flush()` is what makes the record leave;
//! * when there is **not**, `flush()` answers `NoConsumer` on the same call that would have
//!   drained it, so the fallback needs no second query and cannot disagree with itself.
//!
//! # What is NOT here, and why the line is where it is
//!
//! The degrade sites (`W3005`–`W3009`) and the thirty `info!` sites get no fallback. A degrade is
//! a diagnostic: the engine went on running, the operator can re-run with `BOYKO_LOG=warn`, and
//! putting them on `stderr` unconditionally would re-create the exact noise the migration exists
//! to retire. The line is *"is the process still able to tell you this later"*, and for these
//! three the answer is no.
//!
//! Each function owes a row in `print_allowlist.txt` at L8c naming that reason — the same debt
//! `abort_on_task_panic` already carries.

use core::fmt::Write as _;

use boyko_log::DspBuf;
use boyko_log::codes;
use boyko_log::lifecycle::{FlushResult, flush};

/// How much of an error's `Debug` form reaches the record.
///
/// A bound rather than an allocation, because this runs on a path that has already established
/// something is wrong with the device — `format!` here would ask the allocator for memory at the
/// one moment its answer is least trustworthy. 192 bytes holds every `VulkanError` and
/// `HostBootError` variant this engine constructs; a longer one is truncated, which `DspBuf`
/// records rather than hides.
const ERR_BYTES: usize = 192;

/// How much of one pre-rendered diagnostic line reaches the record. The longest is the 12-float
/// `GpuLight` row, at roughly 220 bytes.
const DUMP_BYTES: usize = 256;

/// Render a whole `format_args!` into a bounded stack buffer, spec and all.
///
/// # Why this exists, and why it is used in exactly one place
///
/// The record renderer **ignores format specifications**. `render_payload` scans the literal for
/// `{`, skips to the matching `}` without reading what is between them, and prints the tagged
/// value with its own default — so `{:.3}` on an `f32` reaches the reader as full precision and
/// `{:#06x}` reaches it as decimal. That is the right trade for a tagged payload, whose point is
/// that a *consumer* chooses the presentation.
///
/// `dump_diagnostics` is the one site in this crate where the presentation IS the product: it
/// prints cascade matrices, atlas faces and 12-lane light rows for a human comparing two runs by
/// eye, and at full `f32` precision those rows stop being comparable. So its lines are rendered
/// **before** they enter the record, and the record carries one string.
///
/// Everywhere else in this migration the values are passed as values. This is a deliberate,
/// bounded exception with a stated reason, not the pattern.
#[cold]
#[inline(never)]
pub(crate) fn line(args: core::fmt::Arguments<'_>) -> DspBuf<DUMP_BYTES> {
    let mut buf = DspBuf::<DUMP_BYTES>::new();
    // Cannot fail: `DspBuf`'s `write_str` truncates instead of erroring.
    let _ = buf.write_fmt(args);
    buf
}

/// Render a value's `Debug` form into a bounded stack buffer.
///
/// `DspBuf::render` is `Display`-only, and most of the values these records carry are `Debug` —
/// the sites this rung replaces printed `{e:?}`. Keeping `Debug` keeps the enum variant in the
/// message, which is the half of a Vulkan error or a degrade reason an operator can act on.
///
/// Returned by value, and the caller binds it before taking `as_str()`. `dsp!(format_args!(..))`
/// would read better and does not compile: `format_args!`'s result may not outlive the statement
/// that built it, so the borrow the macro hands to the emission site is already dead.
#[cold]
#[inline(never)]
pub(crate) fn debug_into<E: core::fmt::Debug + ?Sized>(err: &E) -> DspBuf<ERR_BYTES> {
    let mut buf = DspBuf::<ERR_BYTES>::new();
    // Cannot fail: `DspBuf`'s `write_str` truncates instead of erroring. Dropped rather than
    // unwrapped so a `Debug` impl that returns `Err` cannot panic the exit path.
    let _ = write!(&mut buf, "{err:?}");
    buf
}

/// Report that a host boot stage failed and the process is exiting — `boyko-E3002`.
///
/// `stage` names which one (`"vulkan device"`, `"window host"`, `"bindless texture table"`), so
/// three distinguishable failures do not arrive as one format literal. That is `E2103`'s argument
/// applied one crate later: before L6-A's tagged payload the stage could only have been baked into
/// three separate literals, i.e. three codes for one condition and one fix.
#[cold]
#[inline(never)]
pub(crate) fn report_boot_stage_failed<E: core::fmt::Debug>(stage: &str, err: &E) {
    let rendered = debug_into(err);
    let e = rendered.as_str();
    boyko_log::error!(
        boyko_log::App,
        codes::E3002,
        "host boot failed at the {} stage ({}) - exiting",
        stage,
        e
    );
    if flush() == FlushResult::NoConsumer {
        eprintln!("boyko-E3002: host boot failed at the {stage} stage ({e}) - exiting");
    }
}

/// Report a terminal device error inside the frame loop — `boyko-E3003`.
///
/// `site` distinguishes the two places the loop can learn it (`"frame fence wait"`, `"render"`),
/// which is the difference between a device lost before this frame's work was submitted and one
/// lost after. The renderer must not be reused past either (`frame_driver`'s contract), so both
/// end the run.
#[cold]
#[inline(never)]
pub(crate) fn report_terminal_device_error<E: core::fmt::Debug>(site: &str, err: &E) {
    let rendered = debug_into(err);
    let e = rendered.as_str();
    boyko_log::error!(
        boyko_log::App,
        codes::E3003,
        "terminal device error at {} ({}) - exiting",
        site,
        e
    );
    if flush() == FlushResult::NoConsumer {
        eprintln!("boyko-E3003: terminal device error at {site} ({e}) - exiting");
    }
}

/// Report that this platform has no windowing arm — `boyko-E3004`.
///
/// Not a failure to repair: `boyko_rhi_vulkan`'s D8 makes windowing Windows-first and the
/// XCB/Wayland arm lands when Linux on-screen is first targeted. It is a terminal exit all the
/// same, and a silent one would read as a crash.
///
/// `#[cfg(not(windows))]` because its only caller is the non-Windows `run_windowed` arm, and
/// `-D warnings` refuses a function nothing calls. The two above need no gate: both runner arms
/// can reach them.
#[cfg(not(windows))]
#[cold]
#[inline(never)]
pub(crate) fn report_windowing_unsupported() {
    boyko_log::error!(
        boyko_log::App,
        codes::E3004.number(),
        "windowing is not implemented for this platform - exiting"
    );
    if flush() == FlushResult::NoConsumer {
        eprintln!("boyko-E3004: windowing is not implemented for this platform - exiting");
    }
}

/// The extent/VRAM probe half of `W3005`.
///
/// A named module-level `static`, not one inside the function: a `Once` latch is PROCESS
/// state, and an observer that cannot reset it proves only that nothing else in the binary
/// reached this condition first.
pub(crate) static W3005_PROBE_SITE: codes::OnceSite = codes::OnceSite::new();

// ───────────────────────── the degrade reporters (W3005..W3008, E3010) ─────────────────────────
//
// Every one of these is a NAMED function with a NAMED module-level latch, and both halves of that
// are the same decision. L8a's `boyko_render` migration learnt it the expensive way: three of its
// observers re-emitted the `warn!` inline instead of calling the production reporter, so they were
// green against code they had written themselves and could not have failed. A reporter that is a
// function is a reporter a test can CALL; a latch that is a named `static` is a precondition a
// test can RESET, and an observer that cannot control its preconditions is one whose green means
// "in this order, this time".
//
// The conditions themselves are not reachable from a test binary on this box -- they need a
// booted Vulkan device, a window surface and a device whose caps refuse something. Splitting the
// report from the condition is what lets check 5 be satisfied by an observation rather than by a
// row in `untested_codes.txt` saying nobody looked.

/// The SSAA extent/VRAM probe refused the requested scale — `boyko-W3005`, site one of two.
#[cold]
#[inline(never)]
pub(crate) fn report_ssaa_probe_refused(
    want: u32,
    dims_ok: bool,
    vram_ok: bool,
    est: u64,
    heap: u64,
) {
    if !W3005_PROBE_SITE.claim() {
        return;
    }
    boyko_log::warn!(
        boyko_log::App,
        codes::W3005,
        "SSAA {}x unavailable (dims_ok={} vram_ok={} est={} heap={}) -> Off",
        want,
        dims_ok,
        vram_ok,
        est,
        heap
    );
}

/// The admitted-set half of `W3005`. A SECOND latch for the same code, which is F11's point.
///
/// A named module-level `static`, not one inside the function: a `Once` latch is PROCESS
/// state, and an observer that cannot reset it proves only that nothing else in the binary
/// reached this condition first.
pub(crate) static W3005_SCALE_SITE: codes::OnceSite = codes::OnceSite::new();

/// The requested SSAA scale is not one this build admits — `boyko-W3005`, site two of two.
///
/// A SECOND latch for the same code, which is the point rather than an oversight: `Once` is per
/// site (F11), and sharing one would let a probe refusal silence this line for the rest of the
/// process.
#[cold]
#[inline(never)]
pub(crate) fn report_ssaa_scale_unsupported(want: u32, admitted: &str) {
    if !W3005_SCALE_SITE.claim() {
        return;
    }
    boyko_log::warn!(
        boyko_log::App,
        codes::W3005,
        "SSAA scale {} unsupported (admitted: {}) -> Off",
        want,
        admitted
    );
}

/// One reason the resolved render path is not the requested one — `boyko-W3006`.
///
/// **No latch, and that is the decision this row exists for.** The caller walks a SET of reasons,
/// so a latch here would report the first and silently drop the rest — `W2102`'s F11 failure mode
/// reached through iteration instead of through separate sites.
#[cold]
#[inline(never)]
pub(crate) fn report_render_path_degraded(reason: &str) {
    boyko_log::warn!(
        boyko_log::App,
        codes::W3006,
        "render path degraded ({})",
        reason
    );
}

/// `W3007`'s latch. A named module-level `static`, not one inside the function: a `Once` latch is
/// PROCESS state, and an observer that cannot reset it proves only that nothing else in the binary
/// reached this condition first.
pub(crate) static W3007_SITE: codes::OnceSite = codes::OnceSite::new();

/// The VB geometry table could not be built — `boyko-W3007`.
#[cold]
#[inline(never)]
pub(crate) fn report_geometry_table_failed(err: &str) {
    if !W3007_SITE.claim() {
        return;
    }
    boyko_log::warn!(
        boyko_log::App,
        codes::W3007,
        "MeshGeometryTable::new failed ({}) - the VB geometry table is disabled",
        err
    );
}

/// `W3008`'s latch. Same argument as `W3007_SITE` above.
pub(crate) static W3008_SITE: codes::OnceSite = codes::OnceSite::new();

/// A profiling knob was set on a device that cannot serve it — `boyko-W3008`.
#[cold]
#[inline(never)]
pub(crate) fn report_profiling_knob_unserviceable(knob: &str, why: &str) {
    if !W3008_SITE.claim() {
        return;
    }
    boyko_log::warn!(
        boyko_log::App,
        codes::W3008,
        "{} is set but {} - the instrument stays disabled",
        knob,
        why
    );
}

/// A dump or artifact could not be written — `boyko-E3010`.
///
/// One reporter for five call sites, with `kind` naming which one. That is `E2103`'s argument
/// again: five near-identical literals would have been five codes for one condition and one fix,
/// or one code whose page could not say which dump failed.
///
/// `RatePolicy::Every` and no latch: a run that armed three dumps and could write none of them has
/// three things to report.
#[cold]
#[inline(never)]
pub(crate) fn report_dump_write_failed(kind: &str, path: &str, err: &str) {
    boyko_log::error!(
        boyko_log::Host,
        codes::E3010,
        "{} write FAILED ({}) -> {}",
        kind,
        err,
        path
    );
}

/// An environment override named a value this build does not recognise — `boyko-W3009`.
///
/// One reporter, and the LATCH IS THE CALLER'S. Two variables are parsed through this and each
/// owns its own `OnceSite`, because a mistyped `BOYKO_RENDER_PATH` must not silence a mistyped
/// `BOYKO_GEOMETRY_LEGS`. Passing the latch in rather than declaring one here is what keeps that
/// property visible at the site that depends on it.
#[cold]
#[inline(never)]
pub(crate) fn report_unrecognized_env_value(
    site: &codes::OnceSite,
    var: &str,
    value: &str,
    fallback: &str,
    valid: &str,
) {
    if !site.claim() {
        return;
    }
    boyko_log::warn!(
        boyko_log::App,
        codes::W3009,
        "{}='{}' unrecognized -> {} (valid: {})",
        var,
        value,
        fallback,
        valid
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use boyko_log::probe::{last_message, observe_lock, watch, watched};

    /// Raise a target's ceiling so a `Warn`/`Error` is admitted at all.
    ///
    /// Without it every assertion below would read `0` and pass for the wrong reason: with the
    /// runtime ceiling at `Off` the macro's third gate is false and the site is never entered, so
    /// "no emission" is indistinguishable from "the reporter is broken". That is the same shape as
    /// this rung's own central finding about the default run.
    fn arm_app() {
        boyko_log::probe::arm::<boyko_log::App>();
    }

    fn arm_host() {
        boyko_log::probe::arm::<boyko_log::Host>();
    }

    #[test]
    fn e3002_names_the_stage_that_failed() {
        let _lock = observe_lock();
        arm_app();
        watch(b'E', codes::E3002.number());
        report_boot_stage_failed("window host", &"SurfaceCreationFailed");
        assert_eq!(watched(), 1, "the boot-failure reporter must emit exactly one record");
        let msg = last_message();
        assert!(
            msg.contains("window host"),
            "the STAGE is the argument that makes one code serve three sites: {msg}"
        );
        assert!(
            msg.contains("SurfaceCreationFailed"),
            "the carried error must reach the record, not only the stage: {msg}"
        );
    }

    #[test]
    fn e3003_names_where_the_device_died() {
        let _lock = observe_lock();
        arm_app();
        watch(b'E', codes::E3003.number());
        report_terminal_device_error("frame fence wait", &"DeviceLost");
        assert_eq!(watched(), 1);
        let msg = last_message();
        assert!(msg.contains("frame fence wait"), "{msg}");
        assert!(msg.contains("DeviceLost"), "{msg}");
    }

    #[test]
    fn w3005_reports_both_ssaa_refusals_because_the_latches_are_separate() {
        // THE POINT OF THIS TEST. One code, two sites, two latches -- so a probe refusal must not
        // silence the "that scale does not exist" line. A code-scoped latch would make the second
        // assertion below read 0, which is exactly the defect F11 named for `W2102`.
        let _lock = observe_lock();
        arm_app();
        W3005_PROBE_SITE.reset();
        W3005_SCALE_SITE.reset();

        watch(b'W', codes::W3005.number());
        report_ssaa_probe_refused(2, true, false, 1 << 30, 1 << 31);
        assert_eq!(watched(), 1, "the probe refusal reports");
        assert!(last_message().contains("vram_ok=false"), "the probe's verdict must be carried");

        report_ssaa_scale_unsupported(3, "[1, 2, 4]");
        assert_eq!(watched(), 2, "the SECOND site must report despite sharing the code");
        assert!(last_message().contains("[1, 2, 4]"), "the admitted set must be carried");

        // And each is `Once` on its own.
        report_ssaa_probe_refused(2, true, false, 1 << 30, 1 << 31);
        report_ssaa_scale_unsupported(3, "[1, 2, 4]");
        assert_eq!(watched(), 2, "neither site may report twice");
    }

    #[test]
    fn w3006_reports_every_degrade_reason_not_merely_the_first() {
        // The row is `Every` and this is why: the caller walks a SET. A latch here would report
        // one reason and drop the rest, and the reader would never learn the path degraded twice.
        let _lock = observe_lock();
        arm_app();
        watch(b'W', codes::W3006.number());
        report_render_path_degraded("MissingConsumer(VisibilityBuffer)");
        report_render_path_degraded("MissingCapability(MeshShader)");
        assert_eq!(watched(), 2, "a SET of reasons must produce a record per reason");
        assert!(last_message().contains("MissingCapability(MeshShader)"), "{}", last_message());
    }

    #[test]
    fn w3007_reports_the_geometry_table_failure_once() {
        let _lock = observe_lock();
        arm_app();
        W3007_SITE.reset();
        watch(b'W', codes::W3007.number());
        report_geometry_table_failed("OutOfDeviceMemory");
        report_geometry_table_failed("OutOfDeviceMemory");
        assert_eq!(watched(), 1, "`Once` per site: the table is built once per boot");
        assert!(last_message().contains("OutOfDeviceMemory"), "{}", last_message());
    }

    #[test]
    fn w3008_names_the_knob_and_the_reason_it_cannot_be_served() {
        let _lock = observe_lock();
        arm_app();
        W3008_SITE.reset();
        watch(b'W', codes::W3008.number());
        report_profiling_knob_unserviceable("BOYKO_VB_ZONE", "this device's timestamps are unusable");
        assert_eq!(watched(), 1);
        let msg = last_message();
        assert!(msg.contains("BOYKO_VB_ZONE"), "the knob must be named: {msg}");
        assert!(msg.contains("timestamps are unusable"), "the reason must be named: {msg}");
    }

    #[test]
    fn w3009_gives_each_variable_its_own_latch() {
        // The latch is the CALLER's for this code, so the test supplies two -- which is what the
        // two production sites do, and the property it protects is that a mistyped
        // `BOYKO_RENDER_PATH` cannot silence a mistyped `BOYKO_GEOMETRY_LEGS`.
        let _lock = observe_lock();
        arm_app();
        let path_site = codes::OnceSite::new();
        let legs_site = codes::OnceSite::new();

        watch(b'W', codes::W3009.number());
        report_unrecognized_env_value(&path_site, "BOYKO_RENDER_PATH", "forwardplu", "Deferred", "deferred|forward|forwardplus|vb");
        report_unrecognized_env_value(&legs_site, "BOYKO_GEOMETRY_LEGS", "mesg", "Both", "both|mesh|sdf");
        assert_eq!(watched(), 2, "two variables, two latches, two records");

        report_unrecognized_env_value(&path_site, "BOYKO_RENDER_PATH", "forwardplu", "Deferred", "deferred|forward|forwardplus|vb");
        assert_eq!(watched(), 2, "each variable reports once");

        let msg = last_message();
        assert!(msg.contains("mesg"), "the REJECTED value must be carried: {msg}");
        assert!(msg.contains("both|mesh|sdf"), "the admitted set must be carried: {msg}");
    }

    #[test]
    fn e3010_names_which_dump_failed_and_reports_every_one() {
        // One reporter for five sites, `Every`, with `kind` as the argument. A run that armed
        // three dumps and could write none of them has three things to report.
        let _lock = observe_lock();
        arm_host();
        watch(b'E', codes::E3010.number());
        report_dump_write_failed("HZB dump", "D:/out/hzb.bin", "PermissionDenied");
        report_dump_write_failed("census row", "D:/out/row.toml", "NotFound");
        assert_eq!(watched(), 2, "five sites share this code and none may silence another");
        let msg = last_message();
        assert!(msg.contains("census row"), "the KIND must distinguish the sites: {msg}");
        assert!(msg.contains("D:/out/row.toml"), "the path must be carried: {msg}");
        assert!(msg.contains("NotFound"), "the io error must be carried: {msg}");
    }
}
