//! The G-buffer MATERIAL-ID PACKER leaf (Track B Increment G1: the FIRST `float2`-returning leaf —
//! it lands the minimal `float2` axis + the bitwise `uint` `&`/`>>`).
//!
//! `pack_material_id_ba` (`sdf_gbuffer_composite.hlsl:519`) packs a 16-bit `uint id` into the B + A
//! channels of an RGBA8 G-buffer texel: the LOW byte (`id & 255u`) → B, the HIGH byte (`id >> 8u &
//! 255u`) → A, each as a normalized `[0,1]` UNORM (`byte / 255.0`). The resolve reconstructs `id =
//! round(b*255) | (round(a*255) << 8)`. This module authors the BODY ONCE over the control-flow axis
//! `C: Cf`, between the `// === GENERATED pack_material_id_ba BEGIN/END ===` sentinels INSIDE
//! `pack_material_id_ba`; the hand-written signature `float2 pack_material_id_ba(uint id) {` + the
//! closing `}` stay un-generated (framing (b)).
//!
//! # The `float2` return + the bitwise `&`/`>>` — the two new facets
//!
//! Unlike the prior return-bearing leaves, `pack_material_id_ba` returns a `float2`
//! ([`Cf::ret_vec2`] with [`Cf::vec2_from_scalars`]) — the FIRST `float2`-returning leaf, landing the
//! minimal `float2` axis. The byte split uses the bitwise `uint` AND / shift ([`Cf::and_u`] /
//! [`Cf::shr_u`]) — the two DEAD `Node::And` / `Node::Shr` nodes whose printer arms (the `&` / `>>`
//! spellings, unparenthesized) already existed.
//!
//! The named `lo`/`hi` `uint` temps reuse [`Cf::temp_uint`]; the `255u`/`8u` literals reuse
//! [`Cf::uint_lit`]; the `(float)lo` cast reuses [`Cf::float_from_uint`]; the `/ 255.0` divide is the
//! scalar [`crate::scalar::FieldScalar::div`].
//!
//! # The canonical SPELLING (proven byte-neutral spikes)
//!
//! The emit spells `uint lo = id & 255u;` / `uint hi = id >> 8u & 255u;` — `255u` (NOT the committed
//! `0xFFu`; the hex→decimal change is DXC-fold-neutral) and UNPARENTHESIZED (the committed `(id >> 8)
//! & 0xFFu`'s redundant parens removal is byte-identical, proven). The `id >> 8u & 255u` precedence
//! is correct unparenthesized (`>>` binds tighter than `&`). The committed source is re-spliced to
//! match the emit; the `.comp.spv` stays byte-identical.
//!
//! # Instantiation (the established control-axis discipline)
//!
//! - `<EvalCf>` — the CPU oracle (real `&`/`>>`/`(float)`/`/` + a body-local `Cell<[f32; 2]>`). The
//!   eval sweep reproduces the committed byte split to-bits against a host mirror transcribing the
//!   committed body verbatim.
//! - `<EmitCf>` — the HLSL recorder; the printer ([`crate::emit::emit_hlsl_pack_material_id_ba`])
//!   walks the STMT IR into the body span (byte-identical to the committed `.comp.spv`, proven by the
//!   cmp-`.spv`).
//!
//! # `R1` (no compound-assign)
//!
//! Every value is a fresh `temp_uint` (`lo` / `hi`); the return is a fresh `float2` ctor — no `+=`
//! form — so there is no R1 concern.

use crate::cf::{Cf, Flow};
use crate::scalar::FieldScalar;

/// The UNORM divisor — `byte / 255.0` maps a `[0, 255]` byte to a `[0, 1]` UNORM. Mirrors the GPU's
/// literal `255.0` (`sdf_gbuffer_composite.hlsl:522`).
const UNORM_DIVISOR: f32 = 255.0;

/// The low-byte mask — `id & 255u` keeps the low 8 bits. Mirrors the GPU's `0xFFu` / `255u`.
const BYTE_MASK: u32 = 255;

/// The high-byte shift — `id >> 8u` moves the high 8 bits into the low position. Mirrors the GPU's
/// `8` / `8u`.
const HIGH_BYTE_SHIFT: u32 = 8;

/// Packs a 16-bit `uint` material `id` into a `float2` of normalized `[0,1]` UNORM bytes (low byte
/// → `.x`/B, high byte → `.y`/A), depositing the pair into `ret_out`. Authored ONCE over the
/// control-flow axis `C`. Mirrors the GPU `pack_material_id_ba`'s L520-522 body statement-for-
/// statement (the hand-written signature + closing brace stay un-generated).
///
/// On Emit the byte split records `uint lo = id & 255u;` / `uint hi = id >> 8u & 255u;` (named `uint`
/// temps) and the return records `return float2((float)lo / 255.0, (float)hi / 255.0);`. On Eval the
/// body runs the real `&`/`>>`/`(float)`/`/` and deposits the `[lo/255, hi/255]` pair into the cell.
#[inline]
pub fn pack_material_id_ba_body<C: Cf>(id: C::Uint, ret_out: &C::RetCellV2) -> Flow {
    // uint lo = id & 255u;  — the low byte (a NAMED `uint` temp).
    let lo = C::temp_uint("lo", C::and_u(id, C::uint_lit(BYTE_MASK)));
    // uint hi = id >> 8u & 255u;  — the high byte (`>>` binds tighter than `&`, so unparenthesized).
    let hi = C::temp_uint(
        "hi",
        C::and_u(C::shr_u(id, C::uint_lit(HIGH_BYTE_SHIFT)), C::uint_lit(BYTE_MASK)),
    );
    // return float2((float)lo / 255.0, (float)hi / 255.0);  — each byte normalized to a [0,1] UNORM.
    C::ret_vec2(
        ret_out,
        C::vec2_from_scalars(
            C::float_from_uint(lo).div(C::Scalar::lit(UNORM_DIVISOR)),
            C::float_from_uint(hi).div(C::Scalar::lit(UNORM_DIVISOR)),
        ),
    )
}
