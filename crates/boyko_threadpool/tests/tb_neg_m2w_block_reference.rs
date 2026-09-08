//! KE16 M2w — the NEGATIVE control for the block's side of the completion
//! protector. Its success is an ABORT, not a pass.
//!
//! # Why this binary exists, stated as the loss it covers
//!
//! Before stage 3b the scoped cell was freed as soon as the body had been read
//! out of it, *before* the body was invoked — so by the time the release RMW
//! committed, the payload allocation did not exist at all. That is strictly
//! stronger than "no protector covers it", because it needs nothing from the
//! code that runs afterwards. **Stage 3b deleted that clause**: the cell now
//! lives in a `ScopeBlock` chunk that `Scope::drop` frees with `free_all`,
//! immediately after the join, i.e. inside the same window `ScopeShared`'s own
//! `Box::from_raw` sits in.
//!
//! What replaces it is D1 (chunks hold payload bytes only), D2/D3 (type-system
//! facts: `emplace` initialises, `erase() -> *const ()` is `BlockPtr`'s only
//! exit, `ScopedCell` is private), D5 (a reduction leaving exactly ONE function
//! able to hold a protector across the release — `run_scoped`) and **an
//! execution gate over that one function.** This file is the negative half of
//! that gate: it drives the configuration in which `run_scoped`'s replacement
//! DOES form a protected reference into chunk memory, so that a green from the
//! positive gate (`tests/miri_scope_completion_protector.rs`) means "the shipped
//! function was executed in the deciding interleaving and reported nothing"
//! rather than "the interleaving never happened".
//!
//! # The arm, and where it lives
//!
//! Under `--features tb-neg-m2w`, `Task::new_scoped` stores `run_scoped_neg::<F>`
//! in `Task::execute` instead of `run_scoped::<F>`. `run_scoped_neg` differs by
//! exactly two textual changes: it forms `let cell: &ScopedCell<F> = &*ptr.cast()`
//! and passes that reference as an ARGUMENT to
//! `#[inline(never)] fn finish_neg<F>(_cell: &ScopedCell<F>, shared: *const ScopeShared)`,
//! which performs the completion. Both live in `src/task/scoped.rs` beside
//! `run_scoped`. Under Tree Borrows a reference-typed function argument carries a
//! protector that is live for the whole call — to the closing brace, whether or
//! not the callee touches a byte — so `finish_neg`'s frame holds a strong
//! protector over chunk memory across `complete_task`'s release RMW, and the
//! first reclamation after that RMW is `free_all`.
//!
//! **The arm is not buildable natively.** `src/lib.rs` carries
//! `#[cfg(all(feature = "tb-neg-m2w", not(miri)))] compile_error!(…)`, so an
//! `--all-features` build — the one configuration that would otherwise ship
//! deliberate UB into the normal suite — refuses to compile.
//!
//! # The three verdicts this file can produce, and why none of them is silent
//!
//! | configuration | outcome | why |
//! |---|---|---|
//! | native, any features | RED at the `UNDER_MIRI` assert | Tree Borrows is the rule that judges this; a native run decides nothing |
//! | Miri, no `tb-neg-m2w` | RED at `tb-neg-m2w: arm not built` | the arm is what is being controlled for; without it there is nothing to catch |
//! | Miri, `tb-neg-m2w` | Miri aborts with `error: Undefined Behavior: deallocation through <…> is forbidden` | the receipt; see `scripts/tb_neg_gate.ps1` |
//!
//! And if the third row runs to completion, the terminal `panic!` below reds it:
//! **a negative control that goes green has stopped being negative.**
//!
//! There is deliberately **no file-scope `#![cfg(…)]`** on this binary. That is
//! this repository's catalogued trap — a file-scope `cfg` that is false prints
//! `running 0 tests` and exits 0, a vacuous pass that reads exactly like a pass.
//! The single test function is unconditional; every configuration reaches it and
//! every configuration says out loud what it decided.
//!
//! # Run
//!
//! ```text
//! MIRIFLAGS="-Zmiri-tree-borrows -Zmiri-disable-isolation -Zmiri-permissive-provenance
//!   -Zmiri-ignore-leaks -Zmiri-preemption-rate=0 -Zmiri-seed=0"
//!   cargo +nightly-x86_64-pc-windows-gnu miri test -p boyko-threadpool
//!   --features tb-neg-m2w --test tb_neg_m2w_block_reference -- --nocapture
//! ```
//!
//! Driven for all four seeds by `scripts/tb_neg_gate.ps1` (and `.sh`), which owns
//! the receipt rules. `-Zmiri-tree-borrows` is not optional: the protector is a
//! Tree-Borrows object and Stacked Borrows does not install it on this shape.
//! `-Zmiri-preemption-rate=0` is not optional either — at the default rate the
//! sibling gate's own measured table (`tests/miri_scope_completion_protector.rs`,
//! the receiver × seed table in the module header) caught the equivalent defect
//! on a MINORITY of seeds.
//!
//! # The shape, and why it is the sibling gate's shape rather than a new one
//!
//! The window is PER SCOPE: only the decrement that drives `pending` to zero can
//! race the free, and a scope whose last body ran inline on the joiner has no
//! window at all — the same thread decrements and frees. Hoping the joiner's
//! batch steal leaves the last body on a worker is schedule-noise, MEASURED as
//! such by the sibling gate, whose census swung between two windows and zero on
//! unchanged code.
//!
//! So the shape here is that file's, deliberately and not by convenience: each
//! scope spawns exactly `TASKS_PER_SCOPE == WORKERS` bodies; each body announces
//! itself in `started` and then waits on `go`; the scope's own closure waits,
//! bounded, until every body has announced, and only then sets `go` and returns.
//! When `Scope::drop` polls, every task is already CLAIMED and running on a
//! genuine worker, so the joiner has nothing to steal and cannot become a
//! completer itself. The last decrement is a worker's by construction.
//!
//! A second harness would have had to re-establish that property from scratch,
//! and a negative control built on a shape whose window is a coin flip proves
//! nothing when it fails to fire.

/// True exactly when this binary is being interpreted by Miri.
///
/// Named rather than spelled `cfg!(miri)` at the assertion site so the guard
/// below reads as the configuration statement it is. The value is a compile-time
/// constant in either configuration, which is the whole point: this binary has
/// no runtime way to become Miri-interpreted, so the only honest thing it can do
/// natively is say so and fail.
const UNDER_MIRI: bool = cfg!(miri);

/// The negative control: a protector over chunk memory, held across the release.
///
/// Green is a FAILURE mode of this test, not its success. The success is Miri
/// aborting the interpreter with
/// `error: Undefined Behavior: deallocation through <…> is forbidden`, the freeing
/// thread inside `ScopeBlock::free_all` and the protected tag inside
/// `src/task/scoped.rs`. `scripts/tb_neg_gate.ps1` reads that off the receipt
/// rather than off an exit code, because on this machine a `cargo +nightly` that
/// resolves to MSVC dies in the linker with exit 1 — indistinguishable from a red
/// gate.
// `assertions_on_constants` is allowed rather than obeyed, and the lint's own
// suggestion is why. It is RIGHT that the assertion below has a constant value —
// it is a configuration check, constant by construction in each build — but its
// remedy, `const { assert!(…) }`, would turn a native run from `running 1 test`
// plus a red into a BUILD FAILURE of this whole test binary, and therefore of
// `cargo test --workspace`. That is the opposite of what this file is for: the
// catalogued trap here is a configuration that decides nothing while READING as
// a pass, and the cure is a test that runs everywhere and reports what it
// decided, not one that refuses to exist. (The allow sits on the function
// because a statement-level `allow` over a macro invocation is ignored by rustc,
// which reports it as `unused_attributes`.)
//
// ⚠ THE `cfg_attr` BELOW WAS ADDED 2026-09-08, AND IT REVISES THE PARAGRAPH
// ABOVE RATHER THAN CONTRADICTING IT. The argument against `const { assert!(…) }`
// stands — a build failure is worse than a red. But the configuration this
// binary needs is one it can never reach in `cargo test`, so as written it was
// red in EVERY configuration the default suite can produce, permanently. This
// repository's most-measured defect is exactly that: a known-red target teaches
// readers to skip a line, and three reds hid behind one for 87 commits
// (CLAUDE.md, `--no-fail-fast`). The tree already had the answer and had used it
// twice — `brick_field_is_conservative_lower_bound` and its sibling are RED BY
// DESIGN until M2 lands, and they carry `#[ignore = "deferred: …"]` rather than
// standing red. So does this one now.
//
// What is NOT given up: `tests/ignore_reasons_census.rs` fails the build on a
// bare or empty reason, so the ignore cannot decay into a silent disappearance;
// the two assertions below still fire, unchanged, for anyone who runs this
// binary with `--include-ignored` in a configuration that cannot decide; and the
// NATIVE gate over this property is not this test at all but
// `tb_neg_m2w_arm_present.rs`, which is unignored, runs everywhere, and asserts
// that the arm exists in the source, that both driver scripts stay in step, and
// that four committed receipts are red for the declared diagnostic.
//
// The cfg is `all(miri, feature)` and not `miri` alone because the arm-not-built
// panic below is equally undecidable under a plain `cargo miri test`, which
// builds default features — the recipe in `scripts/tb_neg_gate.sh` passes both.
// That script passes `--include-ignored`, and the census requires it to.
#[allow(clippy::assertions_on_constants)]
#[cfg_attr(
    not(all(miri, feature = "tb-neg-m2w")),
    ignore = "miri-arm: decides a Tree-Borrows property that only Miri can install, under an arm \
              that only `--features tb-neg-m2w` builds. Its success is an ABORT, so it is green in \
              no configuration whatsoever and would otherwise stand permanently red in \
              `cargo test --workspace`. Run it through `scripts/tb_neg_gate.sh` (or `.ps1`), which \
              supplies Miri, the feature, the four seeds and `--include-ignored`, and reads the \
              verdict off the receipts in `docs/threadpool/receipts/`. KE16 exit condition 6."
)]
#[test]
fn a_protector_over_chunk_memory_held_across_the_release_is_caught_by_tree_borrows() {
    assert!(
        UNDER_MIRI,
        "this binary decides a Tree-Borrows property and NOTHING natively: the protector it hunts \
         is a Tree-Borrows object that only Miri installs, and the free it races is a few \
         instructions wide without Miri's forced schedule. A native run must be RED rather than \
         quietly green — run it through `scripts/tb_neg_gate.ps1`"
    );

    #[cfg(not(feature = "tb-neg-m2w"))]
    panic!(
        "tb-neg-m2w: arm not built. This binary is the NEGATIVE control for KE16 M2w and it \
         cannot control for anything without `run_scoped_neg` in `Task::execute`. Build it with \
         `--features tb-neg-m2w` (Miri only — `src/lib.rs` refuses the feature natively). \
         `running 1 test` plus this red is the intended outcome of a default build: it is what \
         keeps the absence of the arm from reading as its presence."
    );

    #[cfg(feature = "tb-neg-m2w")]
    armed::drive();
}

/// The driver, compiled only with the arm it drives.
///
/// Feature-gated as a whole rather than per item so that a build without
/// `tb-neg-m2w` carries neither an unused import nor an uncalled function — the
/// test function above stays unconditional either way, which is the property
/// that keeps `running 1 test` true in every configuration.
#[cfg(feature = "tb-neg-m2w")]
mod armed {
    use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};

    use boyko_threadpool::{ThreadPoolBuilder, WORKER_ID_DISPATCHER, current_worker_id};

    /// Worker threads. Two is enough — the window is per scope and needs exactly
    /// one off-joiner completer — and every extra thread multiplies Miri's
    /// interleaving cost. Same value and same reason as the positive gate.
    const WORKERS: usize = 2;

    /// Bodies per scope. MUST equal `WORKERS`: the shape waits for every body to
    /// be claimed before releasing them, so a body with no free worker to claim
    /// it would never announce itself and the bounded spin would fail the run.
    const TASKS_PER_SCOPE: usize = WORKERS;

    /// Scopes driven. ONE window is enough to abort the interpreter, so this is a
    /// margin against a single anomalous scope rather than a sample size — and it
    /// is small because Miri's per-pool cost is superlinear (the sibling probe
    /// measured an exponent near 2.1).
    const SCOPES: usize = 4;

    /// Iterations before a cooperative spin gives up. Same convention and value
    /// as `miri_scope.rs::spin_until` and the positive gate: each turn yields, so
    /// a healthy run needs a handful and an unhealthy one FAILS instead of
    /// hanging Miri forever.
    const SPIN_CAP: usize = 100_000;

    /// Bounded cooperative spin on an arbitrary condition.
    fn spin_until(mut cond: impl FnMut() -> bool, ctx: &str) {
        for _ in 0..SPIN_CAP {
            if cond() {
                return;
            }
            std::thread::yield_now();
        }
        panic!("spin_until timed out waiting for {ctx} — no cross-thread progress");
    }

    /// True iff `id` denotes a genuine worker thread — not the dispatcher
    /// sentinel an `install` frame carries, and not the unattached sentinel.
    fn ran_on_worker(id: u32) -> bool {
        id != WORKER_ID_DISPATCHER && id != u32::MAX
    }

    /// Reads the M2w counter — chunk frees that landed inside a completer's open
    /// release window — or 0 natively where there is none.
    ///
    /// Diagnostic only. It is what tells a reader of a green run WHICH way the
    /// control failed: `block_overlaps == 0` means the free never landed in a
    /// window, so the arm was never offered its chance and the recipe is the
    /// suspect; `block_overlaps >= 1` with no UB report means the free DID land
    /// in a window and Tree Borrows did not object, which refutes M2w itself.
    fn block_frees_inside_window() -> usize {
        #[cfg(miri)]
        {
            boyko_threadpool::miri_block_frees_inside_a_release_window()
        }
        #[cfg(not(miri))]
        {
            0
        }
    }

    /// Drives `SCOPES` dispatcher-joined scopes in which the last completion is a
    /// worker's by construction, so each one's `pending -> 0` races that scope's
    /// `ScopeBlock::free_all`.
    ///
    /// Does not return: either Miri aborts inside `free_all`, or the terminal
    /// `panic!` reports that the negative control stopped being negative.
    pub(super) fn drive() {
        let block_frees_before = block_frees_inside_window();
        // Every body of every scope executed — the primary anti-vacuity census.
        let ran = AtomicUsize::new(0);
        // Scopes whose last-finishing body ran on a genuine worker.
        let mut windows = 0usize;

        // One pool for the whole run: at this volume the superlinear per-scope
        // cost has not begun to matter, and rebuilding would add worker start-up
        // to every scope's schedule.
        let pool = ThreadPoolBuilder::new().num_threads(WORKERS).build();

        for k in 0..SCOPES {
            let started = AtomicUsize::new(0);
            let go = AtomicBool::new(false);
            let done = AtomicUsize::new(0);
            let last_wid = AtomicU32::new(u32::MAX);

            // `install` from the test's own thread rewrites `CURRENT_WORKER_ID`
            // to `WORKER_ID_DISPATCHER`, so `ScopeShared::new` is handed a null
            // W-d' target and every completion takes the EXTERNAL-joiner arm —
            // the arm the default build ships.
            pool.install(|scope| {
                let ran = &ran;
                let started = &started;
                let go = &go;
                let done = &done;
                let last_wid = &last_wid;
                for _ in 0..TASKS_PER_SCOPE {
                    scope.spawn(move || {
                        started.fetch_add(1, Ordering::AcqRel);
                        spin_until(|| go.load(Ordering::Acquire), "the go gate");
                        ran.fetch_add(1, Ordering::Relaxed);
                        // The body that observes `TASKS_PER_SCOPE - 1`
                        // predecessors is the last to finish; its thread is the
                        // one whose completion drives `pending` to zero, and
                        // therefore the one whose `finish_neg` frame is open when
                        // the joiner frees the chunk.
                        if done.fetch_add(1, Ordering::AcqRel) == TASKS_PER_SCOPE - 1 {
                            last_wid.store(current_worker_id(), Ordering::Release);
                        }
                    });
                }

                // THE STRUCTURAL PART. Returning only once every body has
                // announced itself means every body is claimed by a genuine
                // worker before the join below begins, so the joiner has nothing
                // to steal and cannot become a completer. That is what makes the
                // last decrement a worker's in every scope instead of in a lucky
                // fraction of them.
                spin_until(
                    || started.load(Ordering::Acquire) == TASKS_PER_SCOPE,
                    "every body to be claimed by a worker",
                );
                go.store(true, Ordering::Release);
            });

            assert_eq!(
                done.load(Ordering::Acquire),
                TASKS_PER_SCOPE,
                "scope {k}: the join returned with bodies still unfinished"
            );
            if ran_on_worker(last_wid.load(Ordering::Acquire)) {
                windows += 1;
            }
        }

        let bodies = ran.load(Ordering::Acquire);
        let block_overlaps = block_frees_inside_window().saturating_sub(block_frees_before);

        // ONE `write_all` of a pre-formatted line, not `eprintln!`: the driver
        // runs one process per seed against the same unbuffered stderr when a
        // reader tails several receipts, and a multi-fragment `write_fmt`
        // interleaves mid-line (measured 2026-09-04 on the sibling probe).
        let census = format!(
            "KE16-TB-NEG-M2W-CENSUS scopes={SCOPES} tasks_per_scope={TASKS_PER_SCOPE} \
             workers={WORKERS} bodies={bodies} windows={windows} \
             block_overlaps={block_overlaps}\n"
        );
        {
            use std::io::Write as _;
            let mut err = std::io::stderr().lock();
            let _ = err.write_all(census.as_bytes());
            let _ = err.flush();
        }

        panic!(
            "tb-neg-m2w: the negative control RAN TO COMPLETION and Miri reported no Undefined \
             Behavior. A negative control that goes green has stopped being negative, and the \
             positive gate's green now rests on nothing. Read the census line above to tell the \
             two failures apart: block_overlaps={block_overlaps} of {SCOPES} scopes — 0 means no \
             chunk free ever landed inside a completer's release window, so the arm was never \
             offered its chance and the RECIPE is the suspect (`-Zmiri-preemption-rate=0`, \
             `-Zmiri-tree-borrows`, the yield burst in `ScopeShared::complete_task`); >= 1 means \
             the free DID land inside an open window while `finish_neg` held a reference into \
             that chunk and Tree Borrows did not object, which refutes M2w itself. windows={windows} \
             of {SCOPES} says whether the shape still puts the last completion on a worker at all."
        );
    }
}
