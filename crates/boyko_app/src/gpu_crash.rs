//! Render rung GB — the host half of the GPU crash diagnostics: the owner knob
//! ([`GpuCrashDiag`]) and its `BOYKO_GPU_CRASH_DIAG` dev seam.
//!
//! The level is read ONCE, at boot, by the windowed runner (`try_resource`; absent ⇒ `Off`) and
//! handed to `VulkanContext::boot_singleton_with_diag` — device extensions are boot-frozen, so a
//! later change has no effect. The backend clamps it to the build's ceiling
//! (`GpuCrashDiagLevel::CEILING`: `0` in `shipping`), enables each piece the device supports and
//! reports `boyko-W2107` for each it does not.
//!
//! # Levels
//!
//! `off` (the default: nothing enabled, nothing recorded) · `fault` (`VK_EXT_device_fault`) ·
//! `labels` (+ debug-utils pass labels and resource names, for RenderDoc / Nsight) · `markers`
//! (+ per-scope breadcrumbs) · `checkpoints` (+ NVIDIA per-stage checkpoints). Cumulative.

use boyko_macros::Resource;
pub use boyko_rhi_vulkan::gpu_crash::GpuCrashDiagLevel;

/// The owner knob: which GPU crash-diagnostics level the next boot arms. Insert it before
/// `App::run`; an explicit `insert_resource` after `EnginePlugins` wins over the
/// `BOYKO_GPU_CRASH_DIAG` seam. Absent ⇒ [`GpuCrashDiagLevel::Off`].
#[derive(Resource, Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct GpuCrashDiag {
    /// The requested level (clamped to the build's ceiling at boot).
    pub level: GpuCrashDiagLevel,
}

/// The dev seam's variable.
pub const GPU_CRASH_DIAG_ENV: &str = "BOYKO_GPU_CRASH_DIAG";

/// `BOYKO_GPU_CRASH_DIAG`'s admitted values, for the typo report.
const VALID: &str = "off|fault|labels|markers|checkpoints";

/// `boyko-W3009`'s latch for this variable — its own, so a mistyped `BOYKO_GPU_CRASH_DIAG` does
/// not silence a mistyped `BOYKO_RENDER_PATH`.
static W3009_GPU_CRASH_DIAG: boyko_log::codes::OnceSite = boyko_log::codes::OnceSite::new();

/// Parses a `BOYKO_GPU_CRASH_DIAG` value. `Err` carries the rejected text for the report.
pub fn parse_gpu_crash_diag(value: &str) -> Result<GpuCrashDiag, &str> {
    GpuCrashDiagLevel::parse(value).map(|level| GpuCrashDiag { level }).ok_or(value)
}

/// The `BOYKO_GPU_CRASH_DIAG` dev seam, read by `EnginePlugins::build`: `None` when the variable
/// is unset, when its value names no level (`boyko-W3009`, falling back to the default `off`), or
/// when this build compiles GB out (the variable is not read at all then).
pub(crate) fn gpu_crash_diag_from_env() -> Option<GpuCrashDiag> {
    if !GpuCrashDiagLevel::COMPILED_IN {
        return None;
    }
    let value = std::env::var(GPU_CRASH_DIAG_ENV).ok()?;
    match parse_gpu_crash_diag(&value) {
        Ok(diag) => Some(diag),
        Err(bad) => {
            crate::diag::report_unrecognized_env_value(&W3009_GPU_CRASH_DIAG, GPU_CRASH_DIAG_ENV, bad, "off", VALID);
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{GpuCrashDiag, GpuCrashDiagLevel, parse_gpu_crash_diag};

    #[test]
    fn every_level_parses_and_a_typo_is_refused() {
        for level in GpuCrashDiagLevel::ALL {
            assert_eq!(parse_gpu_crash_diag(level.name()), Ok(GpuCrashDiag { level }));
        }
        assert_eq!(parse_gpu_crash_diag("Markers "), Ok(GpuCrashDiag { level: GpuCrashDiagLevel::Markers }));
        assert_eq!(parse_gpu_crash_diag("breadcrumbs"), Err("breadcrumbs"));
        assert_eq!(GpuCrashDiag::default().level, GpuCrashDiagLevel::Off, "absent is off");
    }
}
