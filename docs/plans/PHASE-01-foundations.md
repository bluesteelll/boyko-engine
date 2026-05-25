# Phase 1 — Foundations: UB / leak / race fixes

**Status:** ✅ DONE (Phase 1a + Phase 1b + Phase 1b-finish)
**Branch:** `ecs`
**Closed audit IDs:** C-001, M-001 (× 2), M-002, C-002, Q-004, Q-010,
C-003, M-015, Q-005, C-004 (partial), M-003, C-018, M-004, M-008,
C-008, Q-002, Q-022, Q-026 (partial), Q-001, Q-017, W6, 14 × E0133.

## Goal

Eliminate every UB / data race / resource leak that breaks the engine
the moment it is exercised outside a hand-crafted test. The engine
was correct **by luck** before this phase; it had to become correct
**by construction**.

## Why first

Every subsequent phase reads or writes through `Arena`, `ComponentPool`,
`ArchetypeMaster`, and the global registries. If those leak,
race, or carry dangling pointers, no performance or refactor work is
trustworthy. Foundations land first, full stop.

## Sub-phases

### Phase 1a — Critical UB and leaks

**Audit IDs closed:** C-001, M-001, M-002 / C-002 / Q-004, M-003,
C-018, M-008, C-008, Q-002, Q-022, Q-026 (partial), 14 × E0133.

**Key fixes:**

- **C-001 self-referential `EcsMaster`** — `Arena` moved into
  `Box<Arena>` so the `NonNull<Arena>` in `ArchetypeMaster` survives
  the `EcsMaster::new` return.
- **M-001 `Arena` `Drop`** — added the missing `impl Drop`
  invoking `std::alloc::dealloc`. Stops the 64 MB leak per
  `EcsMaster::new`.
- **M-002 / C-002 / Q-004 registry race** — replaced
  `static mut LAYOUTS: [Option<ComponentLayout>; N]` with
  `[OnceLock<ComponentLayout>; N]`. No more partial reads.
- **M-003 `Arena` UnsafeCell aliasing** — added `// SAFETY:` blocks
  documenting that allocate paths take `&mut self` exclusively in
  every visible call chain.
- **C-008 `debug_assert!` with side-effect** — extracted
  `pop_entity()` out of the macro argument; pops always happen in
  release.
- **E0133 (Rust 2024)** — wrapped every `unsafe` call inside
  `unsafe fn` bodies in explicit `unsafe { … }` blocks with
  `// SAFETY:` comments.

**Result:** `cargo check` clean under Rust 2024. Miri (tree-borrows)
clean except for the Q-001 macro path (closed in 1b-finish).

### Phase 1b — Drop discipline + ID collision

**Audit IDs closed:** M-001 cont., M-004, C-003 / M-015, Q-005,
C-004 (partial typed-API), Q-026 (partial).

**Key fixes:**

- **M-001 cont. type-erased component `Drop`** — added
  `drop_fn: unsafe fn(*mut u8)` to `ComponentLayout`. Every
  `ComponentPool::swap_remove` and `ComponentPool::Drop` now invokes
  it. Components holding `Vec` / `String` / `Box` no longer leak.
- **M-004 `swap_remove` Drop + dirty marks** — `drop_fn` called on
  the removed slot **before** the copy from last; both source and
  destination chunks marked dirty.
- **C-003 / M-015 `ComponentId` collision** — `register_layout`
  now compares `TypeId` on duplicate IDs; mismatch panics with the
  conflicting type names instead of silently overwriting.
- **Q-005 `EventId` collision** — same pattern applied to the event
  registry.
- **C-004 typed API guard** — `ComponentPool::add_typed::<T>` and
  `set_component_typed::<T>` check `T::component_id() ==
  self.component_id` in debug. The raw byte-API remains and is
  closed by Phase 4c.

### Phase 1b-finish — Q-001 Event derive UB

**Audit IDs closed:** Q-001, Q-002 (re-check), Q-017, W6.

**Strategy chosen:** Strategy (a) — `#[event]` attribute macro
rewrites the user struct into native `{ participants: P, parameters: Q }`
nested fields. The previous `*const Self → *const Parameters` cast
(UB without `#[repr(C)]` on the outer struct) is gone.

**Commits on `ecs`:** `c12cba7`, `a618e6e`, `5f35c70`, `6ba2d38`.

**Subsumed findings:**

- **Q-017** — `to_bytes() → Vec<u8>` double allocation is gone.
  `push` now uses `ptr::copy_nonoverlapping` from `&P` into
  `Vec<MaybeUninit<u8>>`.
- **W6** — `push_raw` / `get_raw` removed (zero callers post-rewrite).
- **Q-002** — re-checked; `ptr::read_unaligned` not needed because
  the new path stores into aligned `MaybeUninit<u8>` storage.

## Exit criteria — all met

- [x] `cargo check --all-targets` clean (0 errors, 0 E0133 warnings).
- [x] `cargo clippy --all-targets -- -D warnings` passes (clippy
      pedantic backlog deferred to Phase 5a).
- [x] `cargo test --all-targets` green (152 tests at end of 1b).
- [x] `cargo +nightly miri test` clean under
      `-Zmiri-tree-borrows` for `event_attribute` and `drop_fn` suites.
- [x] No `static mut` remaining in production crates.
- [x] All `unsafe` blocks carry `// SAFETY:` comments.
- [x] Author tags: `Celtokisa <bluesteelll@hotmail.com>` only.

## What this phase did NOT do

- It did **not** touch hot-path performance — that is Phase 2.
- It did **not** redesign the type-erased byte API end-to-end —
  raw `add(&[u8])` still exists for internal use. Phase 4c closes it.
- It did **not** introduce newtype IDs (`EntityId(usize)` etc.) —
  that is Phase 4a.

## Cross-phase residuals

Findings raised in audit but **deferred** by design from Phase 1:

| Audit ID | Reason for deferral | Resolved in |
|----------|---------------------|-------------|
| Q-007 EventPool Drop | `event_pool.rs` is `/* */` — design open | Phase 3d (blocked) |
| C-006 / C-007 / C-009 lifecycle | Non-UB but architecturally important | Phase 3a (DONE) |
| Q-008 / M-010 orphan files | Product decision required | Phase 2c / 3b (DONE) |

## References

- Audit: [`docs/AUDIT-2026-05-23.md`](../AUDIT-2026-05-23.md) §
  C-001, M-001, Q-001 (and all other listed IDs).
- Legacy roadmap: [`docs/ROADMAP-PHASE-2-PLUS.md`](../ROADMAP-PHASE-2-PLUS.md)
  §§ "Closed by Phase 1a / 1b", "Phase 1b finish".

## Lessons carried forward

- **Boxing the arena was non-negotiable** — every later phase that
  considers swapping allocators must preserve the
  `Box<Arena>`-or-equivalent guarantee that `Arena`'s address is
  pinned for the lifetime of `EcsMaster`.
- **Registries are `OnceLock` forever** — do not regress to
  `static mut`. Phase 6 `EventDispatcher` follows the same pattern.
- **`debug_assert!` is for invariants, not side-effects** — every
  agent must check that any expression inside a `debug_assert!` is
  pure. Catching this kind of regression is part of code-reviewer's
  remit going forward.
