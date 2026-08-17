//! The runtime axis: five presets, and the compile axis is **not** one of their columns *(L17/J1)*.
//!
//! A preset selects sinks, rotation, sampling and sink mode. It does **not** select a level
//! ceiling, because the ceiling is the compile axis's (`BOYKO_PROFILE`), and folding two axes into
//! one name is how "which build is this?" stops having an answer.
//!
//! | preset | sinks | rotation | `SinkMode` | intended for |
//! |---|---|---|---|---|
//! | `Dev` | console + file | off | `Thread` | engine work, benches, goldens |
//! | `Editor` | console + file | on | `Thread` | long editor sessions |
//! | `Shipping` | binary + crash | on | `Thread` | a released title |
//! | `ShippingMin` | crash only | on | `Scheduled` | a title that wants no resident diagnostics thread |
//! | `Off` | none | — | `Manual` | the "diagnostics cost nothing" leg |
//!
//! # The default is a default, not a coupling
//!
//! A `shipping` BUILD may select `LogRuntimePreset::Dev` at RUN time. That is the entire reason
//! [`header`] prints **three independent facts** rather than one profile name: a reader who is told
//! "this is a shipping log" cannot tell which of the two axes said so, and will reason about the
//! wrong one.
//!
//! # A preset says what is configured when diagnostics are ON
//!
//! It does not say whether they are on. `Off` configures nothing; a flag-off run of any other
//! preset configures nothing either, because `enable()` never ran. The two reach the same resident
//! cost by different routes, and only one of them can be turned back on without relaunching.

use crate::lifecycle::{LogConfig, SinkMode};

/// The runtime half of the two-axis configuration.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LogRuntimePreset {
    /// Console + file, no rotation, resident sink thread.
    Dev,
    /// Console + file, rotating, resident sink thread.
    Editor,
    /// Binary + crash, rotating, resident sink thread.
    Shipping,
    /// Crash only, drained on the frame thread -- **no resident diagnostics thread**.
    ShippingMin,
    /// Nothing configured.
    Off,
}

impl LogRuntimePreset {
    /// The preset's name, as it appears in the header.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            LogRuntimePreset::Dev => "dev",
            LogRuntimePreset::Editor => "editor",
            LogRuntimePreset::Shipping => "shipping",
            LogRuntimePreset::ShippingMin => "shipping-min",
            LogRuntimePreset::Off => "off",
        }
    }

    /// The `LogConfig` this preset stands for.
    ///
    /// `ShippingMin` is `Scheduled` and **not** `Manual`. `Manual` means no consumer at all, which
    /// would make the crash file structurally contain the session's BEGINNING and nothing else --
    /// the exact inversion of what a crash file is for. What the preset actually buys is no
    /// resident diagnostics *thread*; what it costs is a bounded per-frame drain.
    #[must_use]
    pub const fn config(self) -> LogConfig {
        match self {
            LogRuntimePreset::Dev => LogConfig {
                console: true,
                sink_thread: true,
                ecs_ring: true,
                file: true,
                file_cap_bytes: 0,
                sink_mode: SinkMode::Thread,
            },
            LogRuntimePreset::Editor => LogConfig {
                console: true,
                sink_thread: true,
                ecs_ring: true,
                file: true,
                file_cap_bytes: 0,
                sink_mode: SinkMode::Thread,
            },
            LogRuntimePreset::Shipping => LogConfig {
                console: false,
                sink_thread: true,
                ecs_ring: false,
                file: true,
                file_cap_bytes: 0,
                sink_mode: SinkMode::Thread,
            },
            LogRuntimePreset::ShippingMin => LogConfig {
                console: false,
                sink_thread: false,
                ecs_ring: false,
                file: true,
                file_cap_bytes: 0,
                sink_mode: SinkMode::Scheduled,
            },
            LogRuntimePreset::Off => LogConfig {
                console: false,
                sink_thread: false,
                ecs_ring: false,
                file: false,
                file_cap_bytes: 0,
                sink_mode: SinkMode::Manual,
            },
        }
    }

    /// Whether this preset rotates the file sink.
    ///
    /// Separate from [`config`](Self::config) because rotation is set on the sink rather than
    /// carried in `LogConfig`, and a preset that claimed to rotate without calling
    /// [`crate::sink::file::set_rotation`] would be a table that describes a behaviour nobody
    /// implements -- this campaign's signature defect.
    #[must_use]
    pub const fn rotates(self) -> bool {
        matches!(
            self,
            LogRuntimePreset::Editor | LogRuntimePreset::Shipping | LogRuntimePreset::ShippingMin
        )
    }
}

/// Emit the session header: **three independent facts**, plus the session id.
///
/// ```text
/// build_profile=dev runtime_preset=shipping-min ceiling=trace session=…
/// ```
///
/// Three facts and not one name, because the two axes are independent: a `shipping` build running
/// `LogRuntimePreset::Dev` is legal and ordinary, and a header that printed a single "profile"
/// would send its reader to reason about whichever axis they assumed it meant.
///
/// The `SessionId` is the profiler's, minted once per process, so an uploaded log and an uploaded
/// profiling artifact identify the same session without anyone having to correlate timestamps.
pub fn header(preset: LogRuntimePreset) {
    let session = boyko_diag::clock::session_id();
    crate::info!(
        crate::Log,
        "build_profile={} runtime_preset={} ceiling={} session={:x}{:x}",
        boyko_diag::profile::PROFILE_NAME,
        preset.name(),
        crate::GLOBAL_CEILING.as_str(),
        session.1,
        session.0
    );
}
