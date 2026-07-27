//! **The SDF shadow / contact-AO LEAF ORACLE** — the host-side correctness chain for the two
//! analytic leaves the Deferred resolve and the SDF marcher ship.
//!
//! | layer | test | the defect it catches |
//! |---|---|---|
//! | 3a | [`sdf_shadow_leaf_matches_host_mirror_bit_exact`] | the eDSL body that GENERATES the shipped `sdf_soft_shadow_ranged` and the host mirror the goldens use compute different numbers, or march different control flow |
//! | 3b | [`sdf_ao_leaf_agrees_with_host_mirror`] | the SHIPPED `sdf_ao` text and `goldens::host_ao` compute different things — at a pre-registered tolerance, and it is the WEAKER instrument |
//! | 5 | [`sdf_shadow_and_ao_consts_match_deferred_and_marcher`] | a tuning const drifts in ONE file while every body stays byte-identical |
//!
//! Plus [`nan_is_inert_in_the_shadow_leaf_but_turns_the_ao_leaf_black`], the executable record of
//! how each leaf behaves when a NaN reaches it — the two answers INVERT, and both directions have
//! been asserted wrongly in this repo before.
//!
//! # Provenance
//!
//! These layers were authored for the VB-SV0 SDF-shadow-on-mesh stage (rung S3) and SURVIVED its
//! revert, because none of them is about SV0: they pin `sdf_soft_shadow_ranged` in
//! `deferred_pbr.hlsl` and `sdf_ao` in `sdf_gbuffer_composite.hlsl` — leaves that shipped before
//! that stage and ship after it. The layers that DID go with it were the ones quantified over
//! SV0's own artifacts: the shared-header include contract, the two-copy `sdf_ao` body pin (there
//! is one copy again), and the whole face-normal family. The numbering is kept as-is rather than
//! renumbered, so the two measured findings recorded below stay citable against the record that
//! produced them.
//!
//! # What layer 3b is anchored to, and why it had to change
//!
//! 3b originally compared `shipped_ao_model` — an UNPINNED hand transcription of the HLSL — against
//! `goldens::host_ao`. Both sides were hand-written host models, and the shipped text was read for
//! ONE input (the tap count). A reviewer inverted the AO accumulation term `(h - d)` → `(d - h)` in
//! the shipped shader and every test in this file stayed GREEN — layer 5 because the consts were
//! untouched, 3b because it never looked at the body. An oracle that certifies a model rather than
//! the artifact is this campaign's signature defect, and that was one more instance of it.
//!
//! [`SHIPPED_AO_BODY`] closes it: 3b now asserts the committed `sdf_ao` body is EXACTLY the text
//! [`shipped_ao_model`] transcribes, before comparing anything. See that constant's doc for why a
//! textual anchor rather than a host evaluator of the HLSL.
//!
//! # Why the numeric layers compare two HOSTS and not host-vs-device
//!
//! The `cpu_gpu_sdf_agreement` family is GPU-FREE by design and no on-device leaf probe is ever
//! dispatched; building one means a new `.spv` plus a manifest row. What IS constructible, and what
//! these layers do, is to close the chain on the host: the eDSL body is the SINGLE SOURCE the
//! committed HLSL is emitted from (pinned by
//! `sdf_field_edsl_sync.rs::sdf_soft_shadow_ranged_matches_edsl_emit`), and
//! `goldens::host_soft_shadow_ranged` is the mirror the marcher's committed image goldens hold
//! against the real device. Pinning those two to each other joins the shipped leaf to measured GPU
//! behaviour without dispatching anything.

use std::cell::Cell;

use boyko_rhi_vulkan::compute::{
    AO_FALLOFF, AO_STEP, AO_STRENGTH, SDF_TRACE_MAX_IT, SHADOW_HIT_EPS, SHADOW_K, SHADOW_MINT,
    SHADOW_MINT_STEP, SHADOW_NDOTL_EPS, SHADOW_NORMAL_BIAS,
};
use boyko_rhi_vulkan::goldens::{host_ao, host_soft_shadow_ranged};
use boyko_sdf_math::{SdfEdit, sdf_edit_list, sdf_op};
use boyko_shaderdsl::EvalCf;
use boyko_shaderdsl::shadow::sdf_soft_shadow_ranged_body;

// ===============================================================================================
// Shader-source access
// ===============================================================================================

/// Reads a committed shader next to this crate, LF-normalized so body comparisons and byte offsets
/// mean the same thing under either line-ending convention.
fn shader(name: &str) -> String {
    let path = format!("{}/shaders/{name}", env!("CARGO_MANIFEST_DIR"));
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("invariant: shaders/{name} must exist next to this crate: {e}"))
        .replace("\r\n", "\n")
}

/// Extracts a `<ret> NAME(<args>) { ... }` function — the signature line through its MATCHING
/// closing brace — out of a committed shader.
///
/// A brace COUNTER, not a first-`}` scan: the leaves this file pins carry a nested loop, so the
/// first `}` closes the loop and not the function. A local copy of the helper
/// `sdf_field_edsl_sync.rs` uses; integration tests are separate crates and that one is private to
/// its own binary.
fn extract_fn(src: &str, sig: &str) -> String {
    let start = src
        .find(sig)
        .unwrap_or_else(|| panic!("the committed shader is missing `{sig}`"));
    let after = &src[start..];
    let open = after
        .find('{')
        .expect("invariant: a function signature must be followed by an opening brace");
    let mut depth = 0i32;
    for (i, ch) in after[open..].char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return after[..open + i + 1].to_string();
                }
            }
            _ => {}
        }
    }
    panic!("unbalanced braces extracting `{sig}` — the function never closed");
}

// ===============================================================================================
// LAYER 5 — the tuning-const pin, across every file that redeclares one
// ===============================================================================================

/// The two sources that carry the SHADOW march tuning block: the Deferred resolve (whose
/// `sdf_soft_shadow_ranged` is the eDSL-generated leaf) and the SDF marcher (whose own copy of the
/// march is what the image goldens exercise). Both must agree, because both call a march whose
/// numeric behaviour is entirely in these names.
const SHADOW_CONST_SOURCES: [&str; 2] = ["deferred_pbr.hlsl", "sdf_gbuffer_composite.hlsl"];

/// The one source that carries the AO tuning block. A single-source list is NOT a vacuous
/// selection here: the comparison is shipped-value-versus-HOST-MIRROR (`goldens::host_ao`'s
/// consts), so it reds on any drift in either. What it cannot catch — two shipped copies forking —
/// is a defect that does not exist while `sdf_ao` has exactly one definition, and the day a second
/// appears this list is where it is added.
const AO_CONST_SOURCES: [&str; 1] = ["sdf_gbuffer_composite.hlsl"];

/// Returns the right-hand side of a `static const <type> NAME = <rhs>;` declaration, with any
/// trailing `// comment` stripped.
///
/// Anchored on the DECLARATION, not on the bare name: a `find("SHADOW_K")` happily matches the
/// word inside a doc comment, so deleting a declaration while a comment survived would leave the
/// gate green. That is this campaign's signature defect and it is cheap to exclude here.
fn static_const_rhs(text: &str, name: &str) -> Option<String> {
    const PREFIX: &str = "static const ";
    for line in text.lines() {
        let Some(after) = line.trim_start().strip_prefix(PREFIX) else {
            continue;
        };
        let Some(eq) = after.find('=') else { continue };
        let mut lhs = after[..eq].split_whitespace();
        // Exactly two tokens before the `=`: the type and the name. A third means this is not a
        // scalar declaration and the RHS parse below would be meaningless.
        let (Some(_ty), Some(decl_name), None) = (lhs.next(), lhs.next(), lhs.next()) else {
            continue;
        };
        if decl_name != name {
            continue;
        }
        let rhs = &after[eq + 1..];
        let rhs = rhs.split("//").next().unwrap_or(rhs);
        let rhs = rhs.split(';').next().unwrap_or(rhs);
        return Some(rhs.trim().to_string());
    }
    None
}

/// Evaluates one HLSL constant to the `f32` the compiler folds it to.
///
/// The grammar is deliberately the SMALLEST one the shipped declarations use — a literal, a
/// symbol, or a product of those (`16.0 * GRAD_H`, `2.0 * EPS`, `128u`). Anything richer appearing
/// in a tuning block later must extend this rather than be silently mis-evaluated, which is why an
/// unparsable atom recurses-then-PANICS instead of being skipped.
///
/// `own` is the declaring file (`own_name` is its filename, carried purely so a missing-declaration
/// panic names the file it looked in — layer 5 walks a 21-row table and a red that does not say
/// WHICH file lost the declaration is a red that has to be bisected before it can be acted on);
/// `shared` is `sdf_field.hlsli`, where `GRAD_H` lives for every consumer. The product folds
/// left-to-right from `1.0`, matching both HLSL's evaluation order and the host mirrors' (`1.0 * a`
/// is exact, so the seed cannot perturb the result).
fn eval_hlsl_const(name: &str, own_name: &str, own: &str, shared: &str, depth: u32) -> f32 {
    assert!(
        depth < 4,
        "invariant: `{name}` recursed {depth} levels while evaluating shaders/{own_name} — the \
         shipped tuning blocks are at most `<literal> * <symbol>` deep, so this is a cycle or an \
         unexpected declaration shape"
    );
    let rhs = static_const_rhs(own, name)
        .or_else(|| static_const_rhs(shared, name))
        .unwrap_or_else(|| {
            panic!(
                "no `static const <type> {name} = ...;` declaration found in shaders/{own_name}, \
                 nor in the shared shaders/sdf_field.hlsli fallback"
            )
        });
    rhs.split('*')
        .map(|atom| {
            let atom = atom.trim();
            // HLSL's `u` / `f` suffixes are not Rust float syntax; strip them before parsing.
            match atom.trim_end_matches(['u', 'U', 'f', 'F']).parse::<f32>() {
                Ok(v) => v,
                Err(_) => eval_hlsl_const(atom, own_name, own, shared, depth + 1),
            }
        })
        .product()
}

/// **Layer 5.** Every file that redeclares a shadow-march or contact-AO tuning constant must fold
/// it to the SAME `f32`, and that `f32` must be the host mirror's.
///
/// # The defect this catches — and the one a body pin structurally cannot
///
/// A body pin holds the `sdf_ao` / `sdf_soft_shadow_ranged` TEXT. The consts live outside it.
/// Change `AO_FALLOFF` in the marcher only and the body stays byte-identical while computing
/// something different. This is the only instrument that reds for it.
///
/// Values are compared as FOLDED `f32`, not as text: `16.0 * GRAD_H` and `0.008` are the same
/// number and neither spelling is wrong, while `16.0 * GRAD_H` in a file whose `GRAD_H` differs is
/// a real divergence a text pin would call green.
#[test]
fn sdf_shadow_and_ao_consts_match_deferred_and_marcher() {
    let field_hlsli = shader("sdf_field.hlsli");

    // (the shipped const's name, the host mirror it must equal, the files that must declare it).
    //
    // `MAX_IT` appears TWICE, against two DIFFERENT host constants, and that is the point rather
    // than an oversight: `boyko_shaderdsl::shadow::MAX_IT` is the trip count layer 3a's eDSL side
    // executes, and `compute::SDF_TRACE_MAX_IT` (the public mirror of the private `SDF_MAX_IT`) is
    // the one its MIRROR side executes. Pinning both to the shipped `MAX_IT` is what makes them
    // pinned to each other.
    //
    // Why a PIN rather than making layer 3a's fixture sense it: MEASURED — doubling `SDF_MAX_IT`
    // to `256` leaves every test in this file green, because the fixture's rays terminate by
    // occluder hit or by `t > t_max` and never by exhausting the budget (layer 3a reports the
    // observed maximum field-call count per ray, and it is far below `MAX_IT`). Making the
    // fixture sensitive would mean designing an edit list whose only purpose is to force rays to
    // crawl at the `SHADOW_MINT_STEP` floor for >128 steps — a fixture fitted to the gate rather
    // than to the leaf, and one whose sensitivity would still be incidental. A pin is
    // deterministic and says what it means.
    //
    // `T_MAX` is pinned only against `boyko_shaderdsl::shadow::T_MAX`, deliberately: the RANGED
    // leaf takes its escape bound as a runtime PARAMETER, so `compute::SDF_T_MAX` is not executed
    // by layer 3a's mirror side at all and pinning it here would assert a relationship this file
    // does not depend on.
    let consts: [(&str, f32, &[&str]); 12] = [
        ("MAX_IT", boyko_shaderdsl::shadow::MAX_IT as f32, &SHADOW_CONST_SOURCES),
        ("MAX_IT", SDF_TRACE_MAX_IT as f32, &SHADOW_CONST_SOURCES),
        ("T_MAX", boyko_shaderdsl::shadow::T_MAX, &SHADOW_CONST_SOURCES),
        ("SHADOW_K", SHADOW_K, &SHADOW_CONST_SOURCES),
        ("SHADOW_MINT", SHADOW_MINT, &SHADOW_CONST_SOURCES),
        ("SHADOW_MINT_STEP", SHADOW_MINT_STEP, &SHADOW_CONST_SOURCES),
        ("SHADOW_HIT_EPS", SHADOW_HIT_EPS, &SHADOW_CONST_SOURCES),
        ("SHADOW_NDOTL_EPS", SHADOW_NDOTL_EPS, &SHADOW_CONST_SOURCES),
        ("SHADOW_NORMAL_BIAS", SHADOW_NORMAL_BIAS, &SHADOW_CONST_SOURCES),
        ("AO_STEP", AO_STEP, &AO_CONST_SOURCES),
        ("AO_FALLOFF", AO_FALLOFF, &AO_CONST_SOURCES),
        ("AO_STRENGTH", AO_STRENGTH, &AO_CONST_SOURCES),
    ];

    let mut checked = 0usize;
    for (name, host_value, files) in consts {
        for file in files {
            let src = shader(file);
            let value = eval_hlsl_const(name, file, &src, &field_hlsli, 0);
            assert_eq!(
                value.to_bits(),
                host_value.to_bits(),
                "`{name}` in shaders/{file} folds to {value} ({:#010x}), but the host mirror is \
                 {host_value} ({:#010x}). Every consumer of these leaves must agree on the tuning \
                 block or the same body computes different things in different files — a \
                 divergence no body-identity pin can see.",
                value.to_bits(),
                host_value.to_bits()
            );
            checked += 1;
        }
    }

    // MEASURED SELECTION SIZE — 9 shadow rows × 2 sources + 3 AO consts × 1 source. Asserted so
    // that quietly dropping a row from the table (the "gate stops covering things" failure) reds
    // instead of passing over a smaller set.
    assert_eq!(
        checked, 21,
        "the const × file selection changed shape: expected 9×2 shadow rows (MAX_IT is pinned \
         against BOTH host copies) + 3×1 AO = 21 checks"
    );
}

// ===============================================================================================
// The deterministic sample fixture the two numeric layers share
// ===============================================================================================

/// A fixed-seed xorshift64. Deterministic across runs and platforms — the fixture must be a
/// property of the test, not of the machine, or a red is not reproducible.
fn xorshift(state: &mut u64) -> u64 {
    let mut x = *state;
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    *state = x;
    x
}

/// A uniform `[0, 1)` `f32` from the top 24 bits — the full mantissa, no bias from low-bit reuse.
fn unit(state: &mut u64) -> f32 {
    ((xorshift(state) >> 40) as f32) / ((1u32 << 24) as f32)
}

/// A uniformly-distributed unit vector (Archimedes: `z` uniform on `[-1, 1]`, `phi` uniform).
fn unit_vector(state: &mut u64) -> [f32; 3] {
    let z = 2.0 * unit(state) - 1.0;
    let phi = 2.0 * std::f32::consts::PI * unit(state);
    let r = (1.0 - z * z).max(0.0).sqrt();
    [r * phi.cos(), r * phi.sin(), z]
}

/// A sample point in the box that contains [`oracle_edits`] with room around it, so rays start
/// inside the field, on it, and clear of it.
fn sample_point(state: &mut u64) -> [f32; 3] {
    [
        3.0 * unit(state) - 1.5,
        3.0 * unit(state) - 1.5,
        3.0 * unit(state) - 1.5,
    ]
}

/// The edit list both numeric layers march against: a smooth-union pair plus a box and a
/// subtraction, so the field exercises `smin`/`smax` rather than one analytic sphere and the rays
/// reach every branch of the march (hard hit, penumbra grazing, `t > t_max` escape, far field).
fn oracle_edits() -> Vec<SdfEdit> {
    vec![
        SdfEdit::sphere([0.0, 0.0, 0.0], 0.5, sdf_op::UNION, 0.0),
        SdfEdit::sphere([0.45, 0.15, 0.0], 0.3, sdf_op::UNION, 0.12),
        SdfEdit::box_shape([-0.4, -0.2, 0.1], [0.25, 0.35, 0.2], sdf_op::UNION, 0.05),
        SdfEdit::sphere([0.1, 0.35, 0.25], 0.22, sdf_op::SUBTRACT, 0.0),
    ]
}

/// The sample count layer 3a is fixed at (`>= 4096`). Shared with 3b so the two leaves are judged
/// over the same amount of evidence.
const ORACLE_SAMPLES: usize = 4096;

// No `#[inline]`. Private to this test binary, so LTO already sees the body, and principle 7 makes
// inlining a MEASURED decision rather than a default.
fn v_dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

// ===============================================================================================
// LAYER 3a — the SHADOW leaf, bit-exact
// ===============================================================================================

/// **Layer 3a.** The eDSL body that GENERATES the shipped `sdf_soft_shadow_ranged` and
/// `goldens::host_soft_shadow_ranged` return BIT-IDENTICAL `f32` over a 4096-sample `(P, N, L)`
/// fixture.
///
/// # The defect this catches, and why nothing else can
///
/// `sdf_soft_shadow_ranged` in `deferred_pbr.hlsl` is emitted from
/// `boyko_shaderdsl::shadow::sdf_soft_shadow_ranged_body::<EmitCf>`, and
/// `sdf_field_edsl_sync.rs::sdf_soft_shadow_ranged_matches_edsl_emit` pins the committed text to
/// that emission. What no text pin can see is the eDSL's OTHER instantiation — `<EvalCf>`, the host
/// oracle — whose tuning constants (`boyko_shaderdsl::shadow::SHADOW_K` and friends) are SEPARATE
/// declarations from the shader's, spelled symbolically in the emitted HLSL precisely so the
/// emitted text does not move when they do. Retune one and the emitted HLSL stays byte-identical,
/// every span pin stays green, and the host oracle silently begins modelling a different march than
/// the GPU runs. MEASURED: a one-step change to `boyko_shaderdsl`'s `SHADOW_K` reds this test while
/// all 24 eDSL span pins stay green.
///
/// It also closes the chain to a GPU-validated reference without dispatching anything:
/// `host_soft_shadow_ranged` is the mirror the marcher's committed image goldens hold against the
/// real device, so pinning the eDSL body to it transitively pins the shipped leaf to measured GPU
/// behaviour.
///
/// # Why bit-exactness is reachable here and not for the AO leaf
///
/// The march is `min` / `max` / `*` / `/` / `+` / `clamp` over the analytic field — every
/// operation IEEE-exact or correctly rounded, no transcendental anywhere. `EvalCf`'s scalar ops
/// are `f32::min` / `f32::max` / `clamp(0, 1)` and the arithmetic is associated identically on
/// both sides (`SHADOW_K * d / t` is `(SHADOW_K * d) / t` in each), so equality is `to_bits()` and
/// never an epsilon. Layer 3b is the tolerance case, and its reason is written down there.
///
/// # The fixture's domain, stated because it is a scoping decision
///
/// Only samples with `dot(N, L) > SHADOW_NDOTL_EPS` are compared. That is not fixture-fitting: the
/// mirror carries the shader's hand-written back-face preamble (returning `0.0`) while the eDSL
/// body is the loop+tail SPAN only, with the preamble owned by the caller — and the shipped call
/// sites gate on exactly this condition. Comparing outside the region either side is ever called in
/// would be comparing two functions neither of which ships there.
///
/// # Control flow is compared too, not only the returned number
///
/// Both sides call `field` exactly once per march iteration, so COUNTING those calls measures each
/// side's trip count exactly, through a seam that already exists and with no third hand model.
/// Result equality alone would let two different marches agree by coincidence — a `clamp` tail
/// saturating at `1.0`, or a `res` that reached its minimum before the paths diverged. It also
/// puts the layer-5 `MAX_IT` pin's justification on the record as a MEASURED number: the observed
/// maximum trip count is printed, and it is what shows the fixture cannot sense the trip-count
/// budget itself.
#[test]
fn sdf_shadow_leaf_matches_host_mirror_bit_exact() {
    let edits = oracle_edits();
    // The field-call counter lives outside the closure so BOTH sides march through the same
    // instrumented gateway. A closure capturing shared references is `Copy`, so it can still be
    // passed by value to the eDSL body once per sample.
    let calls = Cell::new(0u32);
    let field = |q: [f32; 3]| {
        calls.set(calls.get() + 1);
        sdf_edit_list(&edits, q)
    };
    let t_max = boyko_shaderdsl::shadow::T_MAX;

    let mut state = 0x5D0_5AD0_u64;
    let mut compared = 0usize;
    let mut drawn = 0usize;
    let mut mismatches = 0usize;
    let mut first_mismatch = String::new();
    let mut flow_mismatches = 0usize;
    let mut first_flow_mismatch = String::new();
    let mut max_iters = 0u32;
    // How many samples actually reached the accumulator's interesting regions, reported so a
    // fixture that degenerated into "every ray misses" is visible rather than silently vacuous.
    let mut fully_occluded = 0usize;
    let mut penumbral = 0usize;

    while compared < ORACLE_SAMPLES {
        drawn += 1;
        assert!(
            drawn < ORACLE_SAMPLES * 64,
            "invariant: the `dot(N, L) > SHADOW_NDOTL_EPS` half-space should accept ~half the \
             draws; {drawn} draws yielded only {compared} comparable samples"
        );
        let origin = sample_point(&mut state);
        let n = unit_vector(&mut state);
        let l = unit_vector(&mut state);
        if v_dot(n, l) <= SHADOW_NDOTL_EPS {
            continue;
        }
        compared += 1;

        calls.set(0);
        let edsl = {
            let out = Cell::new(0.0f32);
            sdf_soft_shadow_ranged_body::<EvalCf, _>(origin, n, l, t_max, field, &out);
            out.get()
        };
        let edsl_iters = calls.get();

        calls.set(0);
        let mirror = host_soft_shadow_ranged(origin, n, l, t_max, &field);
        let mirror_iters = calls.get();

        max_iters = max_iters.max(edsl_iters).max(mirror_iters);

        if edsl == 0.0 {
            fully_occluded += 1;
        } else if edsl < 1.0 {
            penumbral += 1;
        }

        if edsl_iters != mirror_iters {
            flow_mismatches += 1;
            if first_flow_mismatch.is_empty() {
                first_flow_mismatch = format!(
                    "P={origin:?} N={n:?} L={l:?}: eDSL marched {edsl_iters} field calls, host \
                     mirror marched {mirror_iters}"
                );
            }
        }

        if edsl.to_bits() != mirror.to_bits() {
            mismatches += 1;
            if first_mismatch.is_empty() {
                first_mismatch = format!(
                    "P={origin:?} N={n:?} L={l:?}: eDSL {edsl} ({:#010x}) vs host mirror {mirror} \
                     ({:#010x})",
                    edsl.to_bits(),
                    mirror.to_bits()
                );
            }
        }
    }

    // Fixture adequacy: a march that never occludes and never grazes would compare `1.0` against
    // `1.0` 4096 times and prove nothing. Both regions must be populated.
    assert!(
        fully_occluded > 0 && penumbral > 0,
        "the layer-3a fixture is VACUOUS: {fully_occluded} fully-occluded and {penumbral} \
         penumbral samples out of {compared}. Bit-equality on a fixture where every ray returns \
         the far-field `1.0` tests nothing about the march."
    );

    assert_eq!(
        flow_mismatches, 0,
        "the eDSL body and `goldens::host_soft_shadow_ranged` took DIFFERENT numbers of march \
         steps in {flow_mismatches}/{compared} samples. Each side calls `field` exactly once per \
         iteration, so this is a control-flow divergence — a step size, an escape bound, or a hit \
         threshold — which a result comparison can miss whenever the clamped tail saturates.\n\
         first: {first_flow_mismatch}"
    );

    assert_eq!(
        mismatches, 0,
        "the eDSL `sdf_soft_shadow_ranged_body::<EvalCf>` DRIFTED from \
         `goldens::host_soft_shadow_ranged` in {mismatches}/{compared} samples. These are the two \
         host models of ONE shipped leaf: the eDSL body EMITS the committed HLSL, the mirror is \
         what the marcher's image goldens hold against the GPU. A divergence means the host oracle \
         and the device now disagree, and no span pin can see it because the tuning constants are \
         spelled SYMBOLICALLY in the emitted text. Audit `boyko_shaderdsl::shadow`'s consts against \
         `boyko_rhi_vulkan::compute`'s.\nfirst: {first_mismatch}"
    );

    eprintln!(
        "layer 3a: {compared} (P,N,L) samples, ALL bit-identical and step-for-step identical \
         ({fully_occluded} fully occluded, {penumbral} penumbral, {} far-field) after {drawn} \
         draws; worst observed trip count {max_iters} of MAX_IT={} — the fixture NEVER exhausts \
         the budget, which is why layer 5 PINS the trip count instead of sensing it",
        compared - fully_occluded - penumbral,
        boyko_shaderdsl::shadow::MAX_IT
    );
}

// ===============================================================================================
// LAYER 3b — the AO leaf, at a PRE-REGISTERED tolerance, and it is the WEAKER instrument
// ===============================================================================================

/// The pre-registered agreement bound for layer 3b — fixed BEFORE the run and never widened to
/// make a failing run pass.
///
/// # Why an ABSOLUTE bound and not a ULP bound
///
/// A ULP bound on this leaf's RESULT is not a usable instrument, for an arithmetic reason rather
/// than a matter of taste. What is being tolerated is `AO_FALLOFF.powi(i)` (the host mirror) versus
/// `pow(AO_FALLOFF, (float)i)` (what `sdf_gbuffer_composite.hlsl` spells) — a few ULP in each of
/// five weights, i.e. a bounded RELATIVE error in the accumulator `occ`. The leaf then returns
/// `clamp(1 - AO_STRENGTH * occ, 0, 1)`, and that subtraction is a CANCELLATION: as `occ`
/// approaches `1` the result approaches `0`, the result's ULP shrinks without bound, and a fixed
/// ~2e-7 absolute error in `occ` becomes an unbounded ULP count in the result. A ULP threshold
/// would therefore be set by whichever darkest sample the fixture happened to contain — a property
/// of the fixture, not of the leaf, which is the opposite of what pre-registration is for.
///
/// `1e-6` is registered instead: four orders tighter than the ±3/255 (`0.0118`) the host mirror's
/// own doc concedes against the GPU, and far tighter than any real drift (a tap-count change, a
/// const change, a sign flip) could hide beneath. The observed maximum — in both absolute and ULP
/// terms — is printed on every run, so the margin stays visible instead of becoming folklore.
///
/// # MEASURED, and it narrows what this tolerance is actually for
///
/// On this host the observed maximum is **0 — bit-identical, 0 ULP, over all 4096 samples**.
/// Rust's `powi` and `powf` return the same bits for `0.95^{1..5}` here, so the `powi`-vs-`pow` gap
/// usually cited as making bit-exactness "UNREACHABLE" does not exist *between these two host
/// models*. The genuinely unreachable comparison is host-`powi` versus the DEVICE's `pow`, and no
/// host test performs it. So the honest reading of this bound is: it is a PORTABILITY margin
/// against a libm whose `pow` differs, not the measurement of a gap that is present today. Do not
/// cite it as evidence that the shader and the mirror differ; they do not, at this instrument's
/// resolution.
const AO_LEAF_MAX_ABS_DEVIATION: f32 = 1.0e-6;

/// The signed-lexicographic ULP distance between two `f32`. Reported alongside the absolute
/// deviation so a ULP figure is on the record even though the GATE is absolute (see
/// [`AO_LEAF_MAX_ABS_DEVIATION`] for why).
fn ulp_distance(a: f32, b: f32) -> u64 {
    fn ordered(x: f32) -> i64 {
        let bits = x.to_bits() as i64;
        if bits < 0x8000_0000 {
            bits
        } else {
            -(bits - 0x8000_0000)
        }
    }
    (ordered(a) - ordered(b)).unsigned_abs()
}

/// The COMMITTED body of `sdf_ao`, quoted verbatim — the text [`shipped_ao_model`] transcribes.
///
/// # Why layer 3b needs this, in one sentence
///
/// Without it 3b compares two hand-written host models to each other and never looks at the
/// shader: a reviewer inverted the accumulation term `(h - d)` → `(d - h)` in the shipped shader
/// and every test in this file stayed green (layer 5 because the consts were untouched, 3b because
/// it read the body for nothing but the tap count).
///
/// # Why a TEXTUAL anchor rather than evaluating the shipped body
///
/// The stronger-sounding option is to parse this body and interpret it — [`extract_fn`] already
/// locates the span. What does not exist is an evaluator: interpreting
/// `for (uint i = 1u; i <= 5u; ++i) { ... occ += ... }` means writing a small HLSL statement-and-
/// expression interpreter, and that interpreter would be an UNPINNED hand model of HLSL semantics
/// — exactly the thing being fixed, moved one level up, plus several hundred lines of it. A
/// verbatim anchor makes the model's provenance checkable by eye and reds on ANY shipped-side
/// edit, which is what the defect required.
///
/// Divergence reds in both directions: a shipped-side edit reds THIS assertion, and an edit to
/// [`shipped_ao_model`] alone reds the numeric comparison against `goldens::host_ao`, which is an
/// independent third party to both.
const SHIPPED_AO_BODY: &str = "\
float sdf_ao(float3 p, float3 n) {
    float occ = 0.0;
    [unroll]
    for (uint i = 1u; i <= 5u; ++i) {
        float h = (float)i * AO_STEP;
        float d = field_distance(p + n * h);
        occ += (h - d) * pow(AO_FALLOFF, (float)i);
    }
    return clamp(1.0 - AO_STRENGTH * occ, 0.0, 1.0);
}";

/// The AO tap count read OUT OF THE COMMITTED MARCHER, not hardcoded here.
///
/// Strictly redundant now that [`SHIPPED_AO_BODY`] pins the whole body — a tap-count change reds
/// there too. It is kept because it is the SHARPEST red for the single most likely edit (the loop
/// bound), and because it keeps [`shipped_ao_model`] parameterized by the shipped text rather than
/// by a literal, so the two reds name the same cause instead of one of them being a puzzle.
fn marcher_ao_tap_count(marcher: &str) -> u32 {
    let body = extract_fn(marcher, "float sdf_ao(float3 p, float3 n)");
    const MARKER: &str = "for (uint i = 1u; i <= ";
    let start = body.find(MARKER).unwrap_or_else(|| {
        panic!(
            "sdf_gbuffer_composite.hlsl's `sdf_ao` no longer opens its accumulation with \
             `{MARKER}...`. The tap count is read from the shipped text on purpose (it is the one \
             AO input no const pin can see); re-point this reader at the new loop shape rather \
             than hardcoding a count here.\n--- body ---\n{body}"
        )
    }) + MARKER.len();
    let rest = &body[start..];
    let end = rest
        .find("u;")
        .expect("invariant: the tap-count loop bound must be a `u`-suffixed literal");
    rest[..end]
        .trim()
        .parse::<u32>()
        .unwrap_or_else(|e| panic!("unparsable AO tap count `{}`: {e}", &rest[..end]))
}

/// A transcription of [`SHIPPED_AO_BODY`] — statement for statement, in the shipped order — with
/// HLSL's `pow(AO_FALLOFF, (float)i)` spelled as Rust's `powf`, the one place it deliberately
/// differs from `goldens::host_ao`, which spells `powi`. `taps` comes from
/// [`marcher_ao_tap_count`].
///
/// The transcription is only worth anything because [`SHIPPED_AO_BODY`] is asserted against the
/// committed marcher before this function is called. Do not edit one without the other.
fn shipped_ao_model<F: Fn([f32; 3]) -> f32>(
    taps: u32,
    p: [f32; 3],
    n: [f32; 3],
    field: &F,
) -> f32 {
    let mut occ = 0.0f32;
    for i in 1..=taps {
        let h = (i as f32) * AO_STEP;
        let d = field([p[0] + n[0] * h, p[1] + n[1] * h, p[2] + n[2] * h]);
        occ += (h - d) * AO_FALLOFF.powf(i as f32);
    }
    (1.0 - AO_STRENGTH * occ).clamp(0.0, 1.0)
}

/// **Layer 3b — the WEAKER instrument of the pair. Read this before citing it.**
///
/// Layer 3a is bit-exact against an oracle that GENERATES the shipped text. This one cannot be.
/// There is no `sdf_ao` body in `boyko_shaderdsl`, so BOTH sides here are hand transcriptions, and
/// they are separated by a PLATFORM-DEPENDENT TRANSCENDENTAL — the host mirror computes
/// `AO_FALLOFF.powi(i)` where the HLSL computes `pow(AO_FALLOFF, (float)i)`. Bit-exactness against
/// that is UNREACHABLE, not merely untested, which is why this layer has a tolerance and 3a does
/// not.
///
/// # The defect it DOES catch
///
/// The shipped AO accumulation and `goldens::host_ao` — the mirror the marcher goldens hold against
/// the GPU — computing different things. The model side is ANCHORED to the shipped text by
/// [`SHIPPED_AO_BODY`], asserted first, so the mutation that reds it is an edit to the SHIPPED
/// shader and not only an edit to this test's own model.
#[test]
fn sdf_ao_leaf_agrees_with_host_mirror() {
    let marcher = shader("sdf_gbuffer_composite.hlsl");

    // FIRST — and this is the load-bearing part of this layer: the model below is a transcription
    // of a SPECIFIC piece of shipped text, so that text is pinned before it is trusted. Without
    // this the layer compares two hand models to each other and an inverted term passes.
    let shipped_body = extract_fn(&marcher, "float sdf_ao(float3 p, float3 n)");
    assert_eq!(
        shipped_body, SHIPPED_AO_BODY,
        "the committed `sdf_ao` in sdf_gbuffer_composite.hlsl is no longer the body \
         `shipped_ao_model` (tests/sdf_shadow_leaf_oracle.rs) transcribes. Layer 3b's numeric \
         comparison below runs the RUST model, so an unpinned edit here would leave it green while \
         the shipped leaf changed. Update `SHIPPED_AO_BODY` and `shipped_ao_model` together with \
         the shader, or state why the model may lag.\n\
         --- expected (the model's source) ---\n{SHIPPED_AO_BODY}\n\
         --- committed ---\n{shipped_body}"
    );

    let taps = marcher_ao_tap_count(&marcher);

    let edits = oracle_edits();
    let field = |q: [f32; 3]| sdf_edit_list(&edits, q);

    let mut state = 0xA0_5EED_u64;
    let mut worst_abs = 0.0f32;
    let mut worst_ulp = 0u64;
    let mut worst_at = String::new();
    let mut darkened = 0usize;
    // Tracked SEPARATELY from `worst_abs`, because a `max` over deviations is NaN-BLIND: `NaN >
    // worst_abs` is false, so a sample where one side is NaN and the other is a number would leave
    // `worst_abs` untouched and the gate green. That is the same class of mistake this repo's
    // standing note records for `NMin`/`NMax` — a comparison silently selecting the other operand
    // — and it is not acceptable in the instrument built to catch it.
    let mut non_finite = 0usize;
    let mut first_non_finite = String::new();

    for _ in 0..ORACLE_SAMPLES {
        let p = sample_point(&mut state);
        let n = unit_vector(&mut state);

        let mirror = host_ao(p, n, &field);
        let shipped = shipped_ao_model(taps, p, n, &field);

        if !mirror.is_finite() || !shipped.is_finite() {
            non_finite += 1;
            if first_non_finite.is_empty() {
                first_non_finite = format!(
                    "P={p:?} N={n:?}: mirror {mirror} ({:#010x}) vs shipped model {shipped} \
                     ({:#010x})",
                    mirror.to_bits(),
                    shipped.to_bits()
                );
            }
            continue;
        }

        if shipped < 1.0 {
            darkened += 1;
        }
        let abs = (mirror - shipped).abs();
        if abs > worst_abs {
            worst_abs = abs;
            worst_ulp = ulp_distance(mirror, shipped);
            worst_at = format!("P={p:?} N={n:?}: mirror {mirror} vs shipped model {shipped}");
        }
    }

    assert_eq!(
        non_finite, 0,
        "{non_finite}/{ORACLE_SAMPLES} layer-3b samples put a NaN or an infinity on at least one \
         side. Both leaves are `clamp`ed to `[0, 1]` over a finite analytic field, so a non-finite \
         result is a defect and not a tolerance question — and it must be counted rather than \
         folded into the worst-deviation `max`, which cannot see it (`NaN > x` is false). Note the \
         two hosts do NOT agree here either: Rust's `f32::clamp` passes a NaN through, while the \
         shipped `clamp` lowers to `NMin(NMax(NaN, 0), 1)` = `0`, a BLACK pixel.\n\
         first: {first_non_finite}"
    );

    // Fixture adequacy: if no sample darkens, both sides saturate at `1.0` and the comparison is
    // vacuous — the empty-edit-list trap this repo has hit before.
    assert!(
        darkened > 0,
        "the layer-3b fixture is VACUOUS: 0 of {ORACLE_SAMPLES} samples darken, so both models \
         return the far-field `1.0` everywhere and agreement means nothing"
    );

    assert!(
        worst_abs <= AO_LEAF_MAX_ABS_DEVIATION,
        "the shipped `sdf_ao` model ({taps} taps, read from sdf_gbuffer_composite.hlsl) diverged \
         from `goldens::host_ao` by {worst_abs} — above the PRE-REGISTERED \
         {AO_LEAF_MAX_ABS_DEVIATION}. Do NOT widen the bound to make this pass: it is four orders \
         tighter than the ±3/255 the mirror concedes against the GPU, so anything reaching it is a \
         real divergence (a tap count, a tuning const, or a term). worst: {worst_at}"
    );

    eprintln!(
        "layer 3b (WEAKER instrument — powi-vs-pow, tolerance not bit-exactness; model ANCHORED \
         to the committed body): {taps} taps, {ORACLE_SAMPLES} samples, {darkened} darkened, \
         {non_finite} non-finite; observed max |deviation| = {worst_abs:e} ({worst_ulp} ULP) \
         vs pre-registered {AO_LEAF_MAX_ABS_DEVIATION:e}"
    );
}

// ===============================================================================================
// The NaN behaviour of the two leaves — they INVERT, and both directions have been asserted wrongly
// ===============================================================================================

/// GLSL.std.450 `NMin` — the lowering HLSL's `min` (and therefore `clamp`) takes: the NON-NaN
/// operand wins, rather than NaN propagating.
fn nmin(a: f32, b: f32) -> f32 {
    if a.is_nan() {
        b
    } else if b.is_nan() {
        a
    } else {
        a.min(b)
    }
}

/// GLSL.std.450 `NMax`, the companion to [`nmin`].
fn nmax(a: f32, b: f32) -> f32 {
    if a.is_nan() {
        b
    } else if b.is_nan() {
        a
    } else {
        a.max(b)
    }
}

/// HLSL `clamp(x, lo, hi)` as SPIR-V lowers it: `NMin(NMax(x, lo), hi)`.
fn hlsl_clamp(x: f32, lo: f32, hi: f32) -> f32 {
    nmin(nmax(x, lo), hi)
}

/// **The executable record of how a NaN behaves in each leaf — a claim this repo has now had wrong
/// in BOTH directions.**
///
/// The first version of the claim was "a NaN term degrades to no contribution". The correction was
/// "the leaf's `clamp(res, 0, 1)` tail lowers to `NMin(NMax(NaN, 0), 1)` = `0`, so a NaN turns the
/// pixel BLACK". **The correction named the wrong leaf.** MEASURED, and asserted below:
///
/// * The SHADOW leaf's accumulator can never BE NaN, so its `clamp` tail never sees one.
///   `res = min(res, SHADOW_K * d / t)` drops a NaN `d` under BOTH `f32::min` and `NMin`, and
///   `t = t + max(d / L, SHADOW_MINT_STEP)` drops it again, so the march advances normally and the
///   leaf returns `1.0` — fully LIT — on the host AND through the HLSL lowering. On this leaf the
///   two answers do NOT differ, and "a NaN is inert" happens to be true.
/// * The AO leaf is where the black-pixel property actually holds. `occ += (h - d) * ...` is a
///   PLAIN ADDITION with no `min`/`max` to launder anything, so one NaN tap makes `occ` NaN, the
///   NaN reaches `clamp(1 - AO_STRENGTH * occ, 0, 1)`, and there the two hosts invert: Rust's
///   `f32::clamp` returns NaN, the shipped `clamp` returns **`0.0`** — full occlusion, a BLACK
///   pixel.
///
/// The binding consequence: nothing downstream may lean on either leaf being inert, and the
/// difference between them is not a detail — a caller that feeds a NaN march origin gets a lit
/// pixel from one term and a black pixel from the other.
#[test]
fn nan_is_inert_in_the_shadow_leaf_but_turns_the_ao_leaf_black() {
    // The lowering fact both halves rest on, stated once.
    assert_eq!(
        hlsl_clamp(f32::NAN, 0.0, 1.0),
        0.0,
        "invariant: `clamp(NaN, 0, 1)` under NMin/NMax selects the extreme operand `0.0`"
    );
    assert!(
        f32::NAN.clamp(0.0, 1.0).is_nan(),
        "invariant: Rust's `f32::clamp` passes NaN THROUGH — the divergence from the shipped \
         lowering that makes the AO half of this test meaningful"
    );

    // ---- The SHADOW leaf: a NaN tap at EVERY iteration, and the leaf is still inert. ----
    //
    // A field that returns NaN unconditionally, rather than a NaN march origin: this forces the
    // NaN into the accumulator directly instead of depending on what the analytic field does with
    // a NaN query, which is a separate question.
    let nan_field = |_q: [f32; 3]| f32::NAN;

    let out = Cell::new(0.0f32);
    sdf_soft_shadow_ranged_body::<EvalCf, _>(
        [0.0, 0.0, 0.0],
        [0.0, 0.0, 1.0],
        [0.0, 0.0, 1.0],
        boyko_shaderdsl::shadow::T_MAX,
        nan_field,
        &out,
    );
    let shadow_host = out.get();
    assert_eq!(
        shadow_host, 1.0,
        "the host shadow leaf returned {shadow_host} for an all-NaN field; the expected answer is \
         `1.0` because `min(res, NaN)` keeps `res` and `max(NaN, step)` keeps the step"
    );
    // And the SAME answer through the shipped lowering: the two ops that touch the NaN are the
    // exact ones `NMin`/`NMax` launder it out of, so the value reaching the `clamp` tail is `1.0`
    // on both sides and the tail is a no-op.
    assert_eq!(
        nmin(1.0, f32::NAN),
        1.0,
        "invariant: `res = min(res, NaN)` under NMin keeps `res` — this is why the shadow leaf's \
         `clamp` tail never sees a NaN, and therefore why `a NaN comes out black` does NOT \
         describe this leaf"
    );
    assert_eq!(
        nmax(f32::NAN, SHADOW_MINT_STEP),
        SHADOW_MINT_STEP,
        "invariant: `t = t + max(NaN, SHADOW_MINT_STEP)` under NMax advances by the floor, so the \
         march terminates instead of stalling"
    );
    assert_eq!(
        hlsl_clamp(shadow_host, 0.0, 1.0),
        1.0,
        "the shipped tail applied to the value the march actually delivers is the identity — the \
         shadow leaf is NaN-INERT, on both hosts, and returns `1.0` (fully lit)"
    );

    // ---- The AO leaf: the same NaN, and here it does NOT launder. ----
    //
    // `host_ao`'s accumulator is a plain `+=`, so one NaN tap poisons it, and Rust's `clamp`
    // passes that NaN out unchanged — which is precisely what lets the assertion below read the
    // pre-clamp accumulator without transcribing the body a third time.
    let p = [0.25f32, -0.5, 0.75];
    let n = [0.0f32, 0.0, 1.0];
    let ao_host = host_ao(p, n, &nan_field);
    assert!(
        ao_host.is_nan(),
        "the host AO leaf returned {ao_host} for an all-NaN field; `occ += (h - d) * ...` has no \
         `min`/`max` to drop the NaN, so the accumulator — and, through Rust's NaN-transparent \
         `clamp`, the result — must be NaN"
    );
    assert_eq!(
        hlsl_clamp(ao_host, 0.0, 1.0),
        0.0,
        "the SHIPPED `clamp` applied to the very accumulator value the AO leaf delivers is \
         `0.0` — FULL occlusion, a BLACK pixel, where the host says NaN. THIS is the leaf the \
         black-pixel claim describes; the shadow leaf above is not."
    );

    eprintln!(
        "NaN: an all-NaN field yields shadow={shadow_host} on BOTH hosts (inert, fully lit) but \
         AO={ao_host} on the host vs 0.0 (BLACK) through the shipped `clamp` lowering. The \
         inversion lives in the AO leaf, not the shadow leaf."
    );
}
