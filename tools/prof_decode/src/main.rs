//! `prof_decode` — render a profiling telemetry stream as text.
//!
//! Profiling rung 13. The stream is written by `boyko_app::profiling::stream`; the format is
//! `boyko_diag::telemetry`, and this is its only reader.
//!
//! ```text
//! prof_decode <file> [--zones] [--windows] [--csv]
//! ```
//!
//! # What it prints, and what it refuses to print
//!
//! The header, then one line per zone row, then one line per window record. Three numbers are
//! **never** printed as values:
//!
//! * A `median` / `p95` for a zone outside the quantile subscription. The record's flag says the
//!   fields are absent; this prints `-`. A zero there would be read as a measurement.
//! * A tick figure converted to nanoseconds when the clock was never calibrated. `ticks_per_ns`
//!   is `1.0` in that state, and multiplying by it produces a number that looks like nanoseconds.
//!   The header's `invariant-tsc` flag and the window's `clock-uncalibrated` flag both print, and
//!   the conversion is suppressed.
//! * Anything from a torn block. `truncated_tail_bytes` prints; the bytes do not decode.
//!
//! # Exit codes
//!
//! `0` the file decoded, whole or with a stated tail. `1` the file could not be opened or its
//! header was refused. **A torn tail is not a failure**: a player's disk filling up is the normal
//! way a telemetry file ends, and a decoder that exited non-zero on it would make every such
//! session look like a broken one.

// This binary IS the human-readable rendering the corpus assigns to a tool rather than to the
// engine, so `println!` is its whole purpose. The engine crates deny it; this one is where the
// exception lives, stated once here rather than per call.
#![allow(clippy::print_stdout, clippy::print_stderr)]

use std::process::ExitCode;

use boyko_diag::telemetry::{
    self, Block, DecodeError, Decoded, HEADER_FLAG_INVARIANT_TSC, REC_OVER_RANGE, StreamHeader,
    WINDOW_FLAG_CLOCK_UNCALIBRATED, WindowRec, ZoneRow,
};

fn main() -> ExitCode {
    let mut args = std::env::args_os().skip(1);
    let Some(path) = args.next() else {
        eprintln!("usage: prof_decode <file> [--zones] [--windows] [--csv]");
        return ExitCode::FAILURE;
    };
    let mut show_zones = false;
    let mut show_windows = false;
    let mut csv = false;
    for a in args {
        match a.to_string_lossy().as_ref() {
            "--zones" => show_zones = true,
            "--windows" => show_windows = true,
            "--csv" => {
                csv = true;
                show_windows = true;
            }
            other => {
                eprintln!("prof_decode: unknown option {other}");
                return ExitCode::FAILURE;
            }
        }
    }
    if !show_zones && !show_windows {
        show_zones = true;
        show_windows = true;
    }

    let bytes = match std::fs::read(&path) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("prof_decode: cannot read {}: {e}", path.to_string_lossy());
            return ExitCode::FAILURE;
        }
    };

    let decoded = match telemetry::decode(&bytes) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("prof_decode: {}", describe(e));
            return ExitCode::FAILURE;
        }
    };

    if csv {
        print_csv(&decoded);
    } else {
        print_header(&decoded.header, bytes.len());
        print_summary(&decoded);
        if show_zones {
            print_zones(&decoded);
        }
        if show_windows {
            print_windows(&decoded);
        }
    }
    ExitCode::SUCCESS
}

/// A header refusal, in words rather than in a variant name.
fn describe(e: DecodeError) -> String {
    match e {
        DecodeError::TooShort { len } => format!(
            "the file is {len} bytes; a stream header is {} — there is no header to disagree with",
            telemetry::HEADER_BYTES
        ),
        DecodeError::NotAStream { magic } => {
            format!("not a telemetry stream: the first four bytes are {magic:#010x}")
        }
        DecodeError::Schema { found } => format!(
            "the file claims schema version {found} and this decoder speaks {}; a block layout is \
             exactly what a version changes, so nothing after the header was read",
            telemetry::TELEMETRY_SCHEMA_VERSION
        ),
    }
}

fn print_header(h: &StreamHeader, file_bytes: usize) {
    println!("stream            schema {} · {file_bytes} bytes on disk", h.schema_version);
    println!("session           {:016x}{:016x}", h.session_hi, h.session_lo);
    if h.run_id == 0 {
        println!("run               (none declared)");
    } else {
        println!("run               {}", h.run_id);
    }
    if h.player_tag == [0u8; 16] {
        println!("player tag        (none)");
    } else {
        let tag: String = h.player_tag.iter().map(|b| format!("{b:02x}")).collect();
        println!("player tag        {tag}");
    }
    println!("clock epoch       {}", h.clock_epoch);
    let tsc = if h.flags & HEADER_FLAG_INVARIANT_TSC != 0 { "yes" } else { "NO" };
    println!("invariant tsc     {tsc}");
    if calibrated(h) {
        println!("ticks/ns          {:.6}", h.ticks_per_ns);
    } else {
        // `1.0` is the uncalibrated arm's return, not a measurement of a 1 GHz clock. Printing it
        // as a scale would licence every tick figure below to be read as nanoseconds.
        println!("ticks/ns          (uncalibrated — tick figures are NOT nanoseconds)");
    }
    println!("geometry          {} zones × {} frames", h.zone_stride, h.window);
    println!("build             log ceiling {} · profiling tier {} · region capacity {}",
        h.log_ceiling, h.profiling_tier, h.region_capacity);
}

/// Whether the header's scale is a measurement. `1.0` is what `ticks_per_ns` returns when the clock
/// was never calibrated, and it is the one value that must not be used as a conversion.
fn calibrated(h: &StreamHeader) -> bool {
    (h.ticks_per_ns - 1.0).abs() > f64::EPSILON
}

fn print_summary(d: &Decoded<'_>) {
    println!();
    println!("blocks ok         {}", d.blocks_ok());
    println!("records ok        {}", d.records_ok);
    if d.truncated_tail_bytes == 0 {
        println!("tail              none — every byte decoded");
    } else {
        // The normal end of a player's telemetry file, not an error. Named as what it is.
        println!(
            "tail              {} bytes NOT decoded — the last block is torn or corrupt, and none \
             of its records are reported",
            d.truncated_tail_bytes
        );
    }
}

fn print_zones(d: &Decoded<'_>) {
    println!();
    println!("{:>6}  {:<8} {:>5} {:>5} {:>7}  name", "id", "kind", "scope", "tier", "region");
    for b in &d.blocks {
        for r in &b.zone_rows {
            println!(
                "{:>6}  {:<8} {:>5} {:>5} {:>7}  {}",
                r.id,
                kind_word(r),
                r.scope,
                r.tier,
                region_word(r.region),
                String::from_utf8_lossy(r.name)
            );
        }
    }
}

/// The observed kind, or `?` — which is a real state and not a gap: a `ZoneDesc` carries no kind,
/// so a zone the fold never saw a sample for has no kind to report, and `total` below has no unit.
fn kind_word(r: &ZoneRow<'_>) -> &'static str {
    match telemetry::kind_of_byte(r.kind) {
        Some(boyko_diag::sample::SampleKind::Span) => "span",
        Some(boyko_diag::sample::SampleKind::Counter) => "counter",
        Some(boyko_diag::sample::SampleKind::Gauge) => "gauge",
        None => "?",
    }
}

fn region_word(region: u8) -> &'static str {
    match region {
        0 => "engine",
        1 => "user",
        _ => "?",
    }
}

fn print_windows(d: &Decoded<'_>) {
    let cal = calibrated(&d.header);
    for b in &d.blocks {
        println!();
        let uncal = if b.head.flags & WINDOW_FLAG_CLOCK_UNCALIBRATED != 0 {
            "  [CLOCK UNCALIBRATED]"
        } else {
            ""
        };
        println!(
            "window {:<4} frames {}..={} · epoch {} · drops {} · fixed_elapsed {} ns{uncal}",
            b.seq, b.head.frame_first, b.head.frame_last, b.head.clock_epoch, b.head.drops,
            b.head.fixed_elapsed_ns
        );
        println!(
            "{:>6} {:>9} {:>14} {:>10} {:>10} {:>10} {:>10}  flags",
            "id", "count", "total", "min", "max", "median", "p95"
        );
        for r in &b.recs {
            let (median, p95) = match r.quantiles() {
                // Absent, not zero. The whole reason the format carries a flag rather than a
                // sentinel value.
                None => ("-".to_string(), "-".to_string()),
                Some((m, p)) => (scale(u64::from(m), &d.header, cal), scale(u64::from(p), &d.header, cal)),
            };
            let flags = if r.flags & REC_OVER_RANGE != 0 { "over-range" } else { "" };
            println!(
                "{:>6} {:>9} {:>14} {:>10} {:>10} {:>10} {:>10}  {flags}",
                r.id,
                r.count,
                scale(r.total, &d.header, cal),
                scale(u64::from(r.min), &d.header, cal),
                scale(u64::from(r.max), &d.header, cal),
                median,
                p95,
            );
        }
    }
}

/// A tick figure, converted only when the header says the scale is a measurement.
fn scale(ticks: u64, h: &StreamHeader, cal: bool) -> String {
    if cal {
        format!("{:.1}us", ticks as f64 / h.ticks_per_ns / 1000.0)
    } else {
        format!("{ticks}t")
    }
}

/// One row per record, machine-readable. `median`/`p95` are EMPTY when absent, never `0`.
fn print_csv(d: &Decoded<'_>) {
    println!("seq,frame_first,frame_last,epoch,window_drops,fixed_elapsed_ns,id,count,total,min,max,median,p95,over_range");
    for b in &d.blocks {
        for r in &b.recs {
            print_csv_row(b, r);
        }
    }
}

fn print_csv_row(b: &Block<'_>, r: &WindowRec) {
    let (median, p95) = match r.quantiles() {
        None => (String::new(), String::new()),
        Some((m, p)) => (m.to_string(), p.to_string()),
    };
    println!(
        "{},{},{},{},{},{},{},{},{},{},{},{median},{p95},{}",
        b.seq,
        b.head.frame_first,
        b.head.frame_last,
        b.head.clock_epoch,
        b.head.drops,
        b.head.fixed_elapsed_ns,
        r.id,
        r.count,
        r.total,
        r.min,
        r.max,
        u8::from(r.flags & REC_OVER_RANGE != 0),
    );
}
