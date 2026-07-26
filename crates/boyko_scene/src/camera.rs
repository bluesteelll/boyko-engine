//! The camera vocabulary (S3): the [`Camera`] / [`Projection`] components, the
//! [`ActiveCamera`] selection resource, the derived [`ViewUniform`] resource, and
//! the per-frame [`resolve_active_camera`] system that fills the latter from the
//! active camera's `Projection` + `GlobalTransform`.
//!
//! # Principle 0 (no parallel view data system)
//!
//! [`Camera`] and [`Projection`] are ordinary ECS component columns; the view is
//! NOT a renderer-private struct hand-fed each frame. [`ViewUniform`] is an
//! engine [`Resource`] **derived every frame** from the active camera entity by
//! [`resolve_active_camera`] — the single engine-owned view a renderer is meant
//! to consume (via the `boyko_render::view` bridge) instead of reconstructing its
//! own from a per-backend hardcoded source (the demo `CameraUniform` viewport fit
//! and the marcher push-constant basis). Note the engine ortho view is the
//! standard `orthographic_rh`, NOT the demo's viewport-fit `ortho_fit`; the demo
//! and `boyko_rhi_vulkan` call sites adopt the bridge in a staged follow-up (see
//! `boyko_render::view`'s adoption note).
//!
//! # 2D as a subset (D3)
//!
//! A 2D game uses the SAME types: a [`Projection::Orthographic`] active camera
//! with a [`Transform`](crate::transform::Transform) at `z > 0` looking down −Z.
//! There is no `Camera2D`/`Projection2D`.
//!
//! # GPU convention
//!
//! [`ViewUniform::view_proj`] / [`ViewUniform::inv_view`] are **column-major**
//! [`Mat4`]s (the WGSL `mat4x4` convention), so they upload directly to a GPU
//! uniform. The decomposed eye/basis/FOV lanes serve the SDF-marcher push-constant
//! path, which consumes eye + orthonormal basis + FOV rather than `view_proj`.

use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::core::iters::query::Query;
use boyko_ecs::ecs::core::iters::query::data::Mut;
use boyko_ecs::ecs::core::system::{Res, ResMut};
use boyko_ecs::ecs::core::time::Time;
use boyko_macros::{Component, Resource};

use boyko_input::raw::keycode::KeyCode;
use boyko_input::raw::queue::PhysicalInput;
use boyko_math::{Affine3A, Mat3, Mat4, Quat, Ray, Vec3, Vec4};

use crate::transform::{GlobalTransform, Transform};

/// A sub-rectangle of the render target a camera draws into, in **physical
/// pixels** with the origin at the target's top-left.
///
/// `None` on a [`Camera`] means "the full target". `#[repr(C)]` POD so it rides
/// inside the [`Camera`] component column.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Viewport {
    /// Left edge, in physical pixels.
    pub x: f32,
    /// Top edge, in physical pixels.
    pub y: f32,
    /// Width, in physical pixels.
    pub w: f32,
    /// Height, in physical pixels.
    pub h: f32,
}

/// A camera component: an entity carrying a [`Camera`] (+ a [`Projection`] and a
/// [`GlobalTransform`]) is a view into the scene.
///
/// `#[repr(C)]` fixes the declaration order of the fields, but this type is **not
/// byte-blittable POD**: the `viewport: Option<Viewport>` field carries a
/// `repr(Rust)` enum discriminant whose layout is unspecified, so the struct is
/// not safe to memcpy to the GPU or reinterpret via `bytemuck`. `#[repr(C)]` here
/// only pins the field order for the CPU-side component column, not a wire layout.
/// Selection among multiple cameras is **explicit** (no
/// implicit "first wins"): an [`ActiveCamera`] override takes precedence; absent
/// an override, the highest-[`order`](Self::order) camera with
/// [`is_active`](Self::is_active) set is chosen (see [`resolve_active_camera`]).
///
/// # Required components (S8)
///
/// `#[require(Transform, GlobalTransform, Projection = ...)]` enforces the
/// invariant *a camera is never spawned without a pose AND a projection*:
/// inserting a `Camera` alone auto-inserts a [`Transform`] / [`GlobalTransform`]
/// (each via its `Default`) and a [`Projection`]. `Projection` has no `Default`
/// ([`CameraRig`](crate::bundles::CameraRig) makes the caller fill it), so the
/// require supplies a capture-free perspective preset (60° vertical FOV, 16:9,
/// near 0.1, far 1000.0) as the placeholder — a usable view before the designer
/// sets the real projection. A component supplied explicitly suppresses its
/// auto-insert (no double-insert), so a `Camera` spawned together with a custom
/// `Projection` keeps that projection.
#[repr(C)]
#[derive(Component, Clone, Copy, Debug, PartialEq)]
// The `Projection` preset names `Projection::Perspective`'s private fields, so this
// `#[require(...)]` must stay co-located with the `Projection` enum (same module) —
// splitting `Projection` out would break this expr with a privacy error. The
// placeholder is a 3D-biased default; a 2D caller supplies an explicit
// `Projection::Orthographic` (honored — the require only fills when absent).
#[require(
    Transform,
    GlobalTransform,
    Projection = Projection::Perspective {
        fov_y: core::f32::consts::FRAC_PI_3,
        aspect: 16.0 / 9.0,
        near: 0.1,
        far: 1000.0,
    }
)]
pub struct Camera {
    /// Render order / priority. The active-by-policy camera is the one with the
    /// **highest** `order` among the `is_active` cameras (ties resolve to the
    /// first encountered — but ties are a setup error the caller should avoid).
    pub order: i32,
    /// Whether this camera participates in the active-by-policy selection. A
    /// camera with `is_active == false` is never chosen by policy (it can still
    /// be chosen by an explicit [`ActiveCamera`] override).
    pub is_active: bool,
    /// The target sub-rectangle, or `None` for the full render target.
    pub viewport: Option<Viewport>,
}

impl Camera {
    /// A default active camera: `order == 0`, active, full-target.
    pub const DEFAULT: Self = Self {
        order: 0,
        is_active: true,
        viewport: None,
    };
}

impl Default for Camera {
    /// The default camera is [`Camera::DEFAULT`] (active, order 0, full target).
    #[inline]
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// The projection a camera applies: perspective or orthographic.
///
/// `#[repr(C)]` POD enum. The clip-space convention is right-handed with depth in
/// `[0, 1]` (WGSL/Vulkan), produced via [`Mat4::perspective_rh`] /
/// [`Mat4::orthographic_rh`].
#[repr(C)]
#[derive(Component, Clone, Copy, Debug, PartialEq)]
pub enum Projection {
    /// A perspective projection.
    Perspective {
        /// Vertical field of view, in radians.
        fov_y: f32,
        /// Aspect ratio (`width / height`).
        aspect: f32,
        /// Near clip plane distance (`> 0`).
        near: f32,
        /// Far clip plane distance (`> near`).
        far: f32,
    },
    /// An orthographic projection centred on the view axis.
    Orthographic {
        /// Half the visible vertical extent, in world units (the view spans
        /// `[-half_height, half_height]` vertically).
        half_height: f32,
        /// Aspect ratio (`width / height`); the horizontal half-extent is
        /// `half_height * aspect`.
        aspect: f32,
        /// Near clip plane distance.
        near: f32,
        /// Far clip plane distance (`> near`).
        far: f32,
    },
}

impl Projection {
    /// Builds the column-major projection [`Mat4`] for this projection.
    ///
    /// Right-handed, depth in `[0, 1]` (WGSL/Vulkan), so the result uploads
    /// directly to a GPU uniform.
    #[inline]
    pub fn to_mat4(self) -> Mat4 {
        match self {
            Projection::Perspective {
                fov_y,
                aspect,
                near,
                far,
            } => Mat4::perspective_rh(fov_y, aspect, near, far),
            Projection::Orthographic {
                half_height,
                aspect,
                near,
                far,
            } => {
                let half_width = half_height * aspect;
                Mat4::orthographic_rh(-half_width, half_width, -half_height, half_height, near, far)
            }
        }
    }

    /// The vertical field of view in radians (perspective) or the angle-free
    /// ortho sentinel `0.0` (orthographic), for the marcher's `tan(fov_y/2)`
    /// lane.
    #[inline]
    pub fn fov_y(self) -> f32 {
        match self {
            Projection::Perspective { fov_y, .. } => fov_y,
            Projection::Orthographic { .. } => 0.0,
        }
    }

    /// The aspect ratio (`width / height`), common to both variants.
    #[inline]
    pub fn aspect(self) -> f32 {
        match self {
            Projection::Perspective { aspect, .. } | Projection::Orthographic { aspect, .. } => {
                aspect
            }
        }
    }

    /// The near clip distance, common to both variants.
    #[inline]
    pub fn near(self) -> f32 {
        match self {
            Projection::Perspective { near, .. } | Projection::Orthographic { near, .. } => near,
        }
    }

    /// The far clip distance, common to both variants.
    #[inline]
    pub fn far(self) -> f32 {
        match self {
            Projection::Perspective { far, .. } | Projection::Orthographic { far, .. } => far,
        }
    }
}

/// The explicit active-camera override (Principle: no implicit "first wins").
///
/// `Some(entity)` forces [`resolve_active_camera`] to derive the view from that
/// entity's camera (if it is a live camera); `None` falls back to the
/// highest-`order` `is_active` [`Camera`] by policy. Inserted by
/// [`CameraPlugin`](crate::camera_plugin::CameraPlugin) as `None`.
#[derive(Resource, Clone, Copy, Debug, Default)]
pub struct ActiveCamera(pub Option<Entity>);

/// The per-frame derived view: the renderer's single source of truth.
///
/// Carries BOTH the column-major `view_proj` (the raster/demo path) AND the
/// decomposed eye / orthonormal basis / FOV scalars (the SDF-marcher
/// push-constant path), so each backend reads the form it needs without
/// reconstructing a view. Filled by [`resolve_active_camera`]; defaults to the
/// identity view (a valid view before the first resolve runs).
///
/// `#[repr(C, align(16))]`: the `Mat4` / `Vec4` lanes are `align(16)` GPU lanes.
#[repr(C, align(16))]
#[derive(Resource, Clone, Copy, Debug, PartialEq)]
pub struct ViewUniform {
    /// Column-major world→clip transform (`proj · view`), GPU-ready.
    pub view_proj: Mat4,
    /// Column-major camera world matrix (`view⁻¹`), for world-position
    /// reconstruction (lights / marcher). Equals the camera's `GlobalTransform`
    /// as a `Mat4`.
    pub inv_view: Mat4,
    /// Eye world position (`xyz`); `w` is free (`1.0`).
    pub camera_pos: Vec4,
    /// World-space camera forward basis (`xyz`, normalized); `w` is free.
    pub cam_forward: Vec4,
    /// World-space camera right basis (`xyz`, normalized); `w` is free.
    pub cam_right: Vec4,
    /// World-space camera up basis (`xyz`, normalized); `w` is free.
    pub cam_up: Vec4,
    /// Vertical field of view, in radians (`0.0` for an orthographic camera).
    pub fov_y: f32,
    /// Aspect ratio (`width / height`).
    pub aspect: f32,
    /// Near clip distance.
    pub near: f32,
    /// Far clip distance.
    pub far: f32,
}

impl ViewUniform {
    /// The identity view: identity `view_proj` / `inv_view`, eye at the origin,
    /// the canonical −Z-forward / +X-right / +Y-up basis, unit-ish projection
    /// scalars. A valid view before the first [`resolve_active_camera`] runs.
    pub const IDENTITY: Self = Self {
        view_proj: Mat4::IDENTITY,
        inv_view: Mat4::IDENTITY,
        camera_pos: Vec4::new(0.0, 0.0, 0.0, 1.0),
        cam_forward: Vec4::new(0.0, 0.0, -1.0, 0.0),
        cam_right: Vec4::new(1.0, 0.0, 0.0, 0.0),
        cam_up: Vec4::new(0.0, 1.0, 0.0, 0.0),
        fov_y: 0.0,
        aspect: 1.0,
        near: 0.0,
        far: 0.0,
    };

    /// `true` iff a NORMALIZED basis axis is usable as a camera axis, i.e. finite
    /// and non-zero. The single predicate [`Self::from_camera`]'s degenerate-basis
    /// fallback branches on.
    ///
    /// # Why the published basis must never carry a zero or NaN axis
    ///
    /// [`Vec3::normalize`] returns exactly one of three shapes: [`Vec3::ZERO`]
    /// (its `len_sq <= f32::MIN_POSITIVE` guard), an all-NaN vector (a non-finite
    /// or overflowing input makes `inv_len` `0` or NaN, and `inf * 0` / `NaN * x`
    /// poison every lane), or a finite non-zero unit vector. `len_sq > 0.0` is
    /// false for the first two — `0.0 > 0.0` is false and every ordered compare
    /// against NaN is false — and true for the third, so this one compare
    /// classifies all three cases.
    ///
    /// The zero shape is the dangerous one BECAUSE it is finite: it survives every
    /// host finiteness assertion and is uploaded verbatim into the shared camera
    /// block, where `shaders/ray_gen.hlsli`'s perspective ray-gen computes
    /// `dir = forward + right * sx + up * sy` — exactly `(0,0,0)` for a zeroed
    /// basis — and then `normalize(dir)`, which is `0/0`: UNDEFINED per
    /// GLSL.std.450 and NaN on real drivers. That `rd` is consumed by every
    /// `ray_gen.hlsli` includer (the marcher, the deferred/VB resolves, SSAO, the
    /// à-trous passes, TAA, and `cluster_cull.hlsl`'s froxel-AABB build), so a
    /// single degenerate camera transform poisons the whole frame's ray-gen. This
    /// predicate closes that at the source, on the host, where it costs three
    /// compares once per camera per frame instead of a per-pixel guard in twelve
    /// shader translation units.
    ///
    /// Golden-neutral: for any camera whose linear part yields three usable axes —
    /// every valid camera — the branch is not taken and the published bytes are
    /// bit-identical to the pre-fallback ones.
    #[inline]
    fn basis_axis_is_usable(axis: Vec3) -> bool {
        axis.length_squared() > 0.0
    }

    /// Derives the view from a camera's world pose ([`GlobalTransform`] →
    /// [`Affine3A`]) and its [`Projection`].
    ///
    /// * `view_proj = proj · view`, where `view = global⁻¹` — for a rigid
    ///   (orthonormal) camera the inverse is exact via the affine inverse.
    /// * `inv_view = global` as a column-major [`Mat4`] (the camera world matrix).
    /// * eye = `global.translation`; the orthonormal basis is the world image of
    ///   the camera-local axes (`right = +X`, `up = +Y`, `forward = −Z`) under
    ///   the world rotation, i.e. `matrix3 · local_axis` (the row-major [`Mat3`]
    ///   op). For an identity-rotation camera this reproduces the canonical
    ///   `right (1,0,0) / up (0,1,0) / forward (0,0,−1)` exactly — the prior
    ///   hardcoded marcher basis.
    ///
    /// Pure / alloc-free / FMA-free (delegates to the math vocabulary). Returns
    /// the identity-rotation, eye-at-`translation` fallback view if the camera's
    /// linear part is singular (a non-invertible / degenerate transform).
    ///
    /// # Degenerate-basis fallback
    ///
    /// The three basis lanes get the same treatment `view` already had: a linear
    /// part that cannot produce a usable axis falls back to the canonical
    /// `right (1,0,0) / up (0,1,0) / forward (0,0,−1)` triple — see
    /// [`Self::basis_axis_is_usable`] for why an unusable axis must not be
    /// uploaded, and why the substitution is all-or-nothing.
    #[inline]
    pub fn from_camera(global: Affine3A, projection: Projection) -> Self {
        // Local camera axes (column convention): right +X, up +Y, forward −Z.
        // These double as the CANONICAL WORLD fallback triple below: they are the
        // same three vectors [`Self::IDENTITY`] carries, so a degenerate camera
        // publishes exactly the identity view's basis — matching the identity
        // `view` fallback a singular `global.inverse()` already takes.
        const LOCAL_RIGHT: Vec3 = Vec3::new(1.0, 0.0, 0.0);
        const LOCAL_UP: Vec3 = Vec3::new(0.0, 1.0, 0.0);
        const LOCAL_FORWARD: Vec3 = Vec3::new(0.0, 0.0, -1.0);

        let linear: Mat3 = global.matrix3;
        let eye: Vec3 = global.translation;

        // World-space basis = world image of each local axis (row-major mul_vec).
        // Normalized as belt-and-suspenders against a uniform-scaled camera; for
        // a rigid (orthonormal) camera the inputs are already unit, so this is a
        // no-op up to floating point and reproduces the hardcoded basis exactly.
        let cam_right = linear.mul_vec(LOCAL_RIGHT).normalize();
        let cam_up = linear.mul_vec(LOCAL_UP).normalize();
        let cam_forward = linear.mul_vec(LOCAL_FORWARD).normalize();

        // ALL-OR-NOTHING, not per-axis: a mixed triple (two real axes plus one
        // canonical substitute) is not guaranteed to be a BASIS at all — a camera
        // whose real `right` happens to be (0,0,−1) paired with the canonical
        // forward (0,0,−1) is rank-deficient, so `dir` can still vanish. Falling
        // back to the whole canonical triple is the only substitution that keeps
        // the published basis orthonormal.
        let (cam_right, cam_up, cam_forward) = if Self::basis_axis_is_usable(cam_right)
            && Self::basis_axis_is_usable(cam_up)
            && Self::basis_axis_is_usable(cam_forward)
        {
            (cam_right, cam_up, cam_forward)
        } else {
            (LOCAL_RIGHT, LOCAL_UP, LOCAL_FORWARD)
        };

        let proj = projection.to_mat4();
        // `view = global⁻¹`. The affine inverse handles the general rigid +
        // uniform-scale case; on a singular linear part fall back to identity
        // (the degenerate camera renders the identity view rather than NaN).
        let view = match global.inverse() {
            Some(inv) => inv.to_mat4(),
            None => Mat4::IDENTITY,
        };
        let view_proj = proj.mul_mat4(view);

        Self {
            view_proj,
            inv_view: global.to_mat4(),
            camera_pos: Vec4::from_vec3(eye, 1.0),
            cam_forward: Vec4::from_vec3(cam_forward, 0.0),
            cam_right: Vec4::from_vec3(cam_right, 0.0),
            cam_up: Vec4::from_vec3(cam_up, 0.0),
            fov_y: projection.fov_y(),
            aspect: projection.aspect(),
            near: projection.near(),
            far: projection.far(),
        }
    }
}

impl Default for ViewUniform {
    /// The default view is [`ViewUniform::IDENTITY`].
    #[inline]
    fn default() -> Self {
        Self::IDENTITY
    }
}

/// The world-space cursor ray for a CONTINUOUS logical-pixel sample `(px, py)` at
/// logical viewport extent `(vp_w, vp_h)` under `view` — the exact inverse of
/// `boyko_ui`'s `project_world_to_screen` and a bit-mirror of the SDF marcher's
/// perspective ray-gen (`composite_ray`), so a pick aligns pixel-accurately with
/// what the marcher renders.
///
/// `(px, py)` are in LOGICAL pixels in the `UiViewport` / `ComputedRect` basis
/// (+x right, +y DOWN). They are a CONTINUOUS sample (a cursor position, NOT a
/// pixel center): `ndc_x = px / vp_w * 2 - 1` with NO `+0.5`. This makes
/// `camera_ray` the exact inverse of `project_world_to_screen` (which uses the same
/// `ndc = coord / extent * 2 - 1` + y-flip). Pass `px + 0.5` for an integer pixel
/// `px` to reproduce the marcher's pixel-CENTER ray (what the cross-check golden
/// does).
///
/// # PERSPECTIVE (`view.fov_y != 0.0`)
///
/// `origin = eye`, `dir = normalize(forward + right * sx + up * sy)`, with
/// `aspect = vp_w / vp_h` DERIVED FROM THE HANDED VIEWPORT EXTENT — NOT
/// [`ViewUniform::aspect`]. The marcher's `CompositeCamera::Perspective.aspect` is
/// itself `w / h` from the push constants, so deriving it here from the same
/// viewport extent is the faithful mirror and is immune to a stale `view.aspect`.
/// [`Vec3::normalize`] guards a zero `dir` (→ [`Vec3::ZERO`]) where the marcher's
/// raw `sqrt`+divide does not; for a valid forward camera `dir` is never zero, so
/// they agree to f32 epsilon on the pixels the cross-check exercises (the guard
/// only diverges on a degenerate camera, where the marcher emits a non-finite ray
/// anyway — documented, not a bug).
///
/// # ORTHOGRAPHIC (`view.fov_y == 0.0`)
///
/// Best-effort, camera-driven: `origin = eye + right * ndc_x + up * ndc_y` (a UNIT
/// half-extent placeholder), `dir = forward`. The ortho half-extents are not stored
/// as scalars on [`ViewUniform`], so a marcher-accurate ortho ray is impossible
/// from the view alone — and the marcher's ortho arm uses FIXED legacy constants,
/// NOT the camera. **Ortho pick is therefore APPROXIMATE and does NOT match the
/// marcher's ortho fixture.** P7b targets perspective (the screenshot scene is
/// perspective); a future phase that adds ortho half-extents to [`ViewUniform`]
/// upgrades this arm.
#[inline]
pub fn camera_ray(view: &ViewUniform, px: f32, py: f32, vp_w: f32, vp_h: f32) -> Ray {
    // A zero-extent viewport would explode the NDC divide; the caller (the pick
    // system) guards it, but trip it in debug for an out-of-contract host.
    debug_assert!(
        vp_w > 0.0 && vp_h > 0.0,
        "camera_ray: viewport extent must be > 0 (vp_w {vp_w}, vp_h {vp_h})"
    );

    // NDC, identical to `project_world_to_screen`'s round-trip: +y down on screen
    // is +y up in NDC, so the y term is flipped.
    let ndc_x = (px / vp_w) * 2.0 - 1.0;
    let ndc_y = -((py / vp_h) * 2.0 - 1.0);

    let eye = view.camera_pos.xyz();
    let fwd = view.cam_forward.xyz();
    let right = view.cam_right.xyz();
    let up = view.cam_up.xyz();

    if view.fov_y != 0.0 {
        // PERSPECTIVE: aspect from the HANDED extent (NOT view.aspect) — the
        // marcher's payload aspect is itself w/h from the same viewport.
        let tan_half = (view.fov_y * 0.5).tan();
        let aspect = vp_w / vp_h;
        let sx = ndc_x * aspect * tan_half;
        let sy = ndc_y * tan_half;
        let dir = (fwd + right * sx + up * sy).normalize();
        Ray::new(eye, dir)
    } else {
        // ORTHOGRAPHIC: best-effort, unit-half-extent placeholder (documented
        // approximate; does NOT match the marcher's fixed-constant ortho fixture).
        let origin = eye + right * ndc_x + up * ndc_y;
        Ray::new(origin, fwd)
    }
}

/// Resolves the active camera and derives [`ViewUniform`] from it (S3).
///
/// Per-frame, alloc-free, `.after(propagate_transforms)` (so the camera's
/// `GlobalTransform` is the freshly-propagated world pose). Selection:
///
/// 1. If [`ActiveCamera`] holds `Some(entity)` AND that entity is a live camera
///    (has [`Camera`] + [`Projection`] + [`GlobalTransform`]), use it.
/// 2. Otherwise, pick the **highest-`order`** [`Camera`] with `is_active` set —
///    an alloc-free iterate-and-track-max (no `collect`/`sort`).
///
/// On no camera at all, [`ViewUniform`] is left at its identity default (the
/// renderer draws the identity view rather than panicking).
///
/// # Camera-transform invariant
///
/// A camera's `GlobalTransform` is constrained to **rigid + uniform-scale**. A
/// sheared / non-uniform-scaled camera parent would distort the view; a
/// `debug_assert!` (release-free) catches it via [`debug_assert_camera_rigid`].
//
// `clippy::needless_pass_by_value`: `Res` / `ResMut` are by-value `SystemParam`s
// read/written through reborrows — the same false-positive the light / physics
// systems carry. The camera + projection + global query is one read-only query;
// the explicit override is `Res`, the derived view is `ResMut`.
#[allow(clippy::needless_pass_by_value)]
pub fn resolve_active_camera(
    cameras: Query<(&Camera, &Projection, &GlobalTransform)>,
    active: Res<ActiveCamera>,
    mut view: ResMut<ViewUniform>,
) {
    // 1) Explicit override: if the resource names an entity, find it among the
    //    camera rows by id and derive its view. A named-but-non-camera entity
    //    falls through to the policy pass (no silent stale view).
    if let Some(target) = active.0 {
        for (entity_id, (_cam, projection, global)) in cameras.iter_entities() {
            if entity_id == target.id() {
                debug_assert_camera_rigid(global.0);
                *view = ViewUniform::from_camera(global.0, *projection);
                return;
            }
        }
    }

    // 2) Policy: the highest-`order` `is_active` camera. Iterate-and-track-max in
    //    registers — no allocation, single pass.
    let mut best: Option<(i32, Projection, GlobalTransform)> = None;
    for (cam, projection, global) in cameras.iter() {
        if !cam.is_active {
            continue;
        }
        let take = match best {
            None => true,
            Some((best_order, _, _)) => cam.order > best_order,
        };
        if take {
            best = Some((cam.order, *projection, *global));
        }
    }

    if let Some((_order, projection, global)) = best {
        debug_assert_camera_rigid(global.0);
        *view = ViewUniform::from_camera(global.0, projection);
    }
    // No active camera: leave `ViewUniform` at its prior / identity value.
}

/// Debug-only check that a camera's world linear part is rigid up to a uniform
/// scale (mutually-orthogonal, equal-length rows). A sheared / non-uniform
/// camera trips this in debug builds; it compiles to nothing in release.
#[inline]
fn debug_assert_camera_rigid(global: Affine3A) {
    #[cfg(debug_assertions)]
    {
        const ORTHO_EPS: f32 = 1e-3;
        const LEN_EPS: f32 = 1e-3;
        let r = &global.matrix3.rows;
        let l0 = r[0].length_squared();
        let l1 = r[1].length_squared();
        let l2 = r[2].length_squared();
        // Equal row lengths (uniform scale) — compare squared lengths.
        debug_assert!(
            (l0 - l1).abs() <= LEN_EPS * (1.0 + l0) && (l0 - l2).abs() <= LEN_EPS * (1.0 + l0),
            "invariant: camera GlobalTransform must be uniform-scaled (equal basis \
             lengths); a non-uniform camera scale distorts the view"
        );
        // Mutual orthogonality of the basis rows (no shear).
        let d01 = r[0].dot(r[1]);
        let d02 = r[0].dot(r[2]);
        let d12 = r[1].dot(r[2]);
        debug_assert!(
            d01.abs() <= ORTHO_EPS * (1.0 + l0)
                && d02.abs() <= ORTHO_EPS * (1.0 + l0)
                && d12.abs() <= ORTHO_EPS * (1.0 + l0),
            "invariant: camera GlobalTransform must be unsheared (orthogonal basis \
             rows); a sheared camera distorts the view"
        );
    }
    let _ = global;
}

/// Orbit-camera RIG: the camera entity's pose is DERIVED from these fields by
/// [`orbit_camera_system`] — the eye orbits `target` on a sphere of radius
/// `distance`, oriented to look AT `target`.
///
/// The rig is **pure state**: a caller (an example loop, an input system, an
/// animation) advances `yaw` / `pitch`; the system only re-derives the
/// [`Transform`]. Keeping the motion in the caller makes the system a
/// deterministic, policy-free kernel (no hidden `Res<Time>`), and lets any
/// driver compose the orbit. Principle 0: a component on ECS storage + a system,
/// never a side data store.
///
/// `#[repr(C)]` POD (24 B, natural `f32` alignment), `Copy`. All fields are read
/// together once per camera per frame, so there is no hot/cold split.
///
/// # Required components
///
/// `#[require(Transform, GlobalTransform)]` is a convenience: inserting an
/// `OrbitCamera` alone auto-inserts the pose columns [`orbit_camera_system`]
/// writes and [`propagate_transforms`](crate::propagation::propagate_transforms)
/// needs. It is a pure ergonomic nicety — a rig camera is normally spawned with
/// the explicit `Camera + Projection + OrbitCamera + Transform + GlobalTransform`
/// list, which depends on nothing here.
#[repr(C)]
#[derive(Component, Clone, Copy, Debug, PartialEq)]
#[require(Transform, GlobalTransform)]
pub struct OrbitCamera {
    /// World-space point the camera orbits and looks at.
    pub target: [f32; 3],
    /// Orbit radius (eye distance from `target`). Guarded to
    /// [`MIN_DISTANCE`](Self::MIN_DISTANCE) at read so a zero / negative radius
    /// cannot collapse the look-at (`eye == target` → singular basis).
    pub distance: f32,
    /// Azimuth, radians. `yaw == 0` places the eye on `+Z` of `target`; `+yaw`
    /// sweeps the eye `+Z → +X`.
    pub yaw: f32,
    /// Elevation, radians. `pitch == 0` is level (eye in the `target` `XZ` plane);
    /// `+pitch` raises the eye toward `+Y`. CLAMPED to
    /// `±`[`PITCH_LIMIT`](Self::PITCH_LIMIT) at read so the look direction is never
    /// collinear with world-up (which would make the right axis degenerate).
    pub pitch: f32,
}

// Layout pin (house style — cf. `Transform`, `CameraUniform`). 12 + 4 + 4 + 4 =
// 24 B. A change here is a deliberate decision, not an accident.
const _: () = assert!(size_of::<OrbitCamera>() == 24);

impl OrbitCamera {
    /// Pole guard margin (radians). Keeps the clamped pitch strictly off the
    /// `±π/2` poles where the look direction would be collinear with world-up.
    pub const PITCH_EPS: f32 = 1.0e-3;

    /// The pitch clamp bound: `π/2 − `[`PITCH_EPS`](Self::PITCH_EPS). At read,
    /// `pitch` is clamped to `±PITCH_LIMIT`, so the eye never reaches the pole and
    /// the look-at right axis (`up × back`) never collapses.
    pub const PITCH_LIMIT: f32 = core::f32::consts::FRAC_PI_2 - Self::PITCH_EPS;

    /// The minimum orbit radius. At read, `distance` is clamped to at least this,
    /// so `eye != target` and the look-at `back` axis is non-zero.
    pub const MIN_DISTANCE: f32 = 1.0e-4;

    /// Constructs a rig orbiting `target` at `distance`, with the given `yaw` /
    /// `pitch` (radians). Fields are stored verbatim; the clamps are applied by
    /// [`orbit_camera_system`] at read (the rig fields stay author-owned).
    #[inline]
    pub const fn new(target: [f32; 3], distance: f32, yaw: f32, pitch: f32) -> Self {
        Self {
            target,
            distance,
            yaw,
            pitch,
        }
    }
}

impl Default for OrbitCamera {
    /// A sensible default rig: orbits the origin at `distance == 5`, head-on
    /// (`yaw == 0`, `pitch == 0`) — the eye on `+Z` of the origin, looking `−Z`.
    #[inline]
    fn default() -> Self {
        Self::new([0.0, 0.0, 0.0], 5.0, 0.0, 0.0)
    }
}

/// Derives each [`OrbitCamera`] entity's local [`Transform`] from its rig fields:
/// the eye sits on the orbit sphere and looks AT `target`.
///
/// PURE rig fields → pose. It does NOT advance `yaw` / `pitch` itself —
/// animation / input is the caller's loop (e.g. `rig.yaw += dt * omega`). For
/// each rig:
///
/// 1. Read the CLAMPED `pitch` (`±`[`PITCH_LIMIT`](OrbitCamera::PITCH_LIMIT)) and
///    `distance` (`≥ `[`MIN_DISTANCE`](OrbitCamera::MIN_DISTANCE)) — the clamp
///    keeps the pose finite without mutating the author-owned `&OrbitCamera`.
/// 2. `eye = target + distance · (cos·sin(yaw), sin(pitch), cos·cos(yaw))`, so
///    `yaw == 0, pitch == 0` ⇒ `eye == target + (0, 0, distance)` (on `+Z`,
///    looking `−Z` at `target`); `+yaw` sweeps the eye toward `+X`; `+pitch`
///    raises it toward `+Y`.
/// 3. Build the rigid camera world transform with
///    [`Affine3A::look_at_rh`](boyko_math::Affine3A::look_at_rh) (world up `+Y`),
///    and write `Transform { translation: eye, rotation: Quat::from_mat3(basis),
///    scale: ONE }`.
///
/// # Change detection
///
/// The pose is written through the change-tracking
/// [`Mut`](boyko_ecs::ecs::core::iters::query::data::Mut)`<Transform>` query
/// guard, NOT a bare `&mut Transform`. In THIS engine a `&mut T` query term does
/// NOT stamp the row's `changed_tick` (its `QueryData` impl declares
/// `NEEDS_CHANGE_DETECTION = false`); the tick stamp lives in the `Mut<T>` guard's
/// `DerefMut`. Writing through the guard (`*transform = …`) stamps `changed_tick`
/// directly, so [`propagate_transforms`](crate::propagation::propagate_transforms)
/// — which dirty-gates on `Transform.changed_tick` — sees the camera dirty and
/// recomposes its [`GlobalTransform`] the SAME frame. No manual
/// `world.bump_change_tick()` is needed under a real `Schedule::run` frame.
///
/// # Schedule order
///
/// Runs `.before(propagate_transforms)` so the same-frame
/// [`resolve_active_camera`] (`.after(propagate_transforms)`) sees the new pose.
//
// `clippy::needless_pass_by_value`: `Query` is a by-value `SystemParam`
// reborrowed internally — the same false-positive `resolve_active_camera` carries.
#[allow(clippy::needless_pass_by_value)]
pub fn orbit_camera_system(mut rigs: Query<(&OrbitCamera, Mut<Transform>)>) {
    for (rig, mut transform) in rigs.iter_mut() {
        // Clamp for the math only; the rig fields stay author/animation-owned.
        let pitch = rig
            .pitch
            .clamp(-OrbitCamera::PITCH_LIMIT, OrbitCamera::PITCH_LIMIT);
        let dist = rig.distance.max(OrbitCamera::MIN_DISTANCE);

        let (sp, cp) = pitch.sin_cos();
        let (sy, cy) = rig.yaw.sin_cos();

        let target = Vec3::new(rig.target[0], rig.target[1], rig.target[2]);
        // Eye on the orbit sphere: yaw rotates in the XZ plane (+Z at yaw 0),
        // pitch lifts toward +Y. cos(pitch) shrinks the horizontal radius as the
        // eye climbs, keeping |eye − target| == dist.
        let offset = Vec3::new(dist * cp * sy, dist * sp, dist * cp * cy);
        let eye = target + offset;

        let world = Affine3A::look_at_rh(eye, target, Vec3::new(0.0, 1.0, 0.0));

        *transform = Transform {
            translation: world.translation,
            rotation: Quat::from_mat3(world.matrix3),
            scale: Vec3::ONE,
        };
    }
}

/// A first-person FLY camera controller (host plan R6): the camera entity's pose
/// is driven each frame from the [`PhysicalInput`] snapshot by
/// [`fly_camera_system`] — mouse-look accumulates `yaw` / `pitch`, and WASD +
/// vertical keys translate the eye along the derived orthonormal basis.
///
/// Unlike [`OrbitCamera`] (pure rig state the caller advances), `FlyCamera` OWNS
/// its own drive: `fly_camera_system` reads the engine's per-frame input snapshot
/// plus [`Time`] and both mutates the accumulators AND writes the entity's
/// [`Transform`]. The interactive quit binding and the OS→ECS input bridge live
/// in the host (`boyko_app::FlyCameraPlugin`); this component + system are the
/// windowing-independent core, testable headless over a synthetic snapshot.
///
/// # Parity (the `run_interactive_viewer` fly math)
///
/// The defaults + the basis formula reproduce the reference interactive viewer
/// (`boyko_rhi_vulkan`'s `run_interactive_viewer`): sensitivity `0.0035`,
/// pitch-clamp `±1.5533`, move speed `3.5 u/s`, and `forward(yaw=0, pitch=0) ==
/// [0, 0, -1]` (the `-Z` look).
///
/// `#[repr(C)]` POD (16 B, natural `f32` alignment), `Copy`. All four fields are
/// read/written together once per frame — no hot/cold split.
///
/// # Required components
///
/// `#[require(Transform, GlobalTransform)]` auto-inserts the pose columns
/// `fly_camera_system` writes and
/// [`propagate_transforms`](crate::propagation::propagate_transforms) needs, so
/// inserting a `FlyCamera` alone is well-formed. A camera is normally spawned via
/// [`FlyCameraBundle`](crate::bundles::FlyCameraBundle) with the explicit
/// `Camera + Projection + FlyCamera + Transform + GlobalTransform` list.
#[repr(C)]
#[derive(Component, Clone, Copy, Debug, PartialEq)]
#[require(Transform, GlobalTransform)]
pub struct FlyCamera {
    /// Azimuth, radians. `yaw == 0` looks down `-Z`; `+yaw` sweeps the look
    /// toward `+X`. Accumulated from the summed mouse-X delta each frame.
    pub yaw: f32,
    /// Elevation, radians. `pitch == 0` is level; `+pitch` looks up. CLAMPED to
    /// `±`[`PITCH_LIMIT`](Self::PITCH_LIMIT) each frame so the look direction is
    /// never collinear with world-up (which would collapse the right axis).
    pub pitch: f32,
    /// Planar/vertical move speed in world units per second.
    pub speed: f32,
    /// Mouse-look sensitivity (radians per raw mouse-delta unit).
    pub sensitivity: f32,
}

// Layout pin (house style — cf. `OrbitCamera`, `Transform`). 4 × 4 = 16 B. A
// change here is a deliberate decision, not an accident.
const _: () = assert!(size_of::<FlyCamera>() == 16);

impl FlyCamera {
    /// The pitch clamp bound (radians), matching the reference viewer's
    /// `VIEWER_PITCH_LIMIT`. Keeps the look direction strictly off the `±π/2`
    /// poles where the right axis (`forward × up`) would collapse.
    pub const PITCH_LIMIT: f32 = 1.5533;

    /// Default mouse-look sensitivity (radians per raw mouse-delta unit),
    /// matching the reference viewer's `VIEWER_LOOK_SENS`.
    pub const DEFAULT_SENSITIVITY: f32 = 0.0035;

    /// Default move speed (world units per second), matching the reference
    /// viewer's `VIEWER_MOVE_SPEED`.
    pub const DEFAULT_SPEED: f32 = 3.5;

    /// Constructs a fly camera with the given initial `yaw` / `pitch` (radians)
    /// and the default [`DEFAULT_SPEED`](Self::DEFAULT_SPEED) /
    /// [`DEFAULT_SENSITIVITY`](Self::DEFAULT_SENSITIVITY).
    #[inline]
    pub const fn new(yaw: f32, pitch: f32) -> Self {
        Self {
            yaw,
            pitch,
            speed: Self::DEFAULT_SPEED,
            sensitivity: Self::DEFAULT_SENSITIVITY,
        }
    }
}

impl Default for FlyCamera {
    /// The reference viewer's start orientation: `yaw == 0` (looking down `-Z`)
    /// and `pitch == -0.3805` (`VIEWER_INITIAL_PITCH` — a slight downward tilt),
    /// with the default speed + sensitivity.
    #[inline]
    fn default() -> Self {
        Self::new(0.0, -0.3805)
    }
}

/// Drives every [`FlyCamera`] entity's [`Transform`] from the per-frame
/// [`PhysicalInput`] snapshot + the virtual [`Time`] delta (host plan R6).
///
/// Per fly camera (typically one):
///
/// 1. `dt = time.delta_secs()` — the VIRTUAL delta (clamped / pausable / scaled;
///    ZERO while paused). It tracks the raw delta at normal FPS, so movement is
///    parity-faithful, but a hitch or a pause cannot fling the camera.
/// 2. Accumulate look: `yaw += mouse_delta.x · sensitivity`,
///    `pitch -= mouse_delta.y · sensitivity`, then clamp `pitch` to
///    `±`[`PITCH_LIMIT`](FlyCamera::PITCH_LIMIT). The accumulators are written
///    back to the component.
/// 3. Build the orthonormal basis: `forward = norm([sy·cp, sp, -cy·cp])`,
///    `right = norm(forward × up)`, `up = right × forward` (world up `+Y`), so
///    `forward(0, 0) == [0, 0, -1]`.
/// 4. Sum the movement axes from the LEVEL key bitset (`keys_pressed`):
///    `W/S → ±forward`, `D/A → ±right`, `Space/E → +up`, `ControlLeft/Q → -up`;
///    `eye = translation + norm_or_zero(move) · (speed · dt)`.
/// 5. Write the pose through [`Mut<Transform>`](boyko_ecs::ecs::core::iters::query::data::Mut):
///    `rotation = Quat::from_mat3(from_columns(right, up, -forward))` (the
///    camera-world column convention `ViewUniform::from_camera` reads),
///    `translation = eye`.
///
/// # Change detection (the load-bearing pin)
///
/// The pose is written through the change-tracking `Mut<Transform>` guard, NOT a
/// bare `&mut Transform`. In THIS engine a `&mut T` query term does NOT stamp the
/// row's `changed_tick` (`NEEDS_CHANGE_DETECTION = false`); only the `Mut<T>`
/// guard's `DerefMut` does. `propagate_transforms` dirty-gates on
/// `Transform.changed_tick`, so writing through the guard recomposes the camera's
/// `GlobalTransform` the SAME frame (no one-frame view lag — cf. BUG-S35-1).
///
/// # Schedule order
///
/// Joins `CameraSet::Control` (via `boyko_app::FlyCameraPlugin`), which
/// [`CameraPlugin`](crate::camera_plugin::CameraPlugin) orders
/// `.before(CameraSet::Resolve)` — the propagation + resolve pair — so the
/// same-frame view reflects this write.
//
// `clippy::needless_pass_by_value`: `Res` / `Query` are by-value `SystemParam`s
// reborrowed internally — the same false-positive the orbit / resolve systems
// carry.
#[allow(clippy::needless_pass_by_value)]
pub fn fly_camera_system(
    time: Res<Time>,
    input: Res<PhysicalInput>,
    mut cams: Query<(&mut FlyCamera, Mut<Transform>)>,
) {
    let dt = time.delta_secs();
    for (fly, mut transform) in cams.iter_mut() {
        // Write through Mut<Transform> (stamps changed_tick — the pin). The
        // per-entity math is the pure `fly_step` (alloc-free, unit-tested).
        *transform = fly_step(fly, transform.translation, &input, dt);
    }
}

/// The pure per-entity fly step (steps 2–5): mutate the [`FlyCamera`]
/// accumulators from the input snapshot and return the new [`Transform`] for the
/// camera whose current eye is `translation`.
///
/// Extracted from [`fly_camera_system`] so it is alloc-free-testable over
/// borrowed state without the scheduler's per-run scaffolding (the `ingest_body`
/// pattern from `boyko_input`'s zero-alloc gate). Pure + branch-only: no heap.
#[inline]
pub(crate) fn fly_step(
    fly: &mut FlyCamera,
    translation: Vec3,
    input: &PhysicalInput,
    dt: f32,
) -> Transform {
    const WORLD_UP: Vec3 = Vec3::new(0.0, 1.0, 0.0);

    debug_assert!(
        fly.sensitivity >= 0.0 && fly.speed >= 0.0,
        "invariant: FlyCamera speed/sensitivity must be non-negative"
    );

    // 2) Accumulate mouse-look; clamp pitch off the poles. `mouse_delta` is
    //    f64 (raw relative motion); cast to f32 for the basis math.
    fly.yaw += input.mouse_delta[0] as f32 * fly.sensitivity;
    fly.pitch -= input.mouse_delta[1] as f32 * fly.sensitivity;
    fly.pitch = fly
        .pitch
        .clamp(-FlyCamera::PITCH_LIMIT, FlyCamera::PITCH_LIMIT);
    debug_assert!(
        fly.pitch.abs() <= FlyCamera::PITCH_LIMIT,
        "invariant: FlyCamera pitch stays within the clamp"
    );

    // 3) FPS basis: forward at (yaw=0, pitch=0) is [0, 0, -1].
    let (sy, cy) = fly.yaw.sin_cos();
    let (sp, cp) = fly.pitch.sin_cos();
    let forward = Vec3::new(sy * cp, sp, -cy * cp).normalize();
    let right = forward.cross(WORLD_UP).normalize();
    let up = right.cross(forward);

    // 4) Sum the pressed movement axes over the LEVEL key bitset.
    let held = |code: KeyCode| {
        code.dense_index()
            .is_some_and(|i| input.keys_pressed.get(i))
    };
    let mut mv = Vec3::ZERO;
    if held(KeyCode::KeyW) {
        mv = mv + forward;
    }
    if held(KeyCode::KeyS) {
        mv = mv - forward;
    }
    if held(KeyCode::KeyD) {
        mv = mv + right;
    }
    if held(KeyCode::KeyA) {
        mv = mv - right;
    }
    if held(KeyCode::Space) || held(KeyCode::KeyE) {
        mv = mv + WORLD_UP;
    }
    if held(KeyCode::ControlLeft) || held(KeyCode::KeyQ) {
        mv = mv - WORLD_UP;
    }
    debug_assert!(mv.is_finite(), "invariant: FlyCamera move vector is finite");

    // Diagonal-normalized planar speed (no diagonal boost); a zero vector stays
    // put. Virtual dt so a hitch/pause does not overshoot.
    let step = mv.normalize() * (fly.speed * dt);
    let eye = translation + step;

    // 5) The camera-world basis is column-major: local +X = right, +Y = up,
    //    +Z = back = -forward (the `look_at_rh` / `from_camera` convention).
    //    `Vec3` has no `Neg`, so back is `forward * -1.0`.
    let basis = Mat3::from_columns(right, up, forward * -1.0);
    Transform {
        translation: eye,
        rotation: Quat::from_mat3(basis),
        scale: Vec3::ONE,
    }
}

#[cfg(test)]
mod fly_tests {
    //! Alloc-free gate for the pure fly step (host plan R6, Test 6 — fly half).
    //! Co-located so it can reach the `pub(crate)` `fly_step` without a scheduler.
    //!
    //! Counting is THREAD-LOCAL (not a global flag): the process-global allocator
    //! sees allocations from every parallel test thread, so a global flag would
    //! fold sibling threads' allocations into this measurement (a flaky
    //! over-count). A thread-local flag counts only the measuring thread.

    use std::alloc::{GlobalAlloc, Layout, System};
    use std::cell::Cell;

    use super::*;

    thread_local! {
        static COUNTING: Cell<bool> = const { Cell::new(false) };
        static ACQUISITIONS: Cell<usize> = const { Cell::new(0) };
    }

    struct CountingAlloc;

    #[inline]
    fn note_alloc() {
        let _ = COUNTING.try_with(|c| {
            if c.get() {
                let _ = ACQUISITIONS.try_with(|a| a.set(a.get() + 1));
            }
        });
    }

    // SAFETY: pure delegation to `System` with a thread-local counter side-effect;
    // the layout/pointer contracts are forwarded unchanged.
    unsafe impl GlobalAlloc for CountingAlloc {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            note_alloc();
            // SAFETY: forwarded verbatim to the system allocator.
            unsafe { System.alloc(layout) }
        }
        unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
            note_alloc();
            // SAFETY: forwarded verbatim to the system allocator.
            unsafe { System.alloc_zeroed(layout) }
        }
        unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
            note_alloc();
            // SAFETY: forwarded verbatim to the system allocator.
            unsafe { System.realloc(ptr, layout, new_size) }
        }
        unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
            // SAFETY: forwarded verbatim to the system allocator.
            unsafe { System.dealloc(ptr, layout) }
        }
    }

    #[global_allocator]
    static ALLOC: CountingAlloc = CountingAlloc;

    /// The pure fly step allocates ZERO heap — it is stack POD + branch-only.
    #[test]
    fn fly_step_is_alloc_free() {
        let mut input = PhysicalInput::new();
        input.keys_pressed.set(KeyCode::KeyW.dense_index().unwrap());
        input.keys_pressed.set(KeyCode::KeyD.dense_index().unwrap());
        input.mouse_delta = [12.0, -7.0];
        let mut fly = FlyCamera::new(0.2, 0.1);
        let mut pos = Vec3::new(0.0, 1.0, 5.0);

        // Warm (keeps parity with the `ingest_body` gate shape).
        for _ in 0..4 {
            let t = fly_step(&mut fly, pos, &input, 0.016);
            pos = t.translation;
        }

        ACQUISITIONS.with(|a| a.set(0));
        COUNTING.with(|c| c.set(true));
        let t = fly_step(&mut fly, pos, &input, 0.016);
        COUNTING.with(|c| c.set(false));
        core::hint::black_box(t);

        assert_eq!(
            ACQUISITIONS.with(|a| a.get()),
            0,
            "fly_step must not allocate"
        );
    }
}
