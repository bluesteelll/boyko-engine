//! 2D orthographic camera uniform: world space -> normalized device coords
//! (plan §5.3 / §5.5).
//!
//! The projection is rebuilt each frame from the panel pixel rect so the world
//! stays correctly proportioned on resize (plan §5.5 / M4). World bounds map onto
//! the viewport with an aspect-ratio correction derived from the viewport's
//! width/height so squares stay square regardless of window shape.

use bytemuck::{Pod, Zeroable};

/// GPU-side camera uniform (bind group 0, binding 0).
///
/// Holds a column-major 4x4 world->NDC matrix plus the Phase-20.1
/// interpolation alpha (D7). `#[repr(C)]` matches WGSL's uniform layout: the
/// `mat4x4<f32>` (four 16 B columns) + `alpha` round the WGSL struct size up
/// to align(16) = 80 B, mirrored here with an explicit padding field.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct CameraUniform {
    /// Column-major world->NDC transform consumed as `mat4x4<f32>` in WGSL.
    pub view_proj: [[f32; 4]; 4],
    /// Interpolation alpha ∈ [0, 1) for the GPU lerp
    /// `mix(prev_pos, pos, alpha)` (Phase 20.1 D1/D7); `1.0` in
    /// [`Self::identity`] (snap semantics).
    pub alpha: f32,
    /// Explicit uniform-layout padding mirroring WGSL's align(16) size
    /// round-up. An explicit zeroed field (rather than relying on implicit
    /// padding) keeps the uploaded uniform bytes deterministic — implicit
    /// padding bytes are uninitialized and `cast`-ing them to the GPU would
    /// upload garbage (Phase 20.1 ★n8).
    pub _pad: [f32; 3],
}

/// Expected size of [`CameraUniform`] (`mat4` 64 B + alpha 4 B + pad 12 B = 80 B,
/// the WGSL uniform struct size rounded to align(16)).
pub const CAMERA_UNIFORM_SIZE: usize = 80;

const _: () = assert!(size_of::<CameraUniform>() == CAMERA_UNIFORM_SIZE);
const _: () = assert!(align_of::<CameraUniform>() == 4);

impl CameraUniform {
    /// Identity transform; the initial uniform value before the first `prepare`
    /// rebuilds it from the real viewport.
    ///
    /// `alpha` is `1.0` — snap semantics: `mix(prev, pos, 1.0) == pos`, so an
    /// identity frame renders the latest packed position with no lerp. This is
    /// also what the degenerate-viewport guard in [`Self::ortho_fit`] returns
    /// (Phase 20.1 ★R1-2 — nothing renders in a zero viewport anyway).
    #[inline]
    pub const fn identity() -> Self {
        Self {
            view_proj: [
                [1.0, 0.0, 0.0, 0.0],
                [0.0, 1.0, 0.0, 0.0],
                [0.0, 0.0, 1.0, 0.0],
                [0.0, 0.0, 0.0, 1.0],
            ],
            alpha: 1.0,
            _pad: [0.0; 3],
        }
    }

    /// Builds an orthographic world->NDC matrix that fits the world rectangle
    /// `[-half_world_w, half_world_w] x [-half_world_h, half_world_h]` into the
    /// viewport while preserving aspect ratio (letterboxing the shorter axis).
    ///
    /// `viewport_w` / `viewport_h` are the panel size in physical pixels. The
    /// world half-extents define how much of the world is visible at unit zoom;
    /// the larger viewport axis is widened so world squares render as screen
    /// squares.
    ///
    /// The projection keeps the standard +Y-up NDC: our own quad geometry is
    /// symmetric about the origin, so no screen-space (y-down) flip is needed.
    ///
    /// `alpha` is the Phase-20.1 interpolation alpha (D7), stored verbatim into
    /// the uniform on the non-degenerate path; the shader lerps
    /// `mix(prev_pos, pos, alpha)` per vertex.
    pub fn ortho_fit(
        viewport_w: f32,
        viewport_h: f32,
        half_world_w: f32,
        half_world_h: f32,
        alpha: f32,
    ) -> Self {
        // Guard against a zero/negative viewport (a collapsed panel) so we never
        // divide by zero; fall back to identity. NOTE (★R1-2): identity carries
        // alpha = 1.0 (snap), not the caller's alpha — documented semantics for
        // a zero viewport, where nothing renders anyway.
        if viewport_w <= 0.0 || viewport_h <= 0.0 || half_world_w <= 0.0 || half_world_h <= 0.0 {
            return Self::identity();
        }

        let viewport_aspect = viewport_w / viewport_h;
        let world_aspect = half_world_w / half_world_h;

        // Expand the world extent on whichever axis is "too tall/wide" relative to
        // the viewport so the world is fully contained and proportions hold.
        let (ext_x, ext_y) = if viewport_aspect >= world_aspect {
            (half_world_h * viewport_aspect, half_world_h)
        } else {
            (half_world_w, half_world_w / viewport_aspect)
        };

        // Orthographic scale: world unit -> NDC unit on each axis.
        let sx = 1.0 / ext_x;
        let sy = 1.0 / ext_y;

        // Column-major: each inner array is a column. Centered at the origin (no
        // translation) so the camera looks at world (0, 0).
        Self {
            view_proj: [
                [sx, 0.0, 0.0, 0.0],
                [0.0, sy, 0.0, 0.0],
                [0.0, 0.0, 1.0, 0.0],
                [0.0, 0.0, 0.0, 1.0],
            ],
            alpha,
            _pad: [0.0; 3],
        }
    }

    /// Returns the visible world half-extents `(ext_x, ext_y)` for a viewport of
    /// the given pixel size — the same values [`Self::ortho_fit`] uses to build
    /// the projection. Shared so the inverse mapping ([`Self::screen_to_world`])
    /// can never drift from the forward projection. Returns `None` for a
    /// degenerate viewport.
    pub fn world_extents(
        viewport_w: f32,
        viewport_h: f32,
        half_world_w: f32,
        half_world_h: f32,
    ) -> Option<(f32, f32)> {
        if viewport_w <= 0.0 || viewport_h <= 0.0 || half_world_w <= 0.0 || half_world_h <= 0.0 {
            return None;
        }
        let viewport_aspect = viewport_w / viewport_h;
        let world_aspect = half_world_w / half_world_h;
        let extents = if viewport_aspect >= world_aspect {
            (half_world_h * viewport_aspect, half_world_h)
        } else {
            (half_world_w, half_world_w / viewport_aspect)
        };
        Some(extents)
    }

    /// Maps a pointer position `(px, py)` given in pixels relative to the scene
    /// rect's top-left corner (egui's +Y-down convention) to world coordinates,
    /// inverting [`Self::ortho_fit`]. `viewport_w`/`viewport_h` are the rect's
    /// pixel size; the world half-extents must match the projection's. Returns
    /// `None` for a degenerate viewport.
    pub fn screen_to_world(
        px: f32,
        py: f32,
        viewport_w: f32,
        viewport_h: f32,
        half_world_w: f32,
        half_world_h: f32,
    ) -> Option<[f32; 2]> {
        let (ext_x, ext_y) =
            Self::world_extents(viewport_w, viewport_h, half_world_w, half_world_h)?;
        // Pixel -> NDC in [-1, 1]; flip Y because screen-space grows downward
        // while world/NDC grows upward.
        let ndc_x = px / viewport_w * 2.0 - 1.0;
        let ndc_y = 1.0 - py / viewport_h * 2.0;
        // NDC -> world: undo the orthographic scale (no translation, origin
        // centered).
        Some([ndc_x * ext_x, ndc_y * ext_y])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// T2 (Phase 20.1): the host mirror is exactly 80 B — the WGSL uniform
    /// struct size (mat4 64 + alpha 4, rounded to align(16)).
    #[test]
    fn camera_uniform_is_80_bytes() {
        assert_eq!(size_of::<CameraUniform>(), CAMERA_UNIFORM_SIZE);
        assert_eq!(CAMERA_UNIFORM_SIZE, 80);
    }

    /// T2: `identity()` carries snap semantics (`alpha == 1.0`).
    #[test]
    fn identity_alpha_is_one() {
        assert_eq!(CameraUniform::identity().alpha, 1.0);
    }

    /// T2: `ortho_fit` stores alpha verbatim on the non-degenerate path.
    #[test]
    fn ortho_fit_stores_alpha_verbatim() {
        let alpha = 0.4375_f32;
        let cam = CameraUniform::ortho_fit(800.0, 600.0, 100.0, 100.0, alpha);
        assert_eq!(cam.alpha.to_bits(), alpha.to_bits());
    }

    /// T2 (★R1-2): the degenerate-viewport guard returns identity, whose alpha
    /// is 1.0 (documented snap semantics — nothing renders in a zero viewport).
    #[test]
    fn ortho_fit_degenerate_viewport_returns_identity_alpha() {
        let cam = CameraUniform::ortho_fit(0.0, 600.0, 100.0, 100.0, 0.25);
        assert_eq!(cam.alpha, 1.0);
        assert_eq!(cam.view_proj, CameraUniform::identity().view_proj);
    }
}
