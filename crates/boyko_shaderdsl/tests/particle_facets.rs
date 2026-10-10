//! Rung E — the particle-leaf prerequisite FACET PINS (`feature = "emit"`).
//!
//! One test per facet family (docs/PARTICLES-PLAN.md rung E). Each pins BOTH halves of the
//! dual instantiation of the SAME generic probe body ([`boyko_shaderdsl::particle_facets`]):
//!
//! - the **Eval** value — `<EvalCf>` over host `u32`/`f32` ops, checked against a hand-computed
//!   constant (never against a re-run of the implementation);
//! - the **Emit** text — `<EmitCf>` through the HLSL printer, checked as the FULL span, so a
//!   wrong spelling, a wrong type token, a lost temp or a lost/added paren all fail.
//!
//! Pinning both is what makes the pair meaningful: the Eval half alone cannot see a printer
//! that spells `&` where the body asked for `|`, and the Emit half alone cannot see an Eval arm
//! that shifts the wrong way. The E4 trig test is the sharpest illustration — its Eval oracle
//! (`sin² + cos² == 1`) is symmetric under swapping the two hooks, and only the text catches it.
//!
//! Gated on `feature = "emit"` (the printer surface is `#[cfg(feature = "emit")]`); the
//! backend-independent half-precision conversion vectors live in the un-gated
//! `tests/half_ieee754.rs`.

#![cfg(feature = "emit")]

use core::cell::Cell;

use boyko_shaderdsl::cf::{Cf, EvalCf};
use boyko_shaderdsl::particle_facets as probes;

// ---- Eval drivers: run a probe over `EvalCf` and read its ret cell --------------------

fn eval_e1_bit_mix(state: u32) -> u32 {
    let cell: Cell<u32> = Cell::new(0);
    let _ = probes::e1_bit_mix_body::<EvalCf>(state, &cell);
    cell.get()
}

fn eval_e2_pack_half2(x: f32, y: f32) -> u32 {
    let cell: Cell<u32> = Cell::new(0);
    let _ = probes::e2_pack_half2_body::<EvalCf>(x, y, &cell);
    cell.get()
}

fn eval_e2_unpack_half2(packed: u32) -> f32 {
    let cell: Cell<f32> = Cell::new(0.0);
    let _ = probes::e2_unpack_half2_body::<EvalCf>(packed, &cell);
    cell.get()
}

fn eval_e2_bitcast_sign_flip(x: f32) -> f32 {
    let cell: Cell<f32> = Cell::new(0.0);
    let _ = probes::e2_bitcast_sign_flip_body::<EvalCf>(x, &cell);
    cell.get()
}

fn eval_e3_dot(v: [f32; 3], n: [f32; 3]) -> f32 {
    let cell: Cell<f32> = Cell::new(0.0);
    let _ = probes::e3_dot_body::<EvalCf>(v, n, &cell);
    cell.get()
}

fn eval_e4_trig(theta: f32) -> f32 {
    let cell: Cell<f32> = Cell::new(0.0);
    let _ = probes::e4_trig_body::<EvalCf>(theta, &cell);
    cell.get()
}

fn eval_e5_renorm(c: f32, s: f32) -> f32 {
    let cell: Cell<f32> = Cell::new(0.0);
    let _ = probes::e5_renorm_body::<EvalCf>(c, s, &cell);
    cell.get()
}

// ---- E1: the bitwise / shift `uint` facet ---------------------------------------------

#[test]
fn e1_bitwise_shift_eval_and_emit() {
    // Eval. Hand-computed for `state = 0x1234_5678`:
    //   rot  = 0x1234_5678 >> 28          = 0x1
    //   word = 0x1234_5678 ^ 0x8468_ACF0  = 0x98FB_5678   (0x1234_5678 << 13 = 0x8468_ACF0)
    //   tail = 0x98FB_5678 & 0xFFFF       = 0x5678
    //   res  = 0x5678 | 0x1               = 0x5679
    // The `| rot` and the `& mask` both CHANGE the value here, so an `|` that returned its
    // left operand (or an `&` that returned its right) would fail.
    assert_eq!(eval_e1_bit_mix(0x1234_5678), 0x5679);
    // A second input whose rotation bits do not already sit in the tail:
    //   rot = 0xA, word = 0x1111_05A5, tail = 0x05A5, res = 0x05A5 | 0xA = 0x05AF.
    assert_eq!(eval_e1_bit_mix(0xA5A5_A5A5), 0x05AF);

    // Emit. The parenthesized `(state << 13u)` is the load-bearing part: `<<` binds LOOSER
    // than `^`, so the printer must wrap a shift nested in a bitwise parent.
    let g = boyko_shaderdsl::emit::emit_hlsl_e1_bit_mix().replace("\r\n", "\n");
    assert_eq!(
        g,
        "    uint rot = state >> 28u;\n\
         \x20   uint word = state ^ (state << 13u);\n\
         \x20   uint tail = word & 65535u;\n\
         \x20   return tail | rot;\n",
        "the E1 span must spell all five `uint` bit ops with the shift parenthesized:\n{g}"
    );
}

#[test]
fn e1_shift_amount_masks_to_five_bits() {
    // MEASURED (dxc -T cs_6_0 -spirv, Vulkan SDK 1.4.350): `a << b` lowers to
    // `OpBitwiseAnd %b %uint_31` + `OpShiftLeftLogical`, so the GPU shifts by `b & 31`.
    // A shift of 32 is therefore a shift of ZERO, not a zeroed result — the Eval arm's
    // `wrapping_shl` must reproduce that rather than panicking or saturating.
    assert_eq!(
        EvalCf::ushl(1, 32),
        1,
        "a shift of 32 must mask to a shift of 0"
    );
    assert_eq!(
        EvalCf::ushl(1, 33),
        2,
        "a shift of 33 must mask to a shift of 1"
    );
    assert_eq!(EvalCf::ushl(0xFFFF_FFFF, 31), 0x8000_0000);
    // The in-range ops are the plain host semantics.
    assert_eq!(EvalCf::ushl(0x0000_00FF, 8), 0x0000_FF00);
    assert_eq!(EvalCf::uxor(0xF0F0_F0F0, 0x0FF0_0FF0), 0xFF00_FF00);
    assert_eq!(EvalCf::uor(0xF000_000F, 0x0F00_00F0), 0xFF00_00FF);
}

// ---- E2: the bit-cast + half-precision facet ------------------------------------------

#[test]
fn e2_pack_half2_eval_and_emit() {
    // Eval. `1.0` is binary16 `0x3C00`, `-2.0` is `0xC000`; the pair packs low-then-high.
    assert_eq!(eval_e2_pack_half2(1.0, -2.0), 0xC000_3C00);
    // The narrow is a real conversion, not a truncation: 65520.0 is the exact tie between
    // HALF_MAX and 2^16, and round-to-nearest-EVEN takes it to +Inf (`0x7C00`).
    assert_eq!(eval_e2_pack_half2(65520.0, 0.0), 0x0000_7C00);

    let g = boyko_shaderdsl::emit::emit_hlsl_e2_pack_half2().replace("\r\n", "\n");
    assert_eq!(
        g,
        "    uint lo = f32tof16(x);\n\
         \x20   uint hi = f32tof16(y);\n\
         \x20   return lo | (hi << 16u);\n",
        "the E2 encode span must spell `f32tof16` twice and wrap the shift under `|`:\n{g}"
    );
}

#[test]
fn e2_unpack_half2_eval_and_emit() {
    // Eval: the exact inverse of the encode pin above — `0xC000_3C00` is (1.0, -2.0), summing
    // to -1.0 EXACTLY (both halves are representable, so no tolerance is warranted).
    assert_eq!(eval_e2_unpack_half2(0xC000_3C00), -1.0);
    // The low half is masked, the high half shifted: a swapped pair sums the same, so the
    // second vector uses two values whose sum identifies each side — `0x3800` is 0.5 in the
    // low half, `0x0001` is the smallest subnormal (2^-24) in the high half, and their sum is
    // exactly representable in binary32 (it lands on 0.5's last significand bit).
    assert_eq!(eval_e2_unpack_half2(0x0001_3800), 0.5 + 1.0 / 16_777_216.0);

    let g = boyko_shaderdsl::emit::emit_hlsl_e2_unpack_half2().replace("\r\n", "\n");
    assert_eq!(
        g,
        "    float lo = f16tof32(packed & 65535u);\n\
         \x20   float hi = f16tof32(packed >> 16u);\n\
         \x20   return lo + hi;\n",
        "the E2 decode span must spell `f16tof32` over an UN-wrapped call argument:\n{g}"
    );
}

#[test]
fn e2_bitcast_sign_flip_eval_and_emit() {
    // Eval: a bit-cast round trip is EXACT, which is the whole point of `asuint`/`asfloat`
    // being distinct from the numeric casts.
    assert_eq!(eval_e2_bitcast_sign_flip(3.5), -3.5);
    assert_eq!(eval_e2_bitcast_sign_flip(-3.5), 3.5);
    // The sign of a zero survives (a numeric cast would have lost it).
    assert_eq!(
        eval_e2_bitcast_sign_flip(0.0).to_bits(),
        (-0.0f32).to_bits()
    );
    assert_eq!(eval_e2_bitcast_sign_flip(-0.0).to_bits(), 0.0f32.to_bits());
    // So does a NaN payload — `asfloat` is total over every `u32`.
    let nan = f32::from_bits(0x7FC0_1234);
    assert_eq!(eval_e2_bitcast_sign_flip(nan).to_bits(), 0xFFC0_1234);

    let g = boyko_shaderdsl::emit::emit_hlsl_e2_bitcast_sign_flip().replace("\r\n", "\n");
    assert_eq!(
        g,
        "    uint bits = asuint(x);\n\
         \x20   uint flipped = bits ^ 2147483648u;\n\
         \x20   return asfloat(flipped);\n",
        "the E2 bit-cast span must spell `asuint`/`asfloat`, NOT the numeric casts:\n{g}"
    );
}

// ---- E3: the `dot` intrinsic facet -----------------------------------------------------

#[test]
fn e3_dot_eval_and_emit() {
    // Eval: dot([1,2,3], [0.5,-1,2]) = 0.5 - 2 + 6 = 4.5; dot(v,v) = 1 + 4 + 9 = 14;
    // 4.5 * 14 = 63. Every term is exactly representable, so the product is exact.
    assert_eq!(eval_e3_dot([1.0, 2.0, 3.0], [0.5, -1.0, 2.0]), 63.0);
    // Orthogonal vectors give zero — the sign/pairing of the three products is pinned, not
    // just their magnitude.
    assert_eq!(eval_e3_dot([1.0, 0.0, 0.0], [0.0, 1.0, 1.0]), 0.0);

    let g = boyko_shaderdsl::emit::emit_hlsl_e3_dot().replace("\r\n", "\n");
    assert_eq!(
        g, "    float vn = dot(v, n);\n\x20   return vn * dot(v, v);\n",
        "the E3 span must spell the `dot` INTRINSIC, inline under `*` without parens:\n{g}"
    );
}

// ---- E4: the `sin` / `cos` facet on the control-flow axis ------------------------------

#[test]
fn e4_trig_eval_and_emit() {
    // Eval: sin² + cos² == 1 at several angles (a self-checking identity — no table of
    // expected sine values is needed to prove the hooks reach real trig).
    for &theta in &[0.0f32, 0.7, -1.3, 3.0, 12.5] {
        let got = eval_e4_trig(theta);
        assert!(
            (got - 1.0).abs() <= 1.0e-6,
            "sin^2 + cos^2 must be 1 at theta = {theta}, got {got}"
        );
    }
    // The identity is symmetric under swapping the two hooks, so pin one asymmetric value:
    // at theta = 0, sin = 0 and cos = 1, i.e. `s*s + c*c` is exactly 1.0 and `s` alone is 0.
    assert_eq!(EvalCf::sin(0.0), 0.0);
    assert_eq!(EvalCf::cos(0.0), 1.0);

    // Emit: the SAME `Node::Sin`/`Node::Cos` printer arms the `InterpBackend` recorder feeds,
    // so the two backend axes cannot drift in spelling.
    let g = boyko_shaderdsl::emit::emit_hlsl_e4_trig().replace("\r\n", "\n");
    assert_eq!(
        g,
        "    float s = sin(theta);\n\
         \x20   float c = cos(theta);\n\
         \x20   return s * s + c * c;\n",
        "the E4 span must spell `sin`/`cos` in that order over `theta`:\n{g}"
    );
}

// ---- E5: the `rsqrt` renormalization facet ---------------------------------------------

#[test]
fn e5_rsqrt_eval_and_emit() {
    // Eval: exact where the reciprocal square root is a power of two — these two pin the
    // op itself (a `sqrt` without the reciprocal would give 2.0 and 0.5 respectively).
    assert_eq!(EvalCf::rsqrt(4.0), 0.5);
    assert_eq!(EvalCf::rsqrt(0.25), 2.0);

    // The probe renormalizes a (cos, sin) pair. `(3, 4)` has length 5, so the renormalized
    // cosine is 3/5 = 0.6. TOLERANCE by design, not by accident: the GPU's `InverseSqrt` is
    // an APPROXIMATE instruction (2 ULP allowed), so this value carries no bit-exact
    // contract on either backend — 1e-6 is far inside that band and far outside a wrong-op
    // result (a missing reciprocal would give 15.0).
    let got = eval_e5_renorm(3.0, 4.0);
    assert!(
        (got - 0.6).abs() <= 1.0e-6,
        "renormalized cos must be 3/5, got {got}"
    );
    // An already-unit pair comes back unchanged (within the same band).
    let unit = eval_e5_renorm(1.0, 0.0);
    assert!(
        (unit - 1.0).abs() <= 1.0e-6,
        "a unit pair must survive, got {unit}"
    );

    let g = boyko_shaderdsl::emit::emit_hlsl_e5_renorm().replace("\r\n", "\n");
    assert_eq!(
        g,
        "    float len_sq = c * c + s * s;\n\
         \x20   float inv_len = rsqrt(len_sq);\n\
         \x20   return c * inv_len;\n",
        "the E5 span must spell the single `rsqrt` intrinsic, not `1.0 / sqrt(...)`:\n{g}"
    );
}
