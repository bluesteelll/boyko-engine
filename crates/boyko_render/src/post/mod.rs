//! Render wave PXe / PX1a — the host side of the post chain.
//!
//! - [`gpu::PostFinalGpu`] (PXe): the `post_final` pass's constant buffer, pinned to the
//!   `boyko_shaderdsl::post::POST_FINAL_UBO_LAYOUT` table the generated shader's `cbuffer` is
//!   printed from.
//! - [`PostPlugin`] (PX1a, AD-6): the ECS owner of the scene's **pre-exposure**. Every `lit` producer,
//!   the particle draw, the TAA history ratio and `post_final` read ONE value, [`ResolvedPost`],
//!   written by ONE system, [`resolve_post_policy`], in [`PostPolicySet`]. PX3 grows the plugin with
//!   auto-exposure, PX4 with bloom, PX5 with grading.
//!
//! [`CameraCut`] is the gameplay request for a history reset (a teleport, a cut in a cinematic); an
//! active-camera switch cuts by itself.

pub mod gpu;

use boyko_ecs::ecs::core::app::{App, Plugin};
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::core::iters::query::Query;
use boyko_ecs::ecs::core::system::{Entities, EventReader, Local, Res, ResMut};
use boyko_ecs::ecs::identifiers::primitives::EntityId;
use boyko_macros::{Resource, SystemSet, event};
use boyko_scene::{ActiveCamera, Camera, CameraSet, GlobalTransform, Projection};

use crate::light::LightingConfig;

/// The resolved post settings of the frame — the one pre-exposure every consumer reads.
///
/// Written by exactly one system, [`resolve_post_policy`]; the runner reads it through
/// `try_resource` (a host without [`PostPlugin`] falls back to [`ResolvedPost::from_config`]).
#[repr(C)]
#[derive(Resource, Clone, Copy, Debug, PartialEq)]
pub struct ResolvedPost {
    /// `pre_N`: the exposure the `lit` producers bake into scene radiance this frame
    /// (`LightingConfig::exposure` until PX3's auto-exposure).
    pub pre_exposure: f32,
    /// `pre_N / pre_{N−1}` — the factor a history written last frame is rescaled by; exactly
    /// `1.0` on frame 0 and on a cut.
    pub pre_ratio: f32,
    /// `pre_{N−1}`, carried for the ratio.
    pub prev_pre_exposure: f32,
    /// The [`Tonemapper`](crate::light::Tonemapper) word `post_final` applies.
    pub tonemapper: u32,
    /// `1` on the frame a cut takes effect (a [`CameraCut`] request or an active-camera switch).
    pub cut: u32,
    /// The active camera, index AND generation: a despawned camera whose index the next camera
    /// reuses is a different camera and cuts.
    pub active_camera: Option<Entity>,
}

// H4 budgeted <= 40 B with a 4-B entity; this tree's `Entity` is 16 B (`EntityId(usize)` + a `u32`
// generation), so `Option<Entity>` is 24 B and the struct 48 B. A cold, once-a-frame resource:
// the bound only keeps it from growing unnoticed.
const _: () = assert!(size_of::<ResolvedPost>() <= 48);

impl ResolvedPost {
    /// What [`resolve_post_policy`] writes for `cfg` with no active camera and no cut — the
    /// runner's fallback for a host that composes no [`PostPlugin`]. Pure.
    pub const fn from_config(cfg: &LightingConfig) -> Self {
        Self {
            pre_exposure: cfg.exposure,
            pre_ratio: 1.0,
            prev_pre_exposure: cfg.exposure,
            tonemapper: cfg.tonemapper as u32,
            cut: 0,
            active_camera: None,
        }
    }

    /// The policy step: the frame after `self` under `cfg`, with this frame's resolved
    /// `active_camera` and whether a [`CameraCut`] was read. Pure — [`resolve_post_policy`] is this
    /// plus the reads.
    ///
    /// A cut (`cut_requested`, or an `active_camera` that differs from `self`'s in index OR
    /// generation — acquiring or losing the camera counts) drops the ratio to exactly `1.0`: the
    /// history it would rescale is replaced on that frame.
    pub fn next(&self, cfg: &LightingConfig, active_camera: Option<Entity>, cut_requested: bool) -> Self {
        let pre_exposure = cfg.exposure;
        let cut = cut_requested || active_camera != self.active_camera;
        Self {
            pre_exposure,
            pre_ratio: if cut { 1.0 } else { pre_exposure / self.pre_exposure },
            prev_pre_exposure: self.pre_exposure,
            tonemapper: cfg.tonemapper as u32,
            cut: u32::from(cut),
            active_camera,
        }
    }
}

impl Default for ResolvedPost {
    /// [`ResolvedPost::from_config`] of the default [`LightingConfig`] (exposure `1.0`): the value a
    /// consumer reads before the first policy run.
    fn default() -> Self {
        Self::from_config(&LightingConfig::default())
    }
}

/// The set [`resolve_post_policy`] runs in: every reader of [`ResolvedPost`] orders itself
/// `.after_set(PostPolicySet)` (a set-to-set edge holds whatever the plugin add-order).
#[derive(SystemSet, Clone, Copy, PartialEq, Eq, Debug)]
pub struct PostPolicySet;

/// A gameplay request for a temporal-history reset.
///
/// **Visibility is next-frame** (the kernel event lane, Model B): a cut sent in frame N is read by
/// [`resolve_post_policy`] in frame N+1. Send it the frame BEFORE a same-camera teleport, or
/// accept one frame of history across the jump. Switching the active camera needs no event — it
/// cuts the same frame.
#[event]
pub struct CameraCut {
    /// A caller-chosen reason code carried for diagnostics (`0` from [`CameraCut::request`]); the
    /// policy reads only THAT a request arrived. It exists because the kernel's event lanes refuse
    /// a zero-sized event type at compile time.
    #[parameter]
    pub reason: u32,
}

impl CameraCut {
    /// The request, reason `0`.
    pub const fn request() -> Self {
        Self::with_reason(0)
    }

    /// The request with a caller-chosen diagnostic `reason` (a shot id, a teleport kind).
    pub const fn with_reason(reason: u32) -> Self {
        Self { participants: CameraCutParticipants {}, parameters: CameraCutParameters { reason } }
    }
}

/// The active camera exactly as [`boyko_scene::resolve_active_camera`] picks it — the
/// [`ActiveCamera`] override when it names a live camera row, else the highest-`order`
/// `is_active` camera (the first one met on a tie, the same iteration) — as a full [`Entity`].
fn resolved_active_camera(
    cameras: &Query<(&Camera, &Projection, &GlobalTransform)>,
    active: &ActiveCamera,
    entities: &Entities,
) -> Option<Entity> {
    if let Some(target) = active.0
        && cameras.iter_entities().any(|(id, _)| id == target.id())
    {
        return entities.get(target.id());
    }
    let mut best: Option<(i32, EntityId)> = None;
    for (id, (cam, _, _)) in cameras.iter_entities() {
        if cam.is_active && best.is_none_or(|(order, _)| cam.order > order) {
            best = Some((cam.order, id));
        }
    }
    best.and_then(|(_, id)| entities.get(id))
}

/// The single writer of [`ResolvedPost`] (cold, once a frame, in [`PostPolicySet`], after
/// `CameraSet::Resolve`): [`ResolvedPost::next`] over this frame's [`LightingConfig`], the resolved
/// active camera and the [`CameraCut`] requests read this frame. Its FIRST run is a cut — there is
/// no history before frame 0, so frame 0's ratio is `1.0` whatever the configured exposure.
//
// `clippy::needless_pass_by_value`: `Res` / `ResMut` / `Query` / `Entities` are by-value
// `SystemParam`s read through reborrows — the false positive every policy system carries.
#[allow(clippy::needless_pass_by_value)]
pub fn resolve_post_policy(
    cfg: Res<LightingConfig>,
    active: Res<ActiveCamera>,
    cameras: Query<(&Camera, &Projection, &GlobalTransform)>,
    entities: Entities,
    mut cuts: EventReader<CameraCut>,
    mut has_run: Local<bool>,
    mut post: ResMut<ResolvedPost>,
) {
    // `count()` drains the reader: the cursor advances past every request, so one request is one
    // cut (a `next().is_some()` would leave the rest for the next frame).
    let requested = cuts.read().count() > 0;
    let first_run = !*has_run;
    *has_run = true;
    let camera = resolved_active_camera(&cameras, &active, &entities);
    *post = post.next(&cfg, camera, requested || first_run);
}

/// Owns [`ResolvedPost`] and its writer (render wave PX1a, AD-6).
#[derive(Default)]
pub struct PostPlugin;

impl Plugin for PostPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(ResolvedPost::default());
        // `resolve_post_policy` reads the active-camera override, and this kernel has no
        // `Option<Res<R>>`: a world composing `PostPlugin` without `CameraPlugin` needs the
        // resource. Inserted only if ABSENT — an override the host or `CameraPlugin` already put
        // there is the real one (the `ResolvedRenderPath` insert-if-absent precedent, `CsmPlugin`).
        if !app.world().contains_resource::<ActiveCamera>() {
            app.insert_resource(ActiveCamera::default());
        }
        app.world_mut()
            .preregister_event_default::<CameraCut>()
            .expect("invariant: CameraCut is registered once, by the one PostPlugin of the world");
        // The set-to-set edge holds whatever the plugin add-order (the `PunctualResolveSet`
        // precedent): the policy reads the camera `CameraSet::Resolve` settled this frame. Every
        // reader orders itself `.after_set(PostPolicySet)` in its own plugin.
        app.add_systems_cfg(|b| {
            b.add_system(resolve_post_policy).in_set(PostPolicySet).after_set(CameraSet::Resolve);
        });
    }

    fn name(&self) -> &'static str {
        "boyko_render::PostPlugin"
    }
}
