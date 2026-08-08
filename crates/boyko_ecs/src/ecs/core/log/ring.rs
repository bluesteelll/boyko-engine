//! [`LogRing`] — the durable, displayable log, on engine storage.

use std::sync::OnceLock;

use crate::ecs::constants::COMMIT_GRANULE;
use crate::ecs::core::resources::register_new;
use crate::ecs::core::resources::resource::Resource;
use crate::ecs::identifiers::primitives::ResourceId;
use crate::ecs::memory::vm_column::VmColumn;

/// Lines the ring retains. `4096 * 16 B` is exactly one [`COMMIT_GRANULE`], so the line column's
/// reservation is a single commit step and its ceiling divides the granule with nothing left over.
pub const LINE_CAP: u32 = 4096;

/// Bytes of formatted text the ring retains.
///
/// Sized against [`LINE_CAP`] at the ~120 B per line the sink's renderer actually produces:
/// 4096 × 120 ≈ 492 KiB, so 512 KiB is the smallest power of two that lets the line ring reach its
/// own capacity before the arena forces an eviction. When it does not — a burst of long lines —
/// the arena is what bounds retention, and [`LogRing::len`] reports the truth rather than
/// [`LINE_CAP`].
pub const ARENA_BYTES: u32 = 512 * 1024;

/// One retained line's metadata. **Exactly 16 bytes**, and pinned below.
///
/// The text is not here: it lives in the ring's byte arena at `start`, for `len` bytes. Two
/// columns rather than one fixed-stride column because a fixed stride would either truncate the
/// long lines or waste the short ones, and the long lines are the ones a reader wants.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct LogLine {
    /// Offset of the text in the ring's arena.
    pub start: u32,
    /// Low half of this line's [`LogRing::cursor`] value at the moment it was stored.
    ///
    /// The high half is never stored and never needs to be. For any line still in the ring,
    /// `seq = ring.seq - (ring.seq_lo wrapping_sub line.seq_lo) as u64`, and the difference is
    /// unambiguous because the ring holds at most `LINE_CAP << 2^31` lines. The ~2.4 h wrap at
    /// 500 K rec·s⁻¹ is therefore not a truncation.
    pub seq_lo: u32,
    /// Bytes of formatted text.
    pub len: u16,
    /// The site's printed code number; `0` when the level carries none.
    pub code: u16,
    /// The site's severity, as `boyko_log::Level`'s raw discriminant.
    pub level: u8,
    /// `boyko_log::TargetId::index()`, which `MAX_TARGETS == 256` makes exact in a `u8`.
    pub target: u8,
    /// The record's argument flags.
    pub flags: u8,
    _pad: u8,
}

impl LogLine {
    /// The value the line column is materialized with. Never observable: a slot below
    /// [`LogRing::len`] has always been overwritten by a real line.
    const EMPTY: LogLine = LogLine {
        start: 0,
        seq_lo: 0,
        len: 0,
        code: 0,
        level: 0,
        target: 0,
        flags: 0,
        _pad: 0,
    };
}

// The size is not a cosmetic pin. `VmColumn::<T>::new` PANICS unless
// `COMMIT_GRANULE % size_of::<T>() == 0`, so a layout of, say, 12 bytes (10 payload + align-4
// tail; `65536 % 12 == 4`) would make `LogPlugin::build` panic at construction in every process
// that added the plugin. `repr(C, packed)` does not save that layout either — `65536 % 10 == 6`.
// The fix is a size that divides the granule, and this turns "someone adds a field" from a
// plugin-build panic into a compile error here.
const _: () = assert!(size_of::<LogLine>() == 16);
const _: () = assert!(
    COMMIT_GRANULE.is_multiple_of(size_of::<LogLine>()),
    "LogLine must divide COMMIT_GRANULE or VmColumn::new panics"
);
// `VmColumn<u8>` for the arena is trivially fine: 65536 % 1 == 0. Asserted anyway, because the
// reason it is fine is the same rule, and a reader should not have to know which types are exempt.
const _: () = assert!(COMMIT_GRANULE.is_multiple_of(size_of::<u8>()));

/// The durable, displayable log.
///
/// Backed by the engine's own storage — `VmReservation`-backed columns, not a `Box<[u8]>` heap
/// side-store, which is the shape Principle 0 was re-stated to forbid **even inside a
/// `Resource`**.
///
/// # Materialization is LAZY, and that is load-bearing
///
/// [`LogPlugin::build`](super::LogPlugin) performs no reserve and no commit: it runs before the
/// runtime flag is read, and a diagnostics subsystem may not make a syscall the flag has not
/// authorised. The one growth to full capacity happens inside
/// [`log_drain_system`](super::log_drain_system)'s `ResMut`, on the first drain that actually
/// carries a line. `VmColumn` is lazy by construction, so `new` costs nothing until then.
///
/// # A live line's text is always intact
///
/// The arena wraps independently of the line column, so a line can outlive its own text. It does
/// not: every write evicts the lines whose arena span it is about to overwrite, so `len` counts
/// lines whose text is still readable rather than lines whose metadata still exists.
pub struct LogRing {
    /// Retained line metadata, `LINE_CAP` slots after materialization.
    lines: VmColumn<LogLine>,
    /// Formatted text, `ARENA_BYTES` after materialization.
    arena: VmColumn<u8>,
    /// Index in `lines` of the next slot to write. Wraps at `LINE_CAP`.
    head: u32,
    /// Live lines, `<= LINE_CAP`. Only these have intact text.
    len: u32,
    /// Next free byte in `arena`. Wraps at `ARENA_BYTES`.
    arena_cursor: u32,
    /// Lines ever stored. Monotone; the reader's cursor. Never wraps in any reachable session —
    /// at 500 K lines·s⁻¹ a `u64` lasts ~1.2 million years.
    seq: u64,
}

// ─── `VmColumn` is `!Send` + `!Sync`; `Resource` requires both ───────────────────────────────
//
// `vm_column.rs` states verbatim that `VmColumn` is "NOT `Send`/`Sync` (the `NonNull` inside
// `VmReservation` and `base`): owners that cross threads carry their own exclusivity argument in
// their manual `unsafe impl Send/Sync`", and `resource.rs` reads
// `pub trait Resource: 'static + Send + Sync + Sized`. So a `LogRing` cannot be a `Resource` by
// derivation; it needs the impls below and the argument that makes them true.
//
// COMPILE-TIME PIN — the BOUND only. For `LogRing` the manual impls below are unconditional and
// non-generic, so they forge exactly the property asserted here: this block CANNOT red on a
// future `!Send`/`!Sync` field, for any field set whatsoever. That is the difference from the
// `size_of::<LogLine>()` assert above — `size_of` is a property no impl can forge, `Send + Sync`
// is precisely the property the impl does forge. Nor does it localize the error: without the
// impls, `impl Resource for LogRing` below already fails on the supertrait bound. It reds on
// exactly one edit — deleting the impls — and it is kept for that, and because on `LogStats`,
// which derives both, the same line is not a tautology.
const _: () = {
    const fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<LogRing>();
    assert_send_sync::<super::LogStats>();
};

// WHAT DOES RED ON A NEW FIELD — an exhaustive-destructuring witness, and the only mechanical part
// of the answer. A struct pattern that omits a field is `error[E0027]: pattern does not mention
// field`, so adding a field to `LogRing` breaks THIS, at compile time, in this file, immediately
// above the five SAFETY clauses that adding it obliges a human to re-read. `field: _` counts as
// mentioning the field, so the witness costs nothing at runtime and warns nothing. What it cannot
// do is assert `Send`/`Sync` per field: the fields are deliberately `!Send` (`VmColumn`), which is
// the entire reason the manual impls exist. So the witness forces the re-read; the re-read is what
// decides whether clauses 1-5 still hold.
const _: () = {
    #[allow(dead_code)] // never called; it exists to be type-checked
    fn field_witness(r: &LogRing) {
        let LogRing { lines: _, arena: _, head: _, len: _, arena_cursor: _, seq: _ } = r;
    }
};

// SAFETY (`Send`/`Sync` for `LogRing`):
//   1. WHO MAY HOLD `&mut`: exactly one system, `log_drain_system`, which the scheduler grants
//      `ResMut<LogRing>`. The scheduler's conflict analysis is what makes that exclusive; no other
//      system in this engine declares `ResMut<LogRing>`.
//   2. WHO MAY HOLD `&`: any system declaring `Res<LogRing>` -- a HUD, a console, a telemetry
//      reducer. The scheduler never runs a `Res` reader concurrently with the `ResMut` writer,
//      which is the same guarantee every other `Resource` in this engine rests on.
//   3. WHO MAY NOT TOUCH IT AT ALL: **the sink thread**. This is the clause that makes the impl
//      true rather than merely stated. The sink writes `boyko_log::sink::ecs`'s `.bss` byte ring
//      and never names `LogRing`; that indirection is the entire reason the byte ring exists. If
//      the sink wrote these columns directly, clauses 1-2 would be false and the only repair would
//      be a lock, which Invariant 1 forbids.
//   4. WHY THE UNDERLYING COLUMNS TOLERATE IT — quoting `vm_column.rs`'s own invariant list:
//      `base` is write-once (set at lazy materialization inside the `&mut self`-only `grow_to`,
//      stable thereafter), every mutation requires `&mut self`, and cross-thread `&self` reads
//      touch only committed plain-old-data below `len` with no interior mutability. `LogLine` and
//      `u8` are both POD, so clause 4 holds for every element type used here.
//   5. MATERIALIZATION STAYS LAZY, and clauses 1-2 -- not pre-materialization -- are what exclude
//      a partially-materialized observation. `LogPlugin::build` performs NO reserve and no commit:
//      it runs BEFORE the flag is read. The write-once `base` store happens inside
//      `log_drain_system`'s `ResMut`, on the first drain that actually carries a line. Only that
//      system ever holds `&mut` (clause 1) and the scheduler never runs a `Res` reader
//      concurrently with it (clause 2), so no `&self` reader exists during the store.
//
//      WITH THE FLAG OFF, `log_drain_system` RETURNS BEFORE TOUCHING ANYTHING -- one `Relaxed`
//      load per frame, and no column is reached. That early return is load-bearing, not an
//      optimisation, and it is load-bearing for a reason that is not yet visible at this rung:
//      L16 gives this system two further duties -- the `TARGET_STATS` snapshot and the per-frame
//      `frame_epoch` record -- which it writes ON ITS OWN ACCOUNT rather than out of the handoff.
//      Without the check, the drain would then grow these columns on frame 1 with every target
//      still `Off`, and "the flag is off, so no record exists, so nothing is materialized" would
//      be false. Proving a property at the emission path does not prove it for a system that also
//      writes on its own account.
unsafe impl Send for LogRing {}
unsafe impl Sync for LogRing {}

// Hand-implemented rather than `#[derive(Resource)]`: `boyko-macros` is a dev-dependency of
// `boyko-ecs`, so its derives are unavailable in normal builds. Mirrors exactly what the derive
// expands to.
impl Resource for LogRing {
    #[inline]
    fn resource_id() -> ResourceId {
        static ID: OnceLock<ResourceId> = OnceLock::new();
        *ID.get_or_init(|| ResourceId(register_new::<Self>()))
    }
}

impl Default for LogRing {
    fn default() -> LogRing {
        LogRing::new()
    }
}

impl LogRing {
    /// An empty, **unmaterialized** ring. No reservation, no commit, no syscall.
    #[must_use]
    pub fn new() -> LogRing {
        LogRing {
            lines: VmColumn::new("LogRing.lines", LINE_CAP as usize),
            arena: VmColumn::new("LogRing.arena", ARENA_BYTES as usize),
            head: 0,
            len: 0,
            arena_cursor: 0,
            seq: 0,
        }
    }

    /// Lines ever stored. Monotone, and the value a reader holds between frames.
    #[inline]
    #[must_use]
    pub fn cursor(&self) -> u64 {
        self.seq
    }

    /// Live lines — those whose text is still intact. `<= LINE_CAP`.
    #[inline]
    #[must_use]
    pub fn len(&self) -> u32 {
        self.len
    }

    /// `true` iff no line is retained.
    #[inline]
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// `true` once the columns have been reserved and committed.
    ///
    /// Exists so the laziness claim is a question a test can ask, rather than a sentence in a
    /// SAFETY block that nothing checks.
    #[inline]
    #[must_use]
    pub fn is_materialized(&self) -> bool {
        !self.lines.is_empty()
    }

    /// The retained line at `index`, oldest first. `None` past [`len`](Self::len).
    ///
    /// This is the whole read surface at this rung. `since` / `RingFilter` / `skipped` — the
    /// cursor-based reader a console needs — are L16; exposing them here without the
    /// `frame_epoch` record they attribute against would ship a reader that cannot answer the one
    /// question it exists for.
    #[must_use]
    pub fn line(&self, index: u32) -> Option<(LogLine, &[u8])> {
        if index >= self.len {
            return None;
        }
        let slot = (self.head + LINE_CAP - self.len + index) % LINE_CAP;
        let line = self.lines.get(slot as usize)?;
        let start = line.start as usize;
        let text = self.arena.as_slice().get(start..start + line.len as usize)?;
        Some((line, text))
    }

    /// Store one drained frame, evicting whatever its text overwrites.
    ///
    /// Takes the transport's own `Frame` rather than a half-filled [`LogLine`]: `start`, `seq_lo`
    /// and `len` are this ring's to assign, and a constructor that accepted them would invite a
    /// caller to pass values the ring then silently overwrote.
    ///
    /// Materializes the columns on first use. `text` longer than the arena's capacity is
    /// impossible — the producer caps a line at `boyko_log::sink::ecs::MAX_LINE_BYTES`, three
    /// orders of magnitude below [`ARENA_BYTES`] — and the debug assert states it.
    pub(crate) fn store(&mut self, frame: &boyko_log::sink::ecs::Frame<'_>) {
        let text = frame.text;
        debug_assert!(
            text.len() < ARENA_BYTES as usize,
            "invariant: a single line fits in the arena"
        );
        // `LogLine.len` is a `u16`, and the producer caps a line at
        // `boyko_log::sink::ecs::MAX_LINE_BYTES` (1 KiB) — three orders of magnitude below the
        // arena, so the bound above would not catch a `u16` truncation on its own.
        debug_assert!(
            text.len() <= u16::MAX as usize,
            "invariant: a line's length fits LogLine::len"
        );
        self.materialize();

        let n = text.len() as u32;
        // Text never straddles the end of the arena: a tail too short wraps the cursor and
        // abandons the remainder. A wrapped span would make `line(index)`'s single
        // `get(start..start + len)` a two-part read for no gain.
        //
        // THE ABANDONED REMAINDER MUST STILL EVICT. MEASURED, and it is not a rounding error: a
        // line lying wholly inside the abandoned tail is never overwritten, so it is never
        // evicted, so it becomes the OLDEST live line forever — and `evict_overwritten` stops at
        // the first non-intersecting tail, so from that moment on it evicts NOTHING. Observed as
        // `len` climbing past the arena's capacity (1169 -> 1650 -> 2150 -> …) while the ring
        // silently handed out slices of other lines' text. Treating the remainder as a write is
        // what keeps the walk's premise — "the cursor's next span is the oldest live line's" —
        // true across the wrap, which is the one place it was false.
        if self.arena_cursor + n > ARENA_BYTES {
            self.evict_overwritten(self.arena_cursor, ARENA_BYTES - self.arena_cursor);
            self.arena_cursor = 0;
        }
        let start = self.arena_cursor;
        self.evict_overwritten(start, n);

        self.arena.as_mut_slice()[start as usize..(start + n) as usize].copy_from_slice(text);
        self.arena_cursor = start + n;

        let stored = LogLine {
            start,
            seq_lo: self.seq as u32,
            len: n as u16,
            code: frame.code,
            level: frame.level,
            target: frame.target,
            flags: frame.flags,
            _pad: 0,
        };
        self.lines.set(self.head as usize, stored);
        self.head = (self.head + 1) % LINE_CAP;
        if self.len < LINE_CAP {
            self.len += 1;
        }
        self.seq += 1;
    }

    /// Grow both columns to their full capacity, once.
    ///
    /// `#[cold]`: it runs on the first drain that carries a line and never again, so the branch
    /// that reaches it is predicted-not-taken for the life of the process.
    #[cold]
    #[inline(never)]
    fn materialize_cold(&mut self) {
        self.lines.extend_exact((0..LINE_CAP).map(|_| LogLine::EMPTY));
        self.arena.extend_exact(std::iter::repeat_n(0u8, ARENA_BYTES as usize));
    }

    #[inline]
    fn materialize(&mut self) {
        if self.lines.is_empty() {
            self.materialize_cold();
        }
    }

    /// Drop the live lines whose text spans `[start, start + n)`.
    ///
    /// # Why popping from the tail is enough
    ///
    /// Lines are appended in arena order, and neither a line's span nor a write's span ever wraps
    /// (the cursor jumps to 0 instead). So the next span the cursor advances into is always the
    /// OLDEST live line's, and the first non-intersecting tail ends the walk. A newer line cannot
    /// be clobbered before an older one without the cursor having passed the older one first,
    /// which is the step that evicted it.
    fn evict_overwritten(&mut self, start: u32, n: u32) {
        let end = start + n;
        while self.len > 0 {
            let slot = (self.head + LINE_CAP - self.len) % LINE_CAP;
            let Some(tail) = self.lines.get(slot as usize) else { break };
            let (ts, te) = (tail.start, tail.start + u32::from(tail.len));
            if ts < end && start < te {
                self.len -= 1;
            } else {
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(text: &[u8]) -> boyko_log::sink::ecs::Frame<'_> {
        boyko_log::sink::ecs::Frame { level: 3, target: 0, code: 0, flags: 0, text }
    }

    /// Every line's text is a function of its own sequence number, so an overwrite is visible.
    ///
    /// The byte a line is filled with is `seq % 251`, and its length varies with `seq`. Both are
    /// deliberate, and MEASURED: a first version used 512 B lines from a two-symbol alphabet, and
    /// disarming the eviction walk left this loop GREEN — every line was the same length as the
    /// one overwriting it, at the same alignment (512 KiB is exactly 1024 × 512 B), and the two
    /// symbols made a corrupted read indistinguishable from a correct one. It failed only on the
    /// tail assertion, i.e. on the wrong claim. A test that survives the defect it names is not a
    /// test of it.
    fn pattern(seq: u64) -> (usize, u8) {
        // Lengths that do not divide the arena, so spans straddle each other rather than aligning.
        (256 + (seq as usize % 7) * 64, (seq % 251) as u8)
    }

    #[test]
    fn a_live_line_is_never_overwritten_by_the_arena() {
        let mut ring = LogRing::new();
        let mut buf = [0u8; 640];

        for i in 0..u64::from(LINE_CAP) * 4 {
            let (n, byte) = pattern(i);
            buf[..n].fill(byte);
            ring.store(&frame(&buf[..n]));

            assert_eq!(ring.cursor(), i + 1);
            assert!(ring.len() <= LINE_CAP, "the line ring exceeded its capacity");

            // The oldest, the middle and the newest live line must each read back the text their
            // OWN `seq_lo` implies. The oldest is the one an under-eager eviction corrupts; the
            // newest is the one an over-eager eviction loses.
            let live = ring.len();
            for k in [0, live / 2, live - 1] {
                let (line, text) = ring.line(k).expect("a line below len is live");
                let (want_n, want_b) = pattern(u64::from(line.seq_lo));
                assert_eq!(text.len(), want_n, "line {k} (seq {}) has the wrong length", line.seq_lo);
                assert!(
                    text.iter().all(|&b| b == want_b),
                    "line {k}/{live} (seq {}) was overwritten: expected {want_b:#x}, saw {:#x}; \
                     i={i} start={} n={} head={} len={} cursor={}",
                    line.seq_lo,
                    text[0],
                    line.start,
                    line.len,
                    ring.head,
                    ring.len,
                    ring.arena_cursor
                );
            }
            let (newest, _) = ring.line(live - 1).expect("the newest line is live");
            assert_eq!(u64::from(newest.seq_lo), i, "seq_lo must be the line's own sequence");
        }

        // ~448 B average through a 512 KiB arena bounds retention near 1170 — well below
        // `LINE_CAP`. The ring must report the arena's answer, not the line column's.
        assert!(
            ring.len() < LINE_CAP,
            "the arena, not LINE_CAP, is the binding constraint here: len = {}",
            ring.len()
        );
        assert!(ring.line(ring.len()).is_none(), "one past the last live line must be None");
    }

    /// A fresh ring costs no reservation, and the first stored line pays for both columns.
    #[test]
    fn materialization_is_deferred_to_the_first_line() {
        let mut ring = LogRing::new();
        assert!(!ring.is_materialized());
        assert!(ring.is_empty());

        ring.store(&frame(b"first"));
        assert!(ring.is_materialized());
        assert_eq!(ring.len(), 1);
        assert_eq!(ring.line(0).expect("stored").1, b"first");
    }

    /// The line column reaches its own capacity when the arena is not the binding constraint.
    ///
    /// The companion to the test above: with a short line the arena outlasts `LINE_CAP`, so `len`
    /// saturates at the capacity rather than at whatever the arena allows. Both directions are
    /// checked because a ring that always evicted early would pass the first test alone.
    #[test]
    fn short_lines_fill_the_line_column_to_capacity() {
        let mut ring = LogRing::new();
        for _ in 0..u64::from(LINE_CAP) * 2 {
            ring.store(&frame(b"s"));
        }
        assert_eq!(ring.len(), LINE_CAP, "a 1 B line must let the line column reach LINE_CAP");
        assert_eq!(ring.cursor(), u64::from(LINE_CAP) * 2);
    }
}
