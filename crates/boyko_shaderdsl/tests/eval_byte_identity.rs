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
