> STATUS: COMPLETED — archived 2026-07; implemented on branch `ecs`. See git history + the phase/feature RESULTS docs for the authoritative record.

# Phase 9 — `force_alloc_panic` Release-Mode Allocation Discipline

**Branch:** `ecs`
**Step:** Wave 7 Step 24 (plan §14 / §9.2 / Round 3 O-NEW-1)

This document describes the `force_alloc_panic` cfg gate that escalates
the Phase 9 ALLOC1 invariant from a dev-only `debug_assert!` to a
hard panic in release builds. The gate is the release-mode safety net
required by Round 3 O-NEW-1 and the §13.6 invariant table.

---

## §1 — Background: ALLOC1 invariant

Phase 9 plan §2.7 establishes the **no-allocation-in-system** rule:

> No `Arena::allocate_*` call may occur inside the body of a
> `System::run_unsafe` invocation. All allocation must happen on the
> dispatcher thread (during `ScheduleBuilder::build` or during the
> apply window).

The rationale is structural:

- `Arena` is `!Send + !Sync` — concurrent worker access would race in
  `MemFreeBlockMaster` (Round 2 C1 resolution).
- Even if `Arena` were `Send + Sync`, allocation inside a worker body
  would invalidate the apply-window aliasing contract (SCH7) by
  forcing a `&mut EcsMaster` reborrow inside the worker.

Today, the discipline is enforced by:

- A thread-local `IN_SYSTEM_RUN: Cell<bool>` flag, set by the
  `InSystemRunGuard` RAII around every `System::run_unsafe` call.
- A `debug_assert!(!boyko_threadpool::is_in_system_run())` at the head of
  `Arena::allocate_layout` and `Arena::allocate_from_free_blocks`.

`debug_assert!` compiles to a no-op in release. A future refactor that
accidentally allocates inside a system body would pass tests in release
and only fail in dev — too late for users who only build release.

---

## §2 — The `force_alloc_panic` cfg gate

The Wave 7 Step 24 update extends `arena.rs` with a stronger,
cfg-gated assertion that fires in release **when** the user opts in via
`RUSTFLAGS="--cfg force_alloc_panic"`.

```rust,ignore
// crates/boyko_ecs/src/ecs/memory/arena.rs
#[cfg(any(debug_assertions, force_alloc_panic))]
{
    if boyko_threadpool::is_in_system_run() {
        panic!(
            "Phase 9 ALLOC1 violation: Arena::allocate_* called inside \
             System::run_unsafe. ..."
        );
    }
}
```

When neither `debug_assertions` nor `force_alloc_panic` is set (the
normal release-mode path), the check is compiled out entirely — zero
runtime cost.

---

## §3 — Usage

### Build with the gate enabled

```powershell
$env:RUSTFLAGS = "--cfg force_alloc_panic"
cargo test --release
cargo build --release
```

### CI integration

Add an additional CI job that runs the full test suite under the gate:

```yaml
  force-alloc-panic:
    name: cargo test --release (force_alloc_panic)
    runs-on: ubuntu-latest
    env:
      RUSTFLAGS: "--cfg force_alloc_panic -D warnings"
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - uses: Swatinem/rust-cache@v2
      - run: cargo test --all-targets --release
```

The job catches any future regression where a refactor adds an
allocation inside a system body. Without the gate, such a regression
would silently pass in release.

### Local pre-commit check

A developer-side hook can run the gate before committing scheduler
changes:

```powershell
$env:RUSTFLAGS = "--cfg force_alloc_panic"
cargo test --release --test scheduler_par_iter_concurrent_systems
cargo test --release --test miri_phase9
```

---

## §4 — Today's status

The Wave 7 update to `arena.rs` adds the cfg-gated check alongside the
existing `debug_assert!`. The change is **strictly additive**:

- Existing `debug_assert!` continues to fire in dev builds.
- New cfg-gated panic fires in release **only** if
  `--cfg force_alloc_panic` is set.
- Normal release builds remain check-free.

See `crates/boyko_ecs/src/ecs/memory/arena.rs` for the implementation.

---

## §5 — Relationship to plan §9.2

Plan §9.2 (release-mode enforcement, Round 3 O-NEW-1) calls out this gate
as **the** mechanism for closing the ALLOC1 gap in production binaries
that opt in. The gate is exposed via `RUSTFLAGS` rather than a Cargo
feature because:

- It is a **discipline check**, not a code-path selection.
- Cargo features cascade through dependencies; a `RUSTFLAGS` cfg gate
  is local to the build command line.
- The check should be invisible in normal release builds — no feature
  flag should accidentally pull it in.

The Round 3 plan defers the question of "should the gate be on by
default in release?" to a future phase. Today the conservative position
is "opt-in via `RUSTFLAGS`, enforced in CI".
