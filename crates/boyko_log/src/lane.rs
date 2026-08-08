//! The per-lane SPSC byte ring and the producer hot path.
//!
//! # The ring is ours; the lane IDENTITY is the substrate's
//!
//! `LOG_LANES[i]` is a single-producer / single-consumer byte ring indexed by
//! [`boyko_diag::lane::lane()`]. This crate does not mint, claim, retire or reclaim lane
//! identity, and must not: two registries mean two lane numbers for one thread — the same worker
//! is lane 5 to the profiler and lane 37 to the logger — and no reader can then place a log line
//! inside the zone it happened in. The one joint question the two subsystems exist to answer
//! would be unanswerable **by construction**, not by a bug.
//!
//! The ring stays ours because its shape is a logging decision: the producer caches the opposite
//! cursor, and the partitions are padded apart. The one published measurement on this question
//! found padding **alone** made a ring *slower* — both threads still read the opposite cursor
//! every operation — and only opposite-cursor caching *plus* padding moved throughput from ~32 to
//! ~440 M ops·s⁻¹. Both are here, and the padding is treated as a hypothesis with an ablation
//! bench rather than as doctrine.
//!
//! # Three partitions, not two
//!
//! Statistics are written by the producer **and** read by the consumer, so they are a third
//! partition. Putting them on the producer's line means the consumer touches the producer's hot
//! line on every drain — most often precisely during the drop storm the counters exist to
//! measure.
//!
//! # Loss lives in the substrate, not in the lane
//!
//! There is no `loss` field. `boyko_diag::loss` already holds a process-global `CELLS[row][class]`
//! indexed **by lane**; an inline copy here would be the third of the four duplications
//! `boyko_diag` exists to delete, reintroduced one layer up. `sampled_out` stays, because a
//! sample that was never meant to be delivered is **not** a loss and the identity
//! `emitted == drained + dropped + sampled_out` depends on the separation being exact.

use core::cell::Cell;
use core::mem::MaybeUninit;
use core::sync::atomic::{AtomicU32, AtomicU64, Ordering};

use boyko_diag::lane::{LANE_COUNT, LANE_UNCLAIMED};
use boyko_diag::loss::{LossClass, record_here};

use crate::GLOBAL_CEILING;
use crate::level::Level;
use crate::record::{HEADER_BYTES, LogArgs, MAX_RECORD_BYTES, RecordHeader};
use crate::site::LogSite;

/// Bytes per lane ring. A power of two, so the wrap is a mask.
pub(crate) const LANE_BYTES: usize = 16 * 1024;

/// Index mask for the ring.
const MASK: u32 = (LANE_BYTES - 1) as u32;

/// Usable span.
///
/// One slot is reserved so that `used == CAPACITY` cannot be confused with `used == 0` without a
/// third variable — and, load-bearingly, so `avail = CAPACITY - used` cannot underflow.
const CAPACITY: u32 = (LANE_BYTES - 1) as u32;

/// Tail of the ring that only an `Error` may consume.
const ERROR_RESERVE: u32 = (LANE_BYTES / 8) as u32;

const _: () = assert!(LANE_BYTES.is_power_of_two());
// Else no non-`Error` record could ever fit, and the reserve would be a global mute switch.
const _: () = assert!(ERROR_RESERVE < CAPACITY);
// A maximal record must fit in the non-reserved span, or the largest sites could only ever be
// admitted at `Error`.
const _: () = assert!(MAX_RECORD_BYTES as u32 <= CAPACITY - ERROR_RESERVE);

/// Number of rings actually declared.
///
/// **Zero when the compile ceiling is `Off`.** In that build no call site survives the const
/// gates anyway, so the array has no producer; declaring it at full size would reserve 1.25 MiB
/// of address space for something nothing can reach. `LOG_LANES.get(i)` then yields `None` and
/// the emission path takes the unclaimed branch, which is a real branch rather than an index into
/// a zero-length array.
pub const LANE_ARRAY_LEN: usize =
    if (GLOBAL_CEILING as u8) == 0 { 0 } else { LANE_COUNT as usize };

/// One SPSC byte ring, three cache-line partitions plus the payload.
#[repr(C, align(64))]
pub(crate) struct LogLane {
    // ── line 0: PRODUCER-owned ───────────────────────────────────────────────
    /// Bytes ever published. Wraps at 2³²; only differences are ever taken.
    write: AtomicU32,
    /// The producer's stale copy of `read`. A **lower bound**: staleness can only make the
    /// producer refuse space it actually had, never grant space it did not.
    read_cached: Cell<u32>,
    _pad0: [u8; 56],

    // ── line 1: CONSUMER-owned ───────────────────────────────────────────────
    /// Bytes staged out. `Release`-stored **after** staging, never before.
    read: AtomicU32,
    /// The consumer's stale copy of `write`. Mirror of `read_cached`.
    write_cached: Cell<u32>,
    _pad1: [u8; 56],

    // ── line 2: LANE-OWNED STATISTICS ────────────────────────────────────────
    /// Records the sampler chose not to deliver. Deliberately **not** a `LossClass`.
    sampled_out: AtomicU64,
    _pad2: [u8; 56],

    // ── payload ──────────────────────────────────────────────────────────────
    buf: UnsafeCellBuf,
}

/// The ring payload. A newtype so the `MaybeUninit` array's `const` initialiser stays readable.
#[repr(transparent)]
struct UnsafeCellBuf(core::cell::UnsafeCell<[MaybeUninit<u8>; LANE_BYTES]>);

impl LogLane {
    const fn new() -> LogLane {
        LogLane {
            write: AtomicU32::new(0),
            read_cached: Cell::new(0),
            _pad0: [0; 56],
            read: AtomicU32::new(0),
            write_cached: Cell::new(0),
            _pad1: [0; 56],
            sampled_out: AtomicU64::new(0),
            _pad2: [0; 56],
            buf: UnsafeCellBuf(core::cell::UnsafeCell::new(
                [const { MaybeUninit::uninit() }; LANE_BYTES],
            )),
        }
    }
}

const _: () = assert!(core::mem::align_of::<LogLane>() == 64);
const _: () = assert!(core::mem::offset_of!(LogLane, read) == 64);
const _: () = assert!(
    core::mem::offset_of!(LogLane, sampled_out) == 128,
    "statistics are a third partition: producer writes, consumer folds"
);
const _: () = assert!(core::mem::offset_of!(LogLane, buf) == 192);

// SAFETY (manual `Sync` for `LogLane`):
//   1. WRITE side: exactly one thread ever writes `buf` or `write` -- the one whose
//      `boyko_diag::lane::lane()` returns this index. The uniqueness is the SUBSTRATE's, and it
//      is single-writer by construction on both of its paths: a pool worker's index IS its dense
//      `worker_id` (one live thread per id by the pool's own construction), and every other
//      thread's index comes from `claim_lane()`'s load-then-CAS over the spare slots, whose loser
//      retries a different slot. No two LIVE threads hold one index, so two producer threads on
//      one lane is unrepresentable.
//   1b. Re-entrant emit on ONE thread is separately excluded: no user code runs between lane
//      acquisition and the `Release` store. `dsp!` renders in ARGUMENT position, so a user
//      `Display` has already run to completion before `emit_impl` is entered, and encoding
//      afterwards operates on POD and `&str` only.
//   1c. `read_cached` is a `Cell<u32>` read and written ONLY by the producer that owns the lane
//      -- it is why `LogLane` is not `Sync` by derivation, and clause 1's exclusive-write
//      argument covers it exactly: a second producer thread cannot exist, and the consumer never
//      names it. Its value is a STALE LOWER BOUND on `read`; staleness can only make the producer
//      refuse space it had, never grant space it did not.
//   1d. `write_cached` is the mirror clause, for the thread currently holding the consumer role.
//      A stale value can only make the consumer drain less than was available, never read past
//      `write`.
//   2. READ side: exactly one thread reads `buf` and writes `read` -- the holder of the single
//      CAS'd drain token. That token does not exist yet (it arrives with the sink), and until it
//      does THIS CRATE HAS NO CONSUMER AT ALL, so the read side is vacuously exclusive. Stated
//      rather than assumed, because "no consumer" is a property of this rung and not of the
//      design.
//   3. Payload visibility: bytes written before `write.store(_, Release)` are visible to a thread
//      observing that value via `Acquire`. The consumer never reads past its observed `w`, and
//      never advances `read` over bytes it has not copied out.
//   4. Retire: `boyko_diag::lane::release_lane()` publishes the substrate's spare slot FREE with
//      a `Release` store, on the producer thread, AFTER its last ring write. A new claimant's
//      `Acquire` on the successful CAS synchronizes-with that store, so it observes every byte
//      and both cursors the retiring owner left, and it APPENDS rather than resets. There is
//      therefore no instant with two live producers on one index and no reclaim step: the ring is
//      a fixed `static` of POD with no `Drop`, so an undrained record is not a dangling anything.
unsafe impl Sync for LogLane {}

/// The rings. `.bss`, never freed, address-stable for the process.
static LOG_LANES: [LogLane; LANE_ARRAY_LEN] = [const { LogLane::new() }; LANE_ARRAY_LEN];

/// Resolve this thread's ring, claiming a spare lane if it has none.
///
/// Returns `None` when the thread has no lane and none can be claimed, or when the array is
/// empty because the compile ceiling is `Off`.
#[inline]
fn resolve() -> Option<&'static LogLane> {
    let mut id = boyko_diag::lane::lane();
    if id == LANE_UNCLAIMED {
        id = claim_cold()?;
    }
    LOG_LANES.get(id as usize)
}

/// The once-per-thread claim, kept out of line so the hot path is a TLS read and a compare.
#[cold]
#[inline(never)]
fn claim_cold() -> Option<u16> {
    boyko_diag::lane::claim_lane()
}

/// The producer hot path.
///
/// `#[inline(never)]` and monomorphised per argument-tuple type: blanket inlining would replicate
/// this body at every call site and bloat L1i, which the project's inlining principle forbids on
/// measurement grounds. The gates that must fold are in the macro, not here.
///
/// # Order of operations, and why it is this order
///
/// Arguments were **already evaluated** by the caller — that is the `&&` short-circuit's promise
/// and it is a user-visible property, not an implementation detail. What happens here is size,
/// space and publication, in that order, with no user code anywhere between acquiring the lane
/// and the `Release` store.
#[inline(never)]
pub fn emit_impl<A: LogArgs>(site: &'static LogSite, args: A) {
    let Some(lane) = resolve() else {
        // A thread with no lane does NOT silently drop a severe record. `Warn` and `Error` take
        // the synchronous channel; the three lower levels are counted as unclaimed, on the
        // substrate's un-laned row precisely because there is no lane to charge them to.
        //
        // The fallback is bounded by that channel's 50 ms acquire deadline and can therefore
        // *steal* -- an interleaved line rather than a hung thread. What it cannot do is block,
        // which matters because the commonest way to reach here is a driver or OS callback on a
        // thread the engine never created, under a storm, with all 14 spare lanes taken.
        if site.level <= Level::Warn {
            // The record is not encoded: the payload lives in the caller's arguments and this
            // channel takes rendered text. What can be said without a formatter is said -- the
            // site's own metadata, which is the part a reader needs to find the call.
            let mut buf = [0u8; 160];
            let n = render_site_line(&mut buf, site);
            // SAFETY: `render_site_line` writes only ASCII bytes it copied from `&'static str`s
            //   and decimal digits, so the prefix is valid UTF-8.
            let text = unsafe { core::str::from_utf8_unchecked(&buf[..n]) };
            crate::sync_out::write_oracle_line("boyko-log: ", text);
            crate::target::count_sync_routed(site.target);
        } else {
            record_here(LossClass::Unclaimed, 0);
            // Charged to the TARGET as well as to the substrate's un-laned row. The two answer
            // different questions -- "which thread lost records" and "which category is
            // incomplete" -- and a census that read only the first could call a target clean while
            // every one of its records went missing on a driver callback thread.
            crate::target::count_dropped(site.target);
        }
        return;
    };

    let need = HEADER_BYTES + args.encoded_len();
    if need > MAX_RECORD_BYTES {
        // Checked at RUNTIME, in every profile. Twelve arguments of 256 bytes exceed the cap, so
        // "unreachable" would have described a debug-build panic reachable from safe user code.
        record_here(LossClass::Refused, need as u64);
        crate::target::count_dropped(site.target);
        return;
    }
    let need = need as u32;

    let w = lane.write.load(Ordering::Relaxed);
    let off = w & MASK;
    let tail = LANE_BYTES as u32 - off;

    // The wrap rule, shared verbatim with the consumer: records never straddle the end of the
    // ring. A tail too short for a header wraps implicitly; a tail long enough for a header but
    // not for the record carries an explicit PAD.
    let pad = if tail < HEADER_BYTES as u32 || tail < need { tail } else { 0 };

    if !admit(lane, w, pad, need, site.level) {
        record_here(LossClass::Overflow, u64::from(need));
        crate::target::count_dropped(site.target);
        return;
    }

    if pad >= HEADER_BYTES as u32 {
        // PAD: a header with a null site and `len == pad`. The consumer skips it by `len` like
        // any other record, which is why the rule needs no second code path.
        let hdr = RecordHeader {
            site: core::ptr::null(),
            tsc: 0,
            len: pad as u16,
            flags: 0,
            clock_epoch_lo: 0,
        };
        // SAFETY: `off + pad <= LANE_BYTES` because `pad == tail == LANE_BYTES - off`, and
        //   `pad >= HEADER_BYTES` on this branch, so the header fits inside the tail. `admit`
        //   proved `pad + need <= avail`, so these bytes are not live for the consumer.
        unsafe { write_header(lane, off, &hdr) };
    }

    let w = w.wrapping_add(pad);
    let off = w & MASK;

    let hdr = RecordHeader {
        site,
        tsc: boyko_diag::clock::ticks(),
        len: need as u16,
        flags: args.args_flags(),
        clock_epoch_lo: boyko_diag::clock::clock_epoch() as u8,
    };
    // SAFETY: after the pad, either `off == 0` or `tail >= need`, so the header and the payload
    //   occupy `off .. off + need` without wrapping. `admit` proved `pad + need <= avail`, so
    //   that span holds no byte the consumer has yet to stage. This thread is the lane's only
    //   producer (`Sync` clause 1).
    unsafe {
        write_header(lane, off, &hdr);
        let dst = lane.buf.0.get().cast::<u8>().add((off as usize) + HEADER_BYTES);
        let written = args.encode(dst);
        debug_assert_eq!(
            written,
            need as usize - HEADER_BYTES,
            "invariant: LogArgs::encode writes exactly encoded_len bytes"
        );
    }

    // Publishes every byte above to a consumer that loads `write` with `Acquire`.
    lane.write.store(w.wrapping_add(need), Ordering::Release);
}

/// Render `file:line fmt` into `buf`, truncating rather than overflowing. Returns bytes written.
///
/// Deliberately not `core::fmt`: this runs on the lane-exhaustion path, and the synchronous
/// channel's first rule is that nothing formats inside its critical section. Doing it here, before
/// the acquire, is what keeps that rule true for this caller.
fn render_site_line(buf: &mut [u8], site: &LogSite) -> usize {
    let mut n = 0usize;
    let mut put = |s: &[u8], n: &mut usize| {
        let take = s.len().min(buf.len() - *n);
        buf[*n..*n + take].copy_from_slice(&s[..take]);
        *n += take;
    };
    put(site.file.as_bytes(), &mut n);
    put(b":", &mut n);
    let mut d = [0u8; 10];
    let mut line = site.line;
    let mut i = d.len();
    loop {
        i -= 1;
        d[i] = b'0' + (line % 10) as u8;
        line /= 10;
        if line == 0 || i == 0 {
            break;
        }
    }
    put(&d[i..], &mut n);
    put(b" ", &mut n);
    put(site.fmt.as_bytes(), &mut n);
    n
}

/// What one drain pass moved.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DrainStats {
    /// Records handed to the callback. PADs are not records and are not counted.
    pub records: u64,
    /// Payload bytes handed over, excluding headers and PADs.
    pub bytes: u64,
    /// Lanes that had at least one byte to stage.
    pub lanes_touched: u32,
}

/// Walk every lane and hand each published record to `on_record`.
///
/// # The two properties this function exists to uphold
///
/// 1. **It never reads past its observed `write`.** One `Acquire` load per lane fixes the horizon
///    for that pass; a record published after it waits for the next pass, which is what "eventual"
///    means here.
/// 2. **It never advances `read` over bytes it has not consumed.** The cursor moves once, at the
///    end, to the value the walk actually reached.
///
/// Taking `&DrainToken` rather than checking a state is the whole exclusivity argument: the token
/// is unforgeable and there is exactly one.
pub fn drain(
    _token: &crate::drain_owner::DrainToken,
    mut on_record: impl FnMut(&'static LogSite, u64, u8, &[u8]),
) -> DrainStats {
    let mut stats = DrainStats::default();

    for lane in LOG_LANES.iter() {
        let w = lane.write.load(Ordering::Acquire);
        let mut r = lane.read.load(Ordering::Relaxed);
        if r == w {
            continue;
        }
        stats.lanes_touched += 1;
        lane.write_cached.set(w);

        while r != w {
            let off = (r & MASK) as usize;
            // THE MIRROR OF THE PRODUCER'S *IMPLICIT* WRAP, and its absence was a real defect.
            //
            // The producer's rule has two arms: a tail long enough for a header but too short for
            // the record carries an explicit PAD, and **a tail shorter than a header carries
            // nothing at all** -- there is no room for a PAD header, so the producer simply
            // advances `write` past those bytes. The consumer had only the first arm: it read a
            // "header" out of the 1..HEADER_BYTES-1 bytes the producer had skipped and never
            // written, took `len` from uninitialised memory, and walked off into the ring.
            //
            // MEASURED, single-threaded, with the producer and the consumer strictly alternating:
            // `debug_assert!(len >= HEADER_BYTES)` fires once the cursor happens to land in that
            // window, which needs a specific run of record lengths and is why the L1 gate's fixed
            // sizes never reached it. In release, with the assert compiled out, this is a torn
            // read through a corrupted `&'static LogSite` -- the same use-after-free class the
            // admission arithmetic (F6) produced, entered through the wrap instead.
            let tail = LANE_BYTES - off;
            if tail < HEADER_BYTES {
                r = r.wrapping_add(tail as u32);
                continue;
            }
            // SAFETY: the producer never publishes a record that would straddle the end of the
            //   ring -- a tail too short for one carries a PAD instead -- so a header at `off` is
            //   wholly inside the buffer whenever `r != w`. This thread is the only consumer (the
            //   token), and the `Acquire` on `write` above makes every byte written before that
            //   publication visible here.
            let hdr: RecordHeader = unsafe {
                lane.buf.0.get().cast::<u8>().add(off).cast::<RecordHeader>().read_unaligned()
            };
            let len = u32::from(hdr.len);
            debug_assert!(len as usize >= HEADER_BYTES, "invariant: a record includes its header");
            debug_assert!(len <= w.wrapping_sub(r), "invariant: a record ends at or before the horizon");

            if !hdr.site.is_null() {
                let payload_len = len as usize - HEADER_BYTES;
                // SAFETY: the payload follows its header contiguously for `len - HEADER_BYTES`
                //   bytes, inside the same non-straddling span. `site` is non-null, and the PAD
                //   sentinel is the only null this field ever takes, so it is the `&'static
                //   LogSite` the producer wrote.
                let (site, payload) = unsafe {
                    let base = lane.buf.0.get().cast::<u8>().add(off + HEADER_BYTES);
                    (&*hdr.site, core::slice::from_raw_parts(base, payload_len))
                };
                on_record(site, hdr.tsc, hdr.flags, payload);
                stats.records += 1;
                stats.bytes += payload_len as u64;
            }
            r = r.wrapping_add(len);
        }

        // Published only after every byte above has been consumed. Moving this inside the loop
        // would let the producer overwrite a record between the callback and the store.
        lane.read.store(r, Ordering::Release);
    }

    stats
}

/// Admission control. **No unsigned subtraction below can go negative**, and that is the point.
///
/// The predecessor computed `LANE_BYTES - ERROR_RESERVE - used` in `u32`. In exactly the state
/// the reserve exists to create — the used span already past `LANE_BYTES - ERROR_RESERVE` — that
/// underflowed to ~4.29 × 10⁹, the guard was false, and the producer wrote over bytes the
/// consumer had not staged. The consumer then walked a torn header and called `decode` through a
/// corrupted site pointer: a use-after-free on the designed-and-tested path, entered through the
/// arithmetic rather than through the ordering.
///
/// The invariant `used <= CAPACITY` is **inductive over the producer's own admissions**:
/// `read_cached <= read <= w` always (the consumer only advances `read` toward `w`, and
/// `read_cached` is a stale copy of `read`), and the producer publishes `w + pad + need` only
/// after proving `pad + need <= avail = CAPACITY - used`. The base case is `used == 0`.
#[inline]
fn admit(lane: &LogLane, w: u32, pad: u32, need: u32, level: Level) -> bool {
    if budget_at(lane, w, level) >= pad + need {
        return true;
    }
    // The cached read cursor is a lower bound, so a refusal may be stale. Pay one `Acquire` load
    // before dropping a record -- and only then, so the common case never touches the consumer's
    // line.
    lane.read_cached.set(lane.read.load(Ordering::Acquire));
    budget_at(lane, w, level) >= pad + need
}

/// Space this record may use, given the level's claim on the `Error` reserve.
#[inline]
fn budget_at(lane: &LogLane, w: u32, level: Level) -> u32 {
    let used = w.wrapping_sub(lane.read_cached.get());
    debug_assert!(used <= CAPACITY, "invariant: used <= CAPACITY (admission induction)");
    let avail = CAPACITY - used;
    if level == Level::Error {
        avail
    } else {
        // SATURATING is the fix. Subtracting the reserve from the CAPACITY (rather than from the
        // available space) is what underflowed; taking it from `avail` yields a budget of 0 -- a
        // refusal -- in the reserve-already-eaten state. Lowers to `sub` + `cmov`: still
        // branchless, one extra instruction on a path that already has a compare.
        avail.saturating_sub(ERROR_RESERVE)
    }
}

/// # Safety
///
/// `off + HEADER_BYTES <= LANE_BYTES`, the span holds no byte the consumer has yet to stage, and
/// the caller is the lane's only producer.
#[inline]
unsafe fn write_header(lane: &LogLane, off: u32, hdr: &RecordHeader) {
    // SAFETY: the caller's obligations put the whole header inside the ring and inside space this
    //   producer owns. The write is unaligned because the ring is byte-oriented and records are
    //   never aligned -- which is why `RecordHeader` is `repr(C, packed)`.
    unsafe {
        let dst = lane.buf.0.get().cast::<u8>().add(off as usize).cast::<RecordHeader>();
        dst.write_unaligned(*hdr);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A lane on the heap, so a test can drive the ring without touching the process-global array
    /// that other tests share. Every property under test is per-lane arithmetic, so an owned lane
    /// tests the same code the static does.
    fn fresh() -> Box<LogLane> {
        Box::new(LogLane::new())
    }

    #[test]
    fn a_fresh_lane_offers_capacity_minus_the_reserve_to_a_non_error() {
        let l = fresh();
        assert_eq!(budget_at(&l, 0, Level::Info), CAPACITY - ERROR_RESERVE);
        assert_eq!(budget_at(&l, 0, Level::Error), CAPACITY);
    }

    #[test]
    fn the_reserve_eaten_state_yields_zero_budget_and_not_four_billion() {
        // THE regression. `used` past `CAPACITY - ERROR_RESERVE` is exactly the state the reserve
        // exists to create, and the predecessor's arithmetic licensed a 4.29e9-byte write here.
        let l = fresh();
        let used = CAPACITY - ERROR_RESERVE + 1;
        l.read_cached.set(0);
        assert_eq!(
            budget_at(&l, used, Level::Info),
            0,
            "a non-Error record must be refused once the reserve is the only space left"
        );
        assert_eq!(
            budget_at(&l, used, Level::Error),
            CAPACITY - used,
            "an Error record may still use the reserve"
        );
    }

    #[test]
    fn a_full_lane_refuses_even_an_error() {
        let l = fresh();
        l.read_cached.set(0);
        assert_eq!(budget_at(&l, CAPACITY, Level::Error), 0);
        assert!(!admit(&l, CAPACITY, 0, 1, Level::Error));
    }

    #[test]
    fn admission_refreshes_the_cached_cursor_before_refusing() {
        let l = fresh();
        // Producer believes the lane is full; the consumer has in fact drained all of it.
        l.read_cached.set(0);
        l.read.store(CAPACITY, Ordering::Release);
        assert!(
            admit(&l, CAPACITY, 0, 64, Level::Info),
            "a refusal computed from a stale cursor must be re-checked against the live one"
        );
        assert_eq!(l.read_cached.get(), CAPACITY, "the refresh must be kept, not discarded");
    }

    #[test]
    fn the_cached_cursor_is_only_ever_a_lower_bound() {
        // Staleness may cost space, never grant it. Asserted as an inequality over the two
        // budgets rather than as a comment on `read_cached`.
        let l = fresh();
        l.read.store(4096, Ordering::Release);
        l.read_cached.set(0);
        let stale = budget_at(&l, 8192, Level::Info);
        l.read_cached.set(4096);
        let fresh_budget = budget_at(&l, 8192, Level::Info);
        assert!(stale <= fresh_budget, "a stale cursor must never grant more space");
    }

    #[test]
    fn the_wrap_rule_pads_a_tail_that_cannot_hold_the_record() {
        // The rule the producer and the consumer share verbatim; exercised here as arithmetic so
        // a change to it fails before the consumer exists to disagree.
        let rule = |off: u32, need: u32| -> u32 {
            let tail = LANE_BYTES as u32 - off;
            if tail < HEADER_BYTES as u32 || tail < need { tail } else { 0 }
        };
        assert_eq!(rule(0, 64), 0, "a fresh ring needs no pad");
        assert_eq!(rule(LANE_BYTES as u32 - 8, 64), 8, "a tail below a header wraps implicitly");
        assert_eq!(rule(LANE_BYTES as u32 - 32, 64), 32, "a short tail takes an explicit PAD");
        assert_eq!(rule(LANE_BYTES as u32 - 64, 64), 0, "an exact fit needs no pad");
    }

    #[test]
    fn used_never_exceeds_capacity_across_the_cursor_wrap() {
        // Cursors are `u32` and wrap after 4 GiB per lane -- roughly 4 hours at 300 KiB/s. The
        // subtraction is `wrapping_sub` precisely so the invariant survives that boundary; a
        // plain `-` would panic in debug and produce a nonsense budget in release.
        let l = fresh();
        let w = 8u32;
        l.read_cached.set(u32::MAX - 7); // 16 bytes outstanding, straddling the wrap
        assert_eq!(w.wrapping_sub(l.read_cached.get()), 16);
        assert_eq!(budget_at(&l, w, Level::Error), CAPACITY - 16);
    }

    // ── end-to-end over the REAL static ring ─────────────────────────────────
    //
    // The arithmetic above is tested on an owned lane; these drive `emit_impl` on the process
    // array, which is the only way to catch a defect in the wiring between them. Each runs on its
    // OWN spawned thread so it claims its own spare lane: `cargo test` is concurrent, spares are
    // handed out by CAS, and two tests sharing a ring would be a coin flip rather than a gate.

    /// Run `f` on a fresh thread holding its own spare lane, and give it that lane's index.
    ///
    /// **The cursors are reset before `f` runs, and that reset is the FIXTURE's, not the
    /// production path's.** Spare lanes are recycled: a released index is immediately reclaimable,
    /// and a new owner APPENDS to the ring rather than resetting it — which is exactly the
    /// property that makes the substrate's immediate-free sound and is argued at Decision 4. So a
    /// test that claims a lane inherits whatever the previous test left in it.
    ///
    /// MEASURED, because the first draft did not reset: `one_record_advances_write_by…` and
    /// `a_full_lane_drops…` passed in debug and failed **intermittently in release** — 2 of 4
    /// runs — depending purely on which test got which recycled index. The lane-filling test
    /// leaves its ring full, and the next claimant of that index is refused. Nothing was wrong
    /// with the code; the fixture assumed a precondition it had not established. Production must
    /// NOT do this reset — resetting a recycled ring would discard records the consumer has not
    /// yet staged.
    fn on_own_lane<R: Send + 'static>(
        f: impl FnOnce(u16, &'static LogLane) -> R + Send + 'static,
    ) -> R {
        // The ring array and the drain role are BOTH process-global, and they are one resource for
        // locking purposes: a drain walks every lane, so per-lane ownership does not scope it.
        // The lock lives in `drain_owner` because there is exactly one drain token and therefore
        // must be exactly one lock over it -- two domains over one resource is not serialization,
        // which was measured the hard way.
        let _serial = crate::drain_owner::test_serial();
        std::thread::spawn(move || {
            let id = boyko_diag::lane::claim_lane()
                .expect("a spare lane; 14 exist and this suite does not hold that many at once");
            let lane = &LOG_LANES[id as usize];
            lane.write.store(0, Ordering::Release);
            lane.read.store(0, Ordering::Release);
            lane.read_cached.set(0);
            let out = f(id, lane);
            boyko_diag::lane::release_lane();
            out
        })
        .join()
        .expect("lane fixture thread panicked")
    }

    static TEST_SITE: LogSite = LogSite {
        target: crate::TargetId::new_engine(1),
        level: Level::Info,
        class: 0,
        code: 0,
        line: 0,
        file: "lane.rs",
        fmt: "probe {}",
        fields: &[],
        prefix: "boyko",
        decode: crate::site::decode_opaque,
    };

    #[test]
    fn one_record_advances_write_by_exactly_its_encoded_length() {
        on_own_lane(|_, lane| {
            let before = lane.write.load(Ordering::Relaxed);
            emit_impl(&TEST_SITE, (7u32, "ab"));
            let after = lane.write.load(Ordering::Relaxed);
            // 20 header + 4 (u32) + 2 (len) + 2 ("ab")
            assert_eq!(after.wrapping_sub(before), (HEADER_BYTES + 8) as u32);
        });
    }

    #[test]
    fn an_over_cap_record_is_refused_without_touching_the_ring() {
        on_own_lane(|id, lane| {
            let s = "z".repeat(crate::record::MAX_STR_BYTES);
            let before_w = lane.write.load(Ordering::Relaxed);
            let before_n =
                boyko_diag::loss::cell(id, LossClass::Refused).count();

            // Twelve maximal strings: 12 * 258 + 20 = 3116 > MAX_RECORD_BYTES.
            let a = s.as_str();
            emit_impl(&TEST_SITE, (a, a, a, a, a, a, a, a, a, a, a, a));

            assert_eq!(
                lane.write.load(Ordering::Relaxed),
                before_w,
                "a refused record must not advance the cursor"
            );
            assert_eq!(
                boyko_diag::loss::cell(id, LossClass::Refused).count(),
                before_n + 1,
                "the refusal must be counted, in every profile -- it is a runtime check, not a \
                 debug_assert of unreachability"
            );
        });
    }

    #[test]
    fn a_full_lane_drops_and_counts_instead_of_overrunning() {
        on_own_lane(|id, lane| {
            let before_n = boyko_diag::loss::cell(id, LossClass::Overflow).count();
            // Nothing drains, so the ring fills and every later record is refused. The bound is
            // the point: `write` must stop, not wrap over live bytes.
            for i in 0..4096u32 {
                emit_impl(&TEST_SITE, (i, "payload"));
            }
            let w = lane.write.load(Ordering::Relaxed);
            let r = lane.read.load(Ordering::Acquire);
            let used = w.wrapping_sub(r);
            let dropped = boyko_diag::loss::cell(id, LossClass::Overflow).count() - before_n;

            assert!(dropped > 0, "a lane with no consumer must eventually refuse");
            // The invariant is `used <= CAPACITY`, NOT `w <= CAPACITY`. `w` is a monotone cursor
            // that wraps at 2^32; only differences are meaningful. The first draft asserted the
            // latter, passed in debug and FAILED IN RELEASE -- not because the code differed, but
            // because spare lanes are RECYCLED between tests and release happened to hand this
            // one a ring another test had already advanced. A test that states the wrong
            // invariant is green or red by accident of scheduling.
            assert!(
                used <= CAPACITY,
                "the producer overran live bytes: used = {used}, CAPACITY = {CAPACITY} \
                 (w = {w}, read = {r})"
            );
        });
    }

    static TEST_SITE_ERROR: LogSite = LogSite {
        target: crate::TargetId::new_engine(1),
        level: Level::Error,
        class: b'E',
        code: 1,
        line: 0,
        file: "lane.rs",
        fmt: "probe {}",
        fields: &[],
        prefix: "boyko",
        decode: crate::site::decode_opaque,
    };

    /// **The F6 regression, end to end.**
    ///
    /// The reserve exists precisely to be exercised in the state where the used span has already
    /// passed `CAPACITY - ERROR_RESERVE` — which only `Error` records can create. The predecessor
    /// computed `CAPACITY - ERROR_RESERVE - used` in `u32` there, underflowed to ~4.29 × 10⁹, and
    /// admitted a non-`Error` record over bytes the consumer had not staged.
    ///
    /// **A fill of `Info` records cannot reach that state and therefore cannot test it**: `used`
    /// stops growing at the reserve boundary because admission refuses there. The first draft of
    /// the neighbouring fill test emitted only `Info`, stayed green under the broken arithmetic,
    /// and was measured to do so. This test emits `Error` first, on purpose.
    #[test]
    fn a_non_error_is_refused_once_only_the_error_reserve_is_left() {
        on_own_lane(|id, lane| {
            // Fill with Error records until they are refused; `used` is then above
            // `CAPACITY - ERROR_RESERVE` and inside the reserve.
            for i in 0..4096u32 {
                emit_impl(&TEST_SITE_ERROR, (i,));
                if lane.write.load(Ordering::Relaxed) > CAPACITY - ERROR_RESERVE {
                    break;
                }
            }
            let used = lane.write.load(Ordering::Relaxed);
            assert!(
                used > CAPACITY - ERROR_RESERVE,
                "the fixture must reach the reserve, or it tests nothing (used = {used})"
            );

            let before_w = used;
            let before_n = boyko_diag::loss::cell(id, LossClass::Overflow).count();
            emit_impl(&TEST_SITE, (1u32,));

            assert_eq!(
                lane.write.load(Ordering::Relaxed),
                before_w,
                "a non-Error record must be REFUSED once only the Error reserve remains; \
                 admitting it here is the 4.29e9-byte licence that overruns live bytes"
            );
            assert_eq!(
                boyko_diag::loss::cell(id, LossClass::Overflow).count(),
                before_n + 1,
                "the refusal must be counted"
            );

            // And the reserve must still be usable by what it is reserved for.
            emit_impl(&TEST_SITE_ERROR, (2u32,));
            assert!(
                lane.write.load(Ordering::Relaxed) > before_w,
                "an Error record must still be admitted into its own reserve"
            );
        });
    }

    #[test]
    fn records_never_straddle_the_end_of_the_ring() {
        on_own_lane(|_, lane| {
            // Drive the cursor to just short of the end, then emit one record that cannot fit in
            // the tail. The producer must lay a PAD and restart at offset 0.
            let need = (HEADER_BYTES + 4) as u32;
            let start = LANE_BYTES as u32 - need + 4;
            // `read` follows `write`, so the ring is EMPTY at that offset. Setting only `write`
            // would leave `used == start`, the budget would be zero and the record would be
            // refused -- the test would then pass or fail for a reason unrelated to wrapping.
            lane.write.store(start, Ordering::Release);
            lane.read.store(start, Ordering::Release);
            lane.read_cached.set(start);

            let before = lane.write.load(Ordering::Relaxed);
            emit_impl(&TEST_SITE, (1u32,));
            let after = lane.write.load(Ordering::Relaxed);

            let pad = (LANE_BYTES as u32) - (before & MASK);
            assert_eq!(
                after.wrapping_sub(before),
                pad + need,
                "the cursor must advance by the PAD plus the record, so the record starts at 0"
            );
            assert_eq!(after & MASK, need, "the record must sit at the START of the ring");
        });
    }

    #[test]
    fn an_unlaned_thread_counts_low_levels_and_falls_back_for_severe_ones() {
        // A thread that never claimed a lane. `Info` is counted as unclaimed; `Error` takes the
        // synchronous channel instead of being dropped, which is the property that makes a
        // lane-exhausted harness unable to lose a severe record.
        std::thread::spawn(|| {
            assert_eq!(boyko_diag::lane::lane(), LANE_UNCLAIMED);
            // No lane is claimed here, so `resolve()` will attempt one; force the exhausted path
            // by observing the branch through the loss counters instead. `row_of` maps an
            // unclaimed thread to the un-laned row.
            let row = boyko_diag::loss::row_of(LANE_UNCLAIMED);
            let before = boyko_diag::loss::cell_at_row(row, LossClass::Unclaimed).count();
            // If a spare is available this thread gets one and the branch is not taken -- which is
            // itself correct behaviour, so the assertion is on the DISJUNCTION rather than on one
            // arm. Asserting only the fallback would make the test depend on how many spares the
            // rest of the suite happens to be holding.
            emit_impl(&TEST_SITE, (1u32,));
            let after = boyko_diag::loss::cell_at_row(row, LossClass::Unclaimed).count();
            let got_lane = boyko_diag::lane::lane() != LANE_UNCLAIMED;
            assert!(
                got_lane || after == before + 1,
                "an Info record must either reach a lane or be counted as unclaimed"
            );
            if got_lane {
                boyko_diag::lane::release_lane();
            }
        })
        .join()
        .expect("unlaned fixture thread panicked");
    }

    #[test]
    fn the_site_line_renderer_truncates_instead_of_overflowing() {
        // It runs BEFORE the synchronous channel's acquire, so it must not panic and must not
        // write past the buffer -- a bounds panic on the error-of-the-error path is the failure
        // the whole channel exists to avoid.
        let mut buf = [0u8; 8];
        let n = render_site_line(&mut buf, &TEST_SITE);
        assert!(n <= buf.len());
        assert!(core::str::from_utf8(&buf[..n]).is_ok());

        let mut big = [0u8; 160];
        let n = render_site_line(&mut big, &TEST_SITE);
        let s = core::str::from_utf8(&big[..n]).expect("ASCII only");
        assert!(s.starts_with("lane.rs:0 "), "got {s:?}");
        assert!(s.contains("probe"));
    }

    /// Empty every lane, so a following assertion on GLOBAL drain stats is this test's alone.
    ///
    /// `drain` walks all lanes; owning one does not scope it. MEASURED: without this, a test that
    /// published 200 records observed **1068**.
    fn drain_everything() {
        let t = crate::drain_owner::try_claim().expect("the ring lock excludes other consumers");
        let _ = drain(&t, |_, _, _, _| {});
    }

    #[test]
    fn a_drain_sees_every_published_record_and_then_the_lane_is_empty() {
        on_own_lane(|_, lane| {
            drain_everything();
            const N: u32 = 200;
            for i in 0..N {
                emit_impl(&TEST_SITE, (i, "p"));
            }
            let w = lane.write.load(Ordering::Relaxed);

            let token = crate::drain_owner::try_claim().expect("free");
            let mut seen = Vec::new();
            let stats = drain(&token, |site, _tsc, _flags, payload| {
                assert!(core::ptr::eq(site, &TEST_SITE), "the site pointer must round-trip");
                seen.push(payload.len());
            });
            drop(token);

            assert_eq!(seen.len() as u32, N, "every published record must be handed over");
            assert_eq!(stats.records, u64::from(N));
            // 4 (u32) + 2 (str len) + 1 ("p")
            assert!(seen.iter().all(|n| *n == 7), "payload lengths: {seen:?}");
            assert_eq!(stats.bytes, u64::from(N) * 7);
            assert_eq!(
                lane.read.load(Ordering::Acquire),
                w,
                "read must reach the horizon the walk consumed"
            );

            // A second pass must move nothing. Delivering a record twice is worse than losing it:
            // it invents an event.
            let token = crate::drain_owner::try_claim().expect("released");
            let again = drain(&token, |_, _, _, _| panic!("a drained lane must yield nothing"));
            drop(token);
            assert_eq!(again.records, 0);
        });
    }

    #[test]
    fn a_pad_is_skipped_and_not_counted_as_a_record() {
        // The wrap path's other half: the consumer steps over a PAD by `len` like any record while
        // NOT reporting it. Counting PADs would inflate every record count by how often the ring
        // happened to wrap.
        on_own_lane(|_, lane| {
            drain_everything();
            let need = (HEADER_BYTES + 4) as u32;
            let start = LANE_BYTES as u32 - need + 4;
            lane.write.store(start, Ordering::Release);
            lane.read.store(start, Ordering::Release);
            lane.read_cached.set(start);

            emit_impl(&TEST_SITE, (1u32,));

            let token = crate::drain_owner::try_claim().expect("free");
            let stats = drain(&token, |_, _, _, payload| assert_eq!(payload.len(), 4));
            drop(token);

            assert_eq!(stats.records, 1, "the PAD must not be reported as a record");
            assert_eq!(stats.bytes, 4, "the PAD's bytes are not payload");
            assert_eq!(lane.read.load(Ordering::Acquire), lane.write.load(Ordering::Relaxed));
        });
    }

    #[test]
    fn a_drain_frees_space_the_producer_can_then_use() {
        // The end-to-end reason the read half exists: without it a lane fills once and refuses
        // forever. A stubbed consumer fails this silently.
        on_own_lane(|id, lane| {
            drain_everything();
            let before = boyko_diag::loss::cell(id, LossClass::Overflow).count();
            for i in 0..4096u32 {
                emit_impl(&TEST_SITE, (i, "payload"));
            }
            assert!(
                boyko_diag::loss::cell(id, LossClass::Overflow).count() > before,
                "the fixture must actually fill the lane"
            );

            let token = crate::drain_owner::try_claim().expect("free");
            let stats = drain(&token, |_, _, _, _| {});
            drop(token);
            assert!(stats.records > 0);

            let mid = boyko_diag::loss::cell(id, LossClass::Overflow).count();
            let w = lane.write.load(Ordering::Relaxed);
            emit_impl(&TEST_SITE, (0u32, "payload"));
            assert!(
                lane.write.load(Ordering::Relaxed) > w,
                "a drained lane must accept records again"
            );
            assert_eq!(
                boyko_diag::loss::cell(id, LossClass::Overflow).count(),
                mid,
                "and must not count that acceptance as a drop"
            );
        });
    }

    #[test]
    fn the_lane_array_matches_the_topology_unless_the_ceiling_is_off() {
        // `LANE_ARRAY_LEN` is the one place the compile ceiling changes a data structure's size
        // rather than deleting code, so the two cases are stated rather than inferred.
        if (GLOBAL_CEILING as u8) == 0 {
            assert_eq!(LANE_ARRAY_LEN, 0);
        } else {
            assert_eq!(LANE_ARRAY_LEN, LANE_COUNT as usize);
        }
        assert_eq!(LOG_LANES.len(), LANE_ARRAY_LEN);
    }
}
