//! Render wave PX1a — the HDR boundary's host fixtures (`plan/H4-PX1a-hdr-lit.md` Tests 5;
//! `px1a/cut.md` D-S3, C7; critique O2; orchestrator ruling 67 W5). Host-only, no device.
//!
//! **Test 5 — the background constant, pre-inverted THROUGH the store.** Every pixel no producer
//! covers holds [`LIT_BACKGROUND_SCENE`] (the Deferred no-SkyLight arm, Forward's sky fallback,
//! `FORWARD_LIT_CLEAR`, `VB_LIT_CLEAR`). Before PX1a those sites stored the display-referred
//! `(0.05, 0.05, 0.1)`, which the blessing device converted to `(13, 13, 25)` (blue
//! `0.1 · 255 = 25.5000004` → 25; the PX0 fixtures' `lit` dumps hold it, GL-H0's
//! `premove/fixture_{deferred_base,forward_lit_clear,forward_sky_base}_lit.bin`). After PX1a they
//! store scene-linear radiance and `post_final` displays it, so the constant is chosen so that
//! `quantize8(OETF(ACES(unpack(pack_Z(c)))))` is that background — at the CENTRE of each 8-bit
//! bin (critique O2), so a device's transcendental error cannot flip it by one code.
//!
//! **Ruling 67 W5 — the producers' store bound.** An overflowing highlight (and +Inf) stores
//! `HDR_MAX` through `lit_store_saturate` and displays white; a NaN stays a NaN through it and
//! takes the fault path (`post_final`'s `hdr_sanitize` blacks the pixel). Without the bound, a
//! conformant device that rounds the overflow to +Inf would black the highlight — the case below
//! shows that class too.
//!
//! The device side is `post_final.comp.hlsl` (the SAME traced body as [`post_final_eval`]) and the
//! spliced `lit_store_saturate` span (the SAME leaf, `tests/post_chain_spv_sync.rs`).
//!
//! **Render wave PXt.** [`LIT_BACKGROUND_SCENE`] is PX1a's inversion through ACES Hill, and stays:
//! re-inverting it through AgX would move `lit` on every background pixel at the flip
//! (`pxt/cut.md` D-F4). Under any other default curve (Khronos PBR Neutral since ruling 116) the
//! fallback background displays other codes; `the_fallback_background_under_the_default_curve`
//! prints them for the review legend.

use boyko_rhi_vulkan::present::LIT_BACKGROUND_SCENE;
use boyko_shaderdsl::cf::{EvalCf, HostTex2};
use boyko_shaderdsl::pixfmt::{Channel, Rounding, quantize};
use boyko_shaderdsl::post::{HDR_MAX, TONEMAP_MODE_SHIFT, eval_params, lit_store_saturate, post_final_eval, ubo_word};

/// The 8-bit background the display-referred store wrote before PX1a, on the blessing device.
const OLD_BACKGROUND_8: [u8; 3] = [13, 13, 25];

/// How far from its 8-bit bin's centre the displayed background may land, in codes (O2: the
/// chosen constant is within 0.002; a value near a ±0.5 edge would fail here, not on a device).
const CENTRE_MARGIN: f32 = 0.05;

/// `post_final`'s output for one scene texel: ACES Hill (mode 0), exposure ratio 1, extent 1×1.
fn display(rgb: [f32; 3]) -> [f32; 3] {
    display_under(0, rgb)
}

/// `post_final`'s output for one scene texel under tonemapper word `curve`, exposure ratio 1,
/// extent 1×1.
fn display_under(curve: u32, rgb: [f32; 3]) -> [f32; 3] {
    let mut words = [0u32; 8];
    words[ubo_word("mode_words")] = curve << TONEMAP_MODE_SHIFT;
    words[ubo_word("exposure_ratio")] = 1.0f32.to_bits();
    words[ubo_word("extent")] = 1;
    words[ubo_word("extent") + 1] = 1;
    let texel = [[rgb[0], rgb[1], rgb[2], 1.0]];
    let o = post_final_eval([0, 0], &eval_params(words), HostTex2::new(&texel, 1, 1));
    [o[0], o[1], o[2]]
}

/// The RGBA16F store of one channel under `r` (the blessing device's store class is `Z`,
/// `goldens::RGBA16F_ROUNDING`).
fn store16(x: f32, r: Rounding) -> f32 {
    quantize(Channel::F16, x, r)
}

/// A producer's store: the bound, then the RGBA16F store.
fn producer_store(rgb: [f32; 3], r: Rounding) -> [f32; 3] {
    lit_store_saturate::<EvalCf>(rgb).map(|x| store16(x, r))
}

fn quantize8(x: f32) -> u8 {
    (x.clamp(0.0, 1.0) * 255.0 + 0.5) as u8
}

#[test]
fn the_background_constant_is_binary16_representable() {
    for (k, c) in LIT_BACKGROUND_SCENE.into_iter().enumerate() {
        assert!(c > 0.0 && c.is_finite(), "channel {k}: {c}");
        for r in [Rounding::TowardZero, Rounding::NearestEven] {
            assert_eq!(store16(c, r).to_bits(), c.to_bits(), "channel {k} ({c}) moves through the RGBA16F store under {r:?}");
        }
    }
}

#[test]
fn the_background_constant_displays_the_old_background_at_its_bin_centres() {
    let stored = LIT_BACKGROUND_SCENE.map(|c| store16(c, Rounding::TowardZero));
    let d = display(stored);
    for (k, (&v, &old)) in d.iter().zip(&OLD_BACKGROUND_8).enumerate() {
        let codes = v * 255.0;
        assert_eq!(quantize8(v), old, "channel {k}: post_final displays {codes} codes, not the old background {old}");
        let off = (codes - f32::from(old)).abs();
        assert!(off <= CENTRE_MARGIN, "channel {k}: {codes} codes is {off} from the bin centre {old} (margin {CENTRE_MARGIN})");
    }
}

/// Render wave PXt (`pxt/cut.md` D-F4; critique O1) — RECORDED for the review legend: the 8-bit codes
/// the fallback background displays under the engine's default curve, beside ACES Hill's
/// ([`OLD_BACKGROUND_8`], computed, not a literal). Once the default is not ACES the two must
/// differ — the default reached this display path.
#[test]
fn the_fallback_background_under_the_default_curve() {
    let stored = LIT_BACKGROUND_SCENE.map(|c| store16(c, Rounding::TowardZero));
    let aces = display_under(0, stored).map(quantize8);
    let curve = boyko_render::Tonemapper::default();
    let codes = display_under(curve as u32, stored).map(quantize8);
    println!("fallback background: ACES Hill {aces:?} (the pinned {OLD_BACKGROUND_8:?}), the default {curve:?} {codes:?}");
    assert_eq!(aces, OLD_BACKGROUND_8, "ACES Hill must still display the pinned background");
    if curve != boyko_render::Tonemapper::Aces {
        assert_ne!(codes, aces, "the default {curve:?} displays the background exactly as ACES Hill: the default did not reach the display");
    }
}

/// The ONE HLSL spelling (`lit_format.hlsli`) is the Rust constant, bit for bit.
#[test]
fn the_hlsl_twin_is_the_rust_constant() {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("shaders/lit_format.hlsli");
    let src = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let decl = "static const float3 LIT_BACKGROUND_SCENE = float3(";
    let lines: Vec<&str> = src.lines().filter(|l| l.trim_start().starts_with(decl)).collect();
    assert_eq!(lines.len(), 1, "lit_format.hlsli spells LIT_BACKGROUND_SCENE {} times", lines.len());
    let args = lines[0].trim_start()[decl.len()..].trim_end().trim_end_matches(';').trim_end_matches(')');
    let hlsl: Vec<f32> = args.split(',').map(|a| a.trim().parse::<f32>().unwrap_or_else(|e| panic!("`{a}`: {e}"))).collect();
    assert_eq!(hlsl.len(), 3, "three channels");
    for (k, (h, r)) in hlsl.iter().zip(LIT_BACKGROUND_SCENE).enumerate() {
        assert_eq!(h.to_bits(), r.to_bits(), "channel {k}: HLSL {h} vs Rust {r}");
    }
}

/// The verdict rejects the display-referred value the sites stored before PX1a: stored as
/// scene-linear radiance it no longer displays the old background.
#[test]
fn the_old_display_referred_value_is_not_the_scene_linear_background() {
    let d = display([0.05, 0.05, 0.1]);
    assert_ne!(d.map(quantize8), OLD_BACKGROUND_8, "the pre-PX1a constant would still display as before");
}

#[test]
fn an_overflowing_highlight_stores_hdr_max_and_displays_white() {
    for x in [65_520.0f32, 1.0e6, f32::MAX, f32::INFINITY] {
        for r in [Rounding::TowardZero, Rounding::NearestEven] {
            let s = producer_store([x, 2.0, 0.5], r);
            assert_eq!(s[0], HDR_MAX, "{x} under {r:?}: the producer stores {} (not HDR_MAX)", s[0]);
            assert_eq!(s[1], 2.0, "an in-range channel passes the bound unchanged");
            let d = display(s);
            assert_eq!(quantize8(d[0]), 255, "{x} under {r:?}: the highlight displays {} (not white)", d[0]);
        }
    }
}

/// The class the bound closes: without it, an overflow a conformant device rounds to +Inf reaches
/// `post_final` non-finite, and `hdr_sanitize` blacks the WHOLE pixel.
#[test]
fn without_the_bound_an_overflow_rounded_to_inf_displays_black() {
    let raw = [store16(1.0e6, Rounding::NearestEven), 2.0, 0.5];
    assert_eq!(raw[0], f32::INFINITY, "a NearestEven RGBA16F store rounds 1e6 to +Inf");
    assert_eq!(display(raw), [0.0; 3], "post_final's fault path");
}

#[test]
fn a_nan_stays_a_nan_through_the_bound_and_takes_the_fault_path() {
    let s = producer_store([f32::NAN, 2.0, 0.5], Rounding::TowardZero);
    assert!(s[0].is_nan(), "the bound turned a NaN into {}", s[0]);
    assert_eq!(s[1], 2.0);
    assert_eq!(display(s), [0.0; 3], "a NaN texel displays black (hdr_sanitize, K8's fault reading)");
}

#[test]
fn values_inside_the_range_pass_the_bound_unchanged() {
    for x in [0.0f32, -0.0, 1.0e-7, 0.5, 1.0, 2.0, 1000.0, HDR_MAX] {
        let out = lit_store_saturate::<EvalCf>([x, x, x]);
        assert!(out.iter().all(|o| o.to_bits() == x.to_bits()), "{x} became {out:?}");
    }
}
