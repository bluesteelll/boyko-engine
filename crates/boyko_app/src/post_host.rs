//! Render wave PX1a (H4 Scope 7, rev 4 arch OQ1; `px1a/cut.md` D-E5): the runner's read of the
//! resolved post settings, as pure functions.
//!
//! The runner reads `ResolvedPost` through `try_resource` — its M1 convention for a resource a host
//! may omit — and never panics on a host that composes its own plugin set. On `None` it uses
//! [`ResolvedPost::from_config`], the SAME pure function `resolve_post_policy` evaluates for a world
//! with no active camera and no cut, and reports `post_plugin_absent` once. That fallback is not a
//! second definition: a world with `LightingPlugin` but no `PostPlugin` panics in `collect_lights`
//! before the runner reads anything, so only a host composing neither reaches it.

use boyko_log::codes::{self, OnceSite};
use boyko_render::light::{LightingConfig, TONEMAP_MODE_SHIFT};
use boyko_render::post::ResolvedPost;
use boyko_render::post::gpu::{POST_MODE_SHARPEN, PostFinalGpu};
use boyko_render::taa_config::{SharpenMode, TaaConfig};

/// `boyko-W3012`'s latch: the fallback is reported once per process, not once per frame.
static W3012_SITE: OnceSite = OnceSite::new();

/// The frame's resolved post settings and whether the [`ResolvedPost::from_config`] fallback was
/// taken (the caller reports `post_plugin_absent` once when it was).
#[inline]
pub fn resolved_post_for_frame(
    resolved: Option<&ResolvedPost>,
    config: Option<&LightingConfig>,
) -> (ResolvedPost, bool) {
    match resolved {
        Some(post) => (*post, false),
        None => (ResolvedPost::from_config(&config.copied().unwrap_or_default()), true),
    }
}

/// The `post_final` constant buffer for `post` at the dispatch `extent` (`display`'s size): the
/// tonemapper in the mode word at `TONEMAP_MODE_SHIFT`, an exposure ratio of exactly `1.0` (the
/// producers already baked `pre_exposure` into `lit`; PX3's auto-exposure moves it), the extent,
/// and the sharpen stage off (no `POST_MODE_SHARPEN` bit, `sharpness` 0 — render wave PX1b).
#[inline]
pub const fn post_final_gpu(post: &ResolvedPost, extent: [u32; 2]) -> PostFinalGpu {
    PostFinalGpu {
        mode_words: post.tonemapper << TONEMAP_MODE_SHIFT,
        exposure_ratio: 1.0,
        extent,
        sharpness: 0.0,
        _r: [0; 3],
    }
}

/// Render wave PX1b (`px1b/cut.md` D-R8): RCAS's `con` when this frame runs `post_final`'s sharpen
/// stage — `taa_armed` (the runner's single TAA predicate, `taa_armed_now`: `ResolvedAa == Taa ∧
/// taa_supported()`) and `cfg`'s `sharpen == SharpenMode::Rcas` — else `None`. A host that omits
/// `AaPlugin` (no `TaaConfig`) never sharpens. Pure and `Copy`: no allocation per frame.
#[inline]
pub fn post_sharpen(taa_armed: bool, cfg: Option<&TaaConfig>) -> Option<f32> {
    match cfg {
        Some(c) if taa_armed && c.sharpen == SharpenMode::Rcas => Some(c.rcas_con()),
        _ => None,
    }
}

/// Render wave PX1b (D-R8): `gpu` with the sharpen stage armed at `con` — the mode word's
/// `POST_MODE_SHARPEN` bit and the `sharpness` word — or unchanged for `None`. A live
/// `SharpenMode` toggle is therefore one UBO word: no target rebuild, no TAA history reset.
#[inline]
pub const fn with_rcas(gpu: PostFinalGpu, con: Option<f32>) -> PostFinalGpu {
    match con {
        Some(sharpness) => PostFinalGpu { mode_words: gpu.mode_words | POST_MODE_SHARPEN, sharpness, ..gpu },
        None => gpu,
    }
}

/// Reports `boyko-W3012` (`post_plugin_absent`) once per process: the runner found no
/// `ResolvedPost` and derived the frame's post settings from `LightingConfig`.
#[cold]
#[inline(never)]
pub(crate) fn report_post_plugin_absent() {
    report_post_plugin_absent_at(&W3012_SITE);
}

/// [`report_post_plugin_absent`] on an explicit latch (the unit test's own).
fn report_post_plugin_absent_at(site: &OnceSite) {
    if !site.claim() {
        return;
    }
    boyko_log::warn!(
        boyko_log::App,
        codes::W3012,
        "no PostPlugin composed: the runner derives the frame's post settings (pre-exposure, \
         tonemapper) from LightingConfig through ResolvedPost::from_config, so camera cuts never \
         reset the TAA history"
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use boyko_log::probe::{last_message, observe_lock, watch, watched};

    #[test]
    fn w3012_reports_the_absent_post_plugin_once() {
        let _lock = observe_lock();
        // Raise the target's ceiling so a `Warn` is admitted at all (else "no emission" would
        // read the same as a broken reporter — `diag.rs`'s `arm_app`).
        boyko_log::probe::arm::<boyko_log::App>();
        let site = OnceSite::new();
        watch(b'W', codes::W3012.number());
        report_post_plugin_absent_at(&site);
        assert_eq!(watched(), 1, "the first fallback frame must report");
        assert!(last_message().contains("PostPlugin"), "{}", last_message());
        report_post_plugin_absent_at(&site);
        assert_eq!(watched(), 1, "the fallback is reported once, not once per frame");
    }
}
