//! The recoverable-error channel for `.ui` parsing (P3 Decision 6).
//!
//! Mirrors `boyko_input::persist::grammar::ParseReport`, extended with a byte
//! column so a field-level error (a bad float in the 3rd of 5 fields) is
//! locatable without eyeballing the line. Parsing NEVER fails at the file level:
//! a malformed construct is recorded here and skipped, the rest parses — the
//! `.keys` contract, which a hand- and LLM-edited `.ui` file inherits.

/// The current `.ui` format version the serializer emits and the parser fully
/// understands. A file declaring a *higher* version loads best-effort with a
/// warning (mirrors `KEYS_FORMAT_VERSION`).
pub const UI_FORMAT_VERSION: u32 = 1;

/// The outcome of parsing a `.ui` source: every recoverable per-line / per-field
/// error and every non-fatal warning, plus the parsed version. Parsing always
/// succeeds at the file level — these are observations for logging / a UI, not a
/// failure channel.
#[derive(Clone, Debug, Default)]
pub struct UiParseReport {
    /// The `version=N` value read from the file, or [`UI_FORMAT_VERSION`] if the
    /// file omitted it (a forward-compat default).
    pub version: u32,
    /// `(1-based line, 0-based byte col, reason)` for every construct that could
    /// not be parsed and was skipped. The col is `0` for whole-line errors
    /// (unknown component, off-step indent) and the field's byte offset for a
    /// bad-value error.
    pub errors: Vec<(usize, u16, String)>,
    /// Non-fatal advisories — e.g. an unknown higher format version, or an
    /// anonymous node carrying state-bearing structure (Decision 11 nudge).
    pub warnings: Vec<(usize, u16, String)>,
    /// The 1-based line currently being parsed. A transient cursor set by the
    /// parser before dispatching one component line, so the per-field leaf
    /// parsers report `(line, col)` without threading the line through every
    /// field closure. Not part of the public error contract.
    current_line: usize,
}

impl UiParseReport {
    /// Constructs an empty report seeded with the default version.
    #[inline]
    pub(crate) fn new() -> Self {
        Self {
            version: UI_FORMAT_VERSION,
            errors: Vec::new(),
            warnings: Vec::new(),
            current_line: 0,
        }
    }

    /// Sets the transient current-line cursor (called by the parser before
    /// dispatching a component line).
    #[inline]
    pub(crate) fn set_current_line(&mut self, line: usize) {
        self.current_line = line;
    }

    /// Reads the transient current-line cursor (read by the per-field leaf
    /// parsers).
    #[inline]
    pub(crate) fn current_line(&self) -> usize {
        self.current_line
    }

    /// `true` iff no per-line / per-field error was recorded (warnings do not
    /// count).
    #[inline]
    pub fn is_clean(&self) -> bool {
        self.errors.is_empty()
    }

    /// Records a recoverable error at `(line, col)`.
    #[inline]
    pub(crate) fn error(&mut self, line: usize, col: u16, reason: impl Into<String>) {
        self.errors.push((line, col, reason.into()));
    }

    /// Records a non-fatal warning at `(line, col)`.
    #[inline]
    pub(crate) fn warn(&mut self, line: usize, col: u16, reason: impl Into<String>) {
        self.warnings.push((line, col, reason.into()));
    }
}
