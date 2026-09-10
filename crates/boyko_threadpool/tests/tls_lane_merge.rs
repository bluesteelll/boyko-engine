//! D7's gate: `worker_lane_for` reads the thread-local lane slot EXACTLY ONCE.
//!
//! This is a SOURCE-SHAPE test, and it is one on purpose. The property it pins
//! has no behavioural signature at all: a `worker_lane_for` that asked the TLS
//! twice — say by calling `current_worker_id()` for the id half, the shape this
//! crate had before the merge — returns the same answer for every input, so
//! every routing test, every occupancy receipt and every Miri model stays green
//! while the merge's whole purpose is undone. What changes is the COST, and the
//! cost is the reason the merge exists: under rustc >= 1.98.0 on
//! `x86_64-pc-windows-gnu` each `thread_local!` access is two lock-prefixed
//! RMWs on one process-global cache line plus a `kernel32!FlsSetValue` call
//! (`docs/threadpool/RUSTC-198-WINDOWS-GNU-TLS.md`), and `worker_lane_for` sits
//! on the spawn path of every task.
//!
//! A timing test cannot gate it either: the difference is tens of nanoseconds
//! per spawn, under the noise floor of anything this suite can run on a
//! developer's box, and the owner's benchmark box is not part of `cargo test`.
//! So the gate reads the source.
//!
//! # What makes each row RED
//!
//! - `worker_lane_for_reads_the_lane_slot_exactly_once`: add a second
//!   `lane_deposit()` / `LANE_DEPOSIT.with` / `current_worker_id()` call to
//!   `worker_lane_for`'s body — for instance by restoring the pre-merge
//!   `let wid = current_worker_id();`. That mutation leaves the rest of the
//!   crate green, which is precisely why this row exists.
//! - `lane_deposit_is_the_only_reader_of_the_slot_on_the_spawn_path`: give
//!   `worker_lane_for` its own `LANE_DEPOSIT.with` instead of going through
//!   the one accessor.
//! - `the_two_replaced_slots_are_gone`: reintroduce a `WORKER_DEQUE` or
//!   `CURRENT_WORKER_ID` `thread_local!`, i.e. un-merge the state.
//! - `the_id_writers_never_write_the_deque_half`: write `c.set(LaneDeposit {
//!   wid: id, ..LaneDeposit::DETACHED })` in an id writer.
//! - `the_source_under_test_is_the_one_that_ships`: the anti-vacuity row. It
//!   goes red if `tls.rs` moves, is renamed, or stops containing the function
//!   this file claims to inspect — the failure mode where a source-shape test
//!   reads nothing and reports success.

use std::path::PathBuf;

/// The file whose shape the rows below assert. Resolved from
/// `CARGO_MANIFEST_DIR` so the test does not depend on the working directory a
/// runner happens to use.
fn tls_source() -> String {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("src");
    p.push("tls.rs");
    std::fs::read_to_string(&p)
        .unwrap_or_else(|e| panic!("invariant: `{}` must be readable: {e}", p.display()))
}

/// Extract the body of a function by brace balance, starting at the `{` that
/// follows `signature`.
///
/// Returns `None` when the signature is absent, which the anti-vacuity row
/// turns into a failure rather than into a silent "nothing to check". Brace
/// balancing is literal — it would be fooled by a brace inside a string
/// literal, which is why the anti-vacuity row pins the extracted body's length
/// and its closing expression rather than trusting the scan.
fn fn_body<'a>(src: &'a str, signature: &str) -> Option<&'a str> {
    let start = src.find(signature)?;
    let open = start + src[start..].find('{')?;
    let bytes = src.as_bytes();
    let mut depth = 0usize;
    for (i, b) in bytes.iter().enumerate().skip(open) {
        match b {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(&src[open + 1..i]);
                }
            }
            _ => {}
        }
    }
    None
}

/// Strip `//` line comments, so that a comment MENTIONING a TLS accessor is not
/// counted as a call to it. Without this the rows below could not be documented
/// at their own site and stay green.
fn without_line_comments(body: &str) -> String {
    body.lines()
        .map(|l| match l.find("//") {
            Some(i) => &l[..i],
            None => l,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The anti-vacuity row: every other row asserts about text, so it must first
/// be established that the text exists and is the text intended.
///
/// A source-shape test that silently inspects an empty string is this
/// repository's catalogued failure — a check that cannot fail. This row is what
/// lets the others assume `fn_body` returned the real body.
#[test]
fn the_source_under_test_is_the_one_that_ships() {
    let src = tls_source();
    assert!(
        src.contains("pub(crate) fn worker_lane_for(inner: &PoolInner) -> Option<WorkerLane>"),
        "tls.rs must still define `worker_lane_for` with the signature this file inspects; \
         a rename or a move would make every row below vacuous"
    );
    let body = fn_body(&src, "pub(crate) fn worker_lane_for")
        .expect("invariant: the signature asserted above has a brace-balanced body");
    assert!(
        body.len() > 200,
        "the extracted body is {} bytes, too short to be the real predicate — brace \
         balancing has gone wrong and the rows below would assert about nothing",
        body.len()
    );
    assert!(
        body.contains("Some(WorkerLane {"),
        "the extracted body must reach the lane it mints; got:\n{body}"
    );
}

/// D7, the row the merge exists for: ONE thread-local access on the spawn path.
///
/// Counts every way the slot can be reached from inside `worker_lane_for` — the
/// accessor, the static itself, and the public id reader that used to be the
/// second question — and requires the sum to be one.
#[test]
fn worker_lane_for_reads_the_lane_slot_exactly_once() {
    let src = tls_source();
    let body = without_line_comments(
        fn_body(&src, "pub(crate) fn worker_lane_for")
            .expect("invariant: pinned by `the_source_under_test_is_the_one_that_ships`"),
    );

    let via_accessor = body.matches("lane_deposit()").count();
    let via_static = body.matches("LANE_DEPOSIT").count();
    let via_id_reader = body.matches("current_worker_id()").count();
    let total = via_accessor + via_static + via_id_reader;

    assert_eq!(
        total, 1,
        "D7: `worker_lane_for` must touch the lane slot exactly once \
         (accessor {via_accessor}, static {via_static}, id reader {via_id_reader}). \
         Two reads answer identically and cost twice as much, so no behavioural test \
         can see this. Body:\n{body}"
    );
    assert_eq!(
        via_id_reader, 0,
        "D7: the id half must come out of the merged deposit, never from a second \
         `current_worker_id()` read"
    );
}

/// D7's other half: the one read goes through the one accessor, so the count
/// above has a single place in which to be wrong.
#[test]
fn lane_deposit_is_the_only_reader_of_the_slot_on_the_spawn_path() {
    let src = tls_source();
    let body = without_line_comments(
        fn_body(&src, "pub(crate) fn worker_lane_for")
            .expect("invariant: pinned by `the_source_under_test_is_the_one_that_ships`"),
    );
    assert!(
        body.contains("lane_deposit()"),
        "the predicate must read through `lane_deposit()`; body:\n{body}"
    );
    assert!(
        !body.contains("LANE_DEPOSIT.with"),
        "the predicate must not open the slot itself — that is what `lane_deposit()` is for; \
         body:\n{body}"
    );
}

/// The merge itself: the two slots it replaced must not come back.
///
/// Separate from the count above because the two fail for different reasons —
/// this row catches a re-split of the STATE, that one catches a second READ of
/// the merged state.
#[test]
fn the_two_replaced_slots_are_gone() {
    let src = tls_source();
    assert!(
        src.contains("static LANE_DEPOSIT: Cell<LaneDeposit>"),
        "the merged slot must exist"
    );
    for gone in ["static WORKER_DEQUE", "static CURRENT_WORKER_ID"] {
        assert!(
            !src.contains(gone),
            "`{gone}` is back: the spawn path is paying two thread-local accesses again"
        );
    }
}

/// D6 read off the source: the deque guard and the id writers touch DISJOINT
/// fields, and no writer writes the struct whole.
///
/// The behavioural gates for this live in `tls.rs`'s own module tests (the
/// `0b101` install-frame row, and the deposit guard's two `wid`-unchanged
/// rows). This is their cheap structural echo: it goes red on the exact edit —
/// `c.set(LaneDeposit { .. })` inside an id writer — that those rows exist to
/// catch, and it states the field discipline where a reader looks first.
#[test]
fn the_id_writers_never_write_the_deque_half() {
    let src = tls_source();
    for (sig, who) in [
        (
            "pub(crate) fn set_current_worker_id",
            "set_current_worker_id",
        ),
        (
            "pub(crate) fn clear_current_worker_id",
            "clear_current_worker_id",
        ),
    ] {
        let body = without_line_comments(
            fn_body(&src, sig).unwrap_or_else(|| panic!("invariant: `{who}` must exist")),
        );
        assert!(
            body.contains("d.wid = "),
            "D6: `{who}` must write the `wid` FIELD; body:\n{body}"
        );
        assert!(
            !body.contains("LaneDeposit {") && !body.contains("LaneDeposit::DETACHED"),
            "D6: `{who}` must not write the deposit whole — that nulls the pool/deque half \
             and silently sends every later spawn from that worker to the global injector; \
             body:\n{body}"
        );
    }
}
