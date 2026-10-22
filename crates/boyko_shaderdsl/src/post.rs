//! Render wave PXe — the post eDSL module (AD-3, `plan/H2-PXe-post-edsl.md` rev 5): every curve and
//! boundary leaf the tail tonemap needs, the traced main body [`post_final_main`] that composes
//! them, and the three single-source tables the generator and the host oracle read
//! ([`POST_FINAL_BINDINGS`], [`POST_FINAL_STAGE_ORDER`], [`POST_FINAL_UBO_LAYOUT`]).
//!
//! # One body, two instantiations
//!
//! - `<EvalCf>` — the host oracle. Its fetches are real reads of decoded images
//!   ([`HostTex2`](crate::cf::HostTex2)), so PX0's `post_eval` evaluates THIS function over a
//!   frame's dumps; it replaces the hand mirror `goldens.rs::tonemap_and_oetf` retires.
//! - `<EmitCf>` — the HLSL recorder. `crate::emit::emit_post_final_comp` prints the traced body
//!   into the committed `boyko_rhi_vulkan/shaders/post_final.comp.hlsl`, whose hand-written skeleton
//!   is only the declarations (printed from the tables), `numthreads`, the extent guard, one call
//!   and the store.
//!
//! The body never asks which backend it runs on: the same monomorphised Rust runs on both, and a
//! census (`boyko_rhi_vulkan/tests/post_edsl_sync.rs`) keeps this file free of backend
//! introspection. So what the Emit trace shows about the composition holds for the Eval path.
//!
//! # The stage trace
//!
//! Every stage of [`post_final_main`] passes its result through [`stage3`], which materialises it
//! as a named `float3 post_stage_<stage>` local in program order. The emitted body therefore spells
//! the composition as a chain of declarations, each consumed by the next — the observable the
//! stage-order gate reads against [`POST_FINAL_STAGE_ORDER`], which is written separately. A rung
//! that adds a stage adds a [`PostStage`] variant, its list row and one `stage3` call (and, for a
//! stage that fetches or wraps space, its f64 stage in `boyko_shaderdsl/tests/post_image_oracle.rs`).
//!
//! # Authoring rules (each one a past or predicted defect)
//!
//! - **Leaves are return-free value functions** (`fn leaf<C: Cf>(…) -> C::Vec3f`). A traced leaf
//!   INLINES into the body, so a `ret*` inside it would return from the whole body. Data-dependent
//!   arms (Neutral's `peak < 0.76` early-out, its `x < 0.08` toe) are selects; the tonemapper
//!   dispatch is the only branch. Only [`post_final_main`] returns.
//! - **Per-channel scalar code, scalar selects.** `Cf` has no `float3` select, lerp or
//!   component-wise product, so every leaf works channel by channel and rebuilds the `float3` with
//!   `vec3_from_scalars`. The compiled module therefore carries no `float3` `OpSelect` at all,
//!   which is what lets the dispatch census say "the curves are not if-converted" (D-P8).
//! - **Temps inside a leaf are anonymous** (`Cf::temp`, printed `t<seq>` with a body-wide
//!   sequence), so a leaf may be inlined several times (PX1b's five sanitized taps) without
//!   redeclaring a name. Named locals are reserved to the body-level singletons, all prefixed
//!   `post_`: the stage markers `post_stage_*`, the scene texel `post_scene`, the tonemapper word
//!   `post_tonemap_mode` and the dispatch result `post_tonemapped` (so [`tonemap_select`] is called
//!   once per body).
//! - **The tonemapper dispatch is a real branch** (`if_else_branch`, printed `[branch]`): the mode
//!   word is a UBO word, wave-uniform, so a branch costs no divergence, while an if-converted
//!   dispatch would evaluate every curve per pixel.
//! - **One scene fetch**: the texel is materialised once (`post_scene`) — an un-materialised fetch
//!   would be printed at every use.
//!
//! # Zero cost when off
//!
//! Not feature-gated: the host oracle and PX0's tool instantiate it without `emit`. Generic bodies
//! emit no object code until instantiated, and nothing physics-reachable names this module.

use core::cell::Cell;

use crate::cf::{Cf, EvalCf, Flow, HostTex2, HostTex3};
use crate::scalar::FieldScalar;

// ---- Constants --------------------------------------------------------------------------------

/// The largest finite value of `B10G11R11`'s 10-bit channel (`2¹⁵·(1 + 31/32)`), the sanitize
/// clamp. Exactly representable in binary16 too, so it stays a finite-safe clamp whichever `lit`
/// storage format PX0 settles.
pub const HDR_MAX: f32 = 64512.0;

/// The one string every generator emits into `[[vk::image_format(…)]]` on an HDR storage write.
/// RGBA16F: PX0's lit-format decision took AD-1's escape (row 2 of
/// `docs/measurements/2026-10-16-px0-lit-format-decision.md` §9, from the device arms). Nothing
/// emits it before PX1a, so the change moves no shader.
pub const LIT_STORAGE_FORMAT: &str = "rgba16f";

/// The display storage image's format (`gDisplay`, the post_final output).
pub const DISPLAY_STORAGE_FORMAT: &str = "rgba8";

/// The manual OETF exponent: `1/2.2` rounded ONCE to f32 (`0x3EE8BA2F`), the shader's
/// `OETF_GAMMA_EXP`. It is the value DXC folds the shipped producers' `1.0 / 2.2` to (measured,
/// Vulkan SDK 1.4.350: `OpConstant %float 0.454545468`), so `post_final` keeps today's device
/// exponent; Rust's f32 QUOTIENT `1.0 / 2.2` (two roundings, the `goldens.rs` mirror's spelling) is
/// one ULP below it.
pub const OETF_GAMMA_EXP: f32 = 0.454_545_47;

/// The tonemapper field of the post mode word: bits `TONEMAP_MODE_SHIFT..+4` — the same field the
/// light header's word 7 carries (`boyko_render::light::TONEMAP_MODE_SHIFT`, pinned by a test).
pub const TONEMAP_MODE_SHIFT: u32 = 8;
/// The tonemapper field's mask (4 bits, 16 operators).
pub const TONEMAP_MODE_MASK: u32 = 0xF;
/// Tonemapper word: Stephen Hill's ACES fit (the default; any unknown word falls back to it).
pub const TONEMAP_ACES: u32 = 0;
/// Tonemapper word: Khronos PBR Neutral.
pub const TONEMAP_NEUTRAL: u32 = 1;
/// Tonemapper word: Reinhard-Jodie.
pub const TONEMAP_REINHARD_JODIE: u32 = 2;

/// Mode-word bit 1 (render wave PX1b, H5): the RCAS sharpen stage runs this frame. Bit 0 stays
/// reserved (H2 rev 2 removed the sanitize bit); bits `TONEMAP_MODE_SHIFT..+4` are the tonemapper.
pub const POST_MODE_SHARPEN: u32 = 1 << 1;

/// FSR 1's `FSR_RCAS_LIMIT` (`ffx_fsr1.h`): the lobe never sharpens past `−(0.25 − 1/16)`, which
/// keeps `4·lobe + 1 ≥ 0.25` (the resolve's denominator) away from zero.
pub const RCAS_LIMIT: f32 = 0.25 - 1.0 / 16.0;

/// FSR 2's `FSR2_TONEMAP_EPSILON` (`ffx_fsr2_common.h`, `1 / FP16_MAX`): the floor of
/// [`tonemap_reversible_inverse`]'s `1 − max3(y)`, so a sharpened value at or above 1 inverts to a
/// finite `≤ 65504·y` instead of dividing by zero.
pub const RCAS_INV_EPS: f32 = 1.0 / 65504.0;

/// The f32 exponent-field mask: a value is finite iff `(asuint(x) & F32_EXP_MASK) != F32_EXP_MASK`.
/// An integer test on the bit pattern, so a NotNaN / NotInf-assuming compiler cannot fold it away
/// (as it may `x == x`).
pub const F32_EXP_MASK: u32 = 0x7F80_0000;

/// The Hill ACES fit's input matrix (sRGB → the fit's space), row-major — the resolve's `ACES_IN`.
pub const ACES_IN: [[f32; 3]; 3] = [
    [0.59719, 0.35458, 0.04823],
    [0.07600, 0.90834, 0.01566],
    [0.02840, 0.13383, 0.83777],
];
/// The Hill ACES fit's output matrix, row-major — the resolve's `ACES_OUT`.
pub const ACES_OUT: [[f32; 3]; 3] = [
    [1.60475, -0.53108, -0.07367],
    [-0.10208, 1.10813, -0.00605],
    [-0.00327, -0.07276, 1.07602],
];
/// The fit's rational curve `(v(v + A) − B) / (v(C·v + D) + E)` — its five coefficients.
const ACES_A: f32 = 0.0245786;
const ACES_B: f32 = 0.000090537;
const ACES_C: f32 = 0.983729;
const ACES_D: f32 = 0.432951;
const ACES_E: f32 = 0.238081;

/// Khronos PBR Neutral: the compression start `0.8 − 0.04` (F90 subtracted), computed in f32 like
/// the shipped shader's `const float startCompression = 0.8 - 0.04;`.
const NEUTRAL_START: f32 = 0.8 - 0.04;
/// Khronos PBR Neutral: the desaturation speed.
const NEUTRAL_DESATURATION: f32 = 0.15;
/// `d = 1 − startCompression` (f32, like the shader's `const float d`).
const NEUTRAL_D: f32 = 1.0 - NEUTRAL_START;
/// `d·d`, folded once in f32 so host and device read one constant.
const NEUTRAL_D2: f32 = NEUTRAL_D * NEUTRAL_D;
/// The toe: below `x < 0.08` the offset is `x − 6.25·x²`, else the F90 `0.04`.
const NEUTRAL_TOE: f32 = 0.08;
const NEUTRAL_TOE_K: f32 = 6.25;
const NEUTRAL_F90: f32 = 0.04;

/// Reinhard-Jodie's Rec.709 luma weights.
const REC709_LUMA: [f32; 3] = [0.2126, 0.7152, 0.0722];

/// The sRGB EOTF's knee and its two branches' constants.
const SRGB_EOTF_KNEE: f32 = 0.04045;
const SRGB_LINEAR_DIV: f32 = 12.92;
const SRGB_OFFSET: f32 = 0.055;
const SRGB_SCALE: f32 = 1.055;
const SRGB_GAMMA: f32 = 2.4;

// ---- The stage trace ----------------------------------------------------------------------------

/// One stage of [`post_final_main`], in the documented order of POSTFX §5. A later rung adds its
/// variant (RCAS, bloom, LUT, dither / grain / vignette), its [`POST_FINAL_STAGE_ORDER`] row and
/// one [`stage3`] call.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PostStage {
    /// The whole-pixel non-finite kill plus the `[0, HDR_MAX]` clamp ([`hdr_sanitize`]).
    HdrSanitize,
    /// The exposure ratio `E / pre` ([`pre_ratio_apply`]; exactly 1.0 until PX3).
    PreRatio,
    /// The tonemap curve the mode word selects ([`tonemap_select`]).
    Tonemap,
    /// The manual gamma-2.2 OETF ([`oetf_gamma22`]).
    Oetf,
}

impl PostStage {
    /// The reserved name of the stage's materialised `float3` local — the label the stage-order
    /// gate reads from the emitted body.
    pub const fn label(self) -> &'static str {
        match self {
            PostStage::HdrSanitize => "post_stage_hdr_sanitize",
            PostStage::PreRatio => "post_stage_pre_ratio",
            PostStage::Tonemap => "post_stage_tonemap",
            PostStage::Oetf => "post_stage_oetf",
        }
    }
}

/// The documented stage order (POSTFX §5: `hdr_sanitize` → `pre_ratio_apply` → RCAS (PX1b) → bloom
/// composite (PX4) → `tonemap_select` → LUT (PX5) → `oetf_gamma22` → dither / grain / vignette
/// (PX6)), stated independently of the body; the stage-order gate holds the two to each other.
pub const POST_FINAL_STAGE_ORDER: &[PostStage] =
    &[PostStage::HdrSanitize, PostStage::PreRatio, PostStage::Tonemap, PostStage::Oetf];

/// Passes a stage's result through its marker: a named `float3 <label>` local on Emit, the
/// identity on Eval.
#[inline]
pub fn stage3<C: Cf>(stage: PostStage, v: C::Vec3f) -> C::Vec3f {
    C::temp_vec3(stage.label(), v)
}

// ---- The binding table ----------------------------------------------------------------------------

/// How a `post_final` binding is read. Deliberately NOT `#[non_exhaustive]`: a new kind must break
/// the exhaustive `match` of PX0's dump resolver, the one loud edit `post_eval` ever needs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PostBindingKind {
    /// A `Texture2D<float4>` read with `Load` (no sampler).
    Tex2Load,
    /// A `Texture2D<float4>` read with a linear-clamp `SampleLevel`.
    Tex2Linear,
    /// A `Texture3D<float4>` read with a linear-clamp `SampleLevel`.
    Tex3Linear,
    /// A `StructuredBuffer<uint>` of words.
    Buf,
    /// The `PostFinal` constant buffer ([`POST_FINAL_UBO_LAYOUT`]).
    Ubo,
    /// The `RWTexture2D<float4>` display output.
    Store,
}

/// Where PX0's resolver finds a binding's contents on a dumped frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DumpSource {
    /// A fixed named resource of the frame's dump.
    Resource(&'static str),
    /// A path-dependent pass input (`lit` | `taa_hist[fi]` | `lit_native`), resolved through the
    /// receipt's declared access of `pass`'s `binding`.
    PassInput {
        /// The pass whose declared input this is.
        pass: &'static str,
        /// The pass's binding index.
        binding: u32,
    },
    /// Not read from a dump: the UBO (the receipt's raw words) and the store (the compared output).
    NoDump,
}

/// One row of [`POST_FINAL_BINDINGS`]: the generator prints the declaration from it, PX0's
/// resolver maps it to a dump.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PostBinding {
    /// The HLSL resource name.
    pub name: &'static str,
    /// How the body reads it.
    pub kind: PostBindingKind,
    /// The descriptor set.
    pub set: u32,
    /// The binding index.
    pub binding: u32,
    /// The `[[vk::image_format(…)]]` of a storage image, `None` elsewhere.
    pub image_format: Option<&'static str>,
    /// Where its contents come from on a dumped frame.
    pub dump: DumpSource,
}

/// `post_final`'s bindings, one row per declaration, in binding order.
pub const POST_FINAL_BINDINGS: &[PostBinding] = &[
    PostBinding {
        name: "gScene",
        kind: PostBindingKind::Tex2Load,
        set: 0,
        binding: 0,
        image_format: None,
        dump: DumpSource::PassInput { pass: "post_final", binding: 0 },
    },
    PostBinding {
        name: "gDisplay",
        kind: PostBindingKind::Store,
        set: 0,
        binding: 1,
        image_format: Some(DISPLAY_STORAGE_FORMAT),
        dump: DumpSource::NoDump,
    },
    PostBinding {
        name: "PostFinal",
        kind: PostBindingKind::Ubo,
        set: 0,
        binding: 2,
        image_format: None,
        dump: DumpSource::NoDump,
    },
];

// ---- The UBO layout ---------------------------------------------------------------------------------

/// A member type of the `PostFinal` constant buffer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PostUboKind {
    /// `uint`.
    U32,
    /// `float`.
    F32,
    /// `uint2`.
    Uint2,
}

/// One member of [`POST_FINAL_UBO_LAYOUT`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PostUboField {
    /// The cbuffer member name (the body reads it by this bare name on Emit).
    pub name: &'static str,
    /// The std140 byte offset.
    pub offset: u32,
    /// The member type.
    pub kind: PostUboKind,
}

/// The `PostFinal` constant buffer (32 B, std140), member by member. The generator prints the
/// `cbuffer` from it, [`eval_params`] reads the receipt's words by it, and
/// `boyko_render::post::gpu::PostFinalGpu` is pinned to it by a host test — one layout.
pub const POST_FINAL_UBO_LAYOUT: &[PostUboField] = &[
    PostUboField { name: "mode_words", offset: 0, kind: PostUboKind::U32 },
    PostUboField { name: "exposure_ratio", offset: 4, kind: PostUboKind::F32 },
    PostUboField { name: "extent", offset: 8, kind: PostUboKind::Uint2 },
    PostUboField { name: "_r0", offset: 16, kind: PostUboKind::U32 },
    PostUboField { name: "_r1", offset: 20, kind: PostUboKind::U32 },
    PostUboField { name: "_r2", offset: 24, kind: PostUboKind::U32 },
    PostUboField { name: "_r3", offset: 28, kind: PostUboKind::U32 },
];

/// The `PostFinal` buffer's size in bytes (8 words).
pub const POST_FINAL_UBO_BYTES: u32 = 32;

/// `a == b` over two strings, usable in `const` context.
const fn str_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    let mut i = 0;
    while i < a.len() {
        if a[i] != b[i] {
            return false;
        }
        i += 1;
    }
    true
}

/// The 32-bit word index of the [`POST_FINAL_UBO_LAYOUT`] member `name` (its offset / 4). Panics
/// (at compile time, when used in a `const`) on an unknown name.
pub const fn ubo_word(name: &str) -> usize {
    let mut i = 0;
    while i < POST_FINAL_UBO_LAYOUT.len() {
        if str_eq(POST_FINAL_UBO_LAYOUT[i].name, name) {
            return (POST_FINAL_UBO_LAYOUT[i].offset / 4) as usize;
        }
        i += 1;
    }
    panic!("not a POST_FINAL_UBO_LAYOUT member");
}

const MODE_WORDS_WORD: usize = ubo_word("mode_words");
const EXPOSURE_RATIO_WORD: usize = ubo_word("exposure_ratio");
const EXTENT_WORD: usize = ubo_word("extent");

const _: () = assert!(POST_FINAL_UBO_BYTES as usize == 8 * 4);
const _: () = assert!(EXTENT_WORD + 1 < POST_FINAL_UBO_BYTES as usize / 4);

/// The body's scalar inputs — data, never composition, so the Eval and Emit constructions cannot
/// disagree about what the body does.
pub struct PostParams<C: Cf> {
    /// The mode word (tonemapper at bits `TONEMAP_MODE_SHIFT..+4`).
    pub mode_words: C::Uint,
    /// `E / pre`, exactly 1.0 until PX3.
    pub exposure_ratio: C::Scalar,
    /// The dispatch extent (read by the skeleton's guard; carried so the body sees one input set).
    pub extent: [C::Uint; 2],
}

/// The Eval [`PostParams`] from the receipt's raw `PostFinalGpu` words, each read at the word
/// [`POST_FINAL_UBO_LAYOUT`] assigns it — so PX0's `post_eval` needs no `boyko_render` edge, and a
/// new UBO member cannot desynchronise this from the printed `cbuffer`.
#[inline]
pub fn eval_params(words: [u32; 8]) -> PostParams<EvalCf> {
    PostParams {
        mode_words: words[MODE_WORDS_WORD],
        exposure_ratio: f32::from_bits(words[EXPOSURE_RATIO_WORD]),
        extent: [words[EXTENT_WORD], words[EXTENT_WORD + 1]],
    }
}

/// Render wave PX0 (`px0/cut.md` D-T4): `post_final`'s per-pass texel-footprint radius — the
/// neighbourhood the body reads around a pixel through `gScene`. `0` while the body only `Load`s
/// its own pixel; PX1b sets it to 1 with RCAS (DS-H5). `post_eval bound` perturbs that footprint
/// jointly (verify r5 O2).
pub const POST_FINAL_TAP_RADIUS: u32 = 0;

/// Render wave PX0: the graph receipt's `[config]` keys of the eight `PostFinalGpu` words, in word
/// order — single-sourced for PX1a's receipt writer and `tools/golden_diff.py eval-post`.
pub const POST_FINAL_RECEIPT_KEYS: [&str; 8] =
    ["post_word0", "post_word1", "post_word2", "post_word3", "post_word4", "post_word5", "post_word6", "post_word7"];

/// Render wave PX0: what a dump resolves to for one fetching row of [`POST_FINAL_BINDINGS`] on the
/// Eval backend — the input of [`eval_res`].
#[derive(Clone, Copy, Debug)]
pub enum EvalBinding<'a> {
    /// A `Texture2D` row (`Tex2Load` / `Tex2Linear`).
    Tex2(HostTex2<'a>),
    /// A `Texture3D` row (`Tex3Linear`).
    Tex3(HostTex3<'a>),
    /// A `StructuredBuffer<uint>` row.
    Buf(&'a [u32]),
}

/// The body's resources, one per fetching row of [`POST_FINAL_BINDINGS`]. A row whose mode bit is
/// off resolves on Eval to [`HostTex2::EMPTY`] (0×0), whose every read is a bounds panic — a red,
/// never a silent zero.
pub struct PostRes<'a, C: Cf> {
    /// `gScene` — the scene input (`lit` | `taa_hist[fi]` | `lit_native`).
    pub scene: C::Tex2<'a>,
}

// ---- The leaves ---------------------------------------------------------------------------------------

/// Render wave PX0 (`px0/cut.md` D-T4): the Eval [`PostRes`] from a resolver called once per
/// FETCHING row of [`POST_FINAL_BINDINGS`] (every row but the UBO and the store) — so `post_eval`
/// never names a resource. A rung that adds a `PostRes` field adds its arm here, in its own `post.rs`
/// edit.
///
/// # Panics
/// When the resolver hands a row the wrong kind of binding (a red naming the row, never a zero).
pub fn eval_res<'a>(mut resolve: impl FnMut(&PostBinding) -> EvalBinding<'a>) -> PostRes<'a, EvalCf> {
    let mut scene = None;
    for row in POST_FINAL_BINDINGS {
        if matches!(row.kind, PostBindingKind::Ubo | PostBindingKind::Store) {
            continue;
        }
        let b = resolve(row);
        match (row.name, b) {
            ("gScene", EvalBinding::Tex2(t)) => scene = Some(t),
            (name, other) => panic!("eval_res: row `{name}` resolved to {other:?}, not its kind {:?}", row.kind),
        }
    }
    PostRes { scene: scene.expect("invariant: POST_FINAL_BINDINGS carries the gScene row") }
}

/// The three channels of a `float3`.
#[inline]
fn channels<C: Cf>(c: C::Vec3f) -> [C::Scalar; 3] {
    [C::vec3_x(c), C::vec3_y(c), C::vec3_z(c)]
}

/// A `float3` from three channels.
#[inline]
fn from_channels<C: Cf>(c: [C::Scalar; 3]) -> C::Vec3f {
    C::vec3_from_scalars(c[0], c[1], c[2])
}

/// `true` iff some channel of `ch` is non-finite — the WHOLE-pixel test [`hdr_sanitize`] and
/// [`hdr_history_guard`] share. `finite(x)` is the exponent-bit test `(asuint(x) & F32_EXP_MASK)
/// != F32_EXP_MASK`.
///
/// Branch-free on the device: `(asuint(x) & M) ^ M` is zero exactly for a non-finite channel, so
/// the whole-pixel test is one unsigned `min` of the three words against zero — integer ops only.
/// (A `||` of the three per-channel tests is a SHORT-CIRCUIT in HLSL; measured at PXe-5, DXC
/// lowered it to two data-dependent branches per channel.)
#[inline]
fn any_nonfinite<C: Cf>(ch: [C::Scalar; 3]) -> <C::Scalar as FieldScalar>::Mask {
    let mask = || C::named_uint_val("F32_EXP_MASK", F32_EXP_MASK);
    let finite_word = |x: C::Scalar| C::uxor(C::and_u(C::asuint(x), mask()), mask());
    let lowest = C::umin(C::umin(finite_word(ch[0]), finite_word(ch[1])), finite_word(ch[2]));
    C::ueq(lowest, C::uint_lit(0))
}

/// `finite(c) ? clamp(c, 0, HDR_MAX) : 0` for the WHOLE pixel: one non-finite channel kills all
/// three (the whole-pixel test is [`any_nonfinite`]). The clamp also removes the negatives a
/// Catmull-Rom history with a variance clip can produce. `max` / `min` only ever matter on finite
/// values (a non-finite pixel is selected away).
#[inline]
pub fn hdr_sanitize<C: Cf>(c: C::Vec3f) -> C::Vec3f {
    let ch = channels::<C>(c);
    let bad = any_nonfinite::<C>(ch);
    let zero = C::Scalar::lit(0.0);
    let hdr_max = C::named_lit("HDR_MAX", HDR_MAX);
    from_channels::<C>(ch.map(|x| C::select(bad, zero, x.max(zero).min(hdr_max))))
}

/// The `lit` producers' store bound (render wave PX1a, orchestrator ruling 67 W5): per channel
/// `c > HDR_MAX ? HDR_MAX : c`, applied to every `lit` store through ONE spelling (the generated
/// `lit_store_saturate` span of `shaders/lit_format.hlsli`).
///
/// A narrowing float store may round an out-of-range finite value to +Inf (Vulkan,
/// "Floating-Point Format Conversions"), and every HDR reader ([`hdr_sanitize`], TAA's history
/// guard) reads a non-finite texel as a FAULT and zeroes the pixel — so an overflowing highlight
/// would turn black. Bounding at `HDR_MAX` (binary16-representable) stores it as the brightest
/// finite value instead. +Inf maps to `HDR_MAX` too.
///
/// **A NaN stays a NaN**, so K8's NaN-as-fault reading downstream is unchanged: the bound is a
/// select on an ORDERED compare (`NaN > HDR_MAX` is false, so the select keeps `c`), never `min`,
/// whose device lowering (`NMin`) returns the number when one operand is NaN. Not a display clamp:
/// values in `(1, HDR_MAX]` pass unchanged.
#[inline]
pub fn lit_store_saturate<C: Cf>(c: C::Vec3f) -> C::Vec3f {
    let hdr_max = C::named_lit("HDR_MAX", HDR_MAX);
    from_channels::<C>(channels::<C>(c).map(|x| C::select(x.gt(hdr_max), hdr_max, x)))
}

/// TAA's history write guard (H4 Scope 5, rev 2 perf O6; `px1a/cut.md` D-S2): `finite(hist) ?
/// max(hist, 0) : hdr_sanitize(cur)` for the WHOLE pixel. A Catmull-Rom history with a variance
/// clip can go negative, and the RGBA16F history keeps the sign the old RGBA8 store clamped; a
/// non-finite blend reseeds from the sanitized current sample instead of persisting. `max` only
/// ever reads a finite value (a non-finite pixel takes the other arm).
#[inline]
pub fn hdr_history_guard<C: Cf>(hist: C::Vec3f, cur: C::Vec3f) -> C::Vec3f {
    let h = channels::<C>(hist);
    let bad = any_nonfinite::<C>(h);
    let fresh = channels::<C>(hdr_sanitize::<C>(cur));
    let zero = C::Scalar::lit(0.0);
    from_channels::<C>(core::array::from_fn(|i| C::select(bad, fresh[i], h[i].max(zero))))
}

/// `h · r` — the exposure ratio `E / pre` (POSTFX §5.3 E2). One rounding per channel; an identity
/// at `r = 1`.
#[inline]
pub fn pre_ratio_apply<C: Cf>(h: C::Vec3f, r: C::Scalar) -> C::Vec3f {
    C::vec3_mul_scalar(h, r)
}

// ---- Render wave PX1b: FSR 1 RCAS in FSR 2's reversible space -----------------------------------

/// FSR 2's `Tonemap` (`ffx_fsr2_common.h`): `y = c · rcp(1 + max3(c))`, the reversible map that
/// brings a sanitized HDR colour into `[0, 1)` for RCAS, whose lobe solves for the clip of a
/// `[0, 1]` signal.
#[inline]
pub fn tonemap_reversible<C: Cf>(c: C::Vec3f) -> C::Vec3f {
    let _ = c;
    unimplemented!("render wave PX1b C2: tonemap_reversible")
}

/// FSR 2's `InverseTonemap` (`ffx_fsr2_common.h`): `x = y · rcp(max(1 − max3(y), RCAS_INV_EPS))`,
/// the inverse of [`tonemap_reversible`].
#[inline]
pub fn tonemap_reversible_inverse<C: Cf>(y: C::Vec3f) -> C::Vec3f {
    let _ = y;
    unimplemented!("render wave PX1b C2: tonemap_reversible_inverse")
}

/// FSR 1's `FsrRcasF` lobe (32-bit, no `FSR_RCAS_DENOISE`) over the cross `b` (above), `d` (left),
/// `e` (centre), `f` (right), `h` (below), in reversible space, times `con`.
#[inline]
pub fn rcas_lobe<C: Cf>(b: C::Vec3f, d: C::Vec3f, e: C::Vec3f, f: C::Vec3f, h: C::Vec3f, con: C::Scalar) -> C::Scalar {
    let _ = (b, d, e, f, h, con);
    unimplemented!("render wave PX1b C2: rcas_lobe")
}

/// FSR 1's `FsrRcasF` resolve: `(lobe·b + lobe·d + lobe·h + lobe·f + e) · rcp(4·lobe + 1)` per
/// channel, in FSR 1's operand order.
#[inline]
pub fn rcas_resolve<C: Cf>(lobe: C::Scalar, b: C::Vec3f, d: C::Vec3f, e: C::Vec3f, f: C::Vec3f, h: C::Vec3f) -> C::Vec3f {
    let _ = (lobe, b, d, e, f, h);
    unimplemented!("render wave PX1b C2: rcas_resolve")
}

/// `mul(m, v)` with row `i` dotted in column order 0, 1, 2, left-associated — the op order of the
/// resolve's `aces_fitted` and of `goldens.rs::mat3_mul_vec3`. Each row is materialised.
#[inline]
fn mat3_rows<C: Cf>(m: &[[f32; 3]; 3], v: [C::Scalar; 3]) -> [C::Scalar; 3] {
    core::array::from_fn(|i| {
        let lit = C::Scalar::lit;
        C::temp(
            lit(m[i][0]).mul(v[0]).add(lit(m[i][1]).mul(v[1])).add(lit(m[i][2]).mul(v[2])),
        )
    })
}

/// Stephen Hill's ACES fit (BakingLab), byte-for-byte the resolve's `aces_fitted`: `c = ACES_IN·c;
/// a = c(c + 0.0245786) − 0.000090537; b = c(0.983729c + 0.432951) + 0.238081;
/// saturate(ACES_OUT·(a / b))` — the same op order as `goldens.rs::aces_fitted`, so PX1a's
/// bit-exact host comparison holds by construction.
#[inline]
pub fn aces_hill<C: Cf>(c: C::Vec3f) -> C::Vec3f {
    let lit = C::Scalar::lit;
    let v = mat3_rows::<C>(&ACES_IN, channels::<C>(c));
    let q = v.map(|v| {
        let a = C::temp(v.mul(v.add(lit(ACES_A))).sub(lit(ACES_B)));
        let b = C::temp(v.mul(lit(ACES_C).mul(v).add(lit(ACES_D))).add(lit(ACES_E)));
        C::temp(a.div(b))
    });
    from_channels::<C>(mat3_rows::<C>(&ACES_OUT, q).map(FieldScalar::clamp01))
}

/// Khronos PBR Neutral (KhronosGroup/ToneMapping), as the resolve's `khronos_pbr_neutral`, with its
/// early return made a select (a leaf is return-free): below `peak < startCompression` the result
/// is the toe-offset colour, else the compressed-and-desaturated one. Both arms are evaluated; the
/// discarded one may divide by a tiny `peak`, which a select never reads.
#[inline]
pub fn khronos_pbr_neutral<C: Cf>(c: C::Vec3f) -> C::Vec3f {
    let lit = C::Scalar::lit;
    let ch = channels::<C>(c);
    let x = C::temp(ch[0].min(ch[1].min(ch[2])));
    let offset = C::temp(C::select(
        x.lt(lit(NEUTRAL_TOE)),
        x.sub(lit(NEUTRAL_TOE_K).mul(x).mul(x)),
        lit(NEUTRAL_F90),
    ));
    let col = ch.map(|v| C::temp(v.sub(offset)));
    let peak = C::temp(col[0].max(col[1].max(col[2])));
    let new_peak = C::temp(
        lit(1.0).sub(lit(NEUTRAL_D2).div(peak.add(lit(NEUTRAL_D)).sub(lit(NEUTRAL_START)))),
    );
    let scale = C::temp(new_peak.div(peak));
    let g = C::temp(lit(1.0).sub(
        lit(1.0).div(lit(NEUTRAL_DESATURATION).mul(peak.sub(new_peak)).add(lit(1.0))),
    ));
    let early = peak.lt(lit(NEUTRAL_START));
    from_channels::<C>(col.map(|v| C::select(early, v, v.mul(scale).lerp(new_peak, g))))
}

/// Reinhard-Jodie, as the resolve's `reinhard_jodie`: `l = dot(v, Rec.709 luma); tv = v / (1 + v);
/// saturate(lerp(v / (1 + l), tv, tv))`.
#[inline]
pub fn reinhard_jodie<C: Cf>(c: C::Vec3f) -> C::Vec3f {
    let lit = C::Scalar::lit;
    let luma = C::vec3_from_scalars(lit(REC709_LUMA[0]), lit(REC709_LUMA[1]), lit(REC709_LUMA[2]));
    let l = C::temp(C::vec3_dot(c, luma));
    from_channels::<C>(channels::<C>(c).map(|v| {
        let tv = C::temp(v.div(lit(1.0).add(v)));
        v.div(lit(1.0).add(l)).lerp(tv, tv).clamp01()
    }))
}

/// The tonemapper word: `(mode_words >> TONEMAP_MODE_SHIFT) & TONEMAP_MODE_MASK`, materialised once.
#[inline]
pub fn tonemapper<C: Cf>(mode_words: C::Uint) -> C::Uint {
    C::temp_uint(
        "post_tonemap_mode",
        C::and_u(
            C::shr_u(mode_words, C::named_uint_val("TONEMAP_MODE_SHIFT", TONEMAP_MODE_SHIFT)),
            C::named_uint_val("TONEMAP_MODE_MASK", TONEMAP_MODE_MASK),
        ),
    )
}

/// The tonemap curve `mode` selects — Neutral (1), Reinhard-Jodie (2), else ACES (0 and every
/// unknown word, the resolve's fallback). A REAL branch (`[branch]`): the word is wave-uniform, so
/// only the selected curve runs. Declares the body-level `post_tonemapped`, so it is called once.
#[inline]
pub fn tonemap_select<C: Cf>(c: C::Vec3f, mode: C::Uint) -> C::Vec3f {
    let out = C::decl_var_vec3("post_tonemapped", c);
    let neutral = C::ueq(mode, C::named_uint_val("TONEMAP_NEUTRAL", TONEMAP_NEUTRAL));
    let _ = C::if_else_branch(
        neutral,
        || {
            C::set_var_vec3(&out, khronos_pbr_neutral::<C>(c));
            Flow::Continue(())
        },
        || {
            let jodie = C::ueq(mode, C::named_uint_val("TONEMAP_REINHARD_JODIE", TONEMAP_REINHARD_JODIE));
            C::if_else_branch(
                jodie,
                || {
                    C::set_var_vec3(&out, reinhard_jodie::<C>(c));
                    Flow::Continue(())
                },
                || {
                    C::set_var_vec3(&out, aces_hill::<C>(c));
                    Flow::Continue(())
                },
            )
        },
    );
    C::get_var_vec3(&out)
}

/// The manual gamma OETF `pow(c, OETF_GAMMA_EXP)` per channel — the intrinsic `pow` (E0), so the
/// module carries three `Pow`s on the gamma constant (one per channel), never an `exp2(y·log2 x)`
/// rewrite.
#[inline]
pub fn oetf_gamma22<C: Cf>(c: C::Vec3f) -> C::Vec3f {
    let e = C::named_lit("OETF_GAMMA_EXP", OETF_GAMMA_EXP);
    from_channels::<C>(channels::<C>(c).map(|v| C::pow(v, e)))
}

/// The piecewise sRGB decode `c ≤ 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055)^2.4` — the inverse of
/// the hardware's `*_SRGB` encode, for PX1a's swapchain fallback (NOT `pow(c, 2.2)`, which is ≈ 7
/// LSB off at 0.1). Not called by `post_final`'s body: `emit::emit_hlsl_srgb_eotf` splices it into
/// the present blit's `SWAPCHAIN_SRGB` variant (`fullscreen_sample.fs.hlsl`, render wave PX1a D-P1).
#[inline]
pub fn srgb_eotf<C: Cf>(c: C::Vec3f) -> C::Vec3f {
    let lit = C::Scalar::lit;
    from_channels::<C>(channels::<C>(c).map(|v| {
        C::select(
            v.le(lit(SRGB_EOTF_KNEE)),
            v.div(lit(SRGB_LINEAR_DIV)),
            C::pow(v.add(lit(SRGB_OFFSET)).div(lit(SRGB_SCALE)), lit(SRGB_GAMMA)),
        )
    }))
}

// ---- The traced main body -------------------------------------------------------------------------------

/// `post_final`'s whole per-pixel body: fetch the scene texel at `px` once, then
/// `hdr_sanitize → pre_ratio_apply → tonemap_select → oetf_gamma22`, each stage through its
/// [`stage3`] marker, returning `(o, 1)` through `ret_out`. Its fetch is the resource facet
/// `tex2_load`, a real image read on Eval, so the oracle and the shader run one function.
#[inline]
pub fn post_final_main<C: Cf>(
    px: [C::Uint; 2],
    p: &PostParams<C>,
    r: &PostRes<'_, C>,
    ret_out: &C::RetCellV4,
) -> Flow {
    let scene = C::temp_vec4(
        "post_scene",
        C::tex2_load(r.scene, C::int_from_uint(px[0]), C::int_from_uint(px[1])),
    );
    let s = stage3::<C>(PostStage::HdrSanitize, hdr_sanitize::<C>(C::vec4_xyz(scene)));
    let s = stage3::<C>(PostStage::PreRatio, pre_ratio_apply::<C>(s, p.exposure_ratio));
    let s = stage3::<C>(PostStage::Tonemap, tonemap_select::<C>(s, tonemapper::<C>(p.mode_words)));
    let s = stage3::<C>(PostStage::Oetf, oetf_gamma22::<C>(s));
    let o = channels::<C>(s);
    C::ret_vec4(ret_out, C::vec4_from_scalars(o[0], o[1], o[2], C::Scalar::lit(1.0)))
}

/// Host convenience: [`post_final_main`] over the Eval backend for the pixel `px`.
#[inline]
pub fn post_final_eval(px: [u32; 2], p: &PostParams<EvalCf>, scene: HostTex2<'_>) -> [f32; 4] {
    let cell = Cell::new([0.0f32; 4]);
    let _ = post_final_main::<EvalCf>(px, p, &PostRes { scene }, &cell);
    cell.get()
}
