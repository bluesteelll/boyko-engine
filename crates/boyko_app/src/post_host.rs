//! Render wave PX1a (H4 Scope 7, rev 4 arch OQ1; `px1a/cut.md` D-E5): the runner's read of the
//! resolved post settings, as pure functions.
//!
//! The runner reads `ResolvedPost` through `try_resource` — its M1 convention for a resource a host
//! may omit — and never panics on a host that composes its own plugin set. On `None` it uses
//! [`ResolvedPost::from_config`], the SAME pure function `resolve_post_policy` evaluates for a world
//! with no active camera and no cut, and reports `post_plugin_absent` once. That fallback is not a
//! second definition: a world with `LightingPlugin` but no `PostPlugin` panics in `collect_lights`
//! before the runner reads anything, so only a host composing neither reaches it.

use boyko_render::light::LightingConfig;
use boyko_render::post::ResolvedPost;
use boyko_render::post::gpu::PostFinalGpu;

/// The frame's resolved post settings and whether the [`ResolvedPost::from_config`] fallback was
/// taken (the caller reports `post_plugin_absent` once when it was).
pub fn resolved_post_for_frame(
    resolved: Option<&ResolvedPost>,
    config: Option<&LightingConfig>,
) -> (ResolvedPost, bool) {
    let _ = (resolved, config);
    unimplemented!("PX1a C8: resolved_post_for_frame")
}

/// The `post_final` constant buffer for `post` at the dispatch `extent` (`display`'s size).
pub fn post_final_gpu(post: &ResolvedPost, extent: [u32; 2]) -> PostFinalGpu {
    let _ = (post, extent);
    unimplemented!("PX1a C8: post_final_gpu")
}
