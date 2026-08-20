//! **Profiling rung 8 — `G4c`: the loss reaches the reader.**
//!
//! `G4b` gates that the fold's accumulation is lossless — an injected N reads back as exactly N.
//! Its own row says what it cannot claim: *"that a reader acts on the figure, which is `G4c`'s."*
//! This file is `G4c`.
//!
//! # The two tallies, and why comparing them is not comparing a number with itself
//!
//! A retired GPU pair that came back `Lost` or `Torn` is counted **twice, by two independent
//! mechanisms**:
//!
//! 1. `WindowReducer`'s own [`LabelCensus`] — a `u32` per label, folded from the pairs the recorder
//!    handed it, living in the reducer and written into the artifact beside the zone rows.
//! 2. `boyko_diag`'s process-wide loss cell for [`LossClass::Device`] — bumped at the same site,
//!    read back at write time by `Artifact::collect_losses`.
//!
//! They are two tallies of one fact. That is the point: a gate comparing the census to itself
//! would be a tautology, and a gate comparing the census to nothing would be `G4b` again. The
//! artifact carries **both**, so a reader can see them agree — and a writer that stops recording
//! one of them makes them disagree, which is this file's RED.
//!
//! # What this gate CANNOT claim
//!
//! It cannot claim the drops were *real* — only that the count reached the file. And it cannot run
//! against a GPU: these are host-side folds over synthetic `PairResult`s, so what is exercised is
//! the accounting, not the recorder. The recorder's own labelling is `gpu_zone_label_control.rs`'s.

#![cfg(windows)]

// The ban's own escape shape: an allow beside the use, with the rationale at `DEVICE_CELL`.
#[allow(clippy::disallowed_types)]
use std::sync::Mutex;

use boyko_app::profiling::artifact::Artifact;
use boyko_app::profiling::reduce::WindowReducer;
use boyko_diag::loss::{LOSS_ROW_COUNT, LossClass, cell_at_row};
use boyko_rhi_vulkan::present::gpu_zone::{GpuLabel, PairResult};

/// The wire word `LossClass::Device` renders as.
///
/// ⚠️ **`"Device"`, capitalised — and this gate caught me writing `"device"`.** `boyko_diag`'s
/// `as_str` table is the vocabulary's single spelling, and its own doc says why the table exists at
/// all: *"the value of the vocabulary is that two artifacts a reader joins use the same eight
/// words."* A ninth spelling of one of them defeats exactly that, and it is invisible to every
/// other check — the artifact would have parsed, round-tripped and read fine while joining against
/// nothing. Spelled once here so a rename reds in one place rather than in none.
const DEVICE: &str = "Device";

/// **Serialises the three tests in this binary, and the reason is a measured red.**
///
/// The `Device` cell is a PROCESS-GLOBAL monotone counter, and every test here measures a DELTA
/// across its own fold. `cargo test` runs a binary's tests concurrently, so under the parallel
/// sweep each test's `before`/`after` pair straddled the others' recordings and all three failed —
/// while every targeted run with `--test-threads=1` passed. **A gate that reads a process-global
/// counter as a delta requires that nothing else writes it between the two reads**, and that is a
/// property of the harness unless something here enforces it.
///
/// Serialising is the fix rather than `--test-threads=1`, because the flag is passed by whoever
/// runs the suite and this file cannot require it: a gate whose correctness depends on an
/// invocation flag is green or red by how it was called, which is the same defect one level up.
///
/// `#[allow(clippy::disallowed_types)]`: the hot-path ban on `Mutex` is about the engine's frame
/// path. This is a `#[cfg(test)]` binary's cross-test serialiser, held across three host-side folds
/// that touch no GPU and no scheduler — the ban's own listed exception shape.
#[allow(clippy::disallowed_types)]
static DEVICE_CELL: Mutex<()> = Mutex::new(());

/// Sums `boyko_diag`'s `Device` cells across every lane row, the way the artifact writer does.
fn device_total() -> u64 {
    (0..LOSS_ROW_COUNT).map(|row| cell_at_row(row, LossClass::Device).count()).sum()
}

/// One synthetic retired pair.
fn pair(zone: u16, label: GpuLabel) -> PairResult {
    PairResult { zone, label, begin_ticks: 1_000, dur_ticks: 100 }
}

/// **The clause.** Every drop the reducer folded is named in the artifact with its class and its
/// count, and the two tallies agree.
///
/// The test folds a window whose labels are known, then compares the artifact's `[[loss]]` row
/// against the label census's own `lost + torn`. Both come out of the same window; neither is
/// derived from the other.
///
/// ⚠️ **The `Device` cell is process-wide and monotone**, so this measures the DELTA across the
/// fold rather than the absolute total: another test in this binary may have folded drops of its
/// own. Taking the absolute would make the gate depend on test ordering, which is a property of the
/// harness rather than of the code under test.
#[test]
fn every_drop_the_reducer_folded_is_named_in_the_artifact() {
    // A poisoned lock means another test in this binary panicked while holding it; its own failure
    // is the report, and re-panicking here would bury it under a second one.
    let _serial = DEVICE_CELL.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let before = device_total();

    let mut r = WindowReducer::new(1.0, &[], &[]);
    r.observe_frame(&[
        pair(0, GpuLabel::Measured),
        pair(1, GpuLabel::Lost),
        pair(2, GpuLabel::Torn),
        // NOT a loss, and this row is why the test carries it: a pair the recorder never opened is
        // a STATED ABSENCE. Counting it would make "this leg does not run that pass" and "the
        // numbers went missing" the same observation.
        pair(3, GpuLabel::NotBracketed),
    ]);
    r.observe_frame(&[pair(1, GpuLabel::Lost)]);
    let (_, census, _order) = r.finish();

    let folded_drops = u64::from(census.lost + census.torn);
    assert_eq!(folded_drops, 3, "the fixture folds two Lost and one Torn");
    assert_eq!(census.not_bracketed, 1, "the unbracketed pair is counted, and NOT as a drop");

    let recorded = device_total() - before;
    assert_eq!(
        recorded, folded_drops,
        "`boyko_diag`'s Device cell and the label census disagree about the SAME window. One of \
         the two tallies stopped counting -- which is exactly the failure G4c exists to make \
         visible, because until rung 8 the artifact carried only the census and a silent stop \
         would have looked like a clean run"
    );
}

/// **The artifact names the class**, not merely a number: a count with no vocabulary cannot be
/// joined against any other artifact, which is what the eight shared words in
/// `boyko_diag::loss::LossClass` are for.
#[test]
fn the_artifact_names_the_class_and_absent_means_zero() {
    // A poisoned lock means another test in this binary panicked while holding it; its own failure
    // is the report, and re-panicking here would bury it under a second one.
    let _serial = DEVICE_CELL.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let before = device_total();
    let mut r = WindowReducer::new(1.0, &[], &[]);
    r.observe_frame(&[pair(0, GpuLabel::Lost)]);
    let _ = r.finish();
    assert_eq!(device_total() - before, 1);

    let rows = Artifact::collect_losses();
    let device = rows
        .iter()
        .find(|l| l.class == DEVICE)
        .expect("a folded drop must produce a `device` row");
    assert!(device.count > 0, "the row must carry the count, not just the class");
    assert_eq!(device.bytes, 0, "a lost timestamp pair has no meaningful payload figure");

    // A class nothing recorded gets NO ROW, and `loss_count` says so without the caller deciding.
    for class in LossClass::ALL {
        let named = rows.iter().any(|l| l.class == class.as_str());
        let total: u64 =
            (0..LOSS_ROW_COUNT).map(|row| cell_at_row(row, class).count()).sum();
        assert_eq!(
            named,
            total != 0 || (0..LOSS_ROW_COUNT).any(|r| cell_at_row(r, class).bytes() != 0),
            "class `{}` is {} in the artifact but its cells say otherwise",
            class.as_str(),
            if named { "present" } else { "absent" }
        );
    }
}

/// **`LabelCensus` and the loss cell must disagree if either stops**, and the disagreement has to
/// be VISIBLE rather than absorbed.
///
/// ⚠️ **THE RED, RUN — and its number is not the one I predicted.** Deleting
/// `WindowReducer::record_device_loss`'s call from the `Lost` arm makes the first test above fail
/// with **`left: 1, right: 3`**, not the `2 != 3` this doc first claimed: the fixture folds TWO
/// `Lost` and one `Torn`, so removing the `Lost` recording leaves the cell counting one. The
/// second test reds too, at `0 != 1`. Recording the MEASURED figures rather than the predicted
/// ones, because a gate described by the failure it was expected to have is a gate nobody ran.
///
/// The artifact then reports a `Device` row that under-states what the same file's census says a
/// few lines above it. The RED is a one-line deletion in `reduce.rs`; it is not injectable from a
/// test, because the recording site is private and that is deliberate — a public setter would be a
/// way to write a loss that never happened.
///
/// This test therefore asserts the WEAKER thing it can assert without that deletion: that the two
/// tallies are wired to the same event, by folding a window with drops and one without, and
/// checking that only the first moves both. A test that could inject the failure would need the
/// mechanism the failure is about.
#[test]
fn a_window_with_no_drops_moves_neither_tally() {
    // A poisoned lock means another test in this binary panicked while holding it; its own failure
    // is the report, and re-panicking here would bury it under a second one.
    let _serial = DEVICE_CELL.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let before = device_total();
    let mut r = WindowReducer::new(1.0, &[], &[]);
    r.observe_frame(&[pair(0, GpuLabel::Measured), pair(1, GpuLabel::NotBracketed)]);
    let (_, census, _order) = r.finish();

    assert_eq!(census.lost + census.torn, 0, "no drops in this window");
    assert_eq!(
        device_total() - before,
        0,
        "a clean window must not bump the Device cell -- a counter that moves on a frame with no \
         loss reports drops the recorder never saw, and every artifact would carry them forever"
    );
}
