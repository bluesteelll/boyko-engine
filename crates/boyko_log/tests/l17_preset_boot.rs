//! `boot_preset` applies the WHOLE table row, and `enable` prints the header.
//!
//! # What was missing, and why every earlier gate stayed green on it
//!
//! `preset.rs` shipped with a five-row table, a `config()`, a `rotates()` and a `header()`. Nothing
//! called any of them outside a test. So:
//!
//! * **`enable()` never opened the binary sink** — `LogConfig` had no field for it. The `.blog`
//!   format had a writer, a dictionary, a destination and an offline decoder, and no production
//!   path that would produce one.
//! * **`header()` had no caller**, while `05-LADDER-GATES.md` reported L17/J1 as half-shipped
//!   pending "the three header facts". The function existed and printed nothing.
//! * **`rotates()` had no caller**, which its own doc comment had warned about in advance: "a
//!   preset that claimed to rotate without calling `set_rotation` would be a table that describes
//!   a behaviour nobody implements".
//!
//! This test drives the production path — `boot_preset` then `enable` — and reads the result back
//! through `logdec`. Nothing here calls `header` or `binary::open` by hand, which is the point: the
//! previous test could do that and pass while the shipped path did neither.

use boyko_log::lifecycle::{
    LogConfig, SinkMode, boot_preset, boot_preset_recorded, enable, flush, shutdown,
};
use boyko_log::preset::LogRuntimePreset;
use boyko_log::target::{LogTarget, TargetControl, set_target_control};
use boyko_log::{Level, Log, info};

#[test]
fn shipping_opens_the_binary_sink_and_the_header_is_in_the_file() {
    let blog = std::env::temp_dir().join("boyko_l17_boot.blog");
    let text = std::env::temp_dir().join("boyko_l17_boot.log");
    let _ = std::fs::remove_file(&blog);
    let _ = std::fs::remove_file(&text);

    // ── THE TABLE, ASSERTED BEFORE IT IS APPLIED ────────────────────────────────────────────
    //
    // `Shipping` reads "binary file" in the table. It used to read "binary + crash" while
    // `config()` set `file: true` and no binary at all -- three claims, no two of which agreed.
    let cfg = LogRuntimePreset::Shipping.config();
    assert!(cfg.binary, "the `Shipping` row says binary; its config must open one");
    assert!(!cfg.file, "`Shipping` opens no TEXT file, whatever the row used to say");
    assert!(!cfg.console, "a released title writes no console");
    assert!(LogRuntimePreset::Shipping.rotates(), "a released title's file must not grow forever");
    // Rotation must reach the sink the row actually writes to. The first draft of `boot_preset`
    // set it on the TEXT sink only, so `Shipping` -- whose destination is the binary file --
    // claimed rotation in the table and rotated nothing.
    assert_eq!(
        boyko_log::sink::binary::rotation_state(),
        (0, 0),
        "the binary sink has not rotated before this test boots"
    );

    // ── THE PRODUCTION PATH, WITH NOTHING OPENED BY HAND ────────────────────────────────────
    boot_preset(
        LogRuntimePreset::Shipping,
        Some(text.to_str().expect("a UTF-8 temp path")),
        Some(blog.to_str().expect("a UTF-8 temp path")),
    );
    assert_eq!(
        boot_preset_recorded(),
        Some(LogRuntimePreset::Shipping),
        "the preset must be recorded, or the header cannot name it"
    );
    assert!(enable(), "enable() refused a freshly booted process");
    set_target_control(<Log as LogTarget>::ID, TargetControl::new(Level::Trace, 0, false));
    info!(Log, "a record after the header {}", 9u32);

    // `Shipping` runs a resident sink thread, so the drain is not this thread's. `flush` is what a
    // host calls; using it here means the test exercises the shipped shape rather than a manual
    // drain the preset does not specify.
    flush();
    shutdown();

    // ── THE BINARY FILE EXISTS AND THE TEXT ONE DOES NOT ────────────────────────────────────
    assert!(blog.exists(), "`Shipping` did not open the binary sink `enable` was supposed to open");
    assert!(
        boyko_log::sink::binary::rotation_cap() > 0,
        "`Shipping` rotates by its own table row, and the cap must reach the BINARY sink -- the \
         one this preset writes to"
    );
    assert!(
        !text.exists(),
        "`Shipping` opened a TEXT file; the row says binary, and two destinations for one row is \
         how a reader stops knowing which file a record went to"
    );

    // ── AND THE HEADER IS IN IT, DECODED BY THE SHIPPED TOOL ────────────────────────────────
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_logdec"))
        .arg(&blog)
        .output()
        .expect("logdec is built as part of this crate");
    let decoded = String::from_utf8(out.stdout).expect("logdec prints UTF-8");

    assert!(
        decoded.contains("runtime_preset=shipping"),
        "the header did not name the preset `boot_preset` recorded: {decoded}"
    );
    assert!(
        decoded.contains(&format!("build_profile={}", boyko_diag::profile::PROFILE_NAME)),
        "the compile axis is missing from the header: {decoded}"
    );
    assert!(decoded.contains("ceiling="), "the ceiling is missing from the header: {decoded}");
    assert!(decoded.contains("session="), "the session id is missing from the header: {decoded}");

    // ── THE HEADER'S ID AND THE FILE'S OWN SESSION FRAME ARE THE SAME STRING ────────────────
    //
    // The id exists so an uploaded log and an uploaded profiling artifact can be proved to be one
    // run. Two representations that cannot be compared defeat exactly that.
    //
    // MEASURED before this assertion existed: the frame read `fe63ff00ba2f385d7ff64cce15042cc5`
    // and the header beside it read `183307752869167903659220641735087238341`. The header used
    // `session={:x}{:x}`, and `record::render_payload` consumes a value for any `{…}` group and
    // IGNORES the format spec -- so the two halves printed in DECIMAL, glued end to end, into a
    // number that is the id in no base at all.
    let frame_id = decoded
        .lines()
        .find_map(|l| l.strip_prefix("-- session "))
        .map(str::trim)
        .unwrap_or_else(|| panic!("the file carries no session frame: {decoded}"));
    let header_id = decoded
        .lines()
        .find_map(|l| l.split("session=").nth(1))
        .map(str::trim)
        .unwrap_or_else(|| panic!("the header carries no session: {decoded}"));
    assert_eq!(
        frame_id, header_id,
        "the file's session frame and the header's id are different strings, so nothing can correlate this log with a profiling artifact from the same run"
    );
    assert!(
        decoded.contains("a record after the header 9"),
        "ordinary records must follow the header into the same file: {decoded}"
    );

    // ── THE ANCHOR'S SCALE IS THE CALIBRATED ONE ────────────────────────────────────────────
    //
    // MEASURED, and it is why this assertion exists at all: `enable` opened the sinks BEFORE
    // `clock::calibrate`, so the anchor stamped the uncalibrated `1.0` into the file and `logdec`
    // reported a record 0.2 ms after open as `+85.215ms`. Moving the calibration fixed it -- and
    // NOTHING GATED THE ORDER. Reverting the fix left every test in this crate green, because the
    // only other test that writes a `.blog` opens the sink by hand AFTER `enable` has already
    // calibrated.
    //
    // Compared against the live value rather than against a literal: `ticks_per_ns` is ~3.29 on
    // this box and exactly 1.0 on any target whose clock backend is an `Instant` delta, so a
    // `!= 1.0` assertion would be a fact about x86-64 wearing a correctness claim.
    let scale: f64 = decoded
        .lines()
        .find_map(|l| l.split("ticks_per_ns=").nth(1))
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or_else(|| panic!("no anchor scale in the decode: {decoded}"));
    let live = boyko_diag::clock::ticks_per_ns();
    assert!(
        (scale - live).abs() < 1e-9,
        "the file's anchor carries {scale} and the calibrated clock reads {live}: the sink was \
         opened before `calibrate`, so every stamp in this file is scaled by the wrong number"
    );

    // ── A HAND-BUILT CONFIG IS `custom`, NOT A PRESET NAME ──────────────────────────────────
    //
    // Asserted on the function rather than through a second boot, because a process boots once.
    // The claim is narrow and worth pinning: a host that assembled its own `LogConfig` selected no
    // preset, and printing `runtime_preset=dev` for it would name an axis nobody chose.
    let _unused: LogConfig = LogRuntimePreset::Off.config();
    assert_eq!(
        LogRuntimePreset::from_raw(0),
        None,
        "the zero byte must decode to NO preset, or a hand-built config inherits one"
    );
    assert_eq!(LogRuntimePreset::from_raw(3), Some(LogRuntimePreset::Shipping));
    assert_eq!(LogRuntimePreset::Off.config().sink_mode, SinkMode::Manual);

    let _ = std::fs::remove_file(&blog);
    let _ = std::fs::remove_file(&text);
}
