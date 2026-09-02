//! The ISA baseline is a claim, and this file is what makes it falsifiable.
//!
//! `CLAUDE.md` has named AVX2 the engine's baseline since the project began. Until
//! 2026-09-02 nothing in the build enabled it, so every `cfg(target_feature = "avx2")`
//! arm in the tree compiled to nothing and the scalar fallback shipped. The declared
//! baseline and the built one had diverged for the whole life of the project, and
//! **nothing failed** — because the consequence of the divergence is code that does not
//! exist, and absent code cannot go red.
//!
//! That is the defect class this census exists for, and it has already bitten twice in
//! this repository:
//!
//! * `crates/boyko_physics/src/sdf_simd.rs`'s `o9_kernel_tests` module is declared under
//!   `cfg(all(target_arch = "x86_64", target_feature = "avx2"))`. Because AVX2 was never
//!   enabled, **that module had never compiled and its tests had never run once.** Turning
//!   the declared baseline on turned a vacuously-absent module into a red one.
//! * The `RUSTFLAGS` environment variable **replaces** `build.rustflags` and
//!   `target.<triple>.rustflags` from `.cargo/config.toml` — it does not append to them
//!   (Cargo reference, "Environment variables that Cargo reads"). Any job or shell that
//!   sets `RUSTFLAGS` for an unrelated reason silently drops the ISA baseline, and the
//!   only symptom is less code.
//!
//! So the baseline is asserted here rather than assumed. A build that does not carry it
//! fails with a message naming the cause, instead of quietly compiling a smaller engine.
//!
//! **This census does not decide the baseline** — `.cargo/config.toml` does. It only
//! insists that whatever `.cargo/config.toml` says actually reached the compiler.

/// The engine's declared x86_64 baseline, as `.cargo/config.toml` sets it and `CLAUDE.md`
/// describes it: the `x86-64-v3` microarchitecture level.
///
/// Each entry is a `target_feature` name and the reason the engine relies on it, so a
/// future reader who has to relax one knows what they are giving up.
#[cfg(target_arch = "x86_64")]
const REQUIRED_X86_64_FEATURES: &[(&str, bool, &str)] = &[
    (
        "avx2",
        cfg!(target_feature = "avx2"),
        "the whole point: boyko_physics's 8-wide colored solve, refresh_inertia, \
         position_integrate, apply_gravity and the SDF narrowphase x8 kernels are all \
         declared under cfg(target_feature = \"avx2\") and compile to NOTHING without it",
    ),
    (
        "fma",
        cfg!(target_feature = "fma"),
        "part of the x86-64-v3 level; the physics kernels deliberately do not USE it \
         (they are written mul_add-free and two source censuses enforce that), but its \
         absence here means the level did not arrive whole",
    ),
    (
        "bmi2",
        cfg!(target_feature = "bmi2"),
        "part of the level; bit-manipulation instructions the BitSet and archetype mask \
         code benefit from without naming them",
    ),
    (
        "lzcnt",
        cfg!(target_feature = "lzcnt"),
        "part of the level; leading-zero count backs the slot and generation arithmetic",
    ),
    (
        "f16c",
        cfg!(target_feature = "f16c"),
        "part of the level; half-float conversion for GPU-facing columns",
    ),
];

/// Every feature of the declared baseline reached the compiler.
///
/// Fails loudly, naming the likely cause, when the ISA flags were dropped — which in
/// practice means a `RUSTFLAGS` environment variable replaced the config's rustflags.
#[cfg(target_arch = "x86_64")]
#[test]
fn the_declared_isa_baseline_actually_reached_the_compiler() {
    let missing: Vec<&(&str, bool, &str)> = REQUIRED_X86_64_FEATURES
        .iter()
        .filter(|(_, present, _)| !*present)
        .collect();

    assert!(
        missing.is_empty(),
        "the x86-64-v3 baseline did not reach rustc: {} of {} features are absent.\n\n{}\n\
         \nWHY THIS USUALLY HAPPENS: the `RUSTFLAGS` environment variable REPLACES the \
         `rustflags` set in `.cargo/config.toml` rather than appending to them. If a shell, \
         a CI job or a tool set `RUSTFLAGS` for some other reason (`-D warnings`, \
         `--cfg loom`, `--cfg force_alloc_panic`, `--emit=asm`), the `-C target-cpu=x86-64-v3` \
         from the config was dropped and this build compiled the scalar fallbacks instead of \
         the vectorised kernels.\n\
         \nTHE FIX is to add `-C target-cpu=x86-64-v3` to that RUSTFLAGS value, not to relax \
         this census: a build without the baseline is a DIFFERENT ENGINE, and the modules it \
         silently omits include `boyko_physics`'s entire AVX2 solver and the `o9_kernel_tests` \
         module (which had never compiled once before 2026-09-02 for exactly this reason).\n\
         \nIf the baseline is being lowered ON PURPOSE, change `.cargo/config.toml` and this \
         list together, in the same commit, and say in both why.",
        missing.len(),
        REQUIRED_X86_64_FEATURES.len(),
        missing
            .iter()
            .map(|(name, _, why)| format!("  - `{name}` is ABSENT — {why}"))
            .collect::<Vec<_>>()
            .join("\n"),
    );
}

/// Features the declared baseline does NOT include, carried through the same machinery as
/// the required ones so the instrument's negative arm is exercised rather than asserted.
///
/// `CLAUDE.md` puts AVX-512 behind an opt-in `cfg(target_feature)` and `x86-64-v3` does not
/// include it, so on a correct build every entry here is absent. If one ever becomes
/// present that is not by itself an error — but this file's negative control would then be
/// proving nothing, and the census says so instead of passing quietly.
#[cfg(target_arch = "x86_64")]
const FEATURES_OUTSIDE_THE_BASELINE: &[(&str, bool, &str)] = &[
    (
        "avx512f",
        cfg!(target_feature = "avx512f"),
        "AVX-512 foundation: CLAUDE.md makes it opt-in per call site, not part of the \
         baseline, because it is absent on every AMD part before Zen 4 and fused off on \
         most consumer Intel parts since Alder Lake",
    ),
    (
        "avx512vl",
        cfg!(target_feature = "avx512vl"),
        "the 128/256-bit forms of AVX-512; present only where the foundation is",
    ),
];

/// The census can fail, its lists are not empty, and its predicate reports absence as well
/// as presence.
///
/// A census whose subject list is empty passes over anything, which is the shape this
/// repository has measured repeatedly (an empty trybuild glob exits 0; a filter that
/// selects no test prints `running 0 tests` and succeeds). Both list sizes are pinned so
/// deleting an entry is a deliberate edit rather than a silent weakening.
///
/// The negative control runs the SAME predicate over
/// [`FEATURES_OUTSIDE_THE_BASELINE`] that the real check runs over
/// [`REQUIRED_X86_64_FEATURES`], rather than asserting a bare `cfg!` — which would be a
/// constant expression the compiler folds and clippy rejects, and which would in any case
/// exercise a different path from the one under test.
#[cfg(target_arch = "x86_64")]
#[test]
fn the_census_lists_are_pinned_and_its_predicate_can_report_absence() {
    assert_eq!(
        REQUIRED_X86_64_FEATURES.len(),
        5,
        "the ISA census's required list changed size; update this count in the same commit \
         and say in the list's doc comment why a feature was added or removed"
    );
    assert_eq!(
        FEATURES_OUTSIDE_THE_BASELINE.len(),
        2,
        "the ISA census's outside-the-baseline list changed size; same rule"
    );

    // The same filter the real check uses, pointed at features that must be absent. On a
    // correct build it selects everything; a build that selects nothing has lost the
    // control and is told so.
    let absent: Vec<&(&str, bool, &str)> = FEATURES_OUTSIDE_THE_BASELINE
        .iter()
        .filter(|(_, present, _)| !*present)
        .collect();

    assert_eq!(
        absent.len(),
        FEATURES_OUTSIDE_THE_BASELINE.len(),
        "a feature outside the declared baseline is enabled, so this census's negative \
         control no longer demonstrates that the predicate can report absence.\n\n{}\n\n\
         That is not itself an error — a wider ISA may be deliberate. But pick features \
         this build genuinely lacks, or delete the control and say in its place why the \
         instrument is trusted without one.",
        FEATURES_OUTSIDE_THE_BASELINE
            .iter()
            .filter(|(_, present, _)| *present)
            .map(|(name, _, why)| format!("  - `{name}` is PRESENT — {why}"))
            .collect::<Vec<_>>()
            .join("\n"),
    );
}
