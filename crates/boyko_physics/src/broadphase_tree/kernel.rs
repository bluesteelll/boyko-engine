//! The two 8-wide tests of the packed BVH (`04-DESIGN-REV2.md`, D1, D2): the conservative box
//! test at an internal node and the exact sphere-bound test at a leaf.
//!
//! # The leaf test is the AllPairs expression
//!
//! The shipped all-pairs loop decides a pair with
//! `delta.length_squared() <= bound * bound`, where `delta = p_j - p_i`, `bound = r_i + r_j`,
//! and `length_squared` is `x*x + y*y + z*z` evaluated left to right. The leaf kernel evaluates
//! `t = dx*dx; t = t + dy*dy; t = t + dz*dz; t <= (r_leaf + r_query)^2` on the same bits, one
//! IEEE single-rounded op per step and no fused multiply-add. The two agree bitwise whichever
//! row is the query: `fl(a - b) = -fl(b - a)`, a square is sign-blind, and `f32` addition is
//! commutative. So the tree's pair set is AllPairs' pair set, not an approximation of it.
//!
//! # The box test is conservative
//!
//! An internal lane holds the union AABB of its subtree's leaves, each padded by the slack
//! [`padded_radius`] adds to `|r|`. A query overlaps a lane when both intervals meet on every
//! axis. A pair the exact test accepts is separated by at most `|r_a| + |r_b|` per axis plus
//! the test's rounding, and that rounding has two regimes the two slack terms answer:
//!
//! * `bound²` normal: every rounding is relative. The acceptance radius exceeds `|bound|` by a
//!   few ulp, and the separation, the bound, the padded radius and the box edges are each
//!   rounded at the scale of a row's `‖p‖∞ + |r|` — at most about nine ulp of the two rows'
//!   scales in all (five from the test, two from the box edges, two from the padded radius).
//!   The relative slack gives each row sixteen ulp of its scale.
//! * `bound²` sub-normal (`|bound| < 2^-63`): a square below `2^-126` carries an absolute error
//!   of up to `2^-150`, not a relative one — `fl(dx²) = 0` for every `|dx| ≤ 2^-75` — so with
//!   `bound = 0` the test accepts a separation of `2^-75` per axis whatever the radii, and no
//!   relative term covers that. The absolute slack does: the largest per-axis excess of an
//!   accepted pair over `|r_a| + |r_b|` is `2^-75` (`r = 0`, `dx = 2^-75`, the tie that rounds
//!   `dx²` to even), and two sides of `2^-72` cover it sixteen times.
//!
//! The absolute slack is absorbed by rounding whenever `|r| + (‖p‖∞ + |r|)·2^-20 ≥ 2^-47` — a
//! radius above 7e-15 or a coordinate above 7.5e-9 — so no box of a physical scene moves.
//!
//! # AVX2 and scalar
//!
//! The AVX2 arm compiles under `cfg(target_feature = "avx2")` and not under Miri; every other
//! build takes the scalar arm. They are bit-identical: the AVX2 ops used here (`sub`, `mul`,
//! `add`, `cmp LE_OQ` / `GE_OQ`) round exactly as their scalar `f32` counterparts, and the
//! in-crate G0 property test pins the two against each other on adversarial inputs. The source
//! census test at the end of this file keeps the arm free of fused and approximate ops.

use super::bvh::{LEAF_R, LEAF_X, LEAF_Y, LEAF_Z, MAX_X, MAX_Y, MAX_Z, MIN_X, MIN_Y, MIN_Z, Node8};

/// The relative slack of a cull bound: `2^-20`, sixteen ulp of `f32`.
const SLACK_REL: f32 = 1.0 / 1_048_576.0;

/// The absolute slack of a cull bound: `2^-72`. Once `bound²` is sub-normal the exact test's
/// error is absolute (every `|dx| ≤ 2^-75` squares to `0`), and the box needs a width no
/// relative term supplies. The largest per-axis excess an accepted pair can have over
/// `|r_a| + |r_b|` is `2^-75`; two sides of `2^-72` cover it sixteen times. (The first cut had
/// `2^-126` here, sized for sub-normal *rows*, and missed pairs at `|p| ~ 2^-60`, `|r| ~ 2^-90`
/// — the review of C1, W1; `g1_subnormal_pairs_are_found` pins it.)
const SLACK_ABS: f32 = 1.0 / (1u128 << 72) as f32;

/// The half-extent a row's cull box gets on every axis: `|r| + (‖p‖∞ + |r|)·2^-20 + 2^-72`.
///
/// Finite for every Normal row (‖p‖∞ + |r| ≤ 2^60 by the row-kind rule, D8).
#[inline]
pub(crate) fn padded_radius(x: f32, y: f32, z: f32, r: f32) -> f32 {
    let ar = r.abs();
    let extent = x.abs().max(y.abs()).max(z.abs()) + ar;
    ar + extent * SLACK_REL + SLACK_ABS
}

/// One query's cull box, the closed interval `[lo, hi]` on each axis.
#[derive(Clone, Copy, Debug)]
pub(crate) struct QueryBox {
    pub(crate) lo: [f32; 3],
    pub(crate) hi: [f32; 3],
}

impl QueryBox {
    /// The cull box of a Normal row at `(x, y, z)` with bounding radius `r`.
    #[inline]
    pub(crate) fn of(x: f32, y: f32, z: f32, r: f32) -> Self {
        let h = padded_radius(x, y, z, r);
        Self { lo: [x - h, y - h, z - h], hi: [x + h, y + h, z + h] }
    }
}

/// Lanes of an internal `node` whose box meets `q` (bit `k` set for lane `k`).
#[inline]
pub(crate) fn box_mask(node: &Node8, q: &QueryBox) -> u32 {
    #[cfg(all(target_arch = "x86_64", target_feature = "avx2", not(miri)))]
    {
        box_mask_avx2(node, q)
    }
    #[cfg(not(all(target_arch = "x86_64", target_feature = "avx2", not(miri))))]
    {
        box_mask_scalar(node, q)
    }
}

/// Lanes of a leaf `node` whose exact sphere-bound test against the query at `(x, y, z)` with
/// bounding radius `r` passes (bit `k` set for lane `k`). A killed or empty lane holds
/// `x = +inf`, so its test is always false.
#[inline]
pub(crate) fn leaf_mask(node: &Node8, x: f32, y: f32, z: f32, r: f32) -> u32 {
    #[cfg(all(target_arch = "x86_64", target_feature = "avx2", not(miri)))]
    {
        leaf_mask_avx2(node, x, y, z, r)
    }
    #[cfg(not(all(target_arch = "x86_64", target_feature = "avx2", not(miri))))]
    {
        leaf_mask_scalar(node, x, y, z, r)
    }
}

/// The scalar box test: the reference the AVX2 arm is pinned against (G0), and the arm every
/// non-AVX2 build and Miri run.
//
// `dead_code`: in an AVX2 build (the baseline) the production path takes the AVX2 arm and only
// the G0 test calls this; the scalar arm is the reference, not a leftover.
#[allow(dead_code)]
#[inline]
pub(crate) fn box_mask_scalar(node: &Node8, q: &QueryBox) -> u32 {
    let mut mask = 0u32;
    for k in 0..8 {
        let hit = q.lo[0] <= node.p[MAX_X][k]
            && q.hi[0] >= node.p[MIN_X][k]
            && q.lo[1] <= node.p[MAX_Y][k]
            && q.hi[1] >= node.p[MIN_Y][k]
            && q.lo[2] <= node.p[MAX_Z][k]
            && q.hi[2] >= node.p[MIN_Z][k];
        mask |= u32::from(hit) << k;
    }
    mask
}

/// The scalar leaf test: the AllPairs expression per lane, on the lane's bits.
//
// `dead_code`: as for `box_mask_scalar`.
#[allow(dead_code)]
#[inline]
pub(crate) fn leaf_mask_scalar(node: &Node8, x: f32, y: f32, z: f32, r: f32) -> u32 {
    let mut mask = 0u32;
    for k in 0..8 {
        let dx = node.p[LEAF_X][k] - x;
        let dy = node.p[LEAF_Y][k] - y;
        let dz = node.p[LEAF_Z][k] - z;
        let mut t = dx * dx;
        t += dy * dy;
        t += dz * dz;
        let bound = node.p[LEAF_R][k] + r;
        let hit = t <= bound * bound;
        mask |= u32::from(hit) << k;
    }
    mask
}

#[cfg(all(target_arch = "x86_64", target_feature = "avx2", not(miri)))]
#[inline]
fn box_mask_avx2(node: &Node8, q: &QueryBox) -> u32 {
    use core::arch::x86_64::{
        __m256, _CMP_GE_OQ, _CMP_LE_OQ, _mm256_and_ps, _mm256_cmp_ps, _mm256_load_ps,
        _mm256_movemask_ps, _mm256_set1_ps,
    };
    // SAFETY: `Node8` is `#[repr(C, align(32))]` and each `p[k]` is a `[f32; 8]` at offset
    // `32 * k`, so every `_mm256_load_ps` reads 32 aligned, initialised bytes inside the node
    // borrowed for this call. The `avx2` target feature is enabled at compile time by the
    // `cfg` on this function, so the intrinsics are available on every CPU this binary runs on.
    unsafe {
        let load = |k: usize| -> __m256 { _mm256_load_ps(node.p[k].as_ptr()) };
        let lo_x = _mm256_set1_ps(q.lo[0]);
        let hi_x = _mm256_set1_ps(q.hi[0]);
        let lo_y = _mm256_set1_ps(q.lo[1]);
        let hi_y = _mm256_set1_ps(q.hi[1]);
        let lo_z = _mm256_set1_ps(q.lo[2]);
        let hi_z = _mm256_set1_ps(q.hi[2]);
        let m = _mm256_cmp_ps::<_CMP_LE_OQ>(lo_x, load(MAX_X));
        let m = _mm256_and_ps(m, _mm256_cmp_ps::<_CMP_GE_OQ>(hi_x, load(MIN_X)));
        let m = _mm256_and_ps(m, _mm256_cmp_ps::<_CMP_LE_OQ>(lo_y, load(MAX_Y)));
        let m = _mm256_and_ps(m, _mm256_cmp_ps::<_CMP_GE_OQ>(hi_y, load(MIN_Y)));
        let m = _mm256_and_ps(m, _mm256_cmp_ps::<_CMP_LE_OQ>(lo_z, load(MAX_Z)));
        let m = _mm256_and_ps(m, _mm256_cmp_ps::<_CMP_GE_OQ>(hi_z, load(MIN_Z)));
        _mm256_movemask_ps(m) as u32
    }
}

#[cfg(all(target_arch = "x86_64", target_feature = "avx2", not(miri)))]
#[inline]
fn leaf_mask_avx2(node: &Node8, x: f32, y: f32, z: f32, r: f32) -> u32 {
    use core::arch::x86_64::{
        _CMP_LE_OQ, _mm256_add_ps, _mm256_cmp_ps, _mm256_load_ps, _mm256_movemask_ps,
        _mm256_mul_ps, _mm256_set1_ps, _mm256_sub_ps,
    };
    // SAFETY: as in `box_mask_avx2` — `p[LEAF_X..=LEAF_R]` are 32-byte aligned `[f32; 8]`
    // rows of the borrowed node, and the `avx2` feature is compiled in. The arithmetic is
    // `sub`, `mul`, `add` and an ordered-quiet `<=`: each rounds once, exactly as the scalar
    // arm's `f32` ops do, and none contracts into a fused op (the census below pins that).
    unsafe {
        let dx = _mm256_sub_ps(_mm256_load_ps(node.p[LEAF_X].as_ptr()), _mm256_set1_ps(x));
        let dy = _mm256_sub_ps(_mm256_load_ps(node.p[LEAF_Y].as_ptr()), _mm256_set1_ps(y));
        let dz = _mm256_sub_ps(_mm256_load_ps(node.p[LEAF_Z].as_ptr()), _mm256_set1_ps(z));
        let t = _mm256_mul_ps(dx, dx);
        let t = _mm256_add_ps(t, _mm256_mul_ps(dy, dy));
        let t = _mm256_add_ps(t, _mm256_mul_ps(dz, dz));
        let bound = _mm256_add_ps(_mm256_load_ps(node.p[LEAF_R].as_ptr()), _mm256_set1_ps(r));
        let bb = _mm256_mul_ps(bound, bound);
        _mm256_movemask_ps(_mm256_cmp_ps::<_CMP_LE_OQ>(t, bb)) as u32
    }
}

// `not(miri)`: the census reads this file from disk, an operation Miri's isolation rejects by
// aborting the whole test binary (not by failing the one test), and it asserts a source
// property that the native run establishes.
#[cfg(all(test, not(miri)))]
mod census {
    /// No-FMA / no-approx grep gate over this file: zero fused (`fmadd` / `fmsub` / `fnmadd` /
    /// `fnmsub` / `fmaddsub` / `fmsubadd`), zero approximate (`rsqrt` / `rcp`) and zero
    /// `mul_add` / `algebraic_` call-sites. The sibling of
    /// `systems::o9_manifold_tests::systems_has_no_fma_or_approx_callsites`, with the same needle
    /// list, the same comment skip and the same non-vacuity witness: every file of this crate
    /// that holds an `_mm256_*` call site carries one (`.cargo/config.toml`, the ISA baseline).
    ///
    /// The stake: a fused op rounds once where the AllPairs expression rounds twice, so the
    /// leaf test would stop being the AllPairs predicate and the pair set would move. Doc
    /// prose may name the banned ops; only non-comment lines are scanned.
    #[test]
    fn kernel_has_no_fma_or_approx_callsites() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src")
            .join("broadphase_tree")
            .join("kernel.rs");
        let contents = std::fs::read_to_string(&path).expect("kernel.rs must be readable");

        // Assembled from fragments so no full call token appears literally in this file, which
        // scans itself.
        let suffix = "_ps(";
        let widths = ["_mm256_", "_mm_"];
        let stems = ["fmadd", "fmsub", "fnmadd", "fnmsub", "fmaddsub", "fmsubadd", "rsqrt", "rcp"];
        let mut banned: Vec<String> = Vec::with_capacity(widths.len() * stems.len() + 2);
        for w in widths {
            for s in stems {
                banned.push(format!("{w}{s}{suffix}"));
            }
        }
        banned.push(format!("{}{}", "mul_add", "("));
        banned.push(format!("{}{}", "algebraic", "_"));

        let mut hits = Vec::new();
        let mut witness = 0usize;
        for (i, line) in contents.lines().enumerate() {
            let trimmed = line.trim_start();
            if trimmed.starts_with("//") {
                continue;
            }
            if line.contains("_mm256_") {
                witness += 1;
            }
            for b in &banned {
                if line.contains(b.as_str()) {
                    hits.push(format!("{}:{}: {}", path.display(), i + 1, line.trim()));
                }
            }
        }
        assert!(
            witness > 0,
            "anti-vacuity: kernel.rs has no `_mm256_` call site, so the census scans nothing"
        );
        assert!(hits.is_empty(), "fused / approximate call sites in kernel.rs:\n{}", hits.join("\n"));
    }
}
