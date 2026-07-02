//! View wiring (S3): the conversion seam from the engine-derived
//! [`ViewUniform`](boyko_scene::ViewUniform) to the backend-specific view forms.
//!
//! Before S3 each backend hand-fed its own camera: the marcher's
//! [`CompositePushConstants`] perspective basis (eye + orthonormal basis + FOV)
//! and the demo's `CameraUniform.view_proj` (a viewport-fit ortho matrix).
//! [`ViewUniform`] is the engine's single derived view — the active ECS camera's
//! `Projection` + `GlobalTransform`, resolved each frame by
//! [`resolve_active_camera`](boyko_scene::resolve_active_camera). This module is
//! the **sanctioned bridge** a host consumes to turn that one view into each
//! backend's form ([`composite_from_view`] for the marcher push constants,
//! [`demo_view_proj_from_view`] for the raster `view_proj`), so no backend
//! reconstructs a view of its own.
//!
//! # Adoption status
//!
//! The bridge is the conversion path; the host-side adoption is staged. The
//! `boyko_render` integration tests drive the full
//! `resolve_active_camera` → [`ViewUniform`] → bridge path on a live ECS world
//! (proving it is the executed seam, not dead exports). Migrating the demo's
//! `CameraUniform::ortho_fit` call site and the `boyko_rhi_vulkan` composite
//! recording onto this bridge is a follow-up in their own crates (the low-level
//! `boyko_rhi_vulkan` backend cannot depend upward on the scene crate, so the
//! call must originate in a crate above `boyko_render`).
//!
//! # No-regression contract (perspective)
//!
//! The forward perspective camera (eye `(0,0,3)`, forward `(0,0,−1)`, right
//! `(1,0,0)`, up `(0,1,0)`, 60° FOV, identity rotation) makes
//! [`composite_perspective_from_view`] reproduce
//! [`CompositePushConstants::perspective`] exactly, because `ViewUniform`'s
//! basis lanes are `matrix3 · local_axis` and an identity rotation maps the
//! local axes to themselves. The marcher ORTHO path stays camera-basis-free:
//! an orthographic active camera routes to [`CompositePushConstants::ortho`]
//! verbatim, so the bit-frozen ORTHO golden is untouched.
//!
//! # The raster ortho path is NOT `CameraUniform::ortho_fit`
//!
//! [`demo_view_proj_from_view`] for an orthographic camera emits the **standard**
//! right-handed [`orthographic_rh`](boyko_math::Mat4::orthographic_rh) matrix
//! (fixed world half-extents from the camera's `Projection`, with a `[0,1]` depth
//! remap). The demo's legacy `CameraUniform::ortho_fit` is a different projection
//! — a *viewport-fit* scale (`diag(1/ext_x, 1/ext_y, 1, 1)`, letterboxed from the
//! pixel rect, depth left unscaled). The two are NOT equal; the bridge does not
//! reproduce `ortho_fit`. Migrating the demo onto the engine ortho is a
//! behavioural change (the visible world rect becomes the camera's
//! `half_height`/`aspect`, not the panel-fit), tracked as the demo follow-up
//! above, not a drop-in replacement.

use boyko_rhi_vulkan::compute::{CAM_MODE_ORTHO, CompositePushConstants};
use boyko_rhi_vulkan::swapchain::GBUFFER_PUSH_BYTES;
use boyko_scene::ViewUniform;

use boyko_math::Mat4;

/// Builds the marcher's PERSPECTIVE [`CompositePushConstants`] from a resolved
/// [`ViewUniform`] and a `w × h` extent.
///
/// The marcher consumes the decomposed camera (eye + orthonormal basis + FOV),
/// NOT `view_proj`, so this reads `ViewUniform`'s `camera_pos` / `cam_forward` /
/// `cam_right` / `cam_up` / `fov_y` lanes and forwards them to
/// [`CompositePushConstants::perspective`] (which packs `tan(fov_y/2)` into
/// `cam_forward.w` and the aspect into `cam_right.w`, exactly as before — the
/// struct and its lanes are unchanged; only the fill SOURCE moved to the engine
/// view).
///
/// For the prior forward camera this is byte-identical to the old hand-fed
/// `CompositePushConstants::perspective([0,0,3], [0,0,-1], [1,0,0], [0,1,0],
/// FRAC_PI_3, w, h)`.
#[inline]
pub fn composite_perspective_from_view(view: &ViewUniform, w: u32, h: u32) -> CompositePushConstants {
    let eye = view.camera_pos;
    let fwd = view.cam_forward;
    let right = view.cam_right;
    let up = view.cam_up;
    CompositePushConstants::perspective(
        [eye.x, eye.y, eye.z],
        [fwd.x, fwd.y, fwd.z],
        [right.x, right.y, right.z],
        [up.x, up.y, up.z],
        view.fov_y,
        w,
        h,
    )
}

/// Builds the marcher's [`CompositePushConstants`] from a resolved
/// [`ViewUniform`] and a `w × h` extent, selecting ORTHO vs PERSPECTIVE by the
/// view's projection.
///
/// An orthographic active camera carries `fov_y == 0.0` (the ortho sentinel set
/// by [`Projection::fov_y`](boyko_scene::Projection::fov_y)); it routes to the
/// bit-frozen [`CompositePushConstants::ortho`] golden path so an ORTHO golden
/// stays byte-exact. A perspective camera routes to
/// [`composite_perspective_from_view`].
#[inline]
pub fn composite_from_view(view: &ViewUniform, w: u32, h: u32) -> CompositePushConstants {
    // `fov_y == 0.0` is the orthographic sentinel (perspective FOVs are > 0). The
    // ORTHO fixture is camera-basis-free (the shader ignores it), so the frozen
    // `ortho(w, h)` layout is emitted verbatim — the golden stays byte-exact.
    if view.fov_y == 0.0 {
        let pc = CompositePushConstants::ortho(w, h);
        debug_assert_eq!(pc.camera_mode, CAM_MODE_ORTHO);
        pc
    } else {
        composite_perspective_from_view(view, w, h)
    }
}

/// Builds the 88-byte gbuffer-raster VERTEX push (`{ float4x4 view_proj; float4
/// cam_eye; uint base_instance; uint use_model_matrix }` —
/// [`GBUFFER_PUSH_BYTES`]) from a resolved PERSPECTIVE [`ViewUniform`], for the
/// host's [`GBufferScene::mvp`](boyko_rhi_vulkan::swapchain::GBufferScene) (host
/// plan R3).
///
/// # Marcher-aligned by construction (NOT `view_proj_columns`)
///
/// The raster mesh and the SDF marcher must agree in screen x/y — the deferred
/// resolve reconstructs each pixel's world position from the camera basis +
/// `gViewT`, so a raster projection with a different convention detaches
/// lighting/shadows from the geometry. This bridge therefore reproduces the
/// GPU-verified construction of the `window_present_gbuffer` viewer's
/// `perspective_mvp_bytes` (its convention notes hold verbatim), fed from the
/// SAME [`ViewUniform`] lanes [`composite_perspective_from_view`] feeds the
/// marcher's b5 camera:
///
/// * clip.x = `x_cam / (aspect · tan)`, clip.y = `−y_cam / tan` (the y-flip
///   matching the marcher's `ndc_y = -(...)` ray-gen);
/// * clip.z = clip.w = `forward · (P − eye)` (positive in front — the exact
///   clip-z is irrelevant: the gbuffer FS overwrites depth with the
///   marcher-aligned euclidean `length(cam_eye − P) / T_MAX`);
/// * bytes 64..80 = `cam_eye` (`xyz` = eye, `w` = 1.0 — perspective mode);
/// * bytes 80..88 = `{ base_instance = 0; use_model_matrix }` — the recorder
///   overwrites `base_instance` per batch; `instanced` selects the VS arm
///   (`true` REQUIRED when `GBufferScene::mesh_draw` is non-empty).
///
/// # Aspect is DERIVED FROM THE EXTENT — NOT `ViewUniform::aspect`
///
/// `width`/`height` are the COMPOSITE extent — the SAME `(w, h)` the caller
/// gives [`composite_from_view`] for the marcher's b5 camera (whose
/// `CompositePushConstants::perspective` also computes `aspect = w / h`). Both
/// pushes therefore derive the aspect from one extent BY CONSTRUCTION; the
/// user-authored `Projection` aspect (`view.aspect`) is deliberately NOT
/// consulted — if the OS adjusts the boot client size, an authored aspect
/// would silently diverge from the marcher's, misaligning the raster geometry
/// against the resolve's camera-ray world reconstruction (the static form of
/// the motion-shadow class; the `camera_ray` extent-derived-aspect precedent).
///
/// PERSPECTIVE-only: an orthographic view (`fov_y == 0`) is debug-asserted out
/// — the ortho raster path is tied to the frozen SDF fixture constants and is
/// not a host bridge (v1 scope).
#[rustfmt::skip]
pub fn gbuffer_push_from_view(
    view: &ViewUniform,
    width: u32,
    height: u32,
    instanced: bool,
) -> [u8; GBUFFER_PUSH_BYTES] {
    debug_assert!(
        view.fov_y > 0.0,
        "invariant: the gbuffer raster push bridge is PERSPECTIVE-only (fov_y > 0)"
    );
    debug_assert!(
        width > 0 && height > 0,
        "invariant: the composite extent is non-zero"
    );
    let eye = [view.camera_pos.x, view.camera_pos.y, view.camera_pos.z];
    let forward = [view.cam_forward.x, view.cam_forward.y, view.cam_forward.z];
    let right = [view.cam_right.x, view.cam_right.y, view.cam_right.z];
    let up = [view.cam_up.x, view.cam_up.y, view.cam_up.z];
    let tan = (view.fov_y * 0.5).tan();
    // Extent-derived, matching `CompositePushConstants::perspective` exactly
    // (see the doc section above) — NOT `view.aspect`.
    let aspect = (width as f32) / (height as f32);

    let dot = |a: [f32; 3], b: [f32; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    let tx = -dot(right, eye);
    let ty = -dot(up, eye);
    let tz = dot(forward, eye); // in-front view depth: z_cam = forward·P − tz

    // proj·view, ROW-MAJOR math rows; uploaded column-major (the verified
    // transpose). `forward` points INTO the scene (the marcher's ray direction),
    // so the clip.z/w row is `+forward` (see the viewer's sign-bug note).
    let sx = 1.0 / (aspect * tan);
    let sy = -1.0 / tan;
    let (rx, ry, rz) = (right[0], right[1], right[2]);
    let (ux, uy, uz) = (up[0], up[1], up[2]);
    let (fx, fy, fz) = (forward[0], forward[1], forward[2]);
    let pv: [[f32; 4]; 4] = [
        [sx * rx, sx * ry, sx * rz, sx * tx], // clip.x
        [sy * ux, sy * uy, sy * uz, sy * ty], // clip.y (marcher y-flip)
        [fx,      fy,      fz,      -tz],     // clip.z = forward·(P − eye)
        [fx,      fy,      fz,      -tz],     // clip.w (perspective divide)
    ];

    let mut out = [0u8; GBUFFER_PUSH_BYTES];
    for col in 0..4 {
        for row in 0..4 {
            let b = pv[row][col].to_le_bytes();
            out[(col * 4 + row) * 4..(col * 4 + row) * 4 + 4].copy_from_slice(&b);
        }
    }
    // cam_eye push lane (bytes 64..80): xyz = eye, w = 1.0 (perspective mode).
    let cam_eye = [eye[0], eye[1], eye[2], 1.0_f32];
    for (i, f) in cam_eye.iter().enumerate() {
        out[64 + i * 4..64 + i * 4 + 4].copy_from_slice(&f.to_le_bytes());
    }
    // Trailing selectors: base_instance (@80) stays 0 (the recorder overwrites it
    // per batch); use_model_matrix (@84) selects the instanced VS arm.
    if instanced {
        out[84..88].copy_from_slice(&1u32.to_le_bytes());
    }
    out
}

/// Re-views a column-major [`Mat4`] as the demo `CameraUniform.view_proj` layout
/// (`[[f32; 4]; 4]`, each inner array a COLUMN — the WGSL `mat4x4` upload form).
///
/// `Mat4` is already column-major (`cols[j]` is column `j`), so this is a pure
/// field copy with no transpose. The demo appends its Phase-20.1 `alpha` itself;
/// this only supplies the matrix from the engine view.
#[inline]
pub fn view_proj_columns(m: Mat4) -> [[f32; 4]; 4] {
    [
        [m.cols[0].x, m.cols[0].y, m.cols[0].z, m.cols[0].w],
        [m.cols[1].x, m.cols[1].y, m.cols[1].z, m.cols[1].w],
        [m.cols[2].x, m.cols[2].y, m.cols[2].z, m.cols[2].w],
        [m.cols[3].x, m.cols[3].y, m.cols[3].z, m.cols[3].w],
    ]
}

/// The raster `view_proj` (`[[f32; 4]; 4]`, column-major) from a resolved
/// [`ViewUniform`], for a host that wraps it (plus its own `alpha` / padding) into
/// an 80-byte `CameraUniform`.
///
/// This is the engine view's `view_proj` (`proj · view`) re-laid as WGSL columns,
/// NOT the demo's legacy `CameraUniform::ortho_fit` projection. For an
/// orthographic camera the matrix is the standard
/// [`orthographic_rh`](boyko_math::Mat4::orthographic_rh) (fixed world extents +
/// `[0,1]` depth remap), which differs from `ortho_fit`'s viewport-fit scale (see
/// the module-level "NOT `ortho_fit`" note). Adopting it in the demo is therefore
/// a behavioural change, not a drop-in.
#[inline]
pub fn demo_view_proj_from_view(view: &ViewUniform) -> [[f32; 4]; 4] {
    view_proj_columns(view.view_proj)
}

#[cfg(test)]
mod tests {
    use super::*;

    use boyko_rhi_vulkan::compute::CAM_MODE_PERSPECTIVE;
    use boyko_scene::{Projection, ViewUniform};

    use boyko_math::{Affine3A, Mat3, Mat4, Vec3};

    use core::f32::consts::FRAC_PI_3;

    /// The forward perspective camera the prior marcher hardcoded: eye `(0,0,3)`,
    /// identity rotation (so the world basis is the canonical
    /// `right(1,0,0)/up(0,1,0)/forward(0,0,-1)`), 60° FOV.
    fn forward_perspective_camera() -> (Affine3A, Projection) {
        let global = Affine3A {
            matrix3: Mat3::IDENTITY,
            translation: Vec3::new(0.0, 0.0, 3.0),
        };
        let projection = Projection::Perspective {
            fov_y: FRAC_PI_3,
            aspect: 1.0,
            near: 0.1,
            far: 100.0,
        };
        (global, projection)
    }

    /// S3 no-regression: a forward perspective camera bridged through
    /// [`composite_perspective_from_view`] is BYTE-IDENTICAL to the legacy
    /// hand-fed [`CompositePushConstants::perspective`]. Guards against the bridge
    /// drifting from the frozen marcher push-constant fill.
    #[test]
    fn perspective_bridge_reproduces_legacy_push_constants() {
        let (global, projection) = forward_perspective_camera();
        let view = ViewUniform::from_camera(global, projection);

        let bridged = composite_perspective_from_view(&view, 64, 64);
        let legacy = CompositePushConstants::perspective(
            [0.0, 0.0, 3.0],
            [0.0, 0.0, -1.0],
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            FRAC_PI_3,
            64,
            64,
        );

        assert_eq!(bridged, legacy);
        assert_eq!(bridged.camera_mode, CAM_MODE_PERSPECTIVE);
    }

    /// S3: a perspective camera routes [`composite_from_view`] to the perspective
    /// path (`fov_y > 0`), not the ORTHO sentinel branch.
    #[test]
    fn composite_from_view_routes_perspective() {
        let (global, projection) = forward_perspective_camera();
        let view = ViewUniform::from_camera(global, projection);
        assert_eq!(
            composite_from_view(&view, 64, 64),
            composite_perspective_from_view(&view, 64, 64),
        );
    }

    /// S3: an orthographic camera carries the `fov_y == 0.0` sentinel and routes
    /// [`composite_from_view`] to the bit-frozen ORTHO golden, byte-identical to
    /// [`CompositePushConstants::ortho`] — the ORTHO golden stays untouched.
    #[test]
    fn composite_from_view_routes_ortho_to_frozen_golden() {
        let view = ViewUniform::from_camera(
            Affine3A::IDENTITY,
            Projection::Orthographic {
                half_height: 1.0,
                aspect: 1.0,
                near: 0.0,
                far: 100.0,
            },
        );
        assert_eq!(view.fov_y, 0.0);
        assert_eq!(composite_from_view(&view, 64, 64), CompositePushConstants::ortho(64, 64));
    }

    /// Finding-2 REF TEST: the raster ortho bridge is the STANDARD
    /// `orthographic_rh`, NOT the demo's viewport-fit `ortho_fit`. Pins the
    /// produced `view_proj` to `orthographic_rh(-hw, hw, -hh, hh, near, far)` for
    /// an identity-pose ortho camera, and asserts it is NOT `ortho_fit`'s
    /// `diag(1/ext_x, 1/ext_y, 1, 1)` shape (the z-scale differs: `orthographic_rh`
    /// remaps depth to `[0,1]` via `nf != 1`, `ortho_fit` leaves z untouched).
    #[test]
    fn ortho_bridge_is_orthographic_rh_not_viewport_fit() {
        let half_height = 2.0_f32;
        let aspect = 1.5_f32;
        let near = 0.5_f32;
        let far = 50.0_f32;
        let view = ViewUniform::from_camera(
            Affine3A::IDENTITY,
            Projection::Orthographic {
                half_height,
                aspect,
                near,
                far,
            },
        );

        // The bridge emits the camera's projection (identity view ∘ proj = proj).
        let half_width = half_height * aspect;
        let expected = view_proj_columns(Mat4::orthographic_rh(
            -half_width,
            half_width,
            -half_height,
            half_height,
            near,
            far,
        ));
        assert_eq!(demo_view_proj_from_view(&view), expected);

        // It is NOT the viewport-fit `ortho_fit` shape: `orthographic_rh` remaps
        // depth (column 2, row 2 = 1/(near-far) != 1), whereas `ortho_fit` leaves
        // the z-scale at 1.0. This is the concrete "remap not equal to ortho_fit"
        // the contract previously claimed away.
        let cols = demo_view_proj_from_view(&view);
        let z_scale = cols[2][2];
        assert_ne!(z_scale, 1.0, "engine ortho remaps depth; ortho_fit does not");
        assert!((z_scale - (near - far).recip()).abs() <= 1e-6);
    }

    /// S3: the default [`ViewUniform`] is the identity view, so the raster bridge
    /// emits the identity matrix before the first resolve runs.
    #[test]
    fn default_view_bridges_to_identity_matrix() {
        let cols = demo_view_proj_from_view(&ViewUniform::default());
        assert_eq!(cols, view_proj_columns(Mat4::IDENTITY));
    }
}
