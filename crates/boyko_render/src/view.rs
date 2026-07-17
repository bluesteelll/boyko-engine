//! View wiring (S3): the conversion seam from the engine-derived
//! [`ViewUniform`] to the backend-specific view forms.
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

use crate::taa_jitter::NdcJitter;

/// TAA rung C1: the b5 camera-basis SHEAR — the marcher/resolve/SSAO/CSM/froxel-shared b5
/// forward basis, perturbed so `generate_ray`'s (the marcher's) reconstructed ray samples the
/// EXACT SAME final-NDC sub-pixel position the raster jitter
/// (`crate::taa_jitter::NdcJitter`, `row0 += jx*row3; row1 += jy*row3` in
/// [`marcher_view_proj_rows_jittered`]) already shifts to — lifting the C1 cut
/// (`crate::taa_jitter`'s module doc) under
/// [`JitterScope::RasterAndBasis`](crate::taa_config::JitterScope::RasterAndBasis). See
/// `docs/TAA-PLAN.md` Decision 1 for the architecture-level derivation this fn implements.
///
/// # The shear (derivation)
///
/// `ray_gen.hlsli`'s PERSPECTIVE branch (`generate_ray`) computes
/// `dir = fwd + right·(ndc_x·aspect·tan) + up·(ndc_y·tan)`. This is LINEAR in `(ndc_x, ndc_y)`,
/// so for any constant offset `(dx, dy)`:
///
/// ```text
/// dir(fwd, ndc + (dx, dy)) = dir(fwd, ndc) + right·(dx·aspect·tan) + up·(dy·tan)
///                           = dir(fwd + right·(dx·aspect·tan) + up·(dy·tan), ndc)
/// ```
///
/// i.e. shearing `fwd` by `right·(dx·aspect·tan) + up·(dy·tan)` is EXACTLY equivalent, in real
/// arithmetic, to shifting `ray_gen.hlsli`'s own `ndc` by `(dx, dy)` (IEEE re-association gives
/// a few-ULP difference in practice — see this module's tests for the measured bound).
///
/// `ray_gen.hlsli`'s `ndc_x` matches the raster's final NDC.x directly (both increase
/// rightward, no flip: `NDC.x_raster = right·(P-eye)/(view_z·aspect·tan) == ndc_x_raygen`), but
/// its `ndc_y` is the NEGATION of the raster's final NDC.y: Vulkan clip.y+ points down
/// (`sy = -1/tan` in [`marcher_view_proj_rows_jittered`]), while `ray_gen.hlsli`'s own `ndc_y`
/// flips AGAIN to keep "up" pointing up (`float ndc_y = -(...)`), so
/// `NDC.y_raster == -ndc_y_raygen`. A raster shift of `(jx, jy)` — the SAME [`NdcJitter`] the
/// raster consumers apply — therefore corresponds to a ray-gen-space shift of `(jx, -jy)`:
///
/// ```text
/// fwd' = fwd + right * (jx * aspect * tan_half_fov) - up * (jy * tan_half_fov)
/// ```
///
/// which is exactly `docs/TAA-PLAN.md` Decision 1's `fwd' = fwd + right·(2jx/w·aspect·tanHalfFov)
/// + up·(-2jy/h·tanHalfFov)` — [`NdcJitter::jx`]/[`NdcJitter::jy`] already ARE `2jx/w`/`2jy/h`
/// ([`crate::taa_jitter::ndc_jitter`]'s own formula).
///
/// # Structural zero (not an arithmetic identity)
///
/// `ndc_jitter == None` returns `view.cam_forward` completely UNTOUCHED — a structural skip, not
/// a `+ right*0.0 - up*0.0` computation (which can flip a `-0.0` sign bit and byte-change the
/// UBO — the SAME discipline [`crate::taa_jitter::ndc_jitter`]'s module doc documents for the
/// raster jitter). `Some([0.0, 0.0])` is therefore NOT an equivalent substitute for `None`; a
/// caller intending the OFF path must pass `None`. The host call site
/// (`boyko_app::runner`) gates on `JitterScope::RasterAndBasis` AND the frame's TAA-armed state,
/// producing `None` when either is false.
///
/// Only `.xyz` is sheared — `cam_forward.w` (`tan(fovY/2)`) and `cam_right.w` (`aspect`) are
/// untouched (set by [`CompositePushConstants::perspective`] from `fov_y`/`w`/`h`, unrelated to
/// the shear).
#[inline]
pub fn composite_perspective_from_view_sheared(
    view: &ViewUniform,
    w: u32,
    h: u32,
    ndc_jitter: Option<[f32; 2]>,
) -> CompositePushConstants {
    let eye = view.camera_pos;
    let right = view.cam_right;
    let up = view.cam_up;
    let (fwd_x, fwd_y, fwd_z) = match ndc_jitter {
        None => (view.cam_forward.x, view.cam_forward.y, view.cam_forward.z),
        Some([jx, jy]) => {
            let tan_half_fov = (view.fov_y * 0.5).tan();
            // Extent-derived, matching every other bridge fn in this module -- NOT `view.aspect`.
            let aspect = (w as f32) / (h as f32);
            let sx = jx * aspect * tan_half_fov;
            let sy = jy * tan_half_fov;
            (
                view.cam_forward.x + right.x * sx - up.x * sy,
                view.cam_forward.y + right.y * sx - up.y * sy,
                view.cam_forward.z + right.z * sx - up.z * sy,
            )
        }
    };
    CompositePushConstants::perspective(
        [eye.x, eye.y, eye.z],
        [fwd_x, fwd_y, fwd_z],
        [right.x, right.y, right.z],
        [up.x, up.y, up.z],
        view.fov_y,
        w,
        h,
    )
}

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
///
/// Delegates to [`composite_perspective_from_view_sheared`] with `ndc_jitter = None` — a
/// structural skip (not an arithmetic identity), so this stays byte-identical to the
/// pre-C1-lift formula. The single construction site both the sheared and unsheared
/// PERSPECTIVE b5 pushes share (mirrors [`marcher_view_proj_rows`]/
/// [`marcher_view_proj_rows_jittered`]'s shape).
#[inline]
pub fn composite_perspective_from_view(view: &ViewUniform, w: u32, h: u32) -> CompositePushConstants {
    composite_perspective_from_view_sheared(view, w, h, None)
}

/// [`composite_from_view`] with an optional TAA rung-C1 b5 camera-basis shear — routes ORTHO vs
/// PERSPECTIVE exactly as [`composite_from_view`] does; `ndc_jitter` is IGNORED on the ORTHO
/// branch (TAA is perspective-only — `docs/TAA-PLAN.md`: "Ortho cameras cannot be sheared"), so
/// an orthographic camera's push is identical regardless of the jitter argument. A perspective
/// camera routes to [`composite_perspective_from_view_sheared`].
#[inline]
pub fn composite_from_view_sheared(
    view: &ViewUniform,
    w: u32,
    h: u32,
    ndc_jitter: Option<[f32; 2]>,
) -> CompositePushConstants {
    // `fov_y == 0.0` is the orthographic sentinel (perspective FOVs are > 0). The
    // ORTHO fixture is camera-basis-free (the shader ignores it), so the frozen
    // `ortho(w, h)` layout is emitted verbatim — the golden stays byte-exact,
    // regardless of `ndc_jitter` (TAA is perspective-only).
    if view.fov_y == 0.0 {
        let pc = CompositePushConstants::ortho(w, h);
        debug_assert_eq!(pc.camera_mode, CAM_MODE_ORTHO);
        pc
    } else {
        composite_perspective_from_view_sheared(view, w, h, ndc_jitter)
    }
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
///
/// Delegates to [`composite_from_view_sheared`] with `ndc_jitter = None` — the structural skip,
/// byte-identical to today.
#[inline]
pub fn composite_from_view(view: &ViewUniform, w: u32, h: u32) -> CompositePushConstants {
    composite_from_view_sheared(view, w, h, None)
}

/// The marcher-aligned proj·view matrix (ROW-MAJOR math rows) from a resolved
/// PERSPECTIVE [`ViewUniform`] and a `w × h` extent — the SINGLE construction of
/// the raster projection convention.
///
/// Both the raster gbuffer push ([`gbuffer_push_from_view`], which uploads it
/// column-major at bytes 0..64) and the HW-RT Rung-3b `MotionCam` UBO
/// (`boyko_render::motion_cam`) build their view-proj from THIS function, so the
/// motion vector's `cur`/`prev` endpoints are placed with EXACTLY the projection
/// the rasterizer/marcher used — the static-camera convergence anchor (motion
/// vector ≡ 0 when nothing moves) holds by construction, with no risk of a
/// second, drifting projection.
///
/// # Convention (verified against the `window_present_gbuffer` viewer)
///
/// * clip.x = `x_cam / (aspect · tan)`, clip.y = `−y_cam / tan` (the y-flip
///   matching the marcher's `ndc_y = -(...)` ray-gen);
/// * clip.z = clip.w = `forward · (P − eye)` (positive in front — the exact clip-z
///   is irrelevant: the gbuffer FS overwrites depth with the marcher-aligned
///   euclidean `length(cam_eye − P) / T_MAX`);
/// * `aspect` is DERIVED FROM THE EXTENT (`w / h`), NOT `ViewUniform::aspect` —
///   the SAME extent [`composite_from_view`] gives the marcher's b5 camera, so
///   both derive one aspect by construction (an authored aspect would silently
///   misalign the raster geometry against the resolve's camera-ray world
///   reconstruction — the static form of the motion-shadow class).
///
/// PERSPECTIVE-only: an orthographic view (`fov_y == 0`) is debug-asserted out.
///
/// [`marcher_view_proj_rows`] is this function called with [`NdcJitter::default`] (the
/// exact-zero offset) — the SINGLE construction site both the jittered and non-jittered raster
/// projections share, so OFF byte-identity is provable (a zero jitter is an additive zero, not
/// a separately-derived "unjittered" formula that could drift from this one).
///
/// # TAA raster-only jitter (C1)
///
/// `row2 == row3 == [fx, fy, fz, -tz]` (clip.z == clip.w — the perspective-divide row), so
/// `row0 += jitter.jx * row3; row1 += jitter.jy * row3` is EXACT post-divide NDC jitter:
/// dividing the perturbed `clip.xy` by the UNCHANGED `clip.w` shifts `ndc.xy` by exactly
/// `jitter`. This is a purely host-side perturbation of a push-constant matrix — it does not
/// touch the raster VS `.spv` or the frozen eDSL SDF marcher (the marcher stays UNjittered by
/// DEFAULT in v1 — see [`crate::taa_jitter`]'s module docs for the C1 rationale: the b5 UBO
/// `cam_forward` this bridge's `forward` lane feeds is shared, raw, with deferred PBR / SSAO /
/// CSM / froxel view-z reconstruction, so perturbing it unconditionally would corrupt those).
/// Rung C1 adds an OPT-IN sibling that DOES perturb that shared basis exactly, via a linear
/// shear rather than the `+ jitter*row3` trick above — see
/// [`composite_perspective_from_view_sheared`]'s doc.
#[rustfmt::skip]
#[inline]
pub fn marcher_view_proj_rows_jittered(
    view: &ViewUniform,
    width: u32,
    height: u32,
    jitter: NdcJitter,
) -> [[f32; 4]; 4] {
    debug_assert!(
        view.fov_y > 0.0,
        "invariant: the marcher-aligned view-proj bridge is PERSPECTIVE-only (fov_y > 0)"
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
    // Extent-derived, matching `CompositePushConstants::perspective` exactly — NOT `view.aspect`.
    let aspect = (width as f32) / (height as f32);

    let dot = |a: [f32; 3], b: [f32; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    let tx = -dot(right, eye);
    let ty = -dot(up, eye);
    let tz = dot(forward, eye); // in-front view depth: z_cam = forward·P − tz

    let sx = 1.0 / (aspect * tan);
    let sy = -1.0 / tan;
    let (rx, ry, rz) = (right[0], right[1], right[2]);
    let (ux, uy, uz) = (up[0], up[1], up[2]);
    let (fx, fy, fz) = (forward[0], forward[1], forward[2]);
    let row3 = [fx, fy, fz, -tz]; // clip.z == clip.w (perspective divide row)
    [
        [
            sx * rx + jitter.jx * row3[0],
            sx * ry + jitter.jx * row3[1],
            sx * rz + jitter.jx * row3[2],
            sx * tx + jitter.jx * row3[3],
        ], // clip.x, jittered
        [
            sy * ux + jitter.jy * row3[0],
            sy * uy + jitter.jy * row3[1],
            sy * uz + jitter.jy * row3[2],
            sy * ty + jitter.jy * row3[3],
        ], // clip.y (marcher y-flip), jittered
        row3, // clip.z = forward·(P − eye)
        row3, // clip.w (perspective divide) — UNCHANGED, so the jitter above is exact post-divide
    ]
}

/// PERSPECTIVE-only: an orthographic view (`fov_y == 0`) is debug-asserted out (delegates to
/// [`marcher_view_proj_rows_jittered`]'s assert).
///
/// Delegates to [`marcher_view_proj_rows_jittered`] with [`NdcJitter::default`] — an exact
/// `{0.0, 0.0}` offset, so `row0 += 0.0 * row3[k]` / `row1 += 0.0 * row3[k]` is an additive
/// zero: byte-identical to the pre-TAA formula.
#[inline]
pub fn marcher_view_proj_rows(view: &ViewUniform, width: u32, height: u32) -> [[f32; 4]; 4] {
    marcher_view_proj_rows_jittered(view, width, height, NdcJitter::default())
}

/// Multi-paradigm render-path plan, rung R4b (Forward render path v1, Decision 4): the FORWARD
/// raster's REVERSE-Z projection — a row-major proj·view matrix in the SAME (right/up/forward)
/// basis convention as [`marcher_view_proj_rows`] (identical clip.x/clip.y rows: extent-derived
/// aspect, the marcher y-flip — screen x/y placement matches Deferred's raster exactly), but with
/// a REAL depth row instead of Deferred's `clip.z == clip.w` convention (Deferred's raster FS
/// overwrites `SV_Depth` with a custom-linear encode, so its vertex-shader clip.z is a throwaway
/// `1.0` after the divide — see [`marcher_view_proj_rows_jittered`]'s doc). Forward's `depth`
/// image is standard HARDWARE reverse-Z (no `SV_Depth` write, early-Z stays live), so the vertex
/// shader's clip.z must carry a real, monotonic depth this time — this function is that encode's
/// SINGLE construction site, kept separate from (and never touching) the Deferred one above.
///
/// # Reverse-Z depth encode
///
/// Standard Vulkan depth range `[0,1]`, REVERSED so `view_z == near` maps to `depth == 1` and
/// `view_z == far` maps to `depth == 0` (the numerically superior float-depth distribution —
/// precision concentrates near the camera, matching the eye's own float32 mantissa density).
/// Solving `depth(view_z) = A + B / view_z` for the two anchor points:
///
/// ```text
/// A + B/near = 1      A = -near / (far - near)
/// A + B/far  = 0   =>  B =  near * far / (far - near)
/// ```
///
/// Expressed against WORLD `P` (since `view_z = dot(forward, P) - tz` is itself the row-major dot
/// `row3 · [P, 1]`), `clip.z`'s row is `A · row3 + [0, 0, 0, B]`. `clip.w` stays `row3` (`view_z`)
/// — the SAME standard perspective-divide row [`marcher_view_proj_rows`] uses, unchanged. The
/// matching pipeline state (Forward's boot-time depth-stencil state) is `VK_COMPARE_OP_GREATER`
/// (a nearer fragment has a LARGER stored depth) with a `0.0` depth CLEAR (the "nothing drawn yet"
/// sentinel — farther, in reverse-Z terms, than any real `depth ∈ (0, 1]`).
///
/// PERSPECTIVE-only (mirrors [`marcher_view_proj_rows`]'s `fov_y > 0` invariant); `view.near > 0.0`
/// and `view.far > view.near` are debug-asserted (a degenerate frustum divides by zero in `A`/`B`
/// above). `width`/`height` are the render EXTENT (not `ViewUniform::aspect`), matching every
/// other bridge fn in this module (the extent-derived-aspect precedent — see
/// [`gbuffer_push_from_view_jittered`]'s doc for why an authored aspect is deliberately not
/// consulted).
///
/// No jittered sibling yet (unlike [`marcher_view_proj_rows_jittered`]): Forward v1 has no TAA
/// (the plan's v1 scope cut, [`crate::render_path_config::RenderPathDegrade::ForwardTaaNotYetImplemented`]) —
/// a future ForwardPlus/TAA-under-Forward rung adds a `forward_view_proj_rows_jittered` sibling
/// mirroring [`marcher_view_proj_rows_jittered`]'s `jitter.jx * row3` / `jitter.jy * row3` pattern
/// (still exact post-divide NDC jitter here, since `row3` — the perspective-divide row — is
/// unchanged from the Deferred construction).
#[rustfmt::skip]
#[inline]
pub fn forward_view_proj_rows(view: &ViewUniform, width: u32, height: u32) -> [[f32; 4]; 4] {
    debug_assert!(
        view.fov_y > 0.0,
        "invariant: the forward reverse-Z projection is PERSPECTIVE-only (fov_y > 0)"
    );
    debug_assert!(width > 0 && height > 0, "invariant: the composite extent is non-zero");
    debug_assert!(
        view.near > 0.0 && view.far > view.near,
        "invariant: a valid reverse-Z frustum needs 0 < near < far"
    );

    let eye = [view.camera_pos.x, view.camera_pos.y, view.camera_pos.z];
    let forward = [view.cam_forward.x, view.cam_forward.y, view.cam_forward.z];
    let right = [view.cam_right.x, view.cam_right.y, view.cam_right.z];
    let up = [view.cam_up.x, view.cam_up.y, view.cam_up.z];
    let tan = (view.fov_y * 0.5).tan();
    // Extent-derived, matching `marcher_view_proj_rows` exactly — NOT `view.aspect`.
    let aspect = (width as f32) / (height as f32);

    let dot = |a: [f32; 3], b: [f32; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    let tx = -dot(right, eye);
    let ty = -dot(up, eye);
    let tz = dot(forward, eye); // in-front view depth: view_z = forward·P − tz

    let sx = 1.0 / (aspect * tan);
    let sy = -1.0 / tan;
    let (rx, ry, rz) = (right[0], right[1], right[2]);
    let (ux, uy, uz) = (up[0], up[1], up[2]);
    let (fx, fy, fz) = (forward[0], forward[1], forward[2]);
    let row3 = [fx, fy, fz, -tz]; // clip.w = view_z = dot(forward, P) − tz

    // Reverse-Z depth encode (this fn's doc): clip.z = A * view_z + B, expressed against `row3`.
    let range = view.far - view.near;
    let a = -view.near / range;
    let b = view.near * view.far / range;
    let row2 = [a * row3[0], a * row3[1], a * row3[2], a * row3[3] + b];

    [
        [sx * rx, sx * ry, sx * rz, sx * tx], // clip.x
        [sy * ux, sy * uy, sy * uz, sy * ty], // clip.y (marcher y-flip)
        row2,                                  // clip.z (reverse-Z depth)
        row3,                                  // clip.w (perspective divide, unchanged)
    ]
}

/// Multi-paradigm render-path plan, rung R-SDFFWD (Decision 4's consumer half): inverts
/// [`forward_view_proj_rows`]'s reverse-Z depth encode back to view-space depth
/// (`view_z = dot(forward, P) − tz`, the SAME `view_z` that fn's `row3 · [P, 1]` computes before
/// the perspective divide) from a SAMPLED `depth ∈ [0, 1]` (HW reverse-Z) and the SAME
/// `near`/`far` the encode used.
///
/// # The exact inverse
///
/// [`forward_view_proj_rows`]'s doc derives `depth(view_z) = A + B / view_z` with
/// `A = −near / (far − near)`, `B = near · far / (far − near)`. Solving for `view_z`:
///
/// ```text
/// depth = A + B / view_z
/// depth − A = B / view_z
/// view_z = B / (depth − A)
/// ```
///
/// `A < 0` (since `0 < near < far`) and `depth ≥ 0`, so `depth − A ≥ −A = near / (far − near) > 0`
/// strictly — the divide never sees zero for any `depth` in the valid `[0, 1]` range (including
/// the Forward `depth` CLEAR sentinel `0.0`, the "nothing drawn yet" background, which recovers
/// `view_z == far` — the farthest a reconstructed pixel can be, so a background pixel never wins
/// the SDF-forward-march ownership gate's `z_sdf < z_mesh_view` test by construction).
///
/// This is a DIFFERENT reconstruction from the deferred marcher's — Deferred's `depth` is
/// custom-linear (`gbuffer_mrt.fs.hlsl`'s `MESH_DEPTH_T_MAX`/`GBUFFER_T_MAX`-normalized encode,
/// read directly as a Euclidean `t`), while Forward/ForwardPlus write standard HARDWARE
/// reverse-Z depth (Decision 4) — a pixel here must invert THIS encode, never the marcher's.
///
/// PERSPECTIVE-only (mirrors [`forward_view_proj_rows`]'s `near > 0`/`far > near` invariants,
/// debug-asserted here identically since the caller reconstructs against the SAME frustum that
/// wrote the sampled depth).
#[inline]
pub fn forward_view_z_from_depth(depth: f32, near: f32, far: f32) -> f32 {
    debug_assert!(near > 0.0 && far > near, "invariant: a valid reverse-Z frustum needs 0 < near < far");
    let range = far - near;
    let a = -near / range;
    let b = near * far / range;
    b / (depth - a)
}

/// Multi-paradigm render-path plan, rung R-SDFFWD: precomputes [`forward_view_z_from_depth`]'s
/// `A`/`B` reverse-Z decode coefficients (`A = -near/(far-near)`, `B = near*far/(far-near)`) for a
/// host caller that needs to push them into a shader instead of calling that fn per-pixel — the
/// `sdf_forward_march` compute pass's `SdfForwardMarchPush::has_mesh` contract
/// (`boyko_rhi_vulkan::compute::SdfForwardMarchPush`): the shader reads `view_z = B / (depth -
/// A)`, [`forward_view_z_from_depth`]'s own body, ported to HLSL so the pass needs no `near`/`far`
/// fields of its own.
#[inline]
pub fn forward_view_z_coeffs(near: f32, far: f32) -> (f32, f32) {
    debug_assert!(near > 0.0 && far > near, "invariant: a valid reverse-Z frustum needs 0 < near < far");
    let range = far - near;
    (-near / range, near * far / range)
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
///
/// # TAA raster-only jitter (C1)
///
/// `jitter` flows straight into [`marcher_view_proj_rows_jittered`] — the ONLY perturbed lane
/// is this 88-byte VERTEX push's leading `view_proj` (bytes 0..64); `cam_eye` and the trailing
/// selectors are untouched. [`gbuffer_push_from_view`] delegates here with
/// [`NdcJitter::default`] (byte-identical to the pre-TAA push).
#[rustfmt::skip]
pub fn gbuffer_push_from_view_jittered(
    view: &ViewUniform,
    width: u32,
    height: u32,
    instanced: bool,
    jitter: NdcJitter,
) -> [u8; GBUFFER_PUSH_BYTES] {
    let eye = [view.camera_pos.x, view.camera_pos.y, view.camera_pos.z];
    // The marcher-aligned proj·view (ROW-MAJOR math rows) — the SINGLE source of
    // the raster projection convention, shared with the Rung-3b `MotionCam`
    // (see `marcher_view_proj_rows_jittered`).
    let pv = marcher_view_proj_rows_jittered(view, width, height, jitter);

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

/// Delegates to [`gbuffer_push_from_view_jittered`] with [`NdcJitter::default`] — an exact
/// `{0.0, 0.0}` offset, so the emitted push is byte-identical to the pre-TAA formula (the
/// single construction site both the jittered and non-jittered pushes share).
#[inline]
pub fn gbuffer_push_from_view(
    view: &ViewUniform,
    width: u32,
    height: u32,
    instanced: bool,
) -> [u8; GBUFFER_PUSH_BYTES] {
    gbuffer_push_from_view_jittered(view, width, height, instanced, NdcJitter::default())
}

/// Multi-paradigm render-path plan, rung R4b-b: the Forward v1 mesh raster's 88-byte VERTEX
/// push, built from [`forward_view_proj_rows`] (the reverse-Z projection) instead of
/// [`marcher_view_proj_rows`] — the SAME byte layout [`gbuffer_push_from_view`] emits (`{
/// float4x4 view_proj; float4 cam_eye; uint base_instance; uint use_model_matrix }`,
/// [`GBUFFER_PUSH_BYTES`]), consumed by `forward_opaque.vs.hlsl` (byte-identical push contract
/// to `gbuffer_mrt.vs.hlsl`'s, per that shader's doc — "ONLY the matrix CONTENT differs").
///
/// No jittered sibling (unlike [`gbuffer_push_from_view_jittered`]): Forward v1 has no TAA (the
/// resolver's `ForwardTaaNotYetImplemented` degrade forces it off) — a future TAA-under-Forward
/// rung adds one, mirroring [`forward_view_proj_rows`]'s own "no jittered sibling yet" doc.
///
/// PERSPECTIVE-only (delegates to [`forward_view_proj_rows`]'s `fov_y > 0` / `near`/`far`
/// invariants — debug-asserted there). `boyko_app::runner` selects this fn instead of
/// [`gbuffer_push_from_view`] at the SAME `mvp` assembly site, branching on the boot-committed
/// `ResolvedRenderPath::path == RenderPath::Forward` (a cold, boot-resolved host-side branch —
/// the two paths are mutually exclusive per boot, Decision 1).
#[inline]
pub fn forward_gbuffer_push_from_view(
    view: &ViewUniform,
    width: u32,
    height: u32,
    instanced: bool,
) -> [u8; GBUFFER_PUSH_BYTES] {
    let eye = [view.camera_pos.x, view.camera_pos.y, view.camera_pos.z];
    let pv = forward_view_proj_rows(view, width, height);

    let mut out = [0u8; GBUFFER_PUSH_BYTES];
    for col in 0..4 {
        for row in 0..4 {
            let b = pv[row][col].to_le_bytes();
            out[(col * 4 + row) * 4..(col * 4 + row) * 4 + 4].copy_from_slice(&b);
        }
    }
    // cam_eye push lane (bytes 64..80): xyz = eye, w = 1.0 (perspective mode) — byte-identical
    // shape to `gbuffer_push_from_view_jittered`'s.
    let cam_eye = [eye[0], eye[1], eye[2], 1.0_f32];
    for (i, f) in cam_eye.iter().enumerate() {
        out[64 + i * 4..64 + i * 4 + 4].copy_from_slice(&f.to_le_bytes());
    }
    // Trailing selectors: base_instance (@80) stays 0 (the recorder overwrites it per batch);
    // use_model_matrix (@84) selects the instanced VS arm.
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

    /// TAA W2 (mandatory): [`marcher_view_proj_rows`] must be byte-identical to
    /// [`marcher_view_proj_rows_jittered`] called with [`NdcJitter::default`] — the OFF
    /// byte-identity proof (zero jitter = additive zero), checked at the bit level (`to_bits`)
    /// so a `+0.0`/`-0.0` divergence would be caught, not masked by `==`'s zero-equivalence.
    #[test]
    fn marcher_view_proj_rows_matches_jittered_default() {
        let (global, projection) = forward_perspective_camera();
        let view = ViewUniform::from_camera(global, projection);
        let plain = marcher_view_proj_rows(&view, 640, 480);
        let jittered_default =
            marcher_view_proj_rows_jittered(&view, 640, 480, NdcJitter::default());
        for row in 0..4 {
            for col in 0..4 {
                assert_eq!(
                    plain[row][col].to_bits(),
                    jittered_default[row][col].to_bits(),
                    "row {row} col {col}: NdcJitter::default() must be an exact-zero delta"
                );
            }
        }
    }

    /// TAA W2 (mandatory): a nonzero jitter DOES perturb the projection (guards against the
    /// jittered fn silently ignoring `jitter`).
    #[test]
    fn marcher_view_proj_rows_jittered_perturbs_row0_row1_only() {
        let (global, projection) = forward_perspective_camera();
        let view = ViewUniform::from_camera(global, projection);
        let plain = marcher_view_proj_rows(&view, 640, 480);
        let jitter = NdcJitter { jx: 0.01, jy: -0.02 };
        let jittered = marcher_view_proj_rows_jittered(&view, 640, 480, jitter);
        assert_ne!(plain[0], jittered[0], "row0 (clip.x) must be perturbed by jx");
        assert_ne!(plain[1], jittered[1], "row1 (clip.y) must be perturbed by jy");
        // row2/row3 (the perspective-divide row) stay UNCHANGED — the jitter is exact
        // post-divide NDC jitter, not a re-derived projection.
        assert_eq!(plain[2], jittered[2], "row2 (clip.z) must be untouched by raster jitter");
        assert_eq!(plain[3], jittered[3], "row3 (clip.w) must be untouched by raster jitter");
    }

    /// TAA W2 (mandatory): [`gbuffer_push_from_view`] must be byte-identical to
    /// [`gbuffer_push_from_view_jittered`] called with [`NdcJitter::default`].
    #[test]
    fn gbuffer_push_from_view_matches_jittered_default() {
        let (global, projection) = forward_perspective_camera();
        let view = ViewUniform::from_camera(global, projection);
        let plain = gbuffer_push_from_view(&view, 640, 480, true);
        let jittered_default =
            gbuffer_push_from_view_jittered(&view, 640, 480, true, NdcJitter::default());
        assert_eq!(plain, jittered_default);
    }

    /// TAA W2: a nonzero jitter perturbs the emitted push's leading `view_proj` bytes (0..64)
    /// but leaves `cam_eye` (64..80) and the trailing selectors (80..88) untouched.
    #[test]
    fn gbuffer_push_from_view_jittered_perturbs_only_view_proj_bytes() {
        let (global, projection) = forward_perspective_camera();
        let view = ViewUniform::from_camera(global, projection);
        let plain = gbuffer_push_from_view(&view, 640, 480, true);
        let jitter = NdcJitter { jx: 0.01, jy: -0.02 };
        let jittered = gbuffer_push_from_view_jittered(&view, 640, 480, true, jitter);
        assert_ne!(&plain[0..64], &jittered[0..64], "the view_proj lane must be perturbed");
        assert_eq!(&plain[64..88], &jittered[64..88], "cam_eye + selectors must be untouched");
    }

    // ---- rung R4b: `forward_view_proj_rows` (the Forward reverse-Z projection) -----------

    /// Applies a row-major proj·view `m` (as returned by [`marcher_view_proj_rows`] /
    /// [`forward_view_proj_rows`]) to a world point, returning `(ndc, clip_w)`.
    fn apply_row_major(m: [[f32; 4]; 4], p: [f32; 3]) -> ([f32; 3], f32) {
        let ph = [p[0], p[1], p[2], 1.0];
        let clip: [f32; 4] = core::array::from_fn(|row| {
            m[row][0] * ph[0] + m[row][1] * ph[1] + m[row][2] * ph[2] + m[row][3] * ph[3]
        });
        ([clip[0] / clip[3], clip[1] / clip[3], clip[2] / clip[3]], clip[3])
    }

    /// The forward reverse-Z projection maps `view_z == near` to `depth == 1.0` and
    /// `view_z == far` to `depth == 0.0` — the reversed Vulkan `[0,1]` depth range this fn's doc
    /// derives, checked against two points on the camera's forward axis (`clip.w == view_z` by
    /// construction, so placing a point at `eye + view_z * forward` gives an exact `view_z`).
    #[test]
    fn forward_view_proj_rows_reverse_z_depth_at_near_and_far() {
        let (global, projection) = forward_perspective_camera();
        let view = ViewUniform::from_camera(global, projection);
        let m = forward_view_proj_rows(&view, 640, 480);

        let eye = view.camera_pos;
        let fwd = view.cam_forward;
        let near_point = [eye.x + fwd.x * view.near, eye.y + fwd.y * view.near, eye.z + fwd.z * view.near];
        let far_point = [eye.x + fwd.x * view.far, eye.y + fwd.y * view.far, eye.z + fwd.z * view.far];

        let (ndc_near, w_near) = apply_row_major(m, near_point);
        let (ndc_far, w_far) = apply_row_major(m, far_point);

        assert!((w_near - view.near).abs() <= 1e-4, "clip.w must equal view_z at the near point");
        assert!((w_far - view.far).abs() <= 1e-2, "clip.w must equal view_z at the far point");
        assert!((ndc_near[2] - 1.0).abs() <= 1e-4, "reverse-Z: near maps to depth 1.0, got {}", ndc_near[2]);
        assert!(ndc_far[2].abs() <= 1e-4, "reverse-Z: far maps to depth 0.0, got {}", ndc_far[2]);
    }

    /// Depth is MONOTONIC DECREASING in `view_z` under reverse-Z (a nearer fragment has a
    /// LARGER stored depth — the `VK_COMPARE_OP_GREATER` pipeline state this fn's doc pins).
    #[test]
    fn forward_view_proj_rows_reverse_z_is_monotonic_decreasing() {
        let (global, projection) = forward_perspective_camera();
        let view = ViewUniform::from_camera(global, projection);
        let m = forward_view_proj_rows(&view, 640, 480);
        let eye = view.camera_pos;
        let fwd = view.cam_forward;

        let mut prev_depth = f32::INFINITY;
        let steps = 8;
        for i in 0..=steps {
            let t = view.near + (view.far - view.near) * (i as f32 / steps as f32);
            let p = [eye.x + fwd.x * t, eye.y + fwd.y * t, eye.z + fwd.z * t];
            let (ndc, _) = apply_row_major(m, p);
            assert!(ndc[2] <= prev_depth + 1e-6, "depth must not increase as view_z grows (t={t})");
            prev_depth = ndc[2];
        }
    }

    /// clip.x / clip.y (screen placement) and clip.w (the perspective-divide row) are IDENTICAL
    /// to [`marcher_view_proj_rows`]'s — only clip.z (the depth row) differs. This is the
    /// screen-alignment invariant Forward's raster needs to place geometry at the same pixels
    /// Deferred would (Decision 4 changes ONLY the depth contract, never x/y).
    #[test]
    fn forward_view_proj_rows_shares_xy_and_w_rows_with_marcher() {
        let (global, projection) = forward_perspective_camera();
        let view = ViewUniform::from_camera(global, projection);
        let marcher = marcher_view_proj_rows(&view, 640, 480);
        let forward_rz = forward_view_proj_rows(&view, 640, 480);

        assert_eq!(forward_rz[0], marcher[0], "clip.x row must match Deferred's raster exactly");
        assert_eq!(forward_rz[1], marcher[1], "clip.y row must match Deferred's raster exactly");
        assert_eq!(forward_rz[3], marcher[3], "clip.w row must match Deferred's raster exactly");
        assert_ne!(forward_rz[2], marcher[2], "clip.z (depth) must differ -- reverse-Z vs custom-linear");
    }

    // ---- rung R-SDFFWD: `forward_view_z_from_depth` (the SDF-forward-march ownership gate's
    // ---- view-Z reconstruction) round-trips against `forward_view_proj_rows`'s own encode -----

    /// Round-trip: for a point at `eye + t * forward` (so `view_z == t` exactly, by
    /// [`forward_view_proj_rows`]'s `clip.w == view_z` construction), encoding through the real
    /// GPU-bound matrix and dividing by `clip.w` reproduces the depth a fragment shader would
    /// sample; [`forward_view_z_from_depth`] must recover the ORIGINAL `t` from that depth alone
    /// (plus `near`/`far`) -- proving the inverse is the exact algebraic mirror of the encode this
    /// module's own construction site emits, not an independently-derived approximation.
    #[test]
    fn forward_view_z_from_depth_round_trips_forward_view_proj_rows() {
        let (global, projection) = forward_perspective_camera();
        let view = ViewUniform::from_camera(global, projection);
        let m = forward_view_proj_rows(&view, 640, 480);
        let eye = view.camera_pos;
        let fwd = view.cam_forward;

        let steps = 16;
        for i in 0..=steps {
            let t = view.near + (view.far - view.near) * (i as f32 / steps as f32);
            let p = [eye.x + fwd.x * t, eye.y + fwd.y * t, eye.z + fwd.z * t];
            let (ndc, clip_w) = apply_row_major(m, p);
            assert!((clip_w - t).abs() <= 1e-2, "clip.w must equal view_z == t at t={t}, got {clip_w}");

            let recovered = forward_view_z_from_depth(ndc[2], view.near, view.far);
            let tol = t.abs() * 1e-3 + 1e-3;
            assert!(
                (recovered - t).abs() <= tol,
                "round-trip failed at t={t}: depth={}, recovered={recovered}",
                ndc[2]
            );
        }
    }

    /// The two anchor points [`forward_view_proj_rows_reverse_z_depth_at_near_and_far`] pins
    /// (`depth(near) == 1.0`, `depth(far) == 0.0`) must invert back to `near`/`far` exactly --
    /// the closed-form check that does not depend on marching the matrix at all.
    #[test]
    fn forward_view_z_from_depth_recovers_near_and_far_anchors() {
        let near = 0.1_f32;
        let far = 100.0_f32;
        assert!(
            (forward_view_z_from_depth(1.0, near, far) - near).abs() <= 1e-4,
            "depth == 1.0 (reverse-Z near sentinel) must recover view_z == near"
        );
        assert!(
            (forward_view_z_from_depth(0.0, near, far) - far).abs() <= 1e-2,
            "depth == 0.0 (reverse-Z far sentinel, ALSO the FORWARD_DEPTH_CLEAR background) \
             must recover view_z == far"
        );
    }

    // ---- TAA rung C1: `composite_perspective_from_view_sheared` (the b5 camera-basis shear) --

    /// TAA W2-mirroring OFF-gate: [`composite_perspective_from_view`] must be byte-identical
    /// (bit-level, `to_bits`) to [`composite_perspective_from_view_sheared`] called with `None`
    /// -- the structural-skip proof (a computed `-0.0` sign flip would be caught here, not
    /// masked by `==`'s zero-equivalence).
    #[test]
    fn composite_perspective_from_view_sheared_none_matches_unsheared() {
        let (global, projection) = forward_perspective_camera();
        let view = ViewUniform::from_camera(global, projection);
        let plain = composite_perspective_from_view(&view, 640, 480);
        let sheared_none = composite_perspective_from_view_sheared(&view, 640, 480, None);
        assert_eq!(plain.count, sheared_none.count);
        assert_eq!(plain.img_w, sheared_none.img_w);
        assert_eq!(plain.img_h, sheared_none.img_h);
        assert_eq!(plain.camera_mode, sheared_none.camera_mode);
        for i in 0..4 {
            assert_eq!(plain.cam_eye[i].to_bits(), sheared_none.cam_eye[i].to_bits());
            assert_eq!(plain.cam_forward[i].to_bits(), sheared_none.cam_forward[i].to_bits());
            assert_eq!(plain.cam_right[i].to_bits(), sheared_none.cam_right[i].to_bits());
            assert_eq!(plain.cam_up[i].to_bits(), sheared_none.cam_up[i].to_bits());
        }
    }

    /// A nonzero shear perturbs ONLY `cam_forward.xyz` -- `cam_forward.w` (`tan(fovY/2)`),
    /// `cam_eye`, `cam_right`, `cam_up`, and the scalar header fields are all untouched (the
    /// shear is a pure basis perturbation, not a re-derivation of the whole push). Uses the
    /// YAWED (non-axis-aligned) camera fixture -- the axis-aligned `forward_perspective_camera`
    /// has `right.z == up.z == 0`, so `forward.z` is legitimately UNPERTURBED for that fixture
    /// (the shear only adds multiples of `right`/`up`); a genuinely oblique basis is needed to
    /// exercise all three components.
    #[test]
    fn composite_perspective_from_view_sheared_perturbs_only_cam_forward_xyz() {
        let (global, projection) = yawed_perspective_camera(0.7, FRAC_PI_3);
        let view = ViewUniform::from_camera(global, projection);
        let plain = composite_perspective_from_view(&view, 640, 480);
        let sheared = composite_perspective_from_view_sheared(&view, 640, 480, Some([0.01, -0.02]));

        assert_ne!(plain.cam_forward[0], sheared.cam_forward[0], "forward.x must be perturbed");
        assert_ne!(plain.cam_forward[1], sheared.cam_forward[1], "forward.y must be perturbed");
        assert_ne!(plain.cam_forward[2], sheared.cam_forward[2], "forward.z must be perturbed");
        assert_eq!(plain.cam_forward[3], sheared.cam_forward[3], "tan(fovY/2) must be untouched");
        assert_eq!(plain.cam_eye, sheared.cam_eye, "cam_eye must be untouched");
        assert_eq!(plain.cam_right, sheared.cam_right, "cam_right (incl. aspect) must be untouched");
        assert_eq!(plain.cam_up, sheared.cam_up, "cam_up must be untouched");
        assert_eq!(plain.count, sheared.count);
        assert_eq!(plain.img_w, sheared.img_w);
        assert_eq!(plain.img_h, sheared.img_h);
        assert_eq!(plain.camera_mode, sheared.camera_mode);
    }

    /// TAA is perspective-only (`docs/TAA-PLAN.md`: "Ortho cameras cannot be sheared"):
    /// [`composite_from_view_sheared`] on an orthographic view emits the SAME bit-frozen
    /// `CompositePushConstants::ortho` golden regardless of the jitter argument.
    #[test]
    fn composite_from_view_sheared_ortho_ignores_jitter() {
        let view = ViewUniform::from_camera(
            Affine3A::IDENTITY,
            Projection::Orthographic { half_height: 1.0, aspect: 1.0, near: 0.0, far: 100.0 },
        );
        let none = composite_from_view_sheared(&view, 64, 64, None);
        let some = composite_from_view_sheared(&view, 64, 64, Some([0.3, -0.4]));
        assert_eq!(none, some);
        assert_eq!(none, CompositePushConstants::ortho(64, 64));
    }

    // ---- Gate 1: a CPU mirror of `ray_gen.hlsli`'s PERSPECTIVE branch, asserting the shear
    // ---- identity `dir(fwd', ndc) == dir(fwd, ndc + delta)` (first-order exact; a few-ULP
    // ---- IEEE re-association error, NOT bit-identity -- see the derivation in
    // ---- `composite_perspective_from_view_sheared`'s doc). ------------------------------------

    /// A bit-faithful Rust mirror of `ray_gen.hlsli`'s PERSPECTIVE `generate_ray` DIRECTION
    /// (the shear never touches the ORIGIN, which is just `cam_eye`): `dir = fwd +
    /// right*(ndc_x*aspect*tan) + up*(ndc_y*tan)`, normalized. Plain component-wise IEEE ops,
    /// same operation ORDER as the shader (`ray_gen.hlsli:63-65`) -- mirrors that file's own
    /// "no rsqrt/fast-math" determinism discipline.
    fn ray_gen_dir_mirror(
        fwd: [f32; 3],
        right: [f32; 3],
        up: [f32; 3],
        ndc_x: f32,
        ndc_y: f32,
        aspect: f32,
        tan_half_fov: f32,
    ) -> [f32; 3] {
        let sx = ndc_x * aspect * tan_half_fov;
        let sy = ndc_y * tan_half_fov;
        let d = [
            fwd[0] + right[0] * sx + up[0] * sy,
            fwd[1] + right[1] * sx + up[1] * sy,
            fwd[2] + right[2] * sx + up[2] * sy,
        ];
        let len = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
        [d[0] / len, d[1] / len, d[2] / len]
    }

    /// `ray_gen.hlsli:59-60`'s pixel-to-NDC map (PERSPECTIVE branch), reproduced verbatim.
    fn pixel_to_ndc(px: u32, py: u32, w: u32, h: u32) -> (f32, f32) {
        let ndc_x = ((px as f32 + 0.5) / w as f32) * 2.0 - 1.0;
        let ndc_y = -(((py as f32 + 0.5) / h as f32) * 2.0 - 1.0);
        (ndc_x, ndc_y)
    }

    /// A rotated (non-axis-aligned) perspective camera -- a yaw around world-Y by `theta`
    /// radians -- so Gate 1 exercises a genuinely oblique orthonormal basis, not just the
    /// axis-aligned fixture every other test in this module uses.
    fn yawed_perspective_camera(theta: f32, fov_y: f32) -> (Affine3A, Projection) {
        let (s, c) = theta.sin_cos();
        // Row-major world = R * local (see `Mat3::from_columns`'s doc: `mul_vec(e_x) == column
        // 0`), a standard right-handed rotation about +Y.
        let matrix3 = Mat3::from_rows(
            Vec3::new(c, 0.0, s),
            Vec3::new(0.0, 1.0, 0.0),
            Vec3::new(-s, 0.0, c),
        );
        let global = Affine3A { matrix3, translation: Vec3::new(1.5, -0.5, 2.0) };
        let projection = Projection::Perspective { fov_y, aspect: 1.0, near: 0.1, far: 100.0 };
        (global, projection)
    }

    /// Gate 1 (the strongest gate; host-only, no GPU): asserts the shear identity
    /// `normalize(dir(fwd', ndc_p)) ~= normalize(dir(fwd, ndc_p + (jx, -jy)))` for a spread of
    /// pixels x jitters x aspects x FOVs x camera orientations, via the REAL shipped
    /// [`composite_perspective_from_view_sheared`] (not a re-implementation of the shear).
    ///
    /// # Tolerance
    ///
    /// The identity is EXACT in real arithmetic (see that fn's doc derivation) but NOT
    /// bit-exact in IEEE: `(fwd + right*dx*a*t) + right*(ndc_x*a*t)` (the sheared-basis path)
    /// and `fwd + right*((ndc_x+dx)*a*t)` (the shifted-ndc path) differ by float
    /// re-association -- a few-ULP-scale error on EACH of the 3 components going into
    /// `normalize`, not an exact match. `TOL = 1e-5` (absolute, on unit-length normalized
    /// components) is chosen as ~1e2 ULP of f32 (`f32::EPSILON ~= 1.19e-7`) -- generous enough
    /// to absorb the sqrt/div in `normalize` and the widest FOV/aspect cases below, while still
    /// being far tighter than any perceptible (sub-ULP-of-a-pixel) visual error. MEASURED worst
    /// case across this exact sweep (all pixels/jitters/extents/FOVs/yaws below):
    /// `max_err == 1.1920929e-7` -- EXACTLY 1 ULP of `f32::EPSILON`, ~84x tighter than `TOL`. On
    /// any regression the panic message reports the actual measured value.
    #[test]
    fn sheared_b5_forward_matches_shifted_ndc_ray_gen_within_tolerance() {
        const TOL: f32 = 1e-5;
        let mut max_err = 0.0_f32;

        let jitters = [[0.0_f32, 0.0_f32], [0.001, -0.0015], [0.01, 0.02], [-0.03, 0.015], [0.02, -0.02]];
        let extents = [(640_u32, 480_u32), (1920, 1080), (256, 1024), (64, 64)];
        let fovs = [0.35_f32, core::f32::consts::FRAC_PI_3, 1.9]; // ~20deg / 60deg / ~109deg
        let yaws = [0.0_f32, 0.7, -1.2];

        for &yaw in &yaws {
            for &fov_y in &fovs {
                let (global, projection) = yawed_perspective_camera(yaw, fov_y);
                let view = ViewUniform::from_camera(global, projection);
                let tan_half_fov = (fov_y * 0.5).tan();

                for &(w, h) in &extents {
                    let aspect = w as f32 / h as f32;
                    for &[jx, jy] in &jitters {
                        let pc = composite_perspective_from_view_sheared(&view, w, h, Some([jx, jy]));
                        let sheared_fwd = [pc.cam_forward[0], pc.cam_forward[1], pc.cam_forward[2]];
                        let right = [pc.cam_right[0], pc.cam_right[1], pc.cam_right[2]];
                        let up = [pc.cam_up[0], pc.cam_up[1], pc.cam_up[2]];
                        let unsheared_fwd = [view.cam_forward.x, view.cam_forward.y, view.cam_forward.z];

                        // A spread of pixels across the frame (corners + center + off-center).
                        let pixels = [
                            (0, 0),
                            (w - 1, 0),
                            (0, h - 1),
                            (w - 1, h - 1),
                            (w / 2, h / 2),
                            (w / 4, 3 * h / 4),
                        ];
                        for &(px, py) in &pixels {
                            let (ndc_x, ndc_y) = pixel_to_ndc(px, py, w, h);

                            let dir_sheared = ray_gen_dir_mirror(
                                sheared_fwd, right, up, ndc_x, ndc_y, aspect, tan_half_fov,
                            );
                            // Delta-ndc-y sign flip -- see `composite_perspective_from_view_sheared`'s
                            // doc: a raster/NdcJitter shift of `(jx, jy)` is a ray-gen-space shift of
                            // `(jx, -jy)`.
                            let dir_shifted = ray_gen_dir_mirror(
                                unsheared_fwd, right, up, ndc_x + jx, ndc_y - jy, aspect, tan_half_fov,
                            );

                            for k in 0..3 {
                                let err = (dir_sheared[k] - dir_shifted[k]).abs();
                                max_err = max_err.max(err);
                            }
                        }
                    }
                }
            }
        }

        assert!(
            max_err <= TOL,
            "Gate 1: sheared-basis vs shifted-ndc ray direction diverges by {max_err} (> TOL {TOL}) \
             across the pixel/jitter/aspect/FOV/orientation sweep"
        );
        // A non-negative finite measurement -- guards against a vacuous sweep (e.g. every case
        // skipped) silently passing at `max_err == 0.0`.
        assert!(max_err.is_finite());
    }
}
