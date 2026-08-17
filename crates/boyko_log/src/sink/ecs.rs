//! `ECS_HANDOFF` — the transport that carries formatted lines from the drain token's holder to
//! the ECS.
//!
//! # Deliberately the same shape as `LogLane`, one level up
//!
//! Same header-then-payload framing, same `MASK` arithmetic, same PAD-rather-than-straddle wrap
//! rule, same `Release`/`Acquire` cursor pair. That is not laziness: reusing the rule is why this
//! ring's correctness argument is four clauses instead of a section. A second, subtly different
//! queue protocol in the same crate is how a reviewer stops being able to check either one.
//!
//! # Why a byte ring and not a direct write into the ECS column
//!
//! The consumer of the lanes is whatever thread holds [`DRAIN_OWNER`](crate::drain_owner) — most
//! often the sink thread. The `LogRing` it feeds is a `Resource` on `VmReservation`-backed engine
//! storage, and `VmColumn` is **not** `Send`/`Sync`. If the sink wrote those columns directly, the
//! manual `unsafe impl Send/Sync` on `LogRing` would be false and the only repair would be a lock,
//! which Invariant 1 forbids. So the sink writes plain bytes into `.bss` here, and exactly one ECS
//! system — the one the scheduler grants `ResMut<LogRing>` — copies them out.
//!
//! # What is lost is counted
//!
//! A refused frame is **not** a dropped record: the byte sinks already have it. Only the in-frame
//! view is short. That is why the loss is charged to [`LossClass::Sink`] rather than `Overflow`,
//! and why the count is a ring field a reader can see rather than a silent decrement.

use core::cell::UnsafeCell;
use core::mem::MaybeUninit;
use core::sync::atomic::{AtomicU32, AtomicU64, Ordering};

use boyko_diag::loss::{LossClass, record_here};

use crate::GLOBAL_CEILING;
use crate::level::Level;

/// `true` when this build's compile ceiling is one a shipped title would use.
///
/// The ONE `BOYKO_PROFILE` build axis does not exist yet — it is rung J1, and it is deliberately
/// a single axis so that two rungs cannot each grow their own. Until it lands, the only profile
/// datum in the workspace is [`boyko_diag::profile::LOG_CEILING`], and this is this crate's
/// reading of it: a build that has already deleted `Debug` and `Trace` is a shipping build.
///
/// **J1 obligation.** When the build script lands, this must be replaced by the axis rather than
/// kept beside it. Two answers to "is this a shipping build" is exactly the duplication class the
/// substrate crate exists to delete.
const SHIPPING: bool = (GLOBAL_CEILING as u8) < (Level::Debug as u8);

/// Bytes in the handoff ring.
///
/// **Zero when the compile ceiling is `Off`**, for the same reason [`crate::LANE_ARRAY_LEN`] is
/// zero there: in that build no call site survives the const gates, so nothing can ever be
/// emitted, drained, or pushed here. Reserving a quarter of a megabyte of `.bss` for a ring with
/// no reachable producer is a cost with no corresponding capability.
pub const HANDOFF_BYTES: usize = if (GLOBAL_CEILING as u8) == 0 {
    0
} else if SHIPPING {
    64 * 1024
} else {
    256 * 1024
};

// A ring whose length is not a power of two would need a modulo rather than a mask. The `== 0`
// arm is the `Off`-ceiling ring, which has no producer at all.
const _: () = assert!(HANDOFF_BYTES == 0 || HANDOFF_BYTES.is_power_of_two());

/// Index mask. Meaningless when the ring is empty, and unreachable there — every entry point
/// returns on the `CAPACITY == 0` check first.
const MASK: u32 = HANDOFF_BYTES.wrapping_sub(1) as u32;

/// Usable span. One slot reserved so `used == CAPACITY` is distinguishable from `used == 0`
/// without a third variable, and so `CAPACITY - used` cannot underflow — the arithmetic defect
/// that cost `LogLane` a use-after-free (F6) is not re-derived here, it is copied.
const CAPACITY: u32 = HANDOFF_BYTES.saturating_sub(1) as u32;

/// One frame's header. The payload — the formatted line — follows it contiguously.
///
/// `len` is the **total** frame size, header included, exactly as `RecordHeader.len` is, so the
/// consumer's `r = r.wrapping_add(len)` skip needs no special case for a PAD.
#[repr(C)]
#[derive(Clone, Copy)]
struct FrameHeader {
    /// Total bytes of this frame: `HEADER_BYTES + text.len()`, or the pad width for a PAD.
    len: u16,
    /// The site's printed code number; `0` when the level carries none.
    code: u16,
    /// The site's severity, or [`LEVEL_PAD`] to mark a PAD frame.
    level: u8,
    /// `TargetId::index()`, which `MAX_TARGETS == 256` makes exact in a `u8`.
    target: u8,
    /// The record's argument flags, carried through for the reader.
    flags: u8,
    _pad: u8,
}

/// Bytes of [`FrameHeader`].
const HEADER_BYTES: usize = core::mem::size_of::<FrameHeader>();
const _: () = assert!(HEADER_BYTES == 8);

/// The PAD sentinel, in the one field that has spare encodings.
///
/// [`Level`] occupies `0..=5`, so `0xFF` is not a level any site can carry. `LogLane` uses a null
/// site pointer for the same purpose; there is no pointer here, so the sentinel moves to the
/// field that can hold one without widening the header.
const LEVEL_PAD: u8 = 0xFF;

/// The longest line this ring will accept. A longer one is truncated by the producer *before* it
/// gets here (the drain renders into a fixed buffer), so this is a structural bound rather than a
/// policy: it is what keeps `len` inside its `u16` and a frame inside a fresh ring.
pub const MAX_LINE_BYTES: usize = 1024;
const _: () = assert!(
    HANDOFF_BYTES == 0 || (MAX_LINE_BYTES + HEADER_BYTES) < CAPACITY as usize,
    "a maximal frame must fit in an empty ring, or the largest lines could never be admitted"
);

/// The SPSC byte ring itself: three cache-line partitions plus the payload.
#[repr(C, align(64))]
pub(crate) struct HandoffRing {
    // ── line 0: PRODUCER-owned ───────────────────────────────────────────────
    /// Bytes ever published. Wraps at 2³²; only differences are ever taken.
    write: AtomicU32,
    _p0: [u8; 60],

    // ── line 1: CONSUMER-owned ───────────────────────────────────────────────
    /// Bytes copied into `LogRing`. `Release`-stored **after** the copy, never before.
    read: AtomicU32,
    _p1: [u8; 60],

    // ── line 2: LOSS ─────────────────────────────────────────────────────────
    /// Frames refused for want of space, as [`LossClass::Sink`].
    lost: AtomicU64,
    /// Payload bytes those frames carried.
    lost_bytes: AtomicU64,
    _p2: [u8; 48],

    // ── payload ──────────────────────────────────────────────────────────────
    buf: HandoffBuf,
}

/// The ring payload. A newtype so the `MaybeUninit` array's `const` initialiser stays readable.
#[repr(transparent)]
struct HandoffBuf(UnsafeCell<[MaybeUninit<u8>; HANDOFF_BYTES]>);

impl HandoffRing {
    const fn new() -> HandoffRing {
        HandoffRing {
            write: AtomicU32::new(0),
            _p0: [0; 60],
            read: AtomicU32::new(0),
            _p1: [0; 60],
            lost: AtomicU64::new(0),
            lost_bytes: AtomicU64::new(0),
            _p2: [0; 48],
            buf: HandoffBuf(UnsafeCell::new([const { MaybeUninit::uninit() }; HANDOFF_BYTES])),
        }
    }
}

const _: () = assert!(core::mem::align_of::<HandoffRing>() == 64);
const _: () = assert!(core::mem::offset_of!(HandoffRing, read) == 64);
const _: () = assert!(core::mem::offset_of!(HandoffRing, lost) == 128);
const _: () = assert!(core::mem::offset_of!(HandoffRing, buf) == 192);

// SAFETY (manual `Sync` for `HandoffRing`):
//   1. WRITE side: only the holder of `DRAIN_OWNER` writes `buf` or `write`. That role is a
//      single CAS'd token (`crate::drain_owner`), so two producers is unrepresentable -- the same
//      clause `LogLane`'s block makes about lane ownership, one level up. `push` is `pub(crate)`
//      and takes `&DrainToken`, so the bound is checked by the compiler rather than by a comment.
//   2. READ side: only the ECS's `log_drain_system` reads `buf` and writes `read`, and the
//      scheduler grants it `ResMut<LogRing>` exclusively, so two consumers is unrepresentable.
//      `drain_into` is `pub` because the consumer lives in another crate; what makes it safe is
//      not privacy but that a second caller would need a second `&mut LogRing`, which the
//      scheduler's conflict analysis refuses to hand out.
//   3. Visibility: bytes written before `write.store(_, Release)` are visible to the reader's
//      `Acquire`. The reader never reads past its observed `w`, and never advances `read` over
//      bytes it has not copied out.
//   4. No pointers cross. The payload is UTF-8 the producer already formatted, so `LogLane`'s
//      provenance clause -- the one about a `&'static LogSite` surviving the trip -- has no
//      analogue here and is not silently inherited.
unsafe impl Sync for HandoffRing {}

/// The ring. `.bss`, never freed, address-stable for the process.
static ECS_HANDOFF: HandoffRing = HandoffRing::new();

/// What one frame carries, apart from its text.
///
/// A struct rather than four positional arguments because the three `u8`s are mutually
/// swappable at a call site and the compiler would not object.
#[derive(Clone, Copy)]
pub struct FrameMeta {
    /// The site's severity.
    pub level: Level,
    /// `TargetId::index()` — see [`FrameHeader::target`].
    pub target: u8,
    /// The site's printed code number; `0` when the level carries none.
    pub code: u16,
    /// The record's argument flags.
    pub flags: u8,
}

/// Push one formatted line. Returns `false` when the ring refused it.
///
/// **A refusal is counted, never silent** — [`LossClass::Sink`] on the caller's row, plus the
/// ring's own `lost`/`lost_bytes` pair, which is what lets the ECS drain report a total the
/// substrate's per-lane fold cannot (the loss happened on the consumer's thread, not the
/// emitter's).
///
/// Taking `&DrainToken` is the whole single-producer argument: the token is unforgeable and there
/// is exactly one.
pub(crate) fn push(_token: &crate::drain_owner::DrainToken, meta: FrameMeta, text: &[u8]) -> bool {
    if CAPACITY == 0 {
        // The `Off`-ceiling build. Const-folded away, and unreachable besides: nothing can be
        // emitted in that build, so nothing can be drained into here.
        return false;
    }
    let text = &text[..text.len().min(MAX_LINE_BYTES)];
    let need = (HEADER_BYTES + text.len()) as u32;

    let ring = &ECS_HANDOFF;
    let w = ring.write.load(Ordering::Relaxed);
    let off = w & MASK;
    let tail = HANDOFF_BYTES as u32 - off;

    // The wrap rule, shared verbatim with the consumer: frames never straddle the end. A tail too
    // short for a header wraps implicitly; a tail long enough for a header but not for the frame
    // carries an explicit PAD.
    let pad = if tail < HEADER_BYTES as u32 || tail < need { tail } else { 0 };

    // The one `Acquire` on the consumer's cursor. Unlike `LogLane` this ring keeps no cached copy:
    // its producer runs once per drain pass, not once per record, so the cache would save one load
    // per frame at the cost of a fourth cursor to reason about.
    let r = ring.read.load(Ordering::Acquire);
    let used = w.wrapping_sub(r);
    debug_assert!(used <= CAPACITY, "invariant: the producer never publishes past CAPACITY");
    if CAPACITY - used < pad + need {
        ring.lost.fetch_add(1, Ordering::Relaxed);
        ring.lost_bytes.fetch_add(text.len() as u64, Ordering::Relaxed);
        record_here(LossClass::Sink, text.len() as u64);
        return false;
    }

    if pad >= HEADER_BYTES as u32 {
        let hdr = FrameHeader {
            len: pad as u16,
            code: 0,
            level: LEVEL_PAD,
            target: 0,
            flags: 0,
            _pad: 0,
        };
        // SAFETY: `off + pad == HANDOFF_BYTES` because `pad == tail`, and `pad >= HEADER_BYTES` on
        //   this branch, so the header lies wholly inside the tail. The admission check above
        //   proved `pad + need <= CAPACITY - used`, so these bytes hold nothing the consumer has
        //   yet to copy. This thread holds the drain token, so it is the only producer.
        unsafe { write_header(ring, off, &hdr) };
    }

    let w = w.wrapping_add(pad);
    let off = w & MASK;
    let hdr = FrameHeader {
        len: need as u16,
        code: meta.code,
        level: meta.level as u8,
        target: meta.target,
        flags: meta.flags,
        _pad: 0,
    };
    // SAFETY: after the pad, either `off == 0` or `tail >= need`, so the header and its text
    //   occupy `off .. off + need` without wrapping. The admission check proved that span holds no
    //   byte the consumer has yet to copy, and the drain token proves this is the only producer.
    unsafe {
        write_header(ring, off, &hdr);
        let dst = ring.buf.0.get().cast::<u8>().add(off as usize + HEADER_BYTES);
        core::ptr::copy_nonoverlapping(text.as_ptr(), dst, text.len());
    }

    // Publishes every byte above to a consumer that loads `write` with `Acquire`.
    ring.write.store(w.wrapping_add(need), Ordering::Release);
    true
}

/// Write `hdr` at `off`, unaligned.
///
/// # Safety
///
/// `off + HEADER_BYTES <= HANDOFF_BYTES`, the span holds no byte the consumer has yet to copy,
/// and the caller holds the drain token.
#[inline]
unsafe fn write_header(ring: &HandoffRing, off: u32, hdr: &FrameHeader) {
    // SAFETY: forwarded from the caller's contract -- the span is inside the buffer and is not
    //   live for the consumer. `write_unaligned` needs no alignment guarantee, which is what lets
    //   a frame start at any byte offset.
    unsafe { ring.buf.0.get().cast::<u8>().add(off as usize).cast::<FrameHeader>().write_unaligned(*hdr) }
}

/// What one frame looked like to the consumer.
#[derive(Clone, Copy)]
pub struct Frame<'a> {
    /// The site's severity, as its raw discriminant.
    pub level: u8,
    /// `TargetId::index()`, narrowed.
    pub target: u8,
    /// The site's printed code number.
    pub code: u16,
    /// The record's argument flags.
    pub flags: u8,
    /// The formatted line.
    pub text: &'a [u8],
}

/// Frames refused for want of space, and the payload bytes they carried.
///
/// Cumulative for the process. A reader takes differences; storing a difference here would make
/// two consumers disagree, and there is deliberately only ever one.
/// `boyko-W0117`: this drain pass refused frames to the in-frame view.
///
/// **One per DRAIN, carrying the pass's count** -- not one per refusal. A formatting storm refuses
/// thousands, and a report per refusal would be the storm again in a second channel. The count is
/// the delta over the pass, so consecutive reports add up to the total rather than each restating
/// it.
///
/// The byte sinks already have every one of these records: only the IN-FRAME view is short. That
/// is why this is a `W` and not an `E`, and why it says so -- a reader who sees a gap in a console
/// widget needs to know the file is complete.
#[cold]
#[inline(never)]
pub fn report_overflow(frames: u64, bytes: u64) {
    crate::warn!(
        crate::Log,
        crate::codes::W0117,
        "the ECS handoff refused {} frames ({} bytes) this pass; the in-frame view is short and          the byte sinks are not",
        frames,
        bytes
    );
}

#[must_use]
pub fn lost() -> (u64, u64) {
    (ECS_HANDOFF.lost.load(Ordering::Relaxed), ECS_HANDOFF.lost_bytes.load(Ordering::Relaxed))
}

/// Bytes ever published into the ring, including PADs. Wraps at 2³²; only differences are
/// meaningful.
///
/// Exists so "the ring was never touched" is a question a test can ask. A count of frames would
/// not answer it — a PAD is not a frame, and a producer that wrote only PADs has still touched the
/// pages the flag-off claim says stay untouched.
#[must_use]
pub fn published() -> u32 {
    ECS_HANDOFF.write.load(Ordering::Relaxed)
}

/// Hand every published frame to `on_frame`, oldest first. Returns how many were handed over.
///
/// # The two properties this function exists to uphold
///
/// 1. **It never reads past its observed `write`.** One `Acquire` load fixes the horizon for the
///    pass; a frame published after it waits for the next one.
/// 2. **It never advances `read` over bytes it has not copied.** The cursor moves once, at the
///    end, to the value the walk actually reached.
///
/// # Safety
///
/// The caller must be the ring's single consumer. In this engine that is `log_drain_system`
/// holding `ResMut<LogRing>`, and the scheduler's exclusivity analysis is the proof — which is
/// why this is an `unsafe fn` rather than one taking a token: the token that proves it is a
/// borrow the compiler already checked in another crate, and inventing a second one here would
/// let a caller hold both and still be wrong.
pub unsafe fn drain_into(mut on_frame: impl FnMut(Frame<'_>)) -> u32 {
    if CAPACITY == 0 {
        return 0;
    }
    let ring = &ECS_HANDOFF;
    let w = ring.write.load(Ordering::Acquire);
    let mut r = ring.read.load(Ordering::Relaxed);
    let mut frames = 0u32;

    while r != w {
        let off = (r & MASK) as usize;
        // SAFETY: the producer never publishes a frame that would straddle the end of the ring --
        //   a tail too short for one carries a PAD instead -- so a header at `off` is wholly
        //   inside the buffer whenever `r != w`. The caller is the only consumer, and the
        //   `Acquire` on `write` above makes every byte written before that publication visible.
        let hdr: FrameHeader = unsafe {
            ring.buf.0.get().cast::<u8>().add(off).cast::<FrameHeader>().read_unaligned()
        };
        let len = u32::from(hdr.len);
        debug_assert!(len as usize >= HEADER_BYTES, "invariant: a frame includes its header");
        debug_assert!(len <= w.wrapping_sub(r), "invariant: a frame ends at or before the horizon");

        if hdr.level != LEVEL_PAD {
            let text_len = len as usize - HEADER_BYTES;
            // SAFETY: the text follows its header contiguously for `len - HEADER_BYTES` bytes,
            //   inside the same non-straddling span, and was written by `copy_nonoverlapping` from
            //   a `&[u8]` before the `Release` this walk's `Acquire` paired with.
            let text = unsafe {
                let base = ring.buf.0.get().cast::<u8>().add(off + HEADER_BYTES);
                core::slice::from_raw_parts(base, text_len)
            };
            on_frame(Frame {
                level: hdr.level,
                target: hdr.target,
                code: hdr.code,
                flags: hdr.flags,
                text,
            });
            frames += 1;
        }
        r = r.wrapping_add(len);
    }

    // Published only after every byte above has been copied. Moving this inside the loop would let
    // the producer overwrite a frame between the callback and the store.
    ring.read.store(r, Ordering::Release);
    frames
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The consumer's skip arithmetic must not depend on the header's alignment.
    ///
    /// A frame's text is an arbitrary length, so the next header lands at an arbitrary offset.
    /// Both sides use `*_unaligned`, and this pins the size the arithmetic assumes.
    #[test]
    fn the_frame_header_is_eight_bytes() {
        assert_eq!(core::mem::size_of::<FrameHeader>(), 8);
    }

    /// A full ring refuses, counts what it refused, and loses nothing it had already admitted.
    ///
    /// This is the only rung at which the refusal path is reachable at all — the seam's `W0117`
    /// report and the `lossy` bit are L16 — so it is exercised here rather than left to be
    /// discovered by the first frame that formats more than a ring's worth.
    ///
    /// Takes the process-wide test lock: there is exactly one drain token, so there must be
    /// exactly one serialization point over it.
    #[test]
    fn a_full_ring_refuses_and_keeps_what_it_admitted() {
        let _serial = crate::drain_owner::test_serial();
        let token = crate::drain_owner::try_claim().expect("the drain role is free");

        // Start from a known cursor: another test in this binary may have left frames behind.
        let _ = unsafe { drain_into(|_| {}) };
        let (lost0, bytes0) = lost();

        let meta = FrameMeta { level: Level::Info, target: 7, code: 42, flags: 0 };
        let line = [b'y'; 200];

        let mut admitted = 0u32;
        while push(&token, meta, &line) {
            admitted += 1;
            assert!(admitted < 100_000, "a {HANDOFF_BYTES}-byte ring admitted 100 000 frames");
        }
        assert!(admitted > 0, "an empty ring refused the first frame");

        let (lost1, bytes1) = lost();
        assert_eq!(lost1, lost0 + 1, "the refusal was not counted");
        assert_eq!(bytes1, bytes0 + line.len() as u64, "the refused payload was not counted");

        // Every admitted frame is still readable and still its own: a refusal must never cost a
        // frame the ring had already published, which is the difference between "the view is
        // short" and "the transport is lossy".
        let mut seen = 0u32;
        unsafe {
            drain_into(|f| {
                assert_eq!(f.text, &line[..]);
                assert_eq!(f.target, 7);
                assert_eq!(f.code, 42);
                assert_eq!(f.level, Level::Info as u8);
                seen += 1;
            });
        }
        assert_eq!(seen, admitted, "a full ring lost frames it had admitted");

        // And the drain freed the space it copied out.
        assert!(push(&token, meta, &line), "a drained ring still refused");
        let _ = unsafe { drain_into(|_| {}) };
    }

    /// `LEVEL_PAD` must not collide with a level a site can carry.
    #[test]
    fn the_pad_sentinel_is_not_a_level() {
        for level in [Level::Off, Level::Error, Level::Warn, Level::Info, Level::Debug, Level::Trace]
        {
            assert_ne!(level as u8, LEVEL_PAD);
        }
    }
}
