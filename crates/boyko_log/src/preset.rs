//! The runtime axis: five presets, and the compile axis is **not** one of their columns *(L17/J1)*.
//!
//! A preset selects sinks, rotation, sampling and sink mode. It does **not** select a level
//! ceiling, because the ceiling is the compile axis's (`BOYKO_PROFILE`), and folding two axes into
//! one name is how "which build is this?" stops having an answer.
//!
//! | preset | sinks | rotation | `SinkMode` | intended for |
//! |---|---|---|---|---|
//! | `Dev` | console + text file | off | `Thread` | engine work, benches, goldens |
//! | `Editor` | console + text file | on | `Thread` | long editor sessions |
//! | `Shipping` | binary file | on | `Thread` | a released title |
//! | `ShippingMin` | text file | on | `Scheduled` | a title that wants no resident diagnostics thread |
//! | `Off` | none | — | `Manual` | the "diagnostics cost nothing" leg |
//!
//! # There is no separate "crash" sink, and the table used to say there was
//!
//! The rows for `Shipping` and `ShippingMin` read "binary + crash" and "crash only". **A crash
//! destination distinct from the file sink does not exist**: [`crash::arm`](crate::sink::crash::arm)
//! points the FILE sink at a path and installs the hook's flush protocol on top. Nor should it —
//! a destination that only received records at panic time would be EMPTY, because the drain empties
//! the ring continuously under every `SinkMode`. What makes a crash file work is that it is an
//! ordinary continuous sink which happens to survive the crash.
//!
//! So the column is corrected rather than the code. `crash::arm` is orthogonal to this table: any
//! preset with a file sink can arm it, and arming replaces that sink's path.
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
                binary: false,
                file_cap_bytes: 0,
                sink_mode: SinkMode::Thread,
            },
            LogRuntimePreset::Editor => LogConfig {
                console: true,
                sink_thread: true,
                ecs_ring: true,
                file: true,
                binary: false,
                file_cap_bytes: 0,
                sink_mode: SinkMode::Thread,
            },
            LogRuntimePreset::Shipping => LogConfig {
                console: false,
                sink_thread: true,
                ecs_ring: false,
                file: false,
                binary: true,
                file_cap_bytes: 0,
                sink_mode: SinkMode::Thread,
            },
            LogRuntimePreset::ShippingMin => LogConfig {
                console: false,
                sink_thread: false,
                ecs_ring: false,
                file: true,
                binary: false,
                file_cap_bytes: 0,
                sink_mode: SinkMode::Scheduled,
            },
            LogRuntimePreset::Off => LogConfig {
                console: false,
                sink_thread: false,
                ecs_ring: false,
                file: false,
                binary: false,
                file_cap_bytes: 0,
                sink_mode: SinkMode::Manual,
            },
        }
    }

}

/// Rotate the text sink at 64 MiB.
///
/// A number rather than a knob, because a preset that took one would be a configuration wearing a
/// preset's name. A host that wants its own calls
/// [`set_rotation`](crate::sink::file::set_rotation) directly.
pub const ROTATE_AT_BYTES: u64 = 64 * 1024 * 1024;

/// Keep four rotated files. Enough to hold a session's tail across three restarts.
pub const ROTATE_KEEP: u8 = 4;

impl LogRuntimePreset {
    /// Parse a preset from the name [`name`](Self::name) prints. `None` for anything else.
    ///
    /// The inverse of `name`, and it is the inverse deliberately: a host reads `BOYKO_LOG_PRESET`
    /// and a reader reads `runtime_preset=` in the header, and those two strings being the same
    /// set is what lets someone reproduce a run from its own log.
    #[must_use]
    pub fn from_name(name: &str) -> Option<LogRuntimePreset> {
        match name {
            "dev" => Some(LogRuntimePreset::Dev),
            "editor" => Some(LogRuntimePreset::Editor),
            "shipping" => Some(LogRuntimePreset::Shipping),
            "shipping-min" => Some(LogRuntimePreset::ShippingMin),
            "off" => Some(LogRuntimePreset::Off),
            _ => None,
        }
    }

    /// Rebuild a preset from `boot_preset`'s stored byte, where `0` means "no preset".
    ///
    /// Plus-one encoding so the `.bss`-zero state means ABSENT: a host that built its own
    /// `LogConfig` selected no preset at all, and printing `runtime_preset=dev` for it would name
    /// an axis nobody chose.
    #[must_use]
    pub const fn from_raw(raw: u8) -> Option<LogRuntimePreset> {
        match raw {
            1 => Some(LogRuntimePreset::Dev),
            2 => Some(LogRuntimePreset::Editor),
            3 => Some(LogRuntimePreset::Shipping),
            4 => Some(LogRuntimePreset::ShippingMin),
            5 => Some(LogRuntimePreset::Off),
            _ => None,
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
pub fn header(preset: Option<LogRuntimePreset>) {
    let session = boyko_diag::clock::session_id();
    // ── THE ID IS RENDERED HERE, AS HEX, AND `{:x}` COULD NOT DO IT ─────────────────────────
    //
    // `record::render_payload` consumes a value for any `{…}` group and **ignores the format
    // spec** -- a documented limitation of the one walker both sinks share. So `session={:x}{:x}`
    // printed the two halves in DECIMAL, glued: `183307752869167903659220641735087238341` is
    // `dec(hi)` followed by `dec(lo)`, a number that is not the id in any base.
    //
    // MEASURED by running a host with `BOYKO_LOG_PRESET=shipping` and decoding the `.blog`: the
    // file's own `Session` frame read `fe63ff00ba2f385d7ff64cce15042cc5` and the header beside it
    // read that decimal. The id exists so an uploaded log and an uploaded profiling artifact can be
    // proved to be one run; two representations that cannot be compared defeat exactly that.
    //
    // Rendered into a stack buffer and passed as `&str`, so the sink still receives a plain value
    // and nothing formats on the drain thread.
    let mut hex = [0u8; 32];
    for (i, byte) in hex.iter_mut().enumerate() {
        let nibble = if i < 16 {
            (session.1 >> (60 - 4 * i)) & 0xF
        } else {
            (session.0 >> (60 - 4 * (i - 16))) & 0xF
        } as u8;
        *byte = if nibble < 10 { b'0' + nibble } else { b'a' + nibble - 10 };
    }
    // SAFETY: every byte written above is an ASCII hex digit.
    let hex = unsafe { core::str::from_utf8_unchecked(&hex) };
    crate::info!(
        crate::Log,
        "build_profile={} runtime_preset={} ceiling={} session={}",
        boyko_diag::profile::PROFILE_NAME,
        // `None` prints `custom`, and that is not a cosmetic choice: a host that built its own
        // `LogConfig` selected no preset, and naming one would send a reader to reason about a row
        // of the table above that nobody chose.
        match preset {
            Some(p) => p.name(),
            None => "custom",
        },
        crate::GLOBAL_CEILING.as_str(),
        hex
    );
}
