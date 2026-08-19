//! L13b: the binary wire format round-trips, refuses truncation, and never reuses a site id.
//!
//! ⚠️ **The throughput claim this format exists for is UNPROVEN.** Decision 22 attaches a revert
//! clause: if `sink_sustained_rate_binary` does not measure **≥ 5× the text sink in the same
//! sitting**, L13b is reverted rather than justified. That bench needs the harness L10-C measured
//! as never built, so this file gates the FORMAT and says nothing about the speed.

use boyko_log::lifecycle::{DrainResult, LogConfig, SinkMode, boot, drain, enable};
use boyko_log::target::{LogTarget, TargetControl, set_target_control};
use boyko_log::{Level, Log};
use boyko_log::sink::binary::{
    FrameKind, RECORD_HEADER_BYTES, RecordFrame, SITE_DICT_LEN, decode_record, encode_record,
    intern_site, reset_site_dict, site_dict_used,
};

#[test]
fn a_record_frame_round_trips_byte_for_byte() {
    let payload = [1u8, 2, 3, 4, 5];
    let f = RecordFrame {
        site_id: 0xBEEF,
        tsc_delta: 0x0102_0304,
        flags: 0x5A,
        epoch_lo: 7,
        payload: &payload,
    };
    let mut buf = [0u8; 64];
    let n = encode_record(&mut buf, &f).expect("64 bytes is room for a 5-byte payload");
    assert_eq!(n, RECORD_HEADER_BYTES + payload.len());

    let (back, used) = decode_record(&buf).expect("what was just encoded must decode");
    assert_eq!(used, n, "the decoder must consume exactly what the encoder wrote");
    assert_eq!(back, f, "every field must survive the round trip, not merely the payload");
}

#[test]
fn a_truncated_frame_is_refused_rather_than_read_past() {
    let payload = [9u8; 32];
    let f = RecordFrame { site_id: 1, tsc_delta: 5, flags: 0, epoch_lo: 0, payload: &payload };
    let mut buf = [0u8; 64];
    let n = encode_record(&mut buf, &f).expect("room");

    // Every prefix shorter than the whole frame must be refused. A crashed run leaves exactly this
    // -- a file cut off mid-write -- and it is the run whose tail matters most, so a short read is
    // an ORDINARY outcome rather than a corrupt-input panic.
    for cut in 0..n {
        assert!(
            decode_record(&buf[..cut]).is_none(),
            "a {cut}-byte prefix of an {n}-byte frame decoded; the decoder read past its slice"
        );
    }
    assert!(decode_record(&buf[..n]).is_some(), "the complete frame must still decode");
}

#[test]
fn an_unknown_frame_kind_is_not_guessed_at() {
    let mut buf = [0u8; 32];
    buf[0] = 200; // a kind from a writer newer than this decoder
    assert_eq!(FrameKind::from_raw(200), None);
    assert!(
        decode_record(&buf).is_none(),
        "an unknown kind must be refused, never decoded as a Record -- a frame guessed at is a \
         record attributed to the wrong site"
    );
}

/// A distinct `LogSite` value; the dictionary keys on the ADDRESS, so every box is a new site.
fn probe_site() -> boyko_log::LogSite {
    boyko_log::LogSite {
        target: Some(<Log as LogTarget>::ID),
        level: Level::Info,
        class: 0,
        code: 0,
        line: 0,
        file: "l13b_binary_format.rs",
        fmt: "probe",
        fields: &[],
        prefix: "boyko",
        rate: boyko_log::codes::RatePolicy::Every,
    }
}

#[test]
fn a_full_site_dictionary_refuses_rather_than_reusing_an_id() {
    // THE SINK COMES UP FIRST, and that ordering is the test's own precondition. `W0116` is `Once`
    // PER PROCESS, so if the table is exhausted before a sink exists the latch is spent and the
    // observer reads an empty file -- which is exactly what the first draft of this block did.
    // A report that already fired is indistinguishable from one that never fires.
    let path = std::env::temp_dir().join("boyko_l13b_dict.log");
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
    set_target_control(<Log as LogTarget>::ID, TargetControl::new(Level::Trace, 0, false));

    reset_site_dict();

    // DISTINCT pointers, one per slot. The dictionary is keyed on the site ADDRESS, so filling it
    // needs `SITE_DICT_LEN` different sites.
    //
    // MEASURED: this loop used one null pointer `SITE_DICT_LEN` times and passed, because
    // `intern_site` ignored its argument and minted a fresh id per CALL. The test could only be
    // green while the thing it tested was absent -- it asserted "ids are unique" against an
    // implementation that could not do anything else, and would have gone red the moment the
    // dictionary started working. Which it did.
    let sites: Vec<Box<boyko_log::LogSite>> =
        (0..SITE_DICT_LEN).map(|_| Box::new(probe_site())).collect();

    let mut seen = std::collections::BTreeSet::new();
    for site in &sites {
        let ptr: *const boyko_log::LogSite = &**site;
        let (id, is_new) = intern_site(ptr).expect("the table holds SITE_DICT_LEN entries");
        assert!(is_new, "a site seen for the first time must be reported as new");
        assert!(seen.insert(id), "the dictionary handed out id {id} twice");
    }
    assert_eq!(site_dict_used(), SITE_DICT_LEN);

    // THE POINT OF A DICTIONARY: a repeat visit costs no id and is reported as NOT new, so the
    // caller writes no `SiteDef` frame. Without this the format is a file/line pair per record
    // wearing a dictionary's name.
    let first: *const boyko_log::LogSite = &*sites[0];
    let (again, is_new) = intern_site(first).expect("an interned site is always findable");
    assert!(!is_new, "a second visit to one site must not be reported as new");
    assert_eq!(again, 0, "a site's id must be stable across visits");
    assert_eq!(site_dict_used(), SITE_DICT_LEN, "a repeat visit consumed a slot");

    // THE PROPERTY. A reused id decodes a later record under an EARLIER site's file and line -- a
    // log that lies about where it came from, which is worse than one larger than it needed to be.
    // A site the table has never seen, so the refusal is exhaustion and not a repeat visit.
    let overflow = Box::new(probe_site());
    let site: *const boyko_log::LogSite = &*overflow;
    assert!(
        intern_site(site).is_none(),
        "a full dictionary must REFUSE, so the caller writes an inline site record and reports \
         W0116 -- never wrap into an id another site already owns"
    );
    // And refusing must not keep climbing: the census reports real occupancy, not attempts.
    assert_eq!(site_dict_used(), SITE_DICT_LEN, "a refused intern must not consume a slot");

    // ── W0116 REACHES A READER, ONCE ─────────────────────────────────────────────────────────
    //
    // The exhaustion above drives it; this reads it back off a real manual file sink rather than
    // inferring it from `intern_site` returning `None`. Several more refusals follow, so the count
    // is the claim: `Once` means the storm the report warns about is not made of the report.
    for _ in 0..5 {
        assert!(intern_site(site).is_none(), "the table is still full");
    }
    let DrainResult::Ran(_) = drain() else { panic!("the drain role is free in this process") };
    let text = std::fs::read_to_string(&path).expect("the sink's file is readable");
    let code = format!("boyko-W{:04}", boyko_log::codes::W0116.number());
    assert!(text.contains(&code), "a full dictionary emitted no {code}: {text:?}");
    assert_eq!(
        text.matches(&code).count(),
        1,
        "{code} is `Once` -- past a full table every later site writes inline, so one line stating          the stream grew is the fact; one per site would be a storm made of the very records it          warns about"
    );

    reset_site_dict();
}
