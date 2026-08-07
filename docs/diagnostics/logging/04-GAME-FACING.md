# Logging — the game-facing surface

<!-- CONTRACT
provides: logging/game-facing-surface
assumes:  substrate/clock-source
assumes:  substrate/loss-vocabulary
assumes:  logging/goal-and-audiences
assumes:  logging/emission-path
assumes:  logging/ring-and-statics
assumes:  logging/sink-lifecycle
assumes:  logging/registry-and-walker
-->

> Carved from `docs/LOGGING-SYSTEM-PLAN.md` (v4) — §"Key decisions — the scope extension", Decisions 16, 17, 18, 19b, 20, the reader half of Decision 26, the game-facing blocks of §Data structures, and the game slice of §Public API. Diff against that document until the monolith is retired.

---

## Key decisions — the scope extension (games as a first-class audience)

The four decisions that answer "a game must be able to read its own diagnostics" live here. The
four that a game *configures* rather than reads live next door and are not restated:

| What a reader may be looking for here | Where it actually lives |
|---|---|
| Downstream code tables, `CodeIdx::Dynamic`, `codes_tidy!`, exhaustion at 100 % | `03-CODES-REGISTRY.md` (Decision 19) |
| The session-scale integer audit (`seq_lo`, every `BinarySink` width) | `01-EMISSION-RING.md` (Decision 21) |
| `BinarySink` and its revert clause; runtime control (`CONTROL` CAS, `SINK_REQ`); `LogRuntimePreset`; the `ECS_HANDOFF` transport structure | `02-SINK-LIFECYCLE.md` (Decisions 22, 23, 25, 26) |
| `LogPod`'s place in the emission path's re-entrancy argument | `01-EMISSION-RING.md` (Decision 13) |

---

### Decision 16: What "as much data as possible" can and cannot mean here

The ask is real, and one common answer to it is wrong for this engine: **enlarging the ring does
not raise the capture rate.** The ring's job is to absorb burstiness between a producer offering
up to 66 M records·s⁻¹ (15 ns/record) and a consumer formatting at ~500 K·s⁻¹. Enlarging it moves
the loss point later; it does not move the *ceiling*, which is `core::fmt` on the sink thread.
Four mechanisms actually move the ceiling or make the loss honest, and each is a separate
decision:

1. **Do not format** — `BinarySink` writes `{site_id, tsc_delta, len, flags, payload}` and defers
   formatting to an offline decoder (Decision 22, `02-SINK-LIFECYCLE.md`). This is the only change
   that moves the throughput ceiling, and it ships **with a revert clause** (G12c): if it does not
   measure ≥ 5× the text sink in the same sitting, L13b is reverted rather than justified.
2. **Emit less, on purpose, and say so** — per-target sampling (Decision 20), whose census status
   is `UNPROVEN(sampled)` so a sampled count can never be read as a total.
3. **Keep the loss count honest at session scale** — power-of-two `EveryN`, cursor-wrap
   correctness, and `u64` accumulation that never saturates (Decision 21,
   `01-EMISSION-RING.md`). A session is hours; every `u32` in the design was audited against that.
4. **Do not discard the beginning of the capture silently** — rotation reports what it deleted
   (`W0112`, E21), and `Rotation::NONE` stays the engine default so a bench cannot lose its own
   start.

**What this plan will not do:** promise a lossless capture. It promises that loss is *counted,
attributed to a target, and rendered as a status a reader cannot mistake for a total* — and that
promise is gated by G11 and P2 at session scale, not by a 300-frame argument.

---

### Decision 17: Per-target statistics are the game's read surface — and the census is where a vacuous gate goes to die *(status vocabulary unified by S8)*

`TARGET_STATS: [TargetStatCell; MAX_TARGETS]` (16 KiB `.bss`, one 64 B cell per target, written by
the consumer role, readable by anyone) carries `delivered` / `dropped` / `sampled_out` /
`sync_routed` as `u64`. `LogCensus` (a `Resource`, `VmColumn`-backed) is its ECS-visible snapshot,
refreshed once per drain.

**One status vocabulary for both diagnostics subsystems** — `boyko_diag::LossStatus`
(`../substrate/03-LOSS.md`), so a reader who has learnt the tokens in one artifact has learnt them
in the other:

| `LossStatus` | Meaning here |
|---|---|
| `Measured` | records were delivered; the counts are totals |
| `Unproven` | zero records — **never** `clean` |
| `UnprovenLossy` | `dropped > 0`; the counts are lower bounds |
| `UnprovenSampled` | the target's shift is non-zero; `delivered` is `1/2^k` of the truth |
| `UnprovenUnsunk` | **no `Active` sink's filter accepts this target** — a game enabled a category, saw nothing, and would have concluded "clean". `boyko-W0111` fires once |

*(v3 had a sixth, `dropped=SATURATED(>=4294967295)`. S8 widens the counters to `u64`; there is no
ceiling state left to name, and a token the census could never let a reader **compare** stops
existing. `logging/dispositions` §Refuted records why the v3 rejection of `u64` does not survive.)*

**Type name vs printed form, stated once so the two spellings elsewhere in this corpus are not
read as two vocabularies**: the *type* is `boyko_diag::LossStatus` with the variants above; the
census's *rendered* form keeps the v3 text — `status=UNPROVEN`, `UNPROVEN(lossy)`,
`UNPROVEN(sampled)`, `UNPROVEN(unsunk)`, `MEASURED` — because those strings are what a support
ticket quotes and what a reader greps. Every `UNPROVEN(x)` appearing anywhere in this corpus is
the rendered form of the corresponding `LossStatus` variant.

`LogCensus.lossy` is the single bit a UI must read before rendering any count as a total, and
`G15` gates that the bit exists and flips. **The single game-facing surface is one `Resource`** —
`DiagCensus { log: LogCensus, prof: ProfCensus, lossy: bool }` (S8) — so a game asks one question
about diagnostic completeness rather than two that can disagree.

---

### Decision 18: Dynamic targets — 32 slots, interned by name, and the cost of losing gate (a) is stated

A game or mod names a category from data (`"mod:acme_weapons"`, a script namespace, a save-file
field). `register_dynamic_target(name, initial) -> Option<TargetId>` is **cold, setup-time and
idempotent by name**: it hashes into `DYN_NAMES`, an open-addressed, insert-only, fixed-capacity
table of 32 cache-line slots in `.bss`. Not a map: no rehash, no growth, no allocation, and **the
emission path never touches it** — emission carries the `TargetId`, and the name is resolved by
the sink.

Emission uses `dyn_info!(id, …)` / `dyn_warn!(id, code, …)`, which have **two** gates instead of
three: `T::STATIC_CEILING` does not exist for a target that is not a type. The cost is real and is
not smoothed over: a dynamic site cannot be compiled out per-target, only by `GLOBAL_CEILING`. The
bench `log_dyn_disabled` bounds it at ≤ 4 ns, and **G8d turns the comparison into a claim that can
be withdrawn**: if `log_dyn_disabled − log_disabled_runtime` does **not** resolve above the
sitting's floor, then the per-target `const` ceiling's benefit is unproven on this box and
Decision 2's claim about gate (a) is **struck from this corpus** rather than restated.

**Why 32 and not "unbounded"** — see `logging/dispositions` open question 8. Every slot comes out of
the 256-target space that `CONTROL`, the sink filters (`[u64; 4]`) and `TARGET_STATS` are all
sized by; past 256 those three arrays become two-level structures. 32 data-defined categories is a
lot; needing more is a signal that the taxonomy belongs in source.

**Registration runs only after the runtime flag is on.** `register_dynamic_target` is `#[cold]`
and idempotent, and it is not called under a flag that is off — the `DYN_NAMES` table is a
reserved `.bss` extent that a flag-off run never touches. The rule this follows, and its cost
table, belong to the seam; the fact that this table is reserved rather than resident is stated
where the number lives, in `01-EMISSION-RING.md`'s `.bss` budget matrix.

---

### Decision 19b: `LogPod` — game types as arguments, encoded FIELD BY FIELD *(re-cut by B10)*

**The defect first.** v3 wrote `const POD_LEN: usize == size_of::<Self>()` and "the encode half is
ours — a `copy_nonoverlapping` of `POD_LEN` bytes", with `fn fmt_pod(bytes: &[u8], …)` on the
sink. `#[derive(LogPod)]` requires `#[repr(C)]` and all-`LogValue` fields, and that **admits
padding**: `struct S { a: u8, b: u32 }` has three padding bytes whose contents are uninitialised.
Copying `size_of::<Self>()` bytes copies them, and the sink then materialises a `&[u8]` over
uninitialised memory — **UB regardless of whether `POD_LEN` is honest**, which also makes
"round-trips byte-identically" undefined as a property. G9b named the padded struct only as the
red state for a *lying* `POD_LEN`, so the correct implementation's own defect was uncovered.

**What (corrected).** The blanket byte copy is deleted. The trait requires an encoder:

```rust
pub unsafe trait LogPod: Copy + Send + Sync + 'static {
    /// SUM OF FIELD ENCODED LENGTHS — not `size_of::<Self>()`. Padding is
    /// never part of the record, so no uninitialised byte can reach a sink.
    const POD_LEN: usize;
    /// Writes EXACTLY `POD_LEN` initialised bytes at `dst`. This is the
    /// invariant a hand-written `unsafe impl` takes on; the derive discharges
    /// it mechanically.
    ///
    /// # Safety
    /// `dst` must be valid for `POD_LEN` writes.
    unsafe fn encode_pod(&self, dst: *mut u8);
    /// Runs on the SINK, over the `POD_LEN` bytes `encode_pod` wrote.
    fn fmt_pod(bytes: &[u8], f: &mut LogFormatter);
}
```

`#[derive(LogPod)]` in `boyko_macros` generates `encode_pod` as a **sequence of per-field
`LogValue::encode` calls** — the derive already requires every field to be `LogValue`, so this
needs no new capability — and generates

```rust
const _: () = assert!(<Self as LogPod>::POD_LEN == /* Σ field MAX_ENCODED_LEN */);
const _: () = assert!(<Self as LogPod>::POD_LEN <= MAX_RECORD_BYTES - HEADER_BYTES);
```

Fields whose `MAX_ENCODED_LEN` is `usize::MAX` (dynamic, i.e. `&str`) are **rejected by the
derive** with a named error, which is what keeps `POD_LEN` a `const` and keeps the sum
well-defined. `#[repr(C)]` is still required — not for the copy, which no longer exists, but so
that field *order* in the generated encoder is the declared order and a reordering is a visible
source change.

**Decision 13's structural property is untouched, and now for a better reason.** `encode_pod` is
generated code over `LogValue::encode`, so what runs between lane acquisition and the `Release`
store is still ours, still POD-only, still incapable of calling user `Display`. The user's
`fmt_pod` runs on the **sink thread, from the staging arena, in the same position as
`site.decode`**. Asserted, not argued: mandatory test 24 uses a `LogPod` whose `fmt_pod` sets a TLS
flag and requires the flag to be **unset** at the `Release` store and set only during drain.

A hand-written `unsafe impl` is still allowed and carries the stated burden ("`encode_pod` writes
exactly `POD_LEN` initialised bytes"). **G9b's subject changes** (`logging/gates`): the red
state is no longer "drop the `POD_LEN == size_of` assert" but "**replace the derived field-by-field
encoder with a `copy_nonoverlapping` of `size_of::<Self>()`**" ⇒ the padded-struct Miri leg reports
an uninitialised read. That is a red that responds to the defect v3 actually had. G9b still
**cannot make an arbitrary hand impl safe**, and says so.

The `*_kv!` macros (`info_kv!(Combat, "hit", dmg = d, target = t)`) put field **names** in the
`&'static LogSite`, which is cold and never touched on the emission path — so structured output
costs the same as positional output on every hot path.

---

### Decision 20: Sampling and sync-routing — two bits of `CONTROL`, both default-off, both gated with a revert clause

**Sampling.** `k = (ctl >> 3) & 0x0F`; when `k != 0`, deliver 1 record in `2^k`. The counter is
`SAMPLE_CTR[lane][target]`, a `u16` **written only by the lane's owner** (the row index *is* the
lane index), with plain `Relaxed` load/store and **never an RMW** — so it inherits the `LogLane`
SAFETY block's single-writer clause verbatim and costs no lock prefix. Seeded at claim time with
`(lane * 0x9E37)` so two lanes do not phase-lock.

**What sampling cannot claim**: that the capture is *representative*. `1/2^k` is **strided, not
random**; a periodic emitter aliased to `2^k` yields a systematically biased capture. The census
prints `sampling=1/N (strided, not random)`, `boyko-W0113` fires once per sampled target, and E23
states the residual. A footnote nobody reads is not a control; a line in the log is.

**Sync routing.** Bit 7 routes a target's records to the synchronous channel: format on the
caller, `write_oracle_line`, count `sync_routed`. It serialises the frame — that is the *point*:
it is the per-target opt-in for "this must leave the process before the next instruction", the
only partial answer to a hard crash (E22).

**What sync routing costs, and what it cannot claim** *(corrected by B9)*. v3 wrote "~200+ ns …
durable-on-write". Both halves needed work.

- **~200 ns is the uncontended, console-only figure.** With Decision 9c's durable fan-out the
  uncontended cost is one further `write_all` to the crash handle; **contended, the only bound is
  `OUT_LOCK`'s 50 ms acquire deadline**, after which the writer *steals* and the line may
  interleave with another synchronous line (E25). A mechanism whose reason to exist is integrity
  can therefore be interleaved under contention. That is the trade; it is not smoothed.
- **"Durable-on-write" means the bytes left the process, not that they reached the platter.**
  `sync_data()` is opt-in via `LogConfig.sync_durable` (default off) at ~0.1-10 ms per record,
  because a sync bit that also `fsync`ed would serialise the frame on the disk instead of on the
  format.
- In a profile with **no** synchronous destination the bit is inert, and the census reports the
  target as `UnprovenUnsunk` rather than letting a reader infer durability from a set bit.

Both branches are predicted-not-taken in every default configuration. **G10d decides whether
sampling ships default-on**: `log_enabled_0args` must be NOT RESOLVED against the pre-L12
baseline; if it resolves, `log-sampling` becomes a default-off feature and the ≤ 15 ns row is
annotated with the measured cost. The gate decides the rung's disposition; this corpus does not
pre-decide it.

---

### Decision 26 — the reader surface *(the transport itself is `02-SINK-LIFECYCLE.md`'s)*

`LogRing::since(cursor, &RingFilter) -> LogRingIter` returns records delivered since a monotone
`seq`, oldest first, with `LogRingIter::skipped` reporting how many the ring wrapped past — **a
console cannot silently miss lines**. The ring is fed by `log_drain_system` in `Last`, never from
the emission path: **G15b reds if a record is visible before the drain that consumed it**, which
is also what keeps the hot path from touching ECS storage.

**The stated bound** is "sink park interval + one frame" (≤ 2 frames in practice) under `Thread`,
and **one frame** under `Scheduled` (the drain and the ECS copy are the same system). G15 cannot
claim tighter. A per-frame **`frame_epoch` record** *(renamed from `EPOCH` — S11, three meanings
collided)* lets a reader attribute every record to exactly one frame; a record emitted *during*
the drain is attributed to the next frame, and mandatory test 29 asserts that rather than assuming
it.

The `HandoffRing` / `ECS_HANDOFF` structure that carries formatted lines from the consumer role to
the ECS — its layout, capacity, ordering, overflow accounting (`LossClass::Sink`, `W0117`,
`lossy`), presence rule and four-clause SAFETY block — is specified in `02-SINK-LIFECYCLE.md`. It
is named here only because every claim on this page rests on it: an undefined cross-thread queue
is exactly the object this campaign's defects live in, which is why v3's three bare references to
it were a blocker.

---

## Data structures — the game-facing blocks

```rust
// ────────────────────────── boyko_log/src/target.rs ───────────────────────────

/// Dynamic-target name interning. Open-addressed, insert-only, fixed capacity,
/// one cache line per slot. NOT a map: no rehash, no growth, no allocation, and
/// the emission path never touches it (Decision 18). 2 KiB .bss.
#[repr(C, align(64))]
struct DynSlot { hash: AtomicU64, len: AtomicU8, bytes: UnsafeCell<[u8; 47]> }
static DYN_NAMES: [DynSlot; DYN_BAND_LEN];
// SAFETY: `bytes`/`len` are written before `hash.store(h, Release)`; a reader
// that observes a non-zero hash via Acquire observes the completed name. A
// slot's hash transitions 0 -> h exactly once, by CAS. No slot is ever reused,
// so a published name is immutable for the process lifetime.

/// Sink-written, anyone-readable. NOT a mirror of anything: it is the only
/// place delivered-per-target counts exist (Decision 17). 16 KiB .bss.
#[repr(C, align(64))]
struct TargetStatCell { delivered: AtomicU64, dropped: AtomicU64,
                        sampled_out: AtomicU64, sync_routed: AtomicU64,
                        _pad: [u8; 32] }
static TARGET_STATS: [TargetStatCell; MAX_TARGETS];

/// One u16 row per LANE, one column per target. Written ONLY by the lane's
/// owner with plain Relaxed load/store, never an RMW (Decision 20, LogLane
/// SAFETY clause 1d). Row count follows `boyko_diag::LANE_COUNT` (S3):
/// 40 KiB in `dev`, 16 KiB in `shipping` (v3: 64 KiB at 128 lanes).
static SAMPLE_CTR: [[Cell<u16>; MAX_TARGETS]; LANE_COUNT as usize];

// ─────────────────── boyko_ecs seam: the ECS-visible surface ──────────────────

/// The durable, displayable log. Backed by the engine's own storage — a
/// `VmReservation`-backed byte column, NOT a `Box<[u8]>` heap side-store, which
/// is the shape Principle 0 was re-stated to forbid even inside a `Resource`
/// (M13). Fixed capacity, reserved at plugin build, never grows.
#[derive(Resource)]
pub struct LogRing {
    lines: VmColumn<LogLine>,  // engine storage
    arena: VmColumn<u8>,       // engine storage
    head: u32, len: u32, arena_cursor: u32,   // wrapping; Decision 21, test 20
    seq:  u64,                 // monotone record sequence — the reader's cursor
}

// ─── B1: `VmColumn` is !Send + !Sync; `Resource` requires both ───────────────
//
// Verified against the tree: `crates/boyko_ecs/src/ecs/memory/vm_column.rs:70`
// states verbatim "NOT `Send`/`Sync` (the `NonNull` inside `VmReservation` and
// `base`): owners that cross threads carry their own exclusivity argument in
// their manual `unsafe impl Send/Sync` (SEND10 on `Archetype` …)", and
// `crates/boyko_ecs/src/ecs/core/resources/resource.rs:42` reads
// `pub trait Resource: 'static + Send + Sync + Sized`. v3 declared `LogRing`,
// `LogStats` and `LogCensus` "ordinary `Resource`s" two sections after
// Decision 12 deleted `LogHandle` for exactly this rule — the fold did not
// compile. `LogStats` is `Copy` POD and derives both; the other two need the
// impl below.
//
// COMPILE-TIME PIN — this is the F7 treatment applied to `Send`/`Sync` instead
// of to size. A future field that is not `Send`/`Sync` fails HERE, not in
// `LogPlugin::build`:
const _: () = {
    const fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<LogRing>();
    assert_send_sync::<LogCensus>();
    assert_send_sync::<LogStats>();
};

// SAFETY (SEND10-shaped, for `LogRing` and `LogCensus`):
//   1. WHO MAY HOLD `&mut`: exactly one system, `log_drain_system`, which the
//      scheduler grants `ResMut<LogRing>` / `ResMut<LogCensus>`. The
//      scheduler's conflict analysis is what makes that exclusive; no other
//      system declares `ResMut` on either.
//   2. WHO MAY HOLD `&`: any system declaring `Res<..>` (a HUD, a console, a
//      telemetry reducer). The scheduler never runs a `Res` reader
//      concurrently with the `ResMut` writer, which is the same guarantee
//      every other `Resource` in this engine rests on.
//   3. WHO MAY NOT TOUCH THEM AT ALL: **the sink thread**. This is the clause
//      that makes the impl true rather than merely stated, and it is why B2
//      had to be answered first: the sink writes `ECS_HANDOFF` (a `.bss` byte
//      ring, Decision 26) and never names `LogRing` or `LogCensus`. If the
//      sink wrote the columns directly, clauses 1-2 would be false and the
//      only repair would be a lock — which Invariant 1 forbids.
//   4. WHY THE UNDERLYING COLUMNS TOLERATE IT — quoting `vm_column.rs:73-77`'s
//      own invariant list: `base` is write-once (set at lazy materialization
//      inside the `&mut self`-only `grow_to`, stable thereafter); every
//      mutation requires `&mut self`; cross-thread `&self` reads touch only
//      committed plain-old-data below `len` with no interior mutability.
//      `LogLine`, `u8` and `TargetStat` are all POD, so clause 4 holds for
//      every element type used here.
//   5. MATERIALIZATION IS NOT LAZY IN PRACTICE: `LogPlugin::build` calls
//      `grow_to` to the configured capacity BEFORE the schedule ever runs, so
//      the write-once `base` store happens once, single-threaded, at plugin
//      build. No `&self` reader can observe a partially materialized column.
unsafe impl Send for LogRing {}   unsafe impl Sync for LogRing {}
unsafe impl Send for LogCensus {} unsafe impl Sync for LogCensus {}

/// EXACTLY 16 BYTES, `Copy`, and pinned by a const assert *(fixes F7)*.
/// `crates/boyko_ecs/src/ecs/memory/vm_column.rs:144-149` panics in
/// `VmColumn::<T>::new` unless `COMMIT_GRANULE % size_of::<T>() == 0`, and
/// `COMMIT_GRANULE == 64 KiB` (`crates/boyko_ecs/src/ecs/constants.rs:7`).
/// v2's layout was 12 bytes (10 payload + align-4 tail): `65536 % 12 == 4`, so
/// `LogPlugin::build` would have PANICKED at construction and rung L5 could not
/// have landed green. `repr(C, packed)` does not save it either (`65536 % 10
/// == 6`). The fix is a size that divides the granule, not an attribute — and
/// the const assert below turns "someone adds a field" from a plugin-build
/// panic into a compile error. `VmColumn<T>` also requires `T: Copy`, hence the
/// derive; it is `pub(crate)` to `boyko_ecs` (`vm_column.rs:80`), which is fine
/// because `LogRing` lives inside `boyko_ecs` and the field is private.
#[repr(C)] #[derive(Clone, Copy)]
pub struct LogLine {
    start:  u32,   // offset into `arena`
    seq_lo: u32,   // low half of the record's sequence number
    len:    u16,   // bytes of formatted text
    code:   u16,   // 0 when the level carries none
    level:  u8,
    target: u8,    // MAX_TARGETS == 256 fits a u8 exactly
    flags:  u8,    // STR_TRUNCATED | SUPPRESSED_FOLLOWS | SAMPLED_CONTEXT
    _pad:   u8,
}
const _: () = assert!(core::mem::size_of::<LogLine>() == 16);
const _: () = assert!(COMMIT_GRANULE % core::mem::size_of::<LogLine>() == 0,
    "LogLine must divide COMMIT_GRANULE or VmColumn::new panics (F7)");
// `VmColumn<u8>` for the arena is trivially fine: 65536 % 1 == 0.

/// Monotonic counters; zero per-frame allocation. Mirrors the shape of
/// `crates/boyko_app/src/window_info.rs:34`'s `HostFrameStats` — counters only,
/// no timing fields, because that struct HAS no timing fields (checked).
#[derive(Resource, Clone, Copy, Default)]
pub struct LogStats {
    pub emitted: u64, pub dropped: u64, pub dropped_bytes: u64,
    pub suppressed: u64, pub unlaned_dropped: u64, pub sampled_out: u64,
    /// `LossClass::Sink` on `ECS_HANDOFF` — formatted lines that reached the
    /// byte sinks but not the in-frame view (Decision 26, `W0117`).
    pub handoff_lost: u64,
    /// Emissions that ran with no rate state because the 512-slot code-index
    /// space was exhausted (M3, `E0115`). Never an aliased slot.
    pub codes_unindexed: u64,
    pub lanes_claimed: u32, pub lanes_retired: u32,
    /// Spares held for the process by threads that never called
    /// `boyko_diag::release_lane()`; bounded at 14 (S3).
    pub lanes_leaked: u32,
}

/// Per-target counts for an in-game overlay / support HUD / telemetry payload.
/// `lossy` is the one bit a UI must read before showing a count as a total
/// (Decision 17).
#[derive(Resource)]
pub struct LogCensus {
    per_target: VmColumn<TargetStat>,   // MAX_TARGETS rows, engine storage
    /// `boyko_diag::SessionId` — ONE mint shared with the profiler's artifact
    /// header (S11), so an uploaded log and an uploaded artifact identify the
    /// same session. v3 minted its own.
    pub session: boyko_diag::SessionId,
    pub lossy: bool,
    pub control_epoch: u32,             // CONTROL_EPOCH_CTR at the last drain (S11 rename)
}

/// The SINGLE game-facing diagnostics surface (S8). A game asks one question
/// about completeness rather than two that can disagree — and the `lossy` bit
/// here is the OR of both subsystems', so a UI cannot render one as a total
/// while the other was dropping.
#[derive(Resource)]
pub struct DiagCensus { pub log: LogCensus, pub prof: ProfCensus, pub lossy: bool }
#[repr(C)] #[derive(Clone, Copy)]
pub struct TargetStat { pub delivered: u64, pub dropped: u64,
                        pub sampled_out: u64, pub sync_routed: u64 }
const _: () = assert!(core::mem::size_of::<TargetStat>() == 32);
const _: () = assert!(COMMIT_GRANULE % core::mem::size_of::<TargetStat>() == 0);
// NOTE: there is no `LogFilter`. `CONTROL` is the single owner (Decision 14).
```

Every static above is a **reserved** `.bss` extent, not a resident cost: `DYN_NAMES` (2 KiB),
`TARGET_STATS` (16 KiB) and `SAMPLE_CTR` (40 KiB dev / 16 KiB shipping) are all-zero at link time
and are not touched until a game registers a target, a sink drains, or a lane samples — none of
which happens while the runtime flag is off. The `.bss` budget matrix that owns these numbers, and
the reserved-vs-resident distinction, live in `01-EMISSION-RING.md`.

---

## Public API — the game slice

```rust
// ── dynamic targets ───────────────────────────────────────────────────────────
/// DYNAMIC targets: IDs 224..=255, minted from data/mod/script names.
/// COLD, setup-time, idempotent by name. `None` => band exhausted (`boyko-E0106`).
pub fn register_dynamic_target(name: &str, initial: TargetControl) -> Option<TargetId>;
pub fn find_target(name: &str) -> Option<TargetId>;         // #[cold], linear scan
pub fn targets() -> TargetIter<'static>;                    // #[cold], settings screens

// ── emission: dynamic targets (gate (a) unavailable — Decision 18) ────────────
#[macro_export] macro_rules! dyn_info  { ($id:expr, $fmt:literal $(, $a:expr)*) => {...} }
#[macro_export] macro_rules! dyn_warn  { ($id:expr, $code:expr, $fmt:literal $(, $a:expr)*) => {...} }
// + dyn_debug! dyn_trace! dyn_error!

/// Named-field form; names live in the cold `LogSite`, so this costs the same
/// as the positional form on every hot path (Decision 19b).
/// `info_kv!(Combat, "hit", dmg = d, target = t)`
#[macro_export] macro_rules! info_kv  { ... }   // + debug_kv! trace_kv! warn_kv! error_kv!

// ── game-extensible values ────────────────────────────────────────────────────
/// Game-extensible POD values. Blanket-bridged into `LogValue`, so sealing is
/// preserved and Decision 13's structural property is untouched: the encoder
/// is generated from `LogValue`, `fmt_pod` runs on the sink (D19b, test 24).
/// `POD_LEN` is the SUM OF FIELD ENCODED LENGTHS, not `size_of::<Self>()`, so
/// no padding byte ever reaches a sink (B10).
pub unsafe trait LogPod: Copy + Send + Sync + 'static {
    const POD_LEN: usize;
    /// # Safety
    /// `dst` valid for `POD_LEN` writes; the impl writes exactly that many
    /// INITIALISED bytes.
    unsafe fn encode_pod(&self, dst: *mut u8);
    fn fmt_pod(bytes: &[u8], f: &mut LogFormatter);
}
// boyko_macros: #[derive(LogPod)] — requires #[repr(C)] and all-LogValue
// fields, REJECTS dynamic-length fields (`&str`), and generates `encode_pod`
// field-by-field plus `const _: () = assert!(POD_LEN == Σ field lengths)`.

// ── reading the diagnostics back ──────────────────────────────────────────────
pub fn session_id() -> boyko_diag::SessionId;             // ONE mint (S11)
pub fn census() -> CensusIter<'static>;                   // Measured / Unproven per target

impl LogRing {
    /// Records delivered since `cursor`, oldest first. `cursor` is a monotone
    /// sequence number; a gap means the ring wrapped and `LogRingIter::skipped`
    /// says by how much — a console cannot silently miss lines (Decision 26).
    pub fn since(&self, cursor: u64, filter: &RingFilter) -> LogRingIter<'_>;
    pub fn cursor(&self) -> u64;
}
pub struct RingFilter { pub targets: [u64; 4], pub min_level: Level }

// ── ECS seam (boyko_ecs) ──────────────────────────────────────────────────────
pub struct LogPlugin { pub config: LogConfig }
impl Plugin for LogPlugin { fn build(&self, app: &mut App); }
// inserts LogRing / LogStats / LogCensus; adds `log_drain_system` to `Last`
// (ECS ring feed + TARGET_STATS snapshot + one `frame_epoch` record per frame —
// the sink thread owns the byte sinks). Registers `shutdown` on teardown.
```

No `Vec`, `Box<dyn>`, `HashMap` or internal type appears in any signature.

---

## What a game may NOT do with these numbers

Gameplay **may not branch on log counters**: they are lower bounds under drop, schedule-dependent,
non-deterministic across machines, and therefore break replay. Display and telemetry only; gameplay
counters belong in the game's own components, which is Principle 0's answer. The refusal and its
reasoning are recorded once, in `logging/dispositions` §Refused, so they are not re-derived.
