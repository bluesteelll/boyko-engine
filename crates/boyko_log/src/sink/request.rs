//! Runtime sink control: a request ring the *sink thread* drains *(L14)*.
//!
//! # The contract is about which thread pays, not about the queue
//!
//! `open`ing a file is a syscall, and a console command that opens one **on the caller's thread**
//! blocks whatever that thread was doing — a frame, a physics step, an input poll. So a request is
//! posted into a fixed `.bss` ring and executed by the sink thread, and **no `open`, no allocation
//! and no syscall ever runs on the requesting thread**.
//!
//! A channel was rejected for the same reason it is rejected elsewhere in this crate: it is an
//! allocation, and usually a `Mutex`.
//!
//! # A full queue is `boyko-E0107`, never a silent drop
//!
//! Sixteen slots is generous for human-initiated control, so a full ring means requests are
//! arriving faster than the sink drains — which is a fact about the caller, not the sink, and the
//! caller is the one who can act on it. Dropping silently would leave an operator typing `open
//! /tmp/x.log` and seeing nothing happen, with the log they are trying to open being the place the
//! explanation would have gone.

use core::sync::atomic::{AtomicU32, AtomicU64, Ordering};

/// Slots in the request ring. `.bss`, fixed, never grown.
pub const SINK_REQ_LEN: usize = 16;

/// What a request asks the sink thread to do.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum SinkVerb {
    /// Open (or re-open) the file sink at the recorded path.
    OpenFile = 1,
    /// Close the file sink, flushing what it holds.
    CloseFile = 2,
    /// Re-read the control spec and apply it.
    ApplyControl = 3,
}

impl SinkVerb {
    /// The verb's name, for a record.
    ///
    /// A `&'static str` rather than `{:?}`: the record carries VALUES, and a `Debug` impl would
    /// have to run on the emitting thread inside the open-record window — which Decision 13
    /// forbids. The name is a literal the sink prints.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            SinkVerb::OpenFile => "open-file",
            SinkVerb::CloseFile => "close-file",
            SinkVerb::ApplyControl => "apply-control",
        }
    }

    #[must_use]
    const fn from_raw(b: u8) -> Option<SinkVerb> {
        match b {
            1 => Some(SinkVerb::OpenFile),
            2 => Some(SinkVerb::CloseFile),
            3 => Some(SinkVerb::ApplyControl),
            _ => None,
        }
    }
}

/// One posted request. `slot` selects which sink; `verb` says what to do with it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct SinkReq {
    /// Which sink the verb applies to.
    pub slot: u8,
    /// What to do.
    pub verb: SinkVerb,
}

/// Packed `(verb, slot)`; `0` means empty, which is why `SinkVerb` starts at 1.
static RING: [AtomicU32; SINK_REQ_LEN] = [const { AtomicU32::new(0) }; SINK_REQ_LEN];

/// Monotone write cursor. The reader's cursor is the sink thread's own local.
static WRITE: AtomicU64 = AtomicU64::new(0);
static READ: AtomicU64 = AtomicU64::new(0);


/// The one way [`post`] fails: the 16-slot ring is full.
///
/// A named type and not `()`, because the caller's two sensible responses differ -- retry after a
/// drain, or report `boyko-E0107` and give up -- and `Result<_, ()>` makes both of them a guess.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct RingFull;

/// Post a request, or `Err` when the ring is full.
///
/// **Allocation-free and syscall-free by construction**: one CAS on a cursor and one `Release`
/// store into a `.bss` cell. The caller can be a frame thread, an input callback, or a console
/// command handler, and none of them pays for the `open`.
///
/// `Err(())` is the `E0107` condition. It is returned rather than reported here because the caller
/// knows what it was trying to do and this module does not — a refusal that names the verb is worth
/// more than one that says "a request was dropped".
pub fn post(req: SinkReq) -> Result<(), RingFull> {
    let packed = (u32::from(req.verb as u8) << 8) | u32::from(req.slot);
    loop {
        let w = WRITE.load(Ordering::Acquire);
        let r = READ.load(Ordering::Acquire);
        if w.wrapping_sub(r) >= SINK_REQ_LEN as u64 {
            return Err(RingFull);
        }
        if WRITE
            .compare_exchange(w, w + 1, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            RING[(w as usize) % SINK_REQ_LEN].store(packed, Ordering::Release);
            return Ok(());
        }
    }
}

/// Take the next request, or `None` when the ring is empty.
///
/// The sink thread is the only caller; a request whose cell has not yet been published is treated
/// as **not yet posted** rather than skipped — the writer bumped the cursor before storing, so a
/// zero cell means "in flight", and consuming past it would reorder two operators' commands.
pub fn take() -> Option<SinkReq> {
    let r = READ.load(Ordering::Acquire);
    if r >= WRITE.load(Ordering::Acquire) {
        return None;
    }
    let cell = &RING[(r as usize) % SINK_REQ_LEN];
    let packed = cell.load(Ordering::Acquire);
    if packed == 0 {
        return None;
    }
    cell.store(0, Ordering::Release);
    READ.store(r + 1, Ordering::Release);
    SinkVerb::from_raw((packed >> 8) as u8).map(|verb| SinkReq { slot: packed as u8, verb })
}

/// How many requests are posted and undrained.
#[must_use]
pub fn depth() -> usize {
    WRITE
        .load(Ordering::Acquire)
        .wrapping_sub(READ.load(Ordering::Acquire)) as usize
}

/// Drain the ring. Test and console surface, `pub` for the reason `sample::reset_counters` is.
pub fn clear() {
    while take().is_some() {}
}

/// `boyko-E0107`: a sink control request was refused because the ring is full.
///
/// Emitted by the CALLER, which knows the verb it was posting. `Every`, not `Once`: an operator who
/// types three commands and has two refused needs to know which two, and a latch would name one.
pub fn report_refused(verb: SinkVerb, slot: u8) {
    crate::error!(
        crate::Log,
        crate::codes::E0107,
        "sink control request {} for slot {} was refused: {} requests are already queued",
        verb.name(),
        slot,
        depth()
    );
}
