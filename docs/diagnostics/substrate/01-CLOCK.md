# Substrate A1 — `clock` (and session identity)

<!-- CONTRACT
provides: substrate/clock-source          # the ONE clock and the ONE SessionId both subsystems consume
assumes:  substrate/loss-vocabulary       # raise(DiagFlag::Clock*) is the loss module's mechanism
assumes:  substrate/never-freed-storage   # the no-second-per-OS-backing rule is why the fallback is Instant, not QPC FFI
-->

> **Carved from** `docs/DIAGNOSTICS-SUBSTRATE-PLAN.md` §3 A1, in full.
> Diff against that file until the monoliths are retired.

**The consequence of not sharing this, in one line:** a suspend/resume produces a profiler
window quarantined as `EpochBreak` and, in the same seconds, log lines whose printed wall times
are wrong by the suspend duration with no marker — two artifacts that disagree, neither of which
says why.

---

## API

```rust
pub struct SessionId(pub u64, pub u64);          // 128-bit, one id for both artifact headers

#[inline] pub fn ticks() -> u64;                 // rdtsc on x86_64; Instant delta elsewhere
#[inline] pub fn ticks_per_ns() -> f64;          // published once by calibrate()
#[inline] pub fn clock_epoch() -> u32;           // bumped on a detected discontinuity
#[cold]   pub fn calibrate();                    // 16 probes / 20 ms; idempotent
#[cold]   pub fn note_forward_jump(observed: u64);
          pub fn invariant_tsc() -> bool;        // CPUID.80000007H:EDX[8], probed once
          pub fn session_id() -> SessionId;      // minted once at first touch
```

`SessionId` is minted **once, here**, and joins the two artifact headers. Neither subsystem
mints its own; a per-crate `session.rs` is the shape this crate exists to prevent.

---

## Layout

All state is `.bss` statics; **there is no `ClockState` instance and no constructor.** The five
read-mostly words share one line because `ticks_per_ns` and `clock_epoch` are read together on
every record and every window fold:

```rust
#[repr(C, align(64))]
struct ClockGlobals {
    ticks_per_ns_bits: AtomicU64,   // f64::to_bits; f64 has no atomic type
    session_lo: AtomicU64,
    session_hi: AtomicU64,
    epoch: AtomicU32,
    state: AtomicU32,               // UNCALIBRATED | RUNNING | DONE
    invariant: AtomicU32,           // UNPROBED | NO | YES
    session_state: AtomicU32,       // UNMINTED | MINTING | DONE
    _pad: [u8; 24],
}
static CLOCK: ClockGlobals = /* all-zero const init */;
```

64 B, one line, read-mostly — 8 + 8 + 8 + 4 + 4 + 4 + 4 = 40 B of state, and 40 + 24 = 64 with the
pad. **`session_state` is the session pair's publication word**, exactly as `state` is
`ticks_per_ns_bits`'s: `session_lo`/`session_hi` are two independent atomics, so without a third
word there is nothing for a reader to acquire against and a half-written id is observable (see
§"Memory ordering" below). It is paid for out of the padding that was already there — no second
word of state crosses the line, and the struct is still exactly one line.

**`DIAG_FLAGS` ([`03-LOSS.md`](03-LOSS.md)) is deliberately *not*
in this struct:** `raise` dirties it, and a dirtied line shared with the clock would invalidate
a line every hot reader touches.

---

## Who writes what, from which thread

| Datum | Writer | Thread | When |
|---|---|---|---|
| `ticks_per_ns_bits`, `state` | `calibrate()` | whichever thread calls first — `boyko_log::enable` or `Profiler::arm`, both idempotent | **the ENABLE path. NOT boot.** See below |
| `session_lo/hi`, `session_state` | `session_id()` on first touch | any | first touch, once |
| `epoch` | `note_forward_jump()` | the detecting thread (profiling's fold, on the dispatcher) | rare |
| `invariant` | `invariant_tsc()` on first call | any | once |

**The `calibrate()` row changed at the corpus split.** The decision record had it at "boot,
once". It now runs on **the enable path** — whichever of `boyko_log::enable()` /
`Profiler::arm()` runs first — because nothing in `boyko_diag` may be touched, calibrated,
spawned or committed while the runtime diagnostics flags are off. **This costs no new
mechanism:** `calibrate()` is already idempotent and CAS-guarded (below), so "whichever runs
first" was already the contract; only the call site moved. **With both subsystems off, the clock
is never calibrated and never read**, and `ticks_per_ns()`'s uncalibrated arm is never taken
because nothing stamps.

That is a statement about a **call site**, which is not this module's property to decide. What
this file states is only what the module does when it *is* called: `calibrate()` is idempotent
and CAS-guarded, so "whichever thread calls first wins and the rest observe `DONE`" is the
contract at any call site whatsoever. The joint decision that moved the call one step later, and
the cost accounting that motivated it, are made **above this area** and are deliberately not
cited here — nothing in this module derives from them, and the module is correct under either
placement.

---

## Memory ordering, every clause with its pairing argument

- **`calibrate()`**: `state.compare_exchange(UNCALIBRATED, RUNNING, AcqRel, Acquire)`. The winner
  probes, `ticks_per_ns_bits.store(bits, Relaxed)`, then `state.store(DONE, Release)`. A loser
  spins `state.load(Acquire)` with `core::hint::spin_loop()` + `std::thread::yield_now()` until
  `DONE`. **The `Release` here matches the `Acquire` in `ticks_per_ns()`** — it is what makes the
  probed scale visible to every later reader. **No `Mutex`** (clippy `disallowed-types`); a CAS +
  bounded spin on an enable-path-only routine is the compliant shape.
- **`session_id()`**: `session_state.compare_exchange(UNMINTED, MINTING, AcqRel, Acquire)`. The
  winner stores `session_lo`/`session_hi` `Relaxed`, then `session_state.store(DONE, Release)`; a
  loser spins `session_state.load(Acquire)` until `DONE`, with the same bounded spin as
  `calibrate()` and for the same reason no `Mutex`. Every reader — winner or loser — loads the two
  halves `Relaxed` **after** that `Acquire`. **The `Release` here matches the `Acquire` every
  reader takes before loading the two halves** — without it the pair has no publication word and a
  reader concurrent with the mint can observe a half-written id `(lo, 0)`, which is a 128-bit value
  equal to no mint; the two artifact headers then identify different sessions, and this module
  publishes nothing that would let either side detect that. "Minted once" — on `session_id()` in
  §API, in §API's prose, and again in the writers table above — binds the COUNT, not the
  publication: a `session_lo` CAS alone satisfies "once" and still leaves `session_hi` unordered
  with respect to it.
  **`invariant` needs no such clause, and the asymmetry is principled:** it is one word, so it
  cannot tear, and CPUID is deterministic, so a lost race stores the value the loser would have
  stored. Neither property holds for a 128-bit id split across two words.
- **`ticks_per_ns()`**: `state.load(Acquire)`; if not `DONE`, return `1.0` and
  `raise(DiagFlag::ClockUncalibrated)`. Otherwise
  `f64::from_bits(ticks_per_ns_bits.load(Relaxed))` — `Relaxed` suffices **because the `Acquire`
  on `state` already ordered it**.
- **`clock_epoch()`**: `epoch.load(Acquire)`. **`note_forward_jump()`**:
  `epoch.fetch_add(1, Release)` then `raise(DiagFlag::ClockEpochBreak)`. **`Release`/`Acquire`
  pairs so that a consumer which observes the incremented epoch also observes the counters the
  detector wrote before bumping it.** On x86-64 both lower to a plain `mov`, so the ordering
  costs nothing; **it is written correctly anyway rather than relying on the ISA.**

The pairing is what makes a record that *straddles* an epoch bump legible on both sides: a
reader that sees the new epoch has, by the pairing, also seen everything the detector recorded
before it.

---

## Backends

`ticks()` has exactly two arms and **neither is FFI** — this is what makes the zero-dependency
claim hold:

- `#[cfg(target_arch = "x86_64")]` → `core::arch::x86_64::_rdtsc()`.
- everything else → a monotone `std::time::Instant` delta from a lazily minted base, with
  `ticks_per_ns() == 1.0` and `invariant_tsc() == false`.

**On Windows `Instant` *is* QPC internally**, so the record's "QPC fallback" is honoured without
a `windows-sys` dependency; a hand-declared `QueryPerformanceCounter` FFI is **not** written,
because a second per-OS backing implementation is exactly the breach the never-freed-storage
boundary exists to prevent ([`04-STORAGE.md`](04-STORAGE.md)).

Verified against the tree: **there is no `rdtsc` and no QPC site anywhere today.** This module is
entirely new code; there is no existing clock site to migrate and no second spelling to
reconcile.

---

## `unsafe` obligations

Two, and each carries its `// SAFETY:` text verbatim.

```rust
// SAFETY: the `#[cfg(target_arch = "x86_64")]` gate guarantees the RDTSC instruction
// exists (architectural on x86-64 since its introduction; no CPUID feature bit gates its
// PRESENCE, only its invariance). The intrinsic has no memory operands, reads no pointer
// and has no side effects, so it cannot violate any aliasing or initialisation invariant.
unsafe { core::arch::x86_64::_rdtsc() }
```

```rust
// SAFETY: leaf 0x80000007 is read ONLY after `__cpuid(0x8000_0000).eax >= 0x8000_0007`
// confirms the CPU implements that extended leaf. Without this guard a CPU returns the
// highest leaf it does implement and EDX bit 8 is read from unrelated data. `__cpuid`
// writes no memory and takes no pointer.
unsafe { core::arch::x86_64::__cpuid(0x8000_0007) }
```

**The two-step CPUID probe is mandatory, not defensive.** A single-step read of leaf
`0x8000_0007` on a CPU that does not implement it returns the highest leaf it *does* implement,
and bit 8 of that unrelated word is then reported as "invariant TSC".

**Miri cannot cover either arm** — it has no x86 intrinsic support. Both are `#[cfg]`-excluded
under `cfg(miri)` in favour of the `Instant` backend, and the intrinsic arm's correctness rests
on the two SAFETY arguments above, **not on a test**. Stated rather than papered over; see
[`05-LADDER-GATES.md`](05-LADDER-GATES.md) §Miri.

---

## `.bss` residency

`ClockGlobals` is all-zero at const-init, so the linker emits it with a virtual size and **no
raw data**. **64 B.**

That is the whole of the claim. That the OS leaves the page uncommitted until touched is
**UNPROVEN** and is not asserted here; the limit of the residency argument is stated once, in
[`04-STORAGE.md`](04-STORAGE.md), and gate DG6 proves exactly the checkable half.
