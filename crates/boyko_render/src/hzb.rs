//! VG R3 step S3 — the HOST ORACLE for the hierarchical-Z pyramid (HZB) and the two-pass
//! occlusion test.
//!
//! This module is pure CPU and lands BEFORE any pyramid shader exists. That order is deliberate:
//! every proof below is cheaper to get right in Rust than in HLSL, and once this reference is
//! pinned a GPU/CPU disagreement means a SHADER bug rather than a math bug. It is the same split
//! that made rung R2d's corpus gate a cross-oracle check instead of a guess — see
//! [`crate::frustum`], which plays the identical role for the frustum arm.
//!
//! Nothing here is on a frame path. The shipped pyramid is a GPU image built by compute; this is
//! the thing the gates compare it against.
//!
//! # The pyramid
//!
//! ## Why the base is `prev_pow2`, not the source extent
//!
//! Level 0 is [`prev_pow2`] of each source axis, INDEPENDENTLY. A power-of-two base is what makes
//! every subsequent level exactly half of its predecessor, which in turn makes the
//! containing-texel map the exact shift [`HzbAxis::containing_texel`] and makes a 2×2 reduce a
//! partition of the finer level rather than a rounding argument. Choosing `next_pow2` instead
//! would leave level 0 with texels whose preimage in the source is EMPTY — an empty min is `+∞`,
//! which is the one direction that deletes geometry.
//!
//! Per-level extent is `max(1, base >> k)`. The `max(1, …)` is what lets a non-square base
//! (`1024 × 64`) keep halving on its long axis after the short one has bottomed out; at that
//! point the short axis's 2×2 footprint clamps to 1×2 and the reduce is a copy on that axis.
//!
//! ## The base map, in INTEGER arithmetic
//!
//! Source pixel `x ∈ [0, S)` maps to level-0 texel `t ∈ [0, P)` where `P = prev_pow2(S) ≤ S`:
//!
//! ```text
//!     forward   t = (x * P) / S                    (integer floor division)
//!     inverse   first(t) = (t * S + P - 1) / P     (integer division = ⌈t·S/P⌉)
//! ```
//!
//! Both are integer. Never `f32`: at a 4K extent the products exceed `2^24` and a single-precision
//! round can move a pixel into the neighbouring texel, which is exactly the off-by-one that breaks
//! COVERAGE and turns a conservative test into a geometry-deleting one.
//!
//! **Why the ceiling form is the one that partitions.** For positive integers,
//! `⌊x·P/S⌋ ≥ t  ⟺  x·P ≥ t·S  ⟺  x ≥ t·S/P  ⟺  x ≥ ⌈t·S/P⌉` (the last step because `x` is an
//! integer). So the preimage of texel `t` is exactly the half-open interval
//! `[⌈t·S/P⌉, ⌈(t+1)·S/P⌉)`, and `⌈a/b⌉` in integers is `(a + b - 1)/b` — the form above. Since
//! `P ≤ S`, consecutive `first` values differ by at least `⌊S/P⌋ ≥ 1`, so no interval is empty;
//! `first(0) = 0` and `first(P) = S`, so the intervals tile `[0, S)`. That is a PARTITION: every
//! source pixel belongs to exactly one level-0 texel, no gaps, no overlaps.
//!
//! Writing `first(t) = ⌊t·S/P⌋` instead would also tile `[0, S)` — but it tiles it as the preimage
//! of a DIFFERENT (ceiling-based) forward map. Paired with the floor forward map above it is off
//! by one at every `t` where `P ∤ t·S`, and the resulting interval neither contains nor is
//! contained in the true preimage. The two halves must be derived from each other, not chosen
//! independently.
//!
//! ## The reduce is `min`, and under reverse-Z that is the FARTHEST surface
//!
//! This engine renders hardware reverse-Z: `VK_COMPARE_OP_GREATER`, depth cleared to `0.0`, so a
//! LARGER stored depth is NEARER and `0.0` is the far plane. The late-pass reject predicate is
//! `depth_near(i) < occ(i)`, and its soundness proof needs `occ ≤ D[p]` for every source pixel `p`
//! the sampled texels cover. A lower bound over a footprint is a `min`. Under reverse-Z the
//! smallest depth is the FARTHEST surface, so the pyramid holds, per footprint, the depth of the
//! thing furthest from the eye — an instance may only be rejected if it is behind even that.
//!
//! A `max` here would hold the NEAREST surface in each footprint and would reject anything behind
//! the nearest occluder anywhere in its screen rect, deleting geometry visible through gaps. It
//! would be silently wrong in the one direction that has no visual tell in a static golden, which
//! is why the derivation is written down rather than left to the reader.
//!
//! NaN policy: a NaN depth is UNKNOWN, and the only conservative reading of unknown is
//! "infinitely far", so [`conservative_min`] collapses to `f32::NEG_INFINITY` and the verdict can
//! never be `Reject`. A physical reverse-Z attachment cannot hold NaN (the rasteriser clamps to
//! `[minDepth, maxDepth]`), so this is a contract, not a hot case.
//!
//! # The occlusion test
//!
//! ## The viewport transform, VERIFIED rather than assumed
//!
//! The `vb_raster` pass binds a viewport of `x = 0, y = 0, width = +W, height = +H,
//! minDepth = 0, maxDepth = 1` — POSITIVE height, no `VK_KHR_maintenance1` negative-height flip
//! (`crates/boyko_rhi_vulkan/src/present/passes/vb.rs`, the `full_viewport` binding). Vulkan's
//! viewport transform is therefore
//!
//! ```text
//!     x_win = (x_ndc + 1) * W / 2
//!     y_win = (y_ndc + 1) * H / 2        ← +Y NDC goes DOWN the framebuffer
//!     z_win = z_ndc                      ← minDepth 0, maxDepth 1
//! ```
//!
//! The GL-style form `(1 - y_ndc) * H / 2` would place the rect on the MIRRORED rows. Against a
//! vertically symmetric occlusion field that is invisible; against a real one it reads the wrong
//! texels and can reject a visible instance. Assuming the flip is the default mistake, so the
//! transform above is stated and pinned.
//!
//! **"No flip" is a property of the VIEWPORT STAGE, not of the engine.** The engine's Y-flip is in
//! the PROJECTION: [`crate::view::forward_view_proj_rows`] builds `sy = -1.0 / tan` into row 1, and
//! the positive-height viewport then passes that already-flipped Y through unchanged. So the caller
//! must hand this oracle the SAME matrix the raster draws with. A hand-built "unflipped" projection
//! satisfies every guard here and yields vertically MIRRORED rects, silently.
//!
//! ## `depth_near` is the MAX over the eight projected corners
//!
//! Reverse-Z: larger is nearer, so the nearest point of the bound is the largest `z_ndc`. Taking
//! it over the eight AABB corners is not an approximation when every corner has `w > 0`:
//! `z_ndc = clip.z / clip.w` with both numerator and denominator AFFINE in world position, i.e. a
//! projective function. Along any line segment on which the denominator keeps one sign, a
//! projective function has a derivative of constant sign and is therefore monotone, so its
//! maximum over a convex polytope is attained at a VERTEX. The `w > 0` guard is what buys that
//! argument; without it the claim is simply false.
//!
//! ## The guards, each erring toward KEEP
//!
//! Every guard below returns a named [`KeepReason`]. There is no clamping, no "best effort": the
//! reject predicate is the only path that can remove geometry, so every uncertainty leaves it.
//!
//! * [`KeepReason::UnknownBounds`] — an inverted AABB is the "bounds unknown" sentinel (a mesh
//!   still streaming in, the reserved geometry slot, the zero-vertex fold). Tested FIRST, before
//!   any projection, at the single shared entry point [`project_aabb`] — the same structural
//!   discipline [`crate::frustum::instance_visible_after_cull`] already uses, and for the same
//!   reason: projected first, a degenerate transform collapses the sentinel to a point and
//!   "bounds unknown" silently comes to mean "cull it".
//! * [`KeepReason::BehindEye`] — any corner with `clip.w <= 0`. The perspective divide is
//!   meaningless there and the vertex-extremum argument above fails.
//! * [`KeepReason::NonFinite`] — any non-finite clip coordinate or window coordinate. A NaN
//!   compares false against everything, so left unguarded it would disarm parts of the test in an
//!   uncontrolled way rather than loudly.
//! * [`KeepReason::EmptyRect`] — the pixel rect is empty after clamping to the framebuffer, i.e.
//!   the bound is entirely off-screen. Rejecting an off-screen instance is the frustum arm's job,
//!   not this one's.
//! * [`KeepReason::LevelUnavailable`] — the selector asked for a level the pyramid does not have.
//!   **KEEP, never clamp DOWN.** A finer level than `L` samples strictly fewer than the rect's
//!   texels, so `occ` becomes a min over a SUBSET of the footprint, which can only be too LARGE —
//!   the one direction that produces a false reject. Clamping down is the natural-looking fix and
//!   it is unsound.
//! * [`KeepReason::NotOccluded`] — the test ran and the instance survived it.
//!
//! ## The selector
//!
//! `L = max(msb(tx0 ^ tx1), msb(ty0 ^ ty1))` over the rect's corner TEXEL indices, with
//! `msb(0) := 0`. If `tx0 ^ tx1` has its highest set bit at `L` then `tx0` and `tx1` agree above
//! bit `L` and differ at it, so (with `tx0 ≤ tx1`) `tx1 >> L == (tx0 >> L) + 1`: at most two
//! distinct texels per axis, hence at most four samples. Their combined preimage spans
//! `[(tx0 >> L) << L, ((tx1 >> L) + 1) << L)` in level-0 texels, which contains `[tx0, tx1]`,
//! which contains the rect — that is COVERAGE, and the whole soundness theorem rests on it.
//!
//! ## The verdict
//!
//! Reject iff `depth_near < occ`, STRICTLY. Equality keeps: the soundness proof yields
//! `occ ≤ D[p] ≤ d_i(p) ≤ depth_near` for a visible instance, so `depth_near == occ` is a
//! legitimate visible case and a `<=` here would delete it.

use core::fmt;

/// The largest source extent, per axis, [`HzbLayout`] accepts.
///
/// `65536` exceeds `maxImageDimension2D` on every current Vulkan implementation, so the cap can
/// never bind on a real render target. It exists to make [`MAX_HZB_LEVELS`] a compile-time
/// constant and to keep every `coord * base` product in this module far inside `u64`.
pub const MAX_HZB_EXTENT: u32 = 1 << 16;

/// The level count of the largest accepted layout: `msb(65536) + 1 = 17`.
pub const MAX_HZB_LEVELS: u32 = 17;

const _: () = assert!(MAX_HZB_LEVELS == msb(MAX_HZB_EXTENT) + 1);
const _: () = assert!(MAX_HZB_EXTENT.is_power_of_two());

/// Why [`HzbLayout`] construction was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HzbLayoutError {
    /// A source axis was `0`. A zero extent has no `prev_pow2` and no pyramid.
    ZeroExtent,
    /// A source axis exceeded [`MAX_HZB_EXTENT`].
    ExtentTooLarge,
    /// [`HzbLayout::truncated`] was asked for `0` levels. Level 0 always exists.
    ZeroLevels,
    /// [`HzbLayout::truncated`] was asked for more levels than the base extent supports. Levels
    /// past the `1 × 1` top would all be copies of it, and admitting them would let the
    /// `L <= levels - 1` guard pass for a level with no storage behind it.
    TooManyLevels,
}

impl fmt::Display for HzbLayoutError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HzbLayoutError::ZeroExtent => write!(f, "HZB source extent has a zero axis"),
            HzbLayoutError::ExtentTooLarge => {
                write!(f, "HZB source extent exceeds MAX_HZB_EXTENT ({MAX_HZB_EXTENT})")
            }
            HzbLayoutError::ZeroLevels => write!(f, "HZB level count must be at least 1"),
            HzbLayoutError::TooManyLevels => {
                write!(f, "HZB level count exceeds what the base extent supports")
            }
        }
    }
}

impl std::error::Error for HzbLayoutError {}

/// One axis of an HZB: the source extent and the level-0 (power-of-two) extent derived from it.
///
/// Split per axis because every map in the pyramid half is one-dimensional and because a
/// non-square base (`1024 × 64`) makes the two axes bottom out at different levels. A shared
/// `[u32; 2]` implementation would have to re-derive that split at every call site.
///
/// Both fields are READ-ONLY from outside this module, and deliberately so: every derivation in
/// the module header assumes `0 < base <= source` with `base` a power of two. Writable fields
/// would let a caller set `base = 8` on a 7-pixel source, and texel 7's preimage would then be
/// EMPTY — an empty `min` stays `+∞`, which rejects EVERYTHING, the one direction that deletes
/// geometry. A zero is worse still: `source = 0` divides by zero in [`HzbAxis::texel_of`] and
/// `base = 0` in [`HzbAxis::first_source`]. Only [`HzbLayout::new`] mints an axis, so the
/// invariant holds by construction rather than by convention.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HzbAxis {
    source: u32,
    base: u32,
}

impl HzbAxis {
    /// Source extent in pixels — the framebuffer axis this pyramid reduces.
    #[inline]
    #[must_use]
    pub const fn source(&self) -> u32 {
        self.source
    }

    /// Level-0 extent in texels: `prev_pow2(source)`, hence `0 < base <= source`.
    #[inline]
    #[must_use]
    pub const fn base(&self) -> u32 {
        self.base
    }

    /// The level-`level` extent: `max(1, base >> level)`.
    ///
    /// The clamp is what keeps a short axis alive while the long one keeps halving; at that point
    /// the 2×2 reduce degenerates to a copy on this axis.
    #[inline]
    #[must_use]
    pub const fn level_extent(&self, level: u32) -> u32 {
        // `level >= 32` cannot arise from a level INDEX inside a valid `HzbLayout`
        // (levels <= MAX_HZB_LEVELS = 17), but this method is public and takes any `u32`, and a
        // 32-bit shift overflow is a panic, not a `0`.
        if level >= 32 {
            return 1;
        }
        let e = self.base >> level;
        if e == 0 { 1 } else { e }
    }

    /// The level-0 texel a source pixel belongs to: `(x * base) / source`.
    ///
    /// See the module header for why this floor form and [`Self::first_source`]'s ceiling form are
    /// two halves of one derivation.
    ///
    /// # Panics
    ///
    /// Debug-only: `x < source`.
    #[inline]
    #[must_use]
    pub const fn texel_of(&self, x: u32) -> u32 {
        debug_assert!(x < self.source, "invariant: source pixel is inside the axis");
        // `u64` because `x * base` reaches `MAX_HZB_EXTENT^2 = 2^32` at the cap.
        ((x as u64 * self.base as u64) / self.source as u64) as u32
    }

    /// The FIRST source pixel of level-0 texel `t`: `⌈t·source/base⌉`, i.e. the
    /// `(t * source + base - 1) / base` of the module header, spelled with `div_ceil` so the
    /// rounding direction is in the call rather than in an idiom a reader has to recognise.
    ///
    /// Defined for `t` in `[0, base]` inclusive, so `first_source(t + 1)` is the exclusive end of
    /// texel `t`'s preimage and `first_source(base) == source`.
    ///
    /// # Panics
    ///
    /// Debug-only: `t <= base`.
    #[inline]
    #[must_use]
    pub fn first_source(&self, t: u32) -> u32 {
        debug_assert!(t <= self.base, "invariant: texel index is inside the level-0 extent");
        // `u64` because `t * source` reaches `MAX_HZB_EXTENT^2 = 2^32` at the cap.
        ((t as u64 * self.source as u64).div_ceil(self.base as u64)) as u32
    }

    /// The level-`level` texel containing level-0 texel `t`: `t >> level`.
    ///
    /// Exact because `base` is a power of two: for `t < base = 2^b` and `level <= b`,
    /// `t >> level <= 2^(b-level) - 1 < base >> level`; for `level > b` the level extent is the
    /// clamped `1` and `t >> level == 0`.
    #[inline]
    #[must_use]
    pub const fn containing_texel(&self, t: u32, level: u32) -> u32 {
        if level >= 32 { 0 } else { t >> level }
    }

    /// The half-open SOURCE-PIXEL span `[lo, hi)` covered by level-`level` texel `t`.
    ///
    /// This is the composition the COVERAGE property is stated against: the level-`level` texel
    /// covers level-0 texels `[t << level, min(base, (t+1) << level))`, and each of those covers
    /// its own `first_source` interval.
    ///
    /// ⚠️ It is NOT independent of [`build_pyramid`]. At `level == 0` this returns
    /// `(first_source(t), first_source(t + 1))` — the two calls the builder's own level-0 loop
    /// makes — and above that it is their union. Stated against COVERAGE (whose other half is
    /// [`select_texels`], which never calls `first_source`) the independence is real; stated
    /// against the pyramid it is not, and the soundness property's doc says so plainly.
    ///
    /// # Panics
    ///
    /// If `level >= MAX_HZB_LEVELS`, or (debug-only) if `t` is outside the level extent.
    #[must_use]
    pub fn level_source_span(&self, level: u32, t: u32) -> (u32, u32) {
        assert!(level < MAX_HZB_LEVELS, "invariant: level is inside MAX_HZB_LEVELS");
        debug_assert!(t < self.level_extent(level), "invariant: texel is inside the level extent");
        let lo_texel = (t << level).min(self.base);
        let hi_texel = ((t + 1) << level).min(self.base);
        (self.first_source(lo_texel), self.first_source(hi_texel))
    }
}

/// The full HZB shape: both axes plus the level count.
///
/// Constructed through [`HzbLayout::new`] (the complete chain down to `1 × 1`) or
/// [`HzbLayout::truncated`] (a partial chain). Both enforce `levels >= 1`, which every method
/// here relies on.
///
/// Every field is READ-ONLY from outside this module, for one reason: `levels` was private to keep
/// [`HzbLayout::level_offset`] and [`HzbLayout::texel`] in range, and the axes are what `levels` is
/// DERIVED from — leaving them writable would have guarded the conclusion while leaving the
/// premise open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HzbLayout {
    x: HzbAxis,
    y: HzbAxis,
    levels: u32,
}

impl HzbLayout {
    /// The COMPLETE pyramid for a `width × height` source: level 0 is
    /// `prev_pow2(width) × prev_pow2(height)` and the chain runs down to `1 × 1`.
    ///
    /// `levels = msb(max(base_x, base_y)) + 1`, which is the first `k` at which BOTH axes have
    /// clamped to `1`.
    ///
    /// # Errors
    ///
    /// [`HzbLayoutError::ZeroExtent`] or [`HzbLayoutError::ExtentTooLarge`].
    pub const fn new(width: u32, height: u32) -> Result<Self, HzbLayoutError> {
        if width == 0 || height == 0 {
            return Err(HzbLayoutError::ZeroExtent);
        }
        if width > MAX_HZB_EXTENT || height > MAX_HZB_EXTENT {
            return Err(HzbLayoutError::ExtentTooLarge);
        }
        let bx = prev_pow2(width);
        let by = prev_pow2(height);
        let longest = if bx > by { bx } else { by };
        Ok(Self {
            x: HzbAxis { source: width, base: bx },
            y: HzbAxis { source: height, base: by },
            levels: msb(longest) + 1,
        })
    }

    /// A PARTIAL pyramid: the same level-0 base, but the chain stops after `levels` levels.
    ///
    /// Stopping early is a real implementation choice — the top mips of a full chain are a
    /// handful of texels each and cost a dispatch apiece. It is safe here only because the
    /// selector refuses a level it does not have ([`KeepReason::LevelUnavailable`]) instead of
    /// clamping down to the coarsest one it does; see the module header for why clamping down is
    /// the unsound direction.
    ///
    /// # Errors
    ///
    /// [`HzbLayoutError::ZeroLevels`] / [`HzbLayoutError::TooManyLevels`], plus everything
    /// [`HzbLayout::new`] can return.
    pub const fn truncated(width: u32, height: u32, levels: u32) -> Result<Self, HzbLayoutError> {
        let full = match Self::new(width, height) {
            Ok(l) => l,
            Err(e) => return Err(e),
        };
        if levels == 0 {
            return Err(HzbLayoutError::ZeroLevels);
        }
        if levels > full.levels {
            return Err(HzbLayoutError::TooManyLevels);
        }
        Ok(Self { x: full.x, y: full.y, levels })
    }

    /// The horizontal axis.
    #[inline]
    #[must_use]
    pub const fn x(&self) -> HzbAxis {
        self.x
    }

    /// The vertical axis.
    #[inline]
    #[must_use]
    pub const fn y(&self) -> HzbAxis {
        self.y
    }

    /// The number of levels in this pyramid; always at least `1`.
    #[inline]
    #[must_use]
    pub const fn levels(&self) -> u32 {
        self.levels
    }

    /// The `[width, height]` texel extent of one level.
    #[inline]
    #[must_use]
    pub const fn level_extent(&self, level: u32) -> [u32; 2] {
        [self.x.level_extent(level), self.y.level_extent(level)]
    }

    /// The flat offset of a level's first texel inside a [`Self::pyramid_len`]-sized buffer.
    ///
    /// Levels are stored back to back, finest first, each row-major. `level == levels()` yields
    /// the total length, which is what makes [`Self::pyramid_len`] a call to this.
    ///
    /// # Panics
    ///
    /// If `level > levels()`.
    #[must_use]
    pub const fn level_offset(&self, level: u32) -> usize {
        assert!(level <= self.levels, "invariant: level is inside the pyramid");
        let mut off = 0usize;
        let mut k = 0u32;
        while k < level {
            off += self.x.level_extent(k) as usize * self.y.level_extent(k) as usize;
            k += 1;
        }
        off
    }

    /// The total `f32` count [`build_pyramid`] writes.
    #[inline]
    #[must_use]
    pub const fn pyramid_len(&self) -> usize {
        self.level_offset(self.levels)
    }

    /// The number of source depth samples this layout reduces: `x.source * y.source`.
    #[inline]
    #[must_use]
    pub const fn source_len(&self) -> usize {
        self.x.source as usize * self.y.source as usize
    }

    /// Reads one texel out of a built pyramid.
    ///
    /// # Panics
    ///
    /// If `level`, `tx` or `ty` is outside the pyramid, or `pyramid` is too short. These are hard
    /// assertions rather than `debug_assert!`s because an out-of-range texel index would
    /// otherwise silently read a NEIGHBOURING row or level in release — a wrong `occ`, no crash,
    /// and the failure direction is "reject something visible".
    #[must_use]
    pub fn texel(&self, pyramid: &[f32], level: u32, tx: u32, ty: u32) -> f32 {
        assert!(level < self.levels, "invariant: level is inside the pyramid");
        let [w, h] = self.level_extent(level);
        assert!(tx < w && ty < h, "invariant: texel is inside its level extent");
        pyramid[self.level_offset(level) + (ty as usize * w as usize + tx as usize)]
    }
}

/// The largest power of two `<= v`, and `0` for `v == 0`.
///
/// `0` is deliberately not `1`: a zero extent has no pyramid, and [`HzbLayout::new`] refuses it
/// rather than inventing one.
#[inline]
#[must_use]
pub const fn prev_pow2(v: u32) -> u32 {
    if v == 0 { 0 } else { 1u32 << msb(v) }
}

/// The index of the highest set bit, with `msb(0) := 0`.
///
/// The `0` case is not a fallback: in the level selector `tx0 ^ tx1 == 0` means both rect corners
/// land in the same texel, and level `0` is exactly the right answer for that axis.
const fn msb(v: u32) -> u32 {
    if v == 0 { 0 } else { u32::BITS - 1 - v.leading_zeros() }
}

/// The reverse-Z conservative reduce: `min`, with NaN collapsing to `f32::NEG_INFINITY`.
///
/// See the module header for both halves: why `min` (it is the FARTHEST surface under reverse-Z,
/// and the reject predicate needs a lower bound) and why NaN becomes `-∞` (unknown depth must
/// never be able to reject).
#[inline]
#[must_use]
pub fn conservative_min(a: f32, b: f32) -> f32 {
    if a.is_nan() || b.is_nan() {
        f32::NEG_INFINITY
    } else if b < a {
        b
    } else {
        a
    }
}

/// Builds the whole min chain from a row-major source depth buffer.
///
/// Level 0 reduces each texel's source PREIMAGE (the `first_source` interval on each axis);
/// every later level reduces its 2×2 footprint in the level below, clamped to that level's
/// extent so a bottomed-out axis copies instead of reading out of range.
///
/// # Panics
///
/// If `depth.len() != layout.source_len()` or `pyramid.len() != layout.pyramid_len()`. Both are
/// caller invariants: a short buffer here would silently reduce over the wrong pixels.
pub fn build_pyramid(layout: &HzbLayout, depth: &[f32], pyramid: &mut [f32]) {
    assert_eq!(
        depth.len(),
        layout.source_len(),
        "invariant: the depth buffer matches the source extent"
    );
    assert_eq!(
        pyramid.len(),
        layout.pyramid_len(),
        "invariant: the pyramid storage matches the layout"
    );

    let (ax, ay) = (layout.x(), layout.y());
    let source_w = ax.source() as usize;
    let [base_w, base_h] = layout.level_extent(0);

    for ty in 0..base_h {
        let (y_lo, y_hi) = (ay.first_source(ty), ay.first_source(ty + 1));
        for tx in 0..base_w {
            let (x_lo, x_hi) = (ax.first_source(tx), ax.first_source(tx + 1));
            let mut m = f32::INFINITY;
            for y in y_lo..y_hi {
                let row = y as usize * source_w;
                for x in x_lo..x_hi {
                    m = conservative_min(m, depth[row + x as usize]);
                }
            }
            pyramid[ty as usize * base_w as usize + tx as usize] = m;
        }
    }

    for level in 1..layout.levels() {
        let [fine_w, fine_h] = layout.level_extent(level - 1);
        let [coarse_w, coarse_h] = layout.level_extent(level);
        let fine_off = layout.level_offset(level - 1);
        let coarse_off = layout.level_offset(level);
        for ty in 0..coarse_h {
            for tx in 0..coarse_w {
                let mut m = f32::INFINITY;
                for dy in 0..2u32 {
                    let sy = ty * 2 + dy;
                    if sy >= fine_h {
                        continue;
                    }
                    for dx in 0..2u32 {
                        let sx = tx * 2 + dx;
                        if sx >= fine_w {
                            continue;
                        }
                        let v = pyramid[fine_off + (sy as usize * fine_w as usize + sx as usize)];
                        m = conservative_min(m, v);
                    }
                }
                pyramid[coarse_off + (ty as usize * coarse_w as usize + tx as usize)] = m;
            }
        }
    }
}

/// Why an instance was KEPT.
///
/// Every variant is a decision NOT to remove geometry. They are separate cases rather than one
/// boolean so a gate can assert WHICH guard fired — a test that only checks "kept" cannot tell a
/// working `w > 0` guard from a projection that never ran.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeepReason {
    /// The AABB was inverted on some axis: the "bounds unknown" sentinel. Absence of bounds is
    /// not evidence of invisibility.
    UnknownBounds,
    /// Some corner projected to `clip.w <= 0` — at or behind the eye plane.
    BehindEye,
    /// Some clip or window coordinate was not finite.
    NonFinite,
    /// The pixel rect is empty after clamping to the framebuffer (the bound is off-screen).
    EmptyRect,
    /// The selector asked for a level this pyramid does not have. Never produced by
    /// [`project_aabb`].
    LevelUnavailable,
    /// The test ran to completion and `depth_near >= occ`. Never produced by [`project_aabb`] or
    /// [`select_texels`].
    NotOccluded,
}

/// The verdict of [`occlusion_verdict`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OcclusionVerdict {
    /// Draw it. The reason is carried so a gate can distinguish the guards.
    Keep(KeepReason),
    /// Provably behind the depth already in the buffer over its whole screen rect.
    Reject,
}

impl OcclusionVerdict {
    /// `true` for every [`OcclusionVerdict::Keep`], whatever the reason.
    #[inline]
    #[must_use]
    pub const fn is_keep(&self) -> bool {
        matches!(self, OcclusionVerdict::Keep(_))
    }
}

/// An AABB's screen footprint: an INCLUSIVE pixel rect already clamped to the framebuffer, plus
/// the bound's nearest depth.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScreenRect {
    /// Inclusive minimum pixel `[x, y]`.
    pub min: [u32; 2],
    /// Inclusive maximum pixel `[x, y]`; always `>= min` on both axes.
    pub max: [u32; 2],
    /// The largest `z_ndc` over the eight corners — under reverse-Z, the NEAREST depth the bound
    /// can have.
    pub depth_near: f32,
}

/// The ≤4 texels the test samples, and the level they live on.
///
/// `tx`/`ty` may repeat (a rect inside one texel selects the same texel four times); `min` is
/// idempotent, so the duplicates cost nothing and removing them would only add a branch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TexelSelection {
    /// The pyramid level, always `< HzbLayout::levels()`.
    pub level: u32,
    /// The two horizontal texel indices, `tx[0] <= tx[1]`.
    pub tx: [u32; 2],
    /// The two vertical texel indices, `ty[0] <= ty[1]`.
    pub ty: [u32; 2],
}

/// Projects a world AABB through `view_proj` into a clamped pixel rect plus `depth_near`.
///
/// `view_proj` is `pv[row][col]` in MATH-ROW form, `clip = pv · world` — the same convention
/// [`crate::frustum::frustum_planes_from_view_proj`] takes.
///
/// This is the single shared entry point for the projection, which is why the unknown-bounds
/// sentinel is tested HERE and before anything else: every caller inherits the short-circuit
/// structurally rather than by remembering to repeat it.
///
/// # ⚠️ The raster's push is NOT this layout — the caller must transpose
///
/// The push stores the matrix COLUMN-major (HLSL's `float4x4` storage), i.e. element index
/// `col * 4 + row`; this function takes math ROWS. Handing the 16 pushed floats over as rows is a
/// TRANSPOSE, and a transposed matrix still projects — to a systematically wrong rect on every
/// instance, with every guard below silent. The frustum arm met the same fork and closed it with
/// [`crate::frustum::frustum_planes_from_push_bytes`], which decodes the 64 push bytes back into
/// math rows; an S4/S5 caller whose matrix comes from the push MUST route through that same
/// inversion (byte provenance) rather than through a separately-computed `pv`.
///
/// The reason byte provenance is not pedantry here: something already perturbs the projection per
/// frame. [`crate::view::forward_view_proj_rows_jittered`] folds the TAA jitter into rows 0 and 1
/// and leaves rows 2 and 3 byte-UNTOUCHED. `depth_near` reads only rows 2 and 3, so it is
/// invariant under the jitter — the pixel rect is not, and it is the half that must match the
/// matrix the raster actually drew with.
///
/// # Errors
///
/// [`KeepReason::UnknownBounds`], [`KeepReason::BehindEye`], [`KeepReason::NonFinite`] or
/// [`KeepReason::EmptyRect`] — each a KEEP, see the module header.
///
/// # Panics
///
/// Debug-only: both `source` axes are non-zero. `source[i] - 1` is the last pixel index, so a
/// zero axis underflows — a panic in debug, a wrap to `u32::MAX` (and a rect covering the whole
/// address space) in release. Every [`HzbLayout`] refuses a zero extent
/// ([`HzbLayoutError::ZeroExtent`]), but this entry point can be reached without one.
pub fn project_aabb(
    view_proj: &[[f32; 4]; 4],
    source: [u32; 2],
    aabb_min: [f32; 3],
    aabb_max: [f32; 3],
) -> Result<ScreenRect, KeepReason> {
    debug_assert!(
        source[0] > 0 && source[1] > 0,
        "invariant: the framebuffer extent is non-zero on both axes"
    );

    // FIRST, before any arithmetic on the corners: an inverted box is the sentinel, and a
    // degenerate `view_proj` would fold it into a perfectly plausible small rect.
    //
    // Spelled as `!(min <= max)` rather than `min > max` so that a NaN coordinate — which is
    // neither `<=` nor `>` — also counts as "not a box" here, at the earliest possible point,
    // instead of travelling into the projection to be caught by a later guard.
    let ordered = aabb_min[0] <= aabb_max[0]
        && aabb_min[1] <= aabb_max[1]
        && aabb_min[2] <= aabb_max[2];
    if !ordered {
        return Err(KeepReason::UnknownBounds);
    }

    let half_w = source[0] as f32 * 0.5;
    let half_h = source[1] as f32 * 0.5;

    let mut x_lo = f32::INFINITY;
    let mut x_hi = f32::NEG_INFINITY;
    let mut y_lo = f32::INFINITY;
    let mut y_hi = f32::NEG_INFINITY;
    let mut depth_near = f32::NEG_INFINITY;

    for corner in 0..8u32 {
        let p = [
            if corner & 1 == 0 { aabb_min[0] } else { aabb_max[0] },
            if corner & 2 == 0 { aabb_min[1] } else { aabb_max[1] },
            if corner & 4 == 0 { aabb_min[2] } else { aabb_max[2] },
        ];
        let dot = |r: [f32; 4]| r[0] * p[0] + r[1] * p[1] + r[2] * p[2] + r[3];
        let (cx, cy, cz, cw) =
            (dot(view_proj[0]), dot(view_proj[1]), dot(view_proj[2]), dot(view_proj[3]));

        let clip_finite =
            cx.is_finite() && cy.is_finite() && cz.is_finite() && cw.is_finite();
        if !clip_finite {
            return Err(KeepReason::NonFinite);
        }
        if cw <= 0.0 {
            return Err(KeepReason::BehindEye);
        }

        let inv_w = 1.0 / cw;
        let z_ndc = cz * inv_w;
        let x_win = (cx * inv_w + 1.0) * half_w;
        // POSITIVE viewport height, no flip — see the module header. `+Y` NDC is `+Y` window.
        let y_win = (cy * inv_w + 1.0) * half_h;
        // Repeated AFTER the divide: a finite `clip` over a tiny `w` still overflows to infinity.
        let ndc_finite = x_win.is_finite() && y_win.is_finite() && z_ndc.is_finite();
        if !ndc_finite {
            return Err(KeepReason::NonFinite);
        }

        x_lo = x_lo.min(x_win);
        x_hi = x_hi.max(x_win);
        y_lo = y_lo.min(y_win);
        y_hi = y_hi.max(y_win);
        depth_near = depth_near.max(z_ndc);
    }

    // A pixel `i` covers `[i, i+1)`, so the pixels a span touches are `floor(lo) ..= floor(hi)`.
    // `floor` on the upper end rather than `ceil - 1` deliberately includes the pixel a span
    // ending exactly on a boundary merely touches: one extra column widens the footprint, which
    // is the KEEP direction. The two forms differ ONLY on an exactly-integer edge, so the claim
    // is executed by a fixture built to land on one (`anchor_pixel_rect_at_an_exact_integer_edge`)
    // rather than left to the reader — `ceil(6.0) - 1 = 5` drops a column the bound covers, and a
    // footprint missing the column where the instance is visible is a FALSE REJECT.
    let x_last = (source[0] - 1) as f32;
    let y_last = (source[1] - 1) as f32;
    let px0 = x_lo.floor().max(0.0);
    let px1 = x_hi.floor().min(x_last);
    let py0 = y_lo.floor().max(0.0);
    let py1 = y_hi.floor().min(y_last);
    // Covers both "entirely off-screen" (the clamp crosses the bounds over) and any inversion.
    if px1 < px0 || py1 < py0 {
        return Err(KeepReason::EmptyRect);
    }

    Ok(ScreenRect {
        min: [px0 as u32, py0 as u32],
        max: [px1 as u32, py1 as u32],
        depth_near,
    })
}

/// Picks the coarsest level at which the rect spans at most two texels per axis, and the ≤4 texel
/// indices on it.
///
/// # Errors
///
/// [`KeepReason::LevelUnavailable`] when the required level is past the top of this pyramid. The
/// escape is KEEP and never a clamp to `levels() - 1` — a finer level samples a strict subset of
/// the rect's footprint, so `occ` could only come out too large and reject a visible instance.
pub fn select_texels(layout: &HzbLayout, rect: &ScreenRect) -> Result<TexelSelection, KeepReason> {
    let (ax, ay) = (layout.x(), layout.y());
    let tx0 = ax.texel_of(rect.min[0]);
    let tx1 = ax.texel_of(rect.max[0]);
    let ty0 = ay.texel_of(rect.min[1]);
    let ty1 = ay.texel_of(rect.max[1]);

    let level = msb(tx0 ^ tx1).max(msb(ty0 ^ ty1));
    // `level >= levels` is the `L <= levels - 1` guard written without a subtraction.
    if level >= layout.levels() {
        return Err(KeepReason::LevelUnavailable);
    }

    Ok(TexelSelection {
        level,
        tx: [ax.containing_texel(tx0, level), ax.containing_texel(tx1, level)],
        ty: [ay.containing_texel(ty0, level), ay.containing_texel(ty1, level)],
    })
}

/// `occ`: the min over the selected texels — the depth of the farthest already-rasterised surface
/// anywhere in the rect's covered footprint.
///
/// # Panics
///
/// If the selection is outside the pyramid (see [`HzbLayout::texel`]).
#[must_use]
pub fn occluder_depth(layout: &HzbLayout, pyramid: &[f32], selection: &TexelSelection) -> f32 {
    let mut occ = f32::INFINITY;
    for &ty in &selection.ty {
        for &tx in &selection.tx {
            occ = conservative_min(occ, layout.texel(pyramid, selection.level, tx, ty));
        }
    }
    occ
}

/// THE ORACLE: the late-pass verdict for one instance's world AABB.
///
/// Sentinel → projection guards → level selection → `depth_near < occ`. Every step that cannot
/// answer confidently returns a named [`KeepReason`]; only the final strict comparison can
/// produce [`OcclusionVerdict::Reject`].
///
/// # Panics
///
/// If `pyramid` does not match `layout` (see [`HzbLayout::texel`]).
#[must_use]
pub fn occlusion_verdict(
    layout: &HzbLayout,
    pyramid: &[f32],
    view_proj: &[[f32; 4]; 4],
    aabb_min: [f32; 3],
    aabb_max: [f32; 3],
) -> OcclusionVerdict {
    let source = [layout.x().source(), layout.y().source()];
    let rect = match project_aabb(view_proj, source, aabb_min, aabb_max) {
        Ok(r) => r,
        Err(reason) => return OcclusionVerdict::Keep(reason),
    };
    let selection = match select_texels(layout, &rect) {
        Ok(s) => s,
        Err(reason) => return OcclusionVerdict::Keep(reason),
    };
    let occ = occluder_depth(layout, pyramid, &selection);
    // STRICT `<`. Equality is a legitimate visible case — see the module header.
    if rect.depth_near < occ {
        OcclusionVerdict::Reject
    } else {
        OcclusionVerdict::Keep(KeepReason::NotOccluded)
    }
}

#[cfg(test)]
mod tests {
    use boyko_math::Vec4;
    use boyko_scene::ViewUniform;

    use crate::view::forward_view_proj_rows;

    use super::*;

    // ==========================================================================================
    // PART 1 — THE HAND-COMPUTED ANCHOR
    //
    // Property tests check INVARIANTS, not VALUES: an oracle that is systematically wrong in a
    // self-consistent way satisfies every one of them. A base map off by one in BOTH directions
    // still partitions; a pyramid reduced over the wrong footprint still satisfies soundness
    // against its own preimage definition; a selector one level too coarse still covers.
    //
    // So the properties hang from these point values, every one of which is worked out by hand in
    // the doc comments below and can be re-checked with a pencil.
    // ==========================================================================================

    /// The anchor source extent: **7 × 3**. Odd on both axes, and `prev_pow2` bites on both.
    const ANCHOR_W: u32 = 7;
    const ANCHOR_H: u32 = 3;

    /// The SECOND anchor extent, **8 × 16**, used by exactly one fixture: the one that must land a
    /// projected window coordinate on an EXACT integer. `7 × 3` cannot — `half_w = 3.5` would need
    /// `x_ndc + 1` to be a multiple of `2/7`, which is not dyadic — so every `7 × 3` box lands
    /// strictly inside a pixel and is blind to the upper-edge rounding. The two half-extents here
    /// are `4` and `8`: powers of two, and different from each other so the two axes cannot agree
    /// by accident.
    const EDGE_W: u32 = 8;
    const EDGE_H: u32 = 16;

    /// The anchor depth buffer, row-major, `7 × 3`, reverse-Z (larger = nearer).
    ///
    /// Chosen so the per-texel minima are all distinct where it matters and the global minimum
    /// (`0.05`) sits in the interior rather than at a corner.
    ///
    /// ```text
    ///   y=0 | 0.90 0.80 0.70 0.60 0.50 0.40 0.30
    ///   y=1 | 0.85 0.75 0.65 0.55 0.45 0.35 0.25
    ///   y=2 | 0.10 0.20 0.95 0.05 0.65 0.15 0.55
    /// ```
    const ANCHOR_DEPTH: [f32; 21] = [
        0.90, 0.80, 0.70, 0.60, 0.50, 0.40, 0.30, //
        0.85, 0.75, 0.65, 0.55, 0.45, 0.35, 0.25, //
        0.10, 0.20, 0.95, 0.05, 0.65, 0.15, 0.55,
    ];

    fn anchor_layout() -> HzbLayout {
        HzbLayout::new(ANCHOR_W, ANCHOR_H).expect("invariant: 7x3 is a legal HZB extent")
    }

    fn anchor_pyramid() -> (HzbLayout, Vec<f32>) {
        let layout = anchor_layout();
        let mut pyramid = vec![0.0f32; layout.pyramid_len()];
        build_pyramid(&layout, &ANCHOR_DEPTH, &mut pyramid);
        (layout, pyramid)
    }

    /// THE LEVEL EXTENTS, BY HAND.
    ///
    /// `prev_pow2(7) = 4` (`4 <= 7 < 8`), `prev_pow2(3) = 2` (`2 <= 3 < 4`).
    /// `levels = msb(max(4, 2)) + 1 = msb(4) + 1 = 2 + 1 = 3`.
    ///
    /// | level | `max(1, 4 >> k)` | `max(1, 2 >> k)` | texels |
    /// |---|---|---|---|
    /// | 0 | 4 | 2 | 8 |
    /// | 1 | 2 | 1 | 2 |
    /// | 2 | 1 | 1 | 1 |
    ///
    /// Total storage `8 + 2 + 1 = 11`; offsets `0`, `8`, `10`.
    ///
    /// Level 2's Y extent is `max(1, 2 >> 2) = max(1, 0) = 1` — the clamp, exercised.
    #[test]
    fn anchor_level_extents_and_offsets() {
        let l = anchor_layout();
        assert_eq!(l.x().base(), 4);
        assert_eq!(l.y().base(), 2);
        assert_eq!(l.levels(), 3);
        assert_eq!(l.level_extent(0), [4, 2]);
        assert_eq!(l.level_extent(1), [2, 1]);
        assert_eq!(l.level_extent(2), [1, 1]);
        assert_eq!(l.level_offset(0), 0);
        assert_eq!(l.level_offset(1), 8);
        assert_eq!(l.level_offset(2), 10);
        assert_eq!(l.pyramid_len(), 11);
    }

    /// THE BASE MAP, FOR EVERY PIXEL, BY HAND.
    ///
    /// X axis, `S = 7`, `P = 4`, forward `t = (x * 4) / 7`:
    ///
    /// | x | 0 | 1 | 2 | 3 | 4 | 5 | 6 |
    /// |---|---|---|---|---|---|---|---|
    /// | `x*4` | 0 | 4 | 8 | 12 | 16 | 20 | 24 |
    /// | `/7` | 0 | 0 | 1 | 1 | 2 | 2 | 3 |
    ///
    /// Inverse `first(t) = (t*7 + 3) / 4`: `3/4=0`, `10/4=2`, `17/4=4`, `24/4=6`, `31/4=7`.
    /// So the preimages are `{0,1} {2,3} {4,5} {6}` — 2+2+2+1 = 7 pixels, disjoint, covering
    /// `[0,7)`. A PARTITION, and `first(4) = 7 = S`.
    ///
    /// Y axis, `S = 3`, `P = 2`, forward `t = (y * 2) / 3`: `0/3=0`, `2/3=0`, `4/3=1`.
    /// Inverse `first(t) = (t*3 + 1) / 2`: `1/2=0`, `4/2=2`, `7/2=3`.
    /// Preimages `{0,1} {2}` — 2+1 = 3 pixels, and `first(2) = 3 = S`.
    #[test]
    fn anchor_base_map_every_pixel() {
        let l = anchor_layout();
        let x_expect = [0u32, 0, 1, 1, 2, 2, 3];
        for (x, &t) in x_expect.iter().enumerate() {
            assert_eq!(l.x().texel_of(x as u32), t, "x = {x}");
        }
        // Compared against a slice literal rather than a `vec![…]` operand: the expectation is a
        // fixed list of five values, and `clippy::useless_vec` (the default-on `perf` group) is
        // right that allocating one to compare against is pointless.
        let x_first: Vec<u32> = (0..=4).map(|t| l.x().first_source(t)).collect();
        assert_eq!(x_first.as_slice(), &[0u32, 2, 4, 6, 7]);

        let y_expect = [0u32, 0, 1];
        for (y, &t) in y_expect.iter().enumerate() {
            assert_eq!(l.y().texel_of(y as u32), t, "y = {y}");
        }
        let y_first: Vec<u32> = (0..=2).map(|t| l.y().first_source(t)).collect();
        assert_eq!(y_first.as_slice(), &[0u32, 2, 3]);
    }

    /// THE FULL MIN CHAIN, BY HAND.
    ///
    /// Level 0 is `4 × 2`. Texel `(tx, ty)` reduces the pixels `x ∈ preimage(tx)`,
    /// `y ∈ preimage(ty)` — X preimages `{0,1} {2,3} {4,5} {6}`, Y preimages `{0,1} {2}`.
    ///
    /// | texel | pixels | values | min |
    /// |---|---|---|---|
    /// | (0,0) | x{0,1} y{0,1} | 0.90 0.80 0.85 0.75 | **0.75** |
    /// | (1,0) | x{2,3} y{0,1} | 0.70 0.60 0.65 0.55 | **0.55** |
    /// | (2,0) | x{4,5} y{0,1} | 0.50 0.40 0.45 0.35 | **0.35** |
    /// | (3,0) | x{6}   y{0,1} | 0.30 0.25 | **0.25** |
    /// | (0,1) | x{0,1} y{2}   | 0.10 0.20 | **0.10** |
    /// | (1,1) | x{2,3} y{2}   | 0.95 0.05 | **0.05** |
    /// | (2,1) | x{4,5} y{2}   | 0.65 0.15 | **0.15** |
    /// | (3,1) | x{6}   y{2}   | 0.55 | **0.55** |
    ///
    /// Level 1 is `2 × 1`, so each texel takes a full 2×2 of level 0:
    /// * `(0,0)` ← `{(0,0),(1,0),(0,1),(1,1)}` = min(0.75, 0.55, 0.10, 0.05) = **0.05**
    /// * `(1,0)` ← `{(2,0),(3,0),(2,1),(3,1)}` = min(0.35, 0.25, 0.15, 0.55) = **0.15**
    ///
    /// Level 2 is `1 × 1`. Level 1's height is already `1`, so the Y half of the 2×2 footprint
    /// clamps to the single row `{0}` and the reduce is
    /// `(0,0)` ← `{(0,0),(1,0)}` = min(0.05, 0.15) = **0.05**.
    ///
    /// The top of a complete chain must equal the GLOBAL minimum of the source, and `0.05` is it
    /// (every other entry of `ANCHOR_DEPTH` is larger) — an independent check on the whole chain.
    #[test]
    fn anchor_min_chain() {
        let (l, p) = anchor_pyramid();

        let level0 = [0.75f32, 0.55, 0.35, 0.25, 0.10, 0.05, 0.15, 0.55];
        assert_eq!(&p[0..8], &level0, "level 0 disagrees with the hand-computed table");
        assert_eq!(&p[8..10], &[0.05f32, 0.15], "level 1 disagrees with the hand-computed reduce");
        assert_eq!(p[10], 0.05, "level 2 must be the global minimum of the source");

        // Same numbers through the public accessor, so a wrong `level_offset` cannot hide behind
        // the flat-slice reads above.
        assert_eq!(l.texel(&p, 0, 1, 1), 0.05);
        assert_eq!(l.texel(&p, 0, 3, 0), 0.25);
        assert_eq!(l.texel(&p, 1, 1, 0), 0.15);
        assert_eq!(l.texel(&p, 2, 0, 0), 0.05);

        let global_min = ANCHOR_DEPTH.iter().copied().fold(f32::INFINITY, f32::min);
        assert_eq!(
            global_min, 0.05,
            "the fixture's global minimum moved; the table above is stale"
        );
    }

    /// THE SELECTOR, BY HAND, on the same 4×2 base.
    ///
    /// Rect `x ∈ [1, 5]`, `y ∈ [0, 2]`:
    /// `tx0 = texel_of(1) = 0`, `tx1 = texel_of(5) = 2`, `ty0 = texel_of(0) = 0`,
    /// `ty1 = texel_of(2) = 1`. `msb(0 ^ 2) = msb(2) = 1`, `msb(0 ^ 1) = msb(1) = 0`,
    /// so `L = max(1, 0) = 1` (and `1 <= levels - 1 = 2`).
    /// Texels `tx = [0 >> 1, 2 >> 1] = [0, 1]`, `ty = [0 >> 1, 1 >> 1] = [0, 0]`.
    /// `occ = min(H1[0], H1[1]) = min(0.05, 0.15) = 0.05`.
    ///
    /// Rect `x ∈ [3, 3]`, `y ∈ [1, 1]`:
    /// `tx0 = tx1 = texel_of(3) = 1`, `ty0 = ty1 = texel_of(1) = 0`.
    /// `msb(0) = 0` on both axes, so `L = 0` and the single texel is `(1, 0)`:
    /// `occ = H0[(1,0)] = 0.55`.
    #[test]
    fn anchor_selector_and_occluder_depth() {
        let (l, p) = anchor_pyramid();

        let wide = ScreenRect { min: [1, 0], max: [5, 2], depth_near: 0.0 };
        let s = select_texels(&l, &wide).expect("invariant: level 1 exists in a 3-level pyramid");
        assert_eq!(s, TexelSelection { level: 1, tx: [0, 1], ty: [0, 0] });
        assert_eq!(occluder_depth(&l, &p, &s), 0.05);

        let tight = ScreenRect { min: [3, 1], max: [3, 1], depth_near: 0.0 };
        let s = select_texels(&l, &tight).expect("invariant: level 0 always exists");
        assert_eq!(s, TexelSelection { level: 0, tx: [1, 1], ty: [0, 0] });
        assert_eq!(occluder_depth(&l, &p, &s), 0.55);
    }

    /// THE SELECTOR WHEN **Y** DRIVES THE LEVEL, BY HAND.
    ///
    /// The `7 × 3` anchor has `base_y = 2`, so `ty0 ^ ty1` never has a set bit above bit 0 and
    /// `msb(ty0 ^ ty1)` is always `0`: `L` is always X's answer there, and a selector that dropped
    /// the cross-axis `max` and computed `L = msb(tx0 ^ tx1)` alone would satisfy every other
    /// anchor assertion in this file. This case is the anchor TRANSPOSED, `3 × 7`, where Y is the
    /// coarse axis.
    ///
    /// `prev_pow2(3) = 2`, `prev_pow2(7) = 4`, so `levels = msb(4) + 1 = 3` and level 1 is
    /// `max(1, 2>>1) × max(1, 4>>1) = 1 × 2`.
    /// * X: `S = 3`, `P = 2`, `texel_of(x) = (x*2)/3` → `0/3=0`, `2/3=0`, `4/3=1`.
    /// * Y: `S = 7`, `P = 4`, `texel_of(y) = (y*4)/7` → `0, 0, 1, 1, 2, 2, 3`.
    ///
    /// Rect `x ∈ [0, 2]`, `y ∈ [1, 5]`: `tx0 = 0`, `tx1 = 1` → `msb(0 ^ 1) = msb(1) = 0`;
    /// `ty0 = texel_of(1) = 0`, `ty1 = texel_of(5) = 2` → `msb(0 ^ 2) = msb(2) = 1`. So
    /// `L = max(0, 1) = 1`, **taken from Y**, with `tx = [0 >> 1, 1 >> 1] = [0, 0]` and
    /// `ty = [0 >> 1, 2 >> 1] = [0, 1]`.
    ///
    /// The X-only selector answers `L = 0` with `ty = [0, 2]` — two SAMPLED rows that are two
    /// apart on an axis spanning three, so level-0 row `ty = 1` (source pixels `{2, 3}`) is never
    /// read and the footprint has a hole exactly where `occ` has to be a lower bound.
    #[test]
    fn anchor_selector_level_can_come_from_the_y_axis() {
        let l = HzbLayout::new(3, 7).expect("invariant: 3x7 is a legal HZB extent");
        assert_eq!((l.x().base(), l.y().base()), (2, 4));
        assert_eq!(l.levels(), 3);
        assert_eq!(l.level_extent(1), [1, 2]);
        assert_eq!((l.x().texel_of(0), l.x().texel_of(2)), (0, 1));
        assert_eq!((l.y().texel_of(1), l.y().texel_of(5)), (0, 2));

        let rect = ScreenRect { min: [0, 1], max: [2, 5], depth_near: 0.0 };
        let s = select_texels(&l, &rect).expect("invariant: a 3-level pyramid has level 1");
        assert_eq!(
            s,
            TexelSelection { level: 1, tx: [0, 0], ty: [0, 1] },
            "the level must come from the Y axis; `L = msb(tx0 ^ tx1)` alone answers level 0"
        );
    }

    /// The anchor projection: an INFINITE reverse-Z perspective, camera at the origin looking down
    /// `-z`, `fovY = 90°` (`f = 1`), aspect 1, near `n = 0.1`.
    ///
    /// Rows in the math-row form `clip = pv · world` this module documents:
    ///
    /// ```text
    ///   row0 = ( 1,  0,  0,   0)   →  clip.x = x
    ///   row1 = ( 0,  1,  0,   0)   →  clip.y = y
    ///   row2 = ( 0,  0,  0, 0.1)   →  clip.z = n            (constant)
    ///   row3 = ( 0,  0, -1,   0)   →  clip.w = -z           (view distance)
    /// ```
    ///
    /// So `z_ndc = n / (-z)`: `1.0` at the near plane, `→ 0` at infinity. That is reverse-Z, and
    /// it makes `depth_near` hand-computable as `0.1 / (nearest view distance)`.
    fn anchor_view_proj() -> [[f32; 4]; 4] {
        [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 0.0, 0.1],
            [0.0, 0.0, -1.0, 0.0],
        ]
    }

    /// THE VIEWPORT TRANSFORM AND `depth_near`, BY HAND.
    ///
    /// Source `7 × 3`, so `half_w = 3.5`, `half_h = 1.5`, and (POSITIVE viewport height, no flip)
    /// `x_win = (x_ndc + 1) * 3.5`, `y_win = (y_ndc + 1) * 1.5`.
    ///
    /// Box A = `[-1,-1,-4] .. [1,1,-2]`. `clip.w = -z ∈ {4, 2}`, both `> 0`.
    /// * `x_ndc = ±1/4 = ±0.25` and `±1/2 = ±0.5` → span `[-0.5, 0.5]`; same for `y`.
    /// * `x_win = 0.5*3.5 = 1.75` and `1.5*3.5 = 5.25` → pixels `floor` → `[1, 5]`.
    /// * `y_win = 0.5*1.5 = 0.75` and `1.5*1.5 = 2.25` → pixels `floor` → `[0, 2]`.
    /// * `z_ndc = 0.1/4 = 0.025` and `0.1/2 = 0.05` → `depth_near = 0.05`.
    ///
    /// Every one of those products is EXACT in `f32` (`0.1f32 / 2` and `0.1f32 / 4` are exact —
    /// division by a power of two only decrements the exponent — and `3.5`, `1.5`, `0.5`, `1.5`
    /// are all dyadic), which is what lets the boundary verdict below be asserted on `==`.
    ///
    /// Box D = `[-1,-1,-0.5] .. [1,1,-0.25]`, i.e. very close and hugely magnified.
    /// `clip.w ∈ {0.5, 0.25}`; `x_ndc = ±1/0.25 = ±4` and `±1/0.5 = ±2` → span `[-4, 4]`.
    /// * `x_win = (-4+1)*3.5 = -10.5` → `floor = -11` → clamped to `0`;
    ///   `(4+1)*3.5 = 17.5` → `floor = 17` → clamped to `6`.
    /// * `y_win = -4.5 → floor -5 → 0`; `7.5 → floor 7 → clamped to 2`.
    /// * `z_ndc = 0.1/0.5 = 0.2` and `0.1/0.25 = 0.4` → `depth_near = 0.4`.
    #[test]
    fn anchor_projection() {
        let pv = anchor_view_proj();
        let src = [ANCHOR_W, ANCHOR_H];

        let a = project_aabb(&pv, src, [-1.0, -1.0, -4.0], [1.0, 1.0, -2.0])
            .expect("invariant: box A is entirely in front of the eye and on-screen");
        assert_eq!(a.min, [1, 0]);
        assert_eq!(a.max, [5, 2]);
        assert_eq!(a.depth_near, 0.05, "depth_near must be 0.1/2 exactly");

        let d = project_aabb(&pv, src, [-1.0, -1.0, -0.5], [1.0, 1.0, -0.25])
            .expect("invariant: box D is in front of the eye");
        assert_eq!(d.min, [0, 0], "box D overflows the framebuffer and must clamp, not wrap");
        assert_eq!(d.max, [6, 2]);
        assert_eq!(d.depth_near, 0.4, "depth_near must be 0.1/0.25 exactly");
    }

    /// ⚠️ THE UPPER-EDGE ROUNDING, on a window coordinate that is EXACTLY an integer.
    ///
    /// `floor(hi)` and the right-open `ceil(hi) - 1` — the form a shader author reaches for first
    /// — agree everywhere EXCEPT on an exact integer, and not one box in the `7 × 3` anchor lands
    /// on one (`half_w = 3.5` makes that impossible; see [`EDGE_W`]). So the distinction was
    /// stated in a comment and executed by nothing. This fixture executes it.
    ///
    /// Source **8 × 16** → `half_w = 4`, `half_h = 8`. Box `[-1,-1,-4] .. [1,1,-2]` under
    /// `anchor_view_proj`, so `clip.w = -z ∈ {4, 2}`:
    /// * near face (`w = 2`): `x_ndc = ±0.5` → `x_win = 0.5 * 4 = 2.0` and `1.5 * 4 = 6.0`;
    ///   `y_ndc = ±0.5` → `y_win = 0.5 * 8 = 4.0` and `1.5 * 8 = 12.0`.
    /// * far face (`w = 4`): `±0.25` → `x_win ∈ {3.0, 5.0}`, `y_win ∈ {6.0, 10.0}` — strictly
    ///   inside the span above, so the near face supplies both extremes.
    ///
    /// Every factor is dyadic, so all four window coordinates are EXACT integers in `f32`, and
    /// neither clamp can mask the rounding (`6 < 7` and `12 < 15`).
    ///
    /// **Why `floor` is the conservative choice at an exact boundary.** Pixel `i` covers the
    /// half-open window interval `[i, i+1)`, so a coordinate of exactly `6.0` is the LEFT edge of
    /// pixel 6: the bound reaches into pixel 6 and a conservative footprint must contain it.
    /// `floor(6.0) = 6` does. `ceil(6.0) - 1 = 5` does not — it drops a whole column, `occ`
    /// becomes a min over a set that no longer contains the pixel where the instance is visible,
    /// and the verdict flips to a FALSE REJECT.
    #[test]
    fn anchor_pixel_rect_at_an_exact_integer_edge() {
        let pv = anchor_view_proj();

        // The fixture is only discriminating while the edges really ARE integral, so recompute
        // them here: a later change to the box or the extent cannot silently turn this back into
        // an interior case without failing.
        let x_hi = (0.5f32 + 1.0) * (EDGE_W as f32 * 0.5);
        let y_hi = (0.5f32 + 1.0) * (EDGE_H as f32 * 0.5);
        assert_eq!((x_hi, y_hi), (6.0, 12.0), "the upper edges must land ON a pixel boundary");
        assert_eq!(
            (x_hi.floor(), x_hi.ceil() - 1.0),
            (6.0, 5.0),
            "and here the two rounding forms DISAGREE — which is the whole point of the fixture"
        );

        let r = project_aabb(&pv, [EDGE_W, EDGE_H], [-1.0, -1.0, -4.0], [1.0, 1.0, -2.0])
            .expect("invariant: the box is in front of the eye and on-screen");
        assert_eq!(
            r.min,
            [2, 4],
            "an exactly-integer LOWER edge starts the pixel it names: floor(2.0) = 2"
        );
        assert_eq!(
            r.max,
            [6, 12],
            "an exactly-integer UPPER edge is TOUCHED and must be included; `ceil(hi) - 1` would \
             answer [5, 11] and drop a column and a row the bound covers"
        );
        assert!((r.min[0]..=r.max[0]).contains(&6), "pixel 6 is inside the footprint");
        assert!((r.min[1]..=r.max[1]).contains(&12), "row 12 is inside the footprint");
        assert_eq!(r.depth_near, 0.05, "0.1/2, exact — the same near face as box A");
    }

    /// THE VERDICT, END TO END, ON THE HAND-COMPUTED NUMBERS.
    ///
    /// * **Box A** `[-1,-1,-4]..[1,1,-2]` → rect `[1,5]×[0,2]` → `L = 1` → `occ = 0.05`;
    ///   `depth_near = 0.05`. `0.05 < 0.05` is FALSE ⇒ **KEEP**. This is the strictness of the
    ///   predicate, pinned: a `<=` would delete an instance exactly coincident with the occluder,
    ///   and the soundness proof's chain `occ ≤ D[p] ≤ d_i(p) ≤ depth_near` admits equality.
    /// * **Box B** `[-128,-128,-512]..[128,128,-256]` → `x_ndc = ±128/256 = ±0.5` and
    ///   `±128/512 = ±0.25` → the SAME rect `[1,5]×[0,2]`, so the same `L = 1`, `occ = 0.05`.
    ///   `depth_near = 0.1/256 = 3.90625e-4`. `3.9e-4 < 0.05` ⇒ **REJECT**.
    /// * **Box C** `[-1,-1,-40]..[1,1,-20]` → `x_ndc = ±0.05` (max) → `x_win ∈ [3.325, 3.675]`
    ///   → `[3,3]`; `y_win ∈ [1.425, 1.575]` → `[1,1]`. `L = 0`, texel `(1,0)`, `occ = 0.55`.
    ///   `depth_near = 0.1/20 = 5e-3`. `5e-3 < 0.55` ⇒ **REJECT**.
    /// * **Box D** `[-1,-1,-0.5]..[1,1,-0.25]` → rect `[0,6]×[0,2]` → `L = 1`, `occ = 0.05`;
    ///   `depth_near = 0.4`. `0.4 < 0.05` is false ⇒ **KEEP(NotOccluded)**.
    ///
    /// `0.1f32/256` and `0.1f32/20` are not exactly the printed decimals (`0.1` itself is not),
    /// so those two are pinned to `1e-9` — about 30 ulp at that magnitude, and five orders of
    /// magnitude below the gap that decides each verdict.
    #[test]
    fn anchor_verdicts() {
        let (l, p) = anchor_pyramid();
        let pv = anchor_view_proj();

        assert_eq!(
            occlusion_verdict(&l, &p, &pv, [-1.0, -1.0, -4.0], [1.0, 1.0, -2.0]),
            OcclusionVerdict::Keep(KeepReason::NotOccluded),
            "depth_near == occ == 0.05 must KEEP — the predicate is a STRICT `<`"
        );
        assert_eq!(
            occlusion_verdict(&l, &p, &pv, [-128.0, -128.0, -512.0], [128.0, 128.0, -256.0]),
            OcclusionVerdict::Reject,
            "a wide bound 256 units back, behind an occluder at 0.05, must reject"
        );
        assert_eq!(
            occlusion_verdict(&l, &p, &pv, [-1.0, -1.0, -40.0], [1.0, 1.0, -20.0]),
            OcclusionVerdict::Reject
        );
        assert_eq!(
            occlusion_verdict(&l, &p, &pv, [-1.0, -1.0, -0.5], [1.0, 1.0, -0.25]),
            OcclusionVerdict::Keep(KeepReason::NotOccluded)
        );

        // The intermediate numbers the verdicts above depend on, so a failure localises.
        let src = [ANCHOR_W, ANCHOR_H];
        let b = project_aabb(&pv, src, [-128.0, -128.0, -512.0], [128.0, 128.0, -256.0])
            .expect("invariant: box B is in front of the eye");
        assert_eq!((b.min, b.max), ([1, 0], [5, 2]));
        assert!((b.depth_near - 3.906_25e-4).abs() < 1e-9, "depth_near = {}", b.depth_near);

        let c = project_aabb(&pv, src, [-1.0, -1.0, -40.0], [1.0, 1.0, -20.0])
            .expect("invariant: box C is in front of the eye");
        assert_eq!((c.min, c.max), ([3, 1], [3, 1]));
        assert!((c.depth_near - 5.0e-3).abs() < 1e-9, "depth_near = {}", c.depth_near);
        let c_sel = select_texels(&l, &c).expect("invariant: level 0 always exists");
        assert_eq!(c_sel, TexelSelection { level: 0, tx: [1, 1], ty: [0, 0] });
        assert_eq!(occluder_depth(&l, &p, &c_sel), 0.55);
    }

    /// ⚠️ THE VIEWPORT FLIP, as a discriminating fixture.
    ///
    /// The anchor above cannot see a GL-style `(1 - y_ndc)` flip, because its boxes are vertically
    /// symmetric. This one is not: a box confined to the LOWER half of NDC-y.
    ///
    /// Box `y ∈ [0.5, 1.0]` at `z = -2` (a flat quad, so `w = 2` on all eight corners):
    /// `y_ndc = 0.5/2 = 0.25` and `1.0/2 = 0.5`, so `y_win = (0.25+1)*1.5 = 1.875` and
    /// `(0.5+1)*1.5 = 2.25` → rows `[1, 2]`.
    /// Under a GL-style flip it would be `(1 - 0.5)*1.5 = 0.75` and `(1 - 0.25)*1.5 = 1.125` →
    /// rows `[0, 1]`. The two answers differ, so this test fails on a flipped implementation.
    #[test]
    fn positive_viewport_height_means_plus_y_ndc_is_plus_y_window() {
        let pv = anchor_view_proj();
        let r = project_aabb(&pv, [ANCHOR_W, ANCHOR_H], [-1.0, 0.5, -2.0], [1.0, 1.0, -2.0])
            .expect("invariant: the quad is in front of the eye");
        assert_eq!(
            (r.min[1], r.max[1]),
            (1, 2),
            "+Y in NDC must map to INCREASING window Y (positive viewport height, no flip); a \
             GL-style flip would give rows (0, 1)"
        );
    }

    /// THE ENGINE'S OWN REVERSE-Z PROJECTION, hand-computed — the row convention, the Y-flip sign
    /// and a POSITION-DEPENDENT `clip.z`, all against the matrix the raster actually draws with.
    ///
    /// Every other fixture here uses `anchor_view_proj`, whose `row2 = (0, 0, 0, 0.1)` makes
    /// `clip.z` CONSTANT over the eight corners: the per-corner perspective divide is exercised
    /// only in its degenerate form, and the row/column convention is pinned only against a matrix
    /// this file wrote itself. This one takes [`crate::view::forward_view_proj_rows`] verbatim.
    ///
    /// Camera at the origin, forward `(0,0,-1)`, right `(1,0,0)`, up `(0,1,0)`, `near = 1`,
    /// `far = 5`, extent `8 × 8`. `depth_near` reads only rows 2 and 3, and NEITHER contains
    /// `tan`:
    ///
    /// ```text
    ///   row3 = (0, 0, -1, -0)                              →  clip.w = -z   (view distance)
    ///   a = -near/(far-near) = -0.25,   b = near*far/(far-near) = 1.25
    ///   row2 = a*row3 + (0,0,0,b) = (0, 0, 0.25, 1.25)     →  clip.z = 0.25 z + 1.25
    /// ```
    ///
    /// so `z_ndc = (1.25 - 0.25 d)/d` at view distance `d`: `1.0` at `d = near`, `0.0` at
    /// `d = far`. Reverse-Z, as `VK_COMPARE_OP_GREATER` requires.
    ///
    /// Box `[-0.6, 0.2, -4] .. [0.6, 0.6, -2]`, deliberately ASYMMETRIC in Y:
    /// * `d ∈ {2, 4}` → `z_ndc = 0.75/2 = 0.375` and `0.25/4 = 0.0625`, so
    ///   **`depth_near = 0.375`** — every factor dyadic, hence exact in `f32`.
    /// * row 1 carries `sy = -1/tan ≈ -1`, so `y_ndc = -y/d ∈ [-0.3, -0.05]` and
    ///   `y_win = (y_ndc + 1) * 4 ∈ [2.8, 3.8]` → rows `[2, 3]`. Without the projection's Y-flip
    ///   the same box gives `y_win ∈ [4.2, 5.2]` → rows `[4, 5]`.
    /// * `x_ndc = x/d ∈ [-0.3, 0.3]` → `x_win ∈ [2.8, 5.2]` → columns `[2, 5]`.
    ///
    /// The window coordinates carry `tan(π/4)`'s rounding (`sx`, `sy` are `±1` only to about
    /// 1e-7), which is why this box sits MID-pixel — 2.8 and 5.2 are nowhere near a boundary. The
    /// depth assertion is an `==` because rows 2 and 3 are `tan`-free and entirely dyadic.
    #[test]
    fn depth_near_and_the_y_flip_against_the_engines_forward_projection() {
        let view = ViewUniform {
            camera_pos: Vec4::new(0.0, 0.0, 0.0, 1.0),
            cam_forward: Vec4::new(0.0, 0.0, -1.0, 0.0),
            cam_right: Vec4::new(1.0, 0.0, 0.0, 0.0),
            cam_up: Vec4::new(0.0, 1.0, 0.0, 0.0),
            fov_y: core::f32::consts::FRAC_PI_2,
            aspect: 1.0,
            near: 1.0,
            far: 5.0,
            ..ViewUniform::IDENTITY
        };
        let pv = forward_view_proj_rows(&view, 8, 8);
        assert_eq!(pv[2], [0.0, 0.0, 0.25, 1.25], "the reverse-Z depth row, derived above");
        assert_eq!(pv[3], [0.0, 0.0, -1.0, 0.0], "clip.w is the view distance");
        assert!(
            pv[1][1] < 0.0,
            "the Y-flip lives HERE, in the projection's sy = -1/tan — not in the viewport"
        );

        let r = project_aabb(&pv, [8, 8], [-0.6, 0.2, -4.0], [0.6, 0.6, -2.0])
            .expect("invariant: the box is in front of the eye and on-screen");
        assert_eq!(r.depth_near, 0.375, "max over corners of clip.z/clip.w = 0.75/2");
        assert_eq!(
            (r.min[1], r.max[1]),
            (2, 3),
            "an UNFLIPPED projection would put this box on rows (4, 5)"
        );
        assert_eq!((r.min[0], r.max[0]), (2, 5));
    }

    /// ⚠️ `depth_near` is the MAX OF THE QUOTIENTS, not the quotient of the extremes.
    ///
    /// The component-wise variant `max_i(clip.z_i) / min_i(clip.w_i)` agrees with the correct
    /// `max_i(clip.z_i / clip.w_i)` on every other fixture in this file — and, for a STRUCTURAL
    /// reason rather than by luck, on the engine's real projection too:
    /// [`crate::view::forward_view_proj_rows`] builds `row2 = a·row3 + (0,0,0,b)` with
    /// `a = -near/(far-near) < 0`, so `clip.z = a·clip.w + b` is affine in `clip.w` and
    /// `max(a·w + b)/min(w) = a + b/min(w) = max(a + b/w)` identically. A fixture built from the
    /// real matrix therefore CANNOT distinguish the two forms. This one is built to.
    ///
    /// The matrix is `anchor_view_proj` with a `row2` that is NOT a multiple of `row3` plus a
    /// constant: `row2 = (0.125, 0, -0.0625, 0.3125)`, i.e. `clip.z = 0.125 x - 0.0625 z + 0.3125`
    /// varying with X independently of `clip.w = -z`. Box `[0,-1,-4] .. [4,1,-2]`:
    ///
    /// | corner (x, z) | `clip.z` | `clip.w` | `z_ndc` |
    /// |---|---|---|---|
    /// | (0, -2) | 0.4375 | 2 | 0.21875 |
    /// | (0, -4) | 0.5625 | 4 | 0.140625 |
    /// | (4, -2) | 0.9375 | 2 | **0.46875** |
    /// | (4, -4) | 1.0625 | 4 | 0.265625 |
    ///
    /// The largest `clip.z` (1.0625) and the smallest `clip.w` (2) sit on DIFFERENT corners, so
    /// the component-wise variant answers `1.0625 / 2 = 0.53125` while the true `depth_near` is
    /// `0.46875`. Every value is dyadic, so both are exact in `f32`.
    ///
    /// Which way does the wrong form err? While every `clip.z > 0` it OVER-states `depth_near`
    /// (`max z / min w ≥ z_j / w_j` for every corner `j`), and since the predicate is
    /// `depth_near < occ` that only keeps too much — harmless. The moment one corner has
    /// `clip.z < 0`, which under reverse-Z is a bound reaching past the far plane, the same
    /// expression UNDER-states `depth_near` and the error becomes a false REJECT. The oracle has
    /// to be right, not merely conservative for the matrices it happens to be handed.
    #[test]
    fn depth_near_is_the_max_of_the_quotients_not_the_quotient_of_the_extremes() {
        let mut pv = anchor_view_proj();
        pv[2] = [0.125, 0.0, -0.0625, 0.3125];

        let r = project_aabb(&pv, [8, 8], [0.0, -1.0, -4.0], [4.0, 1.0, -2.0])
            .expect("invariant: the box is in front of the eye and on-screen");
        assert_eq!((r.min, r.max), ([4, 2], [7, 6]));
        assert_eq!(
            r.depth_near, 0.46875,
            "depth_near must fold the PER-CORNER quotient; max(clip.z)/min(clip.w) gives 0.53125"
        );

        // The counterfactual, executed rather than argued.
        let component_wise = 1.0625f32 / 2.0;
        assert_eq!(component_wise, 0.53125);
        assert_ne!(component_wise, r.depth_near);
    }

    // ==========================================================================================
    // PART 2 — THE PROPERTIES
    //
    // A deterministic xorshift keeps failures reproducible and adds no dependency (this crate has
    // no `proptest` dev-dependency, and S3 may not change any manifest).
    // ==========================================================================================

    struct Rng(u64);

    impl Rng {
        fn new(seed: u64) -> Self {
            Rng(seed | 1)
        }

        fn next_u32(&mut self) -> u32 {
            let mut x = self.0;
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            self.0 = x;
            (x >> 32) as u32
        }

        /// A depth in `[0, 1]`, the reverse-Z range.
        fn next_depth(&mut self) -> f32 {
            f32::from(self.next_u32() as u16) / f32::from(u16::MAX)
        }

        fn below(&mut self, n: u32) -> u32 {
            self.next_u32() % n
        }
    }

    /// The mandated extents: odd, prime, degenerate on one axis, non-square, and cap-edge.
    /// `65536 × 1` is [`MAX_HZB_EXTENT`] itself — cheap in pixels, maximal in levels.
    const EXTENTS: [(u32, u32); 9] = [
        (7, 3),
        (13, 5),
        (1, 1),
        (1, 1080),
        (1024, 64),
        (1919, 1079),
        (2048, 2048),
        (65536, 1),
        (31, 17),
    ];

    /// The extents cheap enough to build a full pyramid for in a debug-profile test run.
    /// `2048 × 2048` and `1919 × 1079` are deliberately included — a soundness bug that only
    /// appears once the chain is long is exactly the bug this rung exists to preclude.
    const PYRAMID_EXTENTS: [(u32, u32); 8] = [
        (7, 3),
        (13, 5),
        (1, 1),
        (1, 1080),
        (1024, 64),
        (1919, 1079),
        (2048, 2048),
        (65536, 1),
    ];

    fn random_pyramid(layout: &HzbLayout, rng: &mut Rng) -> (Vec<f32>, Vec<f32>) {
        let depth: Vec<f32> = (0..layout.source_len()).map(|_| rng.next_depth()).collect();
        let mut pyramid = vec![0.0f32; layout.pyramid_len()];
        build_pyramid(layout, &depth, &mut pyramid);
        (depth, pyramid)
    }

    /// PROPERTY (a) — the base map's two halves are exact inverses, and the map PARTITIONS the
    /// source.
    ///
    /// Three separate claims, because a single one of them is satisfiable by a wrong map:
    /// 1. `texel_of(first(t)) == t` for every texel (right inverse);
    /// 2. `first(t) <= x < first(t+1)` for every pixel (the preimage really is the interval);
    /// 3. every source pixel is claimed by EXACTLY ONE texel — counted, so a map that both
    ///    double-covers and gaps (the classic symmetric off-by-one) is caught by the count,
    ///    not merely by the endpoints.
    #[test]
    fn property_base_map_inverts_and_partitions() {
        for (w, h) in EXTENTS {
            let l = HzbLayout::new(w, h).expect("invariant: EXTENTS are legal");
            for (axis, extent) in [(l.x(), w), (l.y(), h)] {
                assert!(axis.base() <= extent && axis.base().is_power_of_two());
                assert_eq!(axis.first_source(0), 0, "{w}x{h}: the first texel must start at 0");
                assert_eq!(
                    axis.first_source(axis.base()),
                    extent,
                    "{w}x{h}: the last texel must end at the source extent"
                );

                for t in 0..axis.base() {
                    assert_eq!(axis.texel_of(axis.first_source(t)), t, "{w}x{h}: texel {t}");
                }

                let mut owner_count = vec![0u32; extent as usize];
                for t in 0..axis.base() {
                    let (lo, hi) = (axis.first_source(t), axis.first_source(t + 1));
                    assert!(lo < hi, "{w}x{h}: texel {t} has an EMPTY preimage");
                    for x in lo..hi {
                        owner_count[x as usize] += 1;
                    }
                }
                for (x, &c) in owner_count.iter().enumerate() {
                    assert_eq!(c, 1, "{w}x{h}: pixel {x} is owned by {c} texels, not exactly 1");
                    let px = x as u32;
                    let t = axis.texel_of(px);
                    let interval = axis.first_source(t)..axis.first_source(t + 1);
                    assert!(
                        interval.contains(&px),
                        "{w}x{h}: pixel {x} maps to texel {t} whose interval {interval:?} \
                         excludes it"
                    );
                }
            }
        }
    }

    /// PROPERTY (b) — SOUNDNESS: `H[L][t] <= D[p]` for every level, texel and source pixel in that
    /// texel's span, the span being [`HzbAxis::level_source_span`].
    ///
    /// ⚠️ **SCOPE, stated exactly, because the stronger reading is FALSE.** This does NOT prove
    /// the base map is right. At level 0, `level_source_span(0, t)` is
    /// `(first_source(t), first_source(t + 1))` — literally the two calls [`build_pyramid`]'s own
    /// level-0 loop makes — and at every level above it is the union of those same intervals for
    /// any monotone `first_source`. A builder whose base map is off by one, or shifted, satisfies
    /// this property, because the check is derived from the same off-by-one map.
    ///
    /// What it DOES prove is worth having on its own: the whole chain is a lower bound over the
    /// footprint its own base map claims — i.e. the 2×2 reduce, the `max(1, …)` clamps on a
    /// bottomed-out axis and the level offsets are HIERARCHICALLY CONSISTENT with level 0. That is
    /// what catches a wrong footprint, a missing clamp, a level-offset error, and instantly a
    /// `max` reduce.
    ///
    /// The base map itself is pinned elsewhere, and only there: `anchor_base_map_every_pixel`
    /// (every pixel of `7 × 3`, by hand, both directions) and
    /// `property_base_map_inverts_and_partitions` (the inverse and partition laws, on 9 extents).
    #[test]
    fn property_pyramid_is_sound_over_its_own_level_span() {
        let mut rng = Rng::new(0x5eed_0001);
        for (w, h) in PYRAMID_EXTENTS {
            let l = HzbLayout::new(w, h).expect("invariant: PYRAMID_EXTENTS are legal");
            let (depth, pyramid) = random_pyramid(&l, &mut rng);

            for level in 0..l.levels() {
                let [lw, lh] = l.level_extent(level);
                for ty in 0..lh {
                    let (y_lo, y_hi) = l.y().level_source_span(level, ty);
                    for tx in 0..lw {
                        let (x_lo, x_hi) = l.x().level_source_span(level, tx);
                        let hv = l.texel(&pyramid, level, tx, ty);
                        for y in y_lo..y_hi {
                            let row = y as usize * w as usize;
                            for x in x_lo..x_hi {
                                let d = depth[row + x as usize];
                                assert!(
                                    hv <= d,
                                    "{w}x{h} level {level} texel ({tx},{ty}) = {hv} exceeds \
                                     D[{x},{y}] = {d} — the pyramid is NOT a lower bound"
                                );
                            }
                        }
                    }
                }
            }

            // SENSITIVITY: a pyramid of all-equal values satisfies soundness trivially. Pin that
            // the fixture actually varies, and that the top is the global minimum.
            let top = l.texel(&pyramid, l.levels() - 1, 0, 0);
            let global = depth.iter().copied().fold(f32::INFINITY, f32::min);
            assert_eq!(top, global, "{w}x{h}: the top of a complete chain is the global minimum");
            if l.source_len() > 1 {
                let max = depth.iter().copied().fold(f32::NEG_INFINITY, f32::max);
                assert!(max > global, "{w}x{h}: the random fixture degenerated to a constant");
            }
        }
    }

    /// PROPERTY (c) — COVERAGE: the source-pixel preimage of the ≤4 selected texels CONTAINS the
    /// rect.
    ///
    /// This is the premise of the whole soundness theorem: `occ` is a min over that preimage, so
    /// if the preimage misses even one pixel of the rect the bound `occ ≤ D[p]` fails for exactly
    /// the pixel where the instance is visible.
    ///
    /// The rect set is not random-only. It includes, per extent: the full framebuffer (the
    /// maximum supported rect), every single-pixel rect on a small extent, a rect straddling
    /// EVERY power-of-two texel boundary, and a random sweep.
    #[test]
    fn property_selected_texels_cover_the_rect() {
        let mut rng = Rng::new(0x5eed_0002);
        let mut saw_level_above_zero = false;
        let mut saw_level_zero = false;

        for (w, h) in EXTENTS {
            let l = HzbLayout::new(w, h).expect("invariant: EXTENTS are legal");
            let mut rects: Vec<([u32; 2], [u32; 2])> = Vec::new();

            // The maximum supported rect: the whole framebuffer.
            rects.push(([0, 0], [w - 1, h - 1]));

            // A rect straddling every power-of-two TEXEL boundary on each axis. Texels
            // `[2^b - 1, 2^b]` are the pair whose xor has its top bit exactly at `b`, so this is
            // the family that drives the selector to each level in turn.
            let mut bit = 0u32;
            while (1u32 << bit) < l.x().base() {
                let b = 1u32 << bit;
                let x0 = l.x().first_source(b - 1);
                let x1 = l.x().first_source(b + 1) - 1;
                rects.push(([x0, 0], [x1, h - 1]));
                bit += 1;
            }
            let mut bit = 0u32;
            while (1u32 << bit) < l.y().base() {
                let b = 1u32 << bit;
                let y0 = l.y().first_source(b - 1);
                let y1 = l.y().first_source(b + 1) - 1;
                rects.push(([0, y0], [w - 1, y1]));
                bit += 1;
            }

            for _ in 0..256 {
                let (a, b) = (rng.below(w), rng.below(w));
                let (c, d) = (rng.below(h), rng.below(h));
                rects.push(([a.min(b), c.min(d)], [a.max(b), c.max(d)]));
            }

            for (min, max) in rects {
                let rect = ScreenRect { min, max, depth_near: 0.0 };
                let sel = select_texels(&l, &rect)
                    .expect("invariant: a complete pyramid always has the selected level");
                if sel.level == 0 {
                    saw_level_zero = true;
                } else {
                    saw_level_above_zero = true;
                }

                let (x_lo, _) = l.x().level_source_span(sel.level, sel.tx[0]);
                let (_, x_hi) = l.x().level_source_span(sel.level, sel.tx[1]);
                let (y_lo, _) = l.y().level_source_span(sel.level, sel.ty[0]);
                let (_, y_hi) = l.y().level_source_span(sel.level, sel.ty[1]);
                assert!(
                    x_lo <= min[0] && max[0] < x_hi && y_lo <= min[1] && max[1] < y_hi,
                    "{w}x{h}: rect {min:?}..{max:?} escapes the preimage \
                     [{x_lo},{x_hi})x[{y_lo},{y_hi}) of level {} texels {:?}/{:?}",
                    sel.level,
                    sel.tx,
                    sel.ty
                );
                // The selection really is at most two texels per axis — the property that makes
                // four samples enough.
                assert!(sel.tx[1] - sel.tx[0] <= 1 && sel.ty[1] - sel.ty[0] <= 1);
            }
        }

        assert!(saw_level_zero && saw_level_above_zero, "the rect sweep never varied the level");
    }

    /// PROPERTY (c'), the counterfactual: **at a level finer than the selector's, two texels per
    /// axis are not enough** — they stop being ADJACENT, so their two preimages are no longer one
    /// contiguous span and real source pixels between them go unsampled.
    ///
    /// Without this, "the selection covers" is satisfied by a selector that always returns the
    /// top mip, and the coverage test above (which checks the enclosing interval) would also
    /// accept the `min(L, levels - 1)` clamp this module refuses. Here the gap is exhibited as a
    /// concrete pixel that lies in the rect and in NO sampled texel.
    ///
    /// Worked out by hand on `1919 × 1079` (base `1024 × 1024`, 11 levels), full-screen rect:
    /// `tx0 = 0`, `tx1 = ⌊1918·1024/1919⌋ = 1023`, so `L = msb(1023) = 9`.
    /// * At level 9 the samples are `1023 >> 9 = 1` and `0` — adjacent, spanning level-0 texels
    ///   `[0, 1024)`, i.e. every pixel.
    /// * At level 8 they are `0` and `1023 >> 8 = 3` — three apart. Texel 0 covers level-0 texels
    ///   `[0, 256)` = pixels `[0, ⌈256·1919/1024⌉) = [0, 480)`, and texel 3 covers `[768, 1024)`
    ///   = pixels `[1440, 1919)`. Pixel `1000` is in the rect and in neither.
    #[test]
    fn two_texels_per_axis_stop_covering_below_the_selected_level() {
        let l = HzbLayout::new(1919, 1079).expect("invariant: 1919x1079 is legal");
        let rect = ScreenRect { min: [0, 0], max: [1918, 1078], depth_near: 0.0 };
        let sel = select_texels(&l, &rect).expect("invariant: 1919x1079 has 11 levels");
        assert_eq!(sel.level, 9, "the fixture must land on the coarse end of the chain");
        assert_eq!(sel.tx, [0, 1], "at the selected level the two samples are ADJACENT");

        let finer = sel.level - 1;
        let tx0 = l.x().containing_texel(l.x().texel_of(rect.min[0]), finer);
        let tx1 = l.x().containing_texel(l.x().texel_of(rect.max[0]), finer);
        assert_eq!((tx0, tx1), (0, 3), "one level finer the samples are three texels apart");

        let (lo0, hi0) = l.x().level_source_span(finer, tx0);
        let (lo1, hi1) = l.x().level_source_span(finer, tx1);
        assert_eq!((lo0, hi0), (0, 480));
        assert_eq!((lo1, hi1), (1440, 1919));
        let orphan = 1000u32;
        assert!((rect.min[0]..=rect.max[0]).contains(&orphan));
        let covered = (lo0..hi0).contains(&orphan) || (lo1..hi1).contains(&orphan);
        assert!(
            !covered,
            "pixel {orphan} must be covered by NEITHER sampled texel at level {finer} — if it is \
             covered, this fixture cannot detect a clamp-down"
        );
    }

    /// PROPERTY (d) — MONOTONICITY: raising any `D[p]` never lowers any `H[L][t]`.
    ///
    /// The direction matters: between the two passes the depth buffer only ever gets NEARER as
    /// the early pass rasterises into it, so a pyramid that could move DOWN under a rising input
    /// would not be a stable bound across that update.
    ///
    /// The sensitivity half is made deterministic rather than left to luck: the fixture is
    /// generated in `[0.25, 1.0]` and then ONE pixel is forced to `0.0`, making it the unique
    /// global minimum. Raising that pixel to `1.0` must strictly raise its containing texel on
    /// EVERY level, so "nothing ever moved" cannot masquerade as "never went down".
    #[test]
    fn property_pyramid_is_monotone_in_the_source() {
        let mut rng = Rng::new(0x5eed_0003);
        for (w, h) in [(7u32, 3u32), (13, 5), (1024, 64), (31, 17), (1, 1080)] {
            let l = HzbLayout::new(w, h).expect("invariant: legal extent");
            let mut depth: Vec<f32> =
                (0..l.source_len()).map(|_| 0.25 + rng.next_depth() * 0.75).collect();
            let unique_min = l.source_len() / 3;
            depth[unique_min] = 0.0;

            let mut base = vec![0.0f32; l.pyramid_len()];
            build_pyramid(&l, &depth, &mut base);
            let mut raised = vec![0.0f32; l.pyramid_len()];

            // The `>=` direction, over random single-pixel raises.
            for _ in 0..8 {
                let p = (rng.next_u32() as usize) % depth.len();
                let old = depth[p];
                let new = old + (1.0 - old) * 0.5;
                depth[p] = new;
                build_pyramid(&l, &depth, &mut raised);
                for (i, (&b, &r)) in base.iter().zip(raised.iter()).enumerate() {
                    assert!(
                        r >= b,
                        "{w}x{h}: raising D[{p}] from {old} to {new} LOWERED pyramid[{i}] from \
                         {b} to {r}"
                    );
                }
                depth[p] = old;
            }

            // SENSITIVITY: a raise that must PROPAGATE, so `>=` above is not satisfied by a
            // pyramid that ignores its input.
            depth[unique_min] = 1.0;
            build_pyramid(&l, &depth, &mut raised);
            let px = (unique_min % w as usize) as u32;
            let py = (unique_min / w as usize) as u32;
            for level in 0..l.levels() {
                let tx = l.x().containing_texel(l.x().texel_of(px), level);
                let ty = l.y().containing_texel(l.y().texel_of(py), level);
                assert_eq!(l.texel(&base, level, tx, ty), 0.0, "{w}x{h}: level {level} baseline");
                assert!(
                    l.texel(&raised, level, tx, ty) >= 0.25,
                    "{w}x{h}: raising the unique global minimum did not propagate to level {level}"
                );
            }
            for (&b, &r) in base.iter().zip(raised.iter()) {
                assert!(r >= b, "{w}x{h}: the full raise LOWERED a texel");
            }
        }
    }

    /// PROPERTY (e.1) — the UNKNOWN-BOUNDS sentinel KEEPS, and is tested BEFORE the projection.
    ///
    /// The control is the same instance with REAL bounds at a position the oracle rejects, so the
    /// assertion cannot pass merely because nothing is being culled.
    ///
    /// ⚠️ The ORDER is the load-bearing part, and it is the defect a critic found in the frustum
    /// arm's shader (see [`crate::frustum::instance_visible_after_cull`]'s doc). Projected first,
    /// a large inverted sentinel comes out as some finite rect at some finite depth — here, a
    /// `min`/`max` fold over eight corners built from SWAPPED bounds, which is a perfectly
    /// well-formed rect. "Bounds unknown" would then mean "cull it", inverting the contract.
    #[test]
    fn keep_case_unknown_bounds_is_tested_before_the_projection() {
        let (l, p) = anchor_pyramid();
        let pv = anchor_view_proj();

        // Control: real bounds at this position REJECT.
        assert_eq!(
            occlusion_verdict(&l, &p, &pv, [-1.0, -1.0, -40.0], [1.0, 1.0, -20.0]),
            OcclusionVerdict::Reject,
            "the control must reject, or the sentinel assertions below prove nothing"
        );

        // The sentinel: min > max. Inverted on all three axes...
        assert_eq!(
            occlusion_verdict(&l, &p, &pv, [1.0, 1.0, -20.0], [-1.0, -1.0, -40.0]),
            OcclusionVerdict::Keep(KeepReason::UnknownBounds)
        );
        // ...and on one axis only, which is still not a box.
        assert_eq!(
            occlusion_verdict(&l, &p, &pv, [-1.0, 1.0, -40.0], [1.0, -1.0, -20.0]),
            OcclusionVerdict::Keep(KeepReason::UnknownBounds)
        );
        // The large-magnitude sentinel a never-registered mesh leaves behind.
        let big = 1.0e30f32;
        assert_eq!(
            occlusion_verdict(&l, &p, &pv, [big, big, big], [-big, -big, -big]),
            OcclusionVerdict::Keep(KeepReason::UnknownBounds)
        );
        // The order claim, executed: swapping the corners back produces a rect that the test
        // would happily consume. That it never gets there is what the sentinel buys.
        assert!(
            project_aabb(&pv, [ANCHOR_W, ANCHOR_H], [-1.0, -1.0, -40.0], [1.0, 1.0, -20.0]).is_ok(),
            "the same corners in the right order DO project — so the sentinel is what stops it"
        );
    }

    /// PROPERTY (e.2) — the `w > 0` guard KEEPS.
    ///
    /// `anchor_view_proj` has `clip.w = -z`, so any corner with `z >= 0` is at or behind the eye
    /// plane. Both a box straddling it and a box wholly behind it must KEEP.
    #[test]
    fn keep_case_behind_the_eye() {
        let (l, p) = anchor_pyramid();
        let pv = anchor_view_proj();
        assert_eq!(
            occlusion_verdict(&l, &p, &pv, [-1.0, -1.0, -2.0], [1.0, 1.0, 1.0]),
            OcclusionVerdict::Keep(KeepReason::BehindEye),
            "a bound straddling the eye plane must KEEP: the perspective divide is meaningless \
             there and the max-over-corners argument for depth_near does not hold"
        );
        assert_eq!(
            occlusion_verdict(&l, &p, &pv, [-1.0, -1.0, 20.0], [1.0, 1.0, 40.0]),
            OcclusionVerdict::Keep(KeepReason::BehindEye)
        );
        // Exactly on the plane (`w == 0`) is the boundary, and it is a KEEP too.
        assert_eq!(
            occlusion_verdict(&l, &p, &pv, [-1.0, -1.0, -4.0], [1.0, 1.0, 0.0]),
            OcclusionVerdict::Keep(KeepReason::BehindEye)
        );
    }

    /// PROPERTY (e.3) — the `isfinite` guard KEEPS, for a NaN in the bound and for a non-finite
    /// matrix.
    ///
    /// A NaN compares false against everything, so an unguarded implementation does not fail
    /// loudly — it silently takes whichever branch `false` leads to, and there is no reason that
    /// branch is the safe one.
    #[test]
    fn keep_case_non_finite() {
        let (l, p) = anchor_pyramid();
        let pv = anchor_view_proj();
        let nan = f32::NAN;

        assert_eq!(
            occlusion_verdict(&l, &p, &pv, [-1.0, nan, -40.0], [1.0, 1.0, -20.0]),
            OcclusionVerdict::Keep(KeepReason::UnknownBounds),
            "a NaN bound fails the sentinel's `min <= max` first — also a KEEP, by design"
        );
        // A NaN that survives the sentinel: finite corners, non-finite matrix.
        let mut bad = pv;
        bad[0][0] = nan;
        assert_eq!(
            occlusion_verdict(&l, &p, &bad, [-1.0, -1.0, -40.0], [1.0, 1.0, -20.0]),
            OcclusionVerdict::Keep(KeepReason::NonFinite)
        );
        let mut inf = pv;
        inf[1][3] = f32::INFINITY;
        assert_eq!(
            occlusion_verdict(&l, &p, &inf, [-1.0, -1.0, -40.0], [1.0, 1.0, -20.0]),
            OcclusionVerdict::Keep(KeepReason::NonFinite)
        );
        // A finite matrix, finite bounds and a finite `clip` — but the perspective DIVIDE
        // overflows: `w = 0.25` means `inv_w = 4`, and `4 * f32::MAX` is infinity. This is the
        // case the pre-divide finiteness check alone cannot see, which is why the guard is
        // repeated after the divide.
        let huge = f32::MAX;
        assert_eq!(
            occlusion_verdict(&l, &p, &pv, [-huge, -1.0, -0.5], [huge, 1.0, -0.25]),
            OcclusionVerdict::Keep(KeepReason::NonFinite)
        );
    }

    /// PROPERTY (e.4) — the empty/off-screen rect KEEPS.
    ///
    /// Rejecting an off-screen instance is the frustum arm's job. This test's guard exists so
    /// that the HZB never takes that decision on evidence it does not have.
    #[test]
    fn keep_case_empty_rect() {
        let (l, p) = anchor_pyramid();
        let pv = anchor_view_proj();
        // Far to the right: the smallest `x_ndc` over the eight corners is `40/4 = 10`, so the
        // smallest `x_win` is `11 * 3.5 = 38.5` — every column is past the last pixel, 6.
        assert_eq!(
            occlusion_verdict(&l, &p, &pv, [40.0, -1.0, -4.0], [80.0, 1.0, -2.0]),
            OcclusionVerdict::Keep(KeepReason::EmptyRect)
        );
        // Far to the left.
        assert_eq!(
            occlusion_verdict(&l, &p, &pv, [-80.0, -1.0, -4.0], [-40.0, 1.0, -2.0]),
            OcclusionVerdict::Keep(KeepReason::EmptyRect)
        );
        // Above the framebuffer (negative window Y).
        assert_eq!(
            occlusion_verdict(&l, &p, &pv, [-1.0, -80.0, -4.0], [1.0, -40.0, -2.0]),
            OcclusionVerdict::Keep(KeepReason::EmptyRect)
        );
    }

    /// PROPERTY (e.5) — an unavailable level KEEPS, and clamping DOWN would be a FALSE REJECT.
    ///
    /// Reached through [`HzbLayout::truncated`], the partial-chain constructor that makes the
    /// guard live: on a COMPLETE chain the selector provably never asks for a level past the top
    /// (`msb(tx0 ^ tx1) <= log2(base) - 1 = levels - 2`), so only a stopped chain exercises it.
    ///
    /// The counterfactual is EXECUTED, not asserted in prose. Box E is
    /// `[-0.8,-0.8,-2.0] .. [0.8,0.8,-1.6]`:
    /// * `w ∈ {2.0, 1.6}`; `x_ndc` spans `[-0.5, 0.5]` (the near face) so `x_win ∈ [1.75, 5.25]`
    ///   → pixels `[1, 5]`, and `y_win ∈ [0.75, 2.25]` → rows `[0, 2]`. `L = 1`.
    /// * `depth_near = 0.1 / 1.6 = 0.0625`.
    /// * The CORRECT `occ` at level 1 is `min(0.05, 0.15) = 0.05`, and `0.0625 < 0.05` is false —
    ///   **KEEP**.
    /// * Clamped DOWN to level 0 the samples would be `(0,0)=0.75`, `(2,0)=0.35`, `(0,1)=0.10`,
    ///   `(2,1)=0.15`, a min of `0.10` — larger, because it is a min over a strict SUBSET of the
    ///   rect's footprint (texel column 1 is never sampled). And `0.0625 < 0.10` is true:
    ///   **REJECT**. The bound is visible; the clamp deletes it.
    #[test]
    fn keep_case_level_unavailable_never_clamps_down() {
        let full = anchor_layout();
        let mut full_pyramid = vec![0.0f32; full.pyramid_len()];
        build_pyramid(&full, &ANCHOR_DEPTH, &mut full_pyramid);
        let pv = anchor_view_proj();

        let (e_min, e_max) = ([-0.8f32, -0.8, -2.0], [0.8f32, 0.8, -1.6]);
        let rect = project_aabb(&pv, [ANCHOR_W, ANCHOR_H], e_min, e_max)
            .expect("invariant: box E is in front of the eye");
        assert_eq!((rect.min, rect.max), ([1, 0], [5, 2]));
        assert!(
            rect.depth_near > 0.05 && rect.depth_near < 0.10,
            "box E must sit BETWEEN the true occ (0.05) and the clamped-down occ (0.10), or the \
             counterfactual below cannot flip; depth_near = {}",
            rect.depth_near
        );
        let sel = select_texels(&full, &rect).expect("invariant: the full chain has level 1");
        assert_eq!(sel.level, 1, "the fixture must need level 1 to exercise the guard");
        assert_eq!(occluder_depth(&full, &full_pyramid, &sel), 0.05);
        assert_eq!(
            occlusion_verdict(&full, &full_pyramid, &pv, e_min, e_max),
            OcclusionVerdict::Keep(KeepReason::NotOccluded),
            "control: against its CORRECT level box E is visible"
        );

        // A chain stopped after level 0 alone.
        let stopped = HzbLayout::truncated(ANCHOR_W, ANCHOR_H, 1)
            .expect("invariant: 1 <= 3 levels is a legal truncation");
        assert_eq!(stopped.pyramid_len(), 8, "a stopped chain stores only its level 0");
        let mut stopped_pyramid = vec![0.0f32; stopped.pyramid_len()];
        build_pyramid(&stopped, &ANCHOR_DEPTH, &mut stopped_pyramid);
        assert_eq!(
            &stopped_pyramid[..],
            &full_pyramid[0..8],
            "level 0 must not depend on levels()"
        );

        assert_eq!(
            select_texels(&stopped, &rect),
            Err(KeepReason::LevelUnavailable),
            "level 1 does not exist here; the selector must refuse rather than clamp to level 0"
        );
        assert_eq!(
            occlusion_verdict(&stopped, &stopped_pyramid, &pv, e_min, e_max),
            OcclusionVerdict::Keep(KeepReason::LevelUnavailable)
        );

        // The clamp-down, built by hand and run through the SHIPPED `occluder_depth`.
        let clamped = TexelSelection {
            level: 0,
            tx: [stopped.x().texel_of(rect.min[0]), stopped.x().texel_of(rect.max[0])],
            ty: [stopped.y().texel_of(rect.min[1]), stopped.y().texel_of(rect.max[1])],
        };
        assert_eq!(clamped.tx, [0, 2], "at level 0 the two samples skip texel column 1 entirely");
        let clamped_occ = occluder_depth(&stopped, &stopped_pyramid, &clamped);
        assert_eq!(clamped_occ, 0.10, "min(0.75, 0.35, 0.10, 0.15)");
        assert!(
            rect.depth_near < clamped_occ,
            "a clamped-down cull would REJECT box E — that is the deleted geometry the \
             LevelUnavailable KEEP exists to prevent"
        );
    }

    /// PROPERTY (e.6) — a bound in front of everything KEEPS with [`KeepReason::NotOccluded`],
    /// and the whole oracle is not merely "always KEEP".
    #[test]
    fn keep_case_not_occluded_and_reject_is_reachable() {
        let (l, p) = anchor_pyramid();
        let pv = anchor_view_proj();
        assert_eq!(
            occlusion_verdict(&l, &p, &pv, [-1.0, -1.0, -0.5], [1.0, 1.0, -0.25]),
            OcclusionVerdict::Keep(KeepReason::NotOccluded)
        );
        assert_eq!(
            occlusion_verdict(&l, &p, &pv, [-1.0, -1.0, -40.0], [1.0, 1.0, -20.0]),
            OcclusionVerdict::Reject,
            "if this ever becomes a KEEP the oracle is inert and every KEEP test above is vacuous"
        );
    }

    /// The reverse-Z reduce, pinned in both directions.
    ///
    /// `min` is what the reject predicate needs; `max` would be the silent inversion. Asserted as
    /// a difference in the ORACLE's answer, not just in the reduce, so the consequence is visible.
    #[test]
    fn the_reduce_is_min_and_a_max_would_delete_geometry() {
        let (l, p) = anchor_pyramid();
        assert_eq!(l.texel(&p, 1, 0, 0), 0.05, "min over {{0.75, 0.55, 0.10, 0.05}}");

        // The counterfactual: the SAME footprint reduced with `max`.
        let max_variant = [0.75f32, 0.55, 0.10, 0.05].into_iter().fold(f32::NEG_INFINITY, f32::max);
        assert_eq!(max_variant, 0.75);

        // Box A sits at depth_near = 0.05 — exactly the true `occ`, hence KEPT. Against the `max`
        // variant `occ` would be 0.75 and `0.05 < 0.75` REJECTS it, even though the source holds a
        // pixel at 0.05 inside its footprint: a surface at 0.05 there is not occluded by anything.
        let pv = anchor_view_proj();
        assert_eq!(
            occlusion_verdict(&l, &p, &pv, [-1.0, -1.0, -4.0], [1.0, 1.0, -2.0]),
            OcclusionVerdict::Keep(KeepReason::NotOccluded)
        );
        assert!(0.05f32 < max_variant, "the `max` variant would have rejected this visible bound");
    }

    /// The NaN policy on the reduce: unknown depth becomes `-∞`, which can never reject.
    #[test]
    fn a_nan_depth_sample_can_never_reject() {
        let l = anchor_layout();
        let pv = anchor_view_proj();
        let mut depth = ANCHOR_DEPTH;
        // Pixel (3, 2) is the `0.05` sample, and it lives in level-0 texel (1, 1).
        depth[2 * 7 + 3] = f32::NAN;
        let mut pyramid = vec![0.0f32; l.pyramid_len()];
        build_pyramid(&l, &depth, &mut pyramid);

        assert_eq!(l.texel(&pyramid, 0, 1, 1), f32::NEG_INFINITY);
        assert_eq!(
            l.texel(&pyramid, 2, 0, 0),
            f32::NEG_INFINITY,
            "-inf must propagate up the chain"
        );

        // Box C samples level-0 texel (1, 0), which the NaN never reaches — it still rejects.
        let (c_min, c_max) = ([-1.0f32, -1.0, -40.0], [1.0f32, 1.0, -20.0]);
        assert_eq!(
            occlusion_verdict(&l, &pyramid, &pv, c_min, c_max),
            OcclusionVerdict::Reject,
            "an unrelated texel must not be poisoned, or the KEEP below proves nothing"
        );
        // Box B's footprint DOES reach the poisoned texel, so it is kept.
        let (b_min, b_max) = ([-128.0f32, -128.0, -512.0], [128.0f32, 128.0, -256.0]);
        assert_eq!(
            occlusion_verdict(&l, &pyramid, &pv, b_min, b_max),
            OcclusionVerdict::Keep(KeepReason::NotOccluded),
            "unknown depth must read as infinitely far, never as an occluder"
        );
    }

    /// [`prev_pow2`] and the level count, on the boundaries that matter.
    #[test]
    fn prev_pow2_and_level_count() {
        assert_eq!(prev_pow2(0), 0);
        assert_eq!(prev_pow2(1), 1);
        assert_eq!(prev_pow2(2), 2);
        assert_eq!(prev_pow2(3), 2);
        assert_eq!(prev_pow2(7), 4);
        assert_eq!(prev_pow2(8), 8);
        assert_eq!(prev_pow2(9), 8);
        assert_eq!(prev_pow2(1919), 1024);
        assert_eq!(prev_pow2(MAX_HZB_EXTENT), MAX_HZB_EXTENT);
        assert_eq!(prev_pow2(u32::MAX), 1 << 31);

        for (w, h, levels) in [
            (1u32, 1u32, 1u32),
            (7, 3, 3),
            (1, 1080, 11),
            (1024, 64, 11),
            (1919, 1079, 11),
            (2048, 2048, 12),
            (65536, 1, 17),
        ] {
            let l = HzbLayout::new(w, h).expect("invariant: legal extent");
            assert_eq!(l.levels(), levels, "{w}x{h}");
            let last = l.level_extent(levels - 1);
            assert_eq!(last, [1, 1], "{w}x{h}: the top of a complete chain is 1x1");
        }
        assert_eq!(HzbLayout::new(0, 4), Err(HzbLayoutError::ZeroExtent));
        assert_eq!(HzbLayout::new(4, 0), Err(HzbLayoutError::ZeroExtent));
        assert_eq!(HzbLayout::new(MAX_HZB_EXTENT + 1, 4), Err(HzbLayoutError::ExtentTooLarge));
        assert_eq!(HzbLayout::truncated(7, 3, 0), Err(HzbLayoutError::ZeroLevels));
        assert_eq!(HzbLayout::truncated(7, 3, 4), Err(HzbLayoutError::TooManyLevels));
        assert_eq!(MAX_HZB_LEVELS, 17);
    }

    /// A degenerate axis (`1 × 1080`) exercises the `max(1, e >> k)` clamp from level 0 on: X is
    /// pinned at 1 texel for the whole chain while Y keeps halving.
    #[test]
    fn a_degenerate_axis_clamps_from_level_zero() {
        let l = HzbLayout::new(1, 1080).expect("invariant: 1x1080 is legal");
        assert_eq!(l.x().base(), 1);
        assert_eq!(l.y().base(), 1024);
        assert_eq!(l.levels(), 11);
        for level in 0..l.levels() {
            assert_eq!(l.level_extent(level)[0], 1, "X must stay clamped at level {level}");
        }
        assert_eq!(l.level_extent(10), [1, 1]);
        // The whole source column reduces into the single level-0 texel.
        assert_eq!(l.x().level_source_span(0, 0), (0, 1));
        assert_eq!(
            l.y().level_source_span(0, 0),
            (0, 2),
            "1080/1024 gives 2-pixel texels at the head"
        );
    }
}
