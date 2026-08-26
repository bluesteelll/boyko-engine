//! The **UI-rect leaf** pins (`feature = "emit"`) — `docs/UI-PLAN-SPRITES.md` rung S1 gate
//! G1-3, one test pair per leaf ([`boyko_shaderdsl::ui`]):
//!
//! - the **Eval** value — `<EvalCf>` over host `f32` ops, checked against constants derived
//!   OUTSIDE the implementation (closed-form algebra at points chosen to be EXACT in f32:
//!   3-4-5 corner distances, power-of-two coverages, saturated smoothstep ends) rather than
//!   against a re-run of the leaf;
//! - the **Emit** text — `<EmitCf>` through the HLSL printer, checked as the FULL span, so a
//!   wrong spelling, a lost paren (`* (1.0 / 255.0)` degrading to a vector divide) or a lost
//!   temp all fail here, before `ui_rect_edsl_sync` ever compares against the committed file.
//!
//! `ui_screen_px_range` has an Emit pin ONLY: its `fwidth` is a device derivative with no host
//! semantics ([`Cf::vec2_fwidth`]'s Eval arm is an honest panic), so the leaf is deliberately
//! not oracle-swept — the byte-identity pair (`ui_rect_edsl_sync` + `ui_rect_spv_sync`) is its
//! gate, and this file says so rather than leaving the gap silent.
//!
//! Gated on `feature = "emit"` (the printer surface is `#[cfg(feature = "emit")]`).

#![cfg(feature = "emit")]

use core::cell::Cell;

use boyko_shaderdsl::cf::EvalCf;
use boyko_shaderdsl::emit;
use boyko_shaderdsl::ui as leaves;

// ---- Eval drivers ------------------------------------------------------------------------

/// Runs `ui_unpack_rgba8` over `EvalCf`.
fn eval_unpack(c: u32) -> [f32; 4] {
    let cell: Cell<[f32; 4]> = Cell::new([0.0; 4]);
    let _ = leaves::ui_unpack_rgba8_body::<EvalCf>(c, &cell);
    cell.get()
}

/// Runs `ui_sd_rounded_box` over `EvalCf`.
fn eval_sd(p: [f32; 2], half_size: [f32; 2], r: [f32; 4]) -> f32 {
    let cell: Cell<f32> = Cell::new(f32::NAN);
    let _ = leaves::ui_sd_rounded_box_body::<EvalCf>(p, half_size, r, &cell);
    cell.get()
}

/// Runs `ui_clip_coverage` over `EvalCf`.
fn eval_clip(pos: [f32; 2], clip: [f32; 4], fw: f32) -> f32 {
    let cell: Cell<f32> = Cell::new(f32::NAN);
    let _ = leaves::ui_clip_coverage_body::<EvalCf>(pos, clip, fw, &cell);
    cell.get()
}

/// Runs `ui_median3` over `EvalCf`.
fn eval_median3(r: f32, g: f32, b: f32) -> f32 {
    let cell: Cell<f32> = Cell::new(f32::NAN);
    let _ = leaves::ui_median3_body::<EvalCf>(r, g, b, &cell);
    cell.get()
}

/// Runs `ui_tile_uv` over `EvalCf` (UI-ADVANCED S5).
fn eval_tile_uv(uv: [f32; 4], local_uv: [f32; 2], flags: u32) -> [f32; 2] {
    let cell: Cell<[f32; 2]> = Cell::new([f32::NAN; 2]);
    let _ = leaves::ui_tile_uv_body::<EvalCf>(uv, local_uv, flags, &cell);
    cell.get()
}

/// Packs a `flags` word carrying `FLAG_TILED` and the two repeat counts — the
/// same bit layout `boyko_render::ui::pack` writes and this leaf reads, spelled
/// here from `boyko_shaderdsl::ui`'s own constants so the oracle exercises the
/// DECODE rather than a second copy of it.
fn tiled_flags(tx: u32, ty: u32) -> u32 {
    (1 << leaves::UI_TILE_FLAG_BIT)
        | (tx << leaves::UI_TILE_X_SHIFT)
        | (ty << leaves::UI_TILE_Y_SHIFT)
}

/// Runs `ui_premultiplied_over` over `EvalCf`.
fn eval_over(bc: [f32; 4], border_cov: f32, fill: [f32; 4], inner_cov: f32) -> [f32; 4] {
    let cell: Cell<[f32; 4]> = Cell::new([0.0; 4]);
    let _ = leaves::ui_premultiplied_over_body::<EvalCf>(bc, border_cov, fill, inner_cov, &cell);
    cell.get()
}

// ---- G1-3: the `ui_sd_rounded_box` oracle table ------------------------------------------

/// The rounded-box SDF against a hand-computed table: deep inside (the `length` term
/// vanishes), an exact 3-4-5 corner outside, and all FOUR quadrants of the per-corner radius
/// select with `r = (tl, tr, br, bl) = (1, 2, 3, 4)` — each quadrant's corner placed so the
/// clamped `q` is exactly `(3, 4)` and the distance is exactly `5 - rr`, so the four expected
/// values `{2, 3, 1, 4}` DISCRIMINATE the select (a swapped pair changes the answer).
#[test]
fn ui_sd_rounded_box_oracle_table() {
    // Degenerate r == 0: deep inside — d = max(q) clamped, no corner term.
    assert_eq!(eval_sd([3.0, 4.0], [10.0, 10.0], [0.0; 4]), -6.0);
    // Degenerate r == 0: outside a corner — the exact 3-4-5 distance.
    assert_eq!(eval_sd([13.0, 10.0], [10.0, 6.0], [0.0; 4]), 5.0);

    let r = [1.0, 2.0, 3.0, 4.0]; // (tl, tr, br, bl)
    // TOP-RIGHT quadrant (p.x > 0, p.y > 0): rx = r.yz = (2, 3), rr = rx.y = 3.
    // q = |p| - half + rr = (0, 1) + 3 = (3, 4) → d = 5 - 3 = 2.
    assert_eq!(eval_sd([10.0, 7.0], [10.0, 6.0], r), 2.0);
    // BOTTOM-RIGHT (p.x > 0, p.y < 0): rr = rx.x = 2. q = (1, 2) + 2 = (3, 4) → d = 5 - 2 = 3.
    assert_eq!(eval_sd([11.0, -8.0], [10.0, 6.0], r), 3.0);
    // TOP-LEFT (p.x < 0, p.y > 0): rx = r.xw = (1, 4), rr = rx.y = 4.
    // q = (-1, 0) + 4 = (3, 4) → d = 5 - 4 = 1.
    assert_eq!(eval_sd([-9.0, 6.0], [10.0, 6.0], r), 1.0);
    // BOTTOM-LEFT (p.x < 0, p.y < 0): rr = rx.x = 1. q = (2, 3) + 1 = (3, 4) → d = 5 - 1 = 4.
    assert_eq!(eval_sd([-12.0, -9.0], [10.0, 6.0], r), 4.0);
}

// ---- G1-3: the `ui_median3` orderings ----------------------------------------------------

/// The MSDF median over ALL SIX orderings of three distinct values (the gate names three; six
/// costs nothing and closes the permutation set), plus a two-equal tie.
#[test]
fn ui_median3_all_orderings() {
    for (r, g, b) in [
        (1.0, 2.0, 3.0),
        (1.0, 3.0, 2.0),
        (2.0, 1.0, 3.0),
        (2.0, 3.0, 1.0),
        (3.0, 1.0, 2.0),
        (3.0, 2.0, 1.0),
    ] {
        assert_eq!(eval_median3(r, g, b), 2.0, "median of a permutation of (1,2,3)");
    }
    assert_eq!(eval_median3(2.0, 2.0, 1.0), 2.0, "two-equal tie");
}

// ---- The `ui_clip_coverage` saturated-end table ------------------------------------------

/// The clip coverage where smoothstep is EXACT: fully inside (1), fully outside past either
/// edge (0), and ON an edge (t = 0.5 → coverage exactly 0.5 — `0.5² · (3 − 1)` is exact in
/// f32). The AA mid-band's general values carry the smoothstep divide and are deliberately
/// not pinned (the module doc's carve-out).
#[test]
fn ui_clip_coverage_saturated_ends() {
    let clip = [10.0, 20.0, 110.0, 120.0]; // (min.xy, max.xy)
    let fw = 2.0;
    assert_eq!(eval_clip([60.0, 70.0], clip, fw), 1.0, "fully inside");
    assert_eq!(eval_clip([0.0, 70.0], clip, fw), 0.0, "past min.x");
    assert_eq!(eval_clip([60.0, 130.0], clip, fw), 0.0, "past max.y");
    assert_eq!(eval_clip([10.0, 70.0], clip, fw), 0.5, "exactly on the min.x edge");
}

// ---- The `ui_unpack_rgba8` routing table -------------------------------------------------

/// The RGBA8 unpack: byte→lane ROUTING (byte0 = R .. byte3 = A — a swapped shift lands in the
/// wrong lane) against the spec formula `float(byte) * (1.0 / 255.0)` transcribed here
/// independently, at bytes where the product is trivially exact (0) and at four distinct
/// bytes that discriminate every routing swap.
#[test]
fn ui_unpack_rgba8_routing() {
    let k = 1.0f32 / 255.0f32;
    assert_eq!(eval_unpack(0), [0.0; 4]);
    // 0x11223344 → R = 0x44 (68), G = 0x33 (51), B = 0x22 (34), A = 0x11 (17).
    assert_eq!(
        eval_unpack(0x1122_3344),
        [68.0 * k, 51.0 * k, 34.0 * k, 17.0 * k]
    );
    assert_eq!(
        eval_unpack(0xFFFF_FFFF),
        [255.0 * k, 255.0 * k, 255.0 * k, 255.0 * k]
    );
}

// ---- The `ui_premultiplied_over` algebra table -------------------------------------------

/// The premultiplied OVER at power-of-two coverages (exact in f32): an opaque ring hides the
/// fill, a half-alpha ring passes half the fill through, and a zero-coverage ring is the
/// identity on the fill term.
#[test]
fn ui_premultiplied_over_algebra() {
    // Opaque ring, no fill under it: result IS the ring.
    assert_eq!(
        eval_over([0.5, 0.0, 0.0, 1.0], 1.0, [0.0, 1.0, 0.0, 1.0], 0.0),
        [0.5, 0.0, 0.0, 1.0]
    );
    // Half-alpha ring over a full fill: dst scaled by (1 - 0.5).
    assert_eq!(
        eval_over([0.5, 0.0, 0.0, 0.5], 1.0, [0.0, 0.25, 0.0, 1.0], 1.0),
        [0.5, 0.125, 0.0, 1.0]
    );
    // Zero ring coverage: src = 0, result = fill * inner_cov.
    assert_eq!(
        eval_over([1.0, 1.0, 1.0, 1.0], 0.0, [0.75, 0.5, 0.25, 1.0], 0.5),
        [0.375, 0.25, 0.125, 0.5]
    );
}

/// `ui_tile_uv`'s Eval table (UI-ADVANCED S5 / S-D15) — BOTH arms, at points
/// chosen to be exact in `f32`.
///
/// The leaf is oracle-swept where its S1 siblings' `fwidth` sibling is not: the
/// field decode is integer, `frac` is one exact subtract, and the `lerp` is the
/// `FMix` spec form (mul/add, the standing FMA carve-out). Every constant below
/// is a sum of negative powers of two.
///
/// The UNTILED arm is pinned as hard as the tiled one, and that is deliberate:
/// it is the arm the four S2 / one S3 / one S4 committed image pins ride on, and
/// the whole reason [`Cf::vec2_lerp`] spells the intrinsic rather than the
/// `a + t * (b - a)` decomposition.
#[test]
fn ui_tile_uv_oracle_table() {
    // A sub-rect that is NOT (0,0,1,1), so "wraps inside the SUB-RECT" is
    // falsifiable against "wraps inside the whole texture".
    let sub = [0.5, 0.25, 0.75, 0.5];

    // (1) UNTILED (`FLAG_TILED` clear, both count fields zero): the plain lerp.
    assert_eq!(eval_tile_uv(sub, [0.0, 0.0], 0), [0.5, 0.25]);
    assert_eq!(eval_tile_uv(sub, [1.0, 1.0], 0), [0.75, 0.5]);
    assert_eq!(eval_tile_uv(sub, [0.5, 0.5], 0), [0.625, 0.375]);
    assert_eq!(eval_tile_uv(sub, [0.25, 0.75], 0), [0.5625, 0.4375]);

    // (2) `tiles == (1, 1)` under the FLAG is the SAME picture as untiled, for
    //     every `t` in [0, 1) — `frac(t * 1) == t`. This is S-D15 (1)'s finding,
    //     and it is why `FLAG_TILED` alone (without a count) would have been a
    //     mechanism that does nothing.
    for t in [0.0f32, 0.125, 0.5, 0.875] {
        assert_eq!(
            eval_tile_uv(sub, [t, t], tiled_flags(1, 1)),
            eval_tile_uv(sub, [t, t], 0),
            "a 1x1 tile count is the identity — `frac(t) == t` on [0, 1)"
        );
    }

    // (3) TILED (4, 2): the quad corner sweeps the sub-rect four times across and
    //     twice down, and NEVER leaves it.
    //     t = 0.25 -> frac(1.0) = 0.0  -> u = sub_min.x  (the second repeat's start)
    //     t = 0.375 -> frac(1.5) = 0.5 -> u = the sub-rect's midpoint
    let f = tiled_flags(4, 2);
    assert_eq!(eval_tile_uv(sub, [0.25, 0.5], f), [0.5, 0.25]);
    assert_eq!(eval_tile_uv(sub, [0.375, 0.75], f), [0.625, 0.375]);
    //     t = 0.9375 -> x: frac(3.75)  = 0.75  -> u = 0.5  + 0.75  * 0.25 = 0.6875
    //                   y: frac(1.875) = 0.875 -> v = 0.25 + 0.875 * 0.25 = 0.46875
    //     (the two axes carry DIFFERENT counts, so they land on different fractions of
    //     their own repeat — which is the point of the pair being separate fields)
    assert_eq!(eval_tile_uv(sub, [0.9375, 0.9375], f), [0.6875, 0.46875]);

    // (4) The CONTAINMENT property itself, swept — the assertion G5-8 makes on the
    //     GPU with a palette census, made here directly on the arithmetic for every
    //     count the field can hold.
    for tx in [1u32, 2, 3, 7, 63, 127] {
        for i in 0..64u32 {
            let t = i as f32 / 64.0;
            let got = eval_tile_uv(sub, [t, t], tiled_flags(tx, tx));
            assert!(
                got[0] >= sub[0] && got[0] < sub[2] && got[1] >= sub[1] && got[1] < sub[3],
                "tiles={tx} t={t}: the sample must stay INSIDE the sub-rect — got {got:?}, \
                 sub-rect {sub:?}. `frac` wraps the PARAMETER, so no count can escape"
            );
        }
    }

    // (5) The two count fields are read from DIFFERENT bits: an X count must not
    //     move the Y sample. Pinned because the two shifts differ by one word and
    //     a transposed pair is invisible to any symmetric input.
    assert_eq!(
        eval_tile_uv(sub, [0.25, 0.25], tiled_flags(4, 1)),
        [0.5, 0.3125],
        "tiles = (4, 1): x wraps at t = 0.25, y does not"
    );
    assert_eq!(
        eval_tile_uv(sub, [0.25, 0.25], tiled_flags(1, 4)),
        [0.5625, 0.25],
        "tiles = (1, 4): y wraps at t = 0.25, x does not"
    );
}

// ---- The Emit span pins (the crate-local half of G1-1) -----------------------------------

/// Every UI leaf's FULL emitted span, pinned as a literal — a wrong spelling, a lost paren or
/// a lost temp fails HERE, on any host, before `ui_rect_edsl_sync` (which needs the committed
/// shader beside it) ever runs. The load-bearing characters are called out per span.
#[test]
fn ui_leaf_spans_match_committed_shape() {
    // The `(1.0 / 255.0)` parens are LOAD-BEARING: without them `float4(...) * 1.0 / 255.0`
    // DIVIDES the vector — a different expression and a different `.spv`.
    assert_eq!(
        emit::emit_hlsl_ui_unpack_rgba8(),
        "    return float4((float)(c & 255u), (float)(c >> 8u & 255u), \
         (float)(c >> 16u & 255u), (float)(c >> 24u & 255u)) * (1.0 / 255.0);\n"
    );
    assert_eq!(
        emit::emit_hlsl_ui_sd_rounded_box(),
        "    float2 rx = (p.x > 0.0) ? r.yz : r.xw;\n\
         \x20   float rr = (p.y > 0.0) ? rx.y : rx.x;\n\
         \x20   float2 q = abs(p) - half_size + rr;\n\
         \x20   return min(max(q.x, q.y), 0.0) + length(max(q, 0.0)) - rr;\n"
    );
    assert_eq!(
        emit::emit_hlsl_ui_clip_coverage(),
        "    float2 inside_min = smoothstep(clip.xy - fw, clip.xy + fw, pos);\n\
         \x20   float2 inside_max = smoothstep(clip.zw - fw, clip.zw + fw, pos);\n\
         \x20   float2 cov = inside_min * (1.0 - inside_max);\n\
         \x20   return cov.x * cov.y;\n"
    );
    assert_eq!(
        emit::emit_hlsl_ui_median3(),
        "    return max(min(r, g), min(max(r, g), b));\n"
    );
    // The atlas-UBO globals spell VERBATIM (`g_atlas_ubo.px_range`) — the leaf's only
    // Eval-unreachable span (fwidth), pinned here textually instead.
    assert_eq!(
        emit::emit_hlsl_ui_screen_px_range(),
        "    float2 unit_range = g_atlas_ubo.px_range / g_atlas_ubo.atlas_size;\n\
         \x20   float2 screen_tex_sz = 1.0 / fwidth(uv);\n\
         \x20   return max(0.5 * dot(unit_range, screen_tex_sz), 1.0);\n"
    );
    // UI-ADVANCED S5. The load-bearing characters here are the SYMBOLS: `FLAG_TILED`,
    // `UI_TILE_X_SHIFT`, `UI_TILE_Y_SHIFT` and `UI_TILE_MASK` are emitted as names, not as
    // `32u` / `6u` / `13u` / `0x7Fu`, so the leaf and the generated `ui_flag_consts` span
    // cannot drift into shifting different bits (S-D2 / S-D10). And `tiled` is a TEMP
    // rather than nested into the comparison: `>` binds tighter than `&` in HLSL, so
    // `flags & FLAG_TILED > 0u` would parse as `flags & (FLAG_TILED > 0u)`.
    assert_eq!(
        emit::emit_hlsl_ui_tile_uv(),
        "    uint tiled = flags & FLAG_TILED;\n\
         \x20   uint tx = flags >> UI_TILE_X_SHIFT & UI_TILE_MASK;\n\
         \x20   uint ty = flags >> UI_TILE_Y_SHIFT & UI_TILE_MASK;\n\
         \x20   float2 tiles = float2((float)tx, (float)ty);\n\
         \x20   float2 t = (tiled > 0u) ? frac(local_uv * tiles) : local_uv;\n\
         \x20   return lerp(uv.xy, uv.zw, t);\n"
    );
    // The `(1.0 - src.a)` parens are load-bearing for the same reason as the unpack's.
    assert_eq!(
        emit::emit_hlsl_ui_premultiplied_over(),
        "    float4 src = bc * border_cov;\n\
         \x20   float4 dst = fill * inner_cov;\n\
         \x20   return src + dst * (1.0 - src.a);\n"
    );
}
