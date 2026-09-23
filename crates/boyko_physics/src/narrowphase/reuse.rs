//! Contact reuse (L9, `docs/physics/perf-campaign/levers/L9-contact-reuse/02-DESIGN-REV1.md`).
//!
//! This module holds the per-row orientation frame (D2): each box row's world axes, written once
//! per step so that a box pair reads two frames instead of converting two quaternions. The frame
//! serves the exact fast path (L9a) now; the reuse records, the criterion and the refresh (L9b,
//! D4–D8) are added here in commit C3, and the hit path reads the same frames.
//!
//! # The frame fill (D2, ruling O3)
//!
//! [`fill_row_frames`] runs on the calling thread at the entry of each narrowphase path
//! (`narrowphase_serial`, and `dispatch::prepare` before any chunk runs), so it runs once per step
//! and for every direct caller of either path. It writes the frame of every BOX row and skips every
//! other row, because no pair reads a non-box row through a frame. It writes nothing and returns
//! `None` in two cases, and the step's box pairs then build their two frames per pair with the same
//! [`RowFrame::of`]:
//!
//! * the step has fewer candidate pairs than half its rows. The fill costs one conversion per box
//!   row, and the pairs save at most two per pair, so on such a sparse step the fill would cost more
//!   than it saves (ruling O3: the fill skips the rows no pair reads);
//! * the step has more rows than the column's reserve (the stage ceiling's rule in
//!   `narrowphase/dispatch.rs`: a resize past it panics).
//!
//! # Why a frame gives today's bits (Lemma L9-L2, second half)
//!
//! [`RowFrame::of`] is the axis computation `Obb::new` performed per pair, moved: the three columns
//! of `Mat3::from_quat(rotation)`. The fill and the per-pair path call it on the same gathered
//! rotation, so `Obb::from_frame` receives the bits `Obb::new` computed. The arithmetic is IEEE
//! `f32` add and multiply only, and Rust does not contract them into FMA.
//!
//! ZERO `unsafe`; the column is a kernel `ScratchColumn` owned by
//! [`Manifolds`](crate::resources::Manifolds) (principle 0), grown in place, no per-step heap
//! allocation.

use boyko_ecs::ecs::core::component::scratch::ScratchColumn;

use crate::components::ColliderShape;
use crate::math::{Mat3, Quat, Vec3};
use crate::resources::BodyState;

/// A box row's orientation resolved into world axes for one step (D2): `axes[i]` is the world
/// direction of the box's local axis `i`, column `i` of `Mat3::from_quat(rotation)`.
///
/// 36 B, `#[repr(C)]`, POD. The design's 40 B layout adds the row's bounding radius in C3, together
/// with its first reader (the reuse criterion's choice of the larger body, D4).
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct RowFrame {
    /// The world directions of local axes x, y and z.
    pub(crate) axes: [Vec3; 3],
}

const _: () = assert!(size_of::<RowFrame>() == 36);

impl RowFrame {
    /// The value of a slot the fill has not written this step (a grown slot, a non-box row). No
    /// pair reads it: only a box-box pair reads a frame, and only its own two box rows'.
    pub(crate) const UNWRITTEN: Self = Self { axes: [Vec3::ZERO; 3] };

    /// The frame of a body with orientation `rotation`: the columns of `Mat3::from_quat`.
    #[inline]
    pub(crate) fn of(rotation: Quat) -> Self {
        let r = Mat3::from_quat(rotation);
        // Column `i` of R is the world direction of local axis `i`. With row-major storage,
        // column `i` is `(rows[0][i], rows[1][i], rows[2][i])`.
        Self {
            axes: [
                Vec3::new(r.rows[0].x, r.rows[1].x, r.rows[2].x),
                Vec3::new(r.rows[0].y, r.rows[1].y, r.rows[2].y),
                Vec3::new(r.rows[0].z, r.rows[1].z, r.rows[2].z),
            ],
        }
    }
}

/// Writes the frame of every box row of `bodies` into `frames` and returns the column's read slice,
/// one slot per row; or writes nothing and returns `None` when the step's box pairs build their
/// frames per pair instead (module docs, "The frame fill"). `n_pairs` is the step's candidate pair
/// count.
///
/// Serial, on the calling thread, before any pair of the step is collided; the returned slice is
/// read-only while the pairs run, on any thread.
pub(crate) fn fill_row_frames<'a>(
    frames: &'a mut ScratchColumn<RowFrame>,
    bodies: &[BodyState],
    n_pairs: usize,
) -> Option<&'a [RowFrame]> {
    let rows = bodies.len();
    if n_pairs.saturating_mul(2) < rows || rows > frames.capacity() {
        return None;
    }
    {
        // The view publishes its length on drop, before the read slice below is taken.
        let mut view = frames.build_view();
        view.resize(rows, RowFrame::UNWRITTEN);
        for (frame, body) in view.as_mut_slice().iter_mut().zip(bodies) {
            if matches!(body.shape, ColliderShape::Box { .. }) {
                *frame = RowFrame::of(body.rotation);
            }
        }
    }
    Some(frames.as_read_slice())
}
