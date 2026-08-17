//! Per-call-site metadata, and the formatter a decoder writes into.

use crate::level::Level;
use crate::target::TargetId;

/// Immutable per-call-site metadata: one `'static` per macro expansion, referenced from the
/// record by pointer.
///
/// The record pays 8 bytes instead of re-carrying file, line, format literal and code — and none
/// of this is touched on the emitting thread. It is dereferenced **only** by the sink.
///
/// The per-site `Once` latch is deliberately **not** a field: this is immutable `&'static` data,
/// and a rate latch is mutable per-site state. The macro expands a sibling `static` beside each
/// rate-limited site instead, on its own line.
pub struct LogSite {
    /// Which target's control byte gated this site — or `None` at a **dynamic** site *(L10)*.
    ///
    /// # Why absence, and not a sentinel or a flag bit
    ///
    /// A `dyn_warn!(id, …)` site takes its target as a **runtime argument**, and the same site may
    /// be reached with a different id on every call — a loop over loaded mods is the ordinary case.
    /// So a dynamic site cannot carry its target here, and the question is where the reader learns
    /// that. Three answers were available and this is why the third won:
    ///
    /// * **A placeholder `TargetId`** (say the `log` row) would be a *lie a reader prints*. The
    ///   type deliberately has no `INVALID` — `01-EMISSION-RING.md` deletes it, because an in-band
    ///   sentinel that indexes an array is the same hazard in a nicer coat — so there is no honest
    ///   value to put here.
    /// * **A `DYNAMIC` bit in `RecordHeader::flags`** would work and costs nothing per record, but
    ///   it couples this decision to a byte with five bits left and spends one on a fact the site
    ///   already knows. The corpus records `clock_epoch_lo` *spending* the header's last pad byte;
    ///   the header is not the place to put things that fit elsewhere.
    /// * **`Option<TargetId>` here.** The site is cold `'static` data, never read on the emitting
    ///   thread except on the loss paths, so two bytes here are free in the sense that matters.
    ///   And "this site has no compile-time target" is *structurally* the same statement as
    ///   "absence is `Option<TargetId>`", which is already this crate's rule for target absence.
    ///
    /// The consequence is the contract: **when this is `None` the record's payload is prefixed
    /// with the target id as two little-endian bytes**, written by `emit_impl_dyn` and stripped by
    /// the drain. One reader, one writer, and the discriminant that selects between them is this
    /// field rather than a bit somebody has to remember to set.
    pub target: Option<TargetId>,
    /// The site's severity.
    pub level: Level,
    /// `b'B'`, `b'E'`, `b'W'`, or `0` when the level carries no code.
    pub class: u8,
    /// The printed code number; `0` when the level has none.
    pub code: u16,
    /// Source line.
    pub line: u32,
    /// Source file.
    pub file: &'static str,
    /// The format literal, printed by the sink.
    pub fmt: &'static str,
    /// Field names for the `*_kv!` forms; empty for positional forms.
    ///
    /// `&'static`, cold, and **never touched on the emission path** — which is the whole reason
    /// structured output costs the same as positional output.
    pub fields: &'static [&'static str],
    /// `"boyko"` for the engine; a game declares its own.
    pub prefix: &'static str,
}

// `LogSite` is immutable `'static` data reachable from every thread through a record header, so
// both marker traits are needed and both are trivially justified. Stated with a `const` assert
// rather than left to inference, because adding a mutable or thread-affine field later would
// otherwise silently make records unsound to hand to the sink.
const _: () = {
    const fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<&'static LogSite>();
};

// **There is no per-site `decode` field, and its removal is a measurement, not a simplification.**
//
// L1 gave `LogSite` a `decode: unsafe fn(*const u8, usize, &mut LogFormatter)` described as
// "monomorphised per argument-tuple type", and its own doc comment recorded why it was filled with
// a placeholder instead: the site is a `static` and Rust has no generic statics, so the tuple type
// cannot be named at the initialiser. It was never filled with anything else, **and no drain path
// ever called it** — every sink printed `site.fmt` with its placeholders intact, so a `warn!`
// carrying a set name rendered the literal `{}`. Measured at L6, which is the first rung whose
// whole content is call sites whose value IS their arguments.
//
// The replacement is `record::ValueTag`: one tag byte per value, one non-generic
// `record::render_payload`. See that module's header for the two facts that decided it — a
// per-site pointer can only be installed by publishing it at run time into a mutable site (atomics
// on the path the emitting thread is specified never to touch), and it cannot decode a `.blog`
// file in another process, which is `logdec`'s whole job at L13b.
//
// Gate **G5** — "distinct `decode` symbol upper bound" — loses its subject here and is STRUCK in
// the corpus rather than restated over the walker: there is exactly one, by construction, so a
// census over it could never go red.

/// Where a decoder writes its rendered output.
///
/// A byte sink and nothing more at this rung: the record-formatting policy — timestamps, level
/// names, code prefixes, field labels — belongs to the sink, which does not exist yet. What is
/// pinned here is the **shape**: a decoder appends and cannot fail, so a malformed payload
/// produces a short line rather than an error path inside the sink's drain loop.
pub struct LogFormatter<'a> {
    out: &'a mut dyn core::fmt::Write,
}

impl<'a> LogFormatter<'a> {
    /// Wrap a writer.
    pub fn new(out: &'a mut dyn core::fmt::Write) -> LogFormatter<'a> {
        LogFormatter { out }
    }

    /// Append text. Errors are dropped: a diagnostic that fails to render must not become a
    /// second failure to handle.
    pub fn write_str(&mut self, s: &str) {
        let _ = self.out.write_str(s);
    }

    /// Append a formatted value.
    pub fn write_fmt(&mut self, args: core::fmt::Arguments<'_>) {
        let _ = self.out.write_fmt(args);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formatter_appends_and_swallows_nothing_silently_on_a_good_writer() {
        let mut s = String::new();
        let mut f = LogFormatter::new(&mut s);
        f.write_str("a=");
        f.write_fmt(format_args!("{}", 7));
        assert_eq!(s, "a=7");
    }
}
