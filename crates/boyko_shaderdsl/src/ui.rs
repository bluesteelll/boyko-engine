//! The **UI-rect fragment leaves** (`docs/UI-PLAN-SPRITES.md` rung S1 — architecture D30): the
//! SEVEN generic `C: Cf` bodies `boyko_render/shaders/ui_rect.fs.hlsl` splices its math out of
//! (six at S1; [`ui_tile_uv_body`] joins them at S5).
//!
//! This module is the first `boyko_ui`-side consumer of the eDSL, and the first `float2`/`float4`
//! VALUE math in the crate ([`crate::cf::Cf`]'s UI-ADVANCED S1 facet section). Every leaf is
//! authored ONCE and instantiated twice:
//!
//! - `<EvalCf>` — the CPU oracle (real `f32` ops), pinned by `tests/ui_leaves.rs` against
//!   hand-computed constant tables (rung gate G1-3);
//! - `<EmitCf>` — the HLSL recorder, printed by `crate::emit::emit_hlsl_ui_*` and spliced
//!   between the `// === GENERATED <name> BEGIN/END ===` sentinels of the two files
//!   `boyko_shaderdsl/src/bin/emit_ui.rs` owns.
//!
//! # The family, and its precision carve-outs
//!
//! [`ui_median3_body`] and [`ui_premultiplied_over_body`] are pure min/max/mul/add — exact on
//! both backends modulo FMA contraction (the crate's standing float carve-out).
//! [`ui_sd_rounded_box_body`] adds `length` (sqrt-family precision), so its oracle table pins
//! points whose radicand is an exact square. [`ui_clip_coverage_body`] adds `smoothstep`
//! (a divide inside the spec polynomial), so its table pins the SATURATED ends, where the
//! result is exactly 0 or 1. [`ui_unpack_rgba8_body`] is integer unpack + one multiply.
//! [`ui_tile_uv_body`] is an integer field decode + `frac` + `lerp` — the decode is integer,
//! `frac` is one exact subtract, and `lerp` is mul/add under the standing FMA carve-out — so it
//! IS oracle-swept, on BOTH arms (the tiled one and the identity one).
//! [`ui_screen_px_range_body`] spells `fwidth` — a device derivative with NO host semantics —
//! so it is deliberately **not oracle-swept**: its Eval instantiation panics loudly at the
//! [`Cf::vec2_fwidth`] arm (the honest-panic discipline), and its gate is the byte-identity
//! pair (`ui_rect_edsl_sync` / `ui_rect_spv_sync`) instead.
//!
//! # Divides
//!
//! Two leaves spell a divide (`ui_unpack_rgba8`'s `(1.0 / 255.0)` literal fold and
//! `ui_screen_px_range`'s two broadcast divides). Per the house rule (`OpFDiv` carries
//! 2.5 ULP), a divide is never part of a bit-exact oracle contract: the unpack's divide is a
//! compile-time constant fold (both backends fold the same two literals), and the range leaf
//! is not oracle-swept at all (above).

use crate::cf::{Cf, Flow};
use crate::scalar::FieldScalar;

// ---- UI-ADVANCED S5: the tile-field generator inputs (S-D2's bit budget) -------------------
//
// These four numbers mirror `boyko_render::ui::instance`'s `FLAG_TILED` / `UI_TILE_X_SHIFT` /
// `UI_TILE_Y_SHIFT` / `UI_TILE_BITS`, and they are PINNED to those host constants by
// `boyko_render/tests/ui_rect_edsl_sync.rs` — the same S-D10 discipline that pins
// `emit_ui.rs`'s `UI_INSTANCE_LAYOUT` literals. They are `pub` precisely so that test can
// name them; nothing else reads them outside [`ui_tile_uv_body`].

/// The bit index of `FLAG_TILED` in `UiInstance.flags` (bit 5 — S-D15).
pub const UI_TILE_FLAG_BIT: u32 = 5;
/// The LOW bit of the X repeat-count field in `UiInstance.flags` (bits 6..=12).
pub const UI_TILE_X_SHIFT: u32 = 6;
/// The LOW bit of the Y repeat-count field in `UiInstance.flags` (bits 13..=19).
pub const UI_TILE_Y_SHIFT: u32 = 13;
/// The WIDTH in bits of each repeat-count field (7 — counts `1..=127`, S-D15).
pub const UI_TILE_BITS: u32 = 7;

/// `FLAG_TILED` as a mask — DERIVED from [`UI_TILE_FLAG_BIT`], never spelled.
const UI_TILE_FLAG: u32 = 1 << UI_TILE_FLAG_BIT;
/// The repeat-count field mask AFTER its shift (`0x7F`) — DERIVED from [`UI_TILE_BITS`].
const UI_TILE_MASK: u32 = (1 << UI_TILE_BITS) - 1;

/// The `> 0u` comparand of the `FLAG_TILED` test (`(flags & FLAG_TILED) > 0u`) — a bare
/// `uint` literal, the [`crate::cf::Cf::uint_lit`] spelling.
const UINT_ZERO: u32 = 0;

// The two fields and the flag must fit the fifteen bits S-D2 left free (5..=19) without
// overlapping each other or the bindless slot field at bit 20. Fifteen free, fifteen spent:
// this assert is what says the budget is EXHAUSTED at the site that spends it.
const _: () = assert!(UI_TILE_X_SHIFT == UI_TILE_FLAG_BIT + 1);
const _: () = assert!(UI_TILE_Y_SHIFT == UI_TILE_X_SHIFT + UI_TILE_BITS);
const _: () = assert!(UI_TILE_Y_SHIFT + UI_TILE_BITS == 20);

// ---- Constants mirrored by the generated HLSL as bare literals ----------------------------

/// The RGBA8 byte mask (`255u`), spelled in decimal like every other `uint` literal the
/// printer emits.
const BYTE_MASK: u32 = 255;

/// The RGBA8 channel shifts — G at 8, B at 16, A at 24 (byte0 = R, little-endian packed).
const G_SHIFT: u32 = 8;
/// See [`G_SHIFT`].
const B_SHIFT: u32 = 16;
/// See [`G_SHIFT`].
const A_SHIFT: u32 = 24;

/// The byte → unit-interval normalization, spelled as the literal fold `(1.0 / 255.0)` (the
/// committed text), NOT a pre-folded decimal — both backends fold the same two literals, so the
/// Eval product and the DXC constant agree by construction.
const UNORM8_NUM: f32 = 1.0;
/// See [`UNORM8_NUM`].
const UNORM8_DEN: f32 = 255.0;

/// The quadrant pivot — `p.x > 0.0` / `p.y > 0.0` select the per-corner radius. Mirrors the
/// GPU's literal `0.0`.
const QUADRANT_PIVOT: f32 = 0.0;

/// The outside-distance clamp — `min(max(q.x, q.y), 0.0)` / `max(q, 0.0)`. Mirrors the GPU's
/// literal `0.0`.
const SD_ZERO: f32 = 0.0;

/// The `1.0` of the clip coverage's `(1.0 - inside_max)` complement and the premultiplied
/// over's `(1.0 - src.a)` term.
const ONE: f32 = 1.0;

/// The MSDF screen-px-range fold's `0.5 * dot(...)` factor (the two-axis average).
const RANGE_HALF: f32 = 0.5;

/// The MSDF screen-px-range floor — `max(..., 1.0)`: `fwidth(uv) == 0` on a flat run gives an
/// infinite `screen_tex_sz`; the floor keeps the coverage math finite (the committed NaN/Inf
/// guard).
const RANGE_FLOOR: f32 = 1.0;

// ---- The six S1 leaves + the S5 tile leaf --------------------------------------------------

/// **`ui_unpack_rgba8`** — unpacks a premultiplied RGBA8 word (byte0 = R .. byte3 = A) to a
/// `float4` in `[0, 1]`.
///
/// ```text
/// return float4((float)(c & 255u), (float)(c >> 8u & 255u), (float)(c >> 16u & 255u),
///               (float)(c >> 24u & 255u)) * (1.0 / 255.0);
/// ```
///
/// The top byte's `& 255u` is redundant (`c >> 24u` has no higher bits) and is kept
/// deliberately: the four lanes stay one shape, and the committed `.spv` was compiled from the
/// four-mask form. `>> & ` needs no parens (`>>` binds tighter than `&` — the
/// `pack_material_id_ba` precedent).
#[inline]
pub fn ui_unpack_rgba8_body<C: Cf>(c: C::Uint, ret_out: &C::RetCellV4) -> Flow {
    let lane = |shift: u32| -> C::Scalar {
        let shifted = if shift == 0 {
            c
        } else {
            C::shr_u(c, C::uint_lit(shift))
        };
        C::float_from_uint(C::and_u(shifted, C::uint_lit(BYTE_MASK)))
    };
    C::ret_vec4(
        ret_out,
        C::vec4_mul_scalar(
            C::vec4_from_scalars(lane(0), lane(G_SHIFT), lane(B_SHIFT), lane(A_SHIFT)),
            C::Scalar::lit(UNORM8_NUM).div(C::Scalar::lit(UNORM8_DEN)),
        ),
    )
}

/// **`ui_sd_rounded_box`** — the Quilez/Bevy per-corner rounded-box SDF. `p` is rect-centred;
/// `half_size` is half the rect; `r` is `(tl, tr, br, bl)`. Selects the corner radius by the
/// quadrant of `p`.
///
/// ```text
/// float2 rx = (p.x > 0.0) ? r.yz : r.xw;
/// float rr = (p.y > 0.0) ? rx.y : rx.x;
/// float2 q = abs(p) - half_size + rr;
/// return min(max(q.x, q.y), 0.0) + length(max(q, 0.0)) - rr;
/// ```
///
/// The `x`-select picks the side pair (`(tr, br)` right, `(tl, bl)` left); the `y`-select then
/// picks top vs bottom within it. Inside the box `max(q, 0.0)` is the zero vector, so the
/// `length` term vanishes and the oracle is exact; at a corner the radicand is a sum of exact
/// squares at the table's pinned points (sqrt-family precision, module doc).
#[inline]
pub fn ui_sd_rounded_box_body<C: Cf>(
    p: C::Vec2f,
    half_size: C::Vec2f,
    r: C::Vec4f,
    ret_out: &C::RetCellF,
) -> Flow {
    // float2 rx = (p.x > 0.0) ? r.yz : r.xw;
    let rx = C::temp_vec2(
        "rx",
        C::select_vec2(
            C::vec2_x(p).gt(C::Scalar::lit(QUADRANT_PIVOT)),
            C::vec4_yz(r),
            C::vec4_xw(r),
        ),
    );
    // float rr = (p.y > 0.0) ? rx.y : rx.x;
    let rr = C::temp_float(
        "rr",
        C::Scalar::select(
            C::vec2_y(p).gt(C::Scalar::lit(QUADRANT_PIVOT)),
            C::vec2_y(rx),
            C::vec2_x(rx),
        ),
    );
    // float2 q = abs(p) - half_size + rr;
    let q = C::temp_vec2(
        "q",
        C::vec2_add_scalar(C::vec2_sub(C::vec2_abs(p), half_size), rr),
    );
    // return min(max(q.x, q.y), 0.0) + length(max(q, 0.0)) - rr;
    C::ret_f(
        ret_out,
        C::vec2_x(q)
            .max(C::vec2_y(q))
            .min(C::Scalar::lit(SD_ZERO))
            .add(C::vec2_length(C::vec2_max_scalar(q, C::Scalar::lit(SD_ZERO))))
            .sub(rr),
    )
}

/// **`ui_clip_coverage`** — anti-aliased coverage of a finite clip AABB at physical-px `pos`:
/// 1 inside, 0 outside, a `~fw`-wide AA band at each edge. `clip` is `(min.xy, max.xy)`,
/// always finite when `FLAG_CLIP_PRESENT` is set.
///
/// ```text
/// float2 inside_min = smoothstep(clip.xy - fw, clip.xy + fw, pos);
/// float2 inside_max = smoothstep(clip.zw - fw, clip.zw + fw, pos);
/// float2 cov = inside_min * (1.0 - inside_max);
/// return cov.x * cov.y;
/// ```
///
/// Per-axis: `inside_min` rises past the min edge, `1 - inside_max` falls past the max edge,
/// and the product is the axis's band-limited indicator; the `x * y` fold is the separable
/// AABB coverage. Exact at the saturated ends (module doc), which is where the oracle pins.
#[inline]
pub fn ui_clip_coverage_body<C: Cf>(
    pos: C::Vec2f,
    clip: C::Vec4f,
    fw: C::Scalar,
    ret_out: &C::RetCellF,
) -> Flow {
    // float2 inside_min = smoothstep(clip.xy - fw, clip.xy + fw, pos);
    let inside_min = C::temp_vec2(
        "inside_min",
        C::vec2_smoothstep(
            C::vec2_sub_scalar(C::vec4_xy(clip), fw),
            C::vec2_add_scalar(C::vec4_xy(clip), fw),
            pos,
        ),
    );
    // float2 inside_max = smoothstep(clip.zw - fw, clip.zw + fw, pos);
    let inside_max = C::temp_vec2(
        "inside_max",
        C::vec2_smoothstep(
            C::vec2_sub_scalar(C::vec4_zw(clip), fw),
            C::vec2_add_scalar(C::vec4_zw(clip), fw),
            pos,
        ),
    );
    // float2 cov = inside_min * (1.0 - inside_max);
    let cov = C::temp_vec2(
        "cov",
        C::vec2_mul(
            inside_min,
            C::vec2_rsub_scalar(C::Scalar::lit(ONE), inside_max),
        ),
    );
    // return cov.x * cov.y;
    C::ret_f(ret_out, C::vec2_x(cov).mul(C::vec2_y(cov)))
}

/// **`ui_median3`** — the canonical Chlumsky MSDF median: the per-channel median recovers the
/// sharp signed distance (the three channels disagree only across a true edge).
///
/// ```text
/// return max(min(r, g), min(max(r, g), b));
/// ```
///
/// Pure min/max — bit-exact on both backends for every ordering (rung gate G1-3 pins all
/// three).
#[inline]
pub fn ui_median3_body<C: Cf>(
    r: C::Scalar,
    g: C::Scalar,
    b: C::Scalar,
    ret_out: &C::RetCellF,
) -> Flow {
    C::ret_f(ret_out, r.min(g).max(r.max(g).min(b)))
}

/// **`ui_screen_px_range`** — converts the baked TEXEL distance range into a SCREEN-px range
/// at this fragment, so the MSDF AA band is ~1 device px regardless of glyph scale.
///
/// ```text
/// float2 unit_range = g_atlas_ubo.px_range / g_atlas_ubo.atlas_size;
/// float2 screen_tex_sz = 1.0 / fwidth(uv);
/// return max(0.5 * dot(unit_range, screen_tex_sz), 1.0);
/// ```
///
/// `px_range` / `atlas_size` are the atlas UBO fields — the generated span spells the global
/// reads verbatim (`g_atlas_ubo.px_range`), so the leaf's "parameters" are the printer's input
/// names, not HLSL signature parameters. `fwidth(uv) == 0` on a flat run gives an infinite
/// `screen_tex_sz`; `max(..., 1.0)` floors it (the committed NaN/Inf guard).
///
/// **Not oracle-swept**: `fwidth` has no host semantics ([`Cf::vec2_fwidth`]'s Eval arm is an
/// honest panic), so this leaf's gate is the byte-identity pair, not a table (module doc).
#[inline]
pub fn ui_screen_px_range_body<C: Cf>(
    px_range: C::Scalar,
    atlas_size: C::Vec2f,
    uv: C::Vec2f,
    ret_out: &C::RetCellF,
) -> Flow {
    // float2 unit_range = g_atlas_ubo.px_range / g_atlas_ubo.atlas_size;
    let unit_range = C::temp_vec2("unit_range", C::vec2_rdiv_scalar(px_range, atlas_size));
    // float2 screen_tex_sz = 1.0 / fwidth(uv);
    let screen_tex_sz = C::temp_vec2(
        "screen_tex_sz",
        C::vec2_rdiv_scalar(C::Scalar::lit(ONE), C::vec2_fwidth(uv)),
    );
    // return max(0.5 * dot(unit_range, screen_tex_sz), 1.0);
    C::ret_f(
        ret_out,
        C::Scalar::lit(RANGE_HALF)
            .mul(C::vec2_dot(unit_range, screen_tex_sz))
            .max(C::Scalar::lit(RANGE_FLOOR)),
    )
}

/// **`ui_tile_uv`** — the sprite branch's WHOLE UV computation (UI-ADVANCED S5, S-D15):
/// the untiled `lerp` into the record's UV sub-rect, or — under `FLAG_TILED` — the same
/// `lerp` with the quad corner first swept `tiles` times and wrapped back INSIDE the
/// sub-rect.
///
/// ```text
/// uint tiled = flags & FLAG_TILED;
/// uint tx = flags >> UI_TILE_X_SHIFT & UI_TILE_MASK;
/// uint ty = flags >> UI_TILE_Y_SHIFT & UI_TILE_MASK;
/// float2 tiles = float2((float)tx, (float)ty);
/// float2 t = (tiled > 0u) ? frac(local_uv * tiles) : local_uv;
/// return lerp(uv.xy, uv.zw, t);
/// ```
///
/// # `frac` INSIDE the sub-rect, and why the count cannot ride `uv`
///
/// `local_uv` is the `0..1` quad corner (`ui_rect.vs.hlsl`'s `o.local_uv = corner`), so
/// `frac(local_uv)` is the IDENTITY on every covered fragment — which is why the mechanism
/// S-D11 first ruled rendered exactly as `Stretch`. The repeat count is what makes the wrap
/// mean anything, and it rides `flags` rather than `uv`: all four `uv` floats are consumed
/// as `sub_min`/`sub_max`, so folding a count into them would sweep N whole SUB-RECTS —
/// under a sprite sheet, N whole neighbouring FRAMES, which is the bleed the gate G5-8
/// exists to forbid.
///
/// Because the wrap is `frac` of the *parameter* and the `lerp` is into the *sub-rect*, the
/// sample never leaves the sub-rect for any `tiles`. Under a sheet the sub-rect IS a frame,
/// so `Tile` + `UiSpriteSheet` is correct by construction rather than a forbidden pair.
///
/// # The untiled arm is the committed `lerp`, deliberately
///
/// It spells [`Cf::vec2_lerp`] (the `FMix` intrinsic), NOT `a + t * (b - a)`: the two round
/// differently and the six committed UI image pins were blessed against the intrinsic. See
/// [`Cf::vec2_lerp`]'s doc.
///
/// `FLAG_TILED` is set at pack ONLY when a repeat count exceeds 1, so a corner sub-quad
/// (always `1×1`) leaves the flag AND both fields zero and takes this leaf's untiled arm —
/// which is what makes a `Tile` corner byte-identical to its `Stretch` corner.
#[inline]
pub fn ui_tile_uv_body<C: Cf>(
    uv: C::Vec4f,
    local_uv: C::Vec2f,
    flags: C::Uint,
    ret_out: &C::RetCellV2,
) -> Flow {
    // uint tiled = flags & FLAG_TILED;
    //
    // MATERIALIZED rather than nested into the `> 0u` test: `>` binds TIGHTER than `&` in
    // HLSL, so `flags & FLAG_TILED > 0u` would parse as `flags & (FLAG_TILED > 0u)`. The
    // frozen `and_u` printer arm spells its operands un-wrapped (the `pack_material_id_ba`
    // byte-identity contract), so the temp is the correct spelling, not a stylistic one.
    let tiled = C::temp_uint(
        "tiled",
        C::and_u(flags, C::named_uint_val("FLAG_TILED", UI_TILE_FLAG)),
    );
    // uint tx = flags >> UI_TILE_X_SHIFT & UI_TILE_MASK;   (`>>` binds tighter than `&`)
    let tx = C::temp_uint(
        "tx",
        C::and_u(
            C::shr_u(flags, C::named_uint_val("UI_TILE_X_SHIFT", UI_TILE_X_SHIFT)),
            C::named_uint_val("UI_TILE_MASK", UI_TILE_MASK),
        ),
    );
    // uint ty = flags >> UI_TILE_Y_SHIFT & UI_TILE_MASK;
    let ty = C::temp_uint(
        "ty",
        C::and_u(
            C::shr_u(flags, C::named_uint_val("UI_TILE_Y_SHIFT", UI_TILE_Y_SHIFT)),
            C::named_uint_val("UI_TILE_MASK", UI_TILE_MASK),
        ),
    );
    // float2 tiles = float2((float)tx, (float)ty);
    let tiles = C::temp_vec2(
        "tiles",
        C::vec2_from_scalars(C::float_from_uint(tx), C::float_from_uint(ty)),
    );
    // float2 t = (tiled > 0u) ? frac(local_uv * tiles) : local_uv;
    let t = C::temp_vec2(
        "t",
        C::select_vec2(
            C::ugt(tiled, C::uint_lit(UINT_ZERO)),
            C::vec2_frac(C::vec2_mul(local_uv, tiles)),
            local_uv,
        ),
    );
    // return lerp(uv.xy, uv.zw, t);
    C::ret_vec2(ret_out, C::vec2_lerp(C::vec4_xy(uv), C::vec4_zw(uv), t))
}

/// **`ui_premultiplied_over`** — composites the border ring (premultiplied color `bc`,
/// area-weighted by `border_cov`) ON TOP of the fill restricted to the inner shape
/// (`fill * inner_cov`), in premultiplied space.
///
/// ```text
/// float4 src = bc * border_cov;
/// float4 dst = fill * inner_cov;
/// return src + dst * (1.0 - src.a);
/// ```
///
/// The premultiplied "over": where the ring is translucent (`bc.a < 1`), the fill shows
/// through via the `(1 - src.a)` term — a `lerp` of two premultiplied colors would mis-weight
/// that fall-through (the committed shader's own comment). Pure mul/add — exact on both
/// backends modulo FMA contraction.
#[inline]
pub fn ui_premultiplied_over_body<C: Cf>(
    bc: C::Vec4f,
    border_cov: C::Scalar,
    fill: C::Vec4f,
    inner_cov: C::Scalar,
    ret_out: &C::RetCellV4,
) -> Flow {
    // float4 src = bc * border_cov;
    let src = C::temp_vec4("src", C::vec4_mul_scalar(bc, border_cov));
    // float4 dst = fill * inner_cov;
    let dst = C::temp_vec4("dst", C::vec4_mul_scalar(fill, inner_cov));
    // return src + dst * (1.0 - src.a);
    C::ret_vec4(
        ret_out,
        C::vec4_add(
            src,
            C::vec4_mul_scalar(dst, C::Scalar::lit(ONE).sub(C::vec4_alpha(src))),
        ),
    )
}
