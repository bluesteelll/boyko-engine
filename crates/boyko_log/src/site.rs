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
    /// Which target's control byte gated this site.
    pub target: TargetId,
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
    /// Monomorphised **per argument-tuple type**; identical tuples share one instantiation.
    ///
    /// Cold: called on the sink thread, from the staging arena, never on the emitting thread.
    ///
    /// # Safety
    ///
    /// The caller passes a pointer to `len` bytes produced by `LogArgs::encode` for the **same**
    /// tuple type this pointer was monomorphised from. Pairing a payload with the wrong decoder
    /// reads a differently-shaped record.
    pub decode: unsafe fn(*const u8, usize, &mut LogFormatter),
}

// `LogSite` is immutable `'static` data reachable from every thread through a record header, so
// both marker traits are needed and both are trivially justified. Stated with a `const` assert
// rather than left to inference, because adding a mutable or thread-affine field later would
// otherwise silently make records unsound to hand to the sink.
const _: () = {
    const fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<&'static LogSite>();
};

/// Rung L1's decoder: reports the payload size and nothing else.
///
/// **The monomorphised-per-tuple decoder is NOT here, and is not silently missing.** Rendering a
/// payload means interleaving values with the format literal's placeholders, and that policy —
/// timestamps, level names, code prefixes, field labels, what a `{}` means — belongs to the sink,
/// which does not exist until a later rung. Naming the argument-tuple type at the `static` is
/// also not expressible today: the site is `&'static` and Rust has no generic statics, so the
/// per-`(site, tuple)` instantiation arrives with the same rung that gives it something to
/// render. Gate **G5**, the distinct-decode-symbol census, lands there for the same reason: at
/// this rung there is one symbol by construction, so the census could not go red.
///
/// # Safety
///
/// Matches the [`LogSite::decode`] contract; this implementation reads nothing through `src`.
pub unsafe fn decode_opaque(_src: *const u8, len: usize, f: &mut LogFormatter) {
    f.write_fmt(format_args!("<{len} payload bytes; decoder arrives with the sink>"));
}

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
