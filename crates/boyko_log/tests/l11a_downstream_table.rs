//! A DOWNSTREAM code table, declared the way a game or a mod declares one *(Decision 19)*.
//!
//! This is an integration test rather than a unit one for a reason that is the whole point:
//! `declare_codes!` expands in **another crate**, so every path it emits must be `$crate`-qualified
//! and every type it names must be `pub`. A `#[cfg(test)] mod` inside `boyko_log` would compile
//! against the crate's own private surface and prove none of that — it would be a test of the macro
//! that cannot see the thing the macro is for.

use boyko_log::codes::{CodeIdx, code_occupancy};

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
