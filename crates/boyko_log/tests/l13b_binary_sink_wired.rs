//! The binary sink writes a real file, and the file decodes back to the records that went in.
//!
//! Until this test the format existed and nothing wrote it: `encode_record`, the site dictionary
//! and `W0116` were all reachable only from tests. A format with no writer is the campaign's
//! signature defect wearing a shipped rung's name.

use boyko_log::lifecycle::{DrainResult, LogConfig, SinkMode, boot, drain, enable};
use boyko_log::sink::binary::{Frame, frames, frames_written, site_dict_used};
use boyko_log::sink::slot::{SLOT_BINARY, SLOT_CONSOLE, SLOT_ECS, SLOT_FILE, SinkState, reset, set_state};
use boyko_log::target::{LogTarget, TargetControl, set_target_control};
use boyko_log::{Level, Log, info};

// THE WALKER IS THE LIBRARY'S, NOT THIS FILE'S, AND THAT SWAP IS THE POINT.
//
// This test used to carry its own frame walker. It passed, and it proved a decoder that no tool
// used -- while `logdec`, the decoder a reader actually needs, did not exist. A private copy goes
// on passing after the shipped decoder breaks, which is the vacuous gate this campaign removes.
// `boyko_log::sink::binary::frames` is now the ONE walker: this test, `logdec` and any third
// consumer read the format through it.

#[test]
fn the_binary_sink_writes_a_file_that_decodes_to_the_records_that_went_in() {
    let path = std::env::temp_dir().join("boyko_l13b_wired.blog");
    let _ = std::fs::remove_file(&path);
    assert!(boyko_log::sink::binary::set_path(path.to_str().expect("a UTF-8 temp path")));
    assert!(boyko_log::sink::binary::path_recorded());

    boot(LogConfig {
        console: false,
        sink_thread: false,
        ecs_ring: false,
        file: false,
        file_cap_bytes: 0,
        sink_mode: SinkMode::Manual,
    });
    assert!(enable(), "enable() refused a freshly booted process");
    set_target_control(<Log as LogTarget>::ID, TargetControl::new(Level::Trace, 0, false));

    // ONLY the binary sink is Active. If the text sink were also on, a green test could be green
    // because of the OTHER destination -- and the subject here is the one that never had a writer.
    reset();
    for slot in [SLOT_CONSOLE, SLOT_FILE, SLOT_ECS] {
        set_state(slot, SinkState::Off);
    }
    assert_eq!(
        boyko_log::sink::slot::state(SLOT_BINARY),
        SinkState::Active,
        "the binary slot must be Active or this test would be green because nothing was admitted"
    );
    assert!(boyko_log::sink::binary::open(), "the temp path is openable");

    // Two records from ONE site and one from another: the dictionary's whole purpose is that the
    // repeat costs no `Dictionary` frame, and a single-record test could not show that.
    for i in 0..2u32 {
        info!(Log, "same site {}", i);
    }
    info!(Log, "other site {}", 7u32);
    let DrainResult::Ran(_) = drain() else { panic!("the drain role is free in this process") };

    let bytes = std::fs::read(&path).expect("the binary sink created its file");
    assert!(!bytes.is_empty(), "the sink is wired but wrote nothing");
    let mut walk = frames(&bytes);
    let fr: Vec<Frame<'_>> = walk.by_ref().collect();
    // NOT A RAGGED TAIL. The walker stops at the first frame that does not decode, so a file whose
    // last frame is truncated yields a short list and no error -- and every assertion below would
    // then be about a prefix. Comparing consumed bytes against the file length is what tells the
    // two apart, and it has to be asserted rather than assumed.
    assert_eq!(
        walk.consumed(),
        bytes.len(),
        "the walker stopped early: {} of {} bytes decoded, so everything below is about a prefix",
        walk.consumed(),
        bytes.len()
    );

    // ── THE FILE OPENS WITH AN ANCHOR ────────────────────────────────────────────────────────
    //
    // A file whose first frame is a record has no absolute time to add its deltas to: it decodes
    // to a session that started at zero, which is worse than one that refuses to decode.
    assert!(
        matches!(fr[0], Frame::Anchor { .. }),
        "the file does not open with an anchor: {fr:?}"
    );

    // ── ONE DICTIONARY FRAME PER SITE, NOT PER RECORD ────────────────────────────────────────
    let dicts = fr.iter().filter(|f| matches!(f, Frame::Dictionary { .. })).count();
    let records = fr.iter().filter(|f| matches!(f, Frame::Record(_))).count();
    assert_eq!(records, 3, "three records went in: {fr:?}");
    assert_eq!(
        dicts, 2,
        "two distinct sites emitted, so two dictionary frames -- one per RECORD would make the \
         dictionary a file/line pair per record wearing a dictionary's name: {fr:?}"
    );
    assert_eq!(site_dict_used(), 2, "the dictionary holds one entry per site");

    // ── THE DICTIONARY FRAME NAMES THIS FILE ─────────────────────────────────────────────────
    let named: Vec<&str> = fr
        .iter()
        .filter_map(|f| match f {
            Frame::Dictionary { file, .. } => Some(*file),
            _ => None,
        })
        .collect();
    assert!(
        named.iter().all(|f| f.contains("l13b_binary_sink_wired")),
        "a dictionary frame does not carry this test's file: {named:?}"
    );

    // ── AND IT CARRIES THE FORMAT LITERAL, WHICH IS WHAT MAKES A RECORD RENDERABLE ───────────
    //
    // A dictionary that named the file and line but not the format would locate a record and leave
    // its arguments as raw tags -- exactly the hole the INLINE frame turned out to have.
    let fmts: Vec<&str> = fr
        .iter()
        .filter_map(|f| match f {
            Frame::Dictionary { fmt, .. } => Some(*fmt),
            _ => None,
        })
        .collect();
    assert!(
        fmts.iter().any(|f| f.contains("same site")) && fmts.iter().any(|f| f.contains("other site")),
        "the dictionary frames do not carry both format literals: {fmts:?}"
    );

    // ── THE PAYLOAD SURVIVES VERBATIM ────────────────────────────────────────────────────────
    //
    // The binary sink writes the ring's bytes without rendering them, so the argument values must
    // be findable in the frames. `7u32` little-endian is the third record's only argument.
    let bodies: Vec<&[u8]> = fr
        .iter()
        .filter_map(|f| match f {
            Frame::Record(r) => Some(r.payload),
            _ => None,
        })
        .collect();
    assert!(
        bodies.iter().any(|b| b.windows(4).any(|w| w == 7u32.to_le_bytes())),
        "the record payload did not survive into the file"
    );

    assert!(frames_written() >= 6, "anchor + 2 dictionaries + 3 records is six frames at least");

    boyko_log::sink::slot::reset();
    let _ = std::fs::remove_file(&path);
}
