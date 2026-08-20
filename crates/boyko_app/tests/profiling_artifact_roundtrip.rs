//! **G24 (profiling rung 7) — the artifact round-trips, and a reader refuses a stale one.**
//!
//! `G24`'s green leg: a migrated consumer reads a fresh artifact and gets its numbers back.
//! Its reverse RED: *"point a migrated consumer at a **stale** artifact ⇒ the header's mismatch
//! makes the reader refuse rather than parse"*. Both legs are here, because the corpus's own
//! doctrine is that *"a gate whose green state does not exist is as useless as one whose red state
//! does not"*.
//!
//! # The discriminator is a run token, and that is a correction to `G24`, not a convenience
//!
//! `G24` names `build_hash` and `SessionId` as the header fields whose mismatch makes the reader
//! refuse. Measured against the tree at rung 7's opening:
//!
//! * **`build_hash` does not exist** — `crates/boyko_diag/` has no `build.rs` and `BUILD_HASH`
//!   appears nowhere in the workspace. It is a planned rung-0 artifact that never landed.
//! * **`SessionId` exists but is minted INSIDE the child process**, so a parent cannot predict it.
//!   The only value it could compare against is one the child already told it — which is precisely
//!   what a stale read would have corrupted.
//!
//! And even had `build_hash` existed, it is constant across a whole session: it detects an artifact
//! from a different BUILD, never a stale one from the previous child of the same run — and
//! `vg_decidability_floor.rs` spawns 42 sequential children. So the discriminator is
//! `ArtifactHeader::run_token`, chosen by the parent **before** the child starts. It is the only
//! field that can catch the staleness the gate is about.
//!
//! # What this gate cannot claim
//!
//! Nothing about the NUMBERS being right — it round-trips whatever it was given. That is the
//! artifact's own limit and the reason rung 7b re-measures the floor on this channel rather than
//! reusing the printed one: it is a different instrument. It also claims nothing about a producer,
//! because at this step there is none; the writer's first production caller is the next step, and
//! it is verified by A/B against the still-printing channel while both are live.

use std::path::PathBuf;

use boyko_app::profiling::artifact::{
    LossRow,
    ARTIFACT_SCHEMA_VERSION, Artifact, ArtifactError, ArtifactHeader, Instrument, LabelCensus,
    OrderCensus, PRECISION_DECIMALS, ZoneLabel, ZoneRow,
};
use boyko_app::profiling::correlate::{Correlated, Correlation, Uncorrelated};

/// A scratch path unique to this test binary and case name.
fn scratch(case: &str) -> PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!("boyko_profiling_artifact_{case}.toml"));
    p
}

/// The fixture. Zone ids are the shipped family bases (`ZONE_BASE_GBUFFER = 16`,
/// `ZONE_BASE_SV0 = 32`) so the rows look like the ones a real window produces.
fn fixture(run_token: &str) -> Artifact {
    Artifact {
        header: ArtifactHeader {
            schema_version: ARTIFACT_SCHEMA_VERSION,
            session_lo: 0x0123_4567_89ab_cdef,
            session_hi: 0xfedc_ba98_7654_3211,
            run_token: run_token.to_owned(),
            workload_tag: "vb_mesh_512".to_owned(),
            content_tag: "n14_kronecker".into(),
            // Decision 7: a real window's regime census. `off,forced_on` is two distinct
            // regimes in one window -- the state `vg_occ_split_timing.rs` rejects a worker for,
            // and therefore the one a fixture must be able to represent.
            regimes: "off,forced_on".into(),
            modes: "off".into(),
            regime_n_distinct: 2,
            present_mode: "immediate".to_owned(),
            alloc_shim: false,
            alloc_allocs: 0,
            alloc_deallocs: 0,
            alloc_bytes: 0,
            // NOT the default "neither": a field that round-trips its own zero value proves
            // nothing about the round trip. `profiler+logger` is the state rung 16 baselines in.
            subsystems_tag: "profiler+logger".to_owned(),
            instrument: Instrument::Live,
            precision_decimals: PRECISION_DECIMALS,
            // Rung 9. The fixture carries a MEASURED correlation, not a refusal: the round-trip
            // is the one test that has to prove a number survives the file, and every field is
            // given a distinct value so a writer that transposed two of them reds here. The
            // refusal shape has its own case below.
            correlation: Correlation::Correlated(Correlated {
                offset_ns: -1_234_567_890_123,
                bracket_ns: 141,
                driver_ns: 88,
                accepted: 29,
                rejected: 3,
                epoch: 1,
                drift_ns: -57,
                span_ns: 2_016_000_000,
            }),
        },
        zones: vec![
            ZoneRow {
                zone: 17,
                label: ZoneLabel::Measured,
                n: 10,
                // The ORDER STATISTICS sit on a 12.8 ns lattice (k = 10 and 21, coprime, so their
                // GCD is the quantum itself and not a multiple of it — the rung-4 lesson that two
                // readings establish a common multiple, never a step). The MEAN deliberately does
                // not: see the precision clause below for why that matters.
                median_ns: 128.0,
                mean_ns: 163.2,
                p95_ns: 268.8,
                stddev_ns: 0.0,
                begin_off_ns: 0.0,
                end_off_ns: 128.0,
            },
            ZoneRow {
                zone: 32,
                label: ZoneLabel::NotBracketed,
                n: 10,
                median_ns: 0.0,
                mean_ns: 0.0,
                p95_ns: 0.0,
                stddev_ns: 0.0,
                begin_off_ns: 25_584.0,
                end_off_ns: 25_584.0,
            },
        ],
        census: LabelCensus { measured: 10, not_bracketed: 10, lost: 2, torn: 1 },
        // VB-SV0 DP6-0b: this fixture declares no chain, so the block it round-trips is the
        // `frames_checked = 0` one — the state a reader must be able to tell from a pass.
        order: OrderCensus::default(),
        // Profiling rung 8, `G4c`: three drops, and the `Device` row that must accompany them.
        // `lost + torn == 3` is the label census's count of the SAME events -- two tallies of one
        // fact, which is what the cross-check gate below compares.
        losses: vec![LossRow { class: "Device".to_owned(), count: 3, bytes: 0 }],
    }
}

/// **G24, green leg.** A fresh artifact round-trips through the file, field for field.
#[test]
fn a_fresh_artifact_round_trips_through_the_file() {
    let path = scratch("fresh");
    let written = fixture("run-A");
    written.write(&path).expect("invariant: the artifact writes");

    let read = Artifact::read(&path, "run-A").expect("a fresh artifact parses");
    assert_eq!(read.header, written.header, "the header did not survive the round trip");
    assert_eq!(read.census, written.census, "the label census did not survive the round trip");
    assert_eq!(read.order, written.order, "the [order] block did not survive the round trip");
    assert_eq!(read.zones.len(), written.zones.len());
    for (a, b) in read.zones.iter().zip(written.zones.iter()) {
        assert_eq!(a, b, "a zone row did not survive the round trip");
    }

    let _ = std::fs::remove_file(&path);
}

/// **VB-SV0 DP6-0b.** A POPULATED `[order]` block survives the file — every field non-default.
///
/// # A zero-valued block round-trips through a parser that reads none of it
///
/// The fixture above carries `OrderCensus::default()`, so `read.order == written.order` holds
/// whether the writer emits the block, whether the reader parses it, and whether either of them
/// handles a single key — all four combinations produce two equal all-zero structs. That is the
/// partial-parse false green, and it is exactly the defect class this rung exists to close: a gate
/// satisfied by the absence of the thing it checks.
///
/// So this one populates all five fields with values nothing defaults to, and asserts the block
/// both textually (the keys are IN the file, with the array rendered as an array) and structurally.
#[test]
fn a_populated_order_block_survives_the_file() {
    let path = scratch("order_populated");
    let mut written = fixture("run-ORDER");
    written.order = OrderCensus {
        frames_checked: 217,
        frames_skipped: 4,
        violations: 3,
        worst_ns: 1_536.0,
        derived_inconclusive: vec![14, 27],
    };
    written.write(&path).expect("invariant: the artifact writes");

    let text = std::fs::read_to_string(&path).expect("the file is readable");
    assert!(text.contains("[order]"), "the block header must be in the file:\n{text}");
    assert!(text.contains("frames_checked = 217"), "{text}");
    assert!(text.contains("frames_skipped = 4"), "{text}");
    assert!(text.contains("violations = 3"), "{text}");
    assert!(
        text.contains("derived_inconclusive = [14, 27]"),
        "the id list must render as a TOML array, not as a debug-formatted Vec:\n{text}"
    );

    let read = Artifact::read(&path, "run-ORDER").expect("a populated order block parses");
    assert_eq!(
        read.order, written.order,
        "the [order] block did not survive the round trip field for field"
    );
    // Named individually as well: a `PartialEq` over a struct whose fields all defaulted on both
    // sides would pass, and the equality above cannot say which field carried the file.
    assert_eq!(read.order.frames_checked, 217);
    assert_eq!(read.order.frames_skipped, 4);
    assert_eq!(read.order.violations, 3);
    assert!((read.order.worst_ns - 1_536.0).abs() < 0.05, "worst_ns was {}", read.order.worst_ns);
    assert_eq!(read.order.derived_inconclusive, vec![14, 27]);

    let _ = std::fs::remove_file(&path);
}

/// **W4's close-time refusal.** An `[order]` block that OPENED and omitted a verdict key is
/// MALFORMED, not a block saying "nothing was checked".
///
/// The three keys are required together because their default combination —
/// `frames_checked = 0`, `violations = 0` — is the one every gate reads as INCONCLUSIVE. A
/// truncated block that defaulted into it would look like a deliberate refusal, which is the
/// strongest possible disguise for a broken writer.
#[test]
fn an_incomplete_order_block_is_refused() {
    let mut written = fixture("run-ORDER2");
    written.order = OrderCensus {
        frames_checked: 100,
        frames_skipped: 0,
        violations: 2,
        worst_ns: 512.0,
        derived_inconclusive: Vec::new(),
    };
    let full = written.render();

    for (key, line) in [
        ("frames_checked", "frames_checked = 100"),
        ("violations", "violations = 2"),
        ("derived_inconclusive", "derived_inconclusive = []"),
        ("worst_ns", "worst_ns = 512.0"),
    ] {
        let stripped: String =
            full.lines().filter(|l| l.trim() != line).map(|l| format!("{l}\n")).collect();
        assert!(stripped.len() < full.len(), "`{line}` was not in the rendered file:\n{full}");
        let err = Artifact::parse(&stripped, "run-ORDER2")
            .expect_err(&format!("an [order] block missing `{key}` must be refused"));
        match err {
            ArtifactError::BadHeader(k) => assert!(
                k.contains(key),
                "the refusal must name the missing key; it named `{k}` for `{key}`"
            ),
            other => panic!("expected BadHeader for the missing `{key}`, got {other:?}"),
        }
    }
}

/// An unknown key inside `[order]` ENDS the block instead of eating the keys after it.
///
/// The parser's own doc claims section-order independence. A `_ => {}` inside the block would have
/// made that false in the one direction nobody tests: every flat header key written after an
/// `[order]` block would be swallowed, and the parse would then fail on a header field that IS in
/// the file — or, worse, succeed with a defaulted one.
#[test]
fn an_unknown_order_key_does_not_swallow_the_keys_after_it() {
    let written = fixture("run-ORDER3");
    let full = written.render();
    // Re-emit with the `[order]` block FIRST and an unknown key inside it, followed by every flat
    // header key. If the block swallowed them, the parse loses `session_lo` and fails.
    let mut header_keys = Vec::new();
    let mut rest = Vec::new();
    for l in full.lines() {
        if l.starts_with("[[") || l.starts_with("[order]") {
            rest.push(l);
        } else if rest.is_empty() && l.contains(" = ") {
            header_keys.push(l);
        } else {
            rest.push(l);
        }
    }
    let mut text = String::from("[order]\nframes_checked = 5\nviolations = 0\n");
    text.push_str("derived_inconclusive = []\nan_unknown_future_key = 7\n");
    for l in &header_keys {
        text.push_str(l);
        text.push('\n');
    }
    for l in &rest {
        if l.trim() == "[order]" || l.trim().starts_with("frames_") || l.trim().starts_with("violations")
            || l.trim().starts_with("worst_ns") || l.trim().starts_with("derived_inconclusive")
        {
            continue;
        }
        text.push_str(l);
        text.push('\n');
    }
    let read = Artifact::parse(&text, "run-ORDER3")
        .expect("the header keys after an unknown [order] key must still be parsed");
    assert_eq!(read.order.frames_checked, 5, "the block's own keys still parsed");
    assert_eq!(
        read.header.session_lo, written.header.session_lo,
        "a header key written after the [order] block was swallowed by it"
    );
}

/// **Rung 9.** The correlation survives the file in BOTH shapes, and the refusal keeps D14's own
/// literal in the value.
///
/// Two claims a single equality assert would not separate. The green fixture above already proves
/// a `Correlated` round-trips inside the whole-header comparison; what this adds is (1) the
/// REFUSAL shape, which has no numbers to carry and must therefore be legible from the word alone,
/// and (2) that the word `UNCORRELATED` is textually in the file — a reader or a grep looking for
/// D14's spelling has to find it.
#[test]
fn the_correlation_survives_the_file_in_both_shapes() {
    for reason in [
        Uncorrelated::Unsupported,
        Uncorrelated::NoProbeSurvived,
        Uncorrelated::EpochBreak,
        Uncorrelated::CpuUnscaled,
        Uncorrelated::DeviceUnscaled,
    ] {
        let path = scratch(&format!("corr_{}", reason.as_str()));
        let mut written = fixture("run-C");
        written.header.correlation = Correlation::Uncorrelated(reason);
        written.write(&path).expect("invariant: the artifact writes");

        let text = std::fs::read_to_string(&path).expect("the file is readable");
        assert!(
            text.contains(&format!("cpu_gpu_offset = \"UNCORRELATED({})\"", reason.as_str())),
            "D14's own word must be in the file verbatim; got:\n{text}"
        );

        let read = Artifact::read(&path, "run-C").expect("the refusal parses");
        assert_eq!(
            read.header.correlation,
            Correlation::Uncorrelated(reason),
            "a refusal must come back as the SAME refusal, not as a neighbouring one"
        );
        let _ = std::fs::remove_file(&path);
    }

    // And the measured shape's terms are each carried separately, so a reader can see which one
    // produced the bound rather than being handed the bound alone.
    let path = scratch("corr_measured");
    let written = fixture("run-C");
    written.write(&path).expect("invariant: the artifact writes");
    let read = Artifact::read(&path, "run-C").expect("the measurement parses");
    let Correlation::Correlated(c) = read.header.correlation else {
        panic!("the fixture's measured correlation came back as a refusal");
    };
    assert_eq!(c.offset_ns, -1_234_567_890_123, "a large negative offset must survive");
    assert_eq!((c.bracket_ns, c.driver_ns), (141, 88));
    assert_eq!((c.accepted, c.rejected, c.epoch), (29, 3, 1));
    assert_eq!((c.drift_ns, c.span_ns), (-57, 2_016_000_000));
    assert_eq!(c.max_deviation_ns(), 141, "the bound is the max of the two terms");
    let _ = std::fs::remove_file(&path);
}

/// **Rung 9, the RED.** A `cpu_gpu_offset` this reader does not understand is a MALFORMED file,
/// never a neighbouring reason and never a silent zero.
///
/// The concrete trap: `UNCORRELATED` on its own — D14's prose spelling, without the parenthesised
/// reason this writer emits — is exactly what a hand-written or older file would carry, and
/// mapping it onto `Unsupported` would report a device capability nobody probed.
#[test]
fn an_unreadable_correlation_word_is_malformed_not_a_guess() {
    let path = scratch("corr_bad");
    let written = fixture("run-D");
    let text = written.render().replace(
        "cpu_gpu_offset = \"-1234567890123\"",
        "cpu_gpu_offset = \"UNCORRELATED\"",
    );
    std::fs::write(&path, text).expect("the scratch file is writable");

    match Artifact::read(&path, "run-D") {
        Err(ArtifactError::Malformed { why, .. }) => {
            assert!(
                why.contains("cpu_gpu_offset"),
                "the refusal must name the key it refused, got {why:?}"
            );
        }
        other => panic!(
            "a bare `UNCORRELATED` must be refused, not interpreted. Got {other:?} -- which means \
             an unknown word reached a reader as if it were a known one."
        ),
    }
    let _ = std::fs::remove_file(&path);
}

/// **G24, reverse RED.** A stale artifact is REFUSED, and refused **before any row is parsed**.
///
/// The ordering is the clause, not an optimisation: a reader that parsed rows and then checked the
/// header would already have produced the numbers it was supposed to refuse, and every caller that
/// ignored the error would use them.
#[test]
fn a_stale_artifact_is_refused_on_the_header_instead_of_being_parsed() {
    let path = scratch("stale");
    // The previous child's file, left behind.
    fixture("run-A").write(&path).expect("invariant: the artifact writes");

    // This child was given a different token by its parent.
    let err = Artifact::read(&path, "run-B").expect_err("a stale artifact must be refused");
    match err {
        ArtifactError::TokenMismatch { found, expected } => {
            assert_eq!(found, "run-A");
            assert_eq!(expected, "run-B");
        }
        other => panic!(
            "a stale artifact was refused for the wrong reason: {other}. The gate's subject is that \
             the READER declines a file from another run; any other error means the staleness was \
             not what stopped it."
        ),
    }

    // NON-VACUITY: the same file parses perfectly when the token matches, so the refusal above is
    // about the token and not about a file this reader could never have read.
    let ok = Artifact::read(&path, "run-A").expect("the same file parses for its own run");
    assert_eq!(ok.zones.len(), 2, "the refused file was readable all along");

    let _ = std::fs::remove_file(&path);
}

/// **The ORDERING, observed rather than asserted.**
///
/// The clause above says the refusal happens *before any row is parsed*, and the test could not see
/// that: it checked the error's TYPE, and a reader that parsed every row and then compared the
/// token would return the same `TokenMismatch`. A gate that cannot distinguish its subject from a
/// coincidence is the shape this campaign keeps finding.
///
/// This one can. The file's header is well formed and its rows are NOT. A header-first reader must
/// return `TokenMismatch`; a rows-first reader reaches the broken row and returns `Malformed`. The
/// two orderings are now distinguishable by the error alone.
#[test]
fn the_header_refusal_happens_before_any_row_is_parsed() {
    // ⚠️ Built from `ARTIFACT_SCHEMA_VERSION`, never from a literal. Rung 7c's bump to 2 made this
    // fixture refuse on the SCHEMA — the right outcome for the wrong reason, and had the expected
    // variant happened to be `SchemaMismatch` the clause would have read as passing while testing
    // nothing about ordering at all.
    let text = format!(
        "\
schema_version = {ARTIFACT_SCHEMA_VERSION}
session_lo = 1
session_hi = 2
run_token = \"run-A\"
workload_tag = \"t\"
content_tag = \"c\"
regimes = \"off\"
modes = \"off\"
regime_n_distinct = 1
instrument = \"live\"
precision_decimals = 1
census_measured = 0
census_not_bracketed = 0
census_lost = 0
census_torn = 0

[[zone]]
id = not-a-number
"
    );
    let err = Artifact::parse(&text, "run-B").expect_err("a stale artifact must be refused");
    assert!(
        matches!(err, ArtifactError::TokenMismatch { .. }),
        "the reader reached a malformed ROW before it checked the header, so its refusal is an \
         accident of where the file happened to break rather than the header check `G24` asserts. \
         Got: {err}"
    );
}

/// A schema from another build is refused the same way, and also before any row.
#[test]
fn an_artifact_from_another_schema_is_refused() {
    let mut a = fixture("run-A");
    a.header.schema_version = ARTIFACT_SCHEMA_VERSION + 1;
    let text = a.render();

    let err = Artifact::parse(&text, "run-A").expect_err("a foreign schema must be refused");
    match err {
        ArtifactError::SchemaMismatch { found, expected } => {
            assert_eq!(found, ARTIFACT_SCHEMA_VERSION + 1);
            assert_eq!(expected, ARTIFACT_SCHEMA_VERSION);
        }
        other => panic!("wrong refusal for a foreign schema: {other}"),
    }
}

/// **THE PRECISION CLAUSE.** A tenth of a nanosecond is the instrument's resolution, and the file
/// must not quietly widen or narrow it.
///
/// `vg_occ_split_timing.rs:916` reconstructs the GPU tick lattice by taking a GCD over **tenths**,
/// because that is the precision the channel prints. This test does the same reconstruction across
/// the file: write a set of figures on a 12.8 ns lattice, read them back, recompute the GCD over
/// tenths, and require the lattice to come back.
///
/// # ⚠️ THE WIDENING RED IS NOT PRODUCIBLE, and saying so is the point
///
/// The obvious RED — emit full-precision `f64` from the writer and watch the GCD collapse — was
/// injected and **the gate stayed green**. MEASURED: the consumer's own `(v * 10.0).round()` *is* a
/// rounding to tenths, so it absorbs whatever extra digits the file carries. Across `128.0`,
/// `268.8`, `163.2`, `128.04`, `128.06`, `12.85` and `1234.567` the reconstructed value is identical
/// whether the file was written at one decimal or at full precision.
///
/// So this clause does **not** protect the lattice against a wider file; nothing here does, because
/// nothing here can. It asserts the two things it can show — the lattice survives the round trip,
/// and the header states its own precision — and the disclosure stands in place of a clause that
/// would have looked stronger than it is. The 32× under-statement `vg_occ_split_timing.rs` measures
/// is about choosing the FLOOR term as `period × 1 tick`, a different decision that lands at rung 8.
///
/// # A MEAN IS NOT A LATTICE POINT, and this gate found that on its first run
///
/// The reconstruction reads **order statistics only** — median and p95, which are real samples and
/// therefore sit on the hardware's tick lattice. A mean is an average of samples and sits between
/// them: the first version of this test folded `mean_ns = 163.2` into the GCD and got **32 tenths
/// instead of 128**, a lattice four times finer than the hardware's, from data that was otherwise
/// perfectly quantised. That is the same failure the band would suffer in production, arriving
/// through a different door — so the exclusion is stated here rather than left to whoever writes
/// the reducer.
#[test]
fn a_tenth_is_the_instruments_resolution_and_survives_the_file() {
    let a = fixture("run-A");
    let text = a.render();
    let back = Artifact::parse(&text, "run-A").expect("parses");

    // ORDER STATISTICS ONLY. `mean_ns` is carried in the file (consumers report it) and excluded
    // here, for the reason in this test's doc.
    let tenths: Vec<u64> = back
        .zones
        .iter()
        .flat_map(|z| [z.median_ns, z.p95_ns])
        .filter(|v| *v > 0.0)
        .map(|v| (v * 10.0).round() as u64)
        .collect();
    assert!(!tenths.is_empty(), "the fixture carries no non-zero figure to reconstruct from");

    let lattice = tenths.iter().copied().fold(0u64, gcd);
    assert_eq!(
        lattice, 128,
        "the GPU tick lattice did not survive the artifact. The fixture's ORDER STATISTICS sit on a \
         12.8 ns lattice with coprime multipliers (k = 10 and 21), so their GCD over tenths IS the \
         quantum rather than a multiple of it -- the rung-4 lesson that two readings establish a \
         common multiple, never a step. A different value here means the file changed the figures \
         on the way through. Reconstructed {lattice} tenths from {tenths:?}."
    );

    assert_eq!(
        back.header.precision_decimals, PRECISION_DECIMALS,
        "the header must state the precision, so no reader has to know it by folklore"
    );
    // And the file says so in its own text, not only in the parsed struct.
    assert!(
        text.contains(&format!("precision_decimals = {PRECISION_DECIMALS}")),
        "the rendered file does not state its own precision"
    );
}

/// A row missing a field is MALFORMED, never defaulted.
///
/// A defaulted `median_ns = 0.0` is indistinguishable from a measurement of zero — the confusion
/// this campaign has found at every rung.
#[test]
fn a_row_missing_a_field_is_malformed_rather_than_defaulted() {
    // Built from the constant, for the reason the ordering clause above records.
    let text = format!(
        "\
schema_version = {ARTIFACT_SCHEMA_VERSION}
session_lo = 1
session_hi = 2
run_token = \"run-A\"
workload_tag = \"t\"
content_tag = \"c\"
regimes = \"off\"
modes = \"off\"
regime_n_distinct = 1
instrument = \"live\"
precision_decimals = 1
census_measured = 0
census_not_bracketed = 0
census_lost = 0
census_torn = 0

[[zone]]
id = 17
label = \"measured\"
n = 10
median_ns = 128.0
mean_ns = 163.2
p95_ns = 288.0
stddev_ns = 40.0
begin_off_ns = 0.0
"
    );
    let err =
        Artifact::parse(&text, "run-A").expect_err("a row without `end_off_ns` must not parse");
    match err {
        ArtifactError::Malformed { why, .. } => {
            assert!(why.contains("end_off_ns"), "wrong field named: {why}");
        }
        other => panic!("a truncated row was accepted or refused wrongly: {other}"),
    }
}

/// An empty expectation waives the token check — the one caller that genuinely cannot know it.
///
/// Stated as its own test because it is the gate's escape hatch, and an escape hatch nobody names
/// is an escape hatch nobody notices.
#[test]
fn an_empty_expectation_waives_the_token_check() {
    let text = fixture("whatever-the-child-chose").render();
    let ok = Artifact::parse(&text, "").expect("an operator reading by hand passes no expectation");
    assert_eq!(ok.header.run_token, "whatever-the-child-chose");
}

/// Euclid, for the lattice reconstruction.
fn gcd(a: u64, b: u64) -> u64 {
    if b == 0 { a } else { gcd(b, a % b) }
}

// ===================================================================================================
// Rung 7c — the workload tag is two halves, and an undeclared one is not a floor
// ===================================================================================================

/// **Decision 7's census survives the file.** Without this the three new header fields would be
/// written and never read back by anything — decoration that a reader cannot rely on. The fixture
/// carries TWO distinct regimes on purpose: `n_distinct == 1` is the state a consumer accepts, so a
/// round-trip that only ever saw `1` could not tell a carried value from a hardcoded one.
#[test]
fn the_regime_census_survives_the_file() {
    let a = fixture("tok");
    let back = Artifact::parse(&a.render(), "tok").expect("the fixture round-trips");
    assert_eq!(back.header.regimes, "off,forced_on");
    assert_eq!(back.header.modes, "off");
    assert_eq!(back.header.regime_n_distinct, 2, "the cardinality is carried, not re-derived");
}

/// **The hole this closes, as a test.** `vg_decidability_floor.rs` measures its NULL experiment
/// across two configurations — `BOYKO_VB_FROXEL_FORCE_OFF` set and unset — and the old tag
/// (`path × legs`) was IDENTICAL for both, because `froxel_light_cull` is a different field of the
/// same struct.
///
/// RED, run: revert `config_tag` to `format!("{path:?}_{legs:?}")` ⇒ the two tags compare equal and
/// this clause fails naming both. It is the whole reason the derivation hashes the WHOLE struct
/// rather than a chosen subset — the bug was not that the wrong field was chosen, it was that
/// fields were chosen at all.
#[test]
fn the_config_tag_separates_the_two_legs_of_the_floor_experiment() {
    use boyko_app::profiling::artifact::config_tag;
    use boyko_render::ResolvedRenderPath;

    let flat = ResolvedRenderPath { froxel_light_cull: false, ..ResolvedRenderPath::default() };
    let froxel = ResolvedRenderPath { froxel_light_cull: true, ..ResolvedRenderPath::default() };
    assert_ne!(
        flat, froxel,
        "the fixture must differ, or it proves nothing about the tag"
    );
    assert_ne!(
        config_tag(&flat),
        config_tag(&froxel),
        "the flat and froxel legs of the floor experiment produced the SAME workload tag, so a \
         floor measured on one would be accepted as bounding a delta measured on the other — the \
         exact confusion the tag exists to prevent.\n  flat:   {}\n  froxel: {}",
        config_tag(&flat),
        config_tag(&froxel)
    );
    // ...and the same input twice is the same tag, or the tag is noise rather than an identity.
    assert_eq!(config_tag(&flat), config_tag(&flat), "the derivation is not deterministic");
}

/// The tag stays GREPPABLE. A pure hash would satisfy the clause above and leave a human unable to
/// tell `visibilitybuffer_mesh` from `deferred_both` without running the engine.
#[test]
fn the_config_tag_keeps_a_readable_prefix() {
    use boyko_app::profiling::artifact::config_tag;
    use boyko_render::{GeometryLegs, RenderPath, ResolvedRenderPath};

    let r = ResolvedRenderPath {
        path: RenderPath::VisibilityBuffer,
        legs: GeometryLegs::Mesh,
        ..ResolvedRenderPath::default()
    };
    let tag = config_tag(&r);
    assert!(
        tag.starts_with("visibilitybuffer_mesh#"),
        "the tag lost its readable prefix and is now only a hash: {tag}"
    );
    assert_eq!(tag.len(), "visibilitybuffer_mesh#".len() + 8, "eight hex digits, no more: {tag}");
}

/// **The owner's strict rule, enforced:** an artifact that does not say what workload it measured
/// cannot serve as a floor.
///
/// Whitespace is the SAME refusal as emptiness — a tag of spaces declares no more about a workload
/// than no tag does, and two spellings of "nothing" must not be two outcomes.
#[test]
fn an_undeclared_content_tag_cannot_serve_as_a_floor() {
    for (case, declared) in [("empty", ""), ("blank", "   \t ")] {
        let mut a = fixture("tok");
        a.header.content_tag = declared.to_owned();
        match a.floor_source() {
            Err(ArtifactError::UndeclaredContent { workload_tag }) => {
                assert_eq!(workload_tag, a.header.workload_tag, "{case}: the refusal must name the file");
            }
            Err(e) => panic!("{case}: refused for the wrong reason: {e}"),
            Ok(_) => panic!(
                "{case}: an artifact declaring no content was accepted as a floor source. A floor \
                 bounds the workload it was measured on; this file does not say which one that was."
            ),
        }
    }
}

/// The green leg of the same rule — without it the refusal above would be satisfied by a
/// `floor_source` that refuses everything.
#[test]
fn a_declared_content_tag_is_accepted_as_a_floor() {
    let a = fixture("tok");
    assert!(!a.header.content_tag.is_empty(), "the fixture must declare, or this proves nothing");
    let got = a.floor_source().expect("a declared artifact is a floor source");
    assert_eq!(got.header.content_tag, a.header.content_tag);
}

/// **"Nobody declared" and "an older writer" are different observations, and the parser keeps them
/// apart.** A MISSING `content_tag` key is a malformed header; a PRESENT empty one is an honest
/// declaration of nothing, which `floor_source` refuses. Defaulting the missing key to `""` would
/// collapse the two and turn a v1 file into a silently-undeclared v2 one.
#[test]
fn a_missing_content_tag_key_is_malformed_not_an_empty_declaration() {
    let text = fixture("tok").render();
    let stripped: String =
        text.lines().filter(|l| !l.starts_with("content_tag")).collect::<Vec<_>>().join("\n");
    match Artifact::parse(&stripped, "tok") {
        Err(ArtifactError::BadHeader("content_tag")) => {}
        other => panic!(
            "a header with no `content_tag` key at all parsed as {other:?} instead of a malformed \
             header. Read as an empty declaration it would be indistinguishable from a writer that \
             declared nothing, and only one of those is a file this build wrote."
        ),
    }
}

// ===================================================================================================
// G24's OTHER half: the census
// ===================================================================================================

/// The four literals that were the stdout measurement channel's whole vocabulary.
///
/// Written as split string pieces so this file's own source does not match the census it performs —
/// the self-reference `logging/registry-and-walker` had to fix for its check 4, arriving here for
/// the same structural reason.
const RETIRED_CHANNEL: [&str; 4] = [
    concat!("VB-", "P1d "),
    concat!("VB-", "P4 pass="),
    concat!("VB-", "P4 regime"),
    concat!("VB-", "SV0-S1.5 "),
];

/// Strip `//`-style comments from one line, **without** touching string literals.
///
/// The distinction is the entire point. A pattern inside a comment is a RECORD of the retired
/// channel; a pattern inside a string literal is a producer of it, and those are the things this
/// census exists to find. A naive "cut at the first `//`" would also truncate at a `//` inside a
/// literal — `"http://…"` — and could hide a real producer sitting after it on the same line.
///
/// Block comments are not handled, and that is a measured decision rather than an omission: all
/// eight prose mentions in this tree are `//` / `///` lines, and a `/* */` walker cannot be written
/// line-by-line. `scripts/check_hotpath_exceptions.py` documents and accepts the same limit for the
/// same reason.
fn strip_line_comment(line: &str) -> &str {
    let b = line.as_bytes();
    let mut in_str = false;
    let mut escaped = false;
    let mut i = 0usize;
    while i < b.len() {
        let c = b[i];
        if in_str {
            if escaped {
                escaped = false;
            } else if c == b'\\' {
                escaped = true;
            } else if c == b'"' {
                in_str = false;
            }
        } else if c == b'"' {
            in_str = true;
        } else if c == b'/' && i + 1 < b.len() && b[i + 1] == b'/' {
            return &line[..i];
        }
        i += 1;
    }
    line
}

/// Every `.rs` file under `crates/*/src`, the scope the gate names.
fn crate_sources() -> Vec<PathBuf> {
    let crates = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..");
    let mut out = Vec::new();
    let mut stack: Vec<PathBuf> = Vec::new();
    let entries = std::fs::read_dir(&crates).expect("invariant: the crates directory is readable");
    for e in entries.flatten() {
        let src = e.path().join("src");
        if src.is_dir() {
            stack.push(src);
        }
    }
    while let Some(dir) = stack.pop() {
        for e in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().is_some_and(|x| x == "rs") {
                out.push(p);
            }
        }
    }
    assert!(
        out.len() > 200,
        "the census walked {} files, which is too few to be the workspace -- a mis-resolved root \
         would make this gate pass by scanning nothing, which is the one way it must not pass",
        out.len()
    );
    out
}

/// **G24's census leg: the stdout measurement channel has no producers left.**
///
/// # The gate as written cannot pass, and that is a defect in the instrument
///
/// `05-LADDER-GATES.md` specifies this as *"`rg 'VB-P1d |VB-P4 pass=|VB-P4 regime|VB-SV0-S1.5 '
/// crates/*/src` returns **zero**"*. Run literally, against a tree where rung 7's subtraction is
/// complete, it returns **eight** — every one of them a comment recording what was deleted and why:
/// `gpu_zone.rs` explaining which half of a retired bracket its constants are what is left of,
/// `occlusion_config.rs` naming the summary line its enum used to feed, `light_policy.rs` citing the
/// bench its thresholds were measured on.
///
/// The corpus already diagnosed exactly this for the SIBLING gate — *"a gate that would be satisfied
/// by erasing the record of what it gated is mis-specified, and the mis-specification is in the
/// instrument, not in the requirement"* — and then said this gate was fine because it is scoped to
/// `crates/*/src`. A directory scope does not exclude comments. The correction was made on one gate
/// and not carried to its twin, and neither was ever armed, so nothing noticed.
///
/// What the requirement means is *no site PRODUCES those lines*. That is what runs here.
#[test]
fn g24_census_the_retired_stdout_channel_has_no_producers() {
    let mut producers: Vec<String> = Vec::new();
    let mut prose = 0usize;

    for path in crate_sources() {
        let Ok(text) = std::fs::read_to_string(&path) else { continue };
        for (n, line) in text.lines().enumerate() {
            let hit = RETIRED_CHANNEL.iter().any(|p| line.contains(p));
            if !hit {
                continue;
            }
            let code = strip_line_comment(line);
            if RETIRED_CHANNEL.iter().any(|p| code.contains(p)) {
                producers.push(format!("{}:{}  {}", path.display(), n + 1, line.trim()));
            } else {
                prose += 1;
            }
        }
    }

    assert!(
        producers.is_empty(),
        "the retired stdout measurement channel still has {} producer(s):\n{}\n\nRung 7 deleted \
         this channel; a surviving producer means a consumer somewhere is still being fed by it, \
         and the artifact it was migrated to is not the only source of those numbers.",
        producers.len(),
        producers.join("\n")
    );

    // Anti-vacuity, and it is load-bearing here in a way it usually is not: this census would pass
    // trivially against an empty walk, a wrong root, or a `strip_line_comment` that ate whole lines.
    // The prose mentions are the positive control -- the tree DOES contain these literals, and the
    // gate finds them and correctly classifies them as records rather than producers.
    assert!(
        prose >= 5,
        "the census found only {prose} prose mention(s) of the retired channel. It should find \
         several: this campaign keeps the record of what it deleted. Too few means the walker is \
         not seeing the files it thinks it is, and a green from a walker that reads nothing is the \
         failure mode this assertion exists to catch."
    );
}

/// The subsystems tag survives the file, and it is DERIVED rather than taken from a caller.
///
/// Two claims, and the second is the one that matters: a caller who could name this field could
/// stamp `profiler+logger` on a file taken with neither, and every regression gate compares against
/// exactly this field. `subsystems_tag()` reads `ARM_MASK` and the target control table at write
/// time, so the only way to make it lie is to actually arm something.
#[test]
fn the_subsystems_tag_survives_the_file_and_is_derived_from_the_live_state() {
    let path = scratch("subsystems_tag");
    let _ = std::fs::remove_file(&path);
    let a = fixture("tok-subsys");
    a.write(&path).expect("the fixture writes");
    let raw = std::fs::read_to_string(&path).expect("readable");
    // The file claims `.toml`, and this was the ONE string field written bare — found by reading
    // an artifact a real windowed run produced (`subsystems_tag = neither`, no quotes), which any
    // actual TOML parser refuses. The shipped reader tolerated its own dialect, so only a textual
    // pin can hold the format to its name.
    assert!(
        raw.contains("subsystems_tag = \"profiler+logger\""),
        "subsystems_tag must be QUOTED like every other string field, or the file is not TOML: \
         {raw:?}"
    );
    let back = Artifact::parse(&raw, "tok-subsys").expect("the fixture parses");
    assert_eq!(
        back.header.subsystems_tag, "profiler+logger",
        "the tag did not survive the file, so a baseline cannot say which subsystems produced it"
    );

    // ── THE DERIVATION, driven through the real control table ───────────────────────────────
    //
    // Nothing is armed in this test binary, so the derivation must say so. Then one target is
    // armed and it must move -- a derivation that returned a constant would pass the first half.
    for (id, _) in boyko_log::target::engine_targets() {
        boyko_log::target::set_target_control(id, boyko_log::target::TargetControl::OFF);
    }
    assert_eq!(
        boyko_app::profiling::artifact::subsystems_tag(),
        "neither",
        "with nothing armed the tag must say so; `enable()` alone is not the logger being live"
    );
    boyko_log::target::set_target_control(
        <boyko_log::Log as boyko_log::LogTarget>::ID,
        boyko_log::target::TargetControl::new(boyko_log::Level::Warn, 0, false),
    );
    assert_eq!(
        boyko_app::profiling::artifact::subsystems_tag(),
        "logger",
        "arming a target must move the tag; a constant would have passed the check above"
    );
    for (id, _) in boyko_log::target::engine_targets() {
        boyko_log::target::set_target_control(id, boyko_log::target::TargetControl::OFF);
    }
    let _ = std::fs::remove_file(&path);
}
