//! `bitset_intersects` — does any bit of `a & b` survive?
//!
//! See Phase 9 plan §7.5 + §7.2. The Phase 9 executor calls this on the
//! per-system conflict bitsets every dispatch round to detect whether a
//! ready system collides with any currently running system; the function
//! sits squarely on the dispatcher hot path, so the AVX2 fast path is
//! load-bearing on x86_64.
//!
//! # Algorithm
//!
//! On AVX2 hosts: process 256 bits (4 × u64 = 4 × `Block`) per iteration
//! via `_mm256_loadu_si256` + `_mm256_and_si256` + `_mm256_testz_si256`.
//! Tail blocks fall back to scalar.
//!
//! Without AVX2: plain scalar — iterate word by word, OR the partial
//! results, early-exit on first non-zero AND. The scalar version is the
//! correctness reference for the SIMD path; both share the same input
//! contract (equal-length `as_slice()` views into `FixedBitSet`).
//!
//! # Input contract
//!
//! Both `FixedBitSet` arguments must have been constructed with the same
//! `with_capacity(n)`; their `as_slice()` views thus carry equal length.
//! The `debug_assert!` enforces this contract — release builds compute
//! the intersection over `min(len_a, len_b)` blocks.

use fixedbitset::{Block, FixedBitSet};

/// Returns `true` iff `a` and `b` share at least one set bit.
///
/// Dispatches to [`bitset_intersects_avx2`] on x86_64 + AVX2 hosts and
/// [`bitset_intersects_scalar`] elsewhere. `#[inline]` because the body
/// is two function calls plus a const branch — the entire thing should
/// inline into the caller's `try_dispatch_ready` loop.
///
/// # Dead-code allowance
///
/// Wave 4 Step 10 ships this helper without a consumer; the Wave 5
/// Step 12 executor (`try_dispatch_ready`) is its sole call site. The
/// lint is silenced here rather than crate-wide so the absence of
/// consumers is intentional at this checkpoint.
#[allow(dead_code)]
#[inline]
pub(crate) fn bitset_intersects(a: &FixedBitSet, b: &FixedBitSet) -> bool {
    let a_slice = a.as_slice();
    let b_slice = b.as_slice();
    debug_assert_eq!(
        a_slice.len(),
        b_slice.len(),
        "bitset_intersects: FixedBitSet inputs must share capacity",
    );

    #[cfg(all(target_arch = "x86_64", target_feature = "avx2"))]
    {
        // SAFETY: `target_feature = "avx2"` is established by the compile-time
        //   gate, so every AVX2 intrinsic used by `bitset_intersects_avx2` is
        //   present on the executing CPU. The function itself documents its
        //   per-load invariants. The `cfg`-gated block is the function tail under
        //   `+avx2` (the `not(avx2)` block is excluded), so no `return` is needed
        //   (clippy::needless_return fires on it only under the `+avx2` gate).
        unsafe { bitset_intersects_avx2(a_slice, b_slice) }
    }

    #[cfg(not(all(target_arch = "x86_64", target_feature = "avx2")))]
    {
        bitset_intersects_scalar(a_slice, b_slice)
    }
}

/// Scalar reference implementation. Iterates word by word; early-exits
/// on the first non-zero AND. Block stride is `Block` = `usize` (8 B on
/// x86_64).
///
/// `#[inline]` so the compiler can fold it into the dispatcher when the
/// AVX2 gate is off.
#[allow(dead_code)] // consumed by `bitset_intersects` (gated) + Wave 5.
#[inline]
pub(crate) fn bitset_intersects_scalar(a: &[Block], b: &[Block]) -> bool {
    // The dispatcher already debug-asserted equal lengths; iterate to the
    // shorter side defensively (release builds with mismatched lengths
    // would otherwise underflow into garbage).
    let len = core::cmp::min(a.len(), b.len());
    for i in 0..len {
        if (a[i] & b[i]) != 0 {
            return true;
        }
    }
    false
}

/// AVX2 fast path — 256 bits (4 × `Block`) per iteration on x86_64.
///
/// # Safety
///
/// The caller must guarantee that the executing CPU supports AVX2. The
/// `#[cfg(target_feature = "avx2")]` gate on the module entry point and
/// the `#[target_feature(enable = "avx2")]` attribute on this function
/// jointly enforce this at compile time — direct callers from non-AVX2
/// hosts cannot link.
///
/// The function performs unaligned 256-bit loads (`_mm256_loadu_si256`),
/// so the input slices need not be aligned beyond `Block`'s natural
/// alignment. `_mm256_testz_si256` returns 1 iff the AND is all-zero;
/// inverting that is the intersection predicate.
#[cfg(all(target_arch = "x86_64", target_feature = "avx2"))]
#[target_feature(enable = "avx2")]
#[inline]
pub(crate) unsafe fn bitset_intersects_avx2(a: &[Block], b: &[Block]) -> bool {
    use core::arch::x86_64::{__m256i, _mm256_and_si256, _mm256_loadu_si256, _mm256_testz_si256};

    let len = core::cmp::min(a.len(), b.len());
    // `Block` = `usize` on supported targets (Phase 9 hardware = x86_64);
    // four blocks = 256 bits = one __m256i load.
    const STRIDE: usize = 4;
    let simd_chunks = len / STRIDE;

    for i in 0..simd_chunks {
        let base = i * STRIDE;
        // SAFETY: `base + STRIDE <= len <= a.len()`. `_mm256_loadu_si256`
        //   accepts any byte alignment; the cast from `*const Block` to
        //   `*const __m256i` is sound because the load reads exactly
        //   `STRIDE * size_of::<Block>() == 32` bytes, which equals
        //   `size_of::<__m256i>()`. `target_feature = "avx2"` gates the
        //   intrinsic to AVX2-capable CPUs.
        unsafe {
            let ai = _mm256_loadu_si256(a.as_ptr().add(base) as *const __m256i);
            let bi = _mm256_loadu_si256(b.as_ptr().add(base) as *const __m256i);
            let and = _mm256_and_si256(ai, bi);
            if _mm256_testz_si256(and, and) == 0 {
                return true;
            }
        }
    }

    // Tail: the last 0..STRIDE blocks fall through to scalar — the
    // compiler unrolls this small loop trivially.
    for i in (simd_chunks * STRIDE)..len {
        if (a[i] & b[i]) != 0 {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use fixedbitset::FixedBitSet;

    /// Two empty bitsets do not intersect.
    #[test]
    fn scalar_disjoint_empty() {
        let a = FixedBitSet::with_capacity(64);
        let b = FixedBitSet::with_capacity(64);
        assert!(!bitset_intersects_scalar(a.as_slice(), b.as_slice()));
    }

    /// Disjoint bit positions across multiple blocks do not intersect.
    #[test]
    fn scalar_disjoint() {
        let mut a = FixedBitSet::with_capacity(256);
        let mut b = FixedBitSet::with_capacity(256);
        a.insert(0);
        a.insert(64);
        a.insert(128);
        b.insert(1);
        b.insert(65);
        b.insert(129);
        assert!(!bitset_intersects_scalar(a.as_slice(), b.as_slice()));
    }

    /// Basic positive case — one shared bit in the first block.
    #[test]
    fn scalar_basic() {
        let mut a = FixedBitSet::with_capacity(64);
        let mut b = FixedBitSet::with_capacity(64);
        a.insert(7);
        b.insert(7);
        assert!(bitset_intersects_scalar(a.as_slice(), b.as_slice()));
    }

    /// Shared bit in a non-first block — verifies multi-word scanning.
    #[test]
    fn scalar_multi_block_match() {
        let mut a = FixedBitSet::with_capacity(512);
        let mut b = FixedBitSet::with_capacity(512);
        a.insert(300);
        b.insert(300);
        assert!(bitset_intersects_scalar(a.as_slice(), b.as_slice()));
    }

    /// SIMD path must agree with the scalar reference on every input.
    /// Gated on `target_feature = "avx2"` so the test only runs when the
    /// AVX2 fast path is compiled in.
    #[cfg(all(target_arch = "x86_64", target_feature = "avx2"))]
    #[test]
    fn avx2_matches_scalar() {
        use rand::{Rng, SeedableRng, rngs::StdRng};

        let mut rng = StdRng::seed_from_u64(0xb0_07_31_a1_b2_c3_d4_e5);
        for cap in [64usize, 256, 1024, 1025, 4096] {
            for trial in 0..16 {
                let mut a = FixedBitSet::with_capacity(cap);
                let mut b = FixedBitSet::with_capacity(cap);
                // Sparse fill — half the bits per side, randomised.
                for _ in 0..(cap / 8) {
                    a.insert(rng.random_range(0..cap));
                    b.insert(rng.random_range(0..cap));
                }
                let scalar = bitset_intersects_scalar(a.as_slice(), b.as_slice());
                // SAFETY: avx2 target feature is required by the test gate.
                let simd = unsafe { bitset_intersects_avx2(a.as_slice(), b.as_slice()) };
                assert_eq!(
                    scalar, simd,
                    "AVX2 path disagrees with scalar on cap={cap} trial={trial}"
                );
            }
        }
    }

    /// Public dispatcher matches the scalar reference (no AVX2 gate —
    /// runs on every host).
    #[test]
    fn public_dispatcher_matches_scalar() {
        let mut a = FixedBitSet::with_capacity(1024);
        let mut b = FixedBitSet::with_capacity(1024);
        a.insert(0);
        a.insert(500);
        a.insert(1023);
        b.insert(500);
        let dispatcher = bitset_intersects(&a, &b);
        let scalar = bitset_intersects_scalar(a.as_slice(), b.as_slice());
        assert_eq!(dispatcher, scalar);
        assert!(dispatcher, "shared bit 500 must trigger intersection");
    }
}
