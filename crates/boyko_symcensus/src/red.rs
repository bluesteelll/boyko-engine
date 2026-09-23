//! The one error type of the instrument: a RED, named by its cause.
//!
//! Every failure this crate can report is a RED and none of them is a skip. The kinds are
//! distinct because they answer distinct questions: "the tool censused nothing" and "the tool
//! read the whole table and your symbol is not in it" produced the same number in the instrument
//! this crate descends from (`crates/profile_fixture/tests/profile_axis_census.rs`, its SECOND
//! FINDING), and that is how a whole gate went dark on one host.

use std::fmt;

/// Why a run is RED.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RedKind {
    /// A required tool is on no path the resolver searches, or an override names a missing file.
    ToolAbsent,
    /// A tool exists but could not be run, exited non-zero, or printed no version line.
    ToolFailed,
    /// A census read nothing: `no symbols` on stderr, or not one symbol line.
    EmptyCensus,
    /// The build finished and there is no post-LTO object for the unit.
    NoObject,
    /// The object and the image are not provably the same invocation's output.
    PairGuard,
    /// `CARGO_TARGET_DIR` is unset, relative, or unusable.
    TargetDir,
    /// The target dir's `.ug15-host` marker names another host triple.
    HostMismatch,
    /// `cargo` failed, or its message stream lacks the unit asked for.
    BuildFailed,
    /// A pinned or required symbol is not defined in the object.
    SymbolAbsent,
    /// An rlib census read zero object members.
    MemberCount,
    /// An rlib member is neither COFF, ELF nor LLVM bitcode.
    MemberFormat,
    /// A tool printed something this crate's parser does not recognise.
    Malformed,
    /// A file could not be read or written.
    Io,
    /// The command line asked for something that does not exist.
    Usage,
    /// Two captures that must be identical are not.
    Mismatch,
    /// The committed leg-(2) data is not the set capture froze: a pin file, a pin block, a
    /// frozen row or a candidate's disposition (pinned or recorded absent) is missing or extra.
    PinSet,
}

impl RedKind {
    /// The short label printed in `RED [<label>]`.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::ToolAbsent => "tool absent",
            Self::ToolFailed => "tool failed",
            Self::EmptyCensus => "empty census",
            Self::NoObject => "no post-LTO object",
            Self::PairGuard => "pair guard",
            Self::TargetDir => "target dir",
            Self::HostMismatch => "host mismatch",
            Self::BuildFailed => "build failed",
            Self::SymbolAbsent => "symbol absent",
            Self::MemberCount => "member count",
            Self::MemberFormat => "member format",
            Self::Malformed => "malformed tool output",
            Self::Io => "io",
            Self::Usage => "usage",
            Self::Mismatch => "mismatch",
            Self::PinSet => "pin set",
        }
    }
}

/// A RED: its kind and a sentence naming what was measured.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Red {
    /// The cause.
    pub kind: RedKind,
    /// What was read, from where, and why it cannot be a pass.
    pub detail: String,
}

impl Red {
    /// A RED of `kind` with `detail`.
    #[cold]
    #[must_use]
    pub fn new(kind: RedKind, detail: impl Into<String>) -> Self {
        Self { kind, detail: detail.into() }
    }

    /// An I/O failure on `path`.
    #[cold]
    #[must_use]
    pub fn io(path: &std::path::Path, err: &std::io::Error) -> Self {
        Self::new(RedKind::Io, format!("{}: {err}", path.display()))
    }
}

impl fmt::Display for Red {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "RED [{}]: {}", self.kind.label(), self.detail)
    }
}

impl std::error::Error for Red {}

/// The crate's result type.
pub type Result<T> = std::result::Result<T, Red>;
