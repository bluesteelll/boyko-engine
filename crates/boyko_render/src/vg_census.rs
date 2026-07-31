//! **VG-R0 rung R0c — the host-side density reducer.**
//!
//! Turns one `vb_id` readback into the statistics a census row carries. The GPU half of R0c (the
//! ring's `TRANSFER_SRC` usage, the `Option`-threaded armed readback) hands this function a buffer
//! of `R32G32_UINT` texels; everything below is pure CPU and is exhaustively testable without a
//! device, which is why it is its own module.
//!
//! # What a row is
//!
//! A **census row** is one reading, at one `(camera path, ladder rung)` pair, of every statistic
//! R0d(b) enumerates *that is readable at that pair*. `D_est` is not among them — it divides
//! `visible_tris` at the TOP rung by `covered_pixels` at the DECISION rung, two different rungs —
//! and neither is the convergence check, which is a relation *between* rungs. Both are derived from
//! rows and reported per path. This module produces exactly the per-pair members.
//!
//! # The one design decision worth stating
//!
//! Distinct visible triangles are counted by **sorting the `(instance, primitive)` keys and
//! counting runs**, not by a hash set — `HashMap`/`HashSet` are disallowed in this workspace, and
//! the sort is the better instrument anyway: a run's *length* is exactly that triangle's covered
//! pixel count, so the histogram falls out of the same pass rather than needing a second structure.

/// A pixel the mesh raster leg never covered — the SDF leg's own hit, or the sky background. Host
/// mirror of the shader-side `VB_ID_SENTINEL` in `vb_pack.hlsli`.
pub const VB_ID_SENTINEL: u32 = 0xFFFF_FFFF;

/// The per-pair statistics of one census row.
///
/// Every field here is readable at a single `(path, rung)` pair. Anything indexed by more than one
/// rung, or by none, is deliberately absent.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CensusRow {
    /// Texels the mesh raster leg covered — `D_est`'s denominator at the decision rung.
    pub covered_pixels: u64,
    /// Distinct `(instance_id, primitive_id)` pairs that won at least one texel — `D_est`'s
    /// numerator at the top rung.
    pub visible_tris: u64,
    /// `histogram[b]` = triangles whose covered-pixel count `c` satisfies `2^b <= c < 2^(b+1)`.
    ///
    /// ⚠️ **Left-censored at one pixel by construction**: a triangle that wins no texel is not
    /// visible and does not appear at all, so bucket 0 is the lowest occupiable bucket and the
    /// distribution cannot represent sub-pixel triangles. In the micro-polygon regime the census
    /// exists to serve, every newly visible triangle enters at bucket 0 and pushes the mode the
    /// wrong way — which is precisely why R0d reports the cross-rung shift instead of gating it.
    pub histogram: Vec<u64>,
    /// The most populated bucket, or `None` on an empty frame. Ties take the LOWER bucket, stated
    /// because a silent tie-break would be an unrecorded decision in a decision-bearing statistic.
    pub modal_bucket: Option<u32>,
}

impl CensusRow {
    /// `visible_tris / covered_pixels` — a `[k1].report_only` statistic. It **saturates at 1.0 by
    /// construction** (one winner per texel), which is why it adjudicates nothing and is reported
    /// rather than gated.
    pub fn visible_tri_per_covered_pixel(&self) -> f64 {
        if self.covered_pixels == 0 {
            return 0.0;
        }
        self.visible_tris as f64 / self.covered_pixels as f64
    }

    /// `submitted / covered_pixels` — the other `[k1].report_only` statistic, a cull-efficiency
    /// reading. `submitted` counts culled and off-screen geometry, so it is an upper bound that
    /// bounds nothing tightly; it comes from the draw path, not from the readback.
    pub fn submitted_per_covered_pixel(&self, submitted_tris: u64) -> f64 {
        if self.covered_pixels == 0 {
            return 0.0;
        }
        submitted_tris as f64 / self.covered_pixels as f64
    }

    /// Whether this row clears the non-degeneracy floors R0c(c′) and R0d(c) assert.
    ///
    /// Both floors exist because `D_est` and the convergence check are **divisions**: on a
    /// sentinel-only readback `visible_tris = 0`, the convergence check reads `0 <= 0` (converged)
    /// and `D_est = 0`, so an empty frame satisfied K1's fire condition in an earlier revision.
    /// A frame that cannot be adjudicated must be refused, not divided by.
    pub fn is_non_degenerate(&self, min_covered_pixels: u64, min_visible_tris: u64) -> bool {
        self.covered_pixels >= min_covered_pixels && self.visible_tris >= min_visible_tris
    }
}

/// Reduces one `vb_id` readback into a [`CensusRow`].
///
/// `texels` is the raw `R32G32_UINT` buffer in row-major order: `.0` is `instance_id`, `.1` is the
/// raw primitive id. Texels carrying [`VB_ID_SENTINEL`] in `.0` are not mesh-covered and are
/// excluded from every statistic — the census's denominator is **mesh-covered pixels**, not all
/// pixels.
pub fn reduce(texels: &[[u32; 2]]) -> CensusRow {
    // One key per covered texel: `(instance << 32) | primitive`. Packing into a `u64` makes the
    // sort a single scalar compare and keeps the working set half the size of a tuple sort.
    let mut keys: Vec<u64> = Vec::with_capacity(texels.len());
    for t in texels {
        if t[0] != VB_ID_SENTINEL {
            keys.push(((t[0] as u64) << 32) | t[1] as u64);
        }
    }
    let covered_pixels = keys.len() as u64;
    if covered_pixels == 0 {
        return CensusRow::default();
    }

    keys.sort_unstable();

    // A run of equal keys is one triangle; the run's LENGTH is its covered-pixel count, so the
    // distinct count and the histogram come out of the same pass.
    let mut histogram: Vec<u64> = Vec::new();
    let mut visible_tris = 0u64;
    let mut run_start = 0usize;
    for i in 1..=keys.len() {
        if i == keys.len() || keys[i] != keys[run_start] {
            visible_tris += 1;
            let run = (i - run_start) as u64;
            let bucket = run.ilog2() as usize;
            if histogram.len() <= bucket {
                histogram.resize(bucket + 1, 0);
            }
            histogram[bucket] += 1;
            run_start = i;
        }
    }

    // Ties take the LOWER bucket: `>` rather than `>=` keeps the first maximum.
    let modal_bucket = histogram
        .iter()
        .enumerate()
        .fold(None::<(usize, u64)>, |best, (b, &n)| match best {
            Some((_, bn)) if n <= bn => best,
            _ if n == 0 => best,
            _ => Some((b, n)),
        })
        .map(|(b, _)| b as u32);

    CensusRow { covered_pixels, visible_tris, histogram, modal_bucket }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `n` texels all belonging to one triangle.
    fn tri(instance: u32, prim: u32, n: usize) -> Vec<[u32; 2]> {
        vec![[instance, prim]; n]
    }

    #[test]
    fn a_sentinel_only_readback_is_degenerate_rather_than_zero_density() {
        let row = reduce(&vec![[VB_ID_SENTINEL, 0]; 4096]);
        assert_eq!(row.covered_pixels, 0);
        assert_eq!(row.visible_tris, 0);
        assert_eq!(row.modal_bucket, None, "an empty frame has no mode to report");
        assert!(
            !row.is_non_degenerate(1024, 1024),
            "the whole point of the floors: an empty frame must be REFUSED, not divided by"
        );
        // And the report-only ratios must not divide by zero.
        assert_eq!(row.visible_tri_per_covered_pixel(), 0.0);
        assert_eq!(row.submitted_per_covered_pixel(5_000), 0.0);
    }

    #[test]
    fn sentinel_texels_are_excluded_from_the_denominator() {
        let mut t = tri(1, 7, 100);
        t.extend(vec![[VB_ID_SENTINEL, 0]; 900]);
        let row = reduce(&t);
        assert_eq!(row.covered_pixels, 100, "the denominator is MESH-covered pixels, not all pixels");
        assert_eq!(row.visible_tris, 1);
    }

    #[test]
    fn distinct_triangles_are_counted_across_instances_and_primitives() {
        let mut t = tri(0, 0, 3);
        t.extend(tri(0, 1, 3)); // same instance, different primitive
        t.extend(tri(1, 0, 3)); // different instance, SAME primitive id
        let row = reduce(&t);
        assert_eq!(
            row.visible_tris, 3,
            "the key is the PAIR — a primitive id is only unique within its instance"
        );
        assert_eq!(row.covered_pixels, 9);
    }

    /// The property R0c gate (b) rests on: with an analytically known screen-space triangle size,
    /// the modal bucket IS the analytic bucket.
    #[test]
    fn the_modal_bucket_is_the_analytic_bucket() {
        for px in [1u32, 2, 3, 4, 7, 8, 32, 1024] {
            let mut t = Vec::new();
            for p in 0..64u32 {
                t.extend(tri(0, p, px as usize));
            }
            let row = reduce(&t);
            assert_eq!(
                row.modal_bucket,
                Some(px.ilog2()),
                "{px} px/triangle must land in bucket floor(log2({px}))"
            );
        }
    }

    /// R0c(b)'s named red mutation, as arithmetic: subdividing the fixture 4x quarters each
    /// triangle's area, so the mode must move by exactly TWO buckets. A control that only asserts
    /// "the number changed" is the defect this campaign keeps finding — the required DIRECTION and
    /// MAGNITUDE is what makes it a gate.
    #[test]
    fn a_four_fold_subdivision_moves_the_mode_by_exactly_two_buckets() {
        let coarse: Vec<[u32; 2]> = (0..64u32).flat_map(|p| tri(0, p, 64)).collect();
        // Same total coverage, four times the triangles, a quarter of the area each.
        let fine: Vec<[u32; 2]> = (0..256u32).flat_map(|p| tri(0, p, 16)).collect();

        let c = reduce(&coarse);
        let f = reduce(&fine);
        assert_eq!(c.covered_pixels, f.covered_pixels, "the fixture covers the same area");
        assert_eq!(
            c.modal_bucket.unwrap() - f.modal_bucket.unwrap(),
            2,
            "4x subdivision must move the mode DOWN by two buckets, not merely move it"
        );
    }

    #[test]
    fn the_histogram_is_left_censored_at_one_pixel() {
        // A triangle winning zero texels is not in the readback at all, so bucket 0 is the floor.
        let row = reduce(&(0..10u32).flat_map(|p| tri(0, p, 1)).collect::<Vec<_>>());
        assert_eq!(row.modal_bucket, Some(0));
        assert_eq!(row.histogram[0], 10);
        assert_eq!(row.histogram.len(), 1, "nothing can occupy a bucket below 0");
    }

    #[test]
    fn the_raw_ratio_saturates_at_one_which_is_why_it_adjudicates_nothing() {
        // Every triangle winning exactly one texel is the densest readable case.
        let row = reduce(&(0..500u32).flat_map(|p| tri(0, p, 1)).collect::<Vec<_>>());
        assert_eq!(
            row.visible_tri_per_covered_pixel(),
            1.0,
            "one winner per texel caps this statistic at 1.0 by construction"
        );
        // The cull-efficiency reading is NOT capped — it counts submitted geometry.
        assert!(row.submitted_per_covered_pixel(5_000) > 1.0);
    }

    #[test]
    fn a_tie_takes_the_lower_bucket() {
        let mut t: Vec<[u32; 2]> = (0..4u32).flat_map(|p| tri(0, p, 2)).collect(); // bucket 1
        t.extend((10..14u32).flat_map(|p| tri(0, p, 8))); // bucket 3, same count
        let row = reduce(&t);
        assert_eq!(row.histogram[1], 4);
        assert_eq!(row.histogram[3], 4);
        assert_eq!(row.modal_bucket, Some(1), "a tie must resolve the way the doc says it does");
    }

    #[test]
    fn the_non_degeneracy_floors_bind_both_terms() {
        // Above the pixel floor, below the triangle floor: one huge triangle.
        let row = reduce(&tri(0, 0, 4096));
        assert!(row.covered_pixels >= 1024);
        assert!(!row.is_non_degenerate(1024, 1024), "the triangle floor must bind on its own");
    }
}
