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
use boyko_macros::{Resource, SystemSet, event};

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
    pub fn from_config(cfg: &LightingConfig) -> Self {
        let _ = cfg;
        unimplemented!("PX1a C8: ResolvedPost::from_config")
    }

    /// The policy step: the frame after `self` under `cfg`, with this frame's resolved
    /// `active_camera` and whether a [`CameraCut`] was read. Pure — [`resolve_post_policy`] is this
    /// plus the reads.
    pub fn next(&self, cfg: &LightingConfig, active_camera: Option<Entity>, cut_requested: bool) -> Self {
        let _ = (cfg, active_camera, cut_requested);
        unimplemented!("PX1a C8: ResolvedPost::next")
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
pub struct CameraCut {}

impl CameraCut {
    /// The request (the event carries no data).
    pub const fn request() -> Self {
        Self { participants: CameraCutParticipants {}, parameters: CameraCutParameters {} }
    }
}

/// The single writer of [`ResolvedPost`] (cold, once a frame, in [`PostPolicySet`]).
pub fn resolve_post_policy() {
    unimplemented!("PX1a C8: resolve_post_policy")
}

/// Owns [`ResolvedPost`] and its writer (render wave PX1a, AD-6).
#[derive(Default)]
pub struct PostPlugin;

impl Plugin for PostPlugin {
    fn build(&self, _app: &mut App) {}

    fn name(&self) -> &'static str {
        "boyko_render::PostPlugin"
    }
}
