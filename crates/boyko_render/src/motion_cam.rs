//! HW-RT Rung 3b — the camera view-proj carry for temporal shadow-vis motion vectors.
//!
//! The temporal denoiser reprojects each pixel's shadow term into the previous frame. That
//! reprojection needs the camera's CURRENT and PREVIOUS `view_proj` in the shaders that write
//! motion vectors (the raster gbuffer VS for mesh pixels, the marcher/VIS front-matter for SDF
//! pixels). No shader has a camera `view_proj` today — the camera reaches shaders only as the
//! decomposed basis (eye + orthonormal axes + FOV), and the sole `view_proj` matrices in flight
//! are LIGHT matrices (CSM cascades / punctual faces). This module supplies the missing camera
//! matrices as a small dedicated UBO.
//!
//! # Principle 0 / ECS-native
//!
//! [`MotionCamState`] is a `#[derive(Resource)]` singleton — the persisted previous-frame
//! `view_proj` lives in the engine's own storage, NOT a host-side `static mut` or a side field.
//! Each frame the host calls [`MotionCamState::advance`], which returns the [`MotionCam`] to
//! upload (this frame's `cur` + last frame's persisted `prev`) and stores `cur` as next frame's
//! `prev`.
//!
//! # Majorness (I-O1)
//!
//! Both matrices are built by [`marcher_view_proj_rows`](crate::view::marcher_view_proj_rows)
//! (ROW-MAJOR math rows) and uploaded COLUMN-MAJOR by [`MotionCam::to_bytes`] — the SAME
//! transpose the CSM/punctual light matrices use (`gpu_scene.rs` cascade/face upload). A
//! convention mismatch would smear every reprojected pixel, so the two producers and this
//! upload are pinned to one construction.
//!
//! # HW-RT-walled
//!
//! The module is compiled only under `#[cfg(feature = "hwrt")]` (gated at the `mod` in `lib.rs`)
//! — a `not(hwrt)` build carries none of it.

use boyko_macros::Resource;

/// The byte size of the [`MotionCam`] UBO — two `float4x4` (`cur`, `prev`), 128 B.
pub const MOTION_CAM_UBO_BYTES: usize = 128;

/// The camera view-proj pair delivered to the motion-vector shaders: this frame's `cur` and
/// last frame's `prev`, both marcher-aligned (see the module docs).
///
/// Stored as ROW-MAJOR math rows (`[[f32; 4]; 4]` where `[row]` is a matrix row — the form
/// [`marcher_view_proj_rows`](crate::view::marcher_view_proj_rows) returns); [`to_bytes`]
/// performs the column-major transpose for the std140 UBO upload.
///
/// [`to_bytes`]: MotionCam::to_bytes
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MotionCam {
    /// This frame's marcher-aligned proj·view (ROW-MAJOR math rows).
    pub cur_view_proj: [[f32; 4]; 4],
    /// Last frame's marcher-aligned proj·view (ROW-MAJOR math rows). Equals `cur_view_proj`
    /// on the first temporal frame, so the initial motion vector is exactly zero.
    pub prev_view_proj: [[f32; 4]; 4],
}

impl MotionCam {
    /// Packs the pair into the 128-byte UBO the motion-vector shaders read as
    /// `{ float4x4 cur_view_proj; float4x4 prev_view_proj; }`.
    ///
    /// Each matrix is written COLUMN-MAJOR (`out[(col*4 + row)*4]` = `m[row][col]`) — the
    /// std140 `float4x4` layout, matching the CSM/punctual light-matrix upload (I-O1). `cur`
    /// occupies bytes 0..64, `prev` bytes 64..128.
    #[inline]
    #[must_use]
    pub fn to_bytes(&self) -> [u8; MOTION_CAM_UBO_BYTES] {
        let mut out = [0u8; MOTION_CAM_UBO_BYTES];
        write_col_major(&mut out[0..64], &self.cur_view_proj);
        write_col_major(&mut out[64..128], &self.prev_view_proj);
        out
    }
}

/// Writes a ROW-MAJOR `[[f32;4];4]` into `dst` (64 B) as a COLUMN-MAJOR std140 `float4x4`:
/// math element `m[row][col]` lands at float index `col*4 + row` (the transpose).
#[inline]
fn write_col_major(dst: &mut [u8], m: &[[f32; 4]; 4]) {
    debug_assert_eq!(dst.len(), 64, "invariant: one float4x4 is 64 bytes");
    for (row_idx, row) in m.iter().enumerate() {
        for (col_idx, value) in row.iter().enumerate() {
            let at = (col_idx * 4 + row_idx) * 4;
            dst[at..at + 4].copy_from_slice(&value.to_le_bytes());
        }
    }
}

/// The ECS-native carry of the previous-frame camera `view_proj` (Principle 0).
///
/// `None` until the first temporal frame; [`advance`](Self::advance) seeds it with the current
/// matrix so the first frame's `prev == cur` (zero motion — no spurious reprojection before any
/// history exists).
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq)]
pub struct MotionCamState {
    /// The persisted previous-frame marcher-aligned proj·view (ROW-MAJOR math rows), or `None`
    /// before the first temporal frame.
    pub prev_view_proj: Option<[[f32; 4]; 4]>,
}

impl MotionCamState {
    /// Advances one frame: returns the [`MotionCam`] to upload (`cur` + the persisted `prev`,
    /// or `cur` itself on the first frame) and stores `cur` as next frame's `prev`.
    ///
    /// The single seam that both consumes and updates the persisted matrix — so the persist
    /// discipline (prev is exactly last frame's cur) lives in one testable place, not smeared
    /// across the host frame loop.
    #[inline]
    pub fn advance(&mut self, cur_view_proj: [[f32; 4]; 4]) -> MotionCam {
        let prev_view_proj = self.prev_view_proj.unwrap_or(cur_view_proj);
        self.prev_view_proj = Some(cur_view_proj);
        MotionCam { cur_view_proj, prev_view_proj }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A distinct, asymmetric test matrix (every entry unique) so a transpose bug is visible.
    fn mat_a() -> [[f32; 4]; 4] {
        [
            [1.0, 2.0, 3.0, 4.0],
            [5.0, 6.0, 7.0, 8.0],
            [9.0, 10.0, 11.0, 12.0],
            [13.0, 14.0, 15.0, 16.0],
        ]
    }

    fn mat_b() -> [[f32; 4]; 4] {
        [
            [-1.0, -2.0, -3.0, -4.0],
            [-5.0, -6.0, -7.0, -8.0],
            [-9.0, -10.0, -11.0, -12.0],
            [-13.0, -14.0, -15.0, -16.0],
        ]
    }

    fn read_f32(bytes: &[u8], i: usize) -> f32 {
        f32::from_le_bytes([bytes[i * 4], bytes[i * 4 + 1], bytes[i * 4 + 2], bytes[i * 4 + 3]])
    }

    /// `to_bytes` transposes ROW-MAJOR rows into a COLUMN-MAJOR std140 `float4x4`: float index
    /// `col*4 + row` must equal `m[row][col]`. Guards the I-O1 majorness pin (a smear-causing
    /// transpose bug fails here, not in a GPU capture).
    #[test]
    fn to_bytes_is_column_major_and_placed() {
        let cam = MotionCam { cur_view_proj: mat_a(), prev_view_proj: mat_b() };
        let bytes = cam.to_bytes();
        assert_eq!(bytes.len(), MOTION_CAM_UBO_BYTES);
        for col in 0..4 {
            for row in 0..4 {
                // cur @ floats 0..16
                assert_eq!(read_f32(&bytes, col * 4 + row), mat_a()[row][col]);
                // prev @ floats 16..32 (byte offset 64)
                assert_eq!(read_f32(&bytes, 16 + col * 4 + row), mat_b()[row][col]);
            }
        }
    }

    /// First frame: no history ⇒ `prev == cur` ⇒ the uploaded pair yields a zero motion vector
    /// (`cur` and `prev` bytes identical). This is the disocclusion-safe seed.
    #[test]
    fn advance_first_frame_prev_equals_cur() {
        let mut state = MotionCamState::default();
        assert_eq!(state.prev_view_proj, None);
        let cam = state.advance(mat_a());
        assert_eq!(cam.cur_view_proj, mat_a());
        assert_eq!(cam.prev_view_proj, mat_a(), "first frame prev must equal cur (MV = 0)");
        let bytes = cam.to_bytes();
        assert_eq!(&bytes[0..64], &bytes[64..128], "cur/prev byte-identical ⇒ MV ≡ 0");
        assert_eq!(state.prev_view_proj, Some(mat_a()), "cur persisted for next frame");
    }

    /// Second frame: `prev` is EXACTLY the first frame's `cur` (the persist contract), so a
    /// moving camera produces a non-zero, correctly-signed reprojection.
    #[test]
    fn advance_second_frame_prev_is_previous_cur() {
        let mut state = MotionCamState::default();
        let _ = state.advance(mat_a());
        let cam2 = state.advance(mat_b());
        assert_eq!(cam2.cur_view_proj, mat_b());
        assert_eq!(cam2.prev_view_proj, mat_a(), "prev must be last frame's cur");
        assert_eq!(state.prev_view_proj, Some(mat_b()));
    }

    /// Static camera (same matrix every frame): after the seed the pair stays `cur == prev`, so
    /// the convergence anchor (motion vector ≡ 0 when nothing moves) holds across frames.
    #[test]
    fn advance_static_camera_stays_zero_motion() {
        let mut state = MotionCamState::default();
        for _ in 0..8 {
            let cam = state.advance(mat_a());
            assert_eq!(cam.cur_view_proj, cam.prev_view_proj);
        }
    }
}
