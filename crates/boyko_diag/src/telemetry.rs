//! Profiling rung 13 — the player-telemetry **wire format**: an append-only, self-delimiting
//! binary stream, and the codec that is its only definition.
//!
//! # Why the format is HOSTED here rather than beside its writer
//!
//! [`crate`]'s growth rule (`docs/diagnostics/substrate/00-GOAL.md` §4) admits a *shared* primitive
//! only when both subsystems write it. This is not shared — the profiler writes it and one tool
//! reads it — so it enters by §5's other door, the one `profiling_abi` came through: **hosted, for
//! a graph reason.**
//!
//! The graph reason, MEASURED (`cargo tree --edges normal --prefix none | sort -u | wc -l`, this
//! tree, today). The figure counts the decoder crate itself, because that is what the command
//! prints and one quantity gets one number:
//!
//! | A `prof_decode` rooted at | Crates in its tree |
//! |---|---|
//! | `boyko_diag` | **2** |
//! | `boyko_ecs` | 12 |
//! | `boyko_app` | 45 |
//!
//! The corpus lists the format's writer as `crates/boyko_app/src/profiling/stream.rs`, and the
//! writer stays there — it needs a `File`, a `.bss` double buffer and the host's frame loop. But
//! the **format** is what the decoder needs, and rooting a file-reading CLI at `boyko_app` makes it
//! build the Vulkan FFI, every shader and the whole render stack in order to print a table. It also
//! makes the decoder inherit that stack's build state; this tree currently has a `boyko_app` feature
//! leg that does not compile, which would take the decoder down with it for a reason having nothing
//! to do with decoding.
//!
//! Nothing here reads a file, opens a handle or prints. Encoding is into a caller-supplied `&mut
//! [u8]`; decoding is over a `&[u8]`. `std::fs` and stdout stay outside this crate, exactly as the
//! mute-leaf rule requires.
//!
//! # Explicit little-endian, not a `#[repr(C)]` blit
//!
//! Every field is written with `to_le_bytes` and read with `from_le_bytes`. A blit would make the
//! file's byte order the *writing host's*, so a stream captured on one machine and decoded on
//! another would silently mean different numbers — and a telemetry file exists precisely to be read
//! somewhere other than where it was written. It also removes every unaligned-read `unsafe` from
//! the decoder, which is walking bytes that arrived from a disk and may be arbitrary.
//!
//! The cost is a few hundred nanoseconds once per two-second window.
//!
//! # Framing: one block per window, one `write_all`, one possible tear
//!
//! ```text
//! file  := header(128 B) block*
//! block := BlockHeader(16 B: magic, len, seq, crc32) WindowHead(32 B) ZoneRow* WindowRec(32 B)*
//! ```
//!
//! `write_all` on `ENOSPC` returns **after a partial write**, so an unframed stream ends
//! mid-record and a decoder cannot tell a torn tail from data. [`decode`] walks blocks and stops at
//! the first one whose magic is wrong, whose `len` exceeds the bytes remaining, or whose CRC
//! mismatches; the bytes from that point on are reported as
//! [`Decoded::truncated_tail_bytes`] and **no record from the torn block is returned**.
//!
//! Framing costs 16 B per two-second window. Per-*record* framing was rejected at 8 B on a 32 B
//! record — 25 % — when there is exactly one `write_all` per window and therefore exactly one
//! possible tear point.
//!
//! # Four corrections to the format the corpus specified, each measured against this tree
//!
//! **1. The per-window values are written ONCE, in a [`WindowHead`], not repeated in every
//! record.** D23 puts `clock_epoch` and `fixed_elapsed_ns` in the `WindowRec`. Both are properties
//! of the *window*: at 64 subscribed zones that is 64 identical copies of 12 bytes — 768 B of a
//! 2560 B payload, **30 %** — and, worse, 64 chances to disagree. A10's own pseudocode reads
//! `FixedTime::elapsed()` *inside* the record loop, where successive reads legitimately differ, and
//! a file whose 64 records claim 64 different window times has no rule saying which is the window's.
//! One head, one value, and the payload drops from 2560 B to 2080 B at 64 zones.
//!
//! **2. `WindowRec::drops` had no source, so it became the head's.** Nothing in this tree counts
//! drops *per zone*: [`crate::loss`] is process-wide and `FrameRecord::drops` is per frame. The
//! honest per-window figure is Σ over the window's frames, which is one number for the window — so
//! it is a [`WindowHead`] field. A per-zone field would have been filled with a per-frame number
//! divided by nothing.
//!
//! **3. `build_hash` and `calib_cv` are ABSENT, on rung 7's precedent and its exact reasoning.**
//! `BUILD_HASH` appears nowhere in the workspace and `crates/boyko_diag/build.rs` does not exist;
//! there is no calibration spread anywhere in [`crate::clock`], which publishes
//! [`ticks_per_ns`](crate::clock::ticks_per_ns), [`clock_epoch`](crate::clock::clock_epoch),
//! [`invariant_tsc`](crate::clock::invariant_tsc) and [`session_id`](crate::clock::session_id) and
//! nothing else. *"A header field that is always absent is indistinguishable from one that is
//! broken"* — `boyko_app::profiling::artifact`'s Decision 4, measured there, applies unchanged.
//! What ships in their place is the thing that does exist and answers the same question:
//! [`HEADER_FLAG_INVARIANT_TSC`], which is what decides whether the tick magnitudes mean anything.
//!
//! **4. `build_profile` is three integers rather than a name, because the name does not exist.**
//! [`crate::profile`]'s own rule — *"only the constants a landed rung actually reads are
//! declared"* — has kept `BOYKO_PROFILE` itself out of the tree: there is no build script and no
//! profile-name constant, only [`LOG_CEILING`](crate::profile::LOG_CEILING),
//! [`PROFILING_TIER`](crate::profile::PROFILING_TIER) and
//! [`REGION_CAPACITY`](crate::profile::REGION_CAPACITY). Those three ARE the build profile,
//! materially, and they are what the header carries. Rung 14 is the rung that lands the axis; a
//! `build_profile` byte can join them there, when there is something for it to be wrong about.
//!
//! # `NO_QUANTILE` is a FLAG, not a sentinel value
//!
//! A `WindowRec` outside the quantile subscription must not carry a zero a reader could read as a
//! measurement — D23 says so and calls for an explicit format value. `u32::MAX` cannot be that
//! value: the store clamps a cell's extrema to `u32::MAX` and labels it `OverRange`, so it is
//! **reachable**, and using it would make *"nobody subscribed"* indistinguishable from *"this zone
//! ran longer than a `u32` of ticks"*. [`REC_HAS_QUANTILES`] in [`WindowRec::flags`] is the value,
//! and it can never collide with one.
//!
//! # The CRC is this module's own, and the duplication is stated rather than hidden
//!
//! `boyko_image::png` has a private CRC-32 table for PNG chunks. It cannot be reached from here —
//! this crate must keep an empty `[dependencies]` — and hoisting a general-purpose checksum *into*
//! the substrate crate is the accretion its own header comment forbids. So there are two 1 KiB
//! `.rodata` tables in this workspace, both IEEE 802.3, and that is a named cost rather than an
//! oversight. It is recorded in `docs/OPEN-QUESTIONS.md`.

use crate::sample::SampleKind;

/// File magic, first four bytes: `b"BKTS"` read little-endian.
pub const STREAM_MAGIC: u32 = u32::from_le_bytes(*b"BKTS");

/// Block magic, repeated at every block header: `b"BBLK"` read little-endian.
///
/// A distinct constant from [`STREAM_MAGIC`] deliberately: a decoder that lost sync and landed on
/// the file header would otherwise accept it as a block.
pub const BLOCK_MAGIC: u32 = u32::from_le_bytes(*b"BBLK");

/// Format version. A decoder refuses anything else **before parsing a block**.
pub const TELEMETRY_SCHEMA_VERSION: u32 = 1;

/// Bytes in the file header, written once per file.
pub const HEADER_BYTES: usize = 128;

/// Bytes in a block header: `magic`, `len`, `seq`, `crc32`.
pub const BLOCK_HEADER_BYTES: usize = 16;

/// Bytes in the per-window preamble that opens every block's payload.
pub const WINDOW_HEAD_BYTES: usize = 32;

/// Bytes in one [`WindowRec`].
pub const WINDOW_REC_BYTES: usize = 32;

/// Bytes in a [`ZoneRow`]'s fixed part, before its name.
pub const ZONE_ROW_FIXED_BYTES: usize = 8;

/// Longest zone name a row can carry. Names are truncated, never dropped: a row with a shortened
/// name still resolves an id, and a missing row leaves a number with no label at all.
pub const ZONE_NAME_MAX: usize = 255;

/// [`StreamHeader::flags`] — the CPU advertises an invariant TSC, so tick magnitudes are
/// comparable across cores and frequency states. Its absence is what `W9207` reports.
pub const HEADER_FLAG_INVARIANT_TSC: u32 = 1 << 0;

/// [`WindowHead::flags`] — at least one frame in this window was recorded on an uncalibrated
/// clock, so `ticks_per_ns` in the file header does not convert this window's numbers.
pub const WINDOW_FLAG_CLOCK_UNCALIBRATED: u32 = 1 << 0;

/// [`WindowRec::flags`] — `median` and `p95` are measurements. Clear means this zone was outside
/// the quantile subscription and both fields are **absent**, not zero.
pub const REC_HAS_QUANTILES: u16 = 1 << 0;

/// [`WindowRec::flags`] — at least one cell in the window was clamped, so `max` (and possibly the
/// quantiles) is a floor rather than a value.
pub const REC_OVER_RANGE: u16 = 1 << 1;

/// A [`ZoneRow::kind`] that was never observed.
///
/// **`0`, and the shift below it is the whole point.** `SampleKind::Span` is discriminant `0`, so a
/// raw cast would make "this zone emitted spans" and "nothing was ever folded for this zone" the
/// same byte — in a file whose reader has no other way to learn the unit of `total`.
pub const KIND_UNKNOWN: u8 = 0;

/// The wire byte for a [`SampleKind`]: its discriminant **plus one**, so [`KIND_UNKNOWN`] can be
/// zero.
#[must_use]
pub const fn kind_byte(kind: SampleKind) -> u8 {
    kind as u8 + 1
}

/// The [`SampleKind`] a wire byte encodes, or `None` for [`KIND_UNKNOWN`] and for anything a later
/// schema might add.
#[must_use]
pub const fn kind_of_byte(b: u8) -> Option<SampleKind> {
    match b {
        1 => Some(SampleKind::Span),
        2 => Some(SampleKind::Counter),
        3 => Some(SampleKind::Gauge),
        _ => None,
    }
}

/// The file header — written once, at the first window, never rewritten.
///
/// Session identity is 36 B of it: [`session_lo`](Self::session_lo) +
/// [`session_hi`](Self::session_hi) (16), [`run_id`](Self::run_id) (4) and
/// [`player_tag`](Self::player_tag) (16). D26 costed it at 44 with an 8-byte `build_hash`; that
/// field does not exist in this tree and is not invented here — see the module docs.
// `PartialEq` without `Eq`: `ticks_per_ns` is an `f64`. The round-trip property is asserted over
// BYTES rather than over this struct precisely because a float comparison is not the claim being
// made — two headers that encode to the same 128 bytes are the same header.
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct StreamHeader {
    /// [`TELEMETRY_SCHEMA_VERSION`] at write time.
    pub schema_version: u32,
    /// `boyko_diag::clock::session_id`'s low half. Shared with the logger's artifact header, which
    /// is what lets the two files join (S11).
    pub session_lo: u64,
    /// Its high half.
    pub session_hi: u64,
    /// The parent-supplied run discriminant, or `0` for *"nobody declared one"*.
    ///
    /// The same shape as the artifact's `run_token` and for the same measured reason: a `SessionId`
    /// is minted inside the child, so a parent cannot predict it and cannot use it to detect a
    /// stale read within one run.
    pub run_id: u32,
    /// The clock epoch when the file was opened. A window whose head disagrees with this one is on
    /// the far side of a suspend.
    pub clock_epoch: u32,
    /// Sixteen opaque bytes the engine never interprets. Zero when the game supplied none.
    pub player_tag: [u8; 16],
    /// `boyko_diag::clock::ticks_per_ns` at open. `1.0` means the clock was never calibrated, which
    /// a window also states per-window through [`WINDOW_FLAG_CLOCK_UNCALIBRATED`].
    pub ticks_per_ns: f64,
    /// The armed zone stride — the width of the store's frame-major columns.
    pub zone_stride: u32,
    /// The armed retained window, in frames.
    pub window: u32,
    /// `boyko_diag::profile::REGION_CAPACITY` — samples one lane region holds before it refuses.
    pub region_capacity: u32,
    /// [`HEADER_FLAG_INVARIANT_TSC`] and nothing else, at this rung.
    pub flags: u32,
    /// `boyko_diag::profile::LOG_CEILING`.
    pub log_ceiling: u8,
    /// `boyko_diag::profile::PROFILING_TIER`.
    pub profiling_tier: u8,
}

/// The per-window preamble: everything true of the whole window, stated once.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct WindowHead {
    /// `FixedTime::elapsed()` — the kernel's determinism witness, so a stream correlates with a
    /// replay (X18). Read **once** for the window; see the module docs on why it is not per-record.
    pub fixed_elapsed_ns: u64,
    /// Σ of every retained frame's `drops` across this window.
    pub drops: u32,
    /// The clock epoch this window was recorded in.
    pub clock_epoch: u32,
    /// The first absolute frame number the window covers.
    pub frame_first: u32,
    /// The last.
    pub frame_last: u32,
    /// [`ZoneRow`]s that follow, before the records.
    pub zone_rows: u16,
    /// [`WindowRec`]s that follow.
    pub recs: u16,
    /// [`WINDOW_FLAG_CLOCK_UNCALIBRATED`] and nothing else, at this rung.
    pub flags: u32,
}

/// A zone's identity, written once per zone per file.
///
/// Borrowed rather than owned: on the encode side the name is a `&'static str` from the registry,
/// and on the decode side it is a slice of the file's own bytes. Bytes rather than `str` so the
/// codec is an exact inverse even for a name that is not valid UTF-8 — the round-trip property
/// `G15` asserts must not depend on the names having been well formed.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ZoneRow<'a> {
    /// The registry zone id these records are keyed by.
    pub id: u16,
    /// [`kind_byte`] of the kind the fold OBSERVED for this zone, or [`KIND_UNKNOWN`].
    ///
    /// Observed rather than declared, because a `ZoneDesc` in this tree carries `name`, `scope`,
    /// `tier` and `region` and **no kind at all** — the kind is a property of the sample. Without
    /// it a reader cannot tell whether `total` is ticks or increments.
    pub kind: u8,
    /// The scope bit index that arms the zone (`0..64`).
    pub scope: u8,
    /// `ZoneTier` as a raw discriminant.
    pub tier: u8,
    /// `Region` as a raw discriminant: engine or user.
    pub region: u8,
    /// The printed name, at most [`ZONE_NAME_MAX`] bytes.
    pub name: &'a [u8],
}

impl ZoneRow<'_> {
    /// Bytes this row occupies on the wire.
    #[must_use]
    pub fn encoded_len(&self) -> usize {
        ZONE_ROW_FIXED_BYTES + self.name.len().min(ZONE_NAME_MAX)
    }
}

/// One zone's window, 32 B.
///
/// `median` and `p95` are quantiles **over the window's per-frame values**, not over individual
/// samples: a reader asking "how long does this zone take in a frame" is asking about frames, and
/// a per-sample quantile over a zone that runs a hundred times per frame answers a different
/// question. They are present only when [`REC_HAS_QUANTILES`] is set.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct WindowRec {
    /// Σ of the window's per-frame totals. Ticks for a span, increments for a counter, the sum of
    /// the sampled levels for a gauge.
    pub total: u64,
    /// Samples folded into this zone across the window.
    pub count: u32,
    /// Smallest per-sample value seen in the window.
    pub min: u32,
    /// Largest.
    pub max: u32,
    /// Median of the per-frame totals, or absent — see [`REC_HAS_QUANTILES`].
    pub median: u32,
    /// p95 of the per-frame totals, or absent.
    pub p95: u32,
    /// The registry zone id.
    pub id: u16,
    /// [`REC_HAS_QUANTILES`] | [`REC_OVER_RANGE`].
    pub flags: u16,
}

impl WindowRec {
    /// The quantile pair, or `None` when this zone was outside the subscription.
    #[must_use]
    pub fn quantiles(&self) -> Option<(u32, u32)> {
        if self.flags & REC_HAS_QUANTILES == 0 { None } else { Some((self.median, self.p95)) }
    }
}

// ---------------------------------------------------------------------------------------------
// CRC-32 (IEEE 802.3), table-driven.
// ---------------------------------------------------------------------------------------------

/// The reversed IEEE 802.3 polynomial.
const CRC32_POLY: u32 = 0xEDB8_8320;

/// One kibibyte of `.rodata`, folded at compile time.
///
/// Table-driven rather than bitwise because the alternative is eight iterations per byte over a
/// ~2 KB payload once per window — sixteen thousand iterations inside a 200 µs budget, spent to
/// save a kibibyte that is never paged in on a run that writes no telemetry.
const CRC32_TABLE: [u32; 256] = {
    let mut table = [0u32; 256];
    let mut i = 0usize;
    while i < 256 {
        let mut c = i as u32;
        let mut k = 0;
        while k < 8 {
            c = if c & 1 != 0 { CRC32_POLY ^ (c >> 1) } else { c >> 1 };
            k += 1;
        }
        table[i] = c;
        i += 1;
    }
    table
};

/// CRC-32 of `bytes`, IEEE 802.3.
#[must_use]
pub fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &b in bytes {
        crc = CRC32_TABLE[((crc ^ u32::from(b)) & 0xFF) as usize] ^ (crc >> 8);
    }
    crc ^ 0xFFFF_FFFF
}

// ---------------------------------------------------------------------------------------------
// Encoding.
// ---------------------------------------------------------------------------------------------

/// Writes the format into a caller-supplied buffer.
///
/// Every method returns `false` when the buffer is too small and **writes nothing** in that case,
/// so a caller that ignores one return does not produce a half-written record. The writer asserts
/// on those returns rather than ignoring them: a fixed `.bss` buffer that silently truncated would
/// produce a block whose `len` disagreed with its contents, which is the one thing the framing
/// exists to make impossible.
pub struct Encoder<'a> {
    buf: &'a mut [u8],
    len: usize,
}

impl<'a> Encoder<'a> {
    /// An encoder over `buf`, starting empty.
    #[must_use]
    pub fn new(buf: &'a mut [u8]) -> Encoder<'a> {
        Encoder { buf, len: 0 }
    }

    /// Bytes written so far.
    #[must_use]
    pub fn len(&self) -> usize {
        self.len
    }

    /// Whether nothing has been written.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// What has been written.
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.buf[..self.len]
    }

    /// Reset to empty, keeping the buffer. The bytes are left as they were: the next encode
    /// overwrites exactly what it writes, and [`bytes`](Self::bytes) never exposes past `len`.
    pub fn reset(&mut self) {
        self.len = 0;
    }

    /// Append `src`, or refuse and write nothing.
    fn put(&mut self, src: &[u8]) -> bool {
        let Some(end) = self.len.checked_add(src.len()) else { return false };
        if end > self.buf.len() {
            return false;
        }
        self.buf[self.len..end].copy_from_slice(src);
        self.len = end;
        true
    }

    /// Append `n` zero bytes.
    fn put_zeros(&mut self, n: usize) -> bool {
        let Some(end) = self.len.checked_add(n) else { return false };
        if end > self.buf.len() {
            return false;
        }
        self.buf[self.len..end].fill(0);
        self.len = end;
        true
    }

    /// Write the file header. Exactly [`HEADER_BYTES`], zero-padded.
    pub fn header(&mut self, h: &StreamHeader) -> bool {
        if self.len + HEADER_BYTES > self.buf.len() {
            return false;
        }
        let start = self.len;
        self.put(&STREAM_MAGIC.to_le_bytes());
        self.put(&h.schema_version.to_le_bytes());
        self.put(&h.session_lo.to_le_bytes());
        self.put(&h.session_hi.to_le_bytes());
        self.put(&h.run_id.to_le_bytes());
        self.put(&h.clock_epoch.to_le_bytes());
        self.put(&h.player_tag);
        self.put(&h.ticks_per_ns.to_le_bytes());
        self.put(&h.zone_stride.to_le_bytes());
        self.put(&h.window.to_le_bytes());
        self.put(&h.region_capacity.to_le_bytes());
        self.put(&h.flags.to_le_bytes());
        self.put(&[h.log_ceiling, h.profiling_tier]);
        // The reserved tail is zero-filled rather than left as whatever the double buffer held, so
        // two files written by two runs of the same build differ only where they measured
        // differently — and so the round-trip property holds byte for byte.
        self.put_zeros(HEADER_BYTES - (self.len - start));
        debug_assert_eq!(self.len - start, HEADER_BYTES, "invariant: the header is a fixed width");
        true
    }

    /// Reserve a block header and return its offset, for [`end_block`](Self::end_block).
    #[must_use]
    pub fn begin_block(&mut self) -> Option<usize> {
        let at = self.len;
        if self.put_zeros(BLOCK_HEADER_BYTES) { Some(at) } else { None }
    }

    /// Write the per-window preamble. Exactly [`WINDOW_HEAD_BYTES`].
    pub fn window_head(&mut self, w: &WindowHead) -> bool {
        if self.len + WINDOW_HEAD_BYTES > self.buf.len() {
            return false;
        }
        self.put(&w.fixed_elapsed_ns.to_le_bytes());
        self.put(&w.drops.to_le_bytes());
        self.put(&w.clock_epoch.to_le_bytes());
        self.put(&w.frame_first.to_le_bytes());
        self.put(&w.frame_last.to_le_bytes());
        self.put(&w.zone_rows.to_le_bytes());
        self.put(&w.recs.to_le_bytes());
        self.put(&w.flags.to_le_bytes());
        true
    }

    /// Write one zone row, truncating an over-long name to [`ZONE_NAME_MAX`].
    pub fn zone_row(&mut self, r: &ZoneRow<'_>) -> bool {
        let name = &r.name[..r.name.len().min(ZONE_NAME_MAX)];
        if self.len + ZONE_ROW_FIXED_BYTES + name.len() > self.buf.len() {
            return false;
        }
        self.put(&r.id.to_le_bytes());
        self.put(&[r.kind, r.scope, r.tier, r.region, name.len() as u8, 0]);
        self.put(name);
        true
    }

    /// Write one record. Exactly [`WINDOW_REC_BYTES`].
    pub fn window_rec(&mut self, r: &WindowRec) -> bool {
        if self.len + WINDOW_REC_BYTES > self.buf.len() {
            return false;
        }
        self.put(&r.total.to_le_bytes());
        self.put(&r.count.to_le_bytes());
        self.put(&r.min.to_le_bytes());
        self.put(&r.max.to_le_bytes());
        self.put(&r.median.to_le_bytes());
        self.put(&r.p95.to_le_bytes());
        self.put(&r.id.to_le_bytes());
        self.put(&r.flags.to_le_bytes());
        true
    }

    /// Close the block opened at `at`: fill in `magic`, the payload length, `seq` and the CRC.
    ///
    /// Returns `false` if `at` is not a block this encoder opened, which is a caller bug rather
    /// than a runtime condition.
    pub fn end_block(&mut self, at: usize, seq: u32) -> bool {
        if at + BLOCK_HEADER_BYTES > self.len {
            return false;
        }
        let payload = self.len - at - BLOCK_HEADER_BYTES;
        let Ok(payload_u32) = u32::try_from(payload) else { return false };
        let crc = crc32(&self.buf[at + BLOCK_HEADER_BYTES..self.len]);
        self.buf[at..at + 4].copy_from_slice(&BLOCK_MAGIC.to_le_bytes());
        self.buf[at + 4..at + 8].copy_from_slice(&payload_u32.to_le_bytes());
        self.buf[at + 8..at + 12].copy_from_slice(&seq.to_le_bytes());
        self.buf[at + 12..at + 16].copy_from_slice(&crc.to_le_bytes());
        true
    }
}

// ---------------------------------------------------------------------------------------------
// Decoding.
// ---------------------------------------------------------------------------------------------

/// Why a file could not be opened at all. A *block* failure is not one of these — it terminates
/// the walk and is reported as a truncated tail, which is a decoded file with a stated limit
/// rather than an error.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DecodeError {
    /// Fewer than [`HEADER_BYTES`] bytes: there is no header to disagree with.
    TooShort {
        /// What was there.
        len: usize,
    },
    /// The first four bytes are not [`STREAM_MAGIC`].
    NotAStream {
        /// What they were.
        magic: u32,
    },
    /// The header parses but names another format version. Refused **before** any block is read,
    /// because a block layout is exactly what a version changes.
    Schema {
        /// The version the file claims.
        found: u32,
    },
}

/// One decoded block.
#[derive(Clone, Debug)]
pub struct Block<'a> {
    /// The writer's window sequence number, monotone across the **session** — not restarted by a
    /// rotation.
    ///
    /// So a rotated file's first block does not begin at 0, deliberately: it is what lets a reader
    /// order two generations of the same stream, which a per-file counter could not do.
    pub seq: u32,
    /// The per-window preamble.
    pub head: WindowHead,
    /// Zone identities introduced by this block.
    pub zone_rows: Vec<ZoneRow<'a>>,
    /// The window's records.
    pub recs: Vec<WindowRec>,
}

/// A decoded file, with what could NOT be decoded stated rather than dropped.
#[derive(Clone, Debug)]
pub struct Decoded<'a> {
    /// The file header.
    pub header: StreamHeader,
    /// Every block that parsed whole, in file order.
    pub blocks: Vec<Block<'a>>,
    /// Records returned across every block.
    pub records_ok: u64,
    /// Bytes after the last whole block, whatever they contain.
    ///
    /// Non-zero means the walk stopped: a bad magic, a `len` past the end of the file, or a CRC
    /// mismatch. **The records inside those bytes are not returned** — a torn block is detected,
    /// never partially believed.
    pub truncated_tail_bytes: usize,
}

impl Decoded<'_> {
    /// Blocks that parsed whole.
    #[must_use]
    pub fn blocks_ok(&self) -> u64 {
        self.blocks.len() as u64
    }
}

/// Read four little-endian bytes at `off`, or `None` past the end.
fn u32_at(b: &[u8], off: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(off..off + 4)?.try_into().ok()?))
}

/// Read eight little-endian bytes at `off`.
fn u64_at(b: &[u8], off: usize) -> Option<u64> {
    Some(u64::from_le_bytes(b.get(off..off + 8)?.try_into().ok()?))
}

/// Read two little-endian bytes at `off`.
fn u16_at(b: &[u8], off: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(off..off + 2)?.try_into().ok()?))
}

/// Parse the file header.
fn decode_header(b: &[u8]) -> Result<StreamHeader, DecodeError> {
    if b.len() < HEADER_BYTES {
        return Err(DecodeError::TooShort { len: b.len() });
    }
    let magic = u32_at(b, 0).unwrap_or(0);
    if magic != STREAM_MAGIC {
        return Err(DecodeError::NotAStream { magic });
    }
    let schema_version = u32_at(b, 4).unwrap_or(0);
    if schema_version != TELEMETRY_SCHEMA_VERSION {
        return Err(DecodeError::Schema { found: schema_version });
    }
    let mut player_tag = [0u8; 16];
    player_tag.copy_from_slice(&b[32..48]);
    Ok(StreamHeader {
        schema_version,
        session_lo: u64_at(b, 8).unwrap_or(0),
        session_hi: u64_at(b, 16).unwrap_or(0),
        run_id: u32_at(b, 24).unwrap_or(0),
        clock_epoch: u32_at(b, 28).unwrap_or(0),
        player_tag,
        ticks_per_ns: f64::from_le_bytes(
            b[48..56].try_into().expect("invariant: the header is at least 128 B"),
        ),
        zone_stride: u32_at(b, 56).unwrap_or(0),
        window: u32_at(b, 60).unwrap_or(0),
        region_capacity: u32_at(b, 64).unwrap_or(0),
        flags: u32_at(b, 68).unwrap_or(0),
        log_ceiling: b[72],
        profiling_tier: b[73],
    })
}

/// Parse one block's payload, or `None` if it does not describe itself consistently.
fn decode_payload(p: &[u8]) -> Option<(WindowHead, Vec<ZoneRow<'_>>, Vec<WindowRec>)> {
    if p.len() < WINDOW_HEAD_BYTES {
        return None;
    }
    let head = WindowHead {
        fixed_elapsed_ns: u64_at(p, 0)?,
        drops: u32_at(p, 8)?,
        clock_epoch: u32_at(p, 12)?,
        frame_first: u32_at(p, 16)?,
        frame_last: u32_at(p, 20)?,
        zone_rows: u16_at(p, 24)?,
        recs: u16_at(p, 26)?,
        flags: u32_at(p, 28)?,
    };
    let mut off = WINDOW_HEAD_BYTES;
    let mut rows = Vec::with_capacity(head.zone_rows as usize);
    for _ in 0..head.zone_rows {
        if off + ZONE_ROW_FIXED_BYTES > p.len() {
            return None;
        }
        let id = u16_at(p, off)?;
        let kind = p[off + 2];
        let scope = p[off + 3];
        let tier = p[off + 4];
        let region = p[off + 5];
        let name_len = p[off + 6] as usize;
        off += ZONE_ROW_FIXED_BYTES;
        let name = p.get(off..off + name_len)?;
        off += name_len;
        rows.push(ZoneRow { id, kind, scope, tier, region, name });
    }
    let mut recs = Vec::with_capacity(head.recs as usize);
    for _ in 0..head.recs {
        if off + WINDOW_REC_BYTES > p.len() {
            return None;
        }
        recs.push(WindowRec {
            total: u64_at(p, off)?,
            count: u32_at(p, off + 8)?,
            min: u32_at(p, off + 12)?,
            max: u32_at(p, off + 16)?,
            median: u32_at(p, off + 20)?,
            p95: u32_at(p, off + 24)?,
            id: u16_at(p, off + 28)?,
            flags: u16_at(p, off + 30)?,
        });
        off += WINDOW_REC_BYTES;
    }
    // A payload longer than what its own head describes is a block that disagrees with itself. It
    // could be a later schema's extra records read by an older decoder, and returning the prefix
    // would be exactly the partial belief the framing exists to prevent.
    if off != p.len() {
        return None;
    }
    Some((head, rows, recs))
}

/// Decode a whole file.
///
/// Allocates: the block and record vectors. This is offline decoder code — one call per file, in a
/// CLI or a gate, never in a frame — and the alternative is an iterator whose lifetime dance buys
/// nothing at a call site that is about to print.
pub fn decode(bytes: &[u8]) -> Result<Decoded<'_>, DecodeError> {
    let header = decode_header(bytes)?;
    let mut blocks = Vec::new();
    let mut records_ok = 0u64;
    let mut off = HEADER_BYTES;
    loop {
        // Anything that is not a whole, self-consistent, CRC-clean block ends the walk. Which of
        // the four conditions it was does not change what a reader may do with the bytes.
        if off + BLOCK_HEADER_BYTES > bytes.len() {
            break;
        }
        let Some(magic) = u32_at(bytes, off) else { break };
        if magic != BLOCK_MAGIC {
            break;
        }
        let (Some(len), Some(seq), Some(crc)) =
            (u32_at(bytes, off + 4), u32_at(bytes, off + 8), u32_at(bytes, off + 12))
        else {
            break;
        };
        let start = off + BLOCK_HEADER_BYTES;
        let Some(end) = start.checked_add(len as usize) else { break };
        if end > bytes.len() {
            break;
        }
        let payload = &bytes[start..end];
        if crc32(payload) != crc {
            break;
        }
        let Some((head, zone_rows, recs)) = decode_payload(payload) else { break };
        records_ok += recs.len() as u64;
        blocks.push(Block { seq, head, zone_rows, recs });
        off = end;
    }
    Ok(Decoded { header, blocks, records_ok, truncated_tail_bytes: bytes.len() - off })
}

/// Re-encode a decoded file into `buf`, returning the bytes written.
///
/// **This is `G15`'s round-trip property in executable form**, and it is stated against the framing
/// rather than against the whole file: re-encoding equals the input **minus
/// [`Decoded::truncated_tail_bytes`]**. A property phrased over the whole file would fail on every
/// torn file, which is exactly the case a player's full disk produces and the one a round-trip
/// gate most needs to hold on.
#[must_use]
pub fn reencode(d: &Decoded<'_>, buf: &mut [u8]) -> Option<usize> {
    let mut e = Encoder::new(buf);
    if !e.header(&d.header) {
        return None;
    }
    for b in &d.blocks {
        let at = e.begin_block()?;
        if !e.window_head(&b.head) {
            return None;
        }
        for r in &b.zone_rows {
            if !e.zone_row(r) {
                return None;
            }
        }
        for r in &b.recs {
            if !e.window_rec(r) {
                return None;
            }
        }
        if !e.end_block(at, b.seq) {
            return None;
        }
    }
    Some(e.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The widths every other claim in this module is stated in.
    #[test]
    fn the_fixed_widths_are_what_the_encoder_writes() {
        let mut buf = [0u8; 512];
        let mut e = Encoder::new(&mut buf);
        assert!(e.header(&StreamHeader { schema_version: TELEMETRY_SCHEMA_VERSION, ..Default::default() }));
        assert_eq!(e.len(), HEADER_BYTES);
        let at = e.begin_block().expect("room for a block header");
        assert_eq!(e.len() - at, BLOCK_HEADER_BYTES);
        let before = e.len();
        assert!(e.window_head(&WindowHead::default()));
        assert_eq!(e.len() - before, WINDOW_HEAD_BYTES);
        let before = e.len();
        assert!(e.window_rec(&WindowRec::default()));
        assert_eq!(e.len() - before, WINDOW_REC_BYTES);
        let before = e.len();
        assert!(e.zone_row(&ZoneRow { id: 1, kind: 1, scope: 0, tier: 0, region: 0, name: b"ab" }));
        assert_eq!(e.len() - before, ZONE_ROW_FIXED_BYTES + 2);
    }

    /// A file this encoder wrote decodes to what was written, and re-encodes byte for byte.
    #[test]
    fn a_whole_file_round_trips_byte_for_byte() {
        let (bytes, _) = sample_file(3);
        let d = decode(&bytes).expect("the file this test just wrote must decode");
        assert_eq!(d.blocks_ok(), 3);
        assert_eq!(d.records_ok, 6, "two records per block");
        assert_eq!(d.truncated_tail_bytes, 0, "a whole file has no tail");
        assert_eq!(d.header.session_lo, 0xDEAD_BEEF);
        assert_eq!(d.blocks[1].seq, 1);
        assert_eq!(d.blocks[1].head.fixed_elapsed_ns, 1_000);
        assert_eq!(d.blocks[0].zone_rows[0].name, b"zone.one");
        assert_eq!(d.blocks[0].recs[0].quantiles(), Some((7, 9)));
        assert_eq!(d.blocks[0].recs[1].quantiles(), None, "an unsubscribed zone has no quantiles");

        let mut out = vec![0u8; bytes.len()];
        let n = reencode(&d, &mut out).expect("re-encoding a decoded file must fit");
        assert_eq!(n, bytes.len());
        assert_eq!(&out[..n], &bytes[..], "the codec is not an exact inverse");
    }

    /// The `ENOSPC` shape: the last block is cut in half. The decoder returns the whole blocks,
    /// reports the tail, and returns NOTHING from the torn one.
    #[test]
    fn a_torn_tail_is_detected_rather_than_partially_decoded() {
        let (bytes, block_len) = sample_file(3);
        let cut = bytes.len() - block_len / 2;
        let torn = &bytes[..cut];

        let d = decode(torn).expect("a torn file still has a whole header");
        assert_eq!(d.blocks_ok(), 2, "the torn block must not be returned");
        assert_eq!(d.records_ok, 4);
        assert_eq!(d.truncated_tail_bytes, block_len - block_len / 2);

        // G15 clause (c): the property holds on the TORN file, stated against the framing.
        let mut out = vec![0u8; torn.len()];
        let n = reencode(&d, &mut out).expect("re-encode must fit inside the torn input");
        assert_eq!(n, torn.len() - d.truncated_tail_bytes);
        assert_eq!(&out[..n], &torn[..n]);
    }

    /// One flipped payload bit ends the walk at that block.
    #[test]
    fn a_crc_mismatch_ends_the_walk_at_that_block() {
        let (mut bytes, block_len) = sample_file(3);
        // Into the SECOND block's payload, past its 16 B header.
        let victim = HEADER_BYTES + block_len + BLOCK_HEADER_BYTES + 4;
        bytes[victim] ^= 0x01;
        let d = decode(&bytes).expect("the header is untouched");
        assert_eq!(d.blocks_ok(), 1, "a corrupt block is not decoded, and neither is anything after");
        assert_eq!(d.truncated_tail_bytes, block_len * 2);
    }

    /// A payload with MORE bytes than its own head accounts for is refused whole.
    ///
    /// **This clause exists because deleting the check it guards left every other test green.** The
    /// shape it catches is a later schema's block read by this decoder: the head says two records,
    /// the payload carries three, the CRC is perfectly valid over all of them. Returning the prefix
    /// would be a decoder silently answering a question about a format it does not know — the same
    /// partial belief the torn-tail rule forbids, arriving through the one door the CRC cannot
    /// close, because the writer of those bytes checksummed exactly what it meant to write.
    #[test]
    fn a_payload_longer_than_its_own_head_is_refused_whole() {
        let (bytes, _) = sample_file(1);
        // Splice one extra payload byte in, then restate `len` and the CRC so the block is
        // self-consistent in every way EXCEPT its head's own record count.
        let mut spliced = bytes.clone();
        spliced.push(0xAB);
        let old_len = u32::from_le_bytes(
            spliced[HEADER_BYTES + 4..HEADER_BYTES + 8].try_into().expect("4 bytes"),
        );
        spliced[HEADER_BYTES + 4..HEADER_BYTES + 8]
            .copy_from_slice(&(old_len + 1).to_le_bytes());
        let crc = crc32(&spliced[HEADER_BYTES + BLOCK_HEADER_BYTES..]);
        spliced[HEADER_BYTES + 12..HEADER_BYTES + 16].copy_from_slice(&crc.to_le_bytes());

        let d = decode(&spliced).expect("the header is untouched");
        assert_eq!(d.blocks_ok(), 0, "a block that disagrees with itself is not decoded");
        assert_eq!(d.records_ok, 0, "and none of its records are returned");
        assert_eq!(d.truncated_tail_bytes, spliced.len() - HEADER_BYTES);
    }

    /// A `len` that runs past the end of the file is refused rather than trusted into a panic.
    #[test]
    fn a_length_past_the_end_is_refused() {
        let (mut bytes, _) = sample_file(1);
        bytes[HEADER_BYTES + 4..HEADER_BYTES + 8].copy_from_slice(&u32::MAX.to_le_bytes());
        let d = decode(&bytes).expect("the header is untouched");
        assert_eq!(d.blocks_ok(), 0);
        assert_eq!(d.records_ok, 0);
    }

    /// The header's three refusals happen before any block is looked at.
    #[test]
    fn the_header_refuses_what_it_cannot_be_sure_of() {
        assert_eq!(decode(&[0u8; 8]).unwrap_err(), DecodeError::TooShort { len: 8 });

        let (mut bytes, _) = sample_file(1);
        bytes[0] ^= 0xFF;
        assert!(matches!(decode(&bytes).unwrap_err(), DecodeError::NotAStream { .. }));

        let (mut bytes, _) = sample_file(1);
        bytes[4..8].copy_from_slice(&99u32.to_le_bytes());
        assert_eq!(decode(&bytes).unwrap_err(), DecodeError::Schema { found: 99 });
    }

    /// `KIND_UNKNOWN` is not `Span`, which a raw discriminant cast would have made it.
    #[test]
    fn an_unobserved_kind_is_not_a_span() {
        assert_eq!(kind_of_byte(KIND_UNKNOWN), None);
        assert_eq!(kind_of_byte(kind_byte(SampleKind::Span)), Some(SampleKind::Span));
        assert_eq!(kind_of_byte(kind_byte(SampleKind::Counter)), Some(SampleKind::Counter));
        assert_eq!(kind_of_byte(kind_byte(SampleKind::Gauge)), Some(SampleKind::Gauge));
        assert_ne!(kind_byte(SampleKind::Span), KIND_UNKNOWN, "the shift is the whole point");
    }

    /// The checksum is the IEEE 802.3 one, pinned against its published check value.
    #[test]
    fn the_crc_is_the_published_ieee_one() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926, "the standard CRC-32 check value");
        assert_eq!(crc32(b""), 0);
    }

    /// The encoder refuses rather than truncating, and refusing writes nothing.
    #[test]
    fn a_full_buffer_refuses_instead_of_writing_half_a_record() {
        let mut buf = [0u8; HEADER_BYTES + BLOCK_HEADER_BYTES + WINDOW_HEAD_BYTES + 8];
        let mut e = Encoder::new(&mut buf);
        assert!(e.header(&StreamHeader { schema_version: TELEMETRY_SCHEMA_VERSION, ..Default::default() }));
        assert!(e.begin_block().is_some());
        assert!(e.window_head(&WindowHead::default()));
        let before = e.len();
        assert!(!e.window_rec(&WindowRec::default()), "8 bytes cannot hold a 32 byte record");
        assert_eq!(e.len(), before, "a refused write must not advance the cursor");
    }

    /// Two blocks, two records each, with one zone row per block. Returns the bytes and the length
    /// of one whole block, which the tearing tests cut against.
    fn sample_file(blocks: usize) -> (Vec<u8>, usize) {
        let mut buf = vec![0u8; 4096];
        let mut e = Encoder::new(&mut buf);
        let h = StreamHeader {
            schema_version: TELEMETRY_SCHEMA_VERSION,
            session_lo: 0xDEAD_BEEF,
            session_hi: 0xFEED,
            run_id: 7,
            clock_epoch: 1,
            player_tag: [3u8; 16],
            ticks_per_ns: 2.5,
            zone_stride: 4096,
            window: 121,
            region_capacity: 1024,
            flags: HEADER_FLAG_INVARIANT_TSC,
            log_ceiling: 5,
            profiling_tier: 2,
        };
        assert!(e.header(&h));
        let after_header = e.len();
        let mut one_block = 0usize;
        for seq in 0..blocks {
            let at = e.begin_block().expect("room");
            assert!(e.window_head(&WindowHead {
                fixed_elapsed_ns: seq as u64 * 1_000,
                drops: 0,
                clock_epoch: 1,
                frame_first: seq as u32 * 121,
                frame_last: seq as u32 * 121 + 120,
                zone_rows: 1,
                recs: 2,
                flags: 0,
            }));
            assert!(e.zone_row(&ZoneRow {
                id: 11,
                kind: kind_byte(SampleKind::Span),
                scope: 0,
                tier: 2,
                region: 0,
                name: b"zone.one",
            }));
            assert!(e.window_rec(&WindowRec {
                total: 1234,
                count: 121,
                min: 5,
                max: 40,
                median: 7,
                p95: 9,
                id: 11,
                flags: REC_HAS_QUANTILES,
            }));
            assert!(e.window_rec(&WindowRec {
                total: 99,
                count: 12,
                min: 1,
                max: 3,
                median: 0,
                p95: 0,
                id: 12,
                flags: 0,
            }));
            assert!(e.end_block(at, seq as u32));
            if seq == 0 {
                one_block = e.len() - after_header;
            }
        }
        let n = e.len();
        buf.truncate(n);
        (buf, one_block)
    }
}
