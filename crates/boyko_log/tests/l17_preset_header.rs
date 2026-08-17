//! L17/J1: the two axes are independent, and the header says so in three separate facts.

use boyko_log::lifecycle::{DrainResult, SinkMode, boot, drain, enable};
use boyko_log::preset::{LogRuntimePreset, header};
use boyko_log::target::{LogTarget, TargetControl, set_target_control};
use boyko_log::{Level, Log};

#[test]
fn the_header_prints_three_independent_facts_and_the_presets_differ_where_the_table_says() {
    // ── THE TABLE IS A CLAIM ABOUT BEHAVIOUR, SO ASSERT THE BEHAVIOUR ────────────────────────
    //
    // `ShippingMin` is `Scheduled` and NOT `Manual`. `Manual` means no consumer at all, which
    // would make the crash file structurally contain the session's BEGINNING and nothing else --
    // the exact inversion of what a crash file is for.
    assert_eq!(LogRuntimePreset::ShippingMin.config().sink_mode, SinkMode::Scheduled);
    assert!(
        !LogRuntimePreset::ShippingMin.config().sink_thread,
        "shipping-min exists to have NO resident diagnostics thread; that is the whole purchase"
    );
    assert!(!LogRuntimePreset::Off.config().file, "`Off` opens no file");
    assert!(!LogRuntimePreset::Off.config().console, "`Off` configures nothing");
    assert!(LogRuntimePreset::Dev.config().console && LogRuntimePreset::Dev.config().file);
    assert!(!LogRuntimePreset::Dev.rotates(), "a bench or golden must not lose its own beginning");
    assert!(LogRuntimePreset::Editor.rotates(), "a long editor session must not grow forever");

    // Every preset has a distinct name: two sharing one would make the header ambiguous about the
    // single thing it exists to disambiguate.
    let names = [
        LogRuntimePreset::Dev.name(),
        LogRuntimePreset::Editor.name(),
        LogRuntimePreset::Shipping.name(),
        LogRuntimePreset::ShippingMin.name(),
        LogRuntimePreset::Off.name(),
    ];
    let mut sorted = names;
    sorted.sort_unstable();
    for pair in sorted.windows(2) {
        assert_ne!(pair[0], pair[1], "two presets share a name: {names:?}");
    }

    // ── THE HEADER: THREE FACTS, NOT ONE PROFILE NAME ────────────────────────────────────────
    let path = std::env::temp_dir().join("boyko_l17_header.log");
    let _ = std::fs::remove_file(&path);
    assert!(boyko_log::sink::file::set_path(path.to_str().expect("a UTF-8 temp path")));
    let mut cfg = LogRuntimePreset::Dev.config();
    // Manual and no thread, because this test drains by hand; the preset under test is named in
    // the HEADER, which is the subject -- not in the sink wiring, which is not.
    cfg.sink_mode = SinkMode::Manual;
    cfg.sink_thread = false;
    cfg.console = false;
    boot(cfg);
    assert!(enable(), "enable() refused a freshly booted process");
    set_target_control(<Log as LogTarget>::ID, TargetControl::new(Level::Trace, 0, false));
    boyko_log::sink::slot::reset();

    // A `shipping` BUILD running `Dev` is legal and ordinary; here the run-time axis says
    // `shipping-min` while the compile axis says whatever this build is. Both must be legible.
    header(LogRuntimePreset::ShippingMin);
    let DrainResult::Ran(_) = drain() else { panic!("the drain role is free in this process") };
    let text = std::fs::read_to_string(&path).expect("the sink's file is readable");

    assert!(text.contains("runtime_preset=shipping-min"), "the runtime axis is missing: {text:?}");
    assert!(
        text.contains(&format!("build_profile={}", boyko_diag::profile::PROFILE_NAME)),
        "the compile axis is missing: {text:?}"
    );
    assert!(text.contains("ceiling="), "the ceiling is missing: {text:?}");
    assert!(text.contains("session="), "the session id is missing: {text:?}");

    // The two axes must be SEPARATELY readable. A header that printed one merged "profile" would
    // send a reader to reason about whichever axis they assumed it meant -- and this run is
    // exactly the case that breaks: the preset is shipping-min and the build is not.
    assert!(
        !text.contains("profile=shipping-min"),
        "the runtime preset leaked into the build-profile field: {text:?}"
    );

    let _ = std::fs::remove_file(&path);
}
