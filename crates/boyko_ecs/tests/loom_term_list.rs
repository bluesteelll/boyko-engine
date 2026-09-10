//! Phase 22.1 Area A — loom exhaustive model of the term-prefilter lock-free
//! publication protocol (`term_list.rs`, P1–P4). Companion to the
//! authoritative Miri-TB oracle (`tests/miri_phase22_1.rs`).
//!
//! These models drive the **real** production methods
//! `TermScratch::resolve_term_filtered` / `TermScratch::reclaim_retired`
//! (Phase-9.1 C1 discipline) through the `#[doc(hidden)]`
//! `term_list::test_exports` shims (one forward call each). Under `--cfg loom`,
//! `term_list.rs` aliases its `AtomicPtr` / `Ordering` to `loom::sync::atomic`,
//! so loom's model checker schedules the genuine `compare_exchange` (Release /
//! Acquire), `retired.swap` (AcqRel), `retired.swap(null)` (Acquire) and the
//! real `Box::from_raw` frees across every permitted interleaving. It SCHEDULES
//! them; what it also CHECKS is a strictly smaller set — see
//! §"What these models DO and DO NOT gate" before citing this file as a proof.
//!
//! # Run
//!
//! ```bash
//! cargo test --release -p boyko-ecs --test loom_term_list \
//!   --config 'target."cfg(windows)".rustflags=["--cfg","loom"]' \
//!   -- --test-threads=1
//! ```
//!
//! ⚠️ **`RUSTFLAGS="--cfg loom"` — which this header prescribed until
//! 2026-09-03 — does not work on this box and is why no `--cfg loom` build of
//! this file ever succeeded.** A `RUSTFLAGS` environment variable REPLACES
//! `target.<triple>.rustflags` rather than appending to it, so it drops the
//! repository's ISA baseline (`-C target-cpu=x86-64-v3`) and, on the
//! windows-gnu host, the machine-local `-Cdlltool` / `-L` / `-Clink-arg=-B`
//! trio that toolchain needs to link at all. `cargo --config` MERGES with the
//! config arrays instead, which is why the form above links.
//!
//! ⚠️ **The key is `cfg(windows)` and not a triple, re-keyed 2026-09-10** —
//! the day this tree's Windows recipes moved from `stable-x86_64-pc-windows-gnu`
//! to `stable-x86_64-pc-windows-msvc`, spelled explicitly through
//! `RUSTUP_TOOLCHAIN`. (The rustup DEFAULT host is still `x86_64-pc-windows-gnu`
//! as of 2026-09-10 — `~/.rustup/settings.toml` reads `default_host_tuple =
//! "x86_64-pc-windows-gnu"` — and `rustup set default-host` is a later,
//! owner-run step. The key below is correct under either state, which is the
//! point of a cfg-spec.) A
//! `target.x86_64-pc-windows-gnu.rustflags` key (this header's spelling before
//! that date) does not match an msvc build at all, so `--cfg loom` never
//! reaches rustc, every `#[cfg(loom)]` model below compiles to nothing, and the
//! binary prints `running 0 tests` and exits **0** — a vacuous green with no
//! model in it and no error to read. `cfg(windows)` matches either host, and
//! cargo JOINS a matching cfg-spec's rustflags with the per-triple ones from
//! `.cargo/config.toml` instead of replacing them, so the ISA baseline survives
//! without being restated here (measured 2026-09-10 off `cargo -v`'s rustc
//! line: `--cfg loom` and `-C target-cpu=x86-64-v3` both present). Do NOT move
//! it to `[build] rustflags`: that key is ignored outright whenever a
//! `[target.*]` one matches — the same vacuous green by another route.
//!
//! # Two gates (matching the architecture plan §"Metrics and validation")
//!
//! * **GATE 11a** (`loom_gate11a_*`): two threads resolve from the same null
//!   `current`; the single-publish CAS lets exactly one win; the loser frees
//!   its own candidate (`Box::from_raw`) and adopts the winner. loom explores
//!   every CAS-vs-CAS ordering (486 executions) and proves the loser never
//!   spins (lock-free, P1) — an unbounded spin exhausts the branch budget.
//!   ⚠️ It does NOT prove "no double-free / no leak / no UAF", which this
//!   bullet claimed until 2026-09-03: loom's leak and UB checkers see only
//!   loom-instrumented allocations, and both candidates are plain `Box`es.
//!   Those three are the Miri-TB oracle's, and remain so.
//!
//! * **GATE 11b** (`loom_gate11b_*`): the reclaim-vs-read race the critic-round
//!   -2 MAJOR flagged. TWO variants make the distinction explicit:
//!   - `..._constrained_clean`: the reader's borrow ENDS (the resolve returns +
//!     a stack-local read completes) BEFORE the dispatcher thread runs
//!     `reclaim`. This mirrors the Phase-9 apply-window ordering (invariant
//!     (a)+(b)). loom reports it CLEAN. ⚠️ It is NOT "the proof that the
//!     atomics are correct GIVEN the scheduler invariants", which this bullet
//!     claimed until 2026-09-03: the ordering it encodes admits exactly ONE
//!     execution (measured), so there is nothing for loom to explore and the
//!     model would stay green with the atomics removed entirely.
//!   - `..._unconstrained_documents_why_invariants_load_bearing`: reclaim is
//!     allowed to interleave WHILE a reader still holds the old pointer. The
//!     comment documents that the protocol's atomics do NOT by themselves
//!     forbid this — only invariants (a) a system is never dispatched
//!     concurrently with itself and (b) epoch changes deferred to the apply
//!     window do. See the test body for how it is expressed without
//!     re-implementing the safe `&mut` funnel.
//!
//! # What these models DO and DO NOT gate — measured 2026-09-03
//!
//! Stated because the header above states more than the harness can deliver,
//! and a claim that outruns its gate is the failure this repository has
//! measured most often. All four figures below come from mutation probes run
//! on this checkout, at the invocation in `# Run`.
//!
//! * **They drive the real production code.** Making `TermList::build` push
//!   each surviving id twice turns ALL FOUR models red (`left: 2, right: 1`).
//!   The `test_exports` shims are not a copy — the Phase-9.1 C1 lesson holds.
//! * **They enumerate real interleavings — for two of the four.** loom reports
//!   `Completed in N iterations`: gate 11a **486**, steady-state **81**,
//!   unconstrained **9**, and constrained **1**. The constrained model is
//!   SEQUENTIAL by construction (thread R is joined before `reclaim` runs), so
//!   it has exactly one execution and carries no interleaving evidence at all.
//!   Its value is as an executable statement of the apply-window ordering; the
//!   gate-11b soundness claim rests on the Miri-TB oracle, not on this model.
//! * **They do NOT gate the ordering table in `term_list.rs`.** Downgrading
//!   EVERY synchronising ordering in the protocol to `Relaxed` — the fast-path
//!   `current.load`, both halves of the publish CAS, `retired.swap` and the
//!   reclaim swap — leaves all four models GREEN. loom detects a causality
//!   violation only on loom-instrumented data, and the published payload is a
//!   plain `Box<TermList>` with plain fields: the POINTER is tracked, the
//!   POINTEE is not. Gating the Release/Acquire pairing would require the
//!   `ids` array to sit behind `loom::cell::UnsafeCell` (an architecture
//!   change to production code, not to this harness).
//!
//! # Branch budget
//!
//! loom's default is 1000 branches per execution and these models need just
//! over it — not because the protocol is branchy, but because `cfg(loom)`
//! aliases `enable_presence.rs` too, and every `ArchetypeMaster` embeds an
//! `EnablePresence` holding `[AtomicPtr; MAX_COMPONENTS]` = 512 loom cells
//! whose `Drop` loads all 512. Those loads are incidental to the term
//! prefilter but loom counts them. MEASURED: 1000 fails, 1100 passes; with
//! `enable_presence.rs` temporarily de-loom'd all four fit inside the default.
//! Hence [`MODEL_MAX_BRANCHES`] — an explicit `LOOM_MAX_BRANCHES` still wins.

#![cfg(loom)]

use loom::sync::Arc;
use loom::sync::atomic::{AtomicUsize, Ordering};
use loom::thread;

use boyko_ecs::ecs::core::iters::query::term_list::test_exports::{
    self, TestScratch,
};

// loom binaries run in isolation; a single fixed id range is safe.
const TAG: usize = 372;
const UNREL: usize = 373;

/// Branch budget per loom execution: ~4x both the default (1000) and the
/// measured need (>1000, <=1100). Sized with headroom for the 512 incidental
/// `EnablePresence` cells rather than trimmed to today's number, so a modest
/// growth of `MAX_COMPONENTS` does not silently turn these models red. Still
/// bounded, so a genuine spin — the thing loom's budget exists to catch, and
/// exactly what P1's "losers never spin" claims cannot happen — is unbounded
/// and still trips it.
const MODEL_MAX_BRANCHES: usize = 4096;

/// Runs `f` under loom with [`MODEL_MAX_BRANCHES`] instead of loom's default.
///
/// An explicit `LOOM_MAX_BRANCHES` in the environment takes precedence: an
/// operator narrowing the budget to hunt a spin must not be silently overruled
/// by the harness.
fn model(f: impl Fn() + Sync + Send + 'static) {
    let mut builder = loom::model::Builder::new();
    if std::env::var_os("LOOM_MAX_BRANCHES").is_none() {
        builder.max_branches = MODEL_MAX_BRANCHES;
    }
    builder.check(f);
}

// ════════════════════════════════════════════════════════════════════════════
// GATE 11a — concurrent first-resolve, single publish.
//
// Two loom threads call the REAL `resolve_term_filtered` against a null
// `current`. Each builds a candidate and races the publish CAS. Across every
// interleaving loom enumerates (486 executions), both threads return a
// length-1 slice: the winner its own published list, the loser the winner's
// after freeing its candidate. `retired` stays null throughout (first publish,
// nothing to retire).
//
// The length-1 assertion is what makes this model fall over on a corrupted
// build (probe-verified, see the module header). It does NOT by itself
// distinguish "one publish" from "two publishes", since either candidate has
// length 1 — the single-publish claim (P1) is carried by the `compare_exchange`
// being the only writer of `current`, and its UB consequences by Miri-TB.
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn loom_gate11a_concurrent_first_resolve_single_publish() {
    model(|| {
        let tag = test_exports::register_tag_layout(TAG);
        let master = Arc::new(test_exports::master_with_tag_archetype(tag));
        let state = Arc::new(test_exports::synced_state(&master, tag));
        let terms = test_exports::one_with_term(tag);
        let scratch = Arc::new(TestScratch::new());

        let len1 = Arc::new(AtomicUsize::new(usize::MAX));
        let len2 = Arc::new(AtomicUsize::new(usize::MAX));

        let h1 = {
            let (s, m, st, l) =
                (Arc::clone(&scratch), Arc::clone(&master), Arc::clone(&state), Arc::clone(&len1));
            thread::spawn(move || {
                let ids = s.resolve(&terms, &m, &st);
                l.store(test_exports::list_len(ids), Ordering::SeqCst);
            })
        };
        let h2 = {
            let (s, m, st, l) =
                (Arc::clone(&scratch), Arc::clone(&master), Arc::clone(&state), Arc::clone(&len2));
            thread::spawn(move || {
                let ids = s.resolve(&terms, &m, &st);
                l.store(test_exports::list_len(ids), Ordering::SeqCst);
            })
        };
        h1.join().unwrap();
        h2.join().unwrap();

        assert_eq!(len1.load(Ordering::SeqCst), 1, "resolver 1 saw the published list");
        assert_eq!(len2.load(Ordering::SeqCst), 1, "resolver 2 saw the published list");

        // The scratch's `Drop` frees the single published `current` exactly
        // once (P4). Dropping it HERE rather than at end-of-closure is what
        // makes the free part of the modelled execution: both joins have
        // already retired the worker Arc clones, so this is the last strong
        // reference and `TermScratch::drop` runs inside the model, where loom
        // still observes the `current`/`retired` loads.
        drop(scratch);
    });
}

// ════════════════════════════════════════════════════════════════════════════
// GATE 11b — CONSTRAINED (apply-window ordering): reader borrow ends BEFORE
// reclaim. EXPECT CLEAN.
//
// Thread R (the "system"): resolves under epoch E1 (rebuild publishes L1,
// retires L0), reads its slice fully, returns — its borrow is dead at the
// `join`. Thread D (the "dispatcher / apply window"): runs `reclaim` ONLY after
// joining R (the join is the happens-before edge standing in for the Phase-9
// completion channel). loom proves the real `reclaim_retired` `Box::from_raw`
// of L0 races no live read of L0 in any interleaving permitted under this
// ordering -> no UAF, no double-free, no leak.
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn loom_gate11b_constrained_reclaim_after_borrow_ends_clean() {
    model(|| {
        let tag = test_exports::register_tag_layout(TAG);
        let unrel = test_exports::register_tag_layout(UNREL);
        let mut master = test_exports::master_with_tag_archetype(tag);
        let mut state = test_exports::synced_state(&master, tag);
        let terms = test_exports::one_with_term(tag);
        let scratch = Arc::new(TestScratch::new());

        // E0 publish (current = L0, retired = null) on the model's main thread.
        {
            let ids = scratch.resolve(&terms, &master, &state);
            assert_eq!(test_exports::list_len(ids), 1);
        }
        // Genuine epoch change E0 -> E1.
        test_exports::bump_epoch_and_resync(&mut master, &mut state, unrel);

        let master = Arc::new(master);
        let state = Arc::new(state);

        // Thread R: resolve under E1 (publishes L1, RETIRES L0), read slice,
        // return. Borrow ends at join.
        let r = {
            let (s, m, st) = (Arc::clone(&scratch), Arc::clone(&master), Arc::clone(&state));
            thread::spawn(move || {
                let ids = s.resolve(&terms, &m, &st);
                let mut acc = 0usize;
                for id in ids {
                    acc = acc.wrapping_add(id.0);
                }
                (test_exports::list_len(ids), acc)
            })
        };
        let (len, _) = r.join().unwrap();
        assert_eq!(len, 1, "E1 reader saw L1");

        // Apply window: reclaim AFTER R's borrow ended -> frees retired L0.
        scratch.reclaim();
        // Idempotent second reclaim (retired now null) — no double free.
        scratch.reclaim();
    });
}

// ════════════════════════════════════════════════════════════════════════════
// GATE 11b — STEADY-STATE concurrent fast-path readers: two threads Acquire-
// load the SAME live `current` and read it; no retire / no reclaim. Proves the
// shared immutable-after-publish read (P3) is race-clean while the list is
// live, across every interleaving.
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn loom_gate11b_steady_state_concurrent_fastpath_clean() {
    model(|| {
        let tag = test_exports::register_tag_layout(TAG);
        let master = Arc::new(test_exports::master_with_tag_archetype(tag));
        let state = Arc::new(test_exports::synced_state(&master, tag));
        let terms = test_exports::one_with_term(tag);
        let scratch = Arc::new(TestScratch::new());

        // Prime the memo on the main thread.
        {
            let ids = scratch.resolve(&terms, &master, &state);
            assert_eq!(test_exports::list_len(ids), 1);
        }

        let h1 = {
            let (s, m, st) = (Arc::clone(&scratch), Arc::clone(&master), Arc::clone(&state));
            thread::spawn(move || {
                let ids = s.resolve(&terms, &m, &st);
                test_exports::list_len(ids)
            })
        };
        let h2 = {
            let (s, m, st) = (Arc::clone(&scratch), Arc::clone(&master), Arc::clone(&state));
            thread::spawn(move || {
                let ids = s.resolve(&terms, &m, &st);
                test_exports::list_len(ids)
            })
        };
        assert_eq!(h1.join().unwrap(), 1, "fast-path reader 1 saw live L0");
        assert_eq!(h2.join().unwrap(), 1, "fast-path reader 2 saw live L0");
    });
}

// ════════════════════════════════════════════════════════════════════════════
// GATE 11b — UNCONSTRAINED (documents WHY (a)+(b) are load-bearing).
//
// This variant lets `reclaim` run on thread D WHILE thread R is still inside an
// epoch where R could observe the retired list. We CANNOT express "R holds &*L0
// across a yield point" through the safe shim API (the slice borrow is bounded
// by the single `resolve` call, and the production `&mut` funnel that gates
// reclaim is exactly what forbids the overlap) — so this test does NOT assert a
// UAF. Instead it documents, executably, the protocol's TRUST BOUNDARY:
//
//   The atomics alone (the D-B ordering table) guarantee Release/Acquire
//   visibility and single-publish, but they DO NOT establish "no reader holds
//   L0 when reclaim frees it". That edge is carried by invariants
//     (a) a system is never dispatched concurrently with itself, and
//     (b) structural epoch changes + reclamation are deferred to the apply
//         window, ordered after all system borrows end (Phase-9 completion
//         channel).
//
// Here we drive reclaim concurrently with a fresh resolve on a FOREIGN scratch
// epoch state to show the reclaim path itself (swap-to-null + Box::from_raw) is
// internally race-clean; the "reader still holding L0" overlap is precisely the
// scenario (a)+(b) make UNREACHABLE in production, which is why the CONSTRAINED
// test above is the soundness proof and this one is the boundary documentation.
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn loom_gate11b_unconstrained_documents_why_invariants_load_bearing() {
    model(|| {
        let tag = test_exports::register_tag_layout(TAG);
        let unrel = test_exports::register_tag_layout(UNREL);
        let mut master = test_exports::master_with_tag_archetype(tag);
        let mut state = test_exports::synced_state(&master, tag);
        let terms = test_exports::one_with_term(tag);
        let scratch = Arc::new(TestScratch::new());

        // Prime + epoch change so `retired` is populated (L0 retired, L1 live).
        {
            let ids = scratch.resolve(&terms, &master, &state);
            assert_eq!(test_exports::list_len(ids), 1);
        }
        test_exports::bump_epoch_and_resync(&mut master, &mut state, unrel);
        {
            let ids = scratch.resolve(&terms, &master, &state);
            assert_eq!(test_exports::list_len(ids), 1);
        }

        let master = Arc::new(master);
        let state = Arc::new(state);

        // Thread D: reclaim (frees retired L0).
        let d = {
            let s = Arc::clone(&scratch);
            thread::spawn(move || s.reclaim())
        };
        // Thread R: a fresh fast-path resolve of the SAME live epoch (reads L1,
        // NOT L0). This is the only overlap expressible through the safe slice
        // borrow — R never holds L0. loom proves reclaim(L0) || resolve(L1) is
        // race-clean (disjoint objects); the L0-overlap is unreachable here for
        // exactly the reason documented in the module/header: the `&mut` funnel.
        let r = {
            let (s, m, st) = (Arc::clone(&scratch), Arc::clone(&master), Arc::clone(&state));
            thread::spawn(move || {
                let ids = s.resolve(&terms, &m, &st);
                test_exports::list_len(ids)
            })
        };
        d.join().unwrap();
        assert_eq!(r.join().unwrap(), 1, "concurrent resolve saw the live L1, never the retired L0");
    });
}
