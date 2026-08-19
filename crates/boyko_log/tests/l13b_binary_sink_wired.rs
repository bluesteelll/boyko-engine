//! The binary sink writes a real file, and the file decodes back to the records that went in.
//!
//! Until this test the format existed and nothing wrote it: `encode_record`, the site dictionary
//! and `W0116` were all reachable only from tests. A format with no writer is the campaign's
//! signature defect wearing a shipped rung's name.

use boyko_log::lifecycle::{DrainResult, LogConfig, SinkMode, boot, drain, enable};
use boyko_log::sink::binary::{FrameKind, RECORD_HEADER_BYTES, frames_written, site_dict_used};
use boyko_log::sink::slot::{SLOT_BINARY, SLOT_CONSOLE, SLOT_ECS, SLOT_FILE, SinkState, reset, set_state};
use boyko_log::target::{LogTarget, TargetControl, set_target_control};
use boyko_log::{Level, Log, info};

/// Walk the frames a file holds, returning `(kind, payload-or-body)` pairs in order.
///
/// Written here rather than reusing `decode_record` alone, because the file carries FOUR kinds and
/// a test that could only read one of them would pass on a file that was mostly unreadable.
fn frames(bytes: &[u8]) -> Vec<(FrameKind, Vec<u8>)> {
    let mut out = Vec::new();
    let mut at = 0usize;
    while at < bytes.len() {
        let Some(kind) = FrameKind::from_raw(bytes[at]) else {
            panic!("unknown frame kind {} at offset {at}", bytes[at]);
        };
        match kind {
            FrameKind::Anchor => {
                assert!(at + 9 <= bytes.len(), "a truncated anchor at {at}");
                out.push((kind, bytes[at + 1..at + 9].to_vec()));
                at += 9;
            }
            FrameKind::Dictionary => {
                let flen = u16::from_le_bytes([bytes[at + 7], bytes[at + 8]]) as usize;
                let fmt_at = at + 9 + flen;
                let mlen = u16::from_le_bytes([bytes[fmt_at], bytes[fmt_at + 1]]) as usize;
                let end = fmt_at + 2 + mlen;
                out.push((kind, bytes[at + 9..at + 9 + flen].to_vec()));
                at = end;
            }
            FrameKind::Record => {
                let len = u16::from_le_bytes([bytes[at + 7], bytes[at + 8]]) as usize;
                let end = at + RECORD_HEADER_BYTES + len;
                assert!(end <= bytes.len(), "a truncated record at {at}");
                out.push((kind, bytes[at + RECORD_HEADER_BYTES..end].to_vec()));
                at = end;
            }
            FrameKind::InlineSite => {
                let flen = u16::from_le_bytes([bytes[at + 5], bytes[at + 6]]) as usize;
                let plen_at = at + 7 + flen;
                let plen = u16::from_le_bytes([bytes[plen_at], bytes[plen_at + 1]]) as usize;
                let end = plen_at + 2 + plen;
                out.push((kind, bytes[plen_at + 2..end].to_vec()));
                at = end;
            }
        }
    }
    out
}

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
    let fr = frames(&bytes);

    // ── THE FILE OPENS WITH AN ANCHOR ────────────────────────────────────────────────────────
    //
    // A file whose first frame is a record has no absolute time to add its deltas to: it decodes
    // to a session that started at zero, which is worse than one that refuses to decode.
    assert_eq!(fr[0].0, FrameKind::Anchor, "the file does not open with an anchor: {fr:?}");

    // ── ONE DICTIONARY FRAME PER SITE, NOT PER RECORD ────────────────────────────────────────
    let dicts = fr.iter().filter(|(k, _)| *k == FrameKind::Dictionary).count();
    let records = fr.iter().filter(|(k, _)| *k == FrameKind::Record).count();
    assert_eq!(records, 3, "three records went in: {fr:?}");
    assert_eq!(
        dicts, 2,
        "two distinct sites emitted, so two dictionary frames -- one per RECORD would make the \
         dictionary a file/line pair per record wearing a dictionary's name: {fr:?}"
    );
    assert_eq!(site_dict_used(), 2, "the dictionary holds one entry per site");

    // ── THE DICTIONARY FRAME NAMES THIS FILE ─────────────────────────────────────────────────
    let named: Vec<String> =
        fr.iter().filter(|(k, _)| *k == FrameKind::Dictionary).map(|(_, b)| String::from_utf8_lossy(b).into_owned()).collect();
    assert!(
        named.iter().all(|f| f.contains("l13b_binary_sink_wired")),
        "a dictionary frame does not carry this test's file: {named:?}"
    );

    // ── THE PAYLOAD SURVIVES VERBATIM ────────────────────────────────────────────────────────
    //
    // The binary sink writes the ring's bytes without rendering them, so the argument values must
    // be findable in the frames. `7u32` little-endian is the third record's only argument.
    let bodies: Vec<&Vec<u8>> = fr.iter().filter(|(k, _)| *k == FrameKind::Record).map(|(_, b)| b).collect();
    assert!(
        bodies.iter().any(|b| b.windows(4).any(|w| w == 7u32.to_le_bytes())),
        "the record payload did not survive into the file"
    );

    assert!(frames_written() >= 6, "anchor + 2 dictionaries + 3 records is six frames at least");

    boyko_log::sink::slot::reset();
    let _ = std::fs::remove_file(&path);
}
