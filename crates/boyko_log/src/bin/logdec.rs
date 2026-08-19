//! `logdec` — read a `.blog` back into text.
//!
//! ```text
//! logdec <file.blog> [more.blog …]
//! ```
//!
//! # Why this is a rung and not a convenience
//!
//! L13b gave the engine a binary format, a site dictionary, `W0116` and — a commit later — a
//! destination. Nothing could read the result. A format with a writer and no reader is the same
//! defect as a format with neither: the bytes are produced, the gates are green, and the one thing
//! the format exists for cannot be done. The only decoder in the tree was a private walker inside
//! one test, which proved a decoder that no tool used.
//!
//! # It shares the library's walker, deliberately
//!
//! `boyko_log::sink::binary::frames` is the one walker. This tool has no parsing of its own, so a
//! change to the format cannot pass the format test and break the tool, or the reverse.
//!
//! # Two format defects this tool found, which no writer-side test could
//!
//! * The `InlineSite` frame carried file and line and **not the format literal**, while the
//!   module header said it carried all three. Such a record was locatable and not renderable.
//! * The `Anchor` carried ticks and **not the scale**. A tick count is a property of the CPU that
//!   produced it, so a reader on another machine could print `+41231 ticks` and nothing better.
//!
//! Both are fixed in the writer. Finding them required someone to try to read the file.

use std::io::Write as _;

use boyko_log::record::{DspBuf, MAX_RENDERED_BYTES, render_payload};
use boyko_log::sink::binary::{Frame, frames};
use boyko_log::site::LogFormatter;

/// One replayed dictionary entry.
#[derive(Clone, Copy)]
struct Site<'a> {
    file: &'a str,
    line: u32,
    fmt: &'a str,
}

/// Exit codes. `2` for usage, `1` for a file that could not be read, `0` otherwise — including a
/// file with a ragged tail, which is reported and is not a failure.
const EXIT_USAGE: i32 = 2;
const EXIT_IO: i32 = 1;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() || args.iter().any(|a| a == "-h" || a == "--help") {
        eprintln!("usage: logdec <file.blog> [more.blog ...]");
        eprintln!();
        eprintln!("Reads the binary log format written by boyko_log's binary sink and prints one");
        eprintln!("line per record. A file whose tail was cut off mid-write is decoded up to the");
        eprintln!("cut and the remainder is reported -- that file is usually the interesting one.");
        std::process::exit(EXIT_USAGE);
    }

    let mut code = 0;
    for path in &args {
        match std::fs::read(path) {
            Ok(bytes) => decode_one(path, &bytes),
            Err(e) => {
                eprintln!("logdec: {path}: {e}");
                code = EXIT_IO;
            }
        }
    }
    std::process::exit(code);
}

/// Decode one file to stdout.
fn decode_one(path: &str, bytes: &[u8]) {
    let out = std::io::stdout();
    let mut out = std::io::BufWriter::new(out.lock());
    let _ = writeln!(out, "== {path} ({} bytes)", bytes.len());

    // Indexed by `site_id`, which the writer hands out densely from zero. A `Vec` rather than a map
    // because the ids ARE the indices -- the same reason the rate table needs no hash.
    let mut dict: Vec<Option<Site<'_>>> = Vec::new();
    // The scale from the most recent anchor. `0.0` until one is seen, which is how a file that does
    // not open with an anchor is reported rather than silently timed from zero.
    let mut ticks_per_ns = 0.0f64;
    let mut anchor_ticks = 0u64;
    let mut records = 0u64;
    let mut line_buf = DspBuf::<MAX_RENDERED_BYTES>::new();

    let mut walk = frames(bytes);
    for frame in walk.by_ref() {
        match frame {
            Frame::Anchor { ticks, ticks_per_ns: scale } => {
                anchor_ticks = ticks;
                ticks_per_ns = scale;
                let _ = writeln!(out, "-- anchor ticks={ticks} ticks_per_ns={scale}");
            }
            Frame::Dictionary { site_id, line, file, fmt } => {
                let i = site_id as usize;
                if dict.len() <= i {
                    dict.resize(i + 1, None);
                }
                dict[i] = Some(Site { file, line, fmt });
            }
            Frame::Record(r) => {
                records += 1;
                let Some(Some(site)) = dict.get(r.site_id as usize).copied() else {
                    // A record naming a site the file never defined. Reported per record rather
                    // than skipped: it means the dictionary frame was lost, and a reader who is
                    // shown nothing concludes the record was not there.
                    let _ = writeln!(
                        out,
                        "{:>12}  <site {} not in this file's dictionary>",
                        stamp(r.tsc_delta, ticks_per_ns),
                        r.site_id
                    );
                    continue;
                };
                line_buf.clear();
                let mut f = LogFormatter::new(&mut line_buf);
                render_payload(r.payload, site.fmt, &mut f);
                let _ = writeln!(
                    out,
                    "{:>12}  {}:{}  {}",
                    stamp(r.tsc_delta, ticks_per_ns),
                    site.file,
                    site.line,
                    line_buf.as_str()
                );
            }
            Frame::InlineSite { line, file, fmt, payload } => {
                records += 1;
                line_buf.clear();
                let mut f = LogFormatter::new(&mut line_buf);
                render_payload(payload, fmt, &mut f);
                // No timestamp: an inline record carries no `tsc_delta`. Stated, rather than
                // printed as `+0.000ms`, which a reader would take for a time.
                let _ = writeln!(out, "{:>12}  {file}:{line}  {}", "[no stamp]", line_buf.as_str());
            }
        }
    }

    let consumed = walk.consumed();
    let _ = writeln!(out, "-- {records} record(s), {} site(s) in the dictionary", dict.len());
    if consumed < bytes.len() {
        // NOT AN ERROR. The file a reader most wants is the one a crash cut off mid-write, so a
        // ragged tail is the ordinary case for the interesting file. It is reported because a
        // reader must not mistake "decoded to the cut" for "decoded to the end".
        let _ = writeln!(
            out,
            "-- RAGGED TAIL: {} of {} bytes decoded; the remaining {} were cut off mid-write",
            consumed,
            bytes.len(),
            bytes.len() - consumed
        );
    }
    if ticks_per_ns == 0.0 {
        let _ = writeln!(
            out,
            "-- NO ANCHOR: this file carries no clock scale, so every stamp above is in RAW TICKS"
        );
    }
    let _ = anchor_ticks;
}

/// Format a tick delta as milliseconds since the anchor, or as raw ticks with no anchor.
///
/// The unit is never guessed. A file with no anchor prints `t=NNN`, which cannot be mistaken for a
/// duration, rather than a plausible-looking `0.014ms` computed against a scale of one.
fn stamp(delta: u32, ticks_per_ns: f64) -> String {
    if ticks_per_ns <= 0.0 {
        return format!("t={delta}");
    }
    format!("+{:.3}ms", f64::from(delta) / ticks_per_ns / 1.0e6)
}
