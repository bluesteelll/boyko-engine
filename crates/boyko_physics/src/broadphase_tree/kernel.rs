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
//!
//! # The leaf-list tests (C3b, F1)
//!
//! The leaf-list query (`mod.rs`, [`QueryKernel::LeafList`](super::QueryKernel)) keeps, per
//! 8-row active leaf, a [`CandList`]: the leaves whose parent-lane box meets the leaf's box, as
//! six box rows, a leaf index and (on the active tree) the leaf's largest row, eight candidates
//! to a chunk. Each row then runs
//!
//! * the prefilter ([`CandList::for_each_kept`]): the six `LE_OQ` / `GE_OQ` compares of
//!   [`box_mask`], on bit copies of the same parent-lane boxes against the same [`QueryBox`] —
//!   so a candidate is kept exactly when the per-row walk would have tested its leaf — and, on
//!   the active tree, `maxrow > row` as a signed 32-bit compare (rows stay below `2^24`);
//! * [`leaf_mask_above`]: [`leaf_mask`] and `lane row > row` in one mask, the per-row walk's
//!   `t > row` filter moved into the vector. [`NO_LANE_ROW`](super::bvh::NO_LANE_ROW) reads as
//!   `-1`, which is above no row.
//!
//! The collection left-packs the selected lanes of a level-1 node with a 256-entry permutation
//! table ([`CandList::push_lanes`]). The integer ops (`cmpgt_epi32`, the permutes, the
//! zero-extension) are exact, so the arms agree bit for bit here too (G-LL4).
//!
//! # The small-segment sort (C3b, F2)
//!
//! [`sort_network`] sorts a leaf-list segment of up to [`NETWORK_SORT_MAX`] entries with a
//! bitonic network in one or two ymm registers (`vpminud` / `vpmaxud`, fixed shuffles and
//! blends, padded with `u32::MAX`): no data-dependent branch, where the insertion sort takes one
//! per shift. A sort's result does not depend on how it is reached, so the network writes the
//! insertion sort's bytes; the property test pins both arms against `sort_unstable` on every
//! length it takes.

use core::mem::MaybeUninit;

use super::bvh::{
    LANES, LEAF_MAXROW, LEAF_R, LEAF_ROW, LEAF_X, LEAF_Y, LEAF_Z, MAX_X, MAX_Y, MAX_Z, MIN_X,
    MIN_Y, MIN_Z, Node8,
};

/// The relative slack of a cull bound: `2^-20`, sixteen ulp of `f32`.
const SLACK_REL: f32 = 1.0 / 1_048_576.0;

/// The absolute slack of a cull bound: `2^-72`. Once `bound²` is sub-normal the exact test's
/// error is absolute (every `|dx| ≤ 2^-75` squares to `0`), and the box needs a width no
/// relative term supplies. The largest per-axis excess an accepted pair can have over
/// `|r_a| + |r_b|` is `2^-75`; two sides of `2^-72` cover it sixteen times. (The first cut had
/// `2^-126` here, sized for sub-normal *rows*, and missed pairs at `|p| ~ 2^-60`, `|r| ~ 2^-90`
/// — the review of C1, W1; `g1_subnormal_pairs_are_found` pins it.)
const SLACK_ABS: f32 = 1.0 / (1u128 << 72) as f32;

/// Candidate leaves one leaf-list collection may hold per tree. A leaf whose collection exceeds
/// it answers its rows with the per-row walk instead (the fallback, counted in
/// [`TreeDiag::fallback_leaves`](super::TreeDiag::fallback_leaves)). Counted at 1 000 rows the
/// largest collection is 89 on J and 126 on the disparity scene (design C3b, §6).
pub(crate) const LEAF_LIST_CAP: usize = 128;

/// A [`CandList`] row's length: the cap plus one chunk, so the eight-lane left-pack store at
/// `len ≤ LEAF_LIST_CAP` and the pad chunk both stay inside it.
const CAND_BUF: usize = LEAF_LIST_CAP + LANES;

// Every row of a `CandList` starts on a 32-byte boundary, so a chunk load is aligned.
const _: () = assert!(CAND_BUF.is_multiple_of(LANES) && (CAND_BUF * 4).is_multiple_of(32));

/// The left-pack permutations: byte `j` of entry `m` is the lane of the `j`-th set bit of `m`;
/// the bytes past `popcount(m)` are `0`.
const PACK_TABLE: [u64; 256] = pack_table();

const fn pack_table() -> [u64; 256] {
    let mut table = [0u64; 256];
    let mut m = 0usize;
    while m < 256 {
        let mut entry = 0u64;
        let mut j = 0u32;
        let mut k = 0u64;
        while k < 8 {
            if m & (1 << k) != 0 {
                entry |= k << (8 * j);
                j += 1;
            }
            k += 1;
        }
        table[m] = entry;
        m += 1;
    }
    table
}

/// Segment length up to which the leaf-list pass sorts with [`sort_network`].
pub(crate) const NETWORK_SORT_MAX: usize = 16;

/// The compare-exchange steps `(k, j)` of a bitonic sort of 16: block size `k`, partner
/// distance `j`. The first six sort each half of eight (ascending below lane 8, descending
/// above); the last four merge them.
const BITONIC_16: [(usize, usize); 10] = [(2, 1), (4, 2), (4, 1), (8, 4), (8, 2), (8, 1), (16, 8), (16, 4), (16, 2), (16, 1)];

/// The lanes `offset .. offset + 8` of step `(k, j)` that keep the larger of their pair, as an
/// eight-bit blend mask: lane `i` pairs with `i ^ j`; the lower of the pair (`i & j == 0`) keeps
/// the smaller in an ascending block (`i & k == 0`) and the larger in a descending one.
const fn keeps_max(k: usize, j: usize, offset: usize) -> i32 {
    let mut mask = 0i32;
    let mut lane = 0;
    while lane < 8 {
        let i = offset + lane;
        if (i & j != 0) != (i & k != 0) {
            mask |= 1 << lane;
        }
        lane += 1;
    }
    mask
}

/// Sorts `segment` ascending, `segment.len() ≤ NETWORK_SORT_MAX`: the leaf-list pass's
/// small-segment sort (C3b, F2; module docs, "The small-segment sort").
#[inline]
pub(crate) fn sort_network(segment: &mut [u32]) {
    debug_assert!(segment.len() <= NETWORK_SORT_MAX, "the network sorts at most 16 entries");
    #[cfg(all(target_arch = "x86_64", target_feature = "avx2", not(miri)))]
    {
        sort_network_avx2(segment);
    }
    #[cfg(not(all(target_arch = "x86_64", target_feature = "avx2", not(miri))))]
    {
        sort_network_scalar(segment);
    }
}

/// The scalar arm of [`sort_network`]: the same bitonic network of 16 over a `u32::MAX`-padded
/// copy. The reference the AVX2 arm is pinned against, and the arm every non-AVX2 build and
/// Miri run.
///
/// # Panics
///
/// If `segment.len() > NETWORK_SORT_MAX`.
//
// `dead_code`: as for `box_mask_scalar`.
#[allow(dead_code)]
#[inline]
pub(crate) fn sort_network_scalar(segment: &mut [u32]) {
    let n = segment.len();
    let mut v = [u32::MAX; NETWORK_SORT_MAX];
    v[..n].copy_from_slice(segment);
    for (k, j) in BITONIC_16 {
        for i in 0..NETWORK_SORT_MAX {
            let l = i ^ j;
            if l > i {
                let (lo, hi) = (v[i].min(v[l]), v[i].max(v[l]));
                let ascending = i & k == 0;
                (v[i], v[l]) = if ascending { (lo, hi) } else { (hi, lo) };
            }
        }
    }
    segment.copy_from_slice(&v[..n]);
}

#[cfg(all(target_arch = "x86_64", target_feature = "avx2", not(miri)))]
#[inline]
fn sort_network_avx2(segment: &mut [u32]) {
    use core::arch::x86_64::{
        __m256i, _mm256_andnot_si256, _mm256_cmpgt_epi32, _mm256_loadu_si256, _mm256_maskload_epi32,
        _mm256_maskstore_epi32, _mm256_max_epu32, _mm256_min_epu32, _mm256_or_si256,
        _mm256_set1_epi32, _mm256_setr_epi32, _mm256_storeu_si256,
    };
    let n = segment.len();
    debug_assert!(n <= NETWORK_SORT_MAX, "the network sorts at most 16 entries");
    if n < 2 {
        return;
    }
    let p = segment.as_mut_ptr();
    // SAFETY: `2 ≤ n ≤ 16`, and `segment` is borrowed mutably. The masked loads and stores
    // touch only the lanes their mask enables: `lane < n` from `p` (the `n ≤ 8` arm) and
    // `8 + lane < n` from `p.add(8)` (the `n > 8` arm, where `n ≥ 9` puts `p.add(8)` inside
    // `segment`), all inside it. The full load and store of the first register in the `n > 8`
    // arm touch `segment[0..8]`, inside it. `loadu` / `storeu` / `maskload` / `maskstore` have no
    // alignment requirement. The shuffles, min / max and blends are register ops. The `avx2`
    // feature is compiled in (the `cfg` on this function).
    unsafe {
        let iota = _mm256_setr_epi32(0, 1, 2, 3, 4, 5, 6, 7);
        let ones = _mm256_set1_epi32(-1);
        if n <= 8 {
            let live = _mm256_cmpgt_epi32(_mm256_set1_epi32(n as i32), iota);
            let v = _mm256_maskload_epi32(p.cast::<i32>(), live);
            let v = _mm256_or_si256(v, _mm256_andnot_si256(live, ones));
            // The bitonic sort of eight: the first six steps of the sixteen, lower half.
            let v = bitonic8_lower_avx2(v);
            _mm256_maskstore_epi32(p.cast::<i32>(), live, v);
        } else {
            let live = _mm256_cmpgt_epi32(_mm256_set1_epi32(n as i32 - 8), iota);
            let a = _mm256_loadu_si256(p.cast::<__m256i>());
            let b = _mm256_maskload_epi32(p.add(8).cast::<i32>(), live);
            let b = _mm256_or_si256(b, _mm256_andnot_si256(live, ones));
            // Each half sorted by the first six steps (the upper half descending, lanes 8..16),
            // then the four merge steps.
            let a = bitonic8_lower_avx2(a);
            let b = bitonic8_upper_avx2(b);
            let (a, b) = (_mm256_min_epu32(a, b), _mm256_max_epu32(a, b));
            let a = merge8_avx2(a);
            let b = merge8_avx2(b);
            _mm256_storeu_si256(p.cast::<__m256i>(), a);
            _mm256_maskstore_epi32(p.add(8).cast::<i32>(), live, b);
        }
    }
}

/// One compare-exchange step at partner distance `J ∈ {1, 2, 4}` within a register: every lane
/// meets `lane ^ J`, the lanes of `KEEP_MAX` keep the larger.
#[cfg(all(target_arch = "x86_64", target_feature = "avx2", not(miri)))]
#[inline]
fn exchange_avx2<const J: usize, const KEEP_MAX: i32>(v: core::arch::x86_64::__m256i) -> core::arch::x86_64::__m256i {
    use core::arch::x86_64::{
        _mm256_blend_epi32, _mm256_max_epu32, _mm256_min_epu32, _mm256_permute4x64_epi64,
        _mm256_shuffle_epi32,
    };
    // SAFETY: register ops only; the `avx2` feature is compiled in (the `cfg` on this function).
    unsafe {
        let partner = match J {
            1 => _mm256_shuffle_epi32::<0b10_11_00_01>(v),
            2 => _mm256_shuffle_epi32::<0b01_00_11_10>(v),
            _ => _mm256_permute4x64_epi64::<0b01_00_11_10>(v),
        };
        _mm256_blend_epi32::<KEEP_MAX>(_mm256_min_epu32(v, partner), _mm256_max_epu32(v, partner))
    }
}

/// The first six steps of [`BITONIC_16`] on lanes `0 .. 8`: sorts them ascending.
#[cfg(all(target_arch = "x86_64", target_feature = "avx2", not(miri)))]
#[inline]
fn bitonic8_lower_avx2(v: core::arch::x86_64::__m256i) -> core::arch::x86_64::__m256i {
    let v = exchange_avx2::<1, { keeps_max(2, 1, 0) }>(v);
    let v = exchange_avx2::<2, { keeps_max(4, 2, 0) }>(v);
    let v = exchange_avx2::<1, { keeps_max(4, 1, 0) }>(v);
    let v = exchange_avx2::<4, { keeps_max(8, 4, 0) }>(v);
    let v = exchange_avx2::<2, { keeps_max(8, 2, 0) }>(v);
    exchange_avx2::<1, { keeps_max(8, 1, 0) }>(v)
}

/// The first six steps of [`BITONIC_16`] on lanes `8 .. 16`: sorts them descending.
#[cfg(all(target_arch = "x86_64", target_feature = "avx2", not(miri)))]
#[inline]
fn bitonic8_upper_avx2(v: core::arch::x86_64::__m256i) -> core::arch::x86_64::__m256i {
    let v = exchange_avx2::<1, { keeps_max(2, 1, 8) }>(v);
    let v = exchange_avx2::<2, { keeps_max(4, 2, 8) }>(v);
    let v = exchange_avx2::<1, { keeps_max(4, 1, 8) }>(v);
    let v = exchange_avx2::<4, { keeps_max(8, 4, 8) }>(v);
    let v = exchange_avx2::<2, { keeps_max(8, 2, 8) }>(v);
    exchange_avx2::<1, { keeps_max(8, 1, 8) }>(v)
}

/// The last three steps of [`BITONIC_16`] within one register (every block ascending, so both
/// halves take the same masks): a bitonic eight becomes sorted.
#[cfg(all(target_arch = "x86_64", target_feature = "avx2", not(miri)))]
#[inline]
fn merge8_avx2(v: core::arch::x86_64::__m256i) -> core::arch::x86_64::__m256i {
    let v = exchange_avx2::<4, { keeps_max(16, 4, 0) }>(v);
    let v = exchange_avx2::<2, { keeps_max(16, 2, 0) }>(v);
    exchange_avx2::<1, { keeps_max(16, 1, 0) }>(v)
}


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

/// [`leaf_mask`] restricted to the lanes whose row is above `row`: the leaf-list query's exact
/// test, with the per-row walk's `t > row` filter in the mask. `row < 2^24`; an empty or
/// killed lane (row [`NO_LANE_ROW`](super::bvh::NO_LANE_ROW), `-1` as `i32`) is never above it.
#[inline]
pub(crate) fn leaf_mask_above(node: &Node8, x: f32, y: f32, z: f32, r: f32, row: u32) -> u32 {
    #[cfg(all(target_arch = "x86_64", target_feature = "avx2", not(miri)))]
    {
        leaf_mask_above_avx2(node, x, y, z, r, row)
    }
    #[cfg(not(all(target_arch = "x86_64", target_feature = "avx2", not(miri))))]
    {
        leaf_mask_above_scalar(node, x, y, z, r, row)
    }
}

/// The scalar arm of [`leaf_mask_above`].
//
// `dead_code`: as for `box_mask_scalar`.
#[allow(dead_code)]
#[inline]
pub(crate) fn leaf_mask_above_scalar(node: &Node8, x: f32, y: f32, z: f32, r: f32, row: u32) -> u32 {
    let mut above = 0u32;
    for k in 0..LANES {
        let hit = node.p[LEAF_ROW][k].to_bits() as i32 > row as i32;
        above |= u32::from(hit) << k;
    }
    leaf_mask_scalar(node, x, y, z, r) & above
}

/// The candidate leaves of one leaf-list collection, structure of arrays: each candidate's
/// parent-lane box (six rows in the [`Node8`] internal row order), its leaf index and, on the
/// active tree, its largest row ([`LEAF_MAXROW`]). A row's tests read it eight candidates — one
/// chunk — at a time.
///
/// Function-local scratch of the query pass (about 4.4 KB), never initialised as a whole: every
/// push writes all eight arrays of each lane it appends, and [`seal`](Self::seal) writes one
/// pad chunk behind the last candidate, whose empty box (`+inf` mins, `-inf` maxes) meets no
/// query box. A lane, once written, stays initialised for the list's lifetime, so every lane
/// below `LANES · chunks` is initialised whenever `chunks` was set by a seal. A push never takes
/// the list past [`LEAF_LIST_CAP`], whatever cap it is given, so the seal's pad chunk fits.
#[repr(C, align(32))]
pub(crate) struct CandList {
    /// `b[MIN_X ..= MAX_Z][i]`: candidate `i`'s parent-lane box.
    b: [[MaybeUninit<f32>; CAND_BUF]; 6],
    /// Candidate `i`'s leaf node index (level 0).
    idx: [MaybeUninit<u32>; CAND_BUF],
    /// Candidate `i`'s largest live row (read by the active list's prefilter only; a static
    /// list's pushes write `0`).
    maxrow: [MaybeUninit<u32>; CAND_BUF],
    /// Candidates pushed.
    len: usize,
    /// Chunks readable since the last seal: `len.div_ceil(LANES)` then, `0` after a clear.
    chunks: usize,
}

impl CandList {
    /// An empty list built in `slot`, writing only its two counts: nothing is written into the
    /// rows, and nothing is moved. (A `Self { .. }` literal with uninit rows lets LLVM merge the
    /// zeroed counts with the adjacent uninit row into one `memset` call per list per pass,
    /// measured in the pass's prologue; this form writes 16 bytes.)
    #[inline]
    pub(crate) fn init_in(slot: &mut MaybeUninit<Self>) -> &mut Self {
        let p = slot.as_mut_ptr();
        // SAFETY: `p` points to `slot`'s storage, valid for writes and aligned for `Self`, and
        // `&raw mut` projects to the two count fields without creating a reference to the
        // uninitialised whole. After the two writes every field is valid: the counts are
        // initialised and the row fields are arrays of `MaybeUninit`, valid in any state. So
        // `slot` holds a valid `Self`, borrowed mutably for the returned reference's lifetime.
        unsafe {
            (&raw mut (*p).len).write(0);
            (&raw mut (*p).chunks).write(0);
            slot.assume_init_mut()
        }
    }

    /// Candidates held.
    #[inline]
    pub(crate) fn len(&self) -> usize {
        self.len
    }

    /// Chunks of eight candidates a row tests (`0` until sealed; test and `bp-query-counts`
    /// builds only).
    #[cfg(any(test, feature = "bp-query-counts"))]
    pub(crate) fn chunks(&self) -> usize {
        self.chunks
    }

    /// Empties the list.
    #[inline]
    pub(crate) fn clear(&mut self) {
        self.len = 0;
        self.chunks = 0;
    }

    /// Appends one candidate: leaf `leaf` with box `[lo, hi]` and largest row `maxrow`. Returns
    /// `false`, appending nothing, when the list would exceed `cap`, clamped to
    /// [`LEAF_LIST_CAP`] as in [`push_lanes`](Self::push_lanes).
    #[inline]
    pub(crate) fn push_one(&mut self, lo: [f32; 3], hi: [f32; 3], leaf: u32, maxrow: u32, cap: usize) -> bool {
        debug_assert!(cap <= LEAF_LIST_CAP, "invariant: the cap fits the buffer");
        let cap = cap.min(LEAF_LIST_CAP);
        let i = self.len;
        if i + 1 > cap {
            return false;
        }
        let rows = [lo[0], lo[1], lo[2], hi[0], hi[1], hi[2]];
        for (row, v) in self.b.iter_mut().zip(rows) {
            row[i] = MaybeUninit::new(v);
        }
        self.idx[i] = MaybeUninit::new(leaf);
        self.maxrow[i] = MaybeUninit::new(maxrow);
        self.len = i + 1;
        self.chunks = 0;
        true
    }

    /// Left-packs the lanes of the level-1 `node` set in `mask` (its box rows, and leaf
    /// indices `first + lane`) behind the candidates held, in ascending lane order. With
    /// `leaves`, each pushed candidate's largest row is read from its leaf node's
    /// [`LEAF_MAXROW`] row; without, it is `0`. Returns `false`, appending nothing, when the
    /// list would exceed `cap`.
    ///
    /// A caller passes `cap ≤ LEAF_LIST_CAP` and an eight-bit `mask` (debug-asserted). Every
    /// build also clamps `cap` to [`LEAF_LIST_CAP`] and `mask` to its eight lane bits, so a
    /// caller that breaks either gets a wrong list, never a store past a row or an appended
    /// lane nobody wrote.
    #[inline]
    pub(crate) fn push_lanes(&mut self, node: &Node8, mask: u32, first: u32, cap: usize, leaves: Option<&[Node8]>) -> bool {
        self.push_lanes_arm::<false>(node, mask, first, cap, leaves)
    }

    /// [`push_lanes`](Self::push_lanes) with the store arm chosen: `SCALAR` forces the scalar
    /// arm (G-LL4's reference); otherwise the build's arm.
    #[inline]
    pub(crate) fn push_lanes_arm<const SCALAR: bool>(
        &mut self,
        node: &Node8,
        mask: u32,
        first: u32,
        cap: usize,
        leaves: Option<&[Node8]>,
    ) -> bool {
        debug_assert!(cap <= LEAF_LIST_CAP, "invariant: the cap fits the buffer");
        debug_assert!(mask < 1 << LANES, "a lane mask has eight bits");
        // The AVX2 arm's stores and every later read of the list rest on these two bounds, so
        // they are enforced here in every build, not trusted to the caller: the list never
        // passes `LEAF_LIST_CAP`, so an eight-lane store at `len` ends inside the rows; and a
        // push appends only lanes it writes (the AVX2 arm writes the eight lanes of `mask`'s low
        // byte). Both are no-ops for `collect_leaves`, whose cap is at most `LEAF_LIST_CAP` and
        // whose mask is a `box_mask`, and both fold away in the production object: the cap
        // there is the constant `LEAF_LIST_CAP`, and a movemask's high bits are known zero.
        // Where the cap is not a constant (the test build), the clamp is loop-invariant.
        let cap = cap.min(LEAF_LIST_CAP);
        let mask = mask & ((1 << LANES) - 1);
        let len = self.len;
        let count = mask.count_ones() as usize;
        if len + count > cap {
            return false;
        }
        if SCALAR {
            self.push_lanes_scalar(node, mask, first);
        } else {
            #[cfg(all(target_arch = "x86_64", target_feature = "avx2", not(miri)))]
            {
                // SAFETY: `len + count ≤ cap ≤ LEAF_LIST_CAP`: the check above, against the cap
                // clamped above, both in every build. So `self.len = len ≤ LEAF_LIST_CAP`.
                unsafe { self.push_lanes_avx2(node, mask, first) };
            }
            #[cfg(not(all(target_arch = "x86_64", target_feature = "avx2", not(miri))))]
            self.push_lanes_scalar(node, mask, first);
        }
        if let Some(leaves) = leaves {
            let mut m = mask;
            let mut i = len;
            while m != 0 {
                let k = m.trailing_zeros();
                m &= m - 1;
                let leaf = &leaves[(first + k) as usize];
                self.maxrow[i] = MaybeUninit::new(leaf.p[LEAF_MAXROW][0].to_bits());
                i += 1;
            }
        }
        self.len = len + count;
        self.chunks = 0;
        true
    }

    /// The scalar arm of the left-pack: the reference the AVX2 arm is pinned against (G-LL4).
    #[inline]
    fn push_lanes_scalar(&mut self, node: &Node8, mask: u32, first: u32) {
        let mut m = mask;
        let mut i = self.len;
        while m != 0 {
            let k = m.trailing_zeros() as usize;
            m &= m - 1;
            for (row, src) in self.b.iter_mut().zip(&node.p) {
                row[i] = MaybeUninit::new(src[k]);
            }
            self.idx[i] = MaybeUninit::new(first + k as u32);
            self.maxrow[i] = MaybeUninit::new(0);
            i += 1;
        }
    }

    /// The AVX2 arm of the left-pack: eight-lane stores at `len` into every row, whatever
    /// `popcount(mask)`.
    ///
    /// # Safety
    ///
    /// `self.len ≤ LEAF_LIST_CAP`.
    #[cfg(all(target_arch = "x86_64", target_feature = "avx2", not(miri)))]
    #[inline]
    unsafe fn push_lanes_avx2(&mut self, node: &Node8, mask: u32, first: u32) {
        use core::arch::x86_64::{
            __m256i, _mm_cvtsi64_si128, _mm256_add_epi32, _mm256_cvtepu8_epi32, _mm256_load_ps,
            _mm256_permutevar8x32_epi32, _mm256_permutevar8x32_ps, _mm256_set1_epi32,
            _mm256_setr_epi32, _mm256_setzero_si256, _mm256_storeu_ps, _mm256_storeu_si256,
        };
        let len = self.len;
        debug_assert!(len <= LEAF_LIST_CAP, "invariant: a push starts at or below the cap");
        let entry = PACK_TABLE[(mask & 0xff) as usize];
        // SAFETY: `len ≤ LEAF_LIST_CAP`, the caller's guarantee. The one caller, `push_lanes_arm`,
        // holds it with a check that runs in every build, `len + popcount(mask) ≤ cap` with its
        // `cap` first clamped to `LEAF_LIST_CAP`, so the bound does not rest on the cap its own
        // caller passes (that one is only debug-asserted). So the eight-lane stores at `len` end
        // at `len + 8 ≤ CAND_BUF`, inside each row array, whose pointer is derived from
        // `&mut self` and so is valid for writes; `storeu` has no alignment requirement and the
        // lanes are `MaybeUninit`, so writing them is always sound. The loads read the 32-byte
        // aligned, initialised `[f32; 8]` rows of the borrowed node (`Node8` is
        // `#[repr(C, align(32))]`). The `avx2` feature is compiled in (the `cfg` on this
        // function). The lanes past `popcount(mask)` receive permuted node lanes: initialised
        // values that the next push or the seal's pad overwrites before a row reads them as
        // candidates.
        unsafe {
            let perm = _mm256_cvtepu8_epi32(_mm_cvtsi64_si128(entry as i64));
            for (row, src) in self.b.iter_mut().zip(&node.p) {
                let v = _mm256_permutevar8x32_ps(_mm256_load_ps(src.as_ptr()), perm);
                _mm256_storeu_ps(row.as_mut_ptr().add(len).cast::<f32>(), v);
            }
            let ids = _mm256_add_epi32(
                _mm256_set1_epi32(first as i32),
                _mm256_setr_epi32(0, 1, 2, 3, 4, 5, 6, 7),
            );
            _mm256_storeu_si256(
                self.idx.as_mut_ptr().add(len).cast::<__m256i>(),
                _mm256_permutevar8x32_epi32(ids, perm),
            );
            _mm256_storeu_si256(self.maxrow.as_mut_ptr().add(len).cast::<__m256i>(), _mm256_setzero_si256());
        }
    }

    /// Closes the list for reading: one pad chunk behind the last candidate (an empty box,
    /// leaf `0`, max row `0`), then `chunks = len.div_ceil(LANES)`.
    #[inline]
    pub(crate) fn seal(&mut self) {
        let len = self.len;
        debug_assert!(len <= LEAF_LIST_CAP, "invariant: a list holds at most the cap");
        for (r, row) in self.b.iter_mut().enumerate() {
            let pad = if r < MAX_X { f32::INFINITY } else { f32::NEG_INFINITY };
            for lane in &mut row[len..len + LANES] {
                *lane = MaybeUninit::new(pad);
            }
        }
        for lane in &mut self.idx[len..len + LANES] {
            *lane = MaybeUninit::new(0);
        }
        for lane in &mut self.maxrow[len..len + LANES] {
            *lane = MaybeUninit::new(0);
        }
        self.chunks = len.div_ceil(LANES);
    }

    /// Calls `f` with the leaf index of every candidate that passes the prefilter against the
    /// query box `q` — the cull of [`box_mask`] on the candidates' parent-lane boxes and, with
    /// `MAXROW`, `maxrow > row` — in candidate order, eight candidates per test. A leaf whose
    /// largest row is at or below `row` holds no partner the row emits, so the active list is
    /// read with `MAXROW` and the static one without.
    ///
    /// The per-row loop of the leaf-list query: it owns the chunk bound, so neither the chunk
    /// reads nor the index reads carry a check of their own.
    #[inline]
    pub(crate) fn for_each_kept<const MAXROW: bool>(&self, q: &QueryBox, row: u32, mut f: impl FnMut(u32)) {
        for c in 0..self.chunks {
            // SAFETY: `c < chunks`, the loop bound.
            let mut m = unsafe { self.prefilter_unchecked::<MAXROW>(c, q, row) };
            while m != 0 {
                let k = m.trailing_zeros() as usize;
                m &= m - 1;
                debug_assert!(k < LANES, "a prefilter mask has eight bits");
                // SAFETY: `c < chunks` (the loop bound) and `k < LANES` (a set bit of a prefilter
                // mask, which both arms build from eight lanes: `movemask_ps` of eight lanes, or
                // eight shifted bits), so `8c + k < LANES · chunks ≤ len + LANES ≤ CAND_BUF`: a
                // lane a push or the seal's pad wrote (see `prefilter_scalar_unchecked`).
                f(unsafe { self.idx.get_unchecked(LANES * c + k).assume_init() });
            }
        }
    }

    /// Test-only: candidates of chunk `c` whose box meets `q` (bit `k` for candidate `8c + k`),
    /// through the build's arm.
    ///
    /// # Panics
    ///
    /// If `c ≥ chunks()`.
    #[cfg(test)]
    pub(crate) fn prefilter_box(&self, c: usize, q: &QueryBox) -> u32 {
        assert!(c < self.chunks, "invariant: a sealed chunk");
        // SAFETY: `c < chunks`, asserted above.
        unsafe { self.prefilter_unchecked::<false>(c, q, 0) }
    }

    /// Test-only: `prefilter_box` and `maxrow > row`, through the build's arm.
    ///
    /// # Panics
    ///
    /// If `c ≥ chunks()`.
    #[cfg(test)]
    pub(crate) fn prefilter_box_maxrow(&self, c: usize, q: &QueryBox, row: u32) -> u32 {
        assert!(c < self.chunks, "invariant: a sealed chunk");
        // SAFETY: `c < chunks`, asserted above.
        unsafe { self.prefilter_unchecked::<true>(c, q, row) }
    }

    /// Test-only: the scalar arm of both prefilters, the reference the AVX2 arm is pinned
    /// against (G-LL4).
    ///
    /// # Panics
    ///
    /// If `c ≥ chunks()`.
    #[cfg(test)]
    pub(crate) fn prefilter_scalar<const MAXROW: bool>(&self, c: usize, q: &QueryBox, row: u32) -> u32 {
        assert!(c < self.chunks, "invariant: a sealed chunk");
        // SAFETY: `c < chunks`, asserted above.
        unsafe { self.prefilter_scalar_unchecked::<MAXROW>(c, q, row) }
    }

    /// The build's prefilter arm.
    ///
    /// # Safety
    ///
    /// `c < self.chunks()`.
    #[inline]
    unsafe fn prefilter_unchecked<const MAXROW: bool>(&self, c: usize, q: &QueryBox, row: u32) -> u32 {
        #[cfg(all(target_arch = "x86_64", target_feature = "avx2", not(miri)))]
        {
            // SAFETY: the caller guarantees `c < chunks`.
            unsafe { self.prefilter_avx2::<MAXROW>(c, q, row) }
        }
        #[cfg(not(all(target_arch = "x86_64", target_feature = "avx2", not(miri))))]
        {
            // SAFETY: the caller guarantees `c < chunks`.
            unsafe { self.prefilter_scalar_unchecked::<MAXROW>(c, q, row) }
        }
    }

    /// The scalar prefilter: the arm every non-AVX2 build and Miri run, and G-LL4's reference.
    ///
    /// # Safety
    ///
    /// `c < self.chunks()`.
    //
    // `dead_code`: as for `box_mask_scalar` (an AVX2 non-test build has no caller).
    #[allow(dead_code)]
    #[inline]
    unsafe fn prefilter_scalar_unchecked<const MAXROW: bool>(&self, c: usize, q: &QueryBox, row: u32) -> u32 {
        debug_assert!(c < self.chunks, "invariant: a sealed chunk");
        let mut mask = 0u32;
        for k in 0..LANES {
            let i = LANES * c + k;
            // SAFETY: the caller guarantees `c < chunks`, which a seal set to
            // `len.div_ceil(LANES)`, so `i < LANES · chunks ≤ len + LANES − 1 < CAND_BUF`: a lane
            // a push wrote (`i < len`; every push writes all eight arrays of each lane it
            // appends) or the seal's pad wrote (`len ≤ i < len + LANES`). A written lane stays
            // initialised (see the type docs).
            let b = unsafe {
                [
                    self.b[MIN_X].get_unchecked(i).assume_init(),
                    self.b[MIN_Y].get_unchecked(i).assume_init(),
                    self.b[MIN_Z].get_unchecked(i).assume_init(),
                    self.b[MAX_X].get_unchecked(i).assume_init(),
                    self.b[MAX_Y].get_unchecked(i).assume_init(),
                    self.b[MAX_Z].get_unchecked(i).assume_init(),
                ]
            };
            let mut hit = q.lo[0] <= b[MAX_X]
                && q.hi[0] >= b[MIN_X]
                && q.lo[1] <= b[MAX_Y]
                && q.hi[1] >= b[MIN_Y]
                && q.lo[2] <= b[MAX_Z]
                && q.hi[2] >= b[MIN_Z];
            if MAXROW {
                // SAFETY: as above; a push writes the lane's max row too.
                let maxrow = unsafe { self.maxrow.get_unchecked(i).assume_init() };
                hit &= maxrow as i32 > row as i32;
            }
            mask |= u32::from(hit) << k;
        }
        mask
    }

    /// The AVX2 prefilter.
    ///
    /// # Safety
    ///
    /// `c < self.chunks()`.
    #[cfg(all(target_arch = "x86_64", target_feature = "avx2", not(miri)))]
    #[inline]
    unsafe fn prefilter_avx2<const MAXROW: bool>(&self, c: usize, q: &QueryBox, row: u32) -> u32 {
        use core::arch::x86_64::{
            __m256, __m256i, _CMP_GE_OQ, _CMP_LE_OQ, _mm256_and_ps, _mm256_castsi256_ps,
            _mm256_cmp_ps, _mm256_cmpgt_epi32, _mm256_load_ps, _mm256_load_si256,
            _mm256_movemask_ps, _mm256_set1_epi32, _mm256_set1_ps,
        };
        debug_assert!(c < self.chunks, "invariant: a sealed chunk");
        let base = LANES * c;
        // SAFETY: the caller guarantees `c < chunks`, so lanes `base .. base + 8` lie below
        // `LANES · chunks ≤ len + LANES ≤ CAND_BUF` and were written by a push or the seal's pad
        // (a written lane stays initialised; see the type docs). Each row array is 32-byte
        // aligned (`#[repr(C, align(32))]`, rows of `CAND_BUF · 4` bytes, a multiple of 32, as
        // the const assertion above pins) and `base · 4` is a multiple of 32, so the aligned
        // loads are aligned. The `avx2` feature is compiled in (the `cfg` on this function).
        unsafe {
            let load = |r: usize| -> __m256 { _mm256_load_ps(self.b[r].as_ptr().add(base).cast::<f32>()) };
            let m = _mm256_cmp_ps::<_CMP_LE_OQ>(_mm256_set1_ps(q.lo[0]), load(MAX_X));
            let m = _mm256_and_ps(m, _mm256_cmp_ps::<_CMP_GE_OQ>(_mm256_set1_ps(q.hi[0]), load(MIN_X)));
            let m = _mm256_and_ps(m, _mm256_cmp_ps::<_CMP_LE_OQ>(_mm256_set1_ps(q.lo[1]), load(MAX_Y)));
            let m = _mm256_and_ps(m, _mm256_cmp_ps::<_CMP_GE_OQ>(_mm256_set1_ps(q.hi[1]), load(MIN_Y)));
            let m = _mm256_and_ps(m, _mm256_cmp_ps::<_CMP_LE_OQ>(_mm256_set1_ps(q.lo[2]), load(MAX_Z)));
            let m = _mm256_and_ps(m, _mm256_cmp_ps::<_CMP_GE_OQ>(_mm256_set1_ps(q.hi[2]), load(MIN_Z)));
            let m = if MAXROW {
                let maxrow = _mm256_load_si256(self.maxrow.as_ptr().add(base).cast::<__m256i>());
                let above = _mm256_cmpgt_epi32(maxrow, _mm256_set1_epi32(row as i32));
                _mm256_and_ps(m, _mm256_castsi256_ps(above))
            } else {
                m
            };
            _mm256_movemask_ps(m) as u32
        }
    }

    /// Test-only: candidate `i`'s box rows, leaf index and max row.
    ///
    /// # Panics
    ///
    /// If `i ≥ len()`.
    #[cfg(test)]
    pub(crate) fn lane(&self, i: usize) -> ([f32; 6], u32, u32) {
        assert!(i < self.len, "a pushed candidate");
        // SAFETY: `i < len`: a lane a push wrote, and every push writes all eight arrays of each
        // lane it appends.
        unsafe {
            (
                [
                    self.b[MIN_X][i].assume_init(),
                    self.b[MIN_Y][i].assume_init(),
                    self.b[MIN_Z][i].assume_init(),
                    self.b[MAX_X][i].assume_init(),
                    self.b[MAX_Y][i].assume_init(),
                    self.b[MAX_Z][i].assume_init(),
                ],
                self.idx[i].assume_init(),
                self.maxrow[i].assume_init(),
            )
        }
    }

    /// Test-only: the leaf index of candidate `8c + k`.
    ///
    /// # Panics
    ///
    /// If `c ≥ chunks()` or `k ≥ 8`.
    #[cfg(test)]
    pub(crate) fn leaf(&self, c: usize, k: usize) -> u32 {
        assert!(c < self.chunks && k < LANES, "invariant: a lane of a sealed chunk");
        // SAFETY: `8c + k < LANES · chunks ≤ len + LANES ≤ CAND_BUF`, a lane a push or the
        // seal's pad wrote (see `prefilter_scalar_unchecked`).
        unsafe { self.idx.get_unchecked(LANES * c + k).assume_init() }
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

#[cfg(all(target_arch = "x86_64", target_feature = "avx2", not(miri)))]
#[inline]
fn leaf_mask_above_avx2(node: &Node8, x: f32, y: f32, z: f32, r: f32, row: u32) -> u32 {
    use core::arch::x86_64::{
        __m256i, _CMP_LE_OQ, _mm256_add_ps, _mm256_and_ps, _mm256_castsi256_ps, _mm256_cmp_ps,
        _mm256_cmpgt_epi32, _mm256_load_ps, _mm256_load_si256, _mm256_movemask_ps,
        _mm256_mul_ps, _mm256_set1_epi32, _mm256_set1_ps, _mm256_sub_ps,
    };
    // SAFETY: as in `leaf_mask_avx2`, plus the `LEAF_ROW` row: a 32-byte aligned `[f32; 8]`
    // of the borrowed node read as eight `i32` (the same bits; every bit pattern is a valid
    // `i32`). The float arithmetic is `leaf_mask_avx2`'s op for op; the row compare is exact.
    unsafe {
        let dx = _mm256_sub_ps(_mm256_load_ps(node.p[LEAF_X].as_ptr()), _mm256_set1_ps(x));
        let dy = _mm256_sub_ps(_mm256_load_ps(node.p[LEAF_Y].as_ptr()), _mm256_set1_ps(y));
        let dz = _mm256_sub_ps(_mm256_load_ps(node.p[LEAF_Z].as_ptr()), _mm256_set1_ps(z));
        let t = _mm256_mul_ps(dx, dx);
        let t = _mm256_add_ps(t, _mm256_mul_ps(dy, dy));
        let t = _mm256_add_ps(t, _mm256_mul_ps(dz, dz));
        let bound = _mm256_add_ps(_mm256_load_ps(node.p[LEAF_R].as_ptr()), _mm256_set1_ps(r));
        let bb = _mm256_mul_ps(bound, bound);
        let hit = _mm256_cmp_ps::<_CMP_LE_OQ>(t, bb);
        let rows = _mm256_load_si256(node.p[LEAF_ROW].as_ptr().cast::<__m256i>());
        let above = _mm256_cmpgt_epi32(rows, _mm256_set1_epi32(row as i32));
        _mm256_movemask_ps(_mm256_and_ps(hit, _mm256_castsi256_ps(above))) as u32
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
