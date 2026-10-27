//! Render rung GB — GPU crash diagnostics: the arming level, its compile-time ceiling, and the
//! optional extension command tables a booted [`VulkanContext`](crate::device::VulkanContext)
//! carries.
//!
//! # Three pieces under one cumulative level
//!
//! | level | adds | what it costs a frame |
//! |---|---|---|
//! | [`GpuCrashDiagLevel::Off`] | nothing — no extension, no command | nothing |
//! | [`GpuCrashDiagLevel::Fault`] | `VK_EXT_device_fault` (read only after a device loss) | nothing |
//! | [`GpuCrashDiagLevel::Labels`] | `VK_EXT_debug_utils` pass labels + resource names | one label pair per scope |
//! | [`GpuCrashDiagLevel::Markers`] | `VK_AMD_buffer_marker` begin/end breadcrumbs | two marker writes per scope |
//! | [`GpuCrashDiagLevel::Checkpoints`] | `VK_NV_device_diagnostic_checkpoints` | two checkpoints per scope |
//!
//! Each piece is armed only when it is requested **and** the device advertises it: a missing
//! extension drops that piece and reports `boyko-W2107`, never a boot failure. The level is read
//! once, at boot (extensions are boot-frozen), from the `boyko_app::GpuCrashDiag` resource or the
//! `BOYKO_GPU_CRASH_DIAG` dev seam.
//!
//! # The ceiling deletes the sites
//!
//! [`GpuCrashDiagLevel::CEILING`] is the build profile's `GPU_CRASH_DIAG_CEILING`
//! (`boyko_diag::profile`). Every GB site in this crate is gated
//! `GpuCrashDiagLevel::COMPILED_IN && <armed byte>`; in a `shipping` build the first conjunct is a
//! `const false`, so the site, its `#[cold]` body and the entry-point names it would load are not
//! in the image. A runtime level above the ceiling is clamped, never refused.
//!
//! # Where the pieces live
//!
//! The per-scope recording (the chained scope, the marker region, the per-frame verification)
//! is `present::gpu_crumbs`. This module holds only the boot-time decision and the loaded entry
//! points, so a context booted for a headless test carries the same answer as the windowed one.

use crate::ffi::{
    PfnVkCmdBeginDebugUtilsLabelExt, PfnVkCmdEndDebugUtilsLabelExt, PfnVkCmdSetCheckpointNv,
    PfnVkCmdWriteBufferMarkerAmd, PfnVkGetQueueCheckpointDataNv, PfnVkSetDebugUtilsObjectNameExt,
};

/// The GPU crash-diagnostics arming level. Cumulative: each level includes every piece below it.
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Default, Hash)]
pub enum GpuCrashDiagLevel {
    /// Nothing is enabled and nothing is recorded — byte-identical to a build without GB.
    #[default]
    Off = 0,
    /// `VK_EXT_device_fault` is enabled, so a device loss can name the faulting address.
    Fault = 1,
    /// Adds debug-utils labels: every graph pass and host scope runs inside a label named after
    /// the pass, and graph resources carry their declared names (RenderDoc / Nsight captures).
    Labels = 2,
    /// Adds breadcrumbs: each scope writes a begin and an end marker (`vkCmdWriteBufferMarkerAMD`),
    /// so a device loss names the last completed scope and the scopes in flight.
    Markers = 3,
    /// Adds NVIDIA per-stage checkpoints at the same scope points.
    Checkpoints = 4,
}

/// The piece bits a booted context carries after the device probe (`requested ∧ supported`).
pub mod piece {
    /// `VK_EXT_device_fault` is enabled.
    pub const FAULT: u8 = 1 << 0;
    /// `VK_EXT_debug_utils` labels and object names are live.
    pub const LABELS: u8 = 1 << 1;
    /// `VK_AMD_buffer_marker` breadcrumbs are live.
    pub const MARKERS: u8 = 1 << 2;
    /// `VK_NV_device_diagnostic_checkpoints` checkpoints are live.
    pub const CHECKPOINTS: u8 = 1 << 3;
    /// Every piece a frame records (everything but the fault query).
    pub const PER_FRAME: u8 = LABELS | MARKERS | CHECKPOINTS;
    /// The pieces in bit order with their report spelling.
    pub const NAMES: [(u8, &str); 4] =
        [(FAULT, "fault"), (LABELS, "labels"), (MARKERS, "markers"), (CHECKPOINTS, "checkpoints")];
}

impl GpuCrashDiagLevel {
    /// Every level, lowest first; a level's index is its discriminant.
    pub const ALL: [Self; 5] = [Self::Off, Self::Fault, Self::Labels, Self::Markers, Self::Checkpoints];

    /// The highest level this build admits — the profile axis's `GPU_CRASH_DIAG_CEILING`.
    /// `from_raw` panics outside `0..=4`, and a panic in a `const` initialiser is a compile
    /// error, so a build script that emitted a nonsense ceiling fails the build.
    pub const CEILING: Self = Self::from_raw(boyko_diag::profile::GPU_CRASH_DIAG_CEILING);

    /// `true` iff this build compiles any GB site in. The FIRST conjunct of every site's gate:
    /// `false` in a `shipping` build, where it deletes the site and its cold body.
    pub const COMPILED_IN: bool = Self::CEILING as u8 != 0;

    /// The level whose discriminant is `raw`.
    ///
    /// # Panics
    ///
    /// When `raw > 4` (a compile error when evaluated in a `const`).
    #[inline]
    pub const fn from_raw(raw: u8) -> Self {
        match raw {
            0 => Self::Off,
            1 => Self::Fault,
            2 => Self::Labels,
            3 => Self::Markers,
            4 => Self::Checkpoints,
            _ => panic!("invariant: a GPU crash-diagnostics level is in 0..=4"),
        }
    }

    /// `self`, lowered to `ceiling` when it is above it. Pure; the ceiling is a parameter so the
    /// clamp is testable in a `dev` build, where [`Self::CEILING`] admits everything.
    #[inline]
    pub const fn clamped_to(self, ceiling: Self) -> Self {
        if self as u8 > ceiling as u8 { ceiling } else { self }
    }

    /// `self` under this build's [`Self::CEILING`] — the level a boot actually arms.
    #[inline]
    pub const fn effective(self) -> Self {
        self.clamped_to(Self::CEILING)
    }

    /// The piece bits this level requests (before the device probe drops unsupported ones).
    #[inline]
    pub const fn requested_pieces(self) -> u8 {
        match self {
            Self::Off => 0,
            Self::Fault => piece::FAULT,
            Self::Labels => piece::FAULT | piece::LABELS,
            Self::Markers => piece::FAULT | piece::LABELS | piece::MARKERS,
            Self::Checkpoints => piece::FAULT | piece::LABELS | piece::MARKERS | piece::CHECKPOINTS,
        }
    }

    /// The `BOYKO_GPU_CRASH_DIAG` spelling of this level.
    #[inline]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Fault => "fault",
            Self::Labels => "labels",
            Self::Markers => "markers",
            Self::Checkpoints => "checkpoints",
        }
    }

    /// Parses a `BOYKO_GPU_CRASH_DIAG` value (ASCII case-insensitive, surrounding whitespace
    /// ignored). `None` for anything that is not one of the five names. Allocates nothing.
    pub fn parse(value: &str) -> Option<Self> {
        let v = value.trim();
        Self::ALL.into_iter().find(|l| l.name().eq_ignore_ascii_case(v))
    }
}

/// `VK_EXT_debug_utils` command-buffer label and object-name entry points, loaded through
/// `vkGetInstanceProcAddr` (the loader trampolines) when [`piece::LABELS`] is armed.
#[derive(Clone, Copy)]
pub struct DebugUtilsCmdFns {
    /// `vkCmdBeginDebugUtilsLabelEXT`.
    pub begin_label: PfnVkCmdBeginDebugUtilsLabelExt,
    /// `vkCmdEndDebugUtilsLabelEXT`.
    pub end_label: PfnVkCmdEndDebugUtilsLabelExt,
    /// `vkSetDebugUtilsObjectNameEXT`.
    pub set_object_name: PfnVkSetDebugUtilsObjectNameExt,
}

/// The `VK_AMD_buffer_marker` entry point, loaded through `vkGetDeviceProcAddr` when
/// [`piece::MARKERS`] is armed.
#[derive(Clone, Copy)]
pub struct MarkerFns {
    /// `vkCmdWriteBufferMarkerAMD` (sync1).
    pub write_buffer_marker: PfnVkCmdWriteBufferMarkerAmd,
}

/// The `VK_NV_device_diagnostic_checkpoints` entry points, loaded through `vkGetDeviceProcAddr`
/// when [`piece::CHECKPOINTS`] is armed.
#[derive(Clone, Copy)]
pub struct CheckpointFns {
    /// `vkCmdSetCheckpointNV`.
    pub set_checkpoint: PfnVkCmdSetCheckpointNv,
    /// `vkGetQueueCheckpointDataNV` (sync1; legal only on a lost device).
    pub get_queue_checkpoint_data: PfnVkGetQueueCheckpointDataNv,
}

/// What a boot armed: the clamped level, the piece bits that survived the probe, and the loaded
/// entry points. `Copy` POD, read once by each consumer at its own construction.
#[derive(Clone, Copy, Default)]
pub struct GpuCrashState {
    /// The level the boot asked for, after the build's ceiling clamp.
    pub level: GpuCrashDiagLevel,
    /// The armed pieces ([`piece`] bits): requested by `level` and supported by the device.
    pub pieces: u8,
    /// `Some` iff [`piece::LABELS`] is armed.
    pub debug_utils: Option<DebugUtilsCmdFns>,
    /// `Some` iff [`piece::MARKERS`] is armed.
    pub markers: Option<MarkerFns>,
    /// `Some` iff [`piece::CHECKPOINTS`] is armed.
    pub checkpoints: Option<CheckpointFns>,
}

impl GpuCrashState {
    /// The state of a context booted with GB off: no piece, no entry point.
    pub const OFF: Self = Self {
        level: GpuCrashDiagLevel::Off,
        pieces: 0,
        debug_utils: None,
        markers: None,
        checkpoints: None,
    };

    /// `true` iff piece `bit` is armed.
    #[inline]
    pub const fn has(&self, bit: u8) -> bool {
        self.pieces & bit != 0
    }
}

/// What the renderer's per-frame breadcrumb verification counted since boot (render rung GB, gate
/// G2). Every field is `0` in an unarmed run. A frame is checked after its slot's fence wait.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CrumbLedger {
    /// Frames verified.
    pub frames_checked: u64,
    /// Verified frames whose breadcrumbs were not all complete (markers piece only): a frame
    /// begin or end, or a scope's begin or end, did not carry the frame's serial.
    pub frames_incomplete: u64,
    /// Verified frames whose graph-pass scopes did not equal the passes the graph declared.
    pub parity_mismatch: u64,
    /// Scopes (record positions) verified, overflow included.
    pub scopes_checked: u64,
    /// Label regions opened / closed by the verified frames (labels piece).
    pub labels_begun: u64,
    pub labels_ended: u64,
    /// Record positions past the breadcrumb cap (labelled, not marked).
    pub overflow: u64,
    /// A slot verified a serial that was not newer than the one it verified before.
    pub serial_regressions: u64,
    /// The newest serial verified (`0`: none).
    pub last_serial: u32,
}

/// Writes the armed pieces as a comma-separated list (`fault,labels,…`), or `none`.
pub fn write_pieces<W: core::fmt::Write>(out: &mut W, pieces: u8) -> core::fmt::Result {
    let mut first = true;
    for (bit, name) in piece::NAMES {
        if pieces & bit != 0 {
            if !first {
                out.write_str(",")?;
            }
            out.write_str(name)?;
            first = false;
        }
    }
    if first {
        out.write_str("none")?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{GpuCrashDiagLevel, piece, write_pieces};

    /// The shipping clamp: under a zero ceiling every level is `Off`, so nothing is enabled.
    ///
    /// RED: make `clamped_to` return `self` unconditionally and `Labels` survives ceiling 0.
    #[test]
    fn ceiling_zero_clamps_every_level_to_off() {
        for level in GpuCrashDiagLevel::ALL {
            assert_eq!(
                level.clamped_to(GpuCrashDiagLevel::Off),
                GpuCrashDiagLevel::Off,
                "{level:?} survived a zero ceiling"
            );
            assert_eq!(level.clamped_to(GpuCrashDiagLevel::Off).requested_pieces(), 0);
        }
        assert_eq!(
            GpuCrashDiagLevel::Checkpoints.clamped_to(GpuCrashDiagLevel::Labels),
            GpuCrashDiagLevel::Labels,
            "a level above the ceiling is lowered to it, not refused"
        );
        assert_eq!(GpuCrashDiagLevel::Fault.clamped_to(GpuCrashDiagLevel::Labels), GpuCrashDiagLevel::Fault);
    }

    /// The ceiling constant agrees with the profile axis it is read from.
    #[test]
    fn the_ceiling_is_the_profile_axis_value() {
        assert_eq!(GpuCrashDiagLevel::CEILING as u8, boyko_diag::profile::GPU_CRASH_DIAG_CEILING);
        assert_eq!(GpuCrashDiagLevel::COMPILED_IN, GpuCrashDiagLevel::CEILING != GpuCrashDiagLevel::Off);
    }

    #[test]
    fn every_level_parses_from_its_name_and_back() {
        for (i, level) in GpuCrashDiagLevel::ALL.into_iter().enumerate() {
            assert_eq!(level as usize, i, "a level's index is its discriminant");
            assert_eq!(GpuCrashDiagLevel::from_raw(i as u8), level);
            assert_eq!(GpuCrashDiagLevel::parse(level.name()), Some(level));
            assert_eq!(GpuCrashDiagLevel::parse(&format!(" {} ", level.name().to_uppercase())), Some(level));
        }
        assert_eq!(GpuCrashDiagLevel::parse("marker"), None);
        assert_eq!(GpuCrashDiagLevel::parse(""), None);
    }

    /// The levels are cumulative: each one's pieces are a strict superset of the one below.
    #[test]
    fn levels_are_ordered_and_cumulative() {
        for pair in GpuCrashDiagLevel::ALL.windows(2) {
            let (lo, hi) = (pair[0], pair[1]);
            assert!(lo < hi);
            let (a, b) = (lo.requested_pieces(), hi.requested_pieces());
            assert_eq!(a & b, a, "{hi:?} drops a piece {lo:?} had");
            assert_ne!(a, b, "{hi:?} adds no piece over {lo:?}");
        }
        assert_eq!(GpuCrashDiagLevel::Checkpoints.requested_pieces(), piece::FAULT | piece::PER_FRAME);
    }

    #[test]
    fn pieces_print_in_bit_order() {
        let mut s = String::new();
        write_pieces(&mut s, piece::CHECKPOINTS | piece::FAULT | piece::LABELS).expect("String write");
        assert_eq!(s, "fault,labels,checkpoints");
        s.clear();
        write_pieces(&mut s, 0).expect("String write");
        assert_eq!(s, "none");
    }
}
