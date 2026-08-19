//! A rotated `.blog` generation is self-contained, and `logdec --merge` puts one session back
//! together while refusing to merge two.
//!
//! # What rotation breaks if it only moves bytes
//!
//! A decoder reads ONE file. Rolling the bytes away without re-emitting the header leaves the new
//! generation starting mid-stream: no anchor for its deltas to be relative to, and record frames
//! naming dictionary ids whose `Dictionary` frames left with the previous generation — so every
//! line decodes under some earlier site's file and line. That is the exact failure the per-FILE
//! dictionary rule already refuses at `open`, arriving through the other door.
//!
//! # Why the merge needs the session frame and not the file names
//!
//! Rotation leaves `foo.blog`, `foo.blog.1`, `foo.blog.2` in one directory, and so does yesterday's
//! run. Merging on names would interleave two sessions into one plausible timeline — plausible
//! being the whole problem, because nothing downstream could tell.

use boyko_log::lifecycle::{DrainResult, LogConfig, SinkMode, boot, drain, enable};
use boyko_log::sink::binary::{Frame, frames};
use boyko_log::sink::slot::{SLOT_CONSOLE, SLOT_ECS, SLOT_FILE, SinkState, reset, set_state};
use boyko_log::target::{LogTarget, TargetControl, set_target_control};
use boyko_log::{Level, Log, info};

/// Every frame kind present in one file, as a set of discriminant names.
fn kinds(bytes: &[u8]) -> Vec<&'static str> {
    frames(bytes)
        .map(|f| match f {
            Frame::Anchor { .. } => "anchor",
            Frame::Session { .. } => "session",
            Frame::Dictionary { .. } => "dictionary",
            Frame::Record(_) => "record",
            Frame::InlineSite { .. } => "inline",
        })
        .collect()
}

#[test]
fn a_rotated_generation_opens_with_its_own_header_and_merges_back_by_session() {
    let path = std::env::temp_dir().join("boyko_l13b_rot.blog");
    for n in 0..=3 {
        let _ = std::fs::remove_file(std::env::temp_dir().join(format!("boyko_l13b_rot.blog.{n}")));
    }
    let _ = std::fs::remove_file(&path);
    assert!(boyko_log::sink::binary::set_path(path.to_str().expect("a UTF-8 temp path")));
    // Small enough that a handful of records rolls it, large enough that a single frame never
    // straddles the cap -- the write happens first and the rotation after, so a frame is never cut.
    boyko_log::sink::binary::set_rotation(512, 2);

    boot(LogConfig {
        console: false,
        sink_thread: false,
        ecs_ring: false,
        file: false,
        binary: false,
        file_cap_bytes: 0,
        sink_mode: SinkMode::Manual,
    });
    assert!(enable(), "enable() refused a freshly booted process");
    set_target_control(<Log as LogTarget>::ID, TargetControl::new(Level::Trace, 0, false));
    reset();
    for slot in [SLOT_CONSOLE, SLOT_FILE, SLOT_ECS] {
        set_state(slot, SinkState::Off);
    }
    assert!(boyko_log::sink::binary::open(), "the temp path is openable");

    for i in 0..60u32 {
        info!(Log, "rotating record {} of sixty", i);
    }
    let DrainResult::Ran(_) = drain() else { panic!("the drain role is free in this process") };

    let (rotations, _lost) = boyko_log::sink::binary::rotation_state();
    assert!(rotations >= 1, "sixty records past a 512-byte cap must have rotated: {rotations}");

    // ── EVERY GENERATION IS SELF-CONTAINED ──────────────────────────────────────────────────
    //
    // Not just the live one. A generation that opened mid-stream would have records and no anchor,
    // which is precisely the file a reader keeps and cannot read.
    let mut generations = vec![path.clone()];
    for n in 1..=2 {
        let g = std::env::temp_dir().join(format!("boyko_l13b_rot.blog.{n}"));
        if g.exists() {
            generations.push(g);
        }
    }
    assert!(generations.len() >= 2, "rotation kept no previous generation: {generations:?}");

    for g in &generations {
        let bytes = std::fs::read(g).expect("a kept generation is readable");
        let k = kinds(&bytes);
        assert_eq!(
            k.first().copied(),
            Some("anchor"),
            "{} does not open with an anchor, so its deltas have no base: {k:?}",
            g.display()
        );
        assert!(
            k.contains(&"session"),
            "{} carries no session, so it cannot be proved to belong to this run: {k:?}",
            g.display()
        );
        // A record without a dictionary frame in the SAME file decodes under another file's site.
        if k.contains(&"record") {
            assert!(
                k.contains(&"dictionary"),
                "{} has records and no dictionary of its own: {k:?}",
                g.display()
            );
        }
    }

    // ── THE MERGE PUTS ONE SESSION BACK TOGETHER ────────────────────────────────────────────
    let mut cmd = std::process::Command::new(env!("CARGO_BIN_EXE_logdec"));
    cmd.arg("--merge");
    for g in &generations {
        cmd.arg(g);
    }
    let out = cmd.output().expect("logdec is built as part of this crate");
    assert!(out.status.success(), "merge failed: {}", String::from_utf8_lossy(&out.stderr));
    let text = String::from_utf8(out.stdout).expect("logdec prints UTF-8");
    assert!(text.contains("merged session"), "the merge printed no session header: {text}");
    assert!(
        text.contains("rotating record 59 of sixty"),
        "the last record did not survive the merge: {text}"
    );

    // Time order, asserted on the stamps the merge prints. Concatenating the files in name order
    // would also "work" here, and would silently be wrong the moment a generation's clock base
    // differed -- which is the whole reason the merge sorts on the ABSOLUTE tick.
    let stamps: Vec<u64> = text
        .lines()
        .filter_map(|l| l.split_whitespace().next())
        .filter_map(|n| n.parse::<u64>().ok())
        .collect();
    assert!(stamps.len() >= 2, "the merge listed fewer than two stamped records: {text}");
    assert!(stamps.windows(2).all(|w| w[0] <= w[1]), "the merged listing is not in time order");

    // ── AND IT REFUSES TWO SESSIONS ─────────────────────────────────────────────────────────
    //
    // A stale generation from an earlier run sits in the same directory under the same name. The
    // file NAMES cannot tell them apart; the session frame can, and this is the assertion that
    // makes the frame load-bearing rather than decorative.
    let alien = std::env::temp_dir().join("boyko_l13b_rot_alien.blog");
    let mut bytes = std::fs::read(&generations[0]).expect("readable");
    // Flip one bit of the session frame's low half. The frame is `[kind][lo:8][hi:8]`, and it is
    // the second frame in the file -- 17 bytes of anchor, then this.
    let session_lo = 17 + 1;
    bytes[session_lo] ^= 0x01;
    std::fs::write(&alien, &bytes).expect("the alien file is writable");

    let out = std::process::Command::new(env!("CARGO_BIN_EXE_logdec"))
        .arg("--merge")
        .arg(&generations[0])
        .arg(&alien)
        .output()
        .expect("logdec runs");
    assert!(
        !out.status.success(),
        "logdec merged two DIFFERENT sessions into one timeline; the session frame is decorative"
    );
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("different SessionId"), "the refusal does not say why: {err}");

    boyko_log::sink::binary::set_rotation(0, 0);
    boyko_log::sink::slot::reset();
    let _ = std::fs::remove_file(&alien);
    for g in &generations {
        let _ = std::fs::remove_file(g);
    }
}
