//! Profiling rung 13's gates — **`G15`** (the stream survives a kill and a torn write), **`G26`**
//! (the telemetry window's cost is measured, not assumed) and **`G9`'s telemetry clause**.
//!
//! # One geometry, one lane, one lock — the rung-12 lesson applied
//!
//! `ARM_MASK`, `ARMED_STRIDE` and the lane rings are process-global. Every fixture here arms with
//! the same [`GEOMETRY`] and holds [`SERIAL`], because a second geometry is refused (correctly) and
//! because two armed tests would fold each other's samples.
//!
//! Every zone these gates assert totals over is **minted**, never a hand-picked id. Rung 12 measured
//! what a hand-picked id costs: `G18` counted 20 013 of 20 000 samples because a per-system zone had
//! landed on the number it chose.

#![allow(clippy::disallowed_types)]

use std::io;
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard};

use boyko_app::profiling::stream::{
    FileSink, MAX_TELEMETRY_QUANTILE_ZONES, TelemetryConfig, TelemetrySink, TelemetryStream,
    counters,
};
use boyko_diag::lane::set_lane;
use boyko_diag::sample::{Region, Sample, SampleKind};
use boyko_diag::telemetry::{self, REC_HAS_QUANTILES};
use boyko_diag::{clock, profiling_abi, sample};
use boyko_ecs::ecs::core::profiling::{
    ArmOutcome, Profiler, ProfilerConfig, ROOT_SCOPE, WINDOW, diag, fold,
};

/// This binary's one geometry. `hist_slots: 0` — tier C is not what rung 13 measures.
const GEOMETRY: ProfilerConfig = ProfilerConfig { user_zone_budget: 0, hist_slots: 0 };

/// The lane every fixture writes on.
const TEST_LANE: u16 = 3;

/// Serialises the fixtures. `Mutex` is on the project's disallowed list for the lock-free
/// discipline; this is the sanctioned test-fixture shape, the same one `profiling::store` uses.
static SERIAL: Mutex<()> = Mutex::new(());

boyko_diag::profiling_partition!(Engine);

boyko_diag::declare_zone!(
    T13_A,
    name = "t13.a",
    scope = ROOT_SCOPE,
    tier = boyko_diag::profiling_abi::ZoneTier::Always,
);
boyko_diag::declare_zone!(
    T13_B,
    name = "t13.b",
    scope = ROOT_SCOPE,
    tier = boyko_diag::profiling_abi::ZoneTier::Always,
);

fn serial() -> MutexGuard<'static, ()> {
    SERIAL.lock().unwrap_or_else(|p| p.into_inner())
}

/// Arm a fresh store on the test lane.
fn armed() -> (MutexGuard<'static, ()>, Profiler) {
    let guard = serial();
    set_lane(TEST_LANE);
    let mut p = Profiler::new();
    let outcome = p.arm(GEOMETRY);
    assert!(
        matches!(outcome, ArmOutcome::Armed | ArmOutcome::Rearmed),
        "the canonical geometry must always arm: {outcome:?}"
    );
    (guard, p)
}

/// A scratch path under the OS temp dir, unique per test name.
fn scratch(name: &str) -> PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!("boyko_prof_{name}_{}.bin", std::process::id()));
    p
}

/// Drive `frames` frames, pushing one span per zone per frame, and fold each one.
fn drive(p: &mut Profiler, zones: &[u16], frames: u32) {
    for f in 0..frames {
        for (i, &z) in zones.iter().enumerate() {
            // Vary by frame AND by zone, so a reducer that transposed its indices would not
            // accidentally agree with the oracle.
            let value = 100 + u64::from(f % 61) * 7 + (i as u64 % 13);
            let s = Sample {
                stamp: clock::ticks(),
                value,
                zone: z,
                flags: SampleKind::Span as u16,
                _pad: 0,
            };
            assert!(sample::push(Region::Engine, s), "the engine region must accept a test sample");
        }
        fold(p);
    }
    // The last frame's samples are drained at the top of the frame after it.
    fold(p);
}

/// A sink that records every window in memory, so a gate can read the bytes without a file.
#[derive(Default)]
struct MemSink {
    bytes: Vec<u8>,
}

impl TelemetrySink for MemSink {
    fn write_all(&mut self, buf: &[u8]) -> io::Result<()> {
        self.bytes.extend_from_slice(buf);
        Ok(())
    }
}

/// A sink that writes the first `whole` windows and then returns after **half** of the next one —
/// the `ENOSPC` shape, which is `write_all` returning *after a partial write*.
struct ShortSink {
    bytes: Vec<u8>,
    whole: usize,
    windows: usize,
}

impl TelemetrySink for ShortSink {
    fn write_all(&mut self, buf: &[u8]) -> io::Result<()> {
        self.windows += 1;
        if self.windows <= self.whole {
            self.bytes.extend_from_slice(buf);
            return Ok(());
        }
        // Exactly the disk-full shape: the bytes that DID land stay on disk, and the caller is
        // told the write failed.
        let half = buf.len() / 2;
        self.bytes.extend_from_slice(&buf[..half]);
        Err(io::Error::other("simulated ENOSPC after a partial write"))
    }
}

// ════════════════════════════════════════════════════════════════════════════════════════════════
// G15 — the stream survives a kill and a torn write.
// ════════════════════════════════════════════════════════════════════════════════════════════════

/// **`G15` (a)** — after every window, the file already holds every window written so far.
///
/// # Why this and not one `process::abort`
///
/// The corpus's clause aborts mid-window once and checks the block count. That proves the bound at
/// **one** point in one run. This asserts it after **every** window: at no instant does a written
/// window live only in the process. A kill at any of those instants therefore loses at most the
/// window in flight, which is the bound `G15` exists to state — and it is deterministic, where an
/// abort's timing is not.
///
/// What it does NOT cover, stated rather than implied: power loss and a driver hang. Neither is
/// reachable from inside a process, by any gate.
///
/// # RED, run — and it landed somewhere other than the prediction
///
/// Give [`MemSink`] a 4 KiB pending buffer — cross-window buffering, which is what D23 forbids and
/// what wrapping the file in a `BufWriter` would give for free. Predicted: `left: 0, right: 1` on
/// the block count. MEASURED: **`what the writer wrote must decode: TooShort { len: 0 }`** — the
/// failure is one assertion EARLIER, because with buffering the sink holds not a short file but no
/// file at all: 2 KiB of window is under the flush threshold, so even the 128-byte header has not
/// arrived. A kill there loses the whole session, not one window.
#[test]
fn g15a_every_written_window_is_on_disk_before_the_next_one_starts() {
    let (_g, mut p) = armed();
    let zones = [profiling_abi::zone_id(&T13_A), profiling_abi::zone_id(&T13_B)];
    let cfg = TelemetryConfig {
        path: &scratch("g15a"),
        max_bytes: 0,
        run_id: 11,
        player_tag: [0; 16],
        zones: &zones,
        quantiles: &zones,
    };
    let mut s = TelemetryStream::with_sink(Some(MemSink::default()), &cfg, &p);

    for window in 1..=5u32 {
        drive(&mut p, &zones, WINDOW as u32);
        let head = s.reduce(&p, u64::from(window) * 16_000_000);
        assert!(s.write(&p, &head), "window {window} must reach the sink");

        let bytes = s.sink().expect("the sink survives a failure").bytes.clone();
        let d = telemetry::decode(&bytes).expect("what the writer wrote must decode");
        assert_eq!(
            d.blocks_ok(),
            u64::from(window),
            "after {window} windows the sink holds {} — a window that has not arrived is a window \
             a kill would lose, and the bound is ONE",
            d.blocks_ok()
        );
        assert_eq!(d.truncated_tail_bytes, 0, "a whole write leaves no tail");
        assert_eq!(d.records_ok, u64::from(window) * zones.len() as u64);
    }
}

/// **`G15` (b)** — the `ENOSPC` shape: the decoder returns `N−1` blocks, states the tail, and
/// returns **no** record from the torn block.
///
/// # RED, run
///
/// Delete the CRC check from the decoder's block walk and flip one payload bit: the walk accepts
/// the corrupt block and `blocks_ok` goes from 1 to 3 (measured in `boyko_diag::telemetry`'s own
/// clause). Here the tear is structural rather than a bit flip, so it is the `len`/CRC pair that
/// catches it — and the two clauses are separate for that reason.
#[test]
fn g15b_a_short_write_tears_exactly_one_block_and_the_decoder_says_so() {
    let (_g, mut p) = armed();
    let zones = [profiling_abi::zone_id(&T13_A), profiling_abi::zone_id(&T13_B)];
    let cfg = TelemetryConfig {
        path: &scratch("g15b"),
        max_bytes: 0,
        run_id: 0,
        player_tag: [0; 16],
        zones: &zones,
        quantiles: &[],
    };
    let before = counters().write_errors;
    let sink = ShortSink { bytes: Vec::new(), whole: 2, windows: 0 };
    let mut s = TelemetryStream::with_sink(Some(sink), &cfg, &p);

    let mut written = 0u64;
    for window in 0..4u32 {
        drive(&mut p, &zones, WINDOW as u32);
        let head = s.reduce(&p, u64::from(window));
        if s.write(&p, &head) {
            written += 1;
        }
    }

    assert_eq!(written, 2, "the sink accepts two windows and tears the third");
    assert!(!s.is_live(), "a failed write must disable streaming, not retry it");
    assert_eq!(
        counters().write_errors,
        before + 1,
        "exactly one write error: the fourth window must not even be attempted"
    );
    assert!(
        diag::report_count(9215) >= 1,
        "the write failure was silent — W9215 owes a reader the fact that the stream stops here"
    );

    let bytes = s.sink().expect("the sink survives a failure").bytes.clone();
    let d = telemetry::decode(&bytes).expect("the header and the whole blocks must still parse");
    assert_eq!(d.blocks_ok(), 2, "the torn block must not be decoded");
    assert_eq!(d.records_ok, 2 * zones.len() as u64, "and none of its records returned");
    assert!(
        d.truncated_tail_bytes > 0,
        "the bytes that DID land must be reported as an undecoded tail, not ignored"
    );

    // G15 (c): the round-trip property, stated against the framing rather than the whole file.
    let mut out = vec![0u8; bytes.len()];
    let n = telemetry::reencode(&d, &mut out).expect("re-encoding must fit inside the input");
    assert_eq!(n, bytes.len() - d.truncated_tail_bytes);
    assert_eq!(&out[..n], &bytes[..n], "the codec is not an exact inverse on a torn file");
}

/// **`G15` (b), the file leg** — the same bound through a real `File`, so the gate is not only a
/// statement about `MemSink`.
///
/// Also the rotation clause: a rotated file owes a header AND every zone row again, or its records
/// key to ids that resolve to nothing.
#[test]
fn g15b_file_a_rotated_file_carries_its_own_header_and_zone_rows() {
    let (_g, mut p) = armed();
    let zones = [profiling_abi::zone_id(&T13_A), profiling_abi::zone_id(&T13_B)];
    let path = scratch("g15rot");
    let cfg = TelemetryConfig {
        path: &path,
        // Small enough that the second window rotates: one window is 32 B of head plus 64 B of
        // records plus framing.
        max_bytes: 200,
        run_id: 3,
        player_tag: [9; 16],
        zones: &zones,
        quantiles: &zones,
    };
    let sink = FileSink::open(&path).expect("the temp dir must be writable");
    let mut s = TelemetryStream::with_sink(Some(sink), &cfg, &p);

    for window in 0..4u32 {
        drive(&mut p, &zones, WINDOW as u32);
        let head = s.reduce(&p, u64::from(window));
        assert!(s.write(&p, &head), "window {window} must reach the file");
    }

    let bytes = std::fs::read(&path).expect("the live file must be readable");
    let d = telemetry::decode(&bytes).expect("a rotated file must carry its OWN header");
    assert!(d.blocks_ok() >= 1, "the live file must hold at least the window that rotated into it");
    let rows: usize = d.blocks.iter().map(|b| b.zone_rows.len()).sum();
    assert_eq!(
        rows,
        zones.len(),
        "a rotated file owes every zone row again — records whose ids resolve to nothing are worse \
         than no file"
    );

    let mut prev = path.clone().into_os_string();
    prev.push(".prev");
    let old = std::fs::read(&prev).expect("rotation must leave the previous generation behind");
    let dp = telemetry::decode(&old).expect("the previous generation must decode on its own");
    assert!(dp.blocks_ok() >= 1);

    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(&prev);
}

// ════════════════════════════════════════════════════════════════════════════════════════════════
// G26 — the telemetry window's cost is measured, not assumed.
// ════════════════════════════════════════════════════════════════════════════════════════════════

/// **`G26`, the cap clause** — the 65th quantile subscription is refused, counted and reported.
///
/// Profile-independent, unlike the budget clause below, and it is the part of `G26` that carries
/// the design: `median` and `p95` are the only super-linear term in the whole telemetry path, so
/// the cap is what makes the budget a bound rather than a hope.
#[test]
fn g26_the_sixty_fifth_quantile_subscription_is_refused_counted_and_reported() {
    let (_g, mut p) = armed();
    let zones: Vec<u16> = (1..=65u16).collect();
    let before = counters().zones_refused;
    let cfg = TelemetryConfig {
        path: &scratch("g26cap"),
        max_bytes: 0,
        run_id: 0,
        player_tag: [0; 16],
        zones: &zones,
        quantiles: &zones,
    };
    let s = TelemetryStream::with_sink(Some(MemSink::default()), &cfg, &p);
    // MEASURED: the refusal RAISES a sticky bit; it does not emit. `report_raised` is driven by
    // the fold's destructive `take_raised`, so nothing reaches a reader until the next fold — and
    // a session that configured telemetry and then never folded again would never emit `W9218` at
    // all. This line is that fold, and it is the mechanism rather than test scaffolding.
    fold(&mut p);

    assert_eq!(s.zone_count(), 65, "every zone still STREAMS; only two fields are refused");
    assert_eq!(
        s.quantile_count(),
        MAX_TELEMETRY_QUANTILE_ZONES,
        "exactly the cap may carry quantiles"
    );
    assert_eq!(
        counters().zones_refused,
        before + 1,
        "the refusal must be counted — a silently dropped subscription gives a zone a median field \
         filled with something, and whatever that is gets read as a measurement"
    );
    assert!(
        diag::report_count(9218) >= 1,
        "the refusal was silent — W9218 owes a reader the difference between a p95 that was \
         REFUSED and one that is absent because the zone never ran"
    );
}

/// **`G26`, the absence clause** — a zone outside the subscription writes no quantile at all, and
/// the record says so with a flag rather than with a zero.
#[test]
fn g26_an_unsubscribed_zone_has_no_quantile_rather_than_a_zero_one() {
    let (_g, mut p) = armed();
    let a = profiling_abi::zone_id(&T13_A);
    let b = profiling_abi::zone_id(&T13_B);
    let zones = [a, b];
    let cfg = TelemetryConfig {
        path: &scratch("g26abs"),
        max_bytes: 0,
        run_id: 0,
        player_tag: [0; 16],
        zones: &zones,
        // Only `a` is subscribed.
        quantiles: &[a],
    };
    let mut s = TelemetryStream::with_sink(Some(MemSink::default()), &cfg, &p);
    drive(&mut p, &zones, WINDOW as u32);
    let head = s.reduce(&p, 42);
    assert!(s.write(&p, &head));

    let bytes = s.sink().expect("the sink survives a failure").bytes.clone();
    let d = telemetry::decode(&bytes).expect("decode");
    let recs = &d.blocks[0].recs;
    let ra = recs.iter().find(|r| r.id == a).expect("zone a must have a record");
    let rb = recs.iter().find(|r| r.id == b).expect("zone b must have a record too");

    assert!(ra.flags & REC_HAS_QUANTILES != 0, "the subscribed zone carries its quantiles");
    assert!(ra.quantiles().is_some());
    assert_eq!(
        rb.quantiles(),
        None,
        "an unsubscribed zone must report ABSENCE. `u32::MAX` could not have been the sentinel \
         either: the store clamps a cell's extrema to it and labels the cell OverRange, so it is a \
         REACHABLE value and would make 'nobody subscribed' look like 'this zone ran for four \
         billion ticks'"
    );
    assert!(rb.count > 0, "and it still streams everything the O(1) folds can give");
    assert!(rb.total > 0);
}

/// **`G26`, the cost clause** — the two halves are measured separately, **at the cap**, and their
/// sum is reported.
///
/// # Sixty-four zones, not two
///
/// The budget is stated *at 64 quantile zones*, and the first draft of this gate timed **two** —
/// a configuration the budget says nothing about, reported as if it were the budgeted figure. The
/// zones below are raw ids near the top of the engine range carrying pushed samples: a `ZoneDesc`
/// is not needed to time a reduction, and the record simply carries an empty name.
///
/// A hand-picked id is a bet, and rung 12 measured what it costs when a gate asserts a TOTAL. Here
/// the claim is a **duration**, so a colliding per-system zone could only make the measured cost
/// larger — never make a failing budget pass. The bet is safe for this clause and for no other.
///
/// # What is asserted, and in which profile
///
/// The budget (`reduce` <= 150 us, `write` <= 200 us, **sum <= 350 us**) is a claim about a RELEASE
/// build. A debug build's timings are an order of magnitude larger, and asserting the budget there
/// would red on every developer's machine while proving nothing about the shipped one — the
/// vacuous-gate pattern pointed the other way.
///
/// So the budget is asserted only under `not(debug_assertions)`, and what is asserted in **every**
/// profile is the property the budget encodes and a profile cannot change: **the reduce is the
/// dominant term and it is the one that scales with the quantile count.** That is M7's whole
/// finding, and a build where it stopped holding is a build where the cap protects nothing.
///
/// The three figures print either way, because a gate that measures and does not report has thrown
/// the measurement away.
///
/// # MEASURED, this box, release, 64 quantile zones — and the budget was MISSED before the fix
///
/// | | zone-major (A10's order) | row-major (shipped) |
/// |---|---|---|
/// | `__telemetry_reduce` p95 | **168.1 µs** — over the 150 µs budget | **128.0 µs** |
/// | the walk alone, no quantiles | 86.8 µs | **40.0 µs** |
/// | `__telemetry_write` p95 | 52.4 µs | 11.2 µs |
/// | **sum** | 220.5 µs | **139.2 µs** |
///
/// A10 specifies the reduce as *"gather 121 strided values"* per zone — zone-major. The columns are
/// frame-major, so that order steps by `zone_stride` and every one of the 7 744 cell reads is its
/// own cache line. It misses the budget on this box. Row-major touches each line once, and the
/// budget holds with 15 % of margin. **The corpus's own budget was only reachable by changing the
/// order the corpus prescribes**, and the reason is Principle 3 rather than anything about
/// telemetry.
///
/// # RED, run — and the corpus's own prediction for it is wrong on this box
///
/// `G26` says: *"Remove the cap ⇒ subscribe 400 ⇒ **the sum** exceeds the budget ⇒ red."* MEASURED
/// at `MAX_TELEMETRY_QUANTILE_ZONES = 400`: reduce 272.3 µs, write 48.3 µs, **sum 320.6 µs — still
/// under the 350 µs total.** The gate reds, but on the *reduce* term, not on the sum. After the
/// row-major fix the sum has enough headroom to absorb six times the cap's worth of zones, so a
/// gate that watched only the total would have let 400 through.
#[test]
fn g26_the_reduce_dominates_and_scales_with_the_quantile_count() {
    let (_g, mut p) = armed();
    let zones: Vec<u16> = (4000..4000 + MAX_TELEMETRY_QUANTILE_ZONES as u16).collect();
    drive(&mut p, &zones, WINDOW as u32);

    // The SAME window reduced twice: once with every zone's quantiles, once with none. The store,
    // the record count, the encode and the sink are identical, so the difference is the gather and
    // the sort and nothing else.
    // p95 over 32 runs, because the budget is stated as a p95. One `Instant::now()` pair is one
    // sample, and a single sample compared against a p95 is a category error the first draft made.
    let with = p95_ns(&mut p, &zones, &zones, time_reduce);
    let without = p95_ns(&mut p, &zones, &zones_empty(), time_reduce);
    let write_ns = p95_ns(&mut p, &zones, &zones, time_write);

    println!(
        "G26 measured at {} quantile zones: reduce(with) {with} ns - reduce(none) {without} ns - \
         write {write_ns} ns - sum {} ns",
        zones.len(),
        with + write_ns
    );

    assert!(
        with > without,
        "the reduce with quantiles ({with} ns) must cost MORE than the reduce without \
         ({without} ns) -- if it does not, the gather and the sort are not happening and the cap \
         protects nothing"
    );

    #[cfg(not(debug_assertions))]
    {
        // The budget, in the profile it was written for.
        assert!(with <= 150_000, "__telemetry_reduce p95 budget is 150 us, measured {with} ns");
        assert!(write_ns <= 200_000, "__telemetry_write budget is 200 us, measured {write_ns} ns");
        assert!(
            with + write_ns <= 350_000,
            "the telemetry window's total budget is 350 us, measured {} ns",
            with + write_ns
        );
    }
}

// ════════════════════════════════════════════════════════════════════════════════════════════════
// G9's telemetry clause — the instrument's own cost is DISCLOSED, and the telemetry frame shows it.
// ════════════════════════════════════════════════════════════════════════════════════════════════

/// **`G9` (d)** — the telemetry window's cost reaches the instrument's OWN account, and it is the
/// only thing in this harness that moves it.
///
/// # What `instrument_measured` is at this rung, and why rung 13 changed it
///
/// `zones.rs` defines it as `__fold`'s cell. D16 puts `__telemetry_reduce` and `__telemetry_write`
/// inside it too, so from rung 13 on the quantity is the **sum of the instrument's own zones** —
/// and that sum cannot be computed inside `boyko_ecs`, because two of the three are declared in a
/// crate above it. It is computed here, the only place that can see all three.
///
/// # `__fold` contributes ZERO in this harness, and the first draft of this gate hid that
///
/// The draft asserted `loud > quiet` and printed `quiet 0`. `__fold`'s bracket lives in
/// `fold_frame`, which needs an `EcsMaster`; this harness drives the free `fold` directly, so
/// `__fold` never opens and the baseline was a **structural zero**. `336534 > 0` is not a
/// measurement of anything: the clause would have stayed green with the entire telemetry pair
/// deleted, because `0 > 0` is the only thing it could ever have failed on.
///
/// What ships asserts the falsifiable shape instead — a quiet stretch adds **exactly nothing**, a
/// telemetry window adds **something**, both halves are individually non-zero, and the growth is
/// **exactly** those two zones and no third.
#[test]
fn g9d_a_frame_carrying_a_telemetry_window_discloses_its_cost() {
    let (_g, mut p) = armed();
    let zones = [profiling_abi::zone_id(&T13_A), profiling_abi::zone_id(&T13_B)];
    let cfg = TelemetryConfig {
        path: &scratch("g9d"),
        max_bytes: 0,
        run_id: 0,
        player_tag: [0; 16],
        zones: &zones,
        quantiles: &zones,
    };
    let mut s = TelemetryStream::with_sink(Some(MemSink::default()), &cfg, &p);

    // Two quiet stretches. The instrument's account must not move between them.
    drive(&mut p, &zones, WINDOW as u32);
    let quiet_a = instrument_ticks(&p);
    drive(&mut p, &zones, WINDOW as u32);
    let quiet_b = instrument_ticks(&p);
    assert_eq!(
        quiet_a, quiet_b,
        "a frame with no telemetry window must add no instrument time; it added {}",
        quiet_b.saturating_sub(quiet_a)
    );

    // A telemetry frame: reduce + write, then the folds that drain their own spans.
    let head = s.reduce(&p, 1);
    assert!(s.write(&p, &head));
    fold(&mut p);
    fold(&mut p);
    let loud = instrument_ticks(&p);

    let reduce_ticks = zone_ticks(&p, "__telemetry_reduce");
    let write_ticks = zone_ticks(&p, "__telemetry_write");
    println!(
        "G9(d) measured: instrument ticks quiet {quiet_b} - with telemetry {loud} - \
         __telemetry_reduce {reduce_ticks} - __telemetry_write {write_ticks} - __fold {}",
        zone_ticks(&p, "__fold")
    );

    assert!(
        loud > quiet_b,
        "a telemetry window added {} instrument ticks, which is not more than zero -- the \
         instrument's own work is not reaching its own account",
        loud.saturating_sub(quiet_b)
    );
    assert!(reduce_ticks > 0, "__telemetry_reduce is not disclosing its own cost");
    assert!(write_ticks > 0, "__telemetry_write is not disclosing its own cost");
    assert_eq!(
        loud - quiet_b,
        reduce_ticks + write_ticks,
        "the instrument's growth must be exactly the two telemetry zones -- anything else means a \
         third zone moved and the account no longer says what it measures"
    );
}

/// One named zone's whole-session ticks.
fn zone_ticks(p: &Profiler, name: &str) -> u64 {
    let mut total = 0u64;
    for id in 0..p.zone_stride() {
        let Ok(id) = u16::try_from(id) else { continue };
        let Some(d) = profiling_abi::zone_desc(id) else { continue };
        if d.name != name {
            continue;
        }
        if let Some(l) = p.lifetime(id) {
            total = total.wrapping_add(l.total);
        }
    }
    total
}

/// Σ of every instrument zone's lifetime ticks: `__fold` plus rung 13's two.
///
/// Read from the **lifetime accumulators** rather than from cells, because the two telemetry zones
/// open once per window and a per-frame cell would be zero in 120 frames out of 121 — which is a
/// structural zero, and the campaign's own rule is that one is indistinguishable from a measured
/// one.
fn instrument_ticks(p: &Profiler) -> u64 {
    let mut total = 0u64;
    for name in ["__fold", "__telemetry_reduce", "__telemetry_write"] {
        for id in 0..p.zone_stride() {
            let Ok(id) = u16::try_from(id) else { continue };
            let Some(d) = profiling_abi::zone_desc(id) else { continue };
            if d.name != name {
                continue;
            }
            if let Some(l) = p.lifetime(id) {
                total = total.wrapping_add(l.total);
            }
        }
    }
    total
}

/// An empty quantile list, as an owned value so the two calls above have the same shape.
fn zones_empty() -> Vec<u16> {
    Vec::new()
}

/// The p95 of 32 runs of `f`, in nanoseconds.
///
/// The budget is a p95, so the measurement is one. `sorted[(n * 0.95) as usize]` is `reduce.rs`'s
/// own convention, kept identical so two figures in this repository are never two conventions.
fn p95_ns(
    p: &mut Profiler,
    zones: &[u16],
    quantiles: &[u16],
    f: fn(&mut Profiler, &[u16], &[u16]) -> u64,
) -> u64 {
    const RUNS: usize = 32;
    let mut samples = [0u64; RUNS];
    for s in &mut samples {
        *s = f(p, zones, quantiles);
    }
    samples.sort_unstable();
    samples[((RUNS as f64 * 0.95) as usize).min(RUNS - 1)]
}

/// Wall-clock nanoseconds for one `reduce` over `zones`, with `quantiles` subscribed.
fn time_reduce(p: &mut Profiler, zones: &[u16], quantiles: &[u16]) -> u64 {
    let cfg = TelemetryConfig {
        path: &scratch("g26cost"),
        max_bytes: 0,
        run_id: 0,
        player_tag: [0; 16],
        zones,
        quantiles,
    };
    let mut s = TelemetryStream::with_sink(Some(MemSink::default()), &cfg, p);
    let t0 = std::time::Instant::now();
    let _ = s.reduce(p, 0);
    u64::try_from(t0.elapsed().as_nanos()).unwrap_or(u64::MAX)
}

/// Wall-clock nanoseconds for one `write`, the reduce excluded.
fn time_write(p: &mut Profiler, zones: &[u16], quantiles: &[u16]) -> u64 {
    let cfg = TelemetryConfig {
        path: &scratch("g26cost"),
        max_bytes: 0,
        run_id: 0,
        player_tag: [0; 16],
        zones,
        quantiles,
    };
    let mut s = TelemetryStream::with_sink(Some(MemSink::default()), &cfg, p);
    let head = s.reduce(p, 0);
    let t0 = std::time::Instant::now();
    assert!(s.write(p, &head));
    u64::try_from(t0.elapsed().as_nanos()).unwrap_or(u64::MAX)
}
