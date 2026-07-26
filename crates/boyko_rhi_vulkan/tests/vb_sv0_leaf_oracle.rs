//! **VB-SV0 rung S3 — the LEAF ORACLE** (`docs/VB-SV0-SDF-SHADOW-PLAN.md` §6 "S3", Rev 6).
//!
//! S3 changes NO production behaviour: `sv0_mode` stays `0`, no `.spv` moves, no shipped host
//! path is touched. It lands the verification layers S2's byte-identity gates structurally cannot
//! provide, because every S2 gate is a gate on the DARK path while SV0's numeric behaviour exists
//! only on the ARMED one.
//!
//! | layer | test | the defect it catches |
//! |---|---|---|
//! | 1 | `sdf_field_edsl_sync.rs::sv0_shadow_leaf_consumers_satisfy_include_contract` | any of the FOUR consumers loses or misorders a precondition the shared header needs |
//! | 2 | [`sdf_ao_body_matches_shared_header`] | the marcher's `sdf_ao` and the shared header's hand copy diverge |
//! | 3a | [`sv0_shadow_leaf_matches_host_mirror_bit_exact`] | the eDSL body that GENERATES the shipped leaf and the host mirror the goldens use compute different numbers, or march different control flow |
//! | 3b | [`sv0_ao_leaf_agrees_with_host_mirror`] | the SHIPPED `sdf_ao` text and `goldens::host_ao` compute different things — at a pre-registered tolerance, and it is the WEAKER instrument |
//! | 4 | [`vb_sv0_face_normal_body_matches_host_mirror`] + the property tests beside it | a degenerate or non-finite triangle reaches the shadow leaf with a NaN march origin |
//! | 5 | [`sv0_consts_match_deferred_and_marcher`] | a tuning const drifts in ONE file while every body stays byte-identical |
//!
//! # What layer 3b is anchored to, and why it had to change
//!
//! 3b originally compared `shipped_ao_model` — an UNPINNED hand transcription of the HLSL — against
//! `goldens::host_ao`. Both sides were hand-written host models, and the shipped text was read for
//! ONE input (the tap count). A reviewer inverted the AO accumulation term `(h - d)` → `(d - h)` in
//! BOTH shipped copies and all ten tests in this file stayed GREEN: layer 2 was green because the
//! two shipped copies still matched each other, layer 5 was green because the consts were untouched,
//! and 3b was green because it never looked at either body. An oracle that certifies a model rather
//! than the artifact is this campaign's signature defect, and that was one more instance of it.
//!
//! [`SHIPPED_AO_BODY`] closes it: 3b now asserts the committed `sdf_ao` body is EXACTLY the text
//! [`shipped_ao_model`] transcribes, before comparing anything. See that constant's doc for why a
//! textual anchor rather than a host evaluator of the HLSL.
//!
//! # Why layer 2 and layer 5 are two tests and not one
//!
//! `sdf_ao` is NOT eDSL-generated (plan §4.1's corrected fact: there is no `emit_hlsl_sdf_ao`, and
//! R8 forecloses writing one), so the shared header carries a hand copy of the marcher's body.
//! Layer 2 pins the two BODIES. But the leaf's entire numeric behaviour lives in `AO_STEP` /
//! `AO_FALLOFF` / `AO_STRENGTH`, declared OUTSIDE the body in both files — change one in one file
//! and the two bodies stay byte-identical while computing different things. Layer 5 is the gate
//! for exactly that divergence, and the pairing (layer 5 red, layer 2 green) is the whole reason
//! it exists.
//!
//! # Why the numeric layers compare two HOSTS and not host-vs-device
//!
//! Plan Rev 4 (P0-E3) withdrew "on device": the `cpu_gpu_sdf_agreement` family is GPU-FREE by
//! design and no on-device leaf probe is ever dispatched, and building one means a new `.spv` plus
//! a manifest row — contradicting §5.1's zero-new-`.spv` invariant that §3.2's whole
//! runtime-gate-vs-`-D` arithmetic rests on. What IS constructible, and what these layers do, is
//! to close the chain on the host: the eDSL body is the SINGLE SOURCE the committed HLSL is
//! emitted from (pinned by `sdf_soft_shadow_ranged_matches_edsl_emit`), and
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
/// A brace COUNTER, not a first-`}` scan: both leaves this file pins carry a nested loop, so the
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
// LAYER 2 — the `sdf_ao` cross-file body pin
// ===============================================================================================

/// **S3 layer 2.** The shared header's `sdf_ao` is byte-identical to the marcher's.
///
/// # The defect this catches, and why it needs its own gate
///
/// `sdf_ao` has exactly TWO shipping definitions: `sdf_gbuffer_composite.hlsl` (the marcher, whose
/// `.spv` this campaign deliberately does not re-DXC) and `sdf_shadow_leaves.hlsli` (the copy the
/// three VB tails and the deferred resolve consume). There is NO generator to re-emit either from,
/// so nothing but this test stops the two from forking.
///
/// That makes it a materially WEAKER instrument than an eDSL re-emit pin — which proves the
/// committed text IS the generator's output, where this proves only that two hand copies agree.
/// The plan labels it so, and so does this doc, at the point of use.
#[test]
fn sdf_ao_body_matches_shared_header() {
    const SIG: &str = "float sdf_ao(float3 p, float3 n)";

    let marcher = extract_fn(&shader("sdf_gbuffer_composite.hlsl"), SIG);
    let header = extract_fn(&shader("sdf_shadow_leaves.hlsli"), SIG);

    assert_eq!(
        marcher, header,
        "`sdf_ao` FORKED between sdf_gbuffer_composite.hlsl (the marcher) and \
         sdf_shadow_leaves.hlsli (the shared VB-SV0 header). There is no generator to re-emit \
         either from (plan §4.1), so these two hand copies are held together by this test ALONE. \
         Re-sync them, and note which side moved: the marcher's `.spv` is frozen by \
         `marcher_spv_sync.rs`, so a marcher-side edit re-pins every marcher variant, while a \
         header-side edit re-pins the six deferred rows AND the ten VB lit-producer rows.\n\
         --- marcher ---\n{marcher}\n--- shared header ---\n{header}"
    );
}

// ===============================================================================================
// LAYER 5 — the tuning-const pin, across every file that redeclares one
// ===============================================================================================

/// The five sources that carry the SHADOW march tuning block. All five must agree, because all
/// five call a leaf whose numeric behaviour is entirely in these names.
const SHADOW_CONST_SOURCES: [&str; 5] = [
    "deferred_pbr.hlsl",
    "sdf_gbuffer_composite.hlsl",
    "vb_resolve.comp.hlsl",
    "vb_shade.comp.hlsl",
    "vb_shade_split.comp.hlsl",
];

/// The two sources that carry the AO tuning block: the marcher, and the shared header §4.1
/// duplicated them into while deliberately leaving the marcher untouched.
const AO_CONST_SOURCES: [&str; 2] = ["sdf_gbuffer_composite.hlsl", "sdf_shadow_leaves.hlsli"];

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
/// panic names the file it looked in — layer 5 walks a 46-row table and a red that does not say
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

/// **S3 layer 5** (plan Rev 4, promoted into S3's enumerated layers and widened to the AO consts).
///
/// Every file that redeclares a shadow-march or contact-AO tuning constant must fold it to the
/// SAME `f32`, and that `f32` must be the host mirror's.
///
/// # The defect this catches — and the one layer 2 structurally cannot
///
/// Layer 2 pins the `sdf_ao` BODY. The consts live outside it. Change `AO_FALLOFF` in the marcher
/// only and the two bodies stay byte-identical while computing different things: layer 2 stays
/// green, and every image gate in this campaign is on the DARK path where the leaf never runs.
/// This is the only instrument that reds for it — which is why the plan pairs the two.
///
/// Values are compared as FOLDED `f32`, not as text: `16.0 * GRAD_H` and `0.008` are the same
/// number and neither spelling is wrong, while `16.0 * GRAD_H` in a file whose `GRAD_H` differs is
/// a real divergence a text pin would call green.
#[test]
fn sv0_consts_match_deferred_and_marcher() {
    let field_hlsli = shader("sdf_field.hlsli");

    // (the host-side label, the shipped const's name, the host mirror it must equal, the files
    // that must declare it).
    //
    // `MAX_IT` appears TWICE, against two DIFFERENT host constants, and that is the point rather
    // than an oversight: `boyko_shaderdsl::shadow::MAX_IT` is the trip count layer 3a's eDSL side
    // executes, and `compute::SDF_TRACE_MAX_IT` (the public mirror of the private `SDF_MAX_IT`) is
    // the one its MIRROR side executes. Pinning both to the shipped `MAX_IT` is what makes them
    // pinned to each other.
    //
    // Why a PIN rather than making layer 3a's fixture sense it: MEASURED — doubling `SDF_MAX_IT`
    // to `256` leaves all ten tests in this file green, because the fixture's rays terminate by
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
                 {host_value} ({:#010x}). VB-SV0 §4.1: the shadow leaf lives in ONE shared header \
                 and the AO leaf in TWO hand copies, so every consumer must agree on the tuning \
                 block or the same body computes different things in different files — a \
                 divergence no body-identity pin and no DARK-path image golden can see.",
                value.to_bits(),
                host_value.to_bits()
            );
            checked += 1;
        }
    }

    // MEASURED SELECTION SIZE — 9 shadow rows × 5 sources + 3 AO consts × 2 sources. Asserted so
    // that quietly dropping a row from the table (the "gate stops covering things" failure) reds
    // instead of passing over a smaller set.
    assert_eq!(
        checked, 51,
        "the const × file selection changed shape: expected 9×5 shadow rows (MAX_IT is pinned \
         against BOTH host copies) + 3×2 AO = 51 checks"
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

/// The sample count plan S3 fixes for layer 3a (`>= 4096`). Shared with 3b so the two leaves are
/// judged over the same amount of evidence.
const ORACLE_SAMPLES: usize = 4096;

// No `#[inline]` on any of these. They are private to this test binary, so LTO already sees every
// body, and principle 7 makes inlining a MEASURED decision rather than a default — an unmeasured
// attribute on a debug-profile test helper is cargo cult either way.

fn v_dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn v_sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn v_mul(a: [f32; 3], s: f32) -> [f32; 3] {
    [a[0] * s, a[1] * s, a[2] * s]
}

fn v_cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

/// HLSL `normalize`, transcribed as written: `v * rsqrt(dot(v, v))` has no zero guard, and neither
/// does this. A guard here would model a shader that does not ship.
fn v_normalize(a: [f32; 3]) -> [f32; 3] {
    v_mul(a, 1.0 / v_dot(a, a).sqrt())
}

// ===============================================================================================
// LAYER 3a — the SHADOW leaf, bit-exact
// ===============================================================================================

/// **S3 layer 3a.** The eDSL body that GENERATES the shipped `sdf_soft_shadow_ranged` and
/// `goldens::host_soft_shadow_ranged` return BIT-IDENTICAL `f32` over a 4096-sample `(P, N, L)`
/// fixture.
///
/// # The defect this catches, and why nothing else can
///
/// `sdf_soft_shadow_ranged` in `sdf_shadow_leaves.hlsli` is emitted from
/// `boyko_shaderdsl::shadow::sdf_soft_shadow_ranged_body::<EmitCf>`, and layer 1's
/// `sdf_soft_shadow_ranged_matches_edsl_emit` pins the committed text to that emission. What no
/// text pin can see is the eDSL's OTHER instantiation — `<EvalCf>`, the host oracle — whose tuning
/// constants (`boyko_shaderdsl::shadow::SHADOW_K` and friends) are SEPARATE declarations from the
/// shader's, spelled symbolically in the emitted HLSL precisely so the emitted text does not move
/// when they do. Retune one and the emitted HLSL stays byte-identical, every span pin stays green,
/// and the host oracle silently begins modelling a different march than the GPU runs.
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
/// body is the loop+tail SPAN only, with the preamble owned by the caller — and every SV0 call
/// site gates on exactly this condition (`vb_resolve.comp.hlsl:436`'s `NoL > SHADOW_NDOTL_EPS`).
/// Comparing outside the region either side is ever called in would be comparing two functions
/// neither of which ships there.
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
fn sv0_shadow_leaf_matches_host_mirror_bit_exact() {
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
        "S3 layer 3a: {compared} (P,N,L) samples, ALL bit-identical and step-for-step identical \
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
/// # Why an ABSOLUTE bound and not the ULP bound plan Rev 4 names
///
/// Rev 4 specified "a pre-registered ULP tolerance". A ULP bound on this leaf's RESULT is not a
/// usable instrument, for an arithmetic reason rather than a matter of taste. What is being
/// tolerated is `AO_FALLOFF.powi(i)` (the host mirror) versus `pow(AO_FALLOFF, (float)i)` (what
/// `sdf_gbuffer_composite.hlsl:538` and the shared header spell) — a few ULP in each of five
/// weights, i.e. a bounded RELATIVE error in the accumulator `occ`. The leaf then returns
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
/// the plan cites as making bit-exactness "UNREACHABLE" does not exist *between these two host
/// models*. The genuinely unreachable comparison is host-`powi` versus the DEVICE's `pow`, and no
/// host test performs it. So the honest reading of this bound is: it is a PORTABILITY margin
/// against a libm whose `pow` differs, not the measurement of a gap that is present today. Do not
/// cite it as evidence that the shader and the mirror differ; they do not, at this instrument's
/// resolution.
const SV0_AO_LEAF_MAX_ABS_DEVIATION: f32 = 1.0e-6;

/// The signed-lexicographic ULP distance between two `f32`. Reported alongside the absolute
/// deviation so the ULP figure the plan asked for is on the record even though the GATE is
/// absolute (see [`SV0_AO_LEAF_MAX_ABS_DEVIATION`] for why).
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
/// shader: a reviewer inverted the accumulation term `(h - d)` → `(d - h)` in BOTH shipped copies
/// and every test in this file stayed green (layer 2 because the two copies still matched each
/// other, layer 5 because the consts were untouched, 3b because it read the body for nothing but
/// the tap count). This is the same pin layer 4 already puts under [`FACE_NORMAL_BODY`], for the
/// same reason.
///
/// # Why a TEXTUAL anchor rather than evaluating the shipped body
///
/// The stronger-sounding option is to parse this body and interpret it — layer 2 already locates
/// the span, so the extraction exists. What does not exist is an evaluator: interpreting
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

/// The AO tap count read OUT OF THE COMMITTED SHARED HEADER, not hardcoded here.
///
/// Strictly redundant now that [`SHIPPED_AO_BODY`] pins the whole body — a tap-count change reds
/// there too. It is kept because it is the SHARPEST red for the single most likely edit (the loop
/// bound), and because it keeps [`shipped_ao_model`] parameterized by the shipped text rather than
/// by a literal, so the two reds name the same cause instead of one of them being a puzzle.
fn shared_header_ao_tap_count(header: &str) -> u32 {
    let body = extract_fn(header, "float sdf_ao(float3 p, float3 n)");
    const MARKER: &str = "for (uint i = 1u; i <= ";
    let start = body.find(MARKER).unwrap_or_else(|| {
        panic!(
            "sdf_shadow_leaves.hlsli's `sdf_ao` no longer opens its accumulation with \
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
/// [`shared_header_ao_tap_count`].
///
/// The transcription is only worth anything because [`SHIPPED_AO_BODY`] is asserted against the
/// committed header before this function is called. Do not edit one without the other.
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

/// **S3 layer 3b — the WEAKER instrument of the pair. Read this before citing it.**
///
/// Layer 3a is bit-exact against an oracle that GENERATES the shipped text. This one cannot be.
/// There is no `sdf_ao` body in `boyko_shaderdsl` (plan §4.1's corrected fact; R8 forecloses
/// writing one), so BOTH sides here are hand transcriptions, and they are separated by a
/// PLATFORM-DEPENDENT TRANSCENDENTAL — the host mirror computes `AO_FALLOFF.powi(i)` where the
/// HLSL computes `pow(AO_FALLOFF, (float)i)`. Bit-exactness against that is UNREACHABLE, not
/// merely untested, which is why this layer has a tolerance and 3a does not. The AO half's
/// strongest correctness evidence remains S4(iv), the owner's visual eval — not this test.
///
/// # The defect it DOES catch
///
/// The shipped header's AO accumulation and `goldens::host_ao` — the mirror the S1 adequacy oracle
/// re-derives and the marcher goldens hold against the GPU — computing different things. The model
/// side is ANCHORED to the shipped text by [`SHIPPED_AO_BODY`], asserted first, so the mutation
/// that reds it is an edit to the SHIPPED shader and not only an edit to this test's own model.
#[test]
fn sv0_ao_leaf_agrees_with_host_mirror() {
    let header = shader("sdf_shadow_leaves.hlsli");

    // FIRST — and this is the whole load-bearing change to this layer: the model below is a
    // transcription of a SPECIFIC piece of shipped text, so that text is pinned before it is
    // trusted. Without this the layer compares two hand models to each other and a term inverted
    // in BOTH shipped copies passes.
    let shipped_body = extract_fn(&header, "float sdf_ao(float3 p, float3 n)");
    assert_eq!(
        shipped_body, SHIPPED_AO_BODY,
        "the committed `sdf_ao` in sdf_shadow_leaves.hlsli is no longer the body \
         `shipped_ao_model` (tests/vb_sv0_leaf_oracle.rs) transcribes. Layer 3b's numeric \
         comparison below runs the RUST model, so an unpinned edit here would leave it green while \
         the shipped leaf changed — and layer 2 would not catch it either, because layer 2 pins \
         the two SHIPPED copies to EACH OTHER and an edit applied to both keeps them equal. Update \
         `SHIPPED_AO_BODY` and `shipped_ao_model` together with the shader, or state why the model \
         may lag.\n--- expected (the model's source) ---\n{SHIPPED_AO_BODY}\n\
         --- committed ---\n{shipped_body}"
    );

    let taps = shared_header_ao_tap_count(&header);

    let edits = oracle_edits();
    let field = |q: [f32; 3]| sdf_edit_list(&edits, q);

    let mut state = 0xA0_5EED_u64;
    let mut worst_abs = 0.0f32;
    let mut worst_ulp = 0u64;
    let mut worst_at = String::new();
    let mut darkened = 0usize;
    // Tracked SEPARATELY from `worst_abs`, because a `max` over deviations is NaN-BLIND: `NaN >
    // worst_abs` is false, so a sample where one side is NaN and the other is a number would leave
    // `worst_abs` untouched and the gate green. That is the same class of mistake this campaign's
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
    // vacuous — exactly the empty-edit-list trap plan §4.4 names.
    assert!(
        darkened > 0,
        "the layer-3b fixture is VACUOUS: 0 of {ORACLE_SAMPLES} samples darken, so both models \
         return the far-field `1.0` everywhere and agreement means nothing"
    );

    assert!(
        worst_abs <= SV0_AO_LEAF_MAX_ABS_DEVIATION,
        "the shipped `sdf_ao` model ({taps} taps, read from sdf_shadow_leaves.hlsli) diverged from \
         `goldens::host_ao` by {worst_abs} — above the PRE-REGISTERED \
         {SV0_AO_LEAF_MAX_ABS_DEVIATION}. Do NOT widen the bound to make this pass: it is four \
         orders tighter than the ±3/255 the mirror concedes against the GPU, so anything reaching \
         it is a real divergence (a tap count, a tuning const, or a term). worst: {worst_at}"
    );

    eprintln!(
        "S3 layer 3b (WEAKER instrument — powi-vs-pow, tolerance not bit-exactness; model ANCHORED \
         to the committed body): {taps} taps, {ORACLE_SAMPLES} samples, {darkened} darkened, \
         {non_finite} non-finite; observed max |deviation| = {worst_abs:e} ({worst_ulp} ULP) \
         vs pre-registered {SV0_AO_LEAF_MAX_ABS_DEVIATION:e}"
    );
}

// ===============================================================================================
// LAYER 4 — the geometric face normal
// ===============================================================================================

/// The degenerate-triangle floor, mirrored from `vb_geom_fetch.hlsli`. Pinned to the shipped value
/// by [`vb_sv0_face_normal_body_matches_host_mirror`].
const FACE_N_EPS2: f32 = 1.0e-20;

/// The COMMITTED body of `vb_sv0_face_normal`, quoted verbatim.
///
/// This is what binds the Rust mirror below to the shader. Without it layer 4 would test the
/// properties of a model nothing ships — the "gate that cannot go red for the failure it exists to
/// catch" in its purest form: delete `isfinite` from the HLSL and every property test would stay
/// green because they exercise the Rust copy.
const FACE_NORMAL_BODY: &str = "\
float3 vb_sv0_face_normal(VbGeomFetchResult geo) {
    float3 fn = cross(geo.tri_p1 - geo.tri_p0, geo.tri_p2 - geo.tri_p0);
    float l2 = dot(fn, fn);
    bool plane_n_usable = isfinite(l2) && l2 > FACE_N_EPS2;
    float3 face_n = plane_n_usable ? (fn * rsqrt(l2)) : normalize(geo.world_normal);
    return (dot(face_n, geo.world_normal) < 0.0) ? -face_n : face_n;
}";

/// The host mirror of `vb_sv0_face_normal` — statement for statement, in the shipped order.
///
/// `rsqrt` is modelled as `1.0 / sqrt(l2)`. HLSL's `rsqrt` is permitted a relaxed ULP budget, so
/// this mirror is NOT a bit-exactness oracle and layer 4 makes no bit-exactness claim: what it
/// tests is STRUCTURE — which branch is taken, what the fallback is, and the orientation — all of
/// which are unaffected by the last bit of a reciprocal square root.
fn host_vb_sv0_face_normal(tri: [[f32; 3]; 3], world_normal: [f32; 3]) -> [f32; 3] {
    let face = v_cross(v_sub(tri[1], tri[0]), v_sub(tri[2], tri[0]));
    let l2 = v_dot(face, face);
    let plane_n_usable = l2.is_finite() && l2 > FACE_N_EPS2;
    let face_n = if plane_n_usable {
        v_mul(face, 1.0 / l2.sqrt())
    } else {
        v_normalize(world_normal)
    };
    if v_dot(face_n, world_normal) < 0.0 {
        v_mul(face_n, -1.0)
    } else {
        face_n
    }
}

/// **S3 layer 4, the fidelity half.** The committed `vb_sv0_face_normal` is textually what
/// [`host_vb_sv0_face_normal`] mirrors, and `FACE_N_EPS2` folds to the mirror's value.
///
/// # The defect this catches
///
/// The shipped leaf being edited — losing the `isfinite` guard, the orientation flip, or the
/// fallback — while the property tests below keep passing because they exercise the Rust copy.
/// Every transcription-based oracle in this repo needs this pin or it silently stops describing
/// the artifact.
#[test]
fn vb_sv0_face_normal_body_matches_host_mirror() {
    let fetch = shader("vb_geom_fetch.hlsli");

    assert!(
        fetch.contains(FACE_NORMAL_BODY),
        "`vb_sv0_face_normal` in vb_geom_fetch.hlsli no longer matches the body \
         `host_vb_sv0_face_normal` (tests/vb_sv0_leaf_oracle.rs) mirrors. The layer-4 property \
         tests exercise the MIRROR, so an unpinned edit here would leave them green while the \
         shipped leaf changed. Update both together, or state why the mirror may lag.\n\
         --- expected (the mirror's model) ---\n{FACE_NORMAL_BODY}"
    );

    let shipped_eps = eval_hlsl_const(
        "FACE_N_EPS2",
        "vb_geom_fetch.hlsli",
        &fetch,
        &shader("sdf_field.hlsli"),
        0,
    );
    assert_eq!(
        shipped_eps.to_bits(),
        FACE_N_EPS2.to_bits(),
        "`FACE_N_EPS2` is {shipped_eps} in vb_geom_fetch.hlsli but {FACE_N_EPS2} in the host \
         mirror — the degenerate-triangle floor decides WHICH BRANCH the leaf takes, so the two \
         must fold to the same number"
    );
}

/// **S3 layer 4.** Under a NON-UNIFORM instance scale the leaf returns the TRUE plane normal —
/// which is exactly what the interpolated shading normal is not.
///
/// This is §4.2's whole justification made executable: `world_normal` is `mul(m3, n)` with no
/// inverse-transpose correction (`vb_geom_fetch.hlsli:539-542` documents the limitation), so under
/// anisotropic scale it leaves the plane. The march origin must be lifted along the real plane
/// normal or the bias points partly along the surface and the acne it exists to remove comes back.
/// The second assertion is the one that makes the first mean something: it shows the two normals
/// genuinely differ on this input, so agreement is not trivially satisfied.
#[test]
fn vb_sv0_face_normal_is_the_true_plane_normal_under_non_uniform_scale() {
    // A triangle in the z = 0 plane, so its untransformed plane normal is +Z.
    let base = [[0.0f32, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
    // Its vertex normals all point along the plane normal — a flat facet.
    let base_normal = [0.0f32, 0.0, 1.0];

    // A non-uniform affine: diag(3, 0.25, 1) plus a shear that tilts the facet out of z = 0.
    let m3 = |v: [f32; 3]| -> [f32; 3] {
        [
            3.0 * v[0] + 0.4 * v[2],
            0.25 * v[1],
            0.7 * v[0] + 1.0 * v[2],
        ]
    };
    let tri = [m3(base[0]), m3(base[1]), m3(base[2])];
    // The SHIPPED (uncorrected) shading-normal transform: plain m3, no inverse transpose.
    let world_normal = v_normalize(m3(base_normal));

    // The analytic plane normal of the transformed triangle, oriented like the shading normal so
    // the comparison is against the same representative the leaf must return.
    let analytic = {
        let raw = v_normalize(v_cross(v_sub(tri[1], tri[0]), v_sub(tri[2], tri[0])));
        if v_dot(raw, world_normal) < 0.0 {
            v_mul(raw, -1.0)
        } else {
            raw
        }
    };

    let got = host_vb_sv0_face_normal(tri, world_normal);
    let err = v_dot(v_sub(got, analytic), v_sub(got, analytic)).sqrt();
    assert!(
        err < 1.0e-5,
        "vb_sv0_face_normal returned {got:?} under a non-uniform affine, but the true plane \
         normal is {analytic:?} (error {err})"
    );

    // And the interpolated normal is NOT that plane normal — which is why the leaf exists.
    let shading_gap = v_dot(world_normal, analytic);
    assert!(
        shading_gap < 0.99,
        "this fixture's non-uniform affine failed to separate the plain-m3 shading normal from \
         the plane normal (cos = {shading_gap}); the first assertion would then be satisfied by \
         either construction and would prove nothing"
    );
}

/// **S3 layer 4.** The leaf is WINDING-INDEPENDENT: reversing the triangle's vertex order returns
/// the same normal, because the orientation flip re-agrees it with the shading normal.
///
/// Without that flip a clockwise-wound triangle would bias the shadow-march origin INTO the
/// surface — a lift in exactly the wrong direction, i.e. self-shadow acne on precisely the
/// triangles whose winding happens to be reversed.
#[test]
fn vb_sv0_face_normal_is_winding_independent() {
    let ccw = [[0.0f32, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
    let cw = [ccw[0], ccw[2], ccw[1]];
    let world_normal = [0.0f32, 0.0, 1.0];

    let a = host_vb_sv0_face_normal(ccw, world_normal);
    let b = host_vb_sv0_face_normal(cw, world_normal);

    assert_eq!(
        a, b,
        "reversing the winding changed the face normal ({a:?} vs {b:?}) — the orientation flip \
         `(dot(face_n, world_normal) < 0) ? -face_n : face_n` is what makes the lift direction a \
         property of the SURFACE rather than of the index order"
    );
    assert!(
        v_dot(a, world_normal) > 0.0,
        "the face normal must lift AWAY from the surface, but {a:?} points against the shading \
         normal {world_normal:?}"
    );
}

/// **S3 layer 4, plan Rev 6's RESTATED assertion.** A degenerate (zero-area) triangle takes the
/// FALLBACK, so the march origin stays finite and the leaf is never reached with a NaN.
///
/// # Rev 6's correction, and why the property had to be restated
///
/// Rev 5 asserted that a NaN term "degrades to no SV0 contribution" because `NMin` returns the
/// non-NaN operand. That reasoning is not safe to rely on and this test does not rely on it — see
/// [`nan_is_inert_in_the_shadow_leaf_but_turns_the_ao_leaf_black`], which measures both leaves and
/// shows the inversion is real but lives in the AO leaf. The property that actually protects the
/// frame is the one asserted here: the NaN never gets that far.
#[test]
fn vb_sv0_face_normal_falls_back_on_a_degenerate_triangle() {
    let world_normal = [0.3f32, 0.6, 0.74];
    // The shipped fallback is `normalize(geo.world_normal)`, so the expectation is the NORMALIZED
    // shading normal — re-normalizing an already-unit vector moves it by ~1 ULP, and comparing
    // against the un-normalized input would fail for a reason that has nothing to do with the
    // branch under test.
    let fallback = v_normalize(world_normal);
    // Zero area: all three corners coincident, and a collinear pair, both of which give `l2 == 0`.
    let coincident = [[1.0f32, 2.0, 3.0]; 3];
    let collinear = [[0.0f32, 0.0, 0.0], [1.0, 1.0, 1.0], [2.0, 2.0, 2.0]];

    for (label, tri) in [("coincident", coincident), ("collinear", collinear)] {
        let got = host_vb_sv0_face_normal(tri, world_normal);
        assert!(
            got.iter().all(|c| c.is_finite()),
            "a {label} degenerate triangle produced a NON-FINITE face normal {got:?} — \
             `rsqrt(0)` is `+inf` and `fn * inf` is NaN, so the `l2 > FACE_N_EPS2` floor is what \
             must route this to the fallback"
        );
        assert_eq!(
            got, fallback,
            "a {label} degenerate triangle must fall back to `normalize(world_normal)`, got \
             {got:?}"
        );

        // The property Rev 6 actually mandates: the SHADOW LEAF never sees a NaN origin.
        let p = [0.25f32, -0.5, 0.75];
        let origin = [
            p[0] + got[0] * SHADOW_NORMAL_BIAS,
            p[1] + got[1] * SHADOW_NORMAL_BIAS,
            p[2] + got[2] * SHADOW_NORMAL_BIAS,
        ];
        assert!(
            origin.iter().all(|c| c.is_finite()),
            "the march origin built from a {label} degenerate triangle is {origin:?} — a \
             non-finite origin is precisely what the fallback exists to prevent"
        );
    }
}

/// **S3 layer 4.** A NON-FINITE triangle also takes the fallback — and the branch that would have
/// produced the NaN is the TAKEN one, not the fallback.
///
/// `l2 == +inf` satisfies a bare `l2 > FACE_N_EPS2` floor, and then `rsqrt(+inf)` is `0` while
/// `fn` is infinite, so `fn * 0` is `inf * 0` = NaN. The direction is counter-intuitive and it is
/// why the shipped test is `isfinite(l2) && l2 > FACE_N_EPS2` rather than a magnitude floor alone.
/// `l2 == NaN` needs no clause of its own: every ordered comparison against NaN is false.
#[test]
fn vb_sv0_face_normal_falls_back_on_a_non_finite_triangle() {
    let world_normal = [0.0f32, 1.0, 1.0];
    // As in the degenerate case: the shipped fallback re-normalizes, so that is the expectation.
    let fallback = v_normalize(world_normal);
    let huge = 1.0e30f32; // squares to +inf in f32

    let cases: [(&str, [[f32; 3]; 3]); 3] = [
        (
            "overflowing cross product",
            [[0.0, 0.0, 0.0], [huge, 0.0, 0.0], [0.0, huge, 0.0]],
        ),
        (
            "infinite vertex",
            [[0.0, 0.0, 0.0], [f32::INFINITY, 0.0, 0.0], [0.0, 1.0, 0.0]],
        ),
        (
            "NaN vertex",
            [[0.0, 0.0, 0.0], [f32::NAN, 0.0, 0.0], [0.0, 1.0, 0.0]],
        ),
    ];

    for (label, tri) in cases {
        let got = host_vb_sv0_face_normal(tri, world_normal);
        assert_eq!(
            got, fallback,
            "a {label} must take the FALLBACK, got {got:?}. Note which branch is dangerous here: \
             for `l2 == +inf` the magnitude floor is SATISFIED, so a bare `l2 > FACE_N_EPS2` lets \
             the taken branch compute `inf * rsqrt(inf)` = `inf * 0` = NaN. `isfinite(l2)` in \
             front is what excludes it."
        );
    }
}

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

/// **S3 layer 4 — the executable record of plan Rev 6's NaN correction, itself corrected.**
///
/// # What Rev 5 said, what Rev 6 said, and what is actually true
///
/// Rev 5 wrote that a NaN term "degrades to no SV0 contribution". Rev 6 (§4.5) withdrew that and
/// replaced it with: the leaf's `clamp(res, 0, 1)` tail lowers to `NMin(NMax(NaN, 0), 1)` = `0`,
/// so a NaN turns the pixel BLACK. **Rev 6 named the wrong leaf.** MEASURED, and asserted below:
///
/// * The SHADOW leaf's accumulator can never BE NaN, so its `clamp` tail never sees one.
///   `res = min(res, SHADOW_K * d / t)` drops a NaN `d` under BOTH `f32::min` and `NMin`, and
///   `t = t + max(d / L, SHADOW_MINT_STEP)` drops it again, so the march advances normally and the
///   leaf returns `1.0` — fully LIT — on the host AND through the HLSL lowering. On this leaf the
///   two answers do NOT differ, and "a NaN is inert" happens to be true.
/// * The AO leaf is where the property Rev 6 describes actually holds. `occ += (h - d) * ...` is a
///   PLAIN ADDITION with no `min`/`max` to launder anything, so one NaN tap makes `occ` NaN, the
///   NaN reaches `clamp(1 - AO_STRENGTH * occ, 0, 1)`, and there the two hosts invert: Rust's
///   `f32::clamp` returns NaN, the shipped `clamp` returns **`0.0`** — full occlusion, a BLACK
///   pixel.
///
/// So the binding conclusion of §4.5 survives and its demonstration moves: nothing downstream may
/// lean on either leaf being inert, and §4.2's finiteness guard in the face-normal leaf is the
/// only thing that is load-bearing. What changes is WHICH leaf a reader should be shown, and that
/// the shadow leaf must not be described as producing black pixels — it does not.
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
         `clamp` tail never sees a NaN, and therefore why Rev 6's `a NaN comes out black` does \
         NOT describe this leaf"
    );
    assert_eq!(
        nmax(f32::NAN, SHADOW_MINT_STEP),
        SHADOW_MINT_STEP,
        "invariant: `t = t + max(NaN, SHADOW_MINT_STEP)` under NMax advances by the floor, so the \
         march terminates instead of stalling (plan §4.4's one DELIBERATE use of these semantics)"
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
         `0.0` — FULL occlusion, a BLACK pixel, where the host says NaN. THIS is the leaf plan \
         §4.5 describes; the shadow leaf above is not."
    );

    eprintln!(
        "S3 layer 4 (NaN): an all-NaN field yields shadow={shadow_host} on BOTH hosts (inert, \
         fully lit) but AO={ao_host} on the host vs 0.0 (BLACK) through the shipped `clamp` \
         lowering. The inversion lives in the AO leaf, not the shadow leaf — which is why the \
         face-normal fallback, not either tail, is the load-bearing guard."
    );
}

/// **S3 layer 4 — a CONFIRMED, UNGUARDED residual, recorded executably rather than asserted away.**
///
/// The shipped fallback is `normalize(geo.world_normal)` and HLSL's `normalize` has no zero guard:
/// `v * rsqrt(dot(v, v))` on a zero vector is `0 * +inf` = NaN in every lane. Take the fallback
/// (a degenerate triangle) with a zero `world_normal` and the face normal — and therefore the
/// shadow-march origin built from it — is NaN.
///
/// # Is a zero `world_normal` reachable?
///
/// It is not excluded anywhere. `VbGeomFetchResult::world_normal` is the perspective-correct
/// interpolation of three `mul(m3, n)` vertex normals (`vb_geom_fetch.hlsli:669-671`) and is
/// neither normalized nor guarded at that point, so a zero arrives from a zero source normal, a
/// rank-deficient `m3` (a zero scale on an axis), or normals that cancel across a fold. It needs
/// the degenerate-triangle branch at the same time, which is why this is a narrow residual rather
/// than a live bug — but "narrow" is not "impossible", and per the NaN test above the consequence
/// is asymmetric: the shadow term stays inert at `1.0`, while the AO term goes to `0.0`, a BLACK
/// pixel.
///
/// This test asserts the CURRENT behaviour, so adding a guard to the shipped shader reds here and
/// forces the residual's record to be updated rather than silently outliving the fix.
#[test]
fn vb_sv0_face_normal_fallback_is_unguarded_against_a_zero_shading_normal() {
    // Zero area, so the `isfinite(l2) && l2 > FACE_N_EPS2` test routes to the fallback...
    let degenerate = [[1.0f32, 2.0, 3.0]; 3];
    // ...and the fallback's own input is zero, which `normalize` does not defend against.
    let got = host_vb_sv0_face_normal(degenerate, [0.0, 0.0, 0.0]);

    assert!(
        got.iter().all(|c| c.is_nan()),
        "the shipped fallback `normalize(geo.world_normal)` is expected to yield NaN for a ZERO \
         shading normal (`0 * rsqrt(0)` = `0 * +inf`), but this returned {got:?}. If the shader \
         gained a zero guard, that is good news and this test must be rewritten to pin the guard \
         — do not delete it, or the residual's record goes with it."
    );

    // The consequence, spelled out where a reader will meet it: the march origin is NaN too.
    let p = [0.25f32, -0.5, 0.75];
    let origin = [
        p[0] + got[0] * SHADOW_NORMAL_BIAS,
        p[1] + got[1] * SHADOW_NORMAL_BIAS,
        p[2] + got[2] * SHADOW_NORMAL_BIAS,
    ];
    assert!(
        origin.iter().all(|c| c.is_nan()),
        "a NaN face normal must carry into the march origin ({origin:?}) — this is the ONE input \
         shape for which §4.2's finiteness guard does not deliver a finite origin"
    );

    eprintln!(
        "S3 layer 4: CONFIRMED RESIDUAL — degenerate triangle + ZERO world_normal takes the \
         `normalize(geo.world_normal)` fallback, which has no zero guard, so face_n = {got:?} and \
         the march origin = {origin:?}. Per the NaN test, the shadow term then stays inert (1.0) \
         while the AO term clamps to 0.0 (BLACK)."
    );
}
