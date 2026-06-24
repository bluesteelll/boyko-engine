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
