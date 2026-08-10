//! **Profiling rung 8 — the API-shape gate.**
//!
//! The comparator's guarantees are *negative*: there is no third verdict, no bare-delta
//! constructor, no caller-supplied sigma, and exactly one way to obtain a [`Floor`]. A unit test
//! can only demonstrate that the functions which exist behave; **it cannot demonstrate that a
//! function does not exist.** That is what this file does, over the module's own source.
//!
//! # Why a source gate rather than a type-system one
//!
//! Rust can enforce some of this — a private field makes a struct unconstructable from outside —
//! and `Floor`'s fields ARE private, so no downstream crate can build one by literal. What it
//! cannot enforce is that nobody adds `Floor::from_quantum` next month, in the same module, where
//! the privacy does not apply. The corpus lists that constructor by name under *"never existed"*
//! and lists `Floor::from_aa_control(control, sigma)` under *"deleted in rev 3"* — both because
//! they were, or nearly were, real. A list of names that must not reappear is a gate only if
//! something reads it.
//!
//! # What this gate CANNOT claim
//!
//! It reads text. A constructor named something else, or one assembled through a builder, passes.
//! It is a tripwire on the shapes this campaign has already seen go wrong, not a proof of the
//! negative — and saying so here is the difference between a gate and a reassurance.

use std::path::PathBuf;

/// Windows line ending, as this repository checks the file out.
const CRLF: &str = "\r\n";
/// What the scans below expect.
const LF: &str = "\n";

/// The module under inspection, **normalised to LF**.
///
/// ⚠️ MEASURED, and it cost a red: this file is checked out with CRLF on Windows, and a tool that
/// rewrites it — a Python edit whose `write_text` translates newlines by default — flips the whole
/// file between one commit and the next. Two of the gates below scan for a closing brace between
/// two newlines to find where an `enum` ends, and they went red on a file **whose content had not
/// changed at all**. A gate whose verdict depends on line endings is measuring its own checkout,
/// not the code.
fn contrast_source() -> String {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/profiling/contrast.rs");
    let raw = std::fs::read_to_string(&p)
        .unwrap_or_else(|e| panic!("invariant: rung 8's module must exist at {}: {e}", p.display()));
    raw.replace(CRLF, LF)
}

/// Lines that are code rather than prose — the doc comments deliberately NAME the forbidden
/// constructors, so a naive substring search over the whole file finds its own documentation.
///
/// That is not a detail: the first draft of this gate red on `contrast.rs`'s own module doc, which
/// is the same class of defect as the anchors gate reading a historical line number as a live
/// citation. A gate whose instrument cannot tell a mention from a use reports the record of a
/// deletion as the deletion's failure.
fn code_lines(src: &str) -> Vec<&str> {
    src.lines()
        .map(str::trim)
        .filter(|l| !l.starts_with("//") && !l.starts_with("///") && !l.starts_with("//!"))
        .collect()
}

/// **`Floor` has exactly one constructor**, and it is `from_session_file`.
#[test]
fn floor_has_exactly_one_constructor() {
    let src = contrast_source();
    let code = code_lines(&src);
    let ctors: Vec<&&str> = code
        .iter()
        .filter(|l| l.starts_with("pub fn from_") || l.starts_with("pub fn new"))
        .collect();
    // `Twin::from_zero_control` is the twin's, and it is the only other `from_` in the file.
    let floor_ctors: Vec<&&&str> =
        ctors.iter().filter(|l| l.contains("session_file") || l.contains("quantum") || l.contains("aa_control")).collect();
    assert_eq!(
        floor_ctors.len(),
        1,
        "rung 8 requires exactly ONE `Floor` constructor (`from_session_file`). Found: {floor_ctors:?}.\n\
         `Floor::from_aa_control(control, sigma)` was DELETED in rev 3 (one sitting, caller-chosen \
         sigma) and `Floor::from_quantum` never existed. Adding either back makes every verdict in \
         this subsystem a caller's choice."
    );
    assert!(
        floor_ctors[0].contains("from_session_file"),
        "the one constructor must be `from_session_file`, found {:?}",
        floor_ctors[0]
    );
}

/// **No caller-supplied sigma anywhere.** `FLOOR_SIGMA` is a `const`; a parameter named `sigma`
/// would move the decision to whoever calls.
#[test]
fn no_function_takes_a_sigma() {
    let src = contrast_source();
    let offenders: Vec<&str> = code_lines(&src)
        .into_iter()
        .filter(|l| l.contains("sigma") && !l.contains("FLOOR_SIGMA"))
        .collect();
    assert!(
        offenders.is_empty(),
        "a caller who may choose the sigma may choose the verdict. `FLOOR_SIGMA` is a const and \
         nothing else in this module may name one. Offending lines: {offenders:?}"
    );
}

/// **The reduction is a `const` and the constructor applies it with no parameter.**
#[test]
fn the_floor_reduction_is_not_a_parameter() {
    let src = contrast_source();
    let code = code_lines(&src);
    assert!(
        code.iter().any(|l| l.starts_with("pub const FLOOR_REDUCTION: Reduction = Reduction::Max;")),
        "FLOOR_REDUCTION must be a const, and it must be `Max` -- the only reduction that cannot \
         manufacture a win. M11 measured the alternative: this protocol's repetitions span \
         6.3 / 14.3 / 4.7 / 13.5 %, a 3x difference between candidate reductions."
    );
    let public_takes_reduction: Vec<&&str> = code
        .iter()
        .filter(|l| l.starts_with("pub fn") && l.contains("Reduction"))
        .collect();
    assert!(
        public_takes_reduction.is_empty(),
        "no public function may take a `Reduction`: the step is const-driven, not a caller's \
         choice. Offending: {public_takes_reduction:?}"
    );
}

/// **`Contrast` has exactly two variants.** The seventh question — *"just give me the delta"* — is
/// structurally unanswerable, and that rests on there being no third arm to return.
///
/// ⚠️ **MEASURED: this gate's RED lands on the COMPILER, not here.** Adding a `BareDelta { .. }`
/// variant fails the build first — `Contrast::median_delta_ns` and `Contrast::band_ns` match
/// exhaustively over the two, so a third arm is a `non-exhaustive patterns` error before any test
/// runs. That is a STRONGER guarantee than this assertion, and it means what this test actually
/// catches is narrower than its name: a variant added *together with* its match arms, by someone
/// who made the code compile and did not ask whether the verdict should have a third answer. The
/// predicted RED and the measured one are different objects; the measured one is recorded because
/// a gate described by the failure it was expected to have is a gate nobody has run.
#[test]
fn the_verdict_has_exactly_two_variants() {
    let src = contrast_source();
    let start = src.find("pub enum Contrast {").expect("invariant: `Contrast` must exist");
    let body = &src[start..];
    let end = body.find("\n}\n").expect("invariant: the enum must close");
    let body = &body[..end];
    let variants: Vec<&str> = body
        .lines()
        .map(str::trim)
        .filter(|l| *l == "Resolved {" || *l == "NotResolved {")
        .collect();
    assert_eq!(
        variants.len(),
        2,
        "`Contrast` must have exactly `Resolved` and `NotResolved`. A third variant is how \
         \"just give me the delta\" becomes answerable again. Found: {variants:?}"
    );
}

/// **Every `NotResolvedReason` round-trips through its wire word** — `G4c`'s clause, checked over
/// the enum as the source declares it rather than over a hand-written list that could go stale.
#[test]
fn every_not_resolved_reason_round_trips() {
    use boyko_app::profiling::contrast::NotResolvedReason as R;
    let src = contrast_source();
    let start = src.find("pub enum NotResolvedReason {").expect("the enum must exist");
    let body = &src[start..];
    let body = &body[..body.find("\n}\n").expect("the enum must close")];
    // Variant lines are the bare `Name,` entries inside the enum body.
    let declared: Vec<String> = body
        .lines()
        .map(str::trim)
        .filter(|l| {
            l.ends_with(',')
                && !l.contains(' ')
                && l.chars().next().is_some_and(char::is_uppercase)
        })
        .map(|l| l.trim_end_matches(',').to_string())
        .collect();
    assert_eq!(
        declared.len(),
        6,
        "rung 8 specifies six refusal reasons; the source declares {}: {declared:?}",
        declared.len()
    );

    let all = [
        R::BelowBand,
        R::FloorWorkloadMismatch,
        R::TwinWorkloadMismatch,
        R::WindowIncomplete,
        R::EpochBreak,
        R::LabelNotMeasured,
    ];
    assert_eq!(all.len(), declared.len(), "the list here must cover every declared variant");
    for r in all {
        assert_eq!(
            R::from_wire(r.as_str()),
            Some(r),
            "`{}` must round-trip: an artifact that writes a reason a reader cannot parse reports \
             a refusal nobody can act on",
            r.as_str()
        );
    }
    // And a word nobody writes is not silently accepted as some default.
    assert_eq!(R::from_wire("resolved"), None);
    assert_eq!(R::from_wire(""), None);
}

/// **`FLOOR_SIGMA`, `FLOOR_SESSIONS` and `FLOOR_REPEATS` are the protocol this build accepts**, and
/// a session file recording anything else is refused rather than read.
///
/// The values are asserted HERE, not only in the module: changing a protocol constant changes what
/// every published floor means, so it should have to change a gate too.
#[test]
fn the_protocol_constants_are_pinned() {
    use boyko_app::profiling::contrast::{FLOOR_REPEATS, FLOOR_SESSIONS, FLOOR_SIGMA};
    assert!((FLOOR_SIGMA - 3.0).abs() < f64::EPSILON, "three sigma, per `vg_decidability_floor.rs`");
    assert_eq!(FLOOR_SESSIONS, 7, "7 separate processes per repetition");
    assert_eq!(FLOOR_REPEATS, 3, "3 independent repetitions, all published, never averaged");
}
