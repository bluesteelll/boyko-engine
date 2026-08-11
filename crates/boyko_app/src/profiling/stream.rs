//! Profiling rung 13 — the telemetry **writer**: one window in, one `write_all` out.
//!
//! [`boyko_diag::telemetry`] is the format; this is what fills it. The split is measured there: a
//! `prof_decode` rooted at `boyko_diag` has **2** crates in its tree, at `boyko_ecs` 12, at this
//! crate **45**.
//! What stays here is everything that needs a file, a frame loop or the host's own clock.
//!
//! # The two halves are separately budgeted because one of them is the whole cost (M7)
//!
//! [`TelemetryStream::reduce`] and [`TelemetryStream::write`] are two zones, not one, and the
//! reason is arithmetic. `count` / `total` / `min` / `max` are O(1) folds over the retained window.
//! `median` and `p95` are a **strided gather of every retained frame plus a sort, per zone** — so
//! at a few hundred subscribed zones the reduction is hundreds of gathers across a multi-megabyte
//! working set and hundreds of sorts, which is an order of magnitude more than the encode and the
//! syscall put together. [`MAX_TELEMETRY_QUANTILE_ZONES`] is the cap that bounds it; past the cap a
//! subscription is **refused, counted and reported once** (`W9218`), never silently dropped.
//!
//! | Term | Budget, p95 |
//! |---|---|
//! | `__telemetry_reduce` at the cap | ≤ 150 µs |
//! | `__telemetry_write` | ≤ 200 µs |
//! | **the sum** | **≤ 350 µs** |
//!
//! # The `.bss` double buffer is here; the FILE HANDLE cannot be, and that is forced
//!
//! D23 asks for *"a `.bss` process-static double buffer … not the `Profiler` `Resource` — the
//! no-`World` rule for `flush_on_panic`"*, and puts the file handle in the same breath. The buffer
//! is a `.bss` static, exactly as asked. **The handle is not, and cannot be:** `boyko_diag`'s
//! storage policy admits a static only through `ZeroInit`, whose whole premise is that the all-zero
//! bit pattern is a valid value — and `Option<File>` has no such guarantee on either platform this
//! workspace targets. So the handle lives in this struct, which the host owns and which is not the
//! `World` either. That satisfies the requirement the rule actually states.
//!
//! MEASURED while writing this: **`flush_on_panic` does not exist in the tree.** `rg` finds it in
//! `docs/diagnostics/SEAM.md` and nowhere in `crates/`. The buffer is `.bss` anyway because it
//! costs nothing to honour and because a page that is never written is never paged in — but the
//! *reason* the corpus gives for it is a mechanism that has not landed, and that is recorded rather
//! than repeated as if it had.
//!
//! # Failure: streaming stops, and nothing else changes
//!
//! A write error clears the stream's `live` flag, counts it, and raises `W9215` **once**. Never a
//! panic,
//! never a retry inside the frame: a write that failed in one frame will fail in the next, and
//! retrying puts an unbounded number of failing syscalls on the dispatcher at the exact moment the
//! machine is in trouble. Every whole block written before the failure stays in the file, which is
//! what makes the loss bound *one window* hold on this path too.
//!
//! # Rotation resets what is "once per FILE"
//!
//! A [`ZoneRow`] is written once per zone **per file**. Rotation therefore clears the introduced
//! set and re-writes the header, or the new file would carry records whose ids resolve to nothing —
//! a file full of anonymous numbers, which is worse than no file.

use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use boyko_diag::loss::{DiagFlag, raise};
use boyko_diag::profiling_abi::ZoneTier;
use boyko_diag::telemetry::{
    self, Encoder, HEADER_FLAG_INVARIANT_TSC, REC_HAS_QUANTILES, REC_OVER_RANGE, StreamHeader,
    WINDOW_FLAG_CLOCK_UNCALIBRATED, WindowHead, WindowRec, ZoneRow,
};
use boyko_diag::{clock, declare_zone, profile, storage::SyncCells};
use boyko_ecs::ecs::core::profiling::{
    CellLabel, FRAME_FLAG_CLOCK_UNCALIBRATED, FrameState, Profiler, ROOT_SCOPE, WINDOW,
};

declare_zone!(
    TELEMETRY_REDUCE,
    name = "__telemetry_reduce",
    scope = ROOT_SCOPE,
    tier = ZoneTier::Always,
);

declare_zone!(
    TELEMETRY_WRITE,
    name = "__telemetry_write",
    scope = ROOT_SCOPE,
    tier = ZoneTier::Always,
);

/// Zones that may carry `median` and `p95` in one session (M7's cap).
///
/// The number is D23's. What it bounds is the only super-linear term in the whole telemetry path.
pub const MAX_TELEMETRY_QUANTILE_ZONES: usize = 64;

/// Zones one session may stream at all.
///
/// A separate, much larger bound than the quantile cap, because the two limit different things: a
/// non-quantile record costs four O(1) folds and 32 bytes, and there is no reason to be stingy
/// with those.
pub const MAX_TELEMETRY_ZONES: usize = 512;

/// One window's encode buffer, per side of the double buffer.
///
/// A window's steady state is `32 + 32 × zones` — 16 KiB at the 512-zone ceiling — because
/// [`ZoneRow`]s are written once per zone per file and not per window. The slack is for that first
/// window, which carries every row.
pub const STREAM_BUFFER_BYTES: usize = 64 * 1024;

/// Bytes a telemetry file may reach before rotation.
pub const DEFAULT_MAX_BYTES: u64 = 64 * 1024 * 1024;

/// A zone with no quantile slot. `u16::MAX` rather than `0`, because `0` is a real slot.
const NO_QUANTILE_SLOT: u16 = u16::MAX;

/// The `.bss` double buffer. Two sides, so the encode of window *n+1* never touches bytes a reader
/// of window *n* could still be holding.
///
/// `SyncCells` rather than a `static mut`: it grants no `&`/`&mut`, so every access carries its
/// aliasing obligation explicitly. **This crate's obligation is discharged by the dispatcher**, not
/// by the lane topology the type's own docs describe — the writer runs on one thread, outside the
/// schedule, and that is stated at each `get_ptr`.
static BUFFERS: SyncCells<[u8; STREAM_BUFFER_BYTES], 2> = SyncCells::zeroed();

/// Windows successfully written, across the process.
static WINDOWS_WRITTEN: AtomicU64 = AtomicU64::new(0);
/// Bytes handed to the sink, across the process.
static BYTES_WRITTEN: AtomicU64 = AtomicU64::new(0);
/// Write failures. At most one per session, because the first disables streaming.
static WRITE_ERRORS: AtomicU64 = AtomicU64::new(0);
/// Quantile subscriptions refused past [`MAX_TELEMETRY_QUANTILE_ZONES`].
static ZONES_REFUSED: AtomicU64 = AtomicU64::new(0);
/// Subscriptions refused past [`MAX_TELEMETRY_ZONES`] — a different bound, so a different count.
static ZONES_OVERFLOWED: AtomicU64 = AtomicU64::new(0);
/// Files rotated.
static ROTATIONS: AtomicU64 = AtomicU64::new(0);
/// Windows abandoned because the encode did not fit the buffer.
///
/// **Abandoned whole, never truncated.** A block whose `len` disagreed with its contents is the one
/// thing the framing exists to make impossible, so a window that will not fit is not written at all.
static WINDOWS_DROPPED: AtomicU64 = AtomicU64::new(0);

/// The telemetry writer's own counters — the twelve-of-eighteen the store's `DropCounters` does not
/// carry.
///
/// **Process-global rather than fields of `Profiler::drops`**, and that is a correction to the
/// store's own prediction. `DropCounters` belongs to one `Profiler` in one `World`; this writer has
/// no `World` by construction (see the module docs), so counters living there would be unreachable
/// from the party that increments them. `boyko_diag::loss` already establishes process-global
/// counting as this workspace's answer for exactly that shape.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct TelemetryCounters {
    /// Windows written whole.
    pub windows: u64,
    /// Bytes handed to the sink.
    pub bytes: u64,
    /// Write failures.
    pub write_errors: u64,
    /// Quantile subscriptions refused past the cap.
    pub zones_refused: u64,
    /// Subscriptions refused past [`MAX_TELEMETRY_ZONES`].
    pub zones_overflowed: u64,
    /// Files rotated.
    pub rotations: u64,
    /// Windows abandoned because the encode did not fit.
    pub windows_dropped: u64,
}

/// Read every telemetry counter.
#[must_use]
pub fn counters() -> TelemetryCounters {
    TelemetryCounters {
        windows: WINDOWS_WRITTEN.load(Ordering::Relaxed),
        bytes: BYTES_WRITTEN.load(Ordering::Relaxed),
        write_errors: WRITE_ERRORS.load(Ordering::Relaxed),
        zones_refused: ZONES_REFUSED.load(Ordering::Relaxed),
        zones_overflowed: ZONES_OVERFLOWED.load(Ordering::Relaxed),
        rotations: ROTATIONS.load(Ordering::Relaxed),
        windows_dropped: WINDOWS_DROPPED.load(Ordering::Relaxed),
    }
}

/// Where a window's bytes go.
///
/// A trait with a generic parameter rather than a `dyn` or a `#[cfg(test)]` hook: `G15`'s
/// short-write clause needs a sink that returns after half a block — the `ENOSPC` shape — and a
/// gate that can only be written by putting test code in the shipped path is a gate that changes
/// what it measures.
pub trait TelemetrySink {
    /// Hand the window's bytes over. One call per window.
    ///
    /// # Errors
    ///
    /// Whatever the underlying sink reports. A short write is an error here: `write_all`'s own
    /// contract is all-or-error, and a sink that returned `Ok` after a partial write would be
    /// claiming the block reached the file.
    fn write_all(&mut self, buf: &[u8]) -> io::Result<()>;

    /// Start a new file, if this sink has files. Default: it does not, so rotation is a no-op and
    /// [`TelemetryStream`] keeps writing into the same place.
    ///
    /// # Errors
    ///
    /// Whatever opening the next file reports.
    fn rotate(&mut self) -> io::Result<bool> {
        Ok(false)
    }
}

/// The shipped sink: one file, opened at arm, rotated at `max_bytes`.
pub struct FileSink {
    /// `None` between the close and the re-open of a rotation, and after one fails.
    file: Option<File>,
    path: PathBuf,
}

impl FileSink {
    /// Open `path`, truncating.
    ///
    /// # Errors
    ///
    /// Whatever `OpenOptions::open` reports. The caller turns that into `W9214`.
    pub fn open(path: &Path) -> io::Result<FileSink> {
        let file = OpenOptions::new().write(true).create(true).truncate(true).open(path)?;
        Ok(FileSink { file: Some(file), path: path.to_path_buf() })
    }
}

impl TelemetrySink for FileSink {
    fn write_all(&mut self, buf: &[u8]) -> io::Result<()> {
        match self.file.as_mut() {
            Some(f) => f.write_all(buf),
            None => Err(io::Error::other("the telemetry file is closed")),
        }
    }

    /// Move the full file aside as `<path>.prev` and open a fresh one.
    ///
    /// One generation is kept rather than a numbered series: the point of rotation is a **bound on
    /// disk**, and an unbounded series of numbered files is not one.
    ///
    /// The handle is dropped **before** the rename and the fresh handle taken **after** it, in that
    /// order. Windows refuses to rename a file that is still open, so the Unix-friendly shortcut —
    /// rename first, then swap the handle — would work on one of this workspace's two stated target
    /// platforms and silently fail on the other.
    fn rotate(&mut self) -> io::Result<bool> {
        self.file = None;
        let mut prev = self.path.clone().into_os_string();
        prev.push(".prev");
        // A rename onto an existing file is refused on Windows; a missing one is not an error here.
        let _ = std::fs::remove_file(&prev);
        std::fs::rename(&self.path, &prev)?;
        self.file =
            Some(OpenOptions::new().write(true).create(true).truncate(true).open(&self.path)?);
        Ok(true)
    }
}

/// How a session streams.
pub struct TelemetryConfig<'a> {
    /// Where the file goes.
    pub path: &'a Path,
    /// Bytes a file may reach before rotation.
    pub max_bytes: u64,
    /// The parent-supplied run discriminant, or `0` for *"nobody declared one"*.
    pub run_id: u32,
    /// Sixteen opaque bytes the engine never interprets.
    pub player_tag: [u8; 16],
    /// The zones to stream, in the order their records will appear.
    ///
    /// **Caller contract: no duplicates.** A zone listed twice gets two subscription slots and
    /// therefore two records with the same id in every window, which a decoder has no rule for —
    /// sum them, or pick one? It is not de-duplicated here because the honest answer to "which of
    /// these two identical subscriptions did you mean" is that the caller made a mistake, and
    /// silently collapsing it would hide a configuration typo behind correct-looking output.
    pub zones: &'a [u16],
    /// The subset that also carries `median` and `p95`. Anything past
    /// [`MAX_TELEMETRY_QUANTILE_ZONES`] is refused.
    pub quantiles: &'a [u16],
}

/// One session's stream.
///
/// Fixed-size throughout: the subscription arrays, the record scratch and the quantile gather are
/// all inline, so a window costs no allocation and the struct's footprint is a compile-time
/// constant. Created once, at arm.
pub struct TelemetryStream<S: TelemetrySink = FileSink> {
    sink: Option<S>,
    live: bool,
    header: StreamHeader,
    header_written: bool,
    max_bytes: u64,
    bytes_in_file: u64,
    seq: u32,
    cur: usize,
    zones: [u16; MAX_TELEMETRY_ZONES],
    /// The quantile slot a zone owns, or [`NO_QUANTILE_SLOT`].
    qslot: [u16; MAX_TELEMETRY_ZONES],
    introduced: [bool; MAX_TELEMETRY_ZONES],
    zone_count: usize,
    recs: [WindowRec; MAX_TELEMETRY_ZONES],
    /// Per-frame values for the quantile zones, `slot * WINDOW + n`.
    gather: [u32; MAX_TELEMETRY_QUANTILE_ZONES * WINDOW],
    /// How many values each slot holds.
    gathered: [u16; MAX_TELEMETRY_QUANTILE_ZONES],
}

impl TelemetryStream<FileSink> {
    /// Open a stream for `cfg`, or return one that streams nothing.
    ///
    /// **The file is opened HERE, on the enable path**, never at process start: a run that does not
    /// arm with a telemetry config opens no file, writes no page of the double buffer and cannot
    /// raise `W9214`.
    #[cold]
    #[must_use]
    pub fn open(cfg: &TelemetryConfig<'_>, p: &Profiler) -> TelemetryStream<FileSink> {
        let sink = match FileSink::open(cfg.path) {
            Ok(s) => Some(s),
            Err(_) => {
                // The errno is the caller's to log with its own context; what the code says is the
                // consequence, which is that this session has no stream.
                raise(DiagFlag::TelemetryPathUnwritable);
                None
            }
        };
        TelemetryStream::with_sink(sink, cfg, p)
    }
}

impl<S: TelemetrySink> TelemetryStream<S> {
    /// A stream over an arbitrary sink. `None` streams nothing, which is the state `open` lands in
    /// when the path is unwritable.
    #[cold]
    #[must_use]
    pub fn with_sink(
        sink: Option<S>,
        cfg: &TelemetryConfig<'_>,
        p: &Profiler,
    ) -> TelemetryStream<S> {
        let mut s = TelemetryStream {
            live: sink.is_some(),
            sink,
            header: StreamHeader {
                schema_version: telemetry::TELEMETRY_SCHEMA_VERSION,
                session_lo: clock::session_id().0,
                session_hi: clock::session_id().1,
                run_id: cfg.run_id,
                clock_epoch: clock::clock_epoch(),
                player_tag: cfg.player_tag,
                ticks_per_ns: clock::ticks_per_ns(),
                zone_stride: p.zone_stride(),
                window: WINDOW as u32,
                region_capacity: profile::REGION_CAPACITY,
                flags: if clock::invariant_tsc() { HEADER_FLAG_INVARIANT_TSC } else { 0 },
                log_ceiling: profile::LOG_CEILING,
                profiling_tier: profile::PROFILING_TIER,
            },
            header_written: false,
            max_bytes: cfg.max_bytes,
            bytes_in_file: 0,
            seq: 0,
            cur: 0,
            zones: [0; MAX_TELEMETRY_ZONES],
            qslot: [NO_QUANTILE_SLOT; MAX_TELEMETRY_ZONES],
            introduced: [false; MAX_TELEMETRY_ZONES],
            zone_count: 0,
            recs: [WindowRec::default(); MAX_TELEMETRY_ZONES],
            gather: [0; MAX_TELEMETRY_QUANTILE_ZONES * WINDOW],
            gathered: [0; MAX_TELEMETRY_QUANTILE_ZONES],
        };
        s.subscribe(cfg);
        s
    }

    /// Take the config's two lists and turn them into the fixed arrays a window walks.
    ///
    /// Both bounds refuse rather than clamp, and they are counted separately because they mean
    /// different things: past [`MAX_TELEMETRY_ZONES`] a zone does not stream at all, past
    /// [`MAX_TELEMETRY_QUANTILE_ZONES`] it streams without its two quantiles.
    #[cold]
    fn subscribe(&mut self, cfg: &TelemetryConfig<'_>) {
        let mut quantiles_taken = 0usize;
        for &z in cfg.zones {
            if self.zone_count == MAX_TELEMETRY_ZONES {
                ZONES_OVERFLOWED.fetch_add(1, Ordering::Relaxed);
                continue;
            }
            let wants_quantiles = cfg.quantiles.contains(&z);
            let slot = if wants_quantiles {
                if quantiles_taken < MAX_TELEMETRY_QUANTILE_ZONES {
                    let s = quantiles_taken;
                    quantiles_taken += 1;
                    u16::try_from(s).unwrap_or(NO_QUANTILE_SLOT)
                } else {
                    ZONES_REFUSED.fetch_add(1, Ordering::Relaxed);
                    raise(DiagFlag::TelemetryZonesRefused);
                    NO_QUANTILE_SLOT
                }
            } else {
                NO_QUANTILE_SLOT
            };
            self.zones[self.zone_count] = z;
            self.qslot[self.zone_count] = slot;
            self.zone_count += 1;
        }
    }

    /// Zones this stream carries.
    #[must_use]
    pub fn zone_count(&self) -> usize {
        self.zone_count
    }

    /// Zones that carry quantiles.
    #[must_use]
    pub fn quantile_count(&self) -> usize {
        self.qslot[..self.zone_count].iter().filter(|s| **s != NO_QUANTILE_SLOT).count()
    }

    /// Whether the stream still has somewhere to write.
    #[must_use]
    pub fn is_live(&self) -> bool {
        self.sink.is_some() && self.live
    }

    /// The sink, for a reader that wants what was written.
    ///
    /// Present **even after a failure**: the handle is kept and only `live` is cleared, so the
    /// file's `close` happens when the host drops the stream rather than at an arbitrary moment
    /// inside a frame. A `close` is a syscall, and putting one on the dispatcher at the instant the
    /// disk is already in trouble is the shape this whole failure path exists to avoid.
    #[must_use]
    pub fn sink(&self) -> Option<&S> {
        self.sink.as_ref()
    }

    /// **`__telemetry_reduce`** — the window's numbers, out of the store's frame-major columns.
    ///
    /// Returns the per-window head. The per-zone records land in `self.recs[..zone_count]`.
    ///
    /// The live frame is excluded: `FrameState::Pending` is the row still accumulating, and a
    /// window that included it would report a frame nobody has finished measuring.
    #[cold]
    pub fn reduce(&mut self, p: &Profiler, fixed_elapsed_ns: u64) -> WindowHead {
        let _z = boyko_diag::zone!(TELEMETRY_REDUCE);

        let mut head = WindowHead {
            fixed_elapsed_ns,
            drops: 0,
            clock_epoch: p.epoch(),
            frame_first: u32::MAX,
            frame_last: 0,
            zone_rows: 0,
            recs: 0,
            flags: 0,
        };

        // Pass one: which rows are sealed, and what is true of the window as a whole. Once, here —
        // not once per zone, and not once per record. Sixty-four copies of one value in sixty-four
        // records would be sixty-four chances for them to disagree.
        let mut sealed = [false; WINDOW];
        let mut any = false;
        for (row, is_sealed) in sealed.iter_mut().enumerate() {
            let Some(fr) = p.frame_record(row as u32) else { continue };
            if fr.state != FrameState::Sealed {
                continue;
            }
            *is_sealed = true;
            any = true;
            head.drops = head.drops.saturating_add(fr.drops);
            head.frame_first = head.frame_first.min(fr.frame);
            head.frame_last = head.frame_last.max(fr.frame);
            if fr.flags & FRAME_FLAG_CLOCK_UNCALIBRATED != 0 {
                head.flags |= WINDOW_FLAG_CLOCK_UNCALIBRATED;
            }
        }
        if !any {
            // No sealed frame: `frame_first` stays `u32::MAX`, which is not a frame number. Say
            // zero frames rather than a range nothing occupies.
            head.frame_first = 0;
        }

        // Pass two: ROW-MAJOR, and the order is the whole of this rung's one perf decision.
        //
        // MEASURED on this box, release, at the 64-zone cap: walking zone-major — the strided
        // gather A10 describes, one zone across all 121 rows — costs a p95 of **168.1 us** against
        // a 150 us budget, of which 86.8 us is the walk itself. The columns are frame-major, so a
        // zone-major step advances by `zone_stride` elements; at stride 4096 that is 32 KiB per
        // step and every one of the 7 744 cell reads is its own cache line.
        //
        // Row-major touches each line once and takes sixteen `u64`s out of it. It costs one fixed
        // array: the quantile gather has to be per zone, so it becomes `64 x 121` u32 — 31 KiB,
        // written 4 KiB-live at a time and inline in this struct, so still no allocation.
        //
        // The figure after the change is in `G26`'s own output; the two are not restated here,
        // because a number in a comment and a number in a gate are two numbers.
        for i in 0..self.zone_count {
            self.recs[i] = WindowRec { id: self.zones[i], min: u32::MAX, ..WindowRec::default() };
        }
        self.gathered = [0; MAX_TELEMETRY_QUANTILE_ZONES];

        for (row, is_sealed) in sealed.iter().enumerate() {
            if !*is_sealed {
                continue;
            }
            for i in 0..self.zone_count {
                let Some(c) = p.cell(row as u32, self.zones[i]) else { continue };
                if c.count == 0 {
                    continue;
                }
                let rec = &mut self.recs[i];
                rec.total = rec.total.wrapping_add(c.total);
                rec.count = rec.count.saturating_add(c.count);
                rec.min = rec.min.min(c.min);
                rec.max = rec.max.max(c.max);
                if c.label == CellLabel::OverRange {
                    rec.flags |= REC_OVER_RANGE;
                }
                let slot = self.qslot[i];
                if slot != NO_QUANTILE_SLOT {
                    // The per-FRAME total, not the per-sample value: a reader asking how long a
                    // zone takes in a frame is asking about frames, and a per-sample quantile over
                    // a zone that runs a hundred times per frame answers a different question.
                    let n = usize::from(self.gathered[slot as usize]);
                    self.gather[slot as usize * WINDOW + n] =
                        u32::try_from(c.total).unwrap_or(u32::MAX);
                    self.gathered[slot as usize] = self.gathered[slot as usize].saturating_add(1);
                }
            }
        }

        // Pass three: the sorts, one per quantile zone, over data that is already compact.
        for i in 0..self.zone_count {
            if self.recs[i].count == 0 {
                // Nothing ran. `min` must not be reported as `u32::MAX`, which is a value.
                self.recs[i].min = 0;
                continue;
            }
            let slot = self.qslot[i];
            if slot == NO_QUANTILE_SLOT {
                continue;
            }
            let n = usize::from(self.gathered[slot as usize]);
            if n == 0 {
                continue;
            }
            let base = slot as usize * WINDOW;
            let g = &mut self.gather[base..base + n];
            g.sort_unstable();
            // `ceil`, matching the retention tier's own convention: a p95 must not land below the
            // 95th percentile by a rounding rule nobody stated.
            self.recs[i].median = g[quantile_index(n, 0.50)];
            self.recs[i].p95 = g[quantile_index(n, 0.95)];
            self.recs[i].flags |= REC_HAS_QUANTILES;
        }
        head.recs = u16::try_from(self.zone_count).unwrap_or(u16::MAX);
        head
    }

    /// **`__telemetry_write`** — encode the window into the `.bss` buffer and hand it over in one
    /// call.
    ///
    /// Returns whether the window reached the sink. `false` covers three states, each counted
    /// separately: no sink, an encode that did not fit, and a failed write.
    #[cold]
    pub fn write(&mut self, p: &Profiler, head: &WindowHead) -> bool {
        let _z = boyko_diag::zone!(TELEMETRY_WRITE);
        if !self.is_live() {
            return false;
        }

        // Rotate BEFORE encoding, so a window is never split across two files.
        if self.max_bytes > 0 && self.bytes_in_file >= self.max_bytes {
            self.rotate();
        }

        let side = self.cur;
        // SAFETY: `side < 2`, and this crate's obligation is the dispatcher rather than the lane
        //   topology `SyncCells` documents: `write` runs on one thread, outside the schedule, and
        //   the pointer does not outlive this call. The two sides alternate, so the bytes a reader
        //   of the previous window could still hold are never the ones being written here.
        let buf: &mut [u8; STREAM_BUFFER_BYTES] = unsafe { &mut *BUFFERS.get_ptr(side) };
        let mut e = Encoder::new(buf);

        let mut head = *head;
        let mut rows = 0u16;
        for i in 0..self.zone_count {
            if !self.introduced[i] {
                rows += 1;
            }
        }
        head.zone_rows = rows;

        let ok = self.encode_window(&mut e, p, &head);
        if !ok {
            // Abandoned WHOLE. A partial block is the one thing the framing exists to prevent, and
            // `introduced` is deliberately not updated — the rows retry in the next window.
            WINDOWS_DROPPED.fetch_add(1, Ordering::Relaxed);
            return false;
        }

        let n = e.len();
        let sink = self.sink.as_mut().expect("invariant: the sink was checked at entry");
        // SAFETY-free: `bytes()` borrows the encoder, which borrows the buffer; the sink call is
        // the last use of both.
        let written = sink.write_all(e.bytes());
        match written {
            Ok(()) => {
                self.header_written = true;
                for i in 0..self.zone_count {
                    self.introduced[i] = true;
                }
                self.seq = self.seq.wrapping_add(1);
                self.cur ^= 1;
                self.bytes_in_file += n as u64;
                WINDOWS_WRITTEN.fetch_add(1, Ordering::Relaxed);
                BYTES_WRITTEN.fetch_add(n as u64, Ordering::Relaxed);
                true
            }
            Err(_) => {
                self.live = false;
                WRITE_ERRORS.fetch_add(1, Ordering::Relaxed);
                raise(DiagFlag::TelemetryWriteFailed);
                false
            }
        }
    }

    /// Encode one window into `e`. Returns `false` if any part did not fit, having written whatever
    /// it managed — which the caller discards, because `e` is never handed on in that case.
    fn encode_window(&self, e: &mut Encoder<'_>, p: &Profiler, head: &WindowHead) -> bool {
        if !self.header_written && !e.header(&self.header) {
            return false;
        }
        let Some(at) = e.begin_block() else { return false };
        if !e.window_head(head) {
            return false;
        }
        for i in 0..self.zone_count {
            if self.introduced[i] {
                continue;
            }
            let zone = self.zones[i];
            let desc = boyko_diag::profiling_abi::zone_desc(zone);
            let row = ZoneRow {
                id: zone,
                // OBSERVED, not declared: a `ZoneDesc` carries no kind, so the only party that can
                // state whether `total` is ticks or increments is the fold.
                kind: p.observed_kind(zone).map_or(telemetry::KIND_UNKNOWN, telemetry::kind_byte),
                scope: desc.map_or(0, |d| u8::try_from(d.scope).unwrap_or(0)),
                tier: desc.map_or(0, |d| d.tier as u8),
                region: desc.map_or(0, |d| d.region as u8),
                // A zone with no descriptor is one the registry never minted. Its name is EMPTY
                // rather than invented: the id is still the record's key, and a decoder printing a
                // blank name knows more than one printing a plausible one.
                name: desc.map_or(&b""[..], |d| d.name.as_bytes()),
            };
            if !e.zone_row(&row) {
                return false;
            }
        }
        for rec in &self.recs[..self.zone_count] {
            if !e.window_rec(rec) {
                return false;
            }
        }
        e.end_block(at, self.seq)
    }

    /// Start a new file: the header and every zone row are owed again.
    #[cold]
    fn rotate(&mut self) {
        let Some(sink) = self.sink.as_mut() else { return };
        // `rotate` is only reached from `write`, which returned early unless `is_live()`.
        match sink.rotate() {
            Ok(true) => {
                self.bytes_in_file = 0;
                self.header_written = false;
                self.introduced = [false; MAX_TELEMETRY_ZONES];
                ROTATIONS.fetch_add(1, Ordering::Relaxed);
            }
            // A sink with no files: nothing to rotate, and `bytes_in_file` keeps climbing, which is
            // the honest state for a sink that has no bound to enforce.
            Ok(false) => {}
            Err(_) => {
                self.live = false;
                WRITE_ERRORS.fetch_add(1, Ordering::Relaxed);
                raise(DiagFlag::TelemetryWriteFailed);
            }
        }
    }
}

/// The index of the `q` quantile among `n` sorted values, 1-based rank, `ceil`.
///
/// Shared by the median and the p95 so the two cannot drift apart in a way nobody notices.
fn quantile_index(n: usize, q: f64) -> usize {
    debug_assert!(n > 0, "invariant: a quantile needs at least one sample");
    let rank = (q * n as f64).ceil().max(1.0) as usize;
    rank.min(n) - 1
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The quantile index is `ceil`-ranked and never runs off the end.
    #[test]
    fn the_quantile_index_is_ceil_ranked_and_bounded() {
        assert_eq!(quantile_index(1, 0.95), 0);
        assert_eq!(quantile_index(100, 0.50), 49);
        assert_eq!(quantile_index(100, 0.95), 94);
        assert_eq!(quantile_index(121, 0.95), 114);
        assert_eq!(quantile_index(3, 1.0), 2, "the top quantile is the last element, not past it");
        assert_eq!(quantile_index(3, 0.0), 0, "and the bottom is the first, not index -1");
    }

    /// The buffer is big enough for the ceiling this module states, so the "steady state fits"
    /// claim in the module docs is arithmetic rather than a hope.
    #[test]
    fn the_buffer_holds_a_full_steady_state_window() {
        let steady = telemetry::BLOCK_HEADER_BYTES
            + telemetry::WINDOW_HEAD_BYTES
            + MAX_TELEMETRY_ZONES * telemetry::WINDOW_REC_BYTES;
        assert!(
            steady <= STREAM_BUFFER_BYTES,
            "a steady-state window at the zone ceiling is {steady} B and the buffer is \
             {STREAM_BUFFER_BYTES} B"
        );
    }
}
