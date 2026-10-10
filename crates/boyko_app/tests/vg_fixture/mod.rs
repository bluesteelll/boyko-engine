//! **VG-R0 rung R0c — the census's procedural sensitivity control.**
//!
//! A grid of ISOLATED right triangles on a plane perpendicular to the view axis, whose
//! screen-space pixel count is analytically known. R0c gate (b) asserts the census's modal bucket
//! IS that analytic bucket, and gate (c) cross-checks the covered-pixel total against
//! `sv0_oracle::rasterize`.
//!
//! # Why isolated triangles rather than a quad grid
//!
//! The obvious fixture — a grid of quads, each split by a diagonal — is WRONG for a modal-bucket
//! assertion, and the arithmetic is worth keeping because it is not obvious. A `C x C` quad split
//! corner-to-corner does not rasterize into two equal halves: the `C` pixel centres lying exactly
//! ON the diagonal all go to ONE triangle by the fill rule, so the two halves are
//! `C(C+1)/2` and `C(C-1)/2`. At `C = 8` that is 36 and 28 — buckets 5 and 4. The mode is then a
//! TIE between two buckets at half the triangles each, resolved by the reducer's documented
//! lower-bucket rule, and the "analytic bucket" the gate compares against is a fiction.
//!
//! Isolated triangles have no shared edge, so every triangle in the fixture rasterizes to the SAME
//! count and the mode is unambiguous.
//!
//! # The quarter-pixel offset, which is the difference between a gate and a coincidence
//!
//! An isolated right triangle with legs `L` at exact pixel-grid corners has its hypotenuse running
//! along `x + y = integer`, and pixel centres sit at `(i+0.5, j+0.5)`, so `L` centres lie EXACTLY
//! ON that edge every time. Whether they are covered is then decided by the fill rule, and the
//! covered count is `L(L-1)/2` or `L(L+1)/2` depending on who rasterises.
//!
//! ⚠️ **That is not a hypothetical.** The first run of this fixture measured the GPU at exactly
//! `1764 x 45` and `sv0_oracle` at exactly `1764 x 55` — the two ends of that band, an 18.2%
//! disagreement against a pre-registered 2% tolerance. Both rasterisers were right; the FIXTURE was
//! pathological, because it put every one of its edges on the sample lattice.
//!
//! The fix is geometric, not a loosened tolerance (the tolerance is frozen and read by name for
//! exactly this reason). Offsetting the grid by [`SUBPIXEL_OFFSET_PX`] = 1/4 pixel makes the
//! hypotenuse intercept a HALF-integer while centre sums `i + j + 1` are integers, so no centre can
//! ever lie on it — and the vertical/horizontal legs move off the lattice too. The covered count
//! becomes a single exact number, `L(L+1)/2`, with no fill rule left to consult.

// Shared by the census worker (which needs `mesh`) and the gate driver (which needs the analytic
// arithmetic); neither uses every item, exactly like the `sv0_oracle` / `sv0_scene` siblings.
#![allow(dead_code)]

use boyko_render::mesh::Vertex;

/// Distance from the eye to the fixture plane, in world units.
pub const CAMERA_DISTANCE: f32 = 10.0;

/// Vertical field of view: 90 degrees, so `tan(fov_y / 2) == 1` exactly and the visible half-height
/// at the plane is exactly [`CAMERA_DISTANCE`]. That is what makes the world-to-pixel factor a
/// clean power-of-two-friendly ratio rather than a transcendental one.
pub const FOV_Y: f32 = core::f32::consts::FRAC_PI_2;

/// The extent the fixture's pixel dimensions are DEFINED at — `[census].resolution_ladder`'s rung
/// 0, which is also the extent gates (b) and (c) are scoped to.
pub const REFERENCE_EXTENT: u32 = 512;

/// Pixels per world unit at [`REFERENCE_EXTENT`]: `H / (2 * D)`. A plane perpendicular to the view
/// axis projects under an EXACT uniform scale (every point shares one `clip.w`), so this single
/// factor is the whole projection — no per-triangle foreshortening, which is what makes the fixture
/// analytic at all.
pub const PX_PER_WORLD: f32 = REFERENCE_EXTENT as f32 / (2.0 * CAMERA_DISTANCE);

/// The grid's sub-pixel offset, in pixels at [`REFERENCE_EXTENT`]. See the module doc: a quarter
/// pixel puts every edge off the sample-centre lattice, which is what makes the covered count a
/// single number instead of a fill-rule-dependent band. `0.25 / 25.6 = 5/512`, exact in binary.
pub const SUBPIXEL_OFFSET_PX: f32 = 0.25;

/// One frozen fixture parameterisation.
#[derive(Clone, Copy, Debug)]
pub struct Fixture {
    /// The right triangle's leg length, in pixels at [`REFERENCE_EXTENT`].
    pub leg_px: f32,
    /// The grid cell's pitch, in pixels at [`REFERENCE_EXTENT`]. Strictly greater than `leg_px`, so
    /// no two triangles touch.
    pub cell_px: f32,
    /// Cells per side.
    pub grid: u32,
    /// Emit at most this many triangles — the `(c')` mutation's lever, and the ONLY difference
    /// between [`BASE`] and [`STARVED`].
    pub triangle_cap: Option<u32>,
}

/// The census fixture proper. 42 x 42 = 1764 triangles, above the 1024 floors with margin; a
/// 504-pixel span inside the 512-pixel reference viewport; 10-pixel legs, whose 45-or-55 covered
/// count sits in the interior of bucket 5.
pub const BASE: Fixture =
    Fixture { leg_px: 10.0, cell_px: 12.0, grid: 42, triangle_cap: None };

/// Gate (b)'s red mutation: the SAME fixture subdivided 4x — four times the triangles, each a
/// quarter of the area (halved legs), over the SAME 504-pixel span. The modal bucket must fall by
/// exactly two.
pub const SUBDIVIDED: Fixture =
    Fixture { leg_px: 5.0, cell_px: 6.0, grid: 84, triangle_cap: None };

/// Gate (c')'s red mutation: [`BASE`] with 31 triangles instead of 1764.
///
/// The triangle SIZE is untouched, which is what makes the mutation isolate — the analytic bucket
/// stays 5, so (b) is green; the oracle rasterises the same 31 triangles, so (c) agrees; the ladder
/// and extents are untouched, so (d) is green; and (a)'s pins render unarmed frames.
///
/// ⚠️ It reds (c') through the VISIBLE-TRIANGLE floor alone. `covered = 31 * 45 = 1395` still
/// clears `min_covered_pixels`, and no mutation of this fixture can red the covered floor ALONE:
/// `covered = visible_tris * pixels_per_triangle`, and (b) pins the second factor, so with the
/// triangle count above its own floor the pixel floor is IMPLIED. The pixel floor's independent red
/// is structurally unavailable here — the same disposition R0c(a) carries, reached from the other
/// side. It is covered where it IS reachable: `vg_census`'s reducer tests drive one huge triangle
/// (clears pixels, fails triangles) and a sentinel-only readback (fails both).
pub const STARVED: Fixture =
    Fixture { leg_px: 10.0, cell_px: 12.0, grid: 42, triangle_cap: Some(31) };

impl Fixture {
    /// Triangles this fixture emits.
    pub fn triangle_count(&self) -> u32 {
        let full = self.grid * self.grid;
        match self.triangle_cap {
            Some(cap) => cap.min(full),
            None => full,
        }
    }

    /// The pixels ONE triangle covers at [`REFERENCE_EXTENT`] — a single exact number, because
    /// [`SUBPIXEL_OFFSET_PX`] leaves no centre on any edge.
    ///
    /// Centres `(i+0.5, j+0.5)` with `i, j >= 0` lie inside the offset triangle iff
    /// `i + j + 1 < L + 2 * 0.25`, i.e. `i + j <= L - 1`, and there are `L(L+1)/2` such pairs.
    pub fn analytic_pixels(&self) -> u64 {
        let l = self.leg_px as u64;
        l * (l + 1) / 2
    }

    /// The power-of-two bucket gate (b) compares the census's mode against.
    pub fn analytic_bucket(&self) -> u32 {
        self.analytic_pixels().ilog2()
    }

    /// The grid's full span in pixels at [`REFERENCE_EXTENT`].
    pub fn span_px(&self) -> f32 {
        self.grid as f32 * self.cell_px
    }

    /// The fixture mesh: `triangle_count()` isolated right triangles on the plane `z == 0`, facing
    /// the eye at `+z`, centred on the view axis.
    ///
    /// Pixel-space `y` grows DOWNWARD (screen convention) and world `y` grows upward, so the
    /// conversion negates it. Every coordinate is an exact binary fraction by construction: the
    /// frozen `leg_px` / `cell_px` divided by [`PX_PER_WORLD`] = 25.6 yield 25/64, 15/32, 25/128 and
    /// 15/64 — so the fixture carries no floating-point rounding of its own into the measurement.
    pub fn mesh(&self) -> (Vec<Vertex>, Vec<u32>) {
        let n = self.triangle_count() as usize;
        let mut verts = Vec::with_capacity(n * 3);
        let mut idx = Vec::with_capacity(n * 3);

        let half = self.span_px() * 0.5 - SUBPIXEL_OFFSET_PX;
        let to_world = |px: f32, py: f32| -> [f32; 3] { [px / PX_PER_WORLD, -py / PX_PER_WORLD, 0.0] };
        const NORMAL: [f32; 3] = [0.0, 0.0, 1.0];
        const COLOR: [f32; 4] = [0.75, 0.75, 0.78, 1.0];

        let mut emitted = 0u32;
        'grid: for row in 0..self.grid {
            for col in 0..self.grid {
                if emitted == self.triangle_count() {
                    break 'grid;
                }
                let x0 = -half + col as f32 * self.cell_px;
                let y0 = -half + row as f32 * self.cell_px;
                // Counter-clockwise as seen from +z (the eye): (x0,y0) -> (x0,y0+L) -> (x0+L,y0)
                // in pixel space is CCW once `y` is flipped into world space.
                let corners = [
                    to_world(x0, y0),
                    to_world(x0, y0 + self.leg_px),
                    to_world(x0 + self.leg_px, y0),
                ];
                let base = verts.len() as u32;
                for c in corners {
                    verts.push(Vertex::new(c, NORMAL, COLOR));
                }
                idx.extend_from_slice(&[base, base + 1, base + 2]);
                emitted += 1;
            }
        }
        (verts, idx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_pixel_centre_can_lie_on_a_fixture_edge() {
        // The property the quarter-pixel offset exists for, checked rather than asserted in prose.
        // Sample centres are at half-integers; an edge that passes through one leaves the covered
        // count to the fill rule, which is what produced an 18.2% GPU-vs-oracle disagreement on the
        // unoffset first draft.
        for f in [BASE, SUBDIVIDED, STARVED] {
            let half = f.span_px() * 0.5 - SUBPIXEL_OFFSET_PX;
            for cell in 0..f.grid {
                let x0 = -half + cell as f32 * f.cell_px;
                // Leg edges sit at `x0` / `y0`; centres sit at `k + 0.5`. Distinct iff the
                // fractional part of the edge is not 1/2.
                assert_ne!(x0.rem_euclid(1.0), 0.5, "a leg edge of cell {cell} is on the lattice");
                // The hypotenuse is `x + y = 2*x0 + L`; centre sums are `i + j + 1`, integers.
                let intercept = 2.0 * x0 + f.leg_px;
                assert_ne!(
                    intercept.fract(),
                    0.0,
                    "the hypotenuse of cell {cell} has an INTEGER intercept {intercept}, so pixel \
                     centres lie exactly on it and the fill rule decides the covered count"
                );
            }
        }
    }

    #[test]
    fn subdivision_moves_the_analytic_bucket_down_by_exactly_two() {
        // Gate (b)'s red mutation, as arithmetic. "The number changed" is not a gate; the required
        // direction AND magnitude is.
        assert_eq!(BASE.analytic_bucket(), 5);
        assert_eq!(SUBDIVIDED.analytic_bucket(), 3);
        assert_eq!(BASE.analytic_bucket() - SUBDIVIDED.analytic_bucket(), 2);
        // 4x the triangles is what "subdivided 4x" means, and it must hold over the SAME span --
        // a subdivision that also grew the grid would change coverage as well as size.
        assert_eq!(SUBDIVIDED.triangle_count(), 4 * BASE.triangle_count());
        assert_eq!(SUBDIVIDED.span_px(), BASE.span_px());
    }

    #[test]
    fn the_base_fixture_clears_both_non_degeneracy_floors_with_margin() {
        assert!(BASE.triangle_count() as u64 >= 1024, "visible_tris floor");
        assert!(
            BASE.triangle_count() as u64 * BASE.analytic_pixels() >= 1024,
            "covered_pixels floor"
        );
    }

    #[test]
    fn the_starved_fixture_reds_only_the_triangle_floor() {
        // The mutation must ISOLATE: it reds (c') and leaves (b) green.
        assert!((STARVED.triangle_count() as u64) < 1024);
        assert_eq!(
            STARVED.analytic_bucket(),
            BASE.analytic_bucket(),
            "the starved fixture must keep the base's triangle SIZE, or it reds (b) too"
        );
        assert!(
            STARVED.triangle_count() as u64 * STARVED.analytic_pixels() >= 1024,
            "and it must NOT red the pixel floor -- that arm is structurally unreachable here, \
             which is recorded on STARVED rather than faked with a second lever"
        );
    }

    #[test]
    fn the_grid_fits_inside_the_reference_viewport() {
        for f in [BASE, SUBDIVIDED, STARVED] {
            assert!(
                f.span_px() < REFERENCE_EXTENT as f32,
                "a grid clipped by the viewport loses triangles, which would show up as a \
                 coverage disagreement in (c) rather than as the fixture bug it is"
            );
        }
    }

    #[test]
    fn every_fixture_coordinate_is_an_exact_binary_fraction() {
        // Not decoration: an inexact world coordinate puts triangle edges at fractional pixel
        // positions, and the covered count stops being L(L±1)/2 at all.
        for f in [BASE, SUBDIVIDED, STARVED] {
            for px in [f.leg_px, f.cell_px, f.span_px(), SUBPIXEL_OFFSET_PX] {
                let w = px / PX_PER_WORLD;
                assert_eq!(
                    w * PX_PER_WORLD,
                    px,
                    "{px} px does not round-trip through the world-space conversion"
                );
            }
        }
    }

    #[test]
    fn the_mesh_emits_one_triangle_per_cell_up_to_the_cap() {
        for f in [BASE, SUBDIVIDED, STARVED] {
            let (verts, idx) = f.mesh();
            assert_eq!(idx.len(), f.triangle_count() as usize * 3);
            assert_eq!(verts.len(), f.triangle_count() as usize * 3);
        }
    }
}
