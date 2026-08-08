//! The file sink — one handle, owned by whoever holds the consumer role.
//!
//! # Why the handle is a `static` and not a field
//!
//! The consumer role moves: it is the sink thread by default, a host calling
//! [`drain`](crate::lifecycle::drain) under `SinkMode::Manual`, the ECS drain under `Scheduled`,
//! and the crash drainer during a panic. All four take the **same** CAS'd `DRAIN_OWNER` token, so
//! "the thread that may touch this handle" is a role rather than a thread — and a role cannot own
//! a field on a thread's stack. The handle therefore lives beside the other consumer-role scratch,
//! reachable only through a `&DrainToken` the compiler checks.
//!
//! # The cap is not rotation
//!
//! When the file reaches its byte cap this sink **stops writing and says so, once**
//! (`boyko-W0103`). It does not truncate, does not delete, and does not roll over — rotation is a
//! later rung, and a sink that silently discarded the beginning of a capture in order to keep
//! writing would destroy exactly the records that explain the ones it kept.

use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::io::Write;

use crate::codes::{OnceSite, W0103};
use crate::drain_owner::DrainToken;

/// The longest path this sink will record. Fixed, because a path recorded at boot may not
/// allocate — `boot` is a pure struct-fill and this is the buffer it fills.
pub const MAX_PATH_BYTES: usize = 256;

/// The recorded destination. Written by [`set_path`], read once by [`open`].
struct PathSlot(UnsafeCell<[u8; MAX_PATH_BYTES]>);

// SAFETY: written only by `set_path`, which is documented single-threaded setup ("before
//   `enable()`, on the host thread") and asserts nothing else; read only by `open`, which runs on
//   the enable path after that. `PATH_LEN`'s `Release`/`Acquire` pair orders the bytes against the
//   read, so a host that violates the setup rule still cannot observe a torn path — it observes
//   either the old length or the new one.
unsafe impl Sync for PathSlot {}

static PATH: PathSlot = PathSlot(UnsafeCell::new([0; MAX_PATH_BYTES]));
static PATH_LEN: AtomicU64 = AtomicU64::new(0);

/// The open handle. `None` until [`open`] succeeds.
struct FileSlot(UnsafeCell<Option<std::fs::File>>);

// SAFETY: `open` runs on the enable path, before any consumer role exists — there is no drain
//   token in the process yet, so no reader can be inside `write_line`. Every subsequent access is
//   through `&DrainToken`, and the token is a single CAS'd role (`crate::drain_owner`), so two
//   threads inside this cell is unrepresentable. `close` takes the token for the same reason.
unsafe impl Sync for FileSlot {}

static FILE: FileSlot = FileSlot(UnsafeCell::new(None));

/// Bytes written to the file so far.
static WRITTEN: AtomicU64 = AtomicU64::new(0);

/// The cap in bytes; `0` means uncapped.
static CAP: AtomicU64 = AtomicU64::new(0);

/// Set once the cap has been reached, so the check is a load rather than a comparison chain.
static CAPPED: AtomicBool = AtomicBool::new(false);

/// The `W0103` latch. Per site in the sense that matters: there is exactly one site.
static CAP_REPORTED: OnceSite = OnceSite::new();

/// Record the destination. **Opens nothing.**
///
/// Call before [`enable`](crate::lifecycle::enable), on the host thread. Returns `false` for a
/// path longer than [`MAX_PATH_BYTES`], which is a refusal rather than a truncation: a truncated
/// path names a different file, and writing a log to the wrong file is worse than not writing one.
pub fn set_path(path: &str) -> bool {
    let bytes = path.as_bytes();
    if bytes.is_empty() || bytes.len() > MAX_PATH_BYTES {
        return false;
    }
    // SAFETY: setup-time, single-threaded by this function's contract; the `Release` below
    //   publishes these bytes to `open`'s `Acquire`.
    unsafe {
        let dst = PATH.0.get().cast::<u8>();
        core::ptr::copy_nonoverlapping(bytes.as_ptr(), dst, bytes.len());
    }
    PATH_LEN.store(bytes.len() as u64, Ordering::Release);
    true
}

/// Whether a destination has been recorded.
#[must_use]
pub fn path_recorded() -> bool {
    PATH_LEN.load(Ordering::Acquire) != 0
}

/// Open the recorded destination, truncating any previous contents. Returns `false` when no path
/// was recorded or the OS refused.
///
/// Runs on the **enable** path and nowhere else: opening a file is a syscall, and a syscall the
/// runtime flag has not authorised is exactly what boot may not do.
///
/// A refusal is not a launch failure. The synchronous channel and the rings still work, and the
/// host learns from the return value rather than from a missing file it has to notice.
pub fn open(cap_bytes: u64) -> bool {
    let len = PATH_LEN.load(Ordering::Acquire) as usize;
    if len == 0 {
        return false;
    }
    // SAFETY: `PATH_LEN`'s `Acquire` pairs with `set_path`'s `Release`, so these `len` bytes are
    //   the ones that call wrote. No consumer role exists yet (see the `FileSlot` block), so no
    //   other thread is inside this cell.
    let path = unsafe {
        let src = core::slice::from_raw_parts(PATH.0.get().cast::<u8>(), len);
        match core::str::from_utf8(src) {
            Ok(s) => s,
            Err(_) => return false,
        }
    };
    let Ok(file) = std::fs::File::create(path) else { return false };

    CAP.store(cap_bytes, Ordering::Relaxed);
    WRITTEN.store(0, Ordering::Relaxed);
    CAPPED.store(false, Ordering::Relaxed);
    // SAFETY: as above — the enable path runs before any drain token exists.
    unsafe { *FILE.0.get() = Some(file) };
    true
}

/// Append one formatted line, with a trailing newline. Returns `false` when nothing was written.
///
/// Taking `&DrainToken` is the whole exclusivity argument: the token is unforgeable and there is
/// exactly one, so this cell has one writer by construction rather than by convention.
pub(crate) fn write_line(_token: &DrainToken, text: &[u8]) -> bool {
    if CAPPED.load(Ordering::Relaxed) {
        return false;
    }
    // SAFETY: the caller holds the drain token, which is a single CAS'd role, so no other thread
    //   is inside this cell. `open` has completed (it runs on the enable path, before any token
    //   can be claimed), so the `Option` is not being written concurrently.
    let slot = unsafe { &mut *FILE.0.get() };
    let Some(file) = slot.as_mut() else { return false };

    let cap = CAP.load(Ordering::Relaxed);
    let written = WRITTEN.load(Ordering::Relaxed);
    let need = text.len() as u64 + 1;
    if cap != 0 && written + need > cap {
        report_cap(cap);
        return false;
    }

    // Two `write_all`s rather than one staged copy: the line is already in a buffer the caller
    // owns, and copying it again to append one byte would double the cost of the common case to
    // save one syscall on a buffered handle that is going to coalesce them anyway.
    if file.write_all(text).is_err() || file.write_all(b"\n").is_err() {
        return false;
    }
    WRITTEN.store(written + need, Ordering::Relaxed);
    true
}

/// Report the cap, once, through the synchronous channel.
///
/// Deliberately **not** through the emission path: the condition is "this destination is full",
/// and a record about it would be routed to the destination that is full.
#[cold]
#[inline(never)]
fn report_cap(cap: u64) {
    CAPPED.store(true, Ordering::Relaxed);
    if !CAP_REPORTED.claim() {
        return;
    }
    let mut buf = [0u8; 96];
    let n = render_cap(&mut buf, cap);
    // SAFETY: `render_cap` writes only ASCII copied from `&'static str`s and decimal digits.
    let text = unsafe { core::str::from_utf8_unchecked(&buf[..n]) };
    crate::sync_out::write_oracle_line("boyko-W0103: ", text);
}

/// Render `file sink reached its N-byte cap; no further lines are written`.
fn render_cap(buf: &mut [u8], cap: u64) -> usize {
    let mut n = 0usize;
    let mut put = |s: &[u8], n: &mut usize| {
        let take = s.len().min(buf.len() - *n);
        buf[*n..*n + take].copy_from_slice(&s[..take]);
        *n += take;
    };
    put(b"file sink reached its ", &mut n);
    let mut d = [0u8; 20];
    let mut v = cap;
    let mut i = d.len();
    loop {
        i -= 1;
        d[i] = b'0' + (v % 10) as u8;
        v /= 10;
        if v == 0 || i == 0 {
            break;
        }
    }
    put(&d[i..], &mut n);
    put(b"-byte cap; no further lines are written", &mut n);
    n
}

/// Bytes written to the file, and whether the cap has stopped it.
#[must_use]
pub fn state() -> (u64, bool) {
    (WRITTEN.load(Ordering::Relaxed), CAPPED.load(Ordering::Relaxed))
}

/// Whether a file is currently open.
#[must_use]
pub fn is_open() -> bool {
    // SAFETY: a shared read of an `Option`'s discriminant. Every write happens either on the
    //   enable path (before a token exists) or under the token, and this function is documented as
    //   an observation rather than a synchronisation point — a caller racing `open` legitimately
    //   sees either answer.
    unsafe { (*FILE.0.get()).is_some() }
}

/// Flush and close the handle. Takes the token for the same reason [`write_line`] does.
pub(crate) fn close(_token: &DrainToken) {
    // SAFETY: the caller holds the single CAS'd drain role, so no other thread is inside the cell.
    let slot = unsafe { &mut *FILE.0.get() };
    if let Some(file) = slot.as_mut() {
        let _ = file.flush();
    }
    *slot = None;
}

/// The registry row this sink emits, named here so a reader of the emitter finds the row.
const _: () = {
    // `W0103` is `RatePolicy::Once`, and this sink honours that with its own site latch rather
    // than through `RATE` — which is what `Once` means, and what `crate::rate` refuses to do.
    let _ = W0103;
};
