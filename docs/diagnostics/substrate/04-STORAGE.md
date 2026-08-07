# Substrate A4 — `storage` (the never-freed policy and its gate helper)

<!-- CONTRACT
provides: substrate/never-freed-storage   # S12's storage policy, stated once, as a RULE
provides: substrate/section-report        # the ONE .bss section probe for both plans
assumes:  substrate/dedup-rationale       # A4 is one of the four duplications this crate removes
assumes:  substrate/lane-registry         # get_ptr's caller obligation is discharged BY the lane topology
-->

> **Carved from** `docs/DIAGNOSTICS-SUBSTRATE-PLAN.md` §3 A4 (in full), §4's `VmReservation`
> row rationale, and §11 F6 / F7 / F8; plus the one sentence of the logging plan's Invariant 7
> that assigns the section probe to this crate.
> Diff against those files until the monoliths are retired.

**The consequence of not sharing this, in one line:** two residency proofs over two statics with
two demand-zero arguments, and the toolchain behaviour both admit is unproven (PE/COFF `.bss`
placement) gets proved twice — so a toolchain change reds one gate and not the other, and the
reader cannot tell which is authoritative.

---

## API

```rust
#[repr(transparent)]
pub struct SyncCells<T, const N: usize>(UnsafeCell<[T; N]>);

pub const fn assert_zero_init_eligible<T: ZeroInit>() -> bool;

#[cfg(feature = "section-gate")]
pub fn section_report(sym: &str) -> SectionReport;
```

---

## The policy, stated once (S12), verbatim

> **Extent known at compile time ⇒ `.bss` static. Extent chosen at run time from config ⇒
> `VmReservation`, which requires the owner to sit at or above `boyko_ecs`.**

It is stated **here, once, as a RULE** — not in each plan as a per-plan exception plea. A rule
that each subsystem restates as its own special case is a rule that two subsystems can state
differently.

---

## The boundary is FORCED, not chosen

`VmReservation` cannot move down. Every citation below was re-read from
`crates/boyko_ecs/src/ecs/memory/vm.rs` this session:

| Fact | Site |
|---|---|
| `pub(crate) struct VmReservation` | `vm.rs:85` |
| `pub(crate) fn reserve(len: usize) -> Self` | `vm.rs:109` |
| `pub(crate) fn os_len(&self) -> usize` | `vm.rs:190` |
| `pub(crate) fn commit(&self, old: usize, new: usize)` | `vm.rs:199` |
| `libc::mmap` (unix arm) | `vm.rs:149` |
| `libc::mprotect` | `vm.rs:242` |
| `impl Drop for VmReservation` | `vm.rs:263` |
| `libc::munmap` | `vm.rs:286` |
| the single-source-of-truth clause | `vm.rs:12-18` |

A std-only zero-dep `boyko_diag` could host it only by **either** taking a third-party
dependency (forbidden) **or** minting a **second** hand-declared per-OS backing implementation
against `vm.rs:12-18`'s clause:

> *"These cfg arms are THE per-OS backing implementation for the whole engine: `ComponentPool`
> (Phase X.I) and `InlandStore` (Phase X.G) build on them."*

**Inventing memory backing twice is a worse Principle-0 breach than the one this crate fixes.**
The boundary at `boyko_ecs` is therefore not a preference; it is what the tree already decided.

*(Citation note: the decision record gives the clause as `:12-17`. The block runs `:12-18` — `:12`
is the heading `# Single source of truth (Phase X.H)` and `:18` closes it. Both point at the same
clause.)*

---

## `SyncCells` and its `unsafe`

```rust
// SAFETY: `SyncCells` grants no `&T` and no `&mut T`. Every access goes through
// `get_ptr(i)`, whose caller carries the single-writer obligation stated on that
// function. The type itself therefore adds no aliasing beyond what a raw pointer
// already permits, which is what `Sync` asserts here.
unsafe impl<T: Send, const N: usize> Sync for SyncCells<T, N> {}

/// # Safety
/// The caller guarantees that (1) `i < N`, and (2) for the lifetime of the returned
/// pointer, no other thread writes index `i`. In this crate obligation (2) is
/// discharged by A2: index `i` is written only by the thread whose `lane() == i`,
/// and a lane is owned by at most one thread at a time (the `release_lane` Release /
/// `claim_lane` Acquire pairing).
pub unsafe fn get_ptr(&self, i: usize) -> *mut T;
```

**Obligation (2) is discharged by the lane topology and by nothing else** — which is why this
file assumes `substrate/lane-registry`. If the lane topology stops being single-owner, this
`unsafe impl Sync` stops being sound, and the two facts must move together.

Miri under Tree Borrows is the instrument for both the `unsafe impl Sync` and the raw-pointer
discipline; see [`05-LADDER-GATES.md`](05-LADDER-GATES.md).

---

## `assert_zero_init_eligible` — what it can and cannot express (F6)

The record's comment reads *"`const`: `T: Zeroable` && extent is a const"*. **Only the first half
is expressible.**

- ***`T` is zero-initialisable*** — **expressible**, via a marker trait defined **in this crate**
  (`ZeroInit`), because **`bytemuck::Zeroable` is third-party and forbidden here**. `ZeroInit` is
  an `unsafe trait` implemented for the integer/atomic primitives and for `#[repr(C)]` structs
  whose fields are all `ZeroInit`; **a type with a `Drop`, a niche (`NonZeroU32`), or a reference
  cannot implement it.**
- ***the extent is a const*** — **NOT expressible, and NOT needed**: Rust array lengths are
  already const *by construction*. An extent read from `ProfilerConfig` cannot be written as
  `[T; n]` at all; it forces a `Vec` or a reservation, i.e. the *other* arm of the policy.
  **The compile-time half of the policy is enforced by the language, and the plan must say so
  instead of gating it.**

The compile-fail red therefore lands **on the expressible half only** — gate DG7 — and DG7's row
says in as many words that the second half is not gated, because **there is no broken input to
construct**.

**F8 — the compile-fail red cannot be a `#[test]`.** The record specifies "a `#[test]` that must
fail at compile time". A `#[test]` that fails to compile fails **the whole test binary's build**,
so it cannot be a *passing* gate. `trybuild` is the workspace's existing mechanism — verified as
a dev-dependency of `boyko_ecs` (`Cargo.toml:55`), `boyko_rhi_vulkan` (`:82`) and `boyko_ui`
(`:45`).

---

## `section_report` is feature-gated, off by default (F7)

**As specified it violates this crate's own mute-leaf rule.** It shells out to a binary
inspector — it opens a process and reads a file, which is precisely what the leaf may not do.

**The resolution is the pattern the tree already uses twice**, both verified this session:

| Precedent | Feature declaration | Self-referential dev-dep that switches it on |
|---|---|---|
| `boyko_rhi_vulkan` | `default = []` / `goldens = []` at `Cargo.toml:22-23` | `:94-99` |
| `boyko_render` | `default = []` / `test-readback = []` at `Cargo.toml:17-27` | in the same manifest's `[dev-dependencies]` |

`section-gate` is declared the same way, is **`default = []`**, and is switched on by **each
consumer's dev-dependency**. A default build compiles **no `std::process` and no `std::fs`
reference at all**, which is what DG9 asserts.

**One implementation for both plans.** `boyko_diag::section_report` is the ONE implementation of
the `llvm-readobj`/`objdump` section probe for **substrate DG6, profiling G22a/G22b/G23 and
logging G3**, so a PE/COFF toolchain change **reds one gate rather than splitting two that
disagree about which is authoritative.**

### MEASURED on this machine: the tool is not installed

Re-measured this session:

```
llvm-readobj: ABSENT     objdump: ABSENT     nm: ABSENT     llvm-nm: ABSENT     readelf: ABSENT
active toolchain bin/:  gcc-ld  rust-lld.exe  rust-objcopy.exe  wasm-component-ld.exe  (+2 dlls)
```

No `llvm-readobj` / `objdump` / `nm` / `llvm-nm` is on `PATH`, and the active
`stable-x86_64-pc-windows-gnu` toolchain ships **only** `rust-objcopy` and `rust-lld` — the
`llvm-tools` component is **not installed**. **The whole `.bss` gate family cannot run on this
machine as written.**

Therefore: **the gate resolves its tool at start and treats absence as a RED, never a SKIP.**
`rustup component add llvm-tools` is a **D0 line item**. A skip-on-absent gate is green on every
machine that lacks the tool, which is this one — the exact vacuity this campaign has now caught
nine times.

---

## The `.bss` residency argument, and its limit

A `static X: SyncCells<T, N>` whose const initialiser is all zeroes is emitted by the linker
with a virtual size and **no raw data** — `.bss` on ELF, and on PE/COFF a section whose
`SizeOfRawData` is 0 while `VirtualSize` is `N`.

**That much is mechanically checkable and DG6 checks it.**

**What is UNPROVEN and must not be claimed: that the OS leaves those pages uncommitted until
touched.** The image tells us the bytes are not in the file; **it does not tell us the loader is
lazy.** The gate proves **absence of raw data in the image**, and the plan claims exactly that
and no more.

**This limit travels with every "resident ≈ 0" figure anyone states anywhere**, and it is stated
here once so that no plan has to re-derive it: a resident-bytes figure rests on demand-zero
paging, and **demand-zero paging is exactly the half this gate does not prove**. Whoever states
such a figure carries the limit with it; the substrate asserts only its two checkable halves —
**nothing in the image** (DG6) and **nothing written at process start** (DG12) — and the two
together still do not add up to a residency proof.
