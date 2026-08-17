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
