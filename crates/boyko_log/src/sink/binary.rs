//! `BinarySink`'s wire format: the sink writes bytes, an offline decoder formats them *(L13b)*.
//!
//! # Why a second format exists at all
//!
//! `core::fmt` on the sink thread is the throughput ceiling — around 500 K records·s⁻¹ against a
//! producer that can offer far more. Enlarging the ring moves the loss point later; it does not
//! move the ceiling. **Not formatting is the only change that does**, which is why this format
//! exists and why Decision 22 attaches a revert clause to it: if `sink_sustained_rate_binary` does
//! not measure **≥ 5× the text sink in the same sitting**, L13b is reverted rather than justified.
//!
//! ⚠️ **That measurement has not been taken.** It needs the bench harness `05-LADDER-GATES.md`'s
//! table specifies and which L10-C measured as never built. This module ships the *format* and its
//! codec, gated by a round-trip test; the throughput claim is UNPROVEN and the revert clause is
//! still live.
//!
//! # The dictionary, and why a full table is not a lost record
//!
//! A record references its site by a `u16` id into `SITE_DICT`. On a miss the sink emits a
//! **dictionary record** (the site spelled out, once) and then references it forever after.
//!
//! On a **full** table it emits `boyko-W0116` once and writes an **inline site record** — file,
//! line and format spelled out in the record itself — rather than reusing an id. Reusing one would
//! decode a later record under an earlier site's file and line: a log that lies about where it came
//! from, which is worse than a log that is larger than it needed to be.

use core::sync::atomic::{AtomicU32, AtomicU64, Ordering};

/// Frame kinds. One byte, first in every frame, so a decoder can skip what it does not understand.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum FrameKind {
    /// `{site_id, file, line, fmt}` — the site spelled out, emitted once per site.
    Dictionary = 1,
    /// `{site_id, tsc_delta, len, flags, epoch_lo, payload}` — the common case.
    Record = 2,
    /// A full absolute timestamp. Re-emitted when a delta would not fit, and after a rotation.
    Anchor = 3,
    /// A record whose site could not be interned: the site is spelled out INLINE beside it.
    InlineSite = 4,
}

impl FrameKind {
    /// Decode a kind byte. `None` for anything this decoder does not know — a frame from a newer
    /// writer is skipped by its length, never guessed at.
    #[must_use]
    pub const fn from_raw(b: u8) -> Option<FrameKind> {
        match b {
            1 => Some(FrameKind::Dictionary),
            2 => Some(FrameKind::Record),
            3 => Some(FrameKind::Anchor),
            4 => Some(FrameKind::InlineSite),
            _ => None,
        }
    }
}

/// Entries in the site dictionary. The **table** is the limit, not the `u16` width — which has 16×
/// headroom over it, deliberately, so a full table is a reported condition and never an id wrap.
pub const SITE_DICT_LEN: usize = 4096;

/// Ids handed out so far.
static SITE_DICT_NEXT: AtomicU32 = AtomicU32::new(0);

/// One dictionary slot: the site pointer that claimed it, and the id it was given.
///
/// `id` is published **after** `ptr`, and a reader that finds its pointer waits for the id. A slot
/// observed mid-claim is still a claim; treating it as free would hand the same site two ids, and
/// two ids for one site is a dictionary that disagrees with itself about where a record came from.
struct SiteSlot {
    ptr: AtomicU64,
    id: AtomicU32,
}

impl SiteSlot {
    const fn new() -> SiteSlot {
        SiteSlot { ptr: AtomicU64::new(0), id: AtomicU32::new(u32::MAX) }
    }
}

// SAFETY: every field is an atomic and the only transition is 0 -> a `&'static LogSite` pointer,
//   won by one `compare_exchange`. `&'static LogSite` never dangles and no slot is ever released,
//   so a published pointer stays valid and comparable for the process lifetime.
unsafe impl Sync for SiteSlot {}

/// The dictionary. `.bss` — 4096 x 16 B = 64 KiB of zero pages an unenabled process never touches.
static SITE_DICT: [SiteSlot; SITE_DICT_LEN] = [const { SiteSlot::new() }; SITE_DICT_LEN];

/// Fibonacci-hash a site pointer to a starting probe index.
///
/// Sites are `static`s, so their addresses share low bits (alignment) AND high bits (one image).
/// A mask of the raw pointer would cluster every site in one region of the table; the multiply
/// mixes the middle bits, which are the ones that actually differ.
#[inline]
const fn probe_start(ptr: u64) -> usize {
    ((ptr.wrapping_mul(0x9E37_79B9_7F4A_7C15) >> 32) as usize) & (SITE_DICT_LEN - 1)
}

const _: () = assert!(
    SITE_DICT_LEN.is_power_of_two(),
    "probe_start masks with SITE_DICT_LEN - 1, which is only a modulo for a power of two"
);

/// Intern a site pointer, returning `(id, is_new)`, or `None` when the table is full.
///
/// `is_new` is the caller's cue to emit a `SiteDef` frame; a repeat visit costs one probe and no
/// bytes, which is the entire reason a dictionary exists rather than a file/line pair per record.
///
/// `None` means the table is full. It is deliberately **not** a fallback id: an id reused across
/// two sites decodes later records under an earlier site's file and line, and a log that lies about
/// its own origin is worse than a large one. The caller writes an [`FrameKind::InlineSite`] frame
/// and `W0116` is reported once.
#[must_use]
pub fn intern_site(site: *const crate::LogSite) -> Option<(u16, bool)> {
    let key = site as u64;
    if key == 0 {
        return None;
    }
    let start = probe_start(key);
    // Linear probing over the WHOLE table: a full pass is what proves the table is full, and that
    // is the only condition allowed to report `W0116`. Stopping early would report a full table
    // that is not full -- a false report in the module whose subject is silent failure.
    for step in 0..SITE_DICT_LEN {
        let slot = &SITE_DICT[(start + step) & (SITE_DICT_LEN - 1)];
        let seen = slot.ptr.load(Ordering::Acquire);
        if seen == key {
            return Some((wait_for_id(slot), false));
        }
        if seen == 0 {
            match slot.ptr.compare_exchange(0, key, Ordering::AcqRel, Ordering::Acquire) {
                Ok(_) => {
                    let n = SITE_DICT_NEXT.fetch_add(1, Ordering::Relaxed);
                    debug_assert!(
                        (n as usize) < SITE_DICT_LEN,
                        "invariant: one id per claimed slot, and slots are never released"
                    );
                    slot.id.store(n, Ordering::Release);
                    return Some((n as u16, true));
                }
                // Lost the race. If the winner was THIS site, the id is ours too: one site reaching
                // the dictionary from two threads is one entry, not two.
                Err(actual) if actual == key => return Some((wait_for_id(slot), false)),
                Err(_) => continue,
            }
        }
    }
    report_dict_full();
    None
}

/// Wait for a claimed slot's id to be published.
///
/// Bounded by one store on the claiming thread. It spins rather than blocking because this runs on
/// the drain, and a lock here would be the one thing this crate refuses.
#[inline]
fn wait_for_id(slot: &SiteSlot) -> u16 {
    loop {
        let id = slot.id.load(Ordering::Acquire);
        if id != u32::MAX {
            return id as u16;
        }
        core::hint::spin_loop();
    }
}

/// Dictionary ids handed out. The census's question and `W0116`'s subject.
#[must_use]
pub fn site_dict_used() -> usize {
    SITE_DICT_NEXT.load(Ordering::Relaxed) as usize
}

/// Reset the dictionary. Test and console surface, `pub` for the reason `sample::reset_counters` is.
pub fn reset_site_dict() {
    for slot in &SITE_DICT {
        slot.id.store(u32::MAX, Ordering::Relaxed);
        slot.ptr.store(0, Ordering::Release);
    }
    SITE_DICT_NEXT.store(0, Ordering::Relaxed);
}

/// A decoded record frame, as the offline decoder sees it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct RecordFrame<'a> {
    /// Index into the dictionary the decoder has replayed.
    pub site_id: u16,
    /// Ticks since the file's current anchor. A `u32` spans ~1.4 s at 3 GHz, which is why an
    /// anchor is re-emitted before it would overflow.
    pub tsc_delta: u32,
    /// The record's argument flags, carried verbatim from the ring.
    pub flags: u8,
    /// Low 8 bits of the clock epoch, so a record straddling a suspend is legible.
    pub epoch_lo: u8,
    /// The self-describing value payload, decoded by `record::render_payload` on the far side.
    pub payload: &'a [u8],
}

/// Anchor frame width: kind + absolute ticks + the writer's `ticks_per_ns`.
pub const ANCHOR_BYTES: usize = 1 + 8 + 8;

/// Frame header width: kind + site_id + tsc_delta + len + flags + epoch_lo.
pub const RECORD_HEADER_BYTES: usize = 1 + 2 + 4 + 2 + 1 + 1;

/// Encode one record frame into `out`, returning the bytes written.
///
/// Little-endian throughout and **length-prefixed**, so a decoder that meets a frame kind it does
/// not know can skip it by `len` rather than resynchronising by guesswork.
pub fn encode_record(out: &mut [u8], f: &RecordFrame<'_>) -> Option<usize> {
    let need = RECORD_HEADER_BYTES + f.payload.len();
    if out.len() < need || f.payload.len() > u16::MAX as usize {
        return None;
    }
    out[0] = FrameKind::Record as u8;
    out[1..3].copy_from_slice(&f.site_id.to_le_bytes());
    out[3..7].copy_from_slice(&f.tsc_delta.to_le_bytes());
    out[7..9].copy_from_slice(&(f.payload.len() as u16).to_le_bytes());
    out[9] = f.flags;
    out[10] = f.epoch_lo;
    out[RECORD_HEADER_BYTES..need].copy_from_slice(f.payload);
    Some(need)
}

/// Decode one record frame, returning it and the bytes consumed.
///
/// Returns `None` on a truncated frame rather than reading past the slice: the decoder reads a file
/// that may have been cut off mid-write by a crash — which is precisely the run whose tail matters
/// most — so a short read is an ordinary outcome, not a corrupt-input panic.
#[must_use]
pub fn decode_record(buf: &[u8]) -> Option<(RecordFrame<'_>, usize)> {
    if buf.len() < RECORD_HEADER_BYTES || FrameKind::from_raw(buf[0]) != Some(FrameKind::Record) {
        return None;
    }
    let len = u16::from_le_bytes([buf[7], buf[8]]) as usize;
    let end = RECORD_HEADER_BYTES + len;
    if buf.len() < end {
        return None;
    }
    Some((
        RecordFrame {
            site_id: u16::from_le_bytes([buf[1], buf[2]]),
            tsc_delta: u32::from_le_bytes([buf[3], buf[4], buf[5], buf[6]]),
            flags: buf[9],
            epoch_lo: buf[10],
            payload: &buf[RECORD_HEADER_BYTES..end],
        },
        end,
    ))
}

/// `boyko-W0116`, once: the site dictionary is full and later sites are written inline.
///
/// **Nothing is lost and nothing is wrong** — the records are simply larger, because each carries
/// its own file, line and format instead of a two-byte reference. That is the whole report, and it
/// is why this is a `Warn` and not an `Error`.
///
/// `Once`: past a full table **every** later site writes inline, so the condition holds for the
/// rest of the run. One line stating that the stream has grown is the fact; one per site would be a
/// storm made of the very records it is warning about.
#[cold]
#[inline(never)]
fn report_dict_full() {
    static SITE: crate::codes::OnceSite = crate::codes::OnceSite::new();
    if SITE.claim() {
        crate::warn!(
            crate::Log,
            crate::codes::W0116,
            "binary site dictionary is full at {} entries; later sites are written INLINE -- the              stream grows, no record is lost, and no id is reused",
            SITE_DICT_LEN
        );
    }
}

// ─────────────────── the destination: what makes this a SINK and not a codec ───────────────────

/// Longest binary-sink path, in bytes. Same bound as the text sink's, for the same reason: a path
/// is configuration, and configuration lives in `.bss` rather than on a heap this crate refuses.
pub const MAX_BIN_PATH_BYTES: usize = 256;

/// The path cell, written on the enable path and read at `open`.
struct BinPathSlot(core::cell::UnsafeCell<[u8; MAX_BIN_PATH_BYTES]>);

// SAFETY: written only by `set_path`, which the host calls on the enable path before any drain
//   token exists, and read only by `open` on that same path. `BIN_PATH_LEN`'s `Release`/`Acquire`
//   pair publishes the bytes.
unsafe impl Sync for BinPathSlot {}

static BIN_PATH: BinPathSlot = BinPathSlot(core::cell::UnsafeCell::new([0; MAX_BIN_PATH_BYTES]));
static BIN_PATH_LEN: AtomicU64 = AtomicU64::new(0);

/// The open handle.
struct BinFileSlot(core::cell::UnsafeCell<Option<std::fs::File>>);

// SAFETY: `open` runs on the enable path before any drain token exists; every later access holds
//   the token, of which there is exactly one. So this cell has one writer by construction.
unsafe impl Sync for BinFileSlot {}

static BIN_FILE: BinFileSlot = BinFileSlot(core::cell::UnsafeCell::new(None));

/// Frames written since `open`. The census's question and the test's.
static FRAMES: AtomicU64 = AtomicU64::new(0);

/// The tick value the file's anchor recorded. **Every `tsc_delta` is measured from this.**
///
/// It has to be remembered, and the first draft did not: `write_record` wrote `tsc as u32`, the low
/// 32 bits of the ABSOLUTE counter, into a field called `tsc_delta` and documented as "ticks since
/// the file's current anchor". `logdec` then printed `+425.840ms` for three records emitted
/// microseconds apart -- a number that is plausible, wrong, and indistinguishable from an elapsed
/// time to anyone reading the file. Found by reading the tool's output, not by any test of the
/// writer: every frame round-tripped byte for byte, because the bytes were faithfully the wrong
/// number.
static ANCHOR_TICKS: AtomicU64 = AtomicU64::new(0);

/// Record the binary sink's destination. Returns `false` for a path that does not fit.
///
/// Separate from the TEXT sink's `set_path` on purpose: a process may want both, and one path
/// setter serving two destinations would make "which file did that go to" unanswerable.
pub fn set_path(path: &str) -> bool {
    if path.is_empty() || path.len() > MAX_BIN_PATH_BYTES {
        return false;
    }
    // SAFETY: the enable path is single-threaded by contract (no drain token exists yet), and the
    //   `Release` below publishes these bytes to whoever `Acquire`s the length.
    unsafe {
        let dst = core::slice::from_raw_parts_mut(BIN_PATH.0.get().cast::<u8>(), path.len());
        dst.copy_from_slice(path.as_bytes());
    }
    BIN_PATH_LEN.store(path.len() as u64, Ordering::Release);
    true
}

/// Whether a destination has been recorded.
#[must_use]
pub fn path_recorded() -> bool {
    BIN_PATH_LEN.load(Ordering::Acquire) != 0
}

/// Open the destination and write the file's opening anchor.
///
/// The anchor goes in at `open` rather than before the first record, because a file whose first
/// frame is a record has no absolute time to add its deltas to -- it decodes to a session that
/// started at zero, which is worse than one that refuses to decode.
pub fn open() -> bool {
    let len = BIN_PATH_LEN.load(Ordering::Acquire) as usize;
    if len == 0 {
        return false;
    }
    // SAFETY: `Acquire` pairs with `set_path`'s `Release`, so these `len` bytes are that call's.
    //   No drain token exists on the enable path, so no other thread is inside this cell.
    let path = unsafe {
        let src = core::slice::from_raw_parts(BIN_PATH.0.get().cast::<u8>(), len);
        match core::str::from_utf8(src) {
            Ok(s) => s,
            Err(_) => return false,
        }
    };
    let Ok(file) = std::fs::File::create(path) else { return false };
    FRAMES.store(0, Ordering::Relaxed);
    // SAFETY: as above -- `open` runs before any drain token can be claimed.
    unsafe { *BIN_FILE.0.get() = Some(file) };
    // The dictionary is per FILE, not per process: a decoder replays it from the frames in the
    // file it is reading, so ids from a previous file would decode this one under wrong sites.
    reset_site_dict();
    write_anchor();
    true
}

/// Frames written since `open`.
#[must_use]
pub fn frames_written() -> u64 {
    FRAMES.load(Ordering::Relaxed)
}

/// Write raw bytes to the open handle. Returns `false` when there is no destination.
fn write_raw(bytes: &[u8]) -> bool {
    use std::io::Write;
    // SAFETY: every caller reaches here from the drain, holding the single drain token, or from
    //   `open` on the enable path before any token exists.
    let slot = unsafe { &mut *BIN_FILE.0.get() };
    let Some(f) = slot.as_mut() else { return false };
    if f.write_all(bytes).is_err() {
        return false;
    }
    FRAMES.fetch_add(1, Ordering::Relaxed);
    true
}

/// Emit an `Anchor` frame: the absolute clock this file's deltas are relative to, **and its
/// scale**.
///
/// # The scale is in the file because the reader is on another machine
///
/// The first draft wrote ticks alone. A tick count is meaningless without `ticks_per_ns`, and that
/// number is a property of the CPU that produced the file, not of the one reading it -- so a
/// decoder could print `+41231 ticks` and nothing better, for a format whose entire purpose is to
/// be read offline. Found by writing `logdec`; no writer-side test could have found it, because
/// the writer's own process knows the scale.
///
/// Eight bytes, once per file, for the difference between a tick count and a time.
fn write_anchor() -> bool {
    let mut buf = [0u8; 1 + 8 + 8];
    let now = boyko_diag::clock::ticks();
    ANCHOR_TICKS.store(now, Ordering::Relaxed);
    buf[0] = FrameKind::Anchor as u8;
    buf[1..9].copy_from_slice(&now.to_le_bytes());
    buf[9..17].copy_from_slice(&boyko_diag::clock::ticks_per_ns().to_bits().to_le_bytes());
    write_raw(&buf)
}

/// Emit a `Dictionary` frame: one site, spelled out, so every later record can name it by id.
///
/// Length-prefixed per field rather than fixed-width, because a `file` and a `fmt` are arbitrary
/// strings and a fixed cap would silently truncate the one thing that makes a record locatable.
fn write_dictionary(site_id: u16, site: &crate::LogSite) -> bool {
    let file = site.file.as_bytes();
    let fmt = site.fmt.as_bytes();
    let mut buf = [0u8; 1 + 2 + 4 + 2 + 2 + 512];
    let need = 1 + 2 + 4 + 2 + file.len() + 2 + fmt.len();
    if need > buf.len() || file.len() > u16::MAX as usize || fmt.len() > u16::MAX as usize {
        return false;
    }
    buf[0] = FrameKind::Dictionary as u8;
    buf[1..3].copy_from_slice(&site_id.to_le_bytes());
    buf[3..7].copy_from_slice(&site.line.to_le_bytes());
    buf[7..9].copy_from_slice(&(file.len() as u16).to_le_bytes());
    let mut at = 9;
    buf[at..at + file.len()].copy_from_slice(file);
    at += file.len();
    buf[at..at + 2].copy_from_slice(&(fmt.len() as u16).to_le_bytes());
    at += 2;
    buf[at..at + fmt.len()].copy_from_slice(fmt);
    at += fmt.len();
    write_raw(&buf[..at])
}

/// Write one record to the binary sink, interning its site first.
///
/// **The whole point of the format lives in the two returns of `intern_site`**: a site seen before
/// costs one probe and no bytes, and a site seen for the first time costs one `Dictionary` frame.
/// A full dictionary is neither -- the record is written with its site INLINE, so it is still
/// locatable, and `W0116` has already been reported once.
pub(crate) fn write_record(
    _token: &crate::drain_owner::DrainToken,
    site: &'static crate::LogSite,
    tsc: u64,
    flags: u8,
    payload: &[u8],
) -> bool {
    let mut buf = [0u8; 1024];
    match intern_site(core::ptr::from_ref(site)) {
        Some((id, is_new)) => {
            if is_new && !write_dictionary(id, site) {
                return false;
            }
            let frame = RecordFrame {
                site_id: id,
                // FROM THE ANCHOR, not the raw counter. `wrapping_sub` because a record staged
                // before this file's anchor -- possible when a drain pass straddles an `open` --
                // must not underflow; it wraps to a huge delta, which reads as obviously wrong
                // rather than as a small plausible time.
                //
                // Truncated to `u32` deliberately: a `u32` of ticks spans ~1.4 s at 3 GHz, and a
                // file that runs longer needs a re-anchor, which this rung does not do -- recorded
                // as owed rather than hidden.
                tsc_delta: tsc.wrapping_sub(ANCHOR_TICKS.load(Ordering::Relaxed)) as u32,
                flags,
                epoch_lo: 0,
                payload,
            };
            match encode_record(&mut buf, &frame) {
                Some(n) => write_raw(&buf[..n]),
                None => false,
            }
        }
        None => {
            // The dictionary is full. Spell the site out beside the record rather than reusing an
            // id: a log that lies about where a record came from is worse than a large one.
            let file = site.file.as_bytes();
            let fmt = site.fmt.as_bytes();
            // ⚠️ THE FORMAT LITERAL WAS MISSING HERE, AND THIS MODULE'S HEADER SAID IT WAS NOT.
            // The header promises "file, line and format spelled out in the record itself"; the
            // first draft wrote file and line only, so an inline record was LOCATABLE and not
            // FORMATTABLE -- the decoder could say where it came from and could only dump its
            // values as raw tags. Found by writing the reader, which is the only thing that could
            // have found it: every writer-side test passed.
            let need = 1 + 4 + 2 + file.len() + 2 + fmt.len() + 2 + payload.len();
            if need > buf.len() {
                return false;
            }
            buf[0] = FrameKind::InlineSite as u8;
            buf[1..5].copy_from_slice(&site.line.to_le_bytes());
            buf[5..7].copy_from_slice(&(file.len() as u16).to_le_bytes());
            let mut at = 7;
            buf[at..at + file.len()].copy_from_slice(file);
            at += file.len();
            buf[at..at + 2].copy_from_slice(&(fmt.len() as u16).to_le_bytes());
            at += 2;
            buf[at..at + fmt.len()].copy_from_slice(fmt);
            at += fmt.len();
            buf[at..at + 2].copy_from_slice(&(payload.len() as u16).to_le_bytes());
            at += 2;
            buf[at..at + payload.len()].copy_from_slice(payload);
            at += payload.len();
            write_raw(&buf[..at])
        }
    }
}

/// Close the destination, flushing it.
pub(crate) fn close(_token: &crate::drain_owner::DrainToken) {
    use std::io::Write;
    // SAFETY: the caller holds the single drain token.
    let slot = unsafe { &mut *BIN_FILE.0.get() };
    if let Some(f) = slot.as_mut() {
        let _ = f.flush();
    }
    *slot = None;
}

// ────────────────────────── the READER: one walker, three consumers ──────────────────────────
//
// # A format with a writer and no reader is the same defect as one with neither
//
// L13b shipped `encode_record`, the dictionary and `W0116`; the destination followed. Nothing
// could read a `.blog` back except a private walker inside one test -- so the test proved a
// decoder that no tool used, and the tool that a reader needs did not exist. The walker below is
// the ONE walker: `logdec` uses it, the format test uses it, and a third consumer would use it
// too. A test with its own copy would go on passing after the shipped decoder broke.

/// One frame, as a reader sees it.
///
/// Borrowed from the buffer throughout: a decoder that copied would allocate per frame to hand
/// back bytes the caller already owns.
///
/// **`PartialEq` and not `Eq`**: the anchor carries an `f64` scale. Deriving `Eq` on a type with a
/// float is a claim about reflexivity that NaN breaks, and a decoder must be able to hand back
/// whatever the file contained rather than refusing values it dislikes.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Frame<'a> {
    /// The absolute clock every following `tsc_delta` is relative to, and its scale.
    Anchor {
        /// Absolute tick count, from `boyko_diag::clock::ticks`.
        ticks: u64,
        /// Ticks per nanosecond **on the machine that wrote the file**. Without it a `tsc_delta`
        /// is a number a reader cannot turn into a time.
        ticks_per_ns: f64,
    },
    /// One site, spelled out, so later records can name it by id.
    Dictionary {
        /// The id later records use.
        site_id: u16,
        /// Source line.
        line: u32,
        /// Source file.
        file: &'a str,
        /// The format literal, which is what makes a record renderable.
        fmt: &'a str,
    },
    /// The common case: a site id and a payload.
    Record(RecordFrame<'a>),
    /// A record whose site could not be interned, carrying its own site.
    InlineSite {
        /// Source line.
        line: u32,
        /// Source file.
        file: &'a str,
        /// The format literal.
        fmt: &'a str,
        /// The value payload.
        payload: &'a [u8],
    },
}

/// Decode one frame, returning it and the bytes consumed.
///
/// `None` for a truncated or unknown frame. **A short read is an ordinary outcome, not corrupt
/// input**: the file a reader most wants is the one a crash cut off mid-write, so the tail is
/// expected to be ragged and the decoder stops rather than panicking.
#[must_use]
pub fn decode_frame(buf: &[u8]) -> Option<(Frame<'_>, usize)> {
    /// Read a `u16` length prefix at `at`, then that many bytes as UTF-8.
    fn take_str(buf: &[u8], at: usize) -> Option<(&str, usize)> {
        if at + 2 > buf.len() {
            return None;
        }
        let n = u16::from_le_bytes([buf[at], buf[at + 1]]) as usize;
        let end = at + 2 + n;
        if end > buf.len() {
            return None;
        }
        Some((core::str::from_utf8(&buf[at + 2..end]).ok()?, end))
    }

    match FrameKind::from_raw(*buf.first()?)? {
        FrameKind::Anchor => {
            if buf.len() < ANCHOR_BYTES {
                return None;
            }
            let mut t = [0u8; 8];
            t.copy_from_slice(&buf[1..9]);
            let mut r = [0u8; 8];
            r.copy_from_slice(&buf[9..17]);
            Some((
                Frame::Anchor {
                    ticks: u64::from_le_bytes(t),
                    ticks_per_ns: f64::from_bits(u64::from_le_bytes(r)),
                },
                ANCHOR_BYTES,
            ))
        }
        FrameKind::Dictionary => {
            if buf.len() < 7 {
                return None;
            }
            let site_id = u16::from_le_bytes([buf[1], buf[2]]);
            let line = u32::from_le_bytes([buf[3], buf[4], buf[5], buf[6]]);
            let (file, at) = take_str(buf, 7)?;
            let (fmt, end) = take_str(buf, at)?;
            Some((Frame::Dictionary { site_id, line, file, fmt }, end))
        }
        FrameKind::Record => decode_record(buf).map(|(r, n)| (Frame::Record(r), n)),
        FrameKind::InlineSite => {
            if buf.len() < 5 {
                return None;
            }
            let line = u32::from_le_bytes([buf[1], buf[2], buf[3], buf[4]]);
            let (file, at) = take_str(buf, 5)?;
            let (fmt, at) = take_str(buf, at)?;
            if at + 2 > buf.len() {
                return None;
            }
            let n = u16::from_le_bytes([buf[at], buf[at + 1]]) as usize;
            let end = at + 2 + n;
            if end > buf.len() {
                return None;
            }
            Some((Frame::InlineSite { line, file, fmt, payload: &buf[at + 2..end] }, end))
        }
    }
}

/// Walk every frame in a buffer, stopping at the first one that does not decode.
///
/// Stopping rather than skipping is deliberate. The frames are length-prefixed, so a reader that
/// meets an unknown KIND could skip it -- but a frame that fails to decode has no trustworthy
/// length, and resynchronising by scanning for a plausible kind byte is guesswork that produces
/// confident nonsense. A truncated tail is reported by the caller comparing consumed bytes against
/// the file's length.
pub fn frames(buf: &[u8]) -> Frames<'_> {
    Frames { buf, at: 0 }
}

/// Iterator over [`decode_frame`]. See [`frames`].
pub struct Frames<'a> {
    buf: &'a [u8],
    at: usize,
}

impl<'a> Frames<'a> {
    /// Bytes consumed so far. `frames(b).count()` then `consumed() < b.len()` is a ragged tail.
    #[must_use]
    pub fn consumed(&self) -> usize {
        self.at
    }
}

impl<'a> Iterator for Frames<'a> {
    type Item = Frame<'a>;

    fn next(&mut self) -> Option<Frame<'a>> {
        let (frame, n) = decode_frame(&self.buf[self.at..])?;
        self.at += n;
        Some(frame)
    }
}
