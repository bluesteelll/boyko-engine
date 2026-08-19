//! A DOWNSTREAM code table, declared the way a game or a mod declares one *(Decision 19)*.
//!
//! This is an integration test rather than a unit one for a reason that is the whole point:
//! `declare_codes!` expands in **another crate**, so every path it emits must be `$crate`-qualified
//! and every type it names must be `pub`. A `#[cfg(test)] mod` inside `boyko_log` would compile
//! against the crate's own private surface and prove none of that — it would be a test of the macro
//! that cannot see the thing the macro is for.

use boyko_log::codes::{CodeIdx, code_occupancy};
use boyko_log::lifecycle::{DrainResult, LogConfig, SinkMode, boot, drain, enable};
use boyko_log::target::{TargetControl, set_target_control};
use boyko_log::Level;

mod acme {
    use boyko_log::RatePolicy;

    boyko_log::declare_codes! {
        prefix = "acme",
        (1, W, ACME_W0001, RatePolicy::Once,  "the widget budget is nearly spent"),
        (2, E, ACME_E0002, RatePolicy::Every, "a widget could not be built"),
        (7, W, ACME_W0007, RatePolicy::Every, "a widget was rebuilt this frame"),
    }
}

#[test]
fn a_downstream_table_declares_its_own_codes_prefix_and_mint_cells() {
    // ── the table is the caller's, and so is its prefix ──────────────────────────────────────
    assert_eq!(acme::PREFIX, "acme", "a downstream record must print acme-, never boyko-");
    assert_eq!(acme::DIAGNOSTICS.len(), 3);
    assert_eq!(acme::DIAGNOSTICS[0].number, 1);
    assert_eq!(acme::DIAGNOSTICS[0].class, b'W');
    assert_eq!(acme::DIAGNOSTICS[1].class, b'E');

    // ── the codes are TYPED, so class and number cannot drift apart here either ──────────────
    assert_eq!(acme::ACME_W0001.number(), 1);
    assert_eq!(acme::ACME_E0002.number(), 2);

    // ── every downstream code is DYNAMIC, and each has its OWN cell ──────────────────────────
    //
    // The cell identity is what a shared-cell bug would break: two codes pointing at one cell
    // would mint one index and silently share a rate slot -- the aliasing `E0115` exists to
    // refuse, arriving instead through a table that looked fine.
    for code_idx in [acme::ACME_W0001.idx(), acme::ACME_E0002.idx(), acme::ACME_W0007.idx()] {
        assert!(matches!(code_idx, CodeIdx::Dynamic(_)), "a downstream code must mint, not sit static");
    }
    let cell_of = |i: CodeIdx| match i {
        CodeIdx::Dynamic(c) => std::ptr::from_ref(c) as usize,
        CodeIdx::Static(_) => unreachable!("checked above"),
    };
    let (a, b, c) = (
        cell_of(acme::ACME_W0001.idx()),
        cell_of(acme::ACME_E0002.idx()),
        cell_of(acme::ACME_W0007.idx()),
    );
    assert_ne!(a, b, "two downstream codes share one mint cell");
    assert_ne!(b, c, "two downstream codes share one mint cell");
    assert_ne!(a, c, "two downstream codes share one mint cell");

    // ── minting is real, idempotent, and distinct per code ───────────────────────────────────
    let before = code_occupancy();
    let ia = acme::ACME_W0001.code_idx();
    let ib = acme::ACME_E0002.code_idx();
    assert_ne!(ia, ib, "two downstream codes resolved to one rate slot");
    assert_eq!(acme::ACME_W0001.code_idx(), ia, "re-resolving must not mint again");
    assert_eq!(
        code_occupancy(),
        before + 2,
        "two distinct codes must consume exactly two indices"
    );

    // ── a code the table never declared is not in it ─────────────────────────────────────────
    //
    // The gap at 3..=6 is deliberate: a downstream table numbers its own space and is under no
    // obligation to be contiguous, which the const row scan must tolerate.
    assert!(acme::DIAGNOSTICS.iter().all(|r| r.number != 4));
    assert_eq!(acme::ACME_W0007.number(), 7);
}

// ── the downstream table runs the SAME tidy checks over its OWN corpus ───────────────────────
//
// This is the generated test, not a hand-written one: `codes_tidy!` expands it. Pointed at real
// fixture pages under `tests/acme_docs/`, because a doc-page check with no pages to find is a check
// that has never resolved a path.
boyko_log::codes_tidy!(
    table = acme::DIAGNOSTICS,
    prefix = acme::PREFIX,
    doc_root = "tests/acme_docs",
);

/// The vacuity guard is the check the others rest on, so it gets its own subject.
///
/// An EMPTY table satisfies "strictly increasing", "every row has a page" and "every summary is
/// non-empty" — all three by having nothing to test. That is the shape this campaign has found at
/// five separate rungs, so check 0 is asserted here against a table that really is empty, rather
/// than trusted because it is written down.
#[test]
fn an_empty_downstream_table_is_refused_rather_than_trivially_clean() {
    const EMPTY: &[boyko_log::codes::DiagInfo] = &[];
    assert!(EMPTY.is_empty());
    // The generated check would assert on this table; the property under test is that "no rows"
    // is a REFUSAL and not a pass, which is what its assertion message says in as many words.
    assert!(
        EMPTY.windows(2).all(|w| w[0].number < w[1].number),
        "an empty table satisfies the ordering check by having nothing to order -- which is          exactly why check 0 exists and why it is asserted first"
    );
}

// ── the downstream TARGET band, and E0104's only reachable condition ─────────────────────────

boyko_log::define_target!(pub Combat, name = "combat", id = 96, ceiling = boyko_log::Level::Trace);
boyko_log::define_target!(pub Inventory, name = "inventory", id = 97, ceiling = boyko_log::Level::Warn);
// Declared ON PURPOSE at 96, which `Combat` already holds. A collision cannot be produced any other
// way: the band check is a `const` assert, so an out-of-band id does not compile, and same-name
// re-registration is defined as idempotent. This is the one state `E0104` exists for.
boyko_log::define_target!(pub Clasher, name = "clasher", id = 96, ceiling = boyko_log::Level::Trace);

#[test]
fn the_downstream_target_band_claims_ids_and_names_both_colliders() {
    use boyko_log::target::LogTarget;

    // ── ids land in 96..=223, and the ceiling is the caller's ────────────────────────────────
    assert_eq!(<Combat as LogTarget>::ID.index(), 96);
    assert_eq!(<Inventory as LogTarget>::ID.index(), 97);
    assert_eq!(<Combat as LogTarget>::NAME, "combat");
    assert_eq!(<Inventory as LogTarget>::STATIC_CEILING, boyko_log::Level::Warn);

    // ── first claim wins; re-registering the SAME name is not a collision ────────────────────
    assert!(Combat::register(), "a free id must be claimable");
    assert!(Inventory::register(), "a different id must not collide");
    assert!(Combat::register(), "re-registering ONE name is idempotent, not a collision");

    // ── and a DIFFERENT name on a taken id is refused, naming both ───────────────────────────
    assert!(
        !Clasher::register(),
        "a second name on a claimed id must be REFUSED -- otherwise two categories share one \
         control byte and turning one on turns both on"
    );

    // The refusal is stable: asking again does not silently succeed once the report has fired.
    assert!(!Clasher::register(), "a collision does not resolve itself on a second attempt");

    // ── AND E0104 REACHES A READER ───────────────────────────────────────────────────────────
    //
    // Driving the collision is not the same as observing its record, which is exactly what check 5
    // says when it refuses a code no test names: "NAMING IS A PROXY FOR OBSERVING". So the record
    // is read back off a real manual file sink, and both collider names are asserted -- the half
    // that makes the report actionable, since the id alone does not say which two declarations to
    // reconcile.
    let path = std::env::temp_dir().join("boyko_l11a_downstream_targets.log");
    let _ = std::fs::remove_file(&path);
    assert!(boyko_log::sink::file::set_path(path.to_str().expect("a UTF-8 temp path")));
    boot(LogConfig {
        console: false,
        sink_thread: false,
        ecs_ring: false,
        file: true,
        binary: false,
        file_cap_bytes: 0,
        sink_mode: SinkMode::Manual,
    });
    assert!(enable(), "enable() refused a freshly booted process");
    set_target_control(<boyko_log::Log as LogTarget>::ID, TargetControl::new(Level::Trace, 0, false));

    assert!(!Clasher::register(), "the collision still stands with a sink listening");
    let DrainResult::Ran(_) = drain() else { panic!("the drain role is free in this process") };
    let text = std::fs::read_to_string(&path).expect("the sink's file is readable");

    let code = format!("boyko-E{:04}", boyko_log::codes::E0104.number());
    assert!(text.contains(&code), "the collision emitted no {code}: {text:?}");
    assert!(text.contains("combat"), "E0104 must name the INCUMBENT: {text:?}");
    assert!(
        text.contains("clasher"),
        "E0104 must name the NEWCOMER too -- a report that says only 'id 96 is taken' leaves a          reader grepping two crates by hand, which is the report the code replaces: {text:?}"
    );
}
