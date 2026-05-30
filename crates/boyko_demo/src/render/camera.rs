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
/// Holds a column-major 4x4 world->NDC matrix. `#[repr(C)]` matches WGSL's
/// `mat4x4<f32>` uniform layout (four 16 B columns).
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct CameraUniform {
    /// Column-major world->NDC transform consumed as `mat4x4<f32>` in WGSL.
    pub view_proj: [[f32; 4]; 4],
}

/// Expected size of [`CameraUniform`] (one `mat4` = 64 B).
pub const CAMERA_UNIFORM_SIZE: usize = 64;

const _: () = assert!(size_of::<CameraUniform>() == CAMERA_UNIFORM_SIZE);
const _: () = assert!(align_of::<CameraUniform>() == 4);

impl CameraUniform {
    /// Identity transform; the initial uniform value before the first `prepare`
    /// rebuilds it from the real viewport.
    #[inline]
    pub const fn identity() -> Self {
        Self {
            view_proj: [
                [1.0, 0.0, 0.0, 0.0],
                [0.0, 1.0, 0.0, 0.0],
                [0.0, 0.0, 1.0, 0.0],
                [0.0, 0.0, 0.0, 1.0],
            ],
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
    pub fn ortho_fit(
        viewport_w: f32,
        viewport_h: f32,
        half_world_w: f32,
        half_world_h: f32,
    ) -> Self {
        // Guard against a zero/negative viewport (a collapsed panel) so we never
        // divide by zero; fall back to identity.
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
