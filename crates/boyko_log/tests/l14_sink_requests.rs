//! L14: sink control is posted, never executed on the caller's thread, and a full ring says so.

use boyko_log::lifecycle::{DrainResult, LogConfig, SinkMode, boot, drain, enable};
use boyko_log::sink::request::{
    SINK_REQ_LEN, SinkReq, SinkVerb, clear, depth, post, report_refused, take,
};
use boyko_log::target::{LogTarget, TargetControl, set_target_control};
use boyko_log::{Level, Log};

#[test]
fn requests_round_trip_in_order_and_a_full_ring_is_refused_not_dropped() {
    clear();
    assert_eq!(depth(), 0);

    // ── FIFO: two operators' commands must not be reordered ─────────────────────────────────
    for slot in 0..3u8 {
        post(SinkReq { slot, verb: SinkVerb::OpenFile }).expect("a fresh ring has room");
    }
    assert_eq!(depth(), 3);
    for slot in 0..3u8 {
        let r = take().expect("three were posted");
        assert_eq!(r.slot, slot, "requests must drain in the order they were posted");
        assert_eq!(r.verb, SinkVerb::OpenFile);
    }
    assert_eq!(take(), None, "an empty ring yields None, not a stale entry");

    // ── A FULL RING REFUSES. It does not drop, and it does not overwrite ────────────────────
    clear();
    for slot in 0..SINK_REQ_LEN {
        post(SinkReq { slot: slot as u8, verb: SinkVerb::CloseFile }).expect("within capacity");
    }
    assert_eq!(depth(), SINK_REQ_LEN);
    assert!(
        post(SinkReq { slot: 99, verb: SinkVerb::ApplyControl }).is_err(),
        "a full ring must REFUSE -- dropping silently leaves an operator typing a command and \
         seeing nothing happen, with the log they are opening being where the explanation went"
    );

    // And the refusal did not corrupt what was already queued: the first entry is still slot 0.
    let first = take().expect("the queued requests survive a refusal");
    assert_eq!(first.slot, 0, "a refused post overwrote a queued request");
    assert_eq!(first.verb, SinkVerb::CloseFile);

    // The block above drained ONE entry and left fifteen. Refilling without clearing overflows at
    // the first post -- which is the ring behaving correctly and the test carrying state it did not
    // mean to. Cleared explicitly, because a shared fixture's leftovers are somebody else's bug.
    clear();
    assert_eq!(depth(), 0, "the ring must be empty before the next claim is set up");

    // ── E0107 REACHES A READER, naming the VERB and the slot ────────────────────────────────
    //
    // The sink comes up BEFORE the ring is refilled: `E0107` is `Every`, but a report emitted
    // before a sink exists still reaches nobody -- the ordering mistake `W0116`'s observer made.
    let path = std::env::temp_dir().join("boyko_l14_reqs.log");
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

    for slot in 0..SINK_REQ_LEN {
        post(SinkReq { slot: slot as u8, verb: SinkVerb::CloseFile }).expect("within capacity");
    }
    let verb = SinkVerb::ApplyControl;
    assert!(post(SinkReq { slot: 7, verb }).is_err(), "the ring is full again");
    report_refused(verb, 7);

    let DrainResult::Ran(_) = drain() else { panic!("the drain role is free in this process") };
    let text = std::fs::read_to_string(&path).expect("the sink's file is readable");
    let code = format!("boyko-E{:04}", boyko_log::codes::E0107.number());
    assert!(text.contains(&code), "a refused request emitted no {code}: {text:?}");
    assert!(
        text.contains(verb.name()),
        "E0107 must name the VERB -- 'a request was dropped' does not tell an operator which          command to retype: {text:?}"
    );

    clear();
    assert_eq!(depth(), 0);
}

#[test]
fn every_verb_has_a_name_and_the_names_are_distinct() {
    // The name is what reaches a reader; two verbs sharing one would make `E0107` ambiguous about
    // which command was refused, which is the one thing the report exists to say.
    let names = [
        SinkVerb::OpenFile.name(),
        SinkVerb::CloseFile.name(),
        SinkVerb::ApplyControl.name(),
    ];
    let mut sorted = names;
    sorted.sort_unstable();
    sorted.iter().zip(sorted.iter().skip(1)).for_each(|(a, b)| {
        assert_ne!(a, b, "two sink verbs share a name: {names:?}");
    });
    assert!(names.iter().all(|n| !n.is_empty()));
}
