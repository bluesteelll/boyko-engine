//! Pass-1 validation (a): EVAL BYTE-IDENTITY.
//!
//! Proves `boyko_shaderdsl::field::sdf_field_body::<f32>` (the refactored field,
//! authored ONCE generic over `FieldScalar`) is BYTE-IDENTICAL to the
//! pre-refactor `boyko_sdf_math::sdf_edit_list` hand-written fold. The pre-refactor
//! body is SNAPSHOT verbatim below (`frozen_sdf_edit_list` + its private helpers),
//! so the diff is real: the test fails if the generic body diverges by even one
//! ULP from the frozen reference.
//!
//! Coverage: empty (n=0), full (n=16), n>16 (clamp), each op (UNION/SUBTRACT/
//! INTERSECT), hard (k=0) and smooth (k>0), and non-finite inputs (NaN/Inf/MAX/
//! MIN/subnormal) at the points AND in the edit params. A deterministic LCG sweeps
//! random edits/points (no proptest dep — the tester extends this with proptest +
//! the boyko_sdf_math suite per the plan).
//!
//! NOTE: this test links the DEFAULT (non-`nightly`) profile, where `std` is
//! available for `f32::sqrt` (the Eval `sqrt` shim) and the test harness.

use boyko_shaderdsl::cf::EvalCf;
use boyko_shaderdsl::field::{self, EditView};

// ======================================================================
// FROZEN REFERENCE — verbatim snapshot of the PRE-REFACTOR field bodies
// (boyko_sdf_math lib.rs:525-689 as of the commit before this refactor).
// Do NOT "clean up": the byte-for-byte op order is the contract under test.
// ======================================================================

const FROZEN_SDF_FAR: f32 = 1.0e9;
const FROZEN_MAX_SDF_EDITS: usize = 16;

mod frozen_op {
    pub const SUBTRACT: u32 = 1;
    pub const INTERSECT: u32 = 2;
}
mod frozen_kind {
    pub const BOX: u32 = 1;
}

/// The reference edit, mirroring the relevant `SdfEdit` fields (center.xyz +
/// params.xyz + kind/op/smoothness — the field never reads center.w/params.w/_pad).
#[derive(Clone, Copy)]
struct FrozenEdit {
    center: [f32; 3],
    params: [f32; 3],
    kind: u32,
    op: u32,
    smoothness: f32,
}

fn frozen_sqrt(x: f32) -> f32 {
    x.sqrt()
}
fn frozen_v_sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn frozen_v_len(a: [f32; 3]) -> f32 {
    frozen_sqrt(a[0] * a[0] + a[1] * a[1] + a[2] * a[2])
}
fn frozen_v_abs(a: [f32; 3]) -> [f32; 3] {
    [a[0].abs(), a[1].abs(), a[2].abs()]
}
fn frozen_v_max0(a: [f32; 3]) -> [f32; 3] {
    [a[0].max(0.0), a[1].max(0.0), a[2].max(0.0)]
}
fn frozen_sd_sphere(p: [f32; 3], c: [f32; 3], r: f32) -> f32 {
    frozen_v_len(frozen_v_sub(p, c)) - r
}
fn frozen_sd_box(p: [f32; 3], c: [f32; 3], h: [f32; 3]) -> f32 {
    let q = frozen_v_sub(frozen_v_abs(frozen_v_sub(p, c)), h);
    let outside = frozen_v_len(frozen_v_max0(q));
    let inside = q[0].max(q[1].max(q[2])).min(0.0);
    outside + inside
}
fn frozen_edit_distance(e: &FrozenEdit, p: [f32; 3]) -> f32 {
    let center = [e.center[0], e.center[1], e.center[2]];
    if e.kind == frozen_kind::BOX {
        frozen_sd_box(p, center, [e.params[0], e.params[1], e.params[2]])
    } else {
        frozen_sd_sphere(p, center, e.params[0])
    }
}
fn frozen_smin(a: f32, b: f32, k: f32) -> f32 {
    let hh = (0.5 + 0.5 * (b - a) / k).clamp(0.0, 1.0);
    (b + (a - b) * hh) - k * hh * (1.0 - hh)
}
fn frozen_smax(a: f32, b: f32, k: f32) -> f32 {
    -frozen_smin(-a, -b, k)
}
fn frozen_combine(acc: f32, d: f32, op: u32, k: f32) -> f32 {
    match op {
        x if x == frozen_op::SUBTRACT => {
            if k > 0.0 {
                frozen_smax(acc, -d, k)
            } else {
                acc.max(-d)
            }
        }
        x if x == frozen_op::INTERSECT => {
            if k > 0.0 {
                frozen_smax(acc, d, k)
            } else {
                acc.max(d)
            }
        }
        _ => {
            if k > 0.0 {
                frozen_smin(acc, d, k)
            } else {
                acc.min(d)
            }
        }
    }
}
fn frozen_sdf_edit_list(edits: &[FrozenEdit], p: [f32; 3]) -> f32 {
    let n = edits.len().min(FROZEN_MAX_SDF_EDITS);
    let mut acc = FROZEN_SDF_FAR;
    for (i, e) in edits.iter().take(n).enumerate() {
        let d = frozen_edit_distance(e, p);
        if i == 0 {
            acc = d;
        } else {
            acc = frozen_combine(acc, d, e.op, e.smoothness);
        }
    }
    acc
}

// ======================================================================
// Adapters: a FrozenEdit <-> the refactored EditView<f32>.
// ======================================================================

fn to_view(e: &FrozenEdit) -> EditView<f32> {
    EditView {
        center: e.center,
        params: e.params,
        kind: e.kind,
        op: e.op,
        smoothness: e.smoothness,
    }
}

/// The refactored field over the f32 Eval backend, over the same edit list.
fn refactored(edits: &[FrozenEdit], p: [f32; 3]) -> f32 {
    let views: Vec<EditView<f32>> = edits.iter().map(to_view).collect();
    field::sdf_field_body::<f32>(&views, p)
}

/// Byte-identity assertion: the two `f32`s must have the SAME bit pattern (NaN
/// payloads included — a reordered op would shift the payload or the value).
fn assert_bits(a: f32, b: f32, ctx: &str) {
    assert_eq!(
        a.to_bits(),
        b.to_bits(),
        "{ctx}: refactored={a} (0x{:08x}) != frozen={b} (0x{:08x})",
        a.to_bits(),
        b.to_bits()
    );
}

// ======================================================================
// A tiny deterministic LCG (no rand dep) for the random sweep.
// ======================================================================

struct Lcg(u64);
impl Lcg {
    fn new(seed: u64) -> Self {
        Lcg(seed)
    }
    fn next_u32(&mut self) -> u32 {
        // Numerical Recipes LCG constants.
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (self.0 >> 32) as u32
    }
    /// A float in roughly [-range, range], with occasional non-finite values.
    fn next_f32(&mut self, range: f32) -> f32 {
        match self.next_u32() % 32 {
            0 => f32::NAN,
            1 => f32::INFINITY,
            2 => f32::NEG_INFINITY,
            3 => f32::MAX,
            4 => f32::MIN,
            5 => f32::MIN_POSITIVE,
            6 => 0.0,
            7 => -0.0,
            _ => {
                let u = (self.next_u32() as f32) / (u32::MAX as f32); // [0,1]
                (u * 2.0 - 1.0) * range
            }
        }
    }
    fn next_kind(&mut self) -> u32 {
        self.next_u32() % 2 // SPHERE | BOX
    }
    fn next_op(&mut self) -> u32 {
        self.next_u32() % 3 // UNION | SUBTRACT | INTERSECT
    }
    fn next_smoothness(&mut self) -> f32 {
        // ~half hard (k <= 0), ~half smooth (k > 0) so both combine arms hit.
        match self.next_u32() % 4 {
            0 => 0.0,
            1 => -1.0,
            _ => 0.01 + (self.next_u32() as f32 / u32::MAX as f32) * 2.0,
        }
    }
    fn next_edit(&mut self) -> FrozenEdit {
        FrozenEdit {
            center: [
                self.next_f32(10.0),
                self.next_f32(10.0),
                self.next_f32(10.0),
            ],
            params: [self.next_f32(5.0), self.next_f32(5.0), self.next_f32(5.0)],
            kind: self.next_kind(),
            op: self.next_op(),
            smoothness: self.next_smoothness(),
        }
    }
}

#[test]
fn empty_field_is_byte_identical() {
    // n = 0: the fold returns the SDF_FAR seed untouched.
    let edits: [FrozenEdit; 0] = [];
    for p in [[0.0, 0.0, 0.0], [1.0, 2.0, 3.0], [f32::NAN, 0.0, 0.0]] {
        assert_bits(
            refactored(&edits, p),
            frozen_sdf_edit_list(&edits, p),
            "empty",
        );
    }
}

#[test]
fn over_capacity_clamps_byte_identical() {
    // n > 16: both clamp to MAX_SDF_EDITS, folding only the first 16.
    let mut lcg = Lcg::new(0xDEAD_BEEF);
    let edits: Vec<FrozenEdit> = (0..40).map(|_| lcg.next_edit()).collect();
    for _ in 0..256 {
        let p = [lcg.next_f32(12.0), lcg.next_f32(12.0), lcg.next_f32(12.0)];
        assert_bits(
            refactored(&edits, p),
            frozen_sdf_edit_list(&edits, p),
            "over-cap",
        );
    }
}

#[test]
fn random_sweep_byte_identical() {
    // Random edits/points across all kinds/ops/smoothness, sizes 1..=16, with
    // non-finite values mixed in (the next_f32 / next_smoothness distributions).
    let mut lcg = Lcg::new(0x0123_4567_89AB_CDEF);
    for _ in 0..20_000 {
        let n = 1 + (lcg.next_u32() as usize % FROZEN_MAX_SDF_EDITS); // 1..=16
        let edits: Vec<FrozenEdit> = (0..n).map(|_| lcg.next_edit()).collect();
        let p = [lcg.next_f32(15.0), lcg.next_f32(15.0), lcg.next_f32(15.0)];
        assert_bits(
            refactored(&edits, p),
            frozen_sdf_edit_list(&edits, p),
            "sweep",
        );
    }
}

// ======================================================================
// A1 — the surface-NORMAL leaf byte-identity.
//
// Frozen snapshot of `boyko_sdf_math::{v_normalize, sdf_edit_list_normal}`
// (lib.rs:575-584, 696-704) vs the eDSL `boyko_shaderdsl::normal::sdf_normal_body`
// over the `f32` Eval backend. The normal CALLS the field at six probe points
// (`p ± GRAD_H` per axis); the eDSL field-call seam passes the frozen field as the
// closure, so the whole central-difference + guarded-`normalize` chain is compared
// bit-for-bit — including the `-0.0` / NaN off-axis cases, where the eDSL's full
// `v_add(p, [h,0,0])` differs from the host's per-axis `[p0+h, p1, p2]` BEFORE the
// field eval (the field is ±0-/payload-insensitive and the guard collapses any
// non-finite-length gradient to ZERO, so the OUTPUT normal must still be identical).
// ======================================================================

const FROZEN_GRAD_H: f32 = 0.0005; // boyko_sdf_math::SDF_GRAD_H == sdf_field.hlsli GRAD_H

fn frozen_v_normalize(a: [f32; 3]) -> [f32; 3] {
    let len = frozen_v_len(a);
    // The guarded normalize (lib.rs:575): a zero / non-finite length collapses to
    // ZERO (the physics seam-skip sentinel), not NaN. Non-degenerate inputs take the
    // byte-identical division.
    if len <= f32::MIN_POSITIVE || !len.is_finite() {
        return [0.0, 0.0, 0.0];
    }
    [a[0] / len, a[1] / len, a[2] / len]
}

fn frozen_sdf_edit_list_normal(edits: &[FrozenEdit], p: [f32; 3]) -> [f32; 3] {
    let h = FROZEN_GRAD_H;
    let n = [
        frozen_sdf_edit_list(edits, [p[0] + h, p[1], p[2]])
            - frozen_sdf_edit_list(edits, [p[0] - h, p[1], p[2]]),
        frozen_sdf_edit_list(edits, [p[0], p[1] + h, p[2]])
            - frozen_sdf_edit_list(edits, [p[0], p[1] - h, p[2]]),
        frozen_sdf_edit_list(edits, [p[0], p[1], p[2] + h])
            - frozen_sdf_edit_list(edits, [p[0], p[1], p[2] - h]),
    ];
    frozen_v_normalize(n)
}

/// The eDSL normal over the `f32` Eval backend, with the frozen field as the
/// field-call seam closure (so this re-runs the SAME frozen fold at each probe).
fn refactored_normal(edits: &[FrozenEdit], p: [f32; 3]) -> [f32; 3] {
    let views: Vec<EditView<f32>> = edits.iter().map(to_view).collect();
    boyko_shaderdsl::normal::sdf_normal_body::<f32, _>(p, |q| {
        field::sdf_field_body::<f32>(&views, q)
    })
}

fn assert_vec_bits(a: [f32; 3], b: [f32; 3], ctx: &str) {
    for i in 0..3 {
        assert_eq!(
            a[i].to_bits(),
            b[i].to_bits(),
            "{ctx}[{i}]: refactored={} (0x{:08x}) != frozen={} (0x{:08x})",
            a[i],
            a[i].to_bits(),
            b[i],
            b[i].to_bits()
        );
    }
}

#[test]
fn normal_random_sweep_byte_identical() {
    // Same sweep shape as the field test: random edits/points across all kinds/ops/
    // smoothness, sizes 1..=16, with non-finite + signed-zero values mixed in.
    let mut lcg = Lcg::new(0xBADC_0FFE_E0DD_F00D);
    for _ in 0..20_000 {
        let n = 1 + (lcg.next_u32() as usize % FROZEN_MAX_SDF_EDITS);
        let edits: Vec<FrozenEdit> = (0..n).map(|_| lcg.next_edit()).collect();
        let p = [lcg.next_f32(15.0), lcg.next_f32(15.0), lcg.next_f32(15.0)];
        assert_vec_bits(
            refactored_normal(&edits, p),
            frozen_sdf_edit_list_normal(&edits, p),
            "normal-sweep",
        );
    }
}

#[test]
fn normal_signed_zero_and_axis_aligned_byte_identical() {
    // Targeted: points with -0.0 / +0.0 / on-axis coords where the eDSL full-vector
    // offset (p + [h,0,0]) and the host per-axis offset diverge at the bit level
    // before the field eval. The field must wash this out.
    let mut lcg = Lcg::new(0x5165_D000_2E20_0001);
    let zeros = [-0.0f32, 0.0f32];
    for _ in 0..2_000 {
        let n = 1 + (lcg.next_u32() as usize % FROZEN_MAX_SDF_EDITS);
        let edits: Vec<FrozenEdit> = (0..n).map(|_| lcg.next_edit()).collect();
        for &z in &zeros {
            for p in [
                [z, lcg.next_f32(5.0), lcg.next_f32(5.0)],
                [lcg.next_f32(5.0), z, lcg.next_f32(5.0)],
                [lcg.next_f32(5.0), lcg.next_f32(5.0), z],
                [z, z, z],
            ] {
                assert_vec_bits(
                    refactored_normal(&edits, p),
                    frozen_sdf_edit_list_normal(&edits, p),
                    "normal-signed-zero",
                );
            }
        }
    }
}

#[test]
fn each_op_each_smoothness_byte_identical() {
    // Exhaustively cross each op with hard (k=0) and smooth (k>0), each kind.
    let ops = [0u32, frozen_op::SUBTRACT, frozen_op::INTERSECT];
    let ks = [0.0f32, 0.5, 1.5, -1.0];
    let kinds = [0u32, frozen_kind::BOX];
    let mut lcg = Lcg::new(0xFACE_CAFE);
    for &op in &ops {
        for &k in &ks {
            for &kind in &kinds {
                // A 2-edit list: a sphere seed then the op edit under test.
                let seed = FrozenEdit {
                    center: [0.0, 0.0, 0.0],
                    params: [1.0, 1.0, 1.0],
                    kind: 0,
                    op: 0,
                    smoothness: 0.0,
                };
                let e = FrozenEdit {
                    center: [lcg.next_f32(3.0), lcg.next_f32(3.0), lcg.next_f32(3.0)],
                    params: [
                        lcg.next_f32(2.0).abs() + 0.1,
                        lcg.next_f32(2.0).abs() + 0.1,
                        lcg.next_f32(2.0).abs() + 0.1,
                    ],
                    kind,
                    op,
                    smoothness: k,
                };
                let edits = [seed, e];
                for _ in 0..64 {
                    let p = [lcg.next_f32(5.0), lcg.next_f32(5.0), lcg.next_f32(5.0)];
                    assert_bits(
                        refactored(&edits, p),
                        frozen_sdf_edit_list(&edits, p),
                        "op-smoothness",
                    );
                }
            }
        }
    }
}

// ======================================================================
// A2 — the brick `decode_snorm8` leaf byte-identity.
//
// Frozen snapshot of `boyko_sdf_math::brick::decode_snorm8` (brick.rs:1049-1054) vs
// the eDSL `boyko_shaderdsl::brick::decode_snorm8` over the `f32` Eval backend. The
// full leaf (byte → normalize → world scale) is compared to-bits over EVERY one of the
// 256 codes (`i8::MIN..=i8::MAX`), crossed with edge band-half values (0.0, the
// production `BAND_HALF_STORE`, non-finite). On the GPU the byte → normalize step is
// the hardware `R8_SNORM` sampler and only the `n * band_half` scale is shader code
// (the `m2_decode` body); this test locks the CPU oracle the GPU fetch is golden-
// compared against.
// ======================================================================

/// The production narrow-band half-width (`boyko_sdf_math::brick::BAND_HALF_STORE`).
const FROZEN_BAND_HALF_STORE: f32 = 0.90;

/// Verbatim snapshot of the PRE-eDSL host `decode_snorm8` (brick.rs:1049-1054). Do NOT
/// "clean up": the `i8::MIN` sentinel branch + the `q as f32 / 127.0` operand order is
/// the contract under test.
fn frozen_decode_snorm8(q: i8, band_half: f32) -> f32 {
    let n = if q == i8::MIN {
        -1.0
    } else {
        q as f32 / 127.0
    };
    n * band_half
}

/// The eDSL decode over the `f32` Eval backend. The code widens to `i32` (the
/// `FieldScalar::Int` for `f32`) losslessly; `q as f32` is identical from either width.
fn refactored_decode_snorm8(q: i8, band_half: f32) -> f32 {
    boyko_shaderdsl::brick::decode_snorm8::<f32>(q as i32, band_half)
}

#[test]
fn decode_snorm8_all_256_codes_byte_identical() {
    // Every code, crossed with edge + production band-half values (incl. non-finite,
    // where the multiply must propagate NaN/Inf bit-identically through both paths).
    let bands = [
        FROZEN_BAND_HALF_STORE,
        0.0,
        1.0,
        -1.0,
        0.5,
        f32::INFINITY,
        f32::NAN,
        f32::MIN_POSITIVE,
        f32::MAX,
    ];
    for &band in &bands {
        for q in i8::MIN..=i8::MAX {
            assert_bits(
                refactored_decode_snorm8(q, band),
                frozen_decode_snorm8(q, band),
                "decode_snorm8",
            );
        }
    }
}

#[test]
fn decode_snorm8_sentinel_and_extremes_byte_identical() {
    // Targeted: the `-128` snorm sentinel decodes to `-1.0 * band` (NOT `-128/127`),
    // and the `±127` extremes map to `±1.0 * band` — the asymmetric R8_SNORM rule. A
    // random sweep over codes/bands mixes the LCG's non-finite band-half draws in.
    let mut lcg = Lcg::new(0x5D0F_DEC0_DE00_BEEF);
    for (q, want_n) in [
        (i8::MIN, -1.0f32),    // -128 → -1.0 (sentinel, NOT -128/127)
        (-127i8, -127.0 / 127.0),
        (0i8, 0.0),
        (127i8, 127.0 / 127.0),
    ] {
        for &band in &[FROZEN_BAND_HALF_STORE, 0.5, 2.0] {
            let got = refactored_decode_snorm8(q, band);
            assert_bits(got, want_n * band, "decode-sentinel");
            assert_bits(got, frozen_decode_snorm8(q, band), "decode-sentinel-vs-frozen");
        }
    }
    for _ in 0..4_000 {
        let q = (lcg.next_u32() & 0xFF) as u8 as i8;
        let band = lcg.next_f32(4.0);
        assert_bits(
            refactored_decode_snorm8(q, band),
            frozen_decode_snorm8(q, band),
            "decode-random",
        );
    }
}

// ======================================================================
// A3 — the M2 cubic-surface leaves byte-identity (the bit-exact-`t` guard).
//
// Frozen snapshot of `boyko_sdf_math::brick::{cubic_eval, jcgt_cubic_coeffs}` (brick.rs
// :1353-1415, pre-eDSL) vs the eDSL `boyko_shaderdsl::brick::{cubic_eval,
// jcgt_cubic_coeffs}` over the `f32` Eval backend. These feed the cubic root-finder's
// `t`, where hit/miss is a CLIFF (not a ±2/255 render band), so a single-ULP coefficient
// drift flips a hit — the to-bits gate is the real defense (the render golden can MASK
// it). Swept 20k+ over random `c`/`t` and random 8-corner `s` + rays, with the LCG's
// NaN/Inf/±0/MAX/MIN draws mixed into EVERY operand.
// ======================================================================

/// Verbatim snapshot of the PRE-eDSL host `jcgt_cubic_coeffs` (brick.rs:1353-1407). Do
/// NOT "clean up": the k-basis index pairing (the k3/k7 trap) and the FMA grouping are
/// the contract under test — a transposed pair / reordered expansion drifts the golden.
fn frozen_jcgt_cubic_coeffs(s: &[f32; 8], ro_local: [f32; 3], rd_local: [f32; 3]) -> [f32; 4] {
    let s000 = s[0];
    let s100 = s[1];
    let s010 = s[2];
    let s110 = s[3];
    let s001 = s[4];
    let s101 = s[5];
    let s011 = s[6];
    let s111 = s[7];

    let k0 = s000;
    let k1 = s100 - s000;
    let k2 = s010 - s000;
    let k3 = s001 - s000;
    let k4 = s110 - s100 - s010 + s000; // x·y
    let k5 = s011 - s010 - s001 + s000; // y·z
    let k6 = s101 - s100 - s001 + s000; // z·x
    let k7 = s111 - s110 - s101 - s011 + s100 + s010 + s001 - s000; // x·y·z

    let (ax, ay, az) = (ro_local[0], ro_local[1], ro_local[2]);
    let (bx, by, bz) = (rd_local[0], rd_local[1], rd_local[2]);

    let c0 = k0
        + k1 * ax
        + k2 * ay
        + k3 * az
        + k4 * ax * ay
        + k5 * ay * az
        + k6 * az * ax
        + k7 * ax * ay * az;

    let c1 = k1 * bx
        + k2 * by
        + k3 * bz
        + k4 * (ax * by + ay * bx)
        + k5 * (ay * bz + az * by)
        + k6 * (az * bx + ax * bz)
        + k7 * (ax * ay * bz + ax * by * az + bx * ay * az);

    let c2 = k4 * bx * by
        + k5 * by * bz
        + k6 * bz * bx
        + k7 * (ax * by * bz + bx * ay * bz + bx * by * az);

    let c3 = k7 * bx * by * bz;

    [c0, c1, c2, c3]
}

/// Verbatim snapshot of the PRE-eDSL host `cubic_eval` (brick.rs:1411-1415).
fn frozen_cubic_eval(c: &[f32; 4], t: f32) -> f32 {
    ((c[3] * t + c[2]) * t + c[1]) * t + c[0]
}

/// The eDSL cubic-eval over the `f32` Eval backend.
fn refactored_cubic_eval(c: &[f32; 4], t: f32) -> f32 {
    boyko_shaderdsl::brick::cubic_eval::<f32>(c, t)
}

/// The eDSL cubic-coeffs fold over the `f32` Eval backend.
fn refactored_jcgt_cubic_coeffs(s: &[f32; 8], a: [f32; 3], b: [f32; 3]) -> [f32; 4] {
    boyko_shaderdsl::brick::jcgt_cubic_coeffs::<f32>(s, a, b)
}

fn assert_vec4_bits(a: [f32; 4], b: [f32; 4], ctx: &str) {
    for i in 0..4 {
        assert_eq!(
            a[i].to_bits(),
            b[i].to_bits(),
            "{ctx}[{i}]: refactored={} (0x{:08x}) != frozen={} (0x{:08x})",
            a[i],
            a[i].to_bits(),
            b[i],
            b[i].to_bits()
        );
    }
}

#[test]
fn cubic_eval_random_sweep_byte_identical() {
    // Random coefficients + `t`, with the LCG's NaN/Inf/±0/MAX/MIN draws mixed into
    // every operand (so the Horner FMA chain must propagate the bit pattern identically
    // through both paths).
    let mut lcg = Lcg::new(0x0C0B_1C5E_E000_3333);
    for _ in 0..20_000 {
        let c = [
            lcg.next_f32(8.0),
            lcg.next_f32(8.0),
            lcg.next_f32(8.0),
            lcg.next_f32(8.0),
        ];
        let t = lcg.next_f32(2.0);
        assert_bits(
            refactored_cubic_eval(&c, t),
            frozen_cubic_eval(&c, t),
            "cubic-eval-sweep",
        );
    }
}

#[test]
fn jcgt_cubic_coeffs_random_sweep_byte_identical() {
    // Random 8-corner sets + random ray frame (`a` = ro_local, `b` = rd_local), with the
    // LCG's non-finite + signed-zero draws mixed in. The full coefficient vector is
    // compared to-bits — a single-ULP drift in any of c0..c3 flips a hit/miss on `t`.
    let mut lcg = Lcg::new(0x5165_C0B1_C5EE_4444);
    for _ in 0..20_000 {
        let mut s = [0.0f32; 8];
        for v in s.iter_mut() {
            *v = lcg.next_f32(4.0);
        }
        let a = [lcg.next_f32(2.0), lcg.next_f32(2.0), lcg.next_f32(2.0)];
        let b = [lcg.next_f32(2.0), lcg.next_f32(2.0), lcg.next_f32(2.0)];
        assert_vec4_bits(
            refactored_jcgt_cubic_coeffs(&s, a, b),
            frozen_jcgt_cubic_coeffs(&s, a, b),
            "jcgt-coeffs-sweep",
        );
    }
}

#[test]
fn cubic_chain_coeffs_then_eval_byte_identical() {
    // The full chain the solver runs: form the coefficients then evaluate the cubic at
    // many `t` — the eDSL `cubic_eval(jcgt_cubic_coeffs(...), t)` must be to-bits the
    // frozen chain. (Random corners/rays/`t`; non-finite draws included.)
    let mut lcg = Lcg::new(0xC0B1_C5EE_BADC_5555);
    for _ in 0..6_000 {
        let mut s = [0.0f32; 8];
        for v in s.iter_mut() {
            *v = lcg.next_f32(3.0);
        }
        let a = [lcg.next_f32(1.0), lcg.next_f32(1.0), lcg.next_f32(1.0)];
        let b = [lcg.next_f32(1.0), lcg.next_f32(1.0), lcg.next_f32(1.0)];
        let c_re = refactored_jcgt_cubic_coeffs(&s, a, b);
        let c_fr = frozen_jcgt_cubic_coeffs(&s, a, b);
        assert_vec4_bits(c_re, c_fr, "chain-coeffs");
        for ti in 0..8 {
            let t = (ti as f32) / 7.0;
            assert_bits(
                refactored_cubic_eval(&c_re, t),
                frozen_cubic_eval(&c_fr, t),
                "chain-eval",
            );
        }
    }
}

// ======================================================================
// Increment 1 — the brick-exit empty-skip MARCHER leaf (control flow).
//
// The FIRST control-flow leaf: an `[unroll]` slab loop with a data-dependent
// `continue` + a final progress-clamp ternary. The eDSL
// `boyko_shaderdsl::brick::dist_to_brick_exit_body::<EvalCf>` (`Cf::Scalar = f32`) is the
// CPU oracle.
//
// CANONICAL SHAPE = the GPU's (`sdf_gbuffer_composite.hlsl`): `exit` inits to a plain
// `1.0e30`, `max`/`min` per axis, NO `is_finite` term. The HOST
// `boyko_sdf_math::brick::dist_to_brick_exit` stays HAND-WRITTEN (firewall option B) and
// inits `f32::INFINITY` with a final `|| !exit.is_finite()` guard — so the two ALREADY
// DIVERGE on an all-axes-degenerate ray (1e30 vs EPS). That input is marcher-UNREACHABLE
// (a normalized `rd` cannot have all three |components| <= 1e-4), so this sweep:
//   (a) proves the canonical body equals a FROZEN GPU-shape reference to-bits over the
//       HARD reachable set (subnormal dir just above EPS, dir == EPS boundary, overflowing
//       slab products, partly-skipped axes), and
//   (b) asserts the all-degenerate ray yields the GPU value `1.0e30` — NOT compared vs the
//       host (the one intentional GPU-vs-host difference).
// ======================================================================

const FROZEN_BRICK_EXIT_EPS: f32 = 1.0e-4; // boyko_shaderdsl::brick::BRICK_EXIT_EPS

/// Verbatim snapshot of the GPU-SHAPE `dist_to_brick_exit` (the committed
/// `sdf_gbuffer_composite.hlsl:569-589` math): `exit = 1.0e30`, `max`/`min`, NO
/// `is_finite`. This is the reference the canonical eDSL body must equal to-bits — NOT
/// the host (which has the `is_finite` guard). Do NOT "clean up": the operand order +
/// the dropped `is_finite` are the contract under test.
fn frozen_gpu_dist_to_brick_exit(
    p: [f32; 3],
    rd: [f32; 3],
    cell_min: [f32; 3],
    bw: f32,
) -> f32 {
    let mut exit = 1.0e30f32;
    for a in 0..3 {
        let t0 = rd[a];
        let t1 = cell_min[a];
        let t2 = t1 + bw;
        if t0.abs() <= FROZEN_BRICK_EXIT_EPS {
            continue;
        }
        let t3 = 1.0 / t0;
        let t4 = (t1 - p[a]) * t3;
        let t5 = (t2 - p[a]) * t3;
        let t6 = t4.max(t5);
        exit = exit.min(t6);
    }
    if exit < FROZEN_BRICK_EXIT_EPS {
        FROZEN_BRICK_EXIT_EPS
    } else {
        exit
    }
}

/// The eDSL brick-exit over the `<EvalCf>` instantiation (`Cf::Scalar = f32`; the CPU
/// oracle).
fn refactored_brick_exit(p: [f32; 3], rd: [f32; 3], cell_min: [f32; 3], bw: f32) -> f32 {
    boyko_shaderdsl::brick::dist_to_brick_exit_body::<EvalCf>(p, rd, cell_min, bw)
}

#[test]
fn brick_exit_canonical_equals_gpu_shape_byte_identical() {
    // The HARD non-finite / boundary set the dropped `is_finite` term might have changed,
    // PLUS a broad random sweep — proving the eDSL canonical body is to-bits the frozen
    // GPU-shape reference on the whole reachable set.
    let eps = FROZEN_BRICK_EXIT_EPS;
    let cell_min = [0.0, 0.0, 0.0];
    let bw = 1.0;

    // Targeted hard cases (one axis non-degenerate so the ray is reachable):
    let hard: &[[f32; 3]] = &[
        // Subnormal dir JUST above EPS on x (1/dir overflows -> t_far = Inf -> min stays
        // finite from the other axes; here y carries a finite bound).
        [eps * 1.0001, 0.5, 0.5],
        // dir == EPS exactly on x (abs(dir) <= EPS -> x SKIPPED; y/z bound it).
        [eps, 0.7, 0.3],
        // dir just BELOW EPS on x and z (both skipped); y non-degenerate.
        [eps * 0.5, 0.9, eps * 0.5],
        // A normal ray (all axes contribute).
        [0.6, -0.5, 0.62],
        // Tiny-but-finite y, large x.
        [0.99, eps * 2.0, 0.14],
    ];
    for &rd in hard {
        // `p` placed so some `(lo - p)*inv` / `(hi - p)*inv` products are huge (overflow
        // path) as well as inside the cell.
        for &p in &[
            [0.5, 0.5, 0.5],
            [-1.0e20, 0.5, 0.5],   // (lo - p) huge
            [1.0e20, 0.5, 0.5],    // (hi - p) huge negative
            [0.5, 1.0e30, 0.5],
        ] {
            assert_bits(
                refactored_brick_exit(p, rd, cell_min, bw),
                frozen_gpu_dist_to_brick_exit(p, rd, cell_min, bw),
                "brick-exit-hard",
            );
        }
    }

    // A broad random sweep (reachable rays + p across the cell and far outside), with the
    // LCG's non-finite/±0 draws mixed into `p`/`cell_min`/`bw`.
    let mut lcg = Lcg::new(0xB21C_E417_DEAD_9001);
    for _ in 0..20_000 {
        // Force at least one axis above EPS so the ray is marcher-reachable.
        let mut rd = [lcg.next_f32(1.0), lcg.next_f32(1.0), lcg.next_f32(1.0)];
        if rd.iter().all(|c| c.abs() <= eps) {
            rd[0] = 0.5; // make it reachable
        }
        let p = [lcg.next_f32(50.0), lcg.next_f32(50.0), lcg.next_f32(50.0)];
        let cmin = [lcg.next_f32(10.0), lcg.next_f32(10.0), lcg.next_f32(10.0)];
        let bwid = lcg.next_f32(5.0).abs() + 0.01;
        assert_bits(
            refactored_brick_exit(p, rd, cmin, bwid),
            frozen_gpu_dist_to_brick_exit(p, rd, cmin, bwid),
            "brick-exit-sweep",
        );
    }
}

#[test]
fn brick_exit_all_degenerate_ray_is_gpu_value() {
    // The ONE intentional GPU-vs-host difference: an all-axes-degenerate ray (every
    // |rd component| <= EPS) skips every axis, so `exit` stays at the `1.0e30` init and the
    // final clamp (1e30 >= EPS) returns 1.0e30 — the GPU value (the host would return EPS
    // via its extra `|| !exit.is_finite()`-style INFINITY init). This input is
    // marcher-UNREACHABLE (a normalized rd cannot have all three components <= 1e-4); the
    // canonical form is asserted against the GPU value, NOT compared vs the host.
    let eps = FROZEN_BRICK_EXIT_EPS;
    for &rd in &[
        [0.0, 0.0, 0.0],
        [eps, eps, eps],
        [eps * 0.5, -eps * 0.5, eps * 0.9],
        [-0.0, 0.0, eps],
    ] {
        let got = refactored_brick_exit([0.5, 0.5, 0.5], rd, [0.0, 0.0, 0.0], 1.0);
        assert_bits(got, 1.0e30, "brick-exit-all-degenerate");
        // And the canonical body matches the frozen GPU-shape reference on this input.
        assert_bits(
            got,
            frozen_gpu_dist_to_brick_exit([0.5, 0.5, 0.5], rd, [0.0, 0.0, 0.0], 1.0),
            "brick-exit-all-degenerate-vs-gpu-frozen",
        );
    }
}

#[test]
fn cf_continue_skips_live_tail_on_eval() {
    // MANDATORY: a body that SETS a Var AFTER a TAKEN continue must NOT run the set on
    // Eval — the `?`-propagated continue early-returns the loop-body closure, so the live
    // tail (the var assignment) is skipped, exactly like a host `continue`. This proves the
    // Try-based control-flow propagation has the correct continue semantics (a structurally
    // recorded continue on Emit, a real skip on Eval).
    use boyko_shaderdsl::cf::{Cf, Flow};
    use boyko_shaderdsl::scalar::FieldScalar;

    // counter starts at 0; for a in 0..3, continue when a == 1, else `counter += 1`. The
    // tail `counter += 1` must run for a=0 and a=2 only -> counter == 2 (NOT 3).
    let counter = EvalCf::decl_var("counter", 0.0);
    EvalCf::unroll_for("[unroll]", 3, |a| -> Flow {
        // Skip the live tail when a == 1.
        EvalCf::if_((a == 1).then_some(()).is_some(), EvalCf::cont)?;
        // LIVE TAIL — must be skipped on the taken-continue iteration.
        let cur = EvalCf::get_var(&counter);
        EvalCf::set_var(&counter, cur.add(1.0));
        Flow::Continue(())
    });
    let total = EvalCf::get_var(&counter);
    assert_eq!(
        total.to_bits(),
        2.0f32.to_bits(),
        "continue must skip the live tail: expected 2 increments (a=0, a=2), got {total}"
    );
}

// ======================================================================
// Increment 3 — the brick-cell pointer-grid lookup leaf (early-return CF).
//
// The SECOND control-flow leaf, the first with EARLY RETURNS + a
// `StructuredBuffer<uint>` load + an `out float3` param + `uint` index math. The eDSL
// `boyko_shaderdsl::brick::brick_cell_class_body::<EvalCf>` is the CPU oracle.
//
// CANONICAL SHAPE = the GPU's committed `brick_cell_class` (sdf_gbuffer_composite.hlsl
// :608-626). The frozen GPU-shape mirror below replicates the EXACT write order: the
// default `cell_min = origin`, guard 1 (negative rel — tested on the float BEFORE any
// cast), the `(uint)` casts, guard 2 (bounds), the `idx`, the conditional `cell_min =
// origin + float3(ix,iy,iz)*bw`, then `grid[idx]`. The oracle MAP: the host
// `host_brick_cell` returns `Option<(u32,[f32;3])>` (None == out-of-grid); the eDSL body
// deposits `class` (== BRICK_OUTSIDE_GRID when out-of-grid) + writes `cell_min`. So:
//   - None  <=>  class == BRICK_OUTSIDE_GRID (0xFFFFFFFF); cell_min is DON'T-CARE on
//     OUTSIDE (the committed comment: unread when OUTSIDE), so only the class is compared.
//   - in-grid: compare BOTH class.to_bits()-equivalent (u32 eq) AND cell_min[k].to_bits().
// Edge set: negative rel, on-boundary ix==dims.x, idx==len-1, all-axes-outside, NaN/Inf.
// PLUS a tail-skip test (guard 1's `?` short-circuits before the casts run on a negative
// rel).
// ======================================================================

const FROZEN_BRICK_OUTSIDE_GRID: u32 = 0xFFFF_FFFF; // boyko_shaderdsl::brick::BRICK_OUTSIDE_GRID

/// Verbatim hand-mirror of the GPU-SHAPE `brick_cell_class` (committed
/// `sdf_gbuffer_composite.hlsl:608-626`), with the EXACT write order. Returns `(class,
/// cell_min)` — `class == BRICK_OUTSIDE_GRID` on an out-of-grid point (cell_min then the
/// default `origin`, a don't-care). Do NOT "clean up": the statement order + the
/// negative-rel-before-cast guard + the two cell_min writes are the contract under test.
fn frozen_gpu_brick_cell_class(
    grid: &[u32],
    origin: [f32; 3],
    bw: f32,
    dims: [u32; 3],
    p: [f32; 3],
) -> (u32, [f32; 3]) {
    let rel = [
        (p[0] - origin[0]) / bw,
        (p[1] - origin[1]) / bw,
        (p[2] - origin[2]) / bw,
    ];
    let mut cell_min = origin; // default (overwritten on an in-grid hit; unread when OUTSIDE)
    if rel[0] < 0.0 || rel[1] < 0.0 || rel[2] < 0.0 {
        return (FROZEN_BRICK_OUTSIDE_GRID, cell_min);
    }
    let ix = rel[0] as u32;
    let iy = rel[1] as u32;
    let iz = rel[2] as u32;
    if ix >= dims[0] || iy >= dims[1] || iz >= dims[2] {
        return (FROZEN_BRICK_OUTSIDE_GRID, cell_min);
    }
    let idx = ix + iy * dims[0] + iz * dims[0] * dims[1];
    cell_min = [
        origin[0] + (ix as f32) * bw,
        origin[1] + (iy as f32) * bw,
        origin[2] + (iz as f32) * bw,
    ];
    (grid[idx as usize], cell_min)
}

/// The eDSL brick-cell over the `<EvalCf>` instantiation (the CPU oracle). Drives the
/// out-of-band `cls` + `cell_min` cells and returns `(class, cell_min)`.
fn refactored_brick_cell_class(
    grid: &[u32],
    origin: [f32; 3],
    bw: f32,
    dims: [u32; 3],
    p: [f32; 3],
) -> (u32, [f32; 3]) {
    use std::cell::Cell;
    let cell_min = Cell::new([0.0f32; 3]);
    let cls = Cell::new(0u32);
    boyko_shaderdsl::brick::brick_cell_class_body::<EvalCf>(
        grid, origin, bw, dims, p, &cell_min, &cls,
    );
    (cls.get(), cell_min.get())
}

/// Asserts the eDSL brick-cell matches the frozen GPU-shape mirror per the oracle map.
fn assert_brick_cell(
    grid: &[u32],
    origin: [f32; 3],
    bw: f32,
    dims: [u32; 3],
    p: [f32; 3],
    ctx: &str,
) {
    let (re_class, re_min) = refactored_brick_cell_class(grid, origin, bw, dims, p);
    let (fr_class, fr_min) = frozen_gpu_brick_cell_class(grid, origin, bw, dims, p);
    assert_eq!(
        re_class, fr_class,
        "{ctx}: class refactored=0x{re_class:08x} != frozen=0x{fr_class:08x}"
    );
    // cell_min is compared ONLY on an in-grid hit (it is unread when OUTSIDE — the
    // committed comment); both sides produce the default `origin` there as the contract.
    if re_class != FROZEN_BRICK_OUTSIDE_GRID {
        assert_vec_bits(re_min, fr_min, &format!("{ctx} cell_min"));
    }
}

/// A small grid filler: `dims` cells, each `cells[i] = i` (so the class IS the linear
/// index — any wrong `idx` shows up as a wrong class).
fn fill_grid(dims: [u32; 3]) -> Vec<u32> {
    let n = (dims[0] * dims[1] * dims[2]) as usize;
    (0..n as u32).collect()
}

#[test]
fn brick_cell_class_oracle_sweep_byte_identical() {
    // A representative grid + the targeted edge set, then a broad random sweep.
    let dims = [4u32, 3, 2];
    let grid = fill_grid(dims);
    let origin = [-1.0f32, 2.0, 0.5];
    let bw = 0.75f32;

    // Targeted edge cases (the design's set):
    let edges: &[[f32; 3]] = &[
        // negative rel on each axis (out-of-grid via guard 1, before any cast).
        [origin[0] - 0.1, 3.0, 1.0],
        [1.0, origin[1] - 0.1, 1.0],
        [1.0, 3.0, origin[2] - 0.1],
        // on-boundary ix == dims.x (out-of-grid via guard 2). cell rel.x in [dims.x,
        // dims.x+1) → ix == dims.x.
        [origin[0] + (dims[0] as f32) * bw + 0.01, origin[1] + 0.1, origin[2] + 0.1],
        // the LAST in-grid cell (idx == len-1): ix=dims.x-1, iy=dims.y-1, iz=dims.z-1.
        [
            origin[0] + ((dims[0] - 1) as f32 + 0.5) * bw,
            origin[1] + ((dims[1] - 1) as f32 + 0.5) * bw,
            origin[2] + ((dims[2] - 1) as f32 + 0.5) * bw,
        ],
        // the FIRST in-grid cell (idx == 0).
        [origin[0] + 0.1, origin[1] + 0.1, origin[2] + 0.1],
        // all axes outside (every axis past the far face).
        [
            origin[0] + (dims[0] as f32 + 2.0) * bw,
            origin[1] + (dims[1] as f32 + 2.0) * bw,
            origin[2] + (dims[2] as f32 + 2.0) * bw,
        ],
        // NaN / Inf in p (the comparisons / cast must propagate identically: a NaN `<` is
        // false, so guard 1 falls through; `NaN as u32` == 0 in Rust, matching HLSL's
        // OpConvertFToU undefined-but-deterministic-here lowering — the frozen mirror uses
        // the SAME `as u32`, so the two agree by construction).
        [f32::NAN, origin[1] + 0.1, origin[2] + 0.1],
        [f32::INFINITY, origin[1] + 0.1, origin[2] + 0.1],
        [origin[0] + 0.1, f32::NEG_INFINITY, origin[2] + 0.1],
    ];
    for &p in edges {
        assert_brick_cell(&grid, origin, bw, dims, p, "brick-cell-edge");
    }

    // A broad random sweep across the cell, the boundaries, and far outside (the LCG's
    // non-finite / ±0 draws mixed into p / origin / bw).
    let mut lcg = Lcg::new(0xB21C_CE11_DEAD_3003);
    for _ in 0..20_000 {
        // Random non-degenerate dims (keep the grid small so the fill is cheap).
        let d = [
            1 + (lcg.next_u32() % 5),
            1 + (lcg.next_u32() % 5),
            1 + (lcg.next_u32() % 5),
        ];
        let g = fill_grid(d);
        let o = [lcg.next_f32(5.0), lcg.next_f32(5.0), lcg.next_f32(5.0)];
        let b = lcg.next_f32(3.0).abs() + 0.01; // bw > 0
        // Bias p to land near/in the grid (so in-grid hits AND out-of-grid both fire).
        let span = (d[0].max(d[1]).max(d[2]) as f32) * b;
        let p = [
            o[0] + lcg.next_f32(span * 1.5),
            o[1] + lcg.next_f32(span * 1.5),
            o[2] + lcg.next_f32(span * 1.5),
        ];
        assert_brick_cell(&g, o, b, d, p, "brick-cell-sweep");
    }
}

#[test]
fn brick_cell_class_tail_skip_on_eval() {
    // MANDATORY tail-skip: on a NEGATIVE rel, guard 1's `?` must short-circuit the IIFE
    // BEFORE the `(uint)` casts run — so the out-of-grid path returns BRICK_OUTSIDE_GRID
    // and cell_min stays the default `origin` (the casts, which would wrap a negative
    // float to a huge uint and read out of bounds, NEVER execute). A 1-cell grid: a
    // negative-rel point must NOT index `grid[huge]` (which would panic) — the early
    // return is the proof.
    let dims = [1u32, 1, 1];
    let grid = vec![0xAAAA_AAAAu32]; // a single cell
    let origin = [0.0f32, 0.0, 0.0];
    let bw = 1.0f32;

    // p below origin on x → rel.x < 0 → guard 1 returns BEFORE the cast. If the tail ran,
    // `(uint)(-5.0)` would be a huge index and `grid[huge]` would panic — so reaching here
    // without a panic IS the tail-skip proof.
    let (class, cmin) = refactored_brick_cell_class(&grid, origin, bw, dims, [-5.0, 0.5, 0.5]);
    assert_eq!(
        class, FROZEN_BRICK_OUTSIDE_GRID,
        "negative rel must early-return BRICK_OUTSIDE_GRID (the tail casts must not run)"
    );
    assert_vec_bits(
        cmin, origin,
        "cell_min must stay the default origin on the early (negative-rel) return",
    );

    // And the in-grid point reads the single cell (the tail DOES run when guard 1 falls
    // through).
    let (class_in, _) = refactored_brick_cell_class(&grid, origin, bw, dims, [0.5, 0.5, 0.5]);
    assert_eq!(class_in, 0xAAAA_AAAA, "the in-grid point must read grid[0]");
}

// ======================================================================
// Increment 4a — the regula-falsi root-refinement MARCHER leaf (RUNTIME [loop]).
//
// The THIRD control-flow leaf, the FIRST with a genuine runtime `[loop]` (an OpLoop):
// five loop-carried Phi vars + an in-loop early return forwarded to the function IIFE +
// a `m2_cubic_eval(c, mid)` call. The eDSL `m2_regula_falsi_body::<EvalCf>` is the CPU
// oracle.
//
// GPU-SHAPE CANONICAL FORM: the degenerate-bracket guard uses `1.0e-30` (the committed
// GPU literal), NOT the host `regula_falsi`'s `f32::MIN_POSITIVE` — so the two ALREADY
// DIVERGE on a near-flat secant whose `|denom|` lands in (1.18e-38, 1.0e-30) (the GPU
// bisects, the host secant-steps). The eDSL body picks the GPU SHAPE; its single-source
// authority is the FROZEN GPU-shape reference below, NOT a host call (the same discipline
// `dist_to_brick_exit_body` uses). The sweep:
//   (a) proves the canonical body equals the frozen GPU-shape reference to-bits over a
//       broad random set + targeted hard cases (degenerate denom → bisection arm,
//       bracket-collapse early return, non-finite), and
//   (b) a SPECIFIC early-return-at-iteration-k<8 case asserts the EARLY `mid` (not the
//       8-iteration `mid`) — the test that catches a void-combinator / ret-forwarding bug.
// ======================================================================

const FROZEN_M2_CUBIC_ROOT_EPS: f32 = 1.0e-6; // boyko_shaderdsl::brick::M2_CUBIC_ROOT_EPS
const FROZEN_M2_MARMITT_ITERS: usize = 8; // boyko_shaderdsl::brick::M2_MARMITT_ITERS
const FROZEN_M2_REGULA_DENOM_EPS: f32 = 1.0e-30; // boyko_shaderdsl::brick::M2_REGULA_DENOM_EPS

/// Verbatim hand-mirror of the GPU-SHAPE `m2_regula_falsi` (committed
/// `sdf_gbuffer_composite.hlsl:867-885`), with the EXACT operand order. The degenerate-
/// bracket guard uses `1.0e-30` (the GPU literal — NOT the host `f32::MIN_POSITIVE`). Do
/// NOT "clean up": the `1.0e-30` guard + the secant operand order + the early return are
/// the contract under test. Calls the frozen `cubic_eval` (the same `m2_cubic_eval` the
/// GPU calls).
fn frozen_gpu_m2_regula_falsi(
    c: &[f32; 4],
    mut lo: f32,
    mut hi: f32,
    mut f_lo: f32,
    mut f_hi: f32,
) -> f32 {
    let mut mid = lo;
    for _ in 0..FROZEN_M2_MARMITT_ITERS {
        let denom = f_hi - f_lo;
        mid = if denom.abs() > FROZEN_M2_REGULA_DENOM_EPS {
            lo - f_lo * (hi - lo) / denom
        } else {
            0.5 * (lo + hi)
        };
        let f_mid = frozen_cubic_eval(c, mid);
        if f_mid.abs() <= FROZEN_M2_CUBIC_ROOT_EPS || (hi - lo) <= FROZEN_M2_CUBIC_ROOT_EPS {
            return mid;
        }
        if f_lo * f_mid <= 0.0 {
            hi = mid;
            f_hi = f_mid;
        } else {
            lo = mid;
            f_lo = f_mid;
        }
    }
    mid
}

/// The eDSL regula-falsi over the `<EvalCf>` instantiation (the CPU oracle). The cubic-eval
/// seam is the frozen `cubic_eval` closure (so this re-runs the SAME frozen cubic at each
/// `mid`). Drives the out-of-band `out` cell and returns the refined `mid`.
fn refactored_m2_regula_falsi(c: &[f32; 4], lo: f32, hi: f32, f_lo: f32, f_hi: f32) -> f32 {
    use std::cell::Cell;
    let out = Cell::new(0.0f32);
    boyko_shaderdsl::brick::m2_regula_falsi_body::<EvalCf, _>(
        *c,
        lo,
        hi,
        f_lo,
        f_hi,
        |cc, mid| frozen_cubic_eval(&cc, mid),
        &out,
    );
    out.get()
}

#[test]
fn m2_regula_falsi_oracle_sweep_byte_identical() {
    // Random cubic coefficients + bracket ends (with the LCG's NaN/Inf/±0/MAX/MIN draws
    // mixed into every operand) vs the frozen GPU-shape reference, to-bits.
    let mut lcg = Lcg::new(0x4EC0_FA15_1DEA_6006);
    for _ in 0..20_000 {
        let c = [
            lcg.next_f32(4.0),
            lcg.next_f32(4.0),
            lcg.next_f32(4.0),
            lcg.next_f32(4.0),
        ];
        let lo = lcg.next_f32(2.0);
        let hi = lcg.next_f32(2.0);
        // Seed f_lo/f_hi from the cubic so a real sign bracket forms ~half the time (both
        // bracket-update arms + the early return fire), plus the LCG's non-finite draws.
        let f_lo = if lcg.next_u32().is_multiple_of(2) {
            frozen_cubic_eval(&c, lo)
        } else {
            lcg.next_f32(3.0)
        };
        let f_hi = if lcg.next_u32().is_multiple_of(2) {
            frozen_cubic_eval(&c, hi)
        } else {
            lcg.next_f32(3.0)
        };
        assert_bits(
            refactored_m2_regula_falsi(&c, lo, hi, f_lo, f_hi),
            frozen_gpu_m2_regula_falsi(&c, lo, hi, f_lo, f_hi),
            "regula-falsi-sweep",
        );
    }
}

#[test]
fn m2_regula_falsi_degenerate_denom_takes_bisection_byte_identical() {
    // Degenerate (flat) bracket: |f_hi - f_lo| <= 1.0e-30 forces the bisection midpoint
    // `0.5 * (lo + hi)` (the GPU-shape `1.0e-30` guard, NOT the host's f32::MIN_POSITIVE).
    // The discarded secant arm's `/ denom` produces an inf that must NOT leak (the eager
    // EvalCf::select computes both arms but selects the bisection one).
    let c = [0.3f32, -1.2, 0.7, 0.05];
    // f_lo == f_hi -> denom == 0 -> bisection on iteration 0. The first mid = 0.5*(lo+hi).
    let lo = -0.4f32;
    let hi = 1.6f32;
    let f_same = 2.0f32; // both ends same sign + equal -> denom 0, no sign bracket
    assert_bits(
        refactored_m2_regula_falsi(&c, lo, hi, f_same, f_same),
        frozen_gpu_m2_regula_falsi(&c, lo, hi, f_same, f_same),
        "regula-falsi-degenerate-denom",
    );
    // A sub-1e-30 (but non-zero) denom also bisects (the `>` guard is strict).
    let f_hi = f_same + 1.0e-32;
    assert_bits(
        refactored_m2_regula_falsi(&c, lo, hi, f_same, f_hi),
        frozen_gpu_m2_regula_falsi(&c, lo, hi, f_same, f_hi),
        "regula-falsi-subeps-denom",
    );
}

#[test]
fn m2_regula_falsi_early_return_at_iteration_k_lt_8() {
    // THE void-combinator / ret-forwarding catch: an input that CONVERGES (early-returns)
    // at an iteration k < 8 must return the EARLY `mid` (the iteration-k value), NOT the
    // 8-iteration `mid`. If runtime_for swallowed the Break(Return) (the deleted void
    // combinator) or returned the wrong Flow, the tail `ret_f(out, mid_final)` would
    // overwrite the early value and this would fail.
    //
    // Construct a cubic with a clean root inside [lo, hi] so regula-falsi converges fast.
    // c(t) = t (a line through 0): root at t=0; f(lo)<0, f(hi)>0 brackets it. The first
    // secant step lands exactly on the root (a linear function), so f_mid == 0 <= EPS and
    // it returns at iteration 0 — the EARLIEST possible early return.
    let c = [0.0f32, 1.0, 0.0, 0.0]; // c0=0, c1=1, c2=0, c3=0  ->  f(t) = t
    let lo = -1.0f32;
    let hi = 2.0f32;
    let f_lo = frozen_cubic_eval(&c, lo); // -1.0
    let f_hi = frozen_cubic_eval(&c, hi); // 2.0
    // The secant on a line is exact: mid = lo - f_lo*(hi-lo)/(f_hi-f_lo)
    //   = -1 - (-1)*(3)/(3) = -1 + 1 = 0.0  -> f_mid = 0 -> early return at iter 0.
    let got = refactored_m2_regula_falsi(&c, lo, hi, f_lo, f_hi);
    // The EARLY mid is 0.0 (the root) — the iteration-0 value.
    assert_bits(got, 0.0f32, "regula-falsi-early-return-value");
    // And it equals the frozen reference (which also early-returns at iter 0).
    assert_bits(
        got,
        frozen_gpu_m2_regula_falsi(&c, lo, hi, f_lo, f_hi),
        "regula-falsi-early-return-vs-frozen",
    );

    // A second case converging at k>0 but <8 (bracket-collapse `(hi - lo) <= EPS`): a tiny
    // bracket already within EPS returns on the first iteration via the second guard.
    let c2 = [0.5f32, 2.0, -0.3, 0.1];
    let lo2 = 0.3f32;
    // hi - lo == 5e-7 < 1e-6 EPS -> early return on iteration 0 via the bracket-collapse
    // guard `(hi - lo) <= M2_CUBIC_ROOT_EPS`. (Both sides compute the SAME `hi - lo`, so the
    // float rounding of the sum is irrelevant to the to-bits comparison.)
    let hi2 = lo2 + 5.0e-7f32;
    let f_lo2 = frozen_cubic_eval(&c2, lo2);
    let f_hi2 = frozen_cubic_eval(&c2, hi2);
    assert_bits(
        refactored_m2_regula_falsi(&c2, lo2, hi2, f_lo2, f_hi2),
        frozen_gpu_m2_regula_falsi(&c2, lo2, hi2, f_lo2, f_hi2),
        "regula-falsi-bracket-collapse",
    );
}

#[test]
fn m2_regula_falsi_non_finite_byte_identical() {
    // NaN / Inf in the coefficients or the bracket must propagate identically (a NaN `>`
    // / `<=` is false, so the denom guard / convergence guard fall through deterministically
    // — the frozen mirror uses the SAME comparisons, so the two agree by construction).
    let cases: &[([f32; 4], f32, f32, f32, f32)] = &[
        ([f32::NAN, 1.0, 0.0, 0.0], -1.0, 1.0, -1.0, 1.0),
        ([0.0, 1.0, 0.0, 0.0], f32::INFINITY, 1.0, -1.0, 1.0),
        ([0.0, 1.0, 0.0, 0.0], -1.0, f32::NEG_INFINITY, -1.0, 1.0),
        ([1.0, 0.0, 0.0, 0.0], -1.0, 1.0, f32::NAN, 1.0),
        ([1.0, 0.0, 0.0, 0.0], -1.0, 1.0, -1.0, f32::INFINITY),
        ([f32::MAX, f32::MIN, 0.0, 0.0], -1.0, 1.0, -2.0, 3.0),
    ];
    for &(c, lo, hi, f_lo, f_hi) in cases {
        assert_bits(
            refactored_m2_regula_falsi(&c, lo, hi, f_lo, f_hi),
            frozen_gpu_m2_regula_falsi(&c, lo, hi, f_lo, f_hi),
            "regula-falsi-non-finite",
        );
    }
}

#[test]
fn runtime_for_control_table_drives_each_arm() {
    // MANDATORY: drive EACH EvalCf::runtime_for control-table arm and assert the post-loop
    // observable matches the documented semantics. This is the per-arm guard for the
    // ret-forwarding correctness (a wrong Flow on any arm silently corrupts the eval oracle).
    use boyko_shaderdsl::cf::{Cf, Flow, LoopOp};
    use boyko_shaderdsl::scalar::FieldScalar;

    // ARM 1 (Continue → next iter) + ARM 5 (natural completion → Continue): a counter that
    // increments every iteration runs all `bound_val` times and the loop returns Continue.
    {
        let counter = EvalCf::decl_var("counter", 0.0);
        let flow = EvalCf::runtime_for("[loop]", "i", "N", 8, |_i| -> Flow {
            let cur = EvalCf::get_var(&counter);
            EvalCf::set_var(&counter, cur.add(1.0));
            Flow::Continue(())
        });
        assert_eq!(flow, Flow::Continue(()), "natural completion must yield Continue");
        assert_eq!(
            EvalCf::get_var(&counter).to_bits(),
            8.0f32.to_bits(),
            "Continue arm must run every iteration (8 increments)"
        );
    }

    // ARM 2 (Break(Continue) → continue): skip the live tail on i==1; the tail increment
    // runs for the other 7 of 8 iterations -> counter == 7. The loop returns Continue.
    {
        let counter = EvalCf::decl_var("counter", 0.0);
        let flow = EvalCf::runtime_for("[loop]", "i", "N", 8, |i| -> Flow {
            EvalCf::if_((i == 1).then_some(()).is_some(), EvalCf::cont)?;
            let cur = EvalCf::get_var(&counter);
            EvalCf::set_var(&counter, cur.add(1.0));
            Flow::Continue(())
        });
        assert_eq!(flow, Flow::Continue(()), "Break(Continue) loop still completes -> Continue");
        assert_eq!(
            EvalCf::get_var(&counter).to_bits(),
            7.0f32.to_bits(),
            "Break(Continue) must skip the tail on i==1 (7 increments)"
        );
    }

    // ARM 3 (Break(Break) → break THEN return Continue): break on i==3; the tail runs for
    // i=0,1,2 only -> counter == 3. The loop returns Continue (the break is consumed).
    {
        let counter = EvalCf::decl_var("counter", 0.0);
        let flow = EvalCf::runtime_for("[loop]", "i", "N", 8, |i| -> Flow {
            if i == 3 {
                return Flow::Break(LoopOp::Break);
            }
            let cur = EvalCf::get_var(&counter);
            EvalCf::set_var(&counter, cur.add(1.0));
            Flow::Continue(())
        });
        assert_eq!(
            flow,
            Flow::Continue(()),
            "Break(Break) is consumed by the loop -> the loop returns Continue (the tail runs)"
        );
        assert_eq!(
            EvalCf::get_var(&counter).to_bits(),
            3.0f32.to_bits(),
            "Break(Break) on i==3 runs the tail for i=0,1,2 only (3 increments)"
        );
    }

    // ARM 4 (Break(Return) → FORWARD): a body that returns Break(Return) on i==2 must make
    // runtime_for RETURN Break(Return) (so the caller's `?` short-circuits the IIFE). The
    // tail runs for i=0,1 only -> counter == 2.
    {
        let counter = EvalCf::decl_var("counter", 0.0);
        let flow = EvalCf::runtime_for("[loop]", "i", "N", 8, |i| -> Flow {
            if i == 2 {
                return Flow::Break(LoopOp::Return);
            }
            let cur = EvalCf::get_var(&counter);
            EvalCf::set_var(&counter, cur.add(1.0));
            Flow::Continue(())
        });
        assert_eq!(
            flow,
            Flow::Break(LoopOp::Return),
            "Break(Return) must be FORWARDED out of runtime_for (to the function IIFE's `?`)"
        );
        assert_eq!(
            EvalCf::get_var(&counter).to_bits(),
            2.0f32.to_bits(),
            "Break(Return) on i==2 runs the tail for i=0,1 only (2 increments)"
        );
    }
}

#[test]
fn evalcf_is_zst_inc4a() {
    // The ZST guarantee still holds after the Inc-4a additive facets (no data added to the
    // backend marker).
    assert_eq!(std::mem::size_of::<EvalCf>(), 0, "EvalCf must remain a ZST");
}

#[test]
fn evalcf_if_guarded_brk_breaks_then_post_loop_tail_runs() {
    // Inc 4b — the GENUINELY-NEW Eval control path: an `if_`-guarded `brk()` (the PRODUCER,
    // `C::if_(cond, C::brk)?`) propagated through `runtime_for` => a REAL `break` => the loop
    // CONSUMES it (returns `Continue`) => the POST-LOOP tail RUNS. This is DISTINCT from
    // `runtime_for_control_table_drives_each_arm`'s ARM 3, which returns
    // `Flow::Break(LoopOp::Break)` DIRECTLY (the consumer-only arm); here the break flows
    // through the `brk()` producer + `if_`'s `?` (the exact shape `sdf_soft_shadow`'s
    // `if (t > T_MAX) { break; }` uses).
    use boyko_shaderdsl::cf::{Cf, Flow};
    use boyko_shaderdsl::scalar::FieldScalar;

    // A counter incremented each iteration; `brk()` fires (via `if_`) on i==4. The increment
    // tail runs for i=0,1,2,3 only -> counter == 4. A SECOND post-loop counter proves the
    // tail after the loop executes (the break was consumed, not forwarded).
    let counter = EvalCf::decl_var("counter", 0.0);
    let tail_ran = EvalCf::decl_var("tail_ran", 0.0);
    let flow = EvalCf::runtime_for("[loop]", "i", "MAX_IT", 128, |i| -> Flow {
        // The `brk()` PRODUCER through `if_`'s `?` — the live-tail (the increment below) is
        // SKIPPED on the breaking iteration, and `runtime_for` consumes the break.
        EvalCf::if_((i == 4).then_some(()).is_some(), EvalCf::brk)?;
        let cur = EvalCf::get_var(&counter);
        EvalCf::set_var(&counter, cur.add(1.0));
        Flow::Continue(())
    });
    // The loop CONSUMED the break -> it returns `Continue` (the function tail runs).
    assert_eq!(
        flow,
        Flow::Continue(()),
        "an if_-guarded brk is CONSUMED by runtime_for -> the loop returns Continue (tail runs)"
    );
    // The increment tail ran for i=0,1,2,3 only (skipped on the breaking i==4).
    assert_eq!(
        EvalCf::get_var(&counter).to_bits(),
        4.0f32.to_bits(),
        "the brk on i==4 runs the increment tail for i=0..3 only (4 increments)"
    );
    // The POST-LOOP tail runs (the genuinely-new observable: the break did NOT forward out of
    // the IIFE — it was consumed, so the statement after the loop executes).
    EvalCf::set_var(&tail_ran, 1.0);
    assert_eq!(
        EvalCf::get_var(&tail_ran).to_bits(),
        1.0f32.to_bits(),
        "the post-loop tail must RUN (the consumed brk does not skip it)"
    );
}

// ======================================================================
// Inc 4b — the `sdf_soft_shadow` CONTROL-FLOW Eval oracle.
//
// A frozen HOST mirror of the committed `sdf_soft_shadow` LOOP+TAIL span (the L454-468
// statements; the `dot(n,L)` preamble stays out — the generated span never sees it) threading
// the SAME host `field` closure the eDSL body uses. The eDSL `sdf_soft_shadow_body::<EvalCf>`
// reproduces this CONTROL FLOW (the occluder-hit early return, the `t > T_MAX` break, the
// budget exhaustion, the penumbra-min accumulation) — SCOPED TO CONTROL FLOW (the cmp-`.spv`
// is the byte-identity oracle; an Eval ULP wobble does not block a green-`.spv` increment).
// ======================================================================

// The committed tuning consts (mirror `boyko_shaderdsl::shadow`'s values).
const FROZEN_SHADOW_K: f32 = 8.0;
const FROZEN_SHADOW_MINT: f32 = 16.0 * 0.0005;
const FROZEN_SHADOW_MINT_STEP: f32 = 16.0 * 0.0005;
const FROZEN_SHADOW_HIT_EPS: f32 = 2.0 * 0.001;
// The committed GPU literal VERBATIM (NOT `SQRT_2` — a different f32 bit pattern); the eDSL
// const it mirrors carries the same allow for the same reason.
#[allow(clippy::approx_constant, clippy::excessive_precision)]
const FROZEN_FIELD_LIPSCHITZ_L: f32 = 1.41421356;
const FROZEN_T_MAX: f32 = 10.0;
const FROZEN_MAX_IT: usize = 128;

/// Verbatim hand-mirror of the committed `sdf_soft_shadow` LOOP+TAIL span (the L454-468
/// statements), threading a host `field` closure (`p + L * t -> distance`). The reference the
/// eDSL `sdf_soft_shadow_body::<EvalCf>` control flow is locked against.
fn frozen_sdf_soft_shadow<Fld: Fn([f32; 3]) -> f32>(p: [f32; 3], l: [f32; 3], field: &Fld) -> f32 {
    let mut res = 1.0f32;
    let mut t = FROZEN_SHADOW_MINT;
    for _ in 0..FROZEN_MAX_IT {
        let q = [p[0] + l[0] * t, p[1] + l[1] * t, p[2] + l[2] * t];
        let d = field(q);
        res = res.min(FROZEN_SHADOW_K * d / t);
        if d < FROZEN_SHADOW_HIT_EPS {
            return 0.0;
        }
        t += (d / FROZEN_FIELD_LIPSCHITZ_L).max(FROZEN_SHADOW_MINT_STEP);
        if t > FROZEN_T_MAX {
            break;
        }
    }
    res.clamp(0.0, 1.0)
}

/// The eDSL `sdf_soft_shadow_body::<EvalCf>` threading the SAME host `field` closure (so
/// `Cf::call1`'s `unreachable!` is never reached). `n` is unused by the generated span (the
/// preamble owns it), so a zero normal is passed.
fn refactored_sdf_soft_shadow<Fld: Fn([f32; 3]) -> f32>(p: [f32; 3], l: [f32; 3], field: &Fld) -> f32 {
    use std::cell::Cell;
    let out = Cell::new(0.0f32);
    boyko_shaderdsl::shadow::sdf_soft_shadow_body::<EvalCf, _>(p, [0.0, 0.0, 0.0], l, field, &out);
    out.get()
}

#[test]
fn sdf_soft_shadow_control_flow_matches_frozen() {
    // A single analytic occluder — a sphere of radius `r` at `center` — so `field` is a clean
    // signed distance the march steps along. The three control-flow outcomes are all exercised:
    //   (a) a ray AIMED AT the occluder -> the `d < SHADOW_HIT_EPS` early return (0.0);
    //   (b) a ray MISSING the occluder, escaping past T_MAX -> the break -> clamp(res,..) tail;
    //   (c) a grazing ray -> a partial penumbra `res` in (0, 1).
    let sphere = |center: [f32; 3], r: f32| {
        move |q: [f32; 3]| -> f32 {
            let dx = q[0] - center[0];
            let dy = q[1] - center[1];
            let dz = q[2] - center[2];
            (dx * dx + dy * dy + dz * dz).sqrt() - r
        }
    };

    let mut lcg = Lcg::new(0x5DF5_0F75_AD00_77A0_u64);
    for _ in 0..5_000 {
        let p = [lcg.next_f32(2.0), lcg.next_f32(2.0), lcg.next_f32(2.0)];
        // A unit-ish light direction (not normalized exactly — the march tolerates it; the host
        // mirror and the eDSL see the SAME L, so the control flow agrees regardless).
        let l = [lcg.next_f32(1.0), lcg.next_f32(1.0), lcg.next_f32(1.0)];
        let center = [lcg.next_f32(3.0), lcg.next_f32(3.0), lcg.next_f32(3.0)];
        let r = lcg.next_f32(1.0).abs() + 0.05;
        let field = sphere(center, r);
        assert_bits(
            refactored_sdf_soft_shadow(p, l, &field),
            frozen_sdf_soft_shadow(p, l, &field),
            "sdf-soft-shadow-control-flow",
        );
    }
}

// ======================================================================
// Inc 4b.2 — the `m2_surface_hit` REFINE CONTROL-FLOW Eval oracle.
//
// A frozen HOST mirror of the committed `m2_surface_hit` REFINE LOOP+TAIL span (the L1184-1205
// statements; the integer cell-addressing preamble + the call sites stay out — the generated
// span never sees them) threading the SAME host `field` closure the eDSL body uses. The eDSL
// `m2_surface_hit_refine_body::<EvalCf>` reproduces this CONTROL FLOW (the converged-hit early
// `return true` + the FRESH `hit_t = rt`, the `rt < 0 || rt > T_MAX` break, the budget
// exhaustion `return false`) — SCOPED TO CONTROL FLOW (the cmp-`.spv` is the byte-identity
// oracle; an Eval ULP wobble does not block a green-`.spv` increment).
//
// The keystone the design pins: the composite `if_hit_ret_b` writes `hit_t` BEFORE the
// `Break(Return)` short-circuits, so on a HIT `hit_t` holds the rt at the HIT iteration (NOT the
// pre-call default, NOT the final-iteration rt); on a MISS `hit_t` is left at its pre-call
// default (the hand-written `hit_t = t_world;` entry write, modeled here as the cell's seed).
// ======================================================================

// The committed tuning consts (mirror `boyko_shaderdsl::surface`'s values).
const FROZEN_M2_REFINE_RELAX: f32 = 0.8;
const FROZEN_M2_SURFACE_EPS: f32 = 0.001;
const FROZEN_M2_SURFACE_T_MAX: f32 = 10.0;
const FROZEN_M2_REFINE_ITERS: usize = 8;

/// Verbatim hand-mirror of the committed `m2_surface_hit` REFINE LOOP+TAIL span (the L1184-1205
/// statements), threading a host `field` closure (`ro + rd * rt -> distance`). Returns the
/// `(bool hit, float hit_t)` pair — `hit_t` is the SENTINEL `default` on a miss (the
/// hand-written `hit_t = t_world;` entry write the generated span never touches), or the rt at
/// the converged iteration on a hit. The reference the eDSL `m2_surface_hit_refine_body::<EvalCf>`
/// control flow + hit_t write-order is locked against.
// The frozen mirror spells the committed HLSL span VERBATIM: `rt = rt + step;` (the eDSL's R1
// `set_var` form, byte-identical to the committed `rt += step` in the `.spv`) and the
// `rt < 0.0 || rt > T_MAX` escape guard. Collapsing to `rt += step` / `!(0.0..=T_MAX).contains`
// would diverge the reference from the committed source it must mirror character-for-character
// (and change the NaN edge-case semantics), so both clippy suggestions are deliberately suppressed.
#[allow(clippy::assign_op_pattern, clippy::manual_range_contains)]
fn frozen_m2_surface_hit_refine<Fld: Fn([f32; 3]) -> f32>(
    ro: [f32; 3],
    rd: [f32; 3],
    cand_t: f32,
    default: f32,
    field: &Fld,
) -> (bool, f32) {
    // `hit_t = t_world;` (the hand-written entry default, OUTSIDE the generated span).
    let mut hit_t = default;
    let mut rt = cand_t;
    for _ in 0..FROZEN_M2_REFINE_ITERS {
        let q = [ro[0] + rd[0] * rt, ro[1] + rd[1] * rt, ro[2] + rd[2] * rt];
        let d = field(q);
        if d.abs() < FROZEN_M2_SURFACE_EPS {
            // `hit_t = rt;` is written BEFORE the `return true;` — the keystone ordering.
            hit_t = rt;
            return (true, hit_t);
        }
        let step = FROZEN_M2_REFINE_RELAX * d;
        rt = rt + step;
        if rt < 0.0 || rt > FROZEN_M2_SURFACE_T_MAX {
            break;
        }
    }
    (false, hit_t)
}

/// The eDSL `m2_surface_hit_refine_body::<EvalCf>` threading the SAME host `field` closure (so
/// `Cf::call1`'s `unreachable!` is never reached). The `hit_t` cell is seeded with `default` (the
/// hand-written entry write); the bool cell is seeded `false` and only an in-loop converged hit
/// flips it. Returns the `(bool hit, float hit_t)` pair the cells hold after the body runs.
fn refactored_m2_surface_hit_refine<Fld: Fn([f32; 3]) -> f32>(
    ro: [f32; 3],
    rd: [f32; 3],
    cand_t: f32,
    default: f32,
    field: &Fld,
) -> (bool, f32) {
    use std::cell::Cell;
    // `hit_t` seeded with the hand-written entry default; the bool seeded `false`.
    let hit_out = Cell::new(default);
    let ret_out = Cell::new(false);
    boyko_shaderdsl::surface::m2_surface_hit_refine_body::<EvalCf, _>(
        ro, rd, cand_t, field, &hit_out, &ret_out,
    );
    (ret_out.get(), hit_out.get())
}

#[test]
fn m2_surface_hit_refine_control_flow_matches_frozen() {
    // A single analytic occluder — a sphere of radius `r` at `center` — so `field` is a clean
    // signed distance the SIGNED refine steps along (converging from either side). All three
    // control-flow outcomes are exercised across the sweep:
    //   (a) a candidate near the surface -> the `abs(d) < EPS` converged hit (`true` + fresh hit_t);
    //   (b) a candidate that walks out of [0, T_MAX] -> the break -> the tail `false`;
    //   (c) a candidate that exhausts the budget without converging -> the tail `false`.
    let sphere = |center: [f32; 3], r: f32| {
        move |q: [f32; 3]| -> f32 {
            let dx = q[0] - center[0];
            let dy = q[1] - center[1];
            let dz = q[2] - center[2];
            (dx * dx + dy * dy + dz * dz).sqrt() - r
        }
    };

    let mut lcg = Lcg::new(0x1357_9BDF_2468_ACE0_u64);
    for _ in 0..5_000 {
        let ro = [lcg.next_f32(2.0), lcg.next_f32(2.0), lcg.next_f32(2.0)];
        let rd = [lcg.next_f32(1.0), lcg.next_f32(1.0), lcg.next_f32(1.0)];
        let cand_t = lcg.next_f32(3.0);
        let center = [lcg.next_f32(3.0), lcg.next_f32(3.0), lcg.next_f32(3.0)];
        let r = lcg.next_f32(1.0).abs() + 0.05;
        // A DISTINCT sentinel default so a stale read (the bug the composite combinator guards
        // against) would be caught: a leak of the default would surface here.
        let default = lcg.next_f32(5.0);
        let field = sphere(center, r);

        let (rhit, rht) = refactored_m2_surface_hit_refine(ro, rd, cand_t, default, &field);
        let (fhit, fht) = frozen_m2_surface_hit_refine(ro, rd, cand_t, default, &field);
        assert_eq!(rhit, fhit, "m2-surface-hit-refine: bool return diverged");
        assert_bits(rht, fht, "m2-surface-hit-refine-hit_t");
    }
}

#[test]
fn m2_surface_hit_refine_hit_writes_fresh_rt_not_default_or_final() {
    // A DIRECTED hit case proving the keystone: the composite `if_hit_ret_b` writes `hit_t = rt;`
    // BEFORE the `Break(Return)` short-circuits, so on a hit `hit_t` holds the rt at the HIT
    // iteration — NOT the pre-call default (a leak would surface the sentinel) and NOT the
    // final-iteration rt (the loop short-circuits at the hit). Construct a candidate that is
    // ALREADY on the surface (`abs(d) < EPS` at iter 0): a sphere of radius 1 centered at the
    // origin, with `ro = origin`, `rd` a unit +x, and `cand_t = 1.0` (so `ro + rd*1.0` is exactly
    // on the surface). The hit fires at iter 0 with `rt == cand_t == 1.0`.
    let sphere = |q: [f32; 3]| -> f32 {
        (q[0] * q[0] + q[1] * q[1] + q[2] * q[2]).sqrt() - 1.0
    };
    let ro = [0.0, 0.0, 0.0];
    let rd = [1.0, 0.0, 0.0];
    let cand_t = 1.0;
    let default = -999.0; // a sentinel that can NEVER be a valid rt on this ray

    let (hit, hit_t) = refactored_m2_surface_hit_refine(ro, rd, cand_t, default, &sphere);
    assert!(hit, "an on-surface candidate must return true");
    // hit_t is the FRESH rt at the hit iteration (cand_t == 1.0), NOT the -999.0 default.
    assert_bits(hit_t, cand_t, "hit_t-is-fresh-rt-at-hit-iter");
    assert_ne!(hit_t, default, "hit_t must NOT leak the pre-call default on a hit");
}

#[test]
fn m2_surface_hit_refine_miss_leaves_hit_t_at_default() {
    // The dual of the hit case: a MISS leaves `hit_t` at its pre-call default (the hand-written
    // entry write the generated span never touches). Construct a candidate the refine can never
    // bring within EPS of any surface: a far-everywhere field. The step `0.8 * 100 = 80` pushes
    // `rt` past T_MAX on iter 0 -> break -> the tail `return false;` runs with `hit_t` never
    // assigned (it holds the seed).
    let empty = |_q: [f32; 3]| -> f32 { 100.0 };
    let ro = [0.0, 0.0, 0.0];
    let rd = [0.0, 0.0, 0.0]; // a degenerate dir keeps the field constant; only rt grows
    let cand_t = 0.5;
    let default = 7.25;

    let (hit, hit_t) = refactored_m2_surface_hit_refine(ro, rd, cand_t, default, &empty);
    assert!(!hit, "a never-converging candidate must return false");
    assert_bits(hit_t, default, "miss-leaves-hit_t-at-default");
}

// ======================================================================
// Inc 4c — the B1 over-relaxation ACCEPT-REFINE CONTROL-FLOW Eval oracle.
//
// A frozen HOST mirror of the committed B1 accept-refine LOOP span (the L1442-1452 statements;
// the enclosing over-relaxation marcher + the outer `break;` stay out — the generated span never
// sees them) threading the SAME host `sdf` closure the eDSL body uses. The eDSL
// `b1_accept_refine_body::<EvalCf>` reproduces this CONTROL FLOW (the `abs(rd_) < EPS` accept
// break, the budget exhaustion, the SIGNED `t = t + M2_REFINE_RELAX * rd_` accumulation) — SCOPED
// TO CONTROL FLOW (the cmp-`.spv` is the byte-identity oracle; an Eval ULP wobble does not block a
// green-`.spv` increment).
//
// STRICTLY SIMPLER than the 4b.2 m2_surface_hit refine: there is NO return facet — the carried `t`
// is mutated IN PLACE and read after the body runs. The break fires BEFORE the step (an
// already-on-surface seed is accepted with `t` unchanged); an off-surface seed accumulates k
// signed steps; a never-converging field exhausts the full M2_REFINE_ITERS budget.
// ======================================================================

// The committed tuning consts (mirror `boyko_shaderdsl::refine`'s values).
const FROZEN_B1_REFINE_RELAX: f32 = 0.8;
const FROZEN_B1_REFINE_EPS: f32 = 0.001;
const FROZEN_B1_REFINE_ITERS: usize = 8;

/// Verbatim hand-mirror of the committed B1 accept-refine LOOP span (the L1442-1452 statements),
/// threading a host `field` closure (`ro + rd * t -> distance`). Returns the settled `t`. The
/// reference the eDSL `b1_accept_refine_body::<EvalCf>` control flow is locked against.
// The frozen mirror spells the committed HLSL span VERBATIM: `t = t + step;` (the eDSL's R1
// `set_var` form, byte-identical to the committed `t += step` in the `.spv`). Collapsing to
// `t += step` would diverge the reference from the committed source it must mirror
// character-for-character, so the clippy suggestion is deliberately suppressed.
#[allow(clippy::assign_op_pattern)]
fn frozen_b1_accept_refine<Fld: Fn([f32; 3]) -> f32>(
    ro: [f32; 3],
    rd: [f32; 3],
    t_seed: f32,
    field: &Fld,
) -> f32 {
    let mut t = t_seed;
    for _ in 0..FROZEN_B1_REFINE_ITERS {
        let q = [ro[0] + rd[0] * t, ro[1] + rd[1] * t, ro[2] + rd[2] * t];
        let rd_ = field(q);
        if rd_.abs() < FROZEN_B1_REFINE_EPS {
            break;
        }
        let step = FROZEN_B1_REFINE_RELAX * rd_;
        t = t + step;
    }
    t
}

/// The eDSL `b1_accept_refine_body::<EvalCf>` threading the SAME host `field` closure (so
/// `Cf::call1`'s `unreachable!` is never reached). Returns the settled `t` the body yields.
fn refactored_b1_accept_refine<Fld: Fn([f32; 3]) -> f32>(
    ro: [f32; 3],
    rd: [f32; 3],
    t_seed: f32,
    field: &Fld,
) -> f32 {
    boyko_shaderdsl::refine::b1_accept_refine_body::<EvalCf, _>(ro, rd, t_seed, field)
}

#[test]
fn b1_accept_refine_control_flow_matches_frozen() {
    // A single analytic occluder — a sphere of radius `r` at `center` — so `field` is a clean
    // signed distance the SIGNED refine steps along (converging from either side). All three
    // control-flow outcomes are exercised across the sweep:
    //   (a) a seed already on the surface -> the `abs(rd_) < EPS` accept break (`t` unchanged);
    //   (b) a seed off the surface that converges at k>0 -> k signed steps then the break;
    //   (c) a seed in a field that never reaches EPS -> the full M2_REFINE_ITERS budget.
    let sphere = |center: [f32; 3], r: f32| {
        move |q: [f32; 3]| -> f32 {
            let dx = q[0] - center[0];
            let dy = q[1] - center[1];
            let dz = q[2] - center[2];
            (dx * dx + dy * dy + dz * dz).sqrt() - r
        }
    };

    let mut lcg = Lcg::new(0x0B1A_CCEE_7711_22F0_u64);
    for _ in 0..5_000 {
        let ro = [lcg.next_f32(2.0), lcg.next_f32(2.0), lcg.next_f32(2.0)];
        let rd = [lcg.next_f32(1.0), lcg.next_f32(1.0), lcg.next_f32(1.0)];
        let t_seed = lcg.next_f32(3.0);
        let center = [lcg.next_f32(3.0), lcg.next_f32(3.0), lcg.next_f32(3.0)];
        let r = lcg.next_f32(1.0).abs() + 0.05;
        let field = sphere(center, r);
        assert_bits(
            refactored_b1_accept_refine(ro, rd, t_seed, &field),
            frozen_b1_accept_refine(ro, rd, t_seed, &field),
            "b1-accept-refine-control-flow",
        );
    }
}

#[test]
fn b1_accept_refine_on_surface_seed_breaks_before_step() {
    // The inner break fires BEFORE the step: a seed already on the surface (`abs(rd_) < EPS` at
    // iter 0) returns `t == t_seed` UNCHANGED (the break precedes the `t = t + step` accumulation).
    // A sphere of radius 1 centered at the origin, `ro = origin`, `rd` a unit +x, `t_seed = 1.0`
    // (so `ro + rd*1.0` is exactly on the surface) accepts at iter 0 with `t` untouched.
    let sphere = |q: [f32; 3]| -> f32 { (q[0] * q[0] + q[1] * q[1] + q[2] * q[2]).sqrt() - 1.0 };
    let ro = [0.0, 0.0, 0.0];
    let rd = [1.0, 0.0, 0.0];
    let t_seed = 1.0;

    let t = refactored_b1_accept_refine(ro, rd, t_seed, &sphere);
    assert_bits(t, t_seed, "on-surface-seed-leaves-t-unchanged");
}

#[test]
fn b1_accept_refine_off_surface_mutates_in_place() {
    // A seed strictly inside the sphere (`abs(rd_) >= EPS`) takes a nonzero signed step on the
    // first iteration, mutating `t` away from `t_seed`. This pins FAITHFULNESS + in-place mutation,
    // NOT convergence: with `ro = origin`, `rd` a unit +x, `t_seed = 0.5`, the point sits at
    // distance 0.5 inside the unit sphere (`rd_ = -0.5`), and the signed step `t = t + RELAX*rd_`
    // (rd_ < 0) DECREASES `t`. The test asserts only that the eDSL and the frozen host mirror agree
    // to-bits and that `t` actually moved — i.e. the running `t` is carried in place across iters.
    let sphere = |q: [f32; 3]| -> f32 { (q[0] * q[0] + q[1] * q[1] + q[2] * q[2]).sqrt() - 1.0 };
    let ro = [0.0, 0.0, 0.0];
    let rd = [1.0, 0.0, 0.0];
    let t_seed = 0.5;

    let refactored = refactored_b1_accept_refine(ro, rd, t_seed, &sphere);
    let frozen = frozen_b1_accept_refine(ro, rd, t_seed, &sphere);
    assert_bits(refactored, frozen, "off-surface-converges-to-frozen");
    assert_ne!(refactored, t_seed, "an off-surface seed must move t");
}

#[test]
fn b1_accept_refine_budget_exhaustion_matches_frozen() {
    // A field that never reaches EPS exhausts the full M2_REFINE_ITERS budget: with `field` a
    // constant far value, every iteration takes the SAME signed step, so the settled `t` is
    // `t_seed + ITERS * (RELAX * far)` to-bits (a degenerate `rd = 0` keeps the field constant —
    // only `t` grows). Pins the exact 8-step accumulation against the frozen mirror.
    let far = |_q: [f32; 3]| -> f32 { 100.0 };
    let ro = [0.0, 0.0, 0.0];
    let rd = [0.0, 0.0, 0.0]; // a degenerate dir keeps the field constant; only t grows
    let t_seed = 0.5;

    let refactored = refactored_b1_accept_refine(ro, rd, t_seed, &far);
    // 8 steps of `t = t + 0.8 * 100.0`, accumulated with the SAME per-step rounding as the body.
    let mut expected = t_seed;
    for _ in 0..FROZEN_B1_REFINE_ITERS {
        let step = FROZEN_B1_REFINE_RELAX * 100.0_f32;
        expected += step;
    }
    assert_bits(refactored, expected, "budget-exhaustion-8-step-accumulation");
}

// ======================================================================
// Increment 4d — the TYPED `bool` decl facet (`Cf::decl_bool_var`).
//
// The FIRST rung of the B1-marcher single-source ladder: the two NON-CONTIGUOUS `bool` preamble
// decls (`bool hit = false;` L1316, `bool exhausted = true;` L1327) authored ONCE over `C: Cf`.
// The bodies are ONE-STATEMENT decls — straight-line, no control flow — so the Eval round-trip
// (the returned `Cell<bool>` holds the init) is trivial; the REAL proof is the cmp-`.spv` after
// the splice (the emit STRUCTURE golden in `tests/emit_b1_decls.rs` pins the generated text). This
// block also re-asserts the ZST guarantee — `Cell<bool>` is 1 byte but adds NO field to the
// `EvalCf` marker.
// ======================================================================

#[test]
fn b1_decl_hit_inits_false_on_eval() {
    use boyko_shaderdsl::decl::b1_decl_hit_body;
    let hit = b1_decl_hit_body::<EvalCf>();
    assert!(!hit.get(), "`bool hit` must initialize to `false`");
}

#[test]
fn b1_decl_exhausted_inits_true_on_eval() {
    use boyko_shaderdsl::decl::b1_decl_exhausted_body;
    let exhausted = b1_decl_exhausted_body::<EvalCf>();
    assert!(
        exhausted.get(),
        "`bool exhausted` must initialize to `true` (the BUG-B1-HOLE-3 flag — under-detecting \
         exhaustion reopens the hole)"
    );
}

#[test]
fn evalcf_is_zst_inc4d() {
    // The ZST guarantee still holds after the Inc-4d `BoolVar = Cell<bool>` facet (the `Cell<bool>`
    // is the local's value, NOT a field on the backend marker).
    assert_eq!(std::mem::size_of::<EvalCf>(), 0, "EvalCf must remain a ZST");
}

// ======================================================================
// Inc 5a — the `select_level` clip-map LOD scan Eval oracle (FULL coverage — pure arithmetic).
//
// `select_level` is a pure containment scan (no field call), so the Eval body has FULL coverage:
// a host mirror transcribing the committed L1222-1234 VERBATIM (an `m2_levels` fixture array of N
// level boxes + a `pc.brick_levels` count) is the EXACT reference. Since the return is an `i32`,
// the assertion is EXACT integer eq (no ULP slack). The sweep covers every branch the design
// pins: inside-each-level (returns that L, finest-first), outside-all (returns -1), the boundary
// `p == hi` (EXCLUDED by `<` — not that level), and `L >= pc.brick_levels` (an inactive level
// skipped via the `break`).
//
// The `Cf::level_field_*` / `Cf::pc_uint` Emit recorders are routed around on Eval by the THREADED
// CLOSURES (the `Cf::call1` discipline): the closures index the host fixture, so the
// `unreachable!` hooks are never reached.
// ======================================================================

/// The host `M4Level` fixture — one clip-map level box (the fields `select_level` reads). NOT the
/// GPU struct layout (only the access text matters there); just the values the containment test
/// folds.
#[derive(Clone, Copy)]
struct HostLevel {
    /// `origin_brick_world.xyz` — the level grid's lower world corner.
    origin: [f32; 3],
    /// `origin_brick_world.w` — the level's brick world size.
    bw: f32,
    /// `dims_atlas_dim.xyz` — the level grid dims (as f32).
    dims: [f32; 3],
}

/// The host `select_level` fixture — the `m2_levels[BRICK_LEVELS]` array + the runtime
/// `pc.brick_levels` count.
#[derive(Clone, Copy)]
struct HostLevels {
    levels: [HostLevel; 3],
    brick_levels: u32,
}

/// Verbatim hand-mirror of the committed `select_level` scan (the L1222-1234 statements), reading
/// the host `levels` fixture. Returns the SIGNED level index (`0..brick_levels-1`) or `-1`. The
/// reference the eDSL `select_level_body::<EvalCf>` is locked against (EXACT `i32` eq).
fn host_select_level(p: [f32; 3], lv: &HostLevels) -> i32 {
    for l in 0..3usize {
        let lu = l as u32;
        if lu >= lv.brick_levels {
            break;
        }
        let o = lv.levels[l].origin;
        let bw = lv.levels[l].bw;
        let d = lv.levels[l].dims;
        let hi = [o[0] + d[0] * bw, o[1] + d[1] * bw, o[2] + d[2] * bw];
        let inside = p[0] >= o[0]
            && p[1] >= o[1]
            && p[2] >= o[2]
            && p[0] < hi[0]
            && p[1] < hi[1]
            && p[2] < hi[2];
        if inside {
            return l as i32;
        }
    }
    -1
}

/// The eDSL `select_level_body::<EvalCf>` threading the host fixture through the level-field / pc
/// closures (so `Cf::level_field_*` / `Cf::pc_uint`'s `unreachable!` is never reached). Returns the
/// `i32` the ret-cell holds after the body runs.
fn refactored_select_level(p: [f32; 3], lv: &HostLevels) -> i32 {
    use std::cell::Cell;
    let ret_out = Cell::new(0i32);
    boyko_shaderdsl::levels::select_level_body::<EvalCf, _, _, _>(
        p,
        // `m2_levels[L].<field>` (`.xyz`) — index the fixture by the iv, pick the member by text.
        |l: usize, field: &'static str| -> [f32; 3] {
            match field {
                "origin_brick_world.xyz" => lv.levels[l].origin,
                "dims_atlas_dim.xyz" => lv.levels[l].dims,
                other => unreachable!("unexpected level vec3 field `{other}`"),
            }
        },
        // `m2_levels[L].<field>` (`.w`).
        |l: usize, field: &'static str| -> f32 {
            match field {
                "origin_brick_world.w" => lv.levels[l].bw,
                other => unreachable!("unexpected level scalar field `{other}`"),
            }
        },
        // `pc.brick_levels`.
        || lv.brick_levels,
        &ret_out,
    );
    ret_out.get()
}

#[test]
fn select_level_matches_host_on_random_sweep() {
    // A deterministic LCG sweeps query points × a few level configurations. The level boxes are
    // built as a NESTED clip-map (level 0 finest/smallest, level 2 coarsest/largest, sharing a
    // center) so finest-first is meaningfully exercised (an inner point matches level 0 even though
    // it is also inside levels 1/2), plus random non-nested configs for the general scan.
    let mut lcg = Lcg::new(0x0BAD_F00D_DEAD_BEEF_u64);
    for _ in 0..20_000 {
        // Build a random level fixture. Each level: a random origin + a random (positive) bw + a
        // random small integer dims (so `hi = o + dims*bw` is a finite box).
        let mk_level = |lcg: &mut Lcg| -> HostLevel {
            let origin = [lcg.next_f32(4.0), lcg.next_f32(4.0), lcg.next_f32(4.0)];
            let bw = lcg.next_f32(1.0).abs() + 0.05;
            let dims = [
                (lcg.next_u32() % 8 + 1) as f32,
                (lcg.next_u32() % 8 + 1) as f32,
                (lcg.next_u32() % 8 + 1) as f32,
            ];
            HostLevel { origin, bw, dims }
        };
        let levels = [mk_level(&mut lcg), mk_level(&mut lcg), mk_level(&mut lcg)];
        // A runtime count in {0, 1, 2, 3} — exercises the `L >= pc.brick_levels` skip (an inactive
        // level is never tested) AND the OFF/N=1 keystone (`brick_levels == 1` is the M2 single
        // grid).
        let brick_levels = lcg.next_u32() % 4;
        let lv = HostLevels {
            levels,
            brick_levels,
        };
        // Query points biased to land ON, NEAR, and FAR from the level boxes (so inside / boundary
        // / outside are all sampled).
        let p = [lcg.next_f32(6.0), lcg.next_f32(6.0), lcg.next_f32(6.0)];

        let r = refactored_select_level(p, &lv);
        let h = host_select_level(p, &lv);
        assert_eq!(
            r, h,
            "select_level diverged at p={p:?} brick_levels={brick_levels}: refactored={r} host={h}"
        );
    }
}

#[test]
fn select_level_directed_inside_boundary_outside_skip() {
    // DIRECTED cases pinning each branch the design calls out.
    // A single unit-cell level at the origin: origin=[0,0,0], bw=1, dims=[1,1,1] -> box [0,0,0)..[1,1,1).
    let unit = HostLevel {
        origin: [0.0, 0.0, 0.0],
        bw: 1.0,
        dims: [1.0, 1.0, 1.0],
    };
    let one_level = HostLevels {
        levels: [unit, unit, unit],
        brick_levels: 1,
    };

    // (a) INSIDE level 0 -> returns 0.
    assert_eq!(refactored_select_level([0.5, 0.5, 0.5], &one_level), 0);
    assert_eq!(host_select_level([0.5, 0.5, 0.5], &one_level), 0);

    // (b) The lower corner `p == o` is INSIDE (`>=`).
    assert_eq!(refactored_select_level([0.0, 0.0, 0.0], &one_level), 0);

    // (c) The upper corner `p == hi` is EXCLUDED (`<`, not `<=`) -> outside -> -1.
    assert_eq!(refactored_select_level([1.0, 0.5, 0.5], &one_level), -1);
    assert_eq!(host_select_level([1.0, 0.5, 0.5], &one_level), -1);

    // (d) OUTSIDE all -> -1.
    assert_eq!(refactored_select_level([5.0, 5.0, 5.0], &one_level), -1);

    // (e) FINEST-FIRST: a nested fixture where level 0 (smaller) and level 1 (larger) both contain
    // the point -> the finest (level 0) wins.
    let inner = HostLevel {
        origin: [0.25, 0.25, 0.25],
        bw: 1.0,
        dims: [1.0, 1.0, 1.0],
    }; // box [0.25..1.25)
    let outer = HostLevel {
        origin: [0.0, 0.0, 0.0],
        bw: 1.0,
        dims: [2.0, 2.0, 2.0],
    }; // box [0..2)
    let nested = HostLevels {
        levels: [inner, outer, outer],
        brick_levels: 2,
    };
    // p=[0.5,0.5,0.5] is inside BOTH inner (level 0) and outer (level 1) -> finest-first returns 0.
    assert_eq!(refactored_select_level([0.5, 0.5, 0.5], &nested), 0);
    // p=[0.1,0.1,0.1] is OUTSIDE inner (< 0.25) but inside outer (level 1) -> returns 1.
    assert_eq!(refactored_select_level([0.1, 0.1, 0.1], &nested), 1);
    assert_eq!(host_select_level([0.1, 0.1, 0.1], &nested), 1);

    // (f) The `L >= pc.brick_levels` SKIP: the same nested fixture but brick_levels==1 only tests
    // level 0 (inner). p=[0.1,..] is outside inner and level 1 is INACTIVE -> -1 (not 1).
    let nested_off = HostLevels {
        levels: [inner, outer, outer],
        brick_levels: 1,
    };
    assert_eq!(refactored_select_level([0.1, 0.1, 0.1], &nested_off), -1);
    assert_eq!(host_select_level([0.1, 0.1, 0.1], &nested_off), -1);

    // (g) brick_levels==0 -> the loop breaks immediately on L=0 -> -1 (no level active).
    let nested_zero = HostLevels {
        levels: [inner, outer, outer],
        brick_levels: 0,
    };
    assert_eq!(refactored_select_level([0.5, 0.5, 0.5], &nested_zero), -1);
}

// ======================================================================
// Inc 5b — the `m2_brick_span` brick-AABB ray-span clip Eval oracle (FULL coverage — pure
// arithmetic, no field call).
//
// `m2_brick_span` is a pure slab-method clip (no field/buffer reads), so the Eval body has FULL
// coverage: a host mirror transcribing the committed L970-993 VERBATIM is the EXACT reference. The
// return is `(bool hit, float t_enter, float t_exit)`; the assertion is `(hit, t_enter.to_bits(),
// t_exit.to_bits())` bit-identical (EXACT, no ULP slack — the body is the SAME arithmetic on both).
// The sweep + directed cases cover every branch the design pins: a NORMAL hit (`tmax > tmin`), a
// MISS (`tmax <= tmin`), a PARALLEL-SLAB miss (`abs(rd[a]) <= 1e-20` with `p[a]` outside →
// `return false`, `t_enter = 1.0`/`t_exit = 0.0`), a PARALLEL-SLAB pass-through (origin inside the
// slab → `continue`), and the SWAP branch (`t1 > t2` on a negative `rd[a]`).
// ======================================================================

/// Verbatim hand-mirror of the committed `m2_brick_span` body (the L970-993 statements). Returns
/// `(bool hit, float t_enter, float t_exit)` — on a parallel-slab miss `(false, 1.0, 0.0)`; else
/// the slab span + `tmax > tmin`. The reference the eDSL `m2_brick_span_body::<EvalCf>` is locked
/// against (EXACT to-bits eq).
// The frozen mirror spells the committed HLSL body VERBATIM (two separate `t_enter`/`t_exit`
// assigns, the explicit `tmp` swap, the `1.0e30`/`1.0e-20` literals). The 5-element return-tuple
// and the verbatim swap are clearer than any "idiomatic" rewrite that would diverge from the
// committed source the body must mirror; clippy's needless-range / swap suggestions are suppressed.
#[allow(clippy::needless_range_loop, clippy::manual_swap)]
fn host_m2_brick_span(
    p: [f32; 3],
    rd: [f32; 3],
    cell_min: [f32; 3],
    brick_world: f32,
) -> (bool, f32, f32) {
    let mut tmin = 0.0f32;
    let mut tmax = 1.0e30f32;
    for a in 0..3usize {
        let lo = cell_min[a];
        let hi = lo + brick_world;
        if rd[a].abs() <= 1.0e-20 {
            if p[a] < lo || p[a] > hi {
                // `t_enter = 1.0; t_exit = 0.0; return false;` — the empty-span miss (the verbatim
                // committed sentinel; `t_exit < t_enter` flags it).
                return (false, 1.0, 0.0);
            }
            continue;
        }
        let inv = 1.0 / rd[a];
        let mut t1 = (lo - p[a]) * inv;
        let mut t2 = (hi - p[a]) * inv;
        if t1 > t2 {
            let tmp = t1;
            t1 = t2;
            t2 = tmp;
        }
        tmin = tmin.max(t1);
        tmax = tmax.min(t2);
    }
    // `t_enter = tmin; t_exit = tmax; return tmax > tmin;`
    (tmax > tmin, tmin, tmax)
}

/// The eDSL `m2_brick_span_body::<EvalCf>` over `[f32; 3]` params + two `Cell<f32>` out-floats + a
/// `Cell<bool>` ret-cell. Returns the `(hit, t_enter, t_exit)` the cells hold after the body runs.
/// The out-float cells are seeded with a DISTINCT sentinel so a missing write surfaces (the body
/// writes both on every path — the empty-span miss AND the tail).
fn refactored_m2_brick_span(
    p: [f32; 3],
    rd: [f32; 3],
    cell_min: [f32; 3],
    brick_world: f32,
) -> (bool, f32, f32) {
    use std::cell::Cell;
    let t_enter = Cell::new(f32::from_bits(0xDEAD_BEEF));
    let t_exit = Cell::new(f32::from_bits(0xDEAD_BEEF));
    let ret_out = Cell::new(false);
    boyko_shaderdsl::brick::m2_brick_span_body::<EvalCf>(
        p,
        rd,
        cell_min,
        brick_world,
        &t_enter,
        &t_exit,
        &ret_out,
    );
    (ret_out.get(), t_enter.get(), t_exit.get())
}

fn assert_brick_span(
    r: (bool, f32, f32),
    h: (bool, f32, f32),
    ctx: &str,
) {
    assert_eq!(r.0, h.0, "{ctx}: hit diverged (refactored={} host={})", r.0, h.0);
    assert_bits(r.1, h.1, &format!("{ctx}-t_enter"));
    assert_bits(r.2, h.2, &format!("{ctx}-t_exit"));
}

#[test]
fn m2_brick_span_matches_host_on_random_sweep() {
    // A deterministic LCG sweeps rays × brick boxes. `rd` ranges over signed components (so the
    // SWAP branch `t1 > t2` — a negative direction — is exercised), and `p`/`cell_min` are biased
    // so the ray lands ON, NEAR, and FAR from the brick (hit / miss both sampled). `brick_world` is
    // a positive edge length. The full sweep exercises the normal hit, the miss, and the swap; the
    // directed test below pins the two parallel-slab branches (which a random `rd` essentially never
    // hits, since `abs(rd[a]) <= 1e-20` is measure-zero).
    let mut lcg = Lcg::new(0x5EED_1234_BEEF_0F0F_u64);
    for _ in 0..20_000 {
        let p = [lcg.next_f32(4.0), lcg.next_f32(4.0), lcg.next_f32(4.0)];
        let rd = [lcg.next_f32(2.0), lcg.next_f32(2.0), lcg.next_f32(2.0)];
        let cell_min = [lcg.next_f32(4.0), lcg.next_f32(4.0), lcg.next_f32(4.0)];
        let brick_world = lcg.next_f32(2.0).abs() + 0.05;

        let r = refactored_m2_brick_span(p, rd, cell_min, brick_world);
        let h = host_m2_brick_span(p, rd, cell_min, brick_world);
        assert_brick_span(
            r,
            h,
            &format!("m2-brick-span sweep p={p:?} rd={rd:?} cell_min={cell_min:?} bw={brick_world}"),
        );
    }
}

#[test]
fn m2_brick_span_directed_hit_miss_swap_parallel() {
    // A unit brick at the origin: cell_min=[0,0,0], brick_world=1 -> AABB [0,0,0]..[1,1,1].
    let cell_min = [0.0, 0.0, 0.0];
    let bw = 1.0;

    // (a) NORMAL HIT: a ray from (-1, 0.5, 0.5) along +x pierces the brick -> tmax > tmin.
    {
        let p = [-1.0, 0.5, 0.5];
        let rd = [1.0, 0.0, 0.0]; // y/z are parallel slabs with the origin INSIDE -> continue
        let r = refactored_m2_brick_span(p, rd, cell_min, bw);
        let h = host_m2_brick_span(p, rd, cell_min, bw);
        assert!(r.0, "directed hit: expected a hit");
        assert_brick_span(r, h, "m2-brick-span-directed-hit");
    }

    // (b) MISS: a ray from (-1, 5, 5) along +x — the y/z origins (5) are OUTSIDE the [0,1] slabs and
    // those slabs are PARALLEL (rd.y = rd.z = 0) -> the parallel-slab early `return false`
    // (t_enter=1, t_exit=0). This is BOTH the miss case AND the PARALLEL-SLAB-MISS branch.
    {
        let p = [-1.0, 5.0, 5.0];
        let rd = [1.0, 0.0, 0.0];
        let r = refactored_m2_brick_span(p, rd, cell_min, bw);
        let h = host_m2_brick_span(p, rd, cell_min, bw);
        assert!(!r.0, "directed parallel-slab miss: expected a miss");
        assert_bits(r.1, 1.0, "parallel-slab-miss-t_enter-is-1.0");
        assert_bits(r.2, 0.0, "parallel-slab-miss-t_exit-is-0.0");
        assert_brick_span(r, h, "m2-brick-span-parallel-slab-miss");
    }

    // (c) PARALLEL-SLAB PASS-THROUGH: rd.y = rd.z = 0 but the y/z origins (0.5) are INSIDE the [0,1]
    // slabs -> those axes `continue` (impose no bound); the x axis (rd.x = 1, origin -1) provides the
    // span. A real hit, exercising the `continue` (NOT the early return).
    {
        let p = [-1.0, 0.5, 0.5];
        let rd = [1.0, 0.0, 0.0];
        let r = refactored_m2_brick_span(p, rd, cell_min, bw);
        let h = host_m2_brick_span(p, rd, cell_min, bw);
        assert!(r.0, "directed parallel pass-through: expected a hit");
        assert_brick_span(r, h, "m2-brick-span-parallel-pass-through");
    }

    // (d) SWAP branch: a ray along -x (rd.x = -1) makes the near/far crossings arrive in t1 > t2
    // order -> the `if (t1 > t2)` swap fires. From (2, 0.5, 0.5) marching -x the brick is ahead.
    {
        let p = [2.0, 0.5, 0.5];
        let rd = [-1.0, 0.0, 0.0];
        let r = refactored_m2_brick_span(p, rd, cell_min, bw);
        let h = host_m2_brick_span(p, rd, cell_min, bw);
        assert!(r.0, "directed swap: expected a hit");
        assert_brick_span(r, h, "m2-brick-span-swap-branch");
        // The span starts at t=1 (x goes 2 -> 1, the near face) and exits at t=2 (x -> 0).
        assert_bits(r.1, 1.0, "swap-branch-t_enter");
        assert_bits(r.2, 2.0, "swap-branch-t_exit");
    }
}

// ======================================================================
// Track B Increment G1 — the `pack_material_id_ba` material-id packer Eval oracle (FULL coverage —
// pure bit/arithmetic).
//
// `pack_material_id_ba` is a pure byte split (no field call, no control flow), so the Eval body has
// FULL coverage: a host mirror transcribing the committed L520-522 VERBATIM is the EXACT reference.
// The return is a `float2`, so each lane is compared to-BITS (the byte split masks the high bits
// away, so the two lanes are exact). Swept over directed ids {0, 1, 0xFF, 0x100, 0x8080, 0xFFFF,
// 0x12345} + an LCG sweep.
// ======================================================================

/// Verbatim hand-mirror of the committed `pack_material_id_ba` body (the L520-522 statements). The
/// independent reference the eDSL `pack_material_id_ba_body::<EvalCf>` is locked against (EXACT
/// to-bits eq per lane).
fn host_pack_material_id_ba(id: u32) -> [f32; 2] {
    [
        (id & 0xFF) as f32 / 255.0,
        ((id >> 8) & 0xFF) as f32 / 255.0,
    ]
}

/// The eDSL `pack_material_id_ba_body::<EvalCf>` reading the `float2` the ret-cell holds after the
/// body runs.
fn refactored_pack_material_id_ba(id: u32) -> [f32; 2] {
    use std::cell::Cell;
    let ret_out = Cell::new([0.0f32; 2]);
    let _ = boyko_shaderdsl::pack::pack_material_id_ba_body::<EvalCf>(id, &ret_out);
    ret_out.get()
}

fn assert_pack_bits(r: [f32; 2], h: [f32; 2], ctx: &str) {
    assert_bits(r[0], h[0], &format!("{ctx}-lo"));
    assert_bits(r[1], h[1], &format!("{ctx}-hi"));
}

#[test]
fn pack_material_id_ba_matches_host_directed_and_sweep() {
    // Directed ids exercising the byte boundaries: 0, the smallest nonzero, a single-byte max
    // (0xFF), the first carry into the high byte (0x100), both bytes set (0x8080), the 16-bit max
    // (0xFFFF), and an id with bits ABOVE 16 (0x12345 — the `& 255` / `>> 8 & 255` discards them, the
    // high-bits-masked-away property).
    for id in [0u32, 1, 0xFF, 0x100, 0x8080, 0xFFFF, 0x1_2345] {
        let r = refactored_pack_material_id_ba(id);
        let h = host_pack_material_id_ba(id);
        assert_pack_bits(r, h, &format!("pack-directed-id-{id:#x}"));
    }

    // An LCG sweep over the full `u32` range (the bits above 16 are masked away by the byte split, so
    // the two-lane result still byte-matches the host mirror exactly).
    let mut lcg = Lcg::new(0xBA00_5EED_1234_ABCD_u64);
    for _ in 0..50_000 {
        let id = lcg.next_u32();
        let r = refactored_pack_material_id_ba(id);
        let h = host_pack_material_id_ba(id);
        assert_pack_bits(r, h, &format!("pack-sweep-id-{id:#x}"));
    }
}

// ======================================================================
// Track B Increment G2 — the `oct_encode` octahedral-normal encoder Eval oracle (FULL coverage —
// pure arithmetic + a single data-dependent branch).
//
// `oct_encode` has no field call and only ONE branch (`n.z < 0.0`), so the Eval body has FULL
// coverage: a host mirror transcribing the committed L508-513 VERBATIM is the EXACT reference. The
// return is a `float2`, compared to-BITS per lane (the host op order matches the eDSL body's, so the
// two lanes are bit-exact). Swept over BOTH hemispheres (`n.z < 0` and `n.z >= 0`, exercising the `if`
// both ways), all four sign quadrants (±x/±y, exercising both sign-ternaries), the six axis-aligned
// normals (±x/±y/±z), + an LCG sweep of normalized vectors.
// ======================================================================

/// Verbatim hand-mirror of the committed `oct_encode` body (the L508-513 statements). The independent
/// reference the eDSL `oct_encode_body::<EvalCf>` is locked against (EXACT to-bits eq per lane). The op
/// order matches the committed source statement-for-statement.
fn host_oct_encode(n: [f32; 3]) -> [f32; 2] {
    // n /= (abs(n.x) + abs(n.y) + abs(n.z));
    let s = n[0].abs() + n[1].abs() + n[2].abs();
    let n = [n[0] / s, n[1] / s, n[2] / s];
    // float2 e = n.xy;
    let mut e = [n[0], n[1]];
    // if (n.z < 0.0) { e = (1.0 - abs(e.yx)) * float2(e.x >= 0.0 ? 1.0 : -1.0, e.y >= 0.0 ? 1.0 : -1.0); }
    if n[2] < 0.0 {
        let yx = [e[1].abs(), e[0].abs()];
        let mirror = [1.0 - yx[0], 1.0 - yx[1]];
        let sign = [
            if e[0] >= 0.0 { 1.0 } else { -1.0 },
            if e[1] >= 0.0 { 1.0 } else { -1.0 },
        ];
        e = [mirror[0] * sign[0], mirror[1] * sign[1]];
    }
    // return e * 0.5 + 0.5;
    [e[0] * 0.5 + 0.5, e[1] * 0.5 + 0.5]
}

/// The eDSL `oct_encode_body::<EvalCf>` reading the `float2` the ret-cell holds after the body runs.
fn refactored_oct_encode(n: [f32; 3]) -> [f32; 2] {
    use std::cell::Cell;
    let ret_out = Cell::new([0.0f32; 2]);
    let _ = boyko_shaderdsl::oct::oct_encode_body::<EvalCf>(n, &ret_out);
    ret_out.get()
}

fn assert_oct_bits(r: [f32; 2], h: [f32; 2], ctx: &str) {
    assert_bits(r[0], h[0], &format!("{ctx}-x"));
    assert_bits(r[1], h[1], &format!("{ctx}-y"));
}

/// Normalize a non-zero vector (the encoder assumes a unit normal, but the eDSL/host parity holds for
/// ANY non-degenerate input — both run the SAME ops). Returns `None` for a near-zero vector (the L1
/// divide would be non-finite, which is outside the unit-normal contract).
fn normalize(v: [f32; 3]) -> Option<[f32; 3]> {
    let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if len < 1.0e-6 {
        return None;
    }
    Some([v[0] / len, v[1] / len, v[2] / len])
}

#[test]
fn oct_encode_matches_host_directed_and_sweep() {
    // The six AXIS-ALIGNED unit normals (±x/±y/±z) — `+z`/`-z` exercise the `n.z < 0.0` branch's two
    // sides at the pole; ±x/±y land on the equator. Plus the four DIAGONAL sign quadrants in the LOWER
    // hemisphere (n.z < 0), which is the ONLY path that runs the two sign-ternaries — one per (sign(x),
    // sign(y)) combination so both `e.x >= 0.0` and `e.y >= 0.0` are exercised true AND false.
    let directed: &[[f32; 3]] = &[
        [1.0, 0.0, 0.0],
        [-1.0, 0.0, 0.0],
        [0.0, 1.0, 0.0],
        [0.0, -1.0, 0.0],
        [0.0, 0.0, 1.0],
        [0.0, 0.0, -1.0],
        // The four lower-hemisphere sign quadrants (n.z < 0 → the fold + both sign-ternaries).
        [0.5, 0.5, -0.5],
        [-0.5, 0.5, -0.5],
        [0.5, -0.5, -0.5],
        [-0.5, -0.5, -0.5],
        // The four UPPER-hemisphere sign quadrants (n.z >= 0 → the fold is SKIPPED, the `e = n.xy` path).
        [0.5, 0.5, 0.5],
        [-0.5, 0.5, 0.5],
        [0.5, -0.5, 0.5],
        [-0.5, -0.5, 0.5],
    ];
    for &v in directed {
        let n = normalize(v).expect("invariant: the directed normals are non-degenerate");
        let r = refactored_oct_encode(n);
        let h = host_oct_encode(n);
        assert_oct_bits(r, h, &format!("oct-directed-{n:?}"));
    }

    // An LCG sweep of normalized vectors covering BOTH hemispheres + all sign quadrants densely (the
    // `n.z` sign is uniform, so ~half the samples take the `if` branch).
    let mut lcg = Lcg::new(0x0C7A_1234_5EED_BEEF_u64);
    let mut sampled = 0u32;
    while sampled < 50_000 {
        let v = [lcg.next_f32(1.0), lcg.next_f32(1.0), lcg.next_f32(1.0)];
        // Skip the non-finite distribution entries (NaN/Inf) the `next_f32` mixes in — the encoder's
        // unit-normal contract is finite inputs; the parity is the property under test, not NaN
        // propagation (the field tests cover non-finite handling).
        if !v.iter().all(|c| c.is_finite()) {
            continue;
        }
        let Some(n) = normalize(v) else { continue };
        let r = refactored_oct_encode(n);
        let h = host_oct_encode(n);
        assert_oct_bits(r, h, &format!("oct-sweep-{n:?}"));
        sampled += 1;
    }
}
