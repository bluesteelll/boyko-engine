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

/// The census can fail, and its own list is not empty.
///
/// A census whose subject list is empty passes over anything, which is the shape this
/// repository has measured repeatedly (an empty trybuild glob exits 0; a filter that
/// selects no test prints `running 0 tests` and succeeds). This test pins the list's size
/// so deleting an entry is a deliberate edit rather than a silent weakening, and proves
/// the negative arm of the predicate works by evaluating it against a feature the baseline
/// deliberately does NOT include.
#[cfg(target_arch = "x86_64")]
#[test]
fn the_census_list_is_non_empty_and_its_predicate_can_report_absence() {
    assert_eq!(
        REQUIRED_X86_64_FEATURES.len(),
        5,
        "the ISA census list changed size; update this count in the same commit and say \
         in the list's doc comment why a feature was added or removed"
    );

    // AVX-512 is explicitly NOT part of the declared baseline: `CLAUDE.md` says it is
    // opt-in via `cfg(target_feature)`, and `x86-64-v3` does not include it. Evaluating
    // the same predicate against it shows the check reports absence rather than always
    // reporting presence — the positive control for this instrument.
    assert!(
        !cfg!(target_feature = "avx512f"),
        "avx512f is enabled, which the declared baseline does not include. That is not \
         itself an error, but this census's negative control now proves nothing: pick a \
         feature the build genuinely lacks, or drop this assertion and say why."
    );
}
