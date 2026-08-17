//! L10's dynamic band: interning, idempotency, exhaustion, and the publication order.
//!
//! **Its own integration binary, and that is forced rather than tidy.** `DYN_NAMES` is
//! process-global, insert-only and never released — 32 slots for the life of the process. A test
//! that fills the band cannot un-fill it, so a sibling in the same binary that registered
//! afterwards would fail for a reason that has nothing to do with its own claim. This is the same
//! shape `l6_query_table_exhaustion.rs` already carries, for the same reason.
//!
//! Ordered by dependence: the first test needs an empty band, and the exhaustion test consumes it.
//!
//! It is also **`boyko-E0106`'s observer**, and that is not an extra: the three refusals are driven
//! here anyway, and a code whose emission nothing reads is the shape this campaign keeps finding
//! (two `92xx` reporters shipped for three rungs unable to emit at all). So the band is registered
//! against a real manual file sink and the refusals are read back off disk, not inferred from the
//! `None`.

use boyko_log::codes::E0106;
use boyko_log::lifecycle::{DrainResult, LogConfig, SinkMode, boot, drain, enable, shutdown};
use boyko_log::target::{
    DYN_BAND_LEN, DYN_BAND_START, LogTarget, MAX_DYN_NAME, MAX_TARGETS, TargetControl,
    dyn_registered, find_target, register_dynamic_target, set_target_control, targets,
};
use boyko_log::{Level, Log, target_control};

/// Everything written to the sink so far.
///
/// Re-read rather than accumulated, because the claim is about what a reader of the FILE sees.
fn sink_text(path: &std::path::Path) -> String {
    let DrainResult::Ran(_) = drain() else { panic!("the drain role is free in this process") };
    std::fs::read_to_string(path).expect("the sink's file is readable")
}

/// The whole of L10's contract, in the order the band's state allows.
///
/// **One `#[test]`, deliberately.** `cargo test` runs a binary's tests concurrently, and every
/// claim below reads or writes one process-global table; splitting them would make the suite's
/// verdict depend on scheduling. The alternative — a `Mutex` — would serialise them into this same
/// sequence while suggesting they were independent.
#[test]
fn the_dynamic_band_interns_idempotently_and_then_refuses() {
    // ── a fresh band, and a sink that can hear E0106 ──────────────────────────────────────────
    assert_eq!(dyn_registered(), 0, "this binary must own an untouched band");
    assert_eq!(find_target("mod:acme"), None, "nothing is registered yet");

    let path = std::env::temp_dir().join("boyko_l10_dyn_targets.log");
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
    // `E0106` emits on the ENGINE `log` row, not on any dynamic one -- the whole point being that a
    // failed registration has no target of its own to report through. Opened explicitly because a
    // default run leaves every ceiling `Off`, which is exactly why this is a test and not a run.
    set_target_control(<Log as LogTarget>::ID, TargetControl::new(Level::Trace, 0, false));

    // ── registration mints an id IN BAND and applies the initial control ──────────────────────
    let ctl = TargetControl::new(Level::Debug, 2, false);
    let id = register_dynamic_target("mod:acme", ctl).expect("a fresh band has room");
    assert!(
        (id.index() as usize) >= DYN_BAND_START && (id.index() as usize) < MAX_TARGETS,
        "a dynamic id must land in 224..=255, got {}",
        id.index()
    );
    assert_eq!(target_control(id), ctl, "the initial control must be applied at registration");
    assert_eq!(dyn_registered(), 1);

    // ── IDEMPOTENT BY NAME, and the second caller does not re-open the target ─────────────────
    //
    // The second half is the one worth asserting: two mods naming one category must agree on the
    // id, and the later one must not be able to raise a ceiling the earlier one chose. A
    // registration that silently re-applied `initial` would make load order decide the level.
    let again = register_dynamic_target("mod:acme", TargetControl::new(Level::Trace, 0, true))
        .expect("re-registration returns the same slot");
    assert_eq!(again, id, "registration is idempotent by NAME");
    assert_eq!(
        target_control(id),
        ctl,
        "a second registration must NOT re-open a target the first one configured"
    );
    assert_eq!(dyn_registered(), 1, "idempotent registration consumes no second slot");

    // ── find_target sees both bands ──────────────────────────────────────────────────────────
    assert_eq!(find_target("mod:acme"), Some(id));
    assert_eq!(
        find_target("ecs").map(|t| t.index()),
        Some(0),
        "an engine name resolves too -- a console user does not know which band a target is in"
    );
    assert_eq!(find_target("mod:nope"), None);

    // ── targets() lists engine rows and REGISTERED dynamic ones, never blank slots ────────────
    let listed: Vec<&str> = targets().map(|(_, n)| n).collect();
    assert!(listed.contains(&"mod:acme"), "a registered dynamic target must be listed");
    assert!(listed.contains(&"ecs"), "engine targets are listed too");
    // DERIVED, not hardcoded. The first draft of this line said `26 + 1` and reddened at once: the
    // engine table had gained a 27th row (`Demo`, at L8b) since the number was written. A test that
    // pins the size of a table other rungs are expected to extend reds for a reason that has
    // nothing to do with its own claim — and the claim here is about the DYNAMIC half.
    let engine_rows = boyko_log::target::engine_targets().count();
    assert_eq!(
        listed.len(),
        engine_rows + 1,
        "every engine row plus exactly one registered dynamic; an unregistered slot must be \
         ABSENT rather than listed blank"
    );

    // Nothing has been refused yet, so the sink must hold no E0106. A negative control, and it is
    // load-bearing: without it every `contains` below is satisfied by a code that was already there.
    let code = format!("boyko-E{:04}", E0106.number());
    assert!(
        !sink_text(&path).contains(&code),
        "{code} is in the log before anything was refused"
    );

    // ── the two refusals that are caller errors, not exhaustion ──────────────────────────────
    assert_eq!(register_dynamic_target("", ctl), None, "an empty name cannot claim a slot");
    let too_long = "x".repeat(MAX_DYN_NAME + 1);
    assert_eq!(register_dynamic_target(&too_long, ctl), None, "a name past 47 bytes does not fit");
    let exactly = "y".repeat(MAX_DYN_NAME);
    assert!(register_dynamic_target(&exactly, ctl).is_some(), "47 bytes DOES fit");
    assert_eq!(dyn_registered(), 2, "a refused name must leave no partial state");

    // ── E0106 REACHES A READER, and says WHICH refusal ───────────────────────────────────────
    //
    // The corpus documents `None` as "band exhausted" alone. These two are neither, and a caller
    // who acted on that reading would treat a rejected 60-byte name as a lost band and stop
    // registering -- so the reason is asserted, not merely the code.
    let text = sink_text(&path);
    assert!(text.contains(&code), "a refused registration emitted no {code}: {text:?}");
    assert!(
        text.contains("the name is empty"),
        "the empty-name refusal did not name itself: {text:?}"
    );
    assert!(
        text.contains("the name is longer than 47 bytes"),
        "the over-long refusal did not name itself: {text:?}"
    );
    assert!(
        !text.contains("the 32-slot band is full"),
        "a caller error was reported as EXHAUSTION, which is the confusion this argument exists to \
         prevent: {text:?}"
    );

    // ── EXHAUSTION: the band is 32, and the 33rd distinct name gets `None` ────────────────────
    //
    // Filled with distinct names rather than by poking the table, so the probe's collision walk is
    // exercised: 30 more names hash all over the 32 slots and every one must find a home.
    for i in 0..(DYN_BAND_LEN - 2) {
        let name = format!("fill:{i}");
        assert!(
            register_dynamic_target(&name, TargetControl::OFF).is_some(),
            "slot {i} of the fill must succeed -- the band holds {DYN_BAND_LEN}"
        );
    }
    assert_eq!(dyn_registered(), DYN_BAND_LEN, "the band is full");

    assert_eq!(
        register_dynamic_target("one:too:many", TargetControl::OFF),
        None,
        "past {DYN_BAND_LEN} the band is exhausted and the report is the `Option` -- there is no \
         in-band sentinel to hand back, which is why `TargetId::INVALID` does not exist"
    );

    // ── and exhaustion does not break idempotency for names already in ───────────────────────
    //
    // The failure this rules out is specific: a full table makes the probe walk all 32 slots, and
    // an implementation that returned `None` on reaching the end WITHOUT comparing names would
    // refuse a name it had already registered -- turning a re-registration into a mint failure.
    assert_eq!(
        register_dynamic_target("mod:acme", TargetControl::OFF),
        Some(id),
        "a FULL band must still resolve a name it already holds"
    );

    // ── and the exhaustion NAMES THE REJECTED STRING ─────────────────────────────────────────
    //
    // Which is the half that makes the record actionable: "the band is full" tells a reader the
    // system's state, and `one:too:many` tells them which module lost its logging.
    let text = sink_text(&path);
    assert!(
        text.contains("the 32-slot band is full"),
        "exhaustion emitted no {code} with its reason: {text:?}"
    );
    assert!(
        text.contains("one:too:many"),
        "the record does not name the string that was rejected: {text:?}"
    );

    // ── the dyn_*! macros: a record on a runtime target reaches a reader ─────────────────────
    //
    // `mod:acme` was registered at `Level::Debug`, which is what makes the three claims below
    // separable: `dyn_info!` and `dyn_warn!` pass gate (c), `dyn_trace!` does not.
    let other = find_target("fill:0").expect("the fill registered it");
    boyko_log::dyn_info!(id, "acme fired {} times", 3u32);
    // The code is passed as its TYPED newtype, so a `dyn_warn!` handed an `ErrorCode` does not
    // compile. `W0103` rather than `E0106`: the first draft of this line passed
    // `E0106.number()` and the sink printed a W-class line carrying 0106 -- an E code's number under a W class,
    // which `explain(b'W', 106)` cannot resolve. See the commit message; the STATIC macros still
    // accept that and it is recorded as a defect rather than fixed here.
    boyko_log::dyn_warn!(id, boyko_log::codes::W0103, "acme warned about {}", "a thing");
    boyko_log::dyn_trace!(id, "THIS MUST NOT APPEAR: {}", 1u32);

    let text = sink_text(&path);
    assert!(
        text.contains("acme fired 3 times"),
        "a dyn_info! record did not reach the sink WITH ITS ARGUMENTS -- if the line is present but
         the values are missing, the dynamic payload prefix was not stripped: {text:?}"
    );
    assert!(text.contains("acme warned about a thing"), "dyn_warn! did not arrive: {text:?}");
    assert!(
        !text.contains("THIS MUST NOT APPEAR"),
        "dyn_trace! emitted through a Debug ceiling -- gate (c) is not reading the RUNTIME target"
    );

    // ── the record is attributed to the DYNAMIC target, under its interned name ───────────────
    //
    // The claim the prefix exists for. Without it the drain would charge these to whatever the
    // site's field said, and a mod's records would be counted against somebody else's row.
    let row = census_row("mod:acme").expect("a registered dynamic target has a census row");
    assert!(row.records >= 2, "dyn records were not charged to their target: {} ", row.records);
    assert_eq!(row.status_str(), "MEASURED");
    assert_eq!(row.level, Level::Debug, "the census reports the target's own ceiling");

    // ── ONE call site, TWO targets — the property that forced the payload prefix ──────────────
    //
    // A per-site `static` target could not express this: the site below is one `static LogSite`
    // and it is reached with two different ids. If the target travelled in the site rather than
    // in the record, the second call would be filed under the first call's target.
    let before = census_row("fill:0").map_or(0, |r| r.records);
    set_target_control(other, TargetControl::new(Level::Info, 0, false));
    for t in [id, other] {
        boyko_log::dyn_info!(t, "one site, two targets");
    }
    let _ = sink_text(&path);
    let after = census_row("fill:0").map_or(0, |r| r.records);
    assert_eq!(
        after,
        before + 1,
        "the second target got {} of the two records from one site; a site-carried target would \
         give it 0 and charge both to the first",
        after - before
    );

    shutdown();
}

/// The census row for an interned dynamic name, or `None` if the census does not list it.
fn census_row(name: &str) -> Option<boyko_log::census::CensusRow> {
    boyko_log::census::rows().find(|r| r.name == name)
}
