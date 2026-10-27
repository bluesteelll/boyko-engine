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
pub use boyko_rhi_vulkan::gpu_crash::{CrumbLedger, GpuCrashDiagLevel};

/// The owner knob: which GPU crash-diagnostics level the next boot arms. Insert it before
/// `App::run`; an explicit `insert_resource` after `EnginePlugins` wins over the
/// `BOYKO_GPU_CRASH_DIAG` seam. Absent ⇒ [`GpuCrashDiagLevel::Off`].
#[derive(Resource, Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct GpuCrashDiag {
    /// The requested level (clamped to the build's ceiling at boot).
    pub level: GpuCrashDiagLevel,
}

/// What the GPU crash diagnostics armed and what the per-frame breadcrumb verification counted
/// (render rung GB, gate G2). Inserted by the windowed runner at boot whenever any piece armed,
/// and updated with the renderer's final ledger when the frame loop returns — so it is the
/// arming witness a test or a pixel-proof driver reads: absent ⇒ nothing armed.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GpuCrumbLedger {
    /// The level the boot armed (after the build's ceiling clamp).
    pub level: GpuCrashDiagLevel,
    /// The pieces the device probe left armed (`boyko_rhi_vulkan::gpu_crash::piece` bits,
    /// `FAULT` included).
    pub pieces: u8,
    /// The per-frame pieces the renderer records (`pieces` minus `FAULT`, minus `MARKERS` when
    /// the breadcrumb buffer could not be created).
    pub frame_pieces: u8,
    /// The verification counters.
    pub counts: CrumbLedger,
}

/// The arming-witness file's variable: when set, the runner writes the final [`GpuCrumbLedger`]
/// there as `key = value` lines when the frame loop returns — also on an unarmed run, where it
/// reads `pieces = 0`, which is what lets a byte-identity driver tell an armed leg from one that
/// silently was not.
pub const GPU_CRASH_WITNESS_ENV: &str = "BOYKO_GPU_CRASH_WITNESS";

/// Renders `l` as the witness file's text.
pub fn witness_text(l: &GpuCrumbLedger) -> String {
    use core::fmt::Write as _;
    let c = l.counts;
    let rows: [(&str, u64); 11] = [
        ("pieces", u64::from(l.pieces)),
        ("frame_pieces", u64::from(l.frame_pieces)),
        ("frames_checked", c.frames_checked),
        ("frames_incomplete", c.frames_incomplete),
        ("parity_mismatch", c.parity_mismatch),
        ("scopes_checked", c.scopes_checked),
        ("labels_begun", c.labels_begun),
        ("labels_ended", c.labels_ended),
        ("overflow", c.overflow),
        ("serial_regressions", c.serial_regressions),
        ("last_serial", u64::from(c.last_serial)),
    ];
    let mut out = String::with_capacity(320);
    // Cannot fail: writing to a `String` never errors.
    let _ = writeln!(out, "level = {}", l.level.name());
    for (key, value) in rows {
        let _ = writeln!(out, "{key} = {value}");
    }
    out
}

/// The runner's end-of-loop GB step: refreshes the ledger resource from the renderer's counters
/// and writes the witness file when [`GPU_CRASH_WITNESS_ENV`] asks. Cold: once per run.
#[cfg(windows)]
#[cold]
#[inline(never)]
pub(crate) fn finish_run(app: &mut boyko_ecs::prelude::App, counts: CrumbLedger) {
    let ledger = match app.world_mut().try_resource_mut::<GpuCrumbLedger>() {
        Some(l) => {
            l.counts = counts;
            *l
        }
        None => GpuCrumbLedger::default(),
    };
    if let Some(path) = std::env::var_os(GPU_CRASH_WITNESS_ENV)
        && let Err(e) = std::fs::write(&path, witness_text(&ledger))
    {
        let shown = path.to_string_lossy();
        let err = crate::diag::debug_into(&e.kind());
        crate::diag::report_dump_write_failed("GPU crash witness", &shown, err.as_str());
    }
}

/// Where the device-loss report writes the `VK_EXT_device_fault` vendor binary; the current
/// directory when unset.
pub const GPU_CRASH_DIR_ENV: &str = "BOYKO_GPU_CRASH_DIR";

/// The explicit opt-in the forced-hang gate (G3) needs beside the `inject_gpu_hang` force key:
/// a forced device loss resets the display driver, so a key alone — which a test sweep could
/// carry — never arms it.
pub const GPU_HANG_OPT_IN_ENV: &str = "BOYKO_GPU_HANG_OPT_IN";
/// [`GPU_HANG_OPT_IN_ENV`]'s one accepted value.
pub const GPU_HANG_OPT_IN_VALUE: &str = "i-accept-a-display-reset";

/// `true` iff [`GPU_HANG_OPT_IN_ENV`] holds exactly [`GPU_HANG_OPT_IN_VALUE`].
pub fn gpu_hang_opted_in() -> bool {
    std::env::var(GPU_HANG_OPT_IN_ENV).is_ok_and(|v| v == GPU_HANG_OPT_IN_VALUE)
}

/// The vendor-binary outcome text's bound.
const OUTCOME_BYTES: usize = 192;

/// Writes the device-loss report's vendor binary to `$BOYKO_GPU_CRASH_DIR` (or the current
/// directory) as `boyko-gpu-fault-<unix_s>-<pid>.bin`, and returns what the report prints after
/// `->`: the path, or why it was not written (`boyko-E3010` for a write failure).
#[cfg(any(windows, test))]
#[cold]
#[inline(never)]
pub(crate) fn write_vendor_binary(bytes: &[u8]) -> boyko_log::DspBuf<OUTCOME_BYTES> {
    let dir = std::env::var_os(GPU_CRASH_DIR_ENV).map_or_else(|| std::path::PathBuf::from("."), std::path::PathBuf::from);
    write_vendor_binary_in(&dir, bytes)
}

/// [`write_vendor_binary`] into `dir`.
#[cfg(any(windows, test))]
pub(crate) fn write_vendor_binary_in(dir: &std::path::Path, bytes: &[u8]) -> boyko_log::DspBuf<OUTCOME_BYTES> {
    use core::fmt::Write as _;
    let mut out = boyko_log::DspBuf::<OUTCOME_BYTES>::new();
    if bytes.is_empty() {
        let _ = out.write_str("not written: no binary");
        return out;
    }
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let mut name = boyko_log::DspBuf::<64>::new();
    let _ = write!(&mut name, "boyko-gpu-fault-{secs}-{}.bin", std::process::id());
    let path = dir.join(name.as_str());
    match std::fs::write(&path, bytes) {
        Ok(()) => {
            let _ = write!(&mut out, "{}", path.display());
        }
        Err(e) => {
            let shown = path.to_string_lossy();
            let err = crate::diag::debug_into(&e.kind());
            crate::diag::report_dump_write_failed("gpu fault vendor binary", &shown, err.as_str());
            let _ = write!(&mut out, "not written: {} (boyko-E3010)", err.as_str());
        }
    }
    out
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
    fn the_witness_names_every_field() {
        let l = super::GpuCrumbLedger {
            level: GpuCrashDiagLevel::Markers,
            pieces: 7,
            frame_pieces: 6,
            counts: super::CrumbLedger { frames_checked: 10, last_serial: 12, ..Default::default() },
        };
        let t = super::witness_text(&l);
        let lines: Vec<&str> = t.lines().collect();
        assert_eq!(lines[..4], ["level = markers", "pieces = 7", "frame_pieces = 6", "frames_checked = 10"], "{t}");
        assert_eq!(lines.last(), Some(&"last_serial = 12"), "{t}");
        assert_eq!(lines.len(), 12);
    }

    /// The vendor binary lands in the named directory; an unwritable one is `boyko-E3010` with
    /// the kind named, and the report line says it was not written.
    #[test]
    fn the_vendor_binary_writer_reports_a_failed_write() {
        use boyko_log::probe::{last_message, observe_lock, watch, watched};
        let dir = std::env::temp_dir().join(format!("boyko_gb_vendor_{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("test setup: temp dir");
        let ok = super::write_vendor_binary_in(&dir, b"vendor");
        let written = std::path::PathBuf::from(ok.as_str());
        assert_eq!(std::fs::read(&written).expect("the binary was written"), b"vendor");
        assert!(written.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("boyko-gpu-fault-")));
        let _ = std::fs::remove_dir_all(&dir);

        let _lock = observe_lock();
        boyko_log::probe::arm::<boyko_log::Host>();
        watch(b'E', boyko_log::codes::E3010.number());
        // A directory under a FILE cannot exist, on every platform.
        let file = std::env::temp_dir().join(format!("boyko_gb_vendor_file_{}", std::process::id()));
        std::fs::write(&file, b"x").expect("test setup: a plain file");
        let bad = super::write_vendor_binary_in(&file.join("sub"), b"vendor");
        let _ = std::fs::remove_file(&file);
        assert!(bad.as_str().starts_with("not written: "), "{}", bad.as_str());
        assert_eq!(watched(), 1, "a failed write is one boyko-E3010");
        assert!(last_message().contains("gpu fault vendor binary"), "{}", last_message());
        assert_eq!(super::write_vendor_binary_in(&dir, b"").as_str(), "not written: no binary");
    }

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
