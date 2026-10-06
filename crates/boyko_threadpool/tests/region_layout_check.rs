//! SR: the region harness's claim-layout check gets a gate of its own (review r6 W1, tester r7 F1).
//!
//! `Frame::region_frame::<P>` runs `assert_table_fits::<P>` before `RegionFrame::new`: a table that
//! `set_table` laid out for another policy or participant count than the region runs is refused
//! with a panic instead of being handed to the protocol. Every region binary calls it, but only
//! with tables that fit, so on its own the check passes at every call site, natively and under
//! Miri: removed or weakened, it would read green everywhere. Its only observed red was the one-table
//! Miri SB leg of `c595b3ab`'s parent (`384ce8f5`, `threads_t1_t2_exactly_once_and_visibility`),
//! which no longer exists. Each test below builds a table that does NOT fit (or, for the
//! anti-vacuity partner, tables that fit exactly) and names the mutation of the check that turns
//! it red:
//!
//! * `W1-CHECK-OFF`: the check returns at once;
//! * `W1-OVERLAP-WEAK`: `end_a <= end_b` for `end_a <= first_b`, so a partial overlap goes unseen;
//! * `W1-STRICT`: `end_a < first_b`, so adjacent ranges (every legal table has them) read as shared;
//! * `W1-COLUMN-OFF` / `W1-ENTRY-OFF`: the claim-column clause / the missing-entry clause removed.
//!
//! The Miri SB leg's own table discipline (`W1-LEG-ONE-TABLE`, `W1-THREADS-FIXED-P`) is gated by
//! that leg, not here. Tiny frames, no threads, and no region ever runs over a returned frame, so
//! every test runs natively and under Miri, Tree Borrows (the workspace's `MIRIFLAGS`) and Stacked
//! Borrows (an explicit `MIRIFLAGS` without `-Zmiri-tree-borrows`):
//!
//!   cargo +nightly-x86_64-pc-windows-msvc miri test -p boyko-threadpool --test region_layout_check

mod region_common;

use boyko_threadpool::{SchedItem, StageEntry};

use region_common::{AllAxesPolicy, Frame, HomePolicy, TestPolicy};

/// The parent's W1 shape: a table laid out by `set_table::<AllAxesPolicy>` (home lines: one claim
/// line per participant, so two 4-block entries take lines 0..2 and 2..4), then checked for
/// `TestPolicy` (one claim line per block: entry 0 would run lines 0..4, entry 1 lines 2..6).
///
/// RED under `W1-CHECK-OFF` (no panic) and `W1-OVERLAP-WEAK` (`end_a <= end_b`: 4 <= 6 holds, so a
/// partial overlap goes unseen).
#[test]
#[should_panic(expected = "share claim lines under this policy at 2 participants")]
fn region_frame_rejects_a_table_laid_out_for_another_policy() {
    let mut frame = Frame::new(2, 2, 16);
    frame.set_table::<AllAxesPolicy>(&[4, 4], &[0, 1], 2, 0);
    // SAFETY: `frame` is this test's own local; its columns and epoch were built together by
    // `Frame::new` and only `set_table` has written to it since (contents, never storage), so they
    // are still this `Frame`'s alone and the counter was never rewound. No region ever runs over the
    // returned frame, so the policy condition of `region_frame`'s contract holds vacuously; the call
    // is expected to panic in `assert_table_fits`, before the unsafe constructor.
    let _ = unsafe { frame.region_frame::<TestPolicy>(2) };
}

/// The same table, run for one participant more than it was laid out for: home lines are
/// `participants` lines per entry, so a layout for 2 participants (0..2, 2..4) is 0..3 and 2..5 at 3.
///
/// RED under `W1-CHECK-OFF` and `W1-OVERLAP-WEAK`.
#[test]
#[should_panic(expected = "share claim lines under this policy at 3 participants")]
fn region_frame_rejects_a_table_laid_out_for_fewer_participants() {
    let mut frame = Frame::new(2, 3, 16);
    frame.set_table::<HomePolicy>(&[4, 4], &[0, 1], 2, 0);
    // SAFETY: `frame` is this test's own local, built by `Frame::new` and written only by
    // `set_table` since, so its columns and epoch are still its own and never rewound; it has 3
    // receipt lines for 3 participants. No region runs over the returned frame, so the policy
    // condition holds vacuously.
    let _ = unsafe { frame.region_frame::<HomePolicy>(3) };
}

/// An entry whose claim lines lie INSIDE another's, built by hand so the overlap does not depend on
/// `claim_lines`' arithmetic or on a policy mismatch: entry 0 owns lines 0..6, entry 1 lines 2..5.
///
/// RED under `W1-CHECK-OFF` (and not under `W1-OVERLAP-WEAK`: the ends run 6 then 5, so even the
/// weak comparison fires here; the partial-overlap tests above are the ones that see that slip).
#[test]
#[should_panic(expected = "share claim lines under this policy at 2 participants")]
fn region_frame_rejects_an_entry_nested_in_another() {
    let mut frame = Frame::new(2, 2, 16);
    frame.set_table::<TestPolicy>(&[6, 3], &[0, 1], 2, 0);
    frame.entries[1] = StageEntry::new(0, 1, 3, 2, 0);
    // SAFETY: `frame` is this test's own local, built by `Frame::new`; `set_table` and the entry
    // rewrite above change table contents, never the columns or the epoch, so those are still its
    // own and never rewound. No region runs over the returned frame, so the policy condition holds
    // vacuously; the call is expected to panic in `assert_table_fits`.
    let _ = unsafe { frame.region_frame::<TestPolicy>(2) };
}

/// Green on a table that does fit: the anti-vacuity partner of the three tests above (the same call
/// shape returns when the policy matches) and the guard against a check that is too strict: legal
/// tables have EXACTLY adjacent ranges (0..2, 2..5, 5..9 here, plus a one-block entry that takes no
/// line), and `AllAxesPolicy`'s home layout is checked for `AllAxesPolicy`.
///
/// RED under `W1-STRICT` (adjacent ranges read as shared).
#[test]
fn region_frame_accepts_a_table_laid_out_for_its_policy() {
    let mut home = Frame::new(2, 2, 16);
    home.set_table::<AllAxesPolicy>(&[4, 4], &[0, 1], 2, 0);
    // SAFETY: `home` is this test's own local, built by `Frame::new` and written only by
    // `set_table` since, so its columns and epoch are still its own and never rewound. Its layout
    // was made for the policy it is checked for, and no region runs over the returned frame.
    let _home_frame = unsafe { home.region_frame::<AllAxesPolicy>(2) };

    let mut per_block = Frame::new(4, 2, 9);
    per_block.set_table::<TestPolicy>(&[2, 3, 1, 4], &[0, 1, 2, 3], 2, 0);
    // SAFETY: `per_block` is this test's own local, built by `Frame::new` and written only by
    // `set_table` since, so its columns and epoch are still its own and never rewound. Its layout
    // was made for the policy it is checked for, and no region runs over the returned frame.
    let _per_block_frame = unsafe { per_block.region_frame::<TestPolicy>(2) };
}

/// The entry's lines for the region's policy end past the claim column: laid out for
/// `AllAxesPolicy` (2 home lines, fits the column of 3), checked for `TestPolicy` (4 lines).
///
/// RED under `W1-COLUMN-OFF` and `W1-CHECK-OFF`.
#[test]
#[should_panic(expected = "pass the column's 3")]
fn region_frame_rejects_a_layout_past_the_claim_column() {
    let mut frame = Frame::new(1, 2, 3);
    frame.set_table::<AllAxesPolicy>(&[4], &[0], 2, 0);
    // SAFETY: `frame` is this test's own local, built by `Frame::new` and written only by
    // `set_table` since, so its columns and epoch are still its own and never rewound. No region
    // runs over the returned frame, so the policy condition holds vacuously; the call is expected
    // to panic in `assert_table_fits`.
    let _ = unsafe { frame.region_frame::<TestPolicy>(2) };
}

/// An item that names an entry the table does not have.
///
/// RED under `W1-ENTRY-OFF` and `W1-CHECK-OFF` (and under `W1-STRICT`, whose overlap clause fires
/// first on this table's adjacent home ranges 0..2 and 2..4, with the wrong message).
#[test]
#[should_panic(expected = "names entry 5 of 2")]
fn region_frame_rejects_an_item_naming_a_missing_entry() {
    let mut frame = Frame::new(2, 2, 16);
    frame.set_table::<AllAxesPolicy>(&[4, 4], &[0, 1], 2, 0);
    frame.schedule.push(SchedItem::new(5, None));
    // SAFETY: `frame` is this test's own local, built by `Frame::new`; `set_table` and the pushed
    // item change table contents, never the columns or the epoch, so those are still its own and
    // never rewound. No region runs over the returned frame, so the policy condition holds
    // vacuously; the call is expected to panic in `assert_table_fits`.
    let _ = unsafe { frame.region_frame::<AllAxesPolicy>(2) };
}
