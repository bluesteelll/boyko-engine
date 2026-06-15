//! Phase 22.1 Area A — multi-threaded Miri-TB soundness proof for the
//! lock-free term-prefilter publication protocol (`term_list.rs`, P1–P4).
//!
//! This file is the **authoritative soundness oracle** for the protocol per
//! the project's standing rule (Miri Tree-Borrows + the data-race detector are
//! the only oracles that have historically caught the F2 / NEW-1 /
//! BUG-P19-TB-1 raw-pointer-aliasing UB class — never review, never loom in
//! isolation). The loom harness (`tests/loom_term_list.rs`) is the *exhaustive
//! interleaving* companion; in environments where loom cannot be built (its
//! `tracing-subscriber -> windows-sys` chain needs `dlltool.exe`, absent on the
//! GNU host here) THIS file carries the central P2 reclaim-vs-read claim
//! (critic-round-2 MAJOR / gate 11b) by driving real std threads.
//!
//! # What is driven — the REAL protocol (Phase-9.1 C1 discipline)
//!
//! Every test calls the genuine `TermScratch::resolve_term_filtered` and
//! `TermScratch::reclaim_retired` through the `#[doc(hidden)]`
//! `term_list::test_exports` shims (one forward call each — no re-implementation
//! of the atomics). Miri therefore observes the real `Acquire` / `Release` /
//! `AcqRel` orderings and the real `Box::from_raw` frees. The shims wrap the
//! `pub(crate)` `TermScratch` / `TagTerms` in opaque `pub` newtypes
//! (`TestScratch` / `TestTerms`), mirroring `loom_exports::LoomScopeShared`.
//!
//! # Run
//!
//! ```powershell
//! $env:MIRIFLAGS="-Zmiri-tree-borrows"
//! cargo +nightly miri test -p boyko-ecs --test miri_phase22_1
//! ```
//!
//! (No `-Zmiri-ignore-leaks` is needed: the protocol's bounded retire/reclaim
//! is exact — `Drop for TermScratch` frees both `current` and `retired`, so a
//! correctly-driven harness leaks nothing.)
//!
//! # File gate
//!
//! `#![cfg(miri)]` — only compiles under Miri. The native behavioral coverage
//! of the same protocol lives in the in-crate unit tests and in
//! `phase22_query_terms.rs`.

#![cfg(miri)]

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;

use boyko_ecs::ecs::core::iters::query::term_list::test_exports::{
    self, TestScratch,
};

const SEQ: Ordering = Ordering::SeqCst;

// Distinct ComponentId ranges per test so concurrently-run binaries never
// collide on the write-once LAYOUTS slots (the crate-wide fixed-id convention).
// Phase 22.1 Miri-TB harness reserves 360-379 (free per the partition map in
// tests/phase22_tags.rs).
const TAG_A_BASE: usize = 360;

// ════════════════════════════════════════════════════════════════════════════
// GATE 11a — concurrent first-resolve: two threads both resolve from the
// SAME null `current` slot. Exactly one CAS wins; the loser frees its own
// candidate (real `Box::from_raw`) and adopts the winner. No double-free, no
// UAF, no leak. This is the C1 scenario the round-1 `UnsafeCell` design could
// not survive — here it is the designed-for race.
//
// Miri-TB asserts: no aliasing/data-race violation across the two real
// `resolve_term_filtered` calls publishing/adopting on one shared slot, and
// (data-race detector) the CAS arbitration is sound. The shared list both
// threads end up reading is immutable-after-publish (P3).
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn miri_gate11a_concurrent_first_resolve_single_publish() {
    let tag = test_exports::register_tag_layout(TAG_A_BASE);
    // A real master with the tag archetype + a real synced state. Both are
    // read-only and shared across the resolvers (matches the production
    // `&ArchetypeMaster` / `&QueryState` borrow on the resolve path).
    let master = Arc::new(test_exports::master_with_tag_archetype(tag));
    let state = Arc::new(test_exports::synced_state(&master, tag));
    let terms = test_exports::one_with_term(tag);
    let scratch = Arc::new(TestScratch::new());

    // Each resolver records the length of the slice it got back. Same epoch ⇒
    // identical content ⇒ identical length, regardless of who won the CAS.
    let len_t1 = Arc::new(AtomicUsize::new(usize::MAX));
    let len_t2 = Arc::new(AtomicUsize::new(usize::MAX));

    let h1 = {
        let (scratch, master, state, len) =
            (Arc::clone(&scratch), Arc::clone(&master), Arc::clone(&state), Arc::clone(&len_t1));
        thread::spawn(move || {
            let ids = scratch.resolve(&terms, &master, &state);
            len.store(test_exports::list_len(ids), SEQ);
        })
    };
    let h2 = {
        let (scratch, master, state, len) =
            (Arc::clone(&scratch), Arc::clone(&master), Arc::clone(&state), Arc::clone(&len_t2));
        thread::spawn(move || {
            let ids = scratch.resolve(&terms, &master, &state);
            len.store(test_exports::list_len(ids), SEQ);
        })
    };
    h1.join().expect("resolver 1 did not panic");
    h2.join().expect("resolver 2 did not panic");

    let l1 = len_t1.load(SEQ);
    let l2 = len_t2.load(SEQ);
    assert_eq!(l1, 1, "resolver 1 saw the single tag archetype");
    assert_eq!(l2, 1, "resolver 2 saw the single tag archetype");
    assert_eq!(l1, l2, "both resolvers adopted the same single published list (P1)");

    // Drop frees `current` (the winner's published list) exactly once; the
    // loser's candidate was already freed inside `rebuild_publish` on the CAS
    // failure path. Miri's leak check (default-on here) confirms no leak.
    Arc::try_unwrap(scratch)
        .unwrap_or_else(|_| panic!("scratch still shared at teardown"))
        .reclaim(); // exercise the reclaim fast path (retired is null here) then Drop
}

// ════════════════════════════════════════════════════════════════════════════
// GATE 11b (CONSTRAINED — the load-bearing case) — resolve-vs-reclaim under
// the Phase-9 apply-window ordering: a reader thread resolves and FULLY
// CONSUMES the slice (its borrow ENDS), and ONLY THEN does the
// dispatcher-thread reclaim run, freeing the retired list. This mirrors
// invariant (b): structural epoch changes + reclamation are deferred to the
// apply window, ordered AFTER all system borrows end by the Phase-9 completion
// channel (here: the reader's `thread::join`, an Acquire/Release sync edge that
// stands in for the completion channel).
//
// EXPECTATION: clean. This is the case that MUST hold for the protocol to be
// sound in production. Miri-TB + the data-race detector prove the real
// `reclaim_retired` `Box::from_raw` frees a list no live reader is touching.
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn miri_gate11b_constrained_reclaim_after_borrow_ends_is_clean() {
    let tag = test_exports::register_tag_layout(TAG_A_BASE + 1);
    let unrelated = test_exports::register_tag_layout(TAG_A_BASE + 2);
    let mut master = test_exports::master_with_tag_archetype(tag);
    let mut state = test_exports::synced_state(&master, tag);
    let terms = test_exports::one_with_term(tag);

    let scratch = Arc::new(TestScratch::new());

    // Epoch E0: publish list L0 (current = L0, retired = null).
    {
        let ids = scratch.resolve(&terms, &master, &state);
        assert_eq!(test_exports::list_len(ids), 1, "E0 published one id");
        // borrow ends here
    }

    // Epoch change E0 -> E1 (genuine archetype-generation bump + resync).
    test_exports::bump_epoch_and_resync(&mut master, &mut state, unrelated);

    // A reader thread resolves under E1: this rebuilds, publishes L1 into
    // `current`, and RETIRES L0 into `retired`. The reader fully reads its
    // slice (a real load through &*list) and returns — its borrow ENDS at the
    // thread boundary (the `join` below is the apply-window-equivalent
    // happens-before edge).
    let observed_len = {
        let scratch_t = Arc::clone(&scratch);
        // Move read-only refs into the thread via raw shared Arcs of the
        // master/state so the resolve drives the real build on the worker.
        let master_t = Arc::new(master);
        let state_t = Arc::new(state);
        let terms_t = terms;
        let m2 = Arc::clone(&master_t);
        let s2 = Arc::clone(&state_t);
        let handle = thread::spawn(move || {
            let ids = scratch_t.resolve(&terms_t, &m2, &s2);
            // Touch every byte of the slice so Miri records a real read of the
            // freshly-published L1 while L0 is now in `retired`.
            let mut acc = 0usize;
            for id in ids {
                acc = acc.wrapping_add(id.0);
            }
            (test_exports::list_len(ids), acc)
        });
        let (len, _acc) = handle.join().expect("E1 resolver did not panic");
        // The reader's borrow is now provably dead (thread joined). Recover the
        // master/state Arcs for the reclaim assertion (kept alive only so the
        // worker could build).
        let _ = (Arc::try_unwrap(master_t), Arc::try_unwrap(state_t));
        len
    };
    assert_eq!(observed_len, 1, "E1 published one id (L1)");

    // Apply window: reclaim runs AFTER the reader's borrow ended (the join
    // above). This is the real `reclaim_retired` `Box::from_raw` freeing L0.
    // Under the constraint, no reader holds L0's header -> TB/data-race clean.
    scratch.reclaim();

    // A second reclaim is a no-op (retired swapped to null) — exercises the
    // defense-in-depth swap-to-null (P2) without a double free.
    scratch.reclaim();

    // Teardown: Drop frees the still-current L1 exactly once.
    Arc::try_unwrap(scratch)
        .unwrap_or_else(|_| panic!("scratch still shared at teardown"))
        .reclaim();
}

// ════════════════════════════════════════════════════════════════════════════
// GATE 11b — REUSE / STEADY STATE: after a publish, a concurrent pair of
// resolvers on the SAME (unchanged) epoch take the fast path (Acquire load,
// `matches`, return) with NO rebuild, NO CAS, NO allocation. Proves the
// fast-path shared `&*current` read is race-clean across threads while the
// list is live (no retire, no reclaim in flight).
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn miri_gate11b_steady_state_concurrent_fastpath_readers_clean() {
    let tag = test_exports::register_tag_layout(TAG_A_BASE + 3);
    let master = Arc::new(test_exports::master_with_tag_archetype(tag));
    let state = Arc::new(test_exports::synced_state(&master, tag));
    let terms = test_exports::one_with_term(tag);
    let scratch = Arc::new(TestScratch::new());

    // Prime the memo: one publish on the main thread (current = L0).
    {
        let ids = scratch.resolve(&terms, &master, &state);
        assert_eq!(test_exports::list_len(ids), 1);
    }

    // Two concurrent fast-path readers: both Acquire-load the SAME live L0 and
    // read its slice. No mutation anywhere — pure shared immutable read (P3).
    let mut handles = Vec::new();
    for _ in 0..2 {
        let (scratch, master, state, terms) =
            (Arc::clone(&scratch), Arc::clone(&master), Arc::clone(&state), terms);
        handles.push(thread::spawn(move || {
            let ids = scratch.resolve(&terms, &master, &state);
            let mut acc = 0usize;
            for id in ids {
                acc = acc.wrapping_add(id.0);
            }
            (test_exports::list_len(ids), acc)
        }));
    }
    for h in handles {
        let (len, _) = h.join().expect("fast-path reader did not panic");
        assert_eq!(len, 1, "steady-state reader saw the live L0");
    }

    Arc::try_unwrap(scratch)
        .unwrap_or_else(|_| panic!("scratch still shared at teardown"))
        .reclaim();
}

// ════════════════════════════════════════════════════════════════════════════
// Sequential rebuild arms (W2b): BOTH epoch-change triggers under Miri-TB on a
// single thread — terms-change is N/A here (the memo keys on the live-prefix
// terms too, but our harness fixes one term set), so this drives the
// GENERATION-change arm explicitly: publish -> bump -> rebuild publishes a new
// list and retires the old -> reclaim frees the old. All on the apply-funnel
// (`&mut` master/state) so the borrow checker enforces P2 exclusivity.
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn miri_rebuild_on_generation_change_then_reclaim() {
    let tag = test_exports::register_tag_layout(TAG_A_BASE + 4);
    let u1 = test_exports::register_tag_layout(TAG_A_BASE + 5);
    let u2 = test_exports::register_tag_layout(TAG_A_BASE + 6);
    let mut master = test_exports::master_with_tag_archetype(tag);
    let mut state = test_exports::synced_state(&master, tag);
    let terms = test_exports::one_with_term(tag);
    let scratch = TestScratch::new();

    // E0 publish.
    assert_eq!(test_exports::list_len(scratch.resolve(&terms, &master, &state)), 1);

    // E0 -> E1: bump, rebuild (retires L0), reclaim (frees L0).
    test_exports::bump_epoch_and_resync(&mut master, &mut state, u1);
    assert_eq!(
        test_exports::list_len(scratch.resolve(&terms, &master, &state)),
        1,
        "E1 rebuild still matches the single tag archetype"
    );
    scratch.reclaim(); // frees retired L0

    // E1 -> E2: a second epoch change, proving the retired slot was empty when
    // the second retire arrived (the P2 `debug_assert!(prev.is_null())`).
    test_exports::bump_epoch_and_resync(&mut master, &mut state, u2);
    assert_eq!(test_exports::list_len(scratch.resolve(&terms, &master, &state)), 1);
    scratch.reclaim(); // frees retired L1

    // Teardown frees current L2.
    drop(scratch);
}

// ════════════════════════════════════════════════════════════════════════════
// Empty-filtered result: an epoch whose terms match ZERO archetypes publishes
// a NON-NULL list with an empty `ids` slice (memoised; distinct from
// never-built null). Drives the empty-slice deref + reclaim under Miri.
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn miri_empty_filtered_publish_and_reclaim() {
    // Tag T present in the archetype, but the query carries a `with` term over
    // a DIFFERENT tag U that no archetype has -> zero matches.
    let present = test_exports::register_tag_layout(TAG_A_BASE + 7);
    let absent = test_exports::register_tag_layout(TAG_A_BASE + 8);
    let master = test_exports::master_with_tag_archetype(present);
    let state = test_exports::synced_state(&master, present);
    let terms_absent = test_exports::one_with_term(absent);
    let scratch = TestScratch::new();

    let ids = scratch.resolve(&terms_absent, &master, &state);
    assert_eq!(
        test_exports::list_len(ids),
        0,
        "term over an absent tag publishes an empty (non-null) list"
    );
    scratch.reclaim();
    drop(scratch);
}
