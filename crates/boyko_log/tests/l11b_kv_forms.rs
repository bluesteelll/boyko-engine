//! L11b's `*_kv!` field-name forms, read back off a real sink.
//!
//! # Why this test asserts the RENDERED LINE and not the site
//!
//! `LogSite.fields` has existed since L1 as a `&'static [&'static str]`, written `&[]` by every
//! expansion and **read by nothing**. Asserting that a `*_kv!` site now carries names would repeat
//! that mistake one level up: the names would be present and still reach no reader. So the claim
//! under test is what a person opening the log file sees.
//!
//! This is the same correction L6 made when it found `site.decode` holding a placeholder no drain
//! ever called, and the same one `check_5` states as "NAMING IS A PROXY FOR OBSERVING".

use boyko_log::lifecycle::{DrainResult, LogConfig, SinkMode, boot, drain, enable};
use boyko_log::target::{LogTarget, TargetControl, set_target_control};
use boyko_log::{Ecs, Level};

#[test]
fn the_kv_forms_render_name_equals_value_in_declaration_order() {
    let path = std::env::temp_dir().join("boyko_l11b_kv.log");
    let _ = std::fs::remove_file(&path);
    assert!(boyko_log::sink::file::set_path(path.to_str().expect("a UTF-8 temp path")));
    boot(LogConfig {
        console: false,
        sink_thread: false,
        ecs_ring: false,
        file: true,
        file_cap_bytes: 0,
        sink_mode: SinkMode::Manual,
    });
    assert!(enable(), "enable() refused a freshly booted process");
    set_target_control(<Ecs as LogTarget>::ID, TargetControl::new(Level::Trace, 0, false));

    boyko_log::info_kv!(Ecs, "hit", dmg = 12u32, target = 7u32);
    boyko_log::warn_kv!(Ecs, boyko_log::codes::W0103, "cap", written = 4096u64);
    // A positional site in the same run: the two forms must not disturb each other, and the
    // positional renderer must still interleave into its format literal.
    boyko_log::info!(Ecs, "positional {} and {}", 1u32, 2u32);

    let DrainResult::Ran(_) = drain() else { panic!("the drain role is free in this process") };
    let text = std::fs::read_to_string(&path).expect("the sink's file is readable");

    // ── the names reach the READER, in declaration order ────────────────────────────────────
    assert!(
        text.contains("hit dmg=12 target=7"),
        "the kv form did not render `name=value` in order -- if the values are present but the \
         names are not, `LogSite.fields` still has no reader: {text:?}"
    );
    assert!(text.contains("cap written=4096"), "the warn kv form did not render: {text:?}");

    // ── and the code still prints, so kv is a rendering choice and not a second record shape ──
    assert!(text.contains("boyko-W0103"), "a kv warn must still carry its code: {text:?}");

    // ── the positional form is untouched ────────────────────────────────────────────────────
    assert!(
        text.contains("positional 1 and 2"),
        "the kv branch must not change how a positional site renders: {text:?}"
    );
}
