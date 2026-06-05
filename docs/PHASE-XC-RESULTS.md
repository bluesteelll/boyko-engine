# Phase X.C — Results: Arena lazy-commit backing store

Branch `ecs`. PERF refactor of `crates/boyko_ecs/src/ecs/memory/arena.rs` — replace the eager
64 MB global `alloc` with a reserve+demand-zero-commit virtual-memory backing. Full pipeline:
architect (with embedded research) → architecture-critic (APPROVED WITH CHANGES) → developer →
code-review (APPROVED) → orchestrator-run bench + Miri gate.

## Status: COMPLETE — Arena acquisition 1.10 µs (was ~23-75 µs)

### The change
`Arena::with_capacity` now acquires its 64 MB backing via ONE reserve+commit syscall whose physical
pages are demand-zero (faulted in lazily on first write, exactly as before), instead of the global
allocator's eager large-request path:
- **Windows** (`#[cfg(all(not(miri), windows))]`): `VirtualAlloc(NULL, size, MEM_RESERVE|MEM_COMMIT,
  PAGE_READWRITE)`; freed `VirtualFree(ptr, 0, MEM_RELEASE)`. Hand-declared `unsafe extern "system"`
  (kernel32 is std-linked — no `windows-sys` dep).
- **Unix** (`#[cfg(all(not(miri), unix, not(windows)))]`): `mmap(NULL, size, PROT_READ|PROT_WRITE,
  MAP_PRIVATE|MAP_ANONYMOUS, -1, 0)`; freed `munmap(ptr, map_len)`. Via `libc` (target-gated
  `cfg(unix)`; already in `Cargo.lock` at 0.2.186 → zero new compilation).
- **Fallback** (`#[cfg(any(miri, not(any(windows, unix))))]`): today's `std::alloc::{alloc,dealloc}`,
  VERBATIM — covers **Miri** (can't execute the syscalls) + **wasm32** (no VM API) + exotic targets.

The cfg matrix is total + disjoint by construction (the `not(windows)` guard on the unix arm makes
windows win any hypothetical both-`target_family` target). The `Backing` release-descriptor is
cfg-gated (no runtime enum tag): fallback `{layout}`, windows `{}`, unix `{map_len}`.

### The win (orchestrator-run bench, `benches/allocator.rs`)
| Bench | Result | vs baseline |
|-------|--------|-------------|
| **`arena_new_default_64mb`** | **1.10 µs** | was the dominant chunk of the old ~23-75 µs `EcsMaster::new` residual → **≈20-70× faster** |
| `ecs_master_new` | **7.23 µs** | down from ~23-75 µs (**~3-10×**). The Arena is no longer the bottleneck; the residual ~6 µs is **non-Arena** init (registries / EntityMaster / resources slab) — OUT OF SCOPE for X.C (a candidate for X.B/X.D, which touch exactly those). |

The roadmap's ≤5 µs target was about the **Arena** residual specifically — now 1.10 µs, comfortably
under. `EcsMaster::new`'s remaining ~6 µs is a separate, non-Arena cost this phase does not address.

### 0%-hot-path
`Arena::allocate_layout` / `allocate_from_free_blocks` / `allocate` / `capacity` / `new` / `Default`
are **byte-identical** (git-diff confirmed; code-review verified). Only `with_capacity` (acquisition),
`Drop` (release), the `backing` field, imports, and the `win` FFI module changed. The demand-zero
first-touch fault profile during gameplay is unchanged (the old global-alloc buffer was also
demand-zero), so there is no cost shifted into the hot loop.

## Verification gate
| Oracle | Result |
|--------|--------|
| **`arena_new` bench** (the X.C gate) | **1.10 µs ≤ 5 µs** ✓ |
| **Miri** (`miri_phase9`, `-Zmiri-tree-borrows`) | **3 passed; 0 failed** — the `cfg(miri)` fallback Drop (`self.backing.layout`) frees correctly; Miri never compiles/hits the FFI |
| arena unit tests | **13 passed** (capacity rounding, alignment, non-overlap, beyond-capacity-panic, OOM, the M-001 drop-loop now exercising real VirtualAlloc/VirtualFree on the Windows host) |
| `cargo check --all-targets` (Windows) + `--target x86_64-unknown-linux-gnu` | both exit 0 (dev + code-reviewer cross-checked the mmap arm; `--cfg miri` fallback also type-checks) |
| `cargo clippy --all-targets -- -D warnings` | clean |

## Soundness (preserved)
- **M-001 single-free / matching deallocator**: each cfg arm's `Drop` pairs with its allocator
  (VirtualAlloc↔VirtualFree, mmap↔munmap, alloc↔dealloc); cross-dealloc is statically impossible
  (the `Backing` variant only carries the matching arm's descriptor). Drop runs once.
- **The `MAP_FAILED` trap**: `MAP_FAILED` is `(void*)-1` (NON-null), so the unix arm checks
  `raw != libc::MAP_FAILED` BEFORE `NonNull::new` (a bare `NonNull::new` would wrongly accept it).
- **Alignment**: VirtualAlloc/mmap bases are page-aligned (≥4 KB ≫ the 64-byte `CACHE_LINE_SIZE` /
  32-byte `SIMD_BUFFER_ALIGN` the Arena needs).
- **`capacity()` stays logical**: `self.capacity` + `MemFreeBlockMaster::new_init` receive the
  cache-line-rounded `aligned_capacity`, not the (possibly page-rounded) OS mapping length — so the
  capacity/OOM tests hold.
- **`!Send + !Sync`** preserved (no `unsafe impl`); no `Arc`/lock; every `unsafe` has a SAFETY block.

## Pipeline notes
- Critic catches (all mechanical, folded in): M1 (unix arm gated `unix, not(windows)` for
  by-construction exhaustiveness), M2 (explicit Win64 ABI types + a `#[link(name="kernel32")]`
  fallback note), O1 (fixed a now-stale `EcsMaster::new` doc comment claiming ~50 µs eager reserve),
  O4 (capacity stays logical), O5 (commit-charge = no worse than today).
- Dev cross-checked the Unix arm on a Windows host via `rustup target add x86_64-unknown-linux-gnu`
  + `cargo check --target` (closing the "unix arm unverified" risk).
- One justified deviation: `#[cfg_attr(all(not(miri), windows), allow(dead_code))]` on the Windows
  zero-sized `Backing {}` field (VirtualFree needs only the base ptr) — cfg-scoped, not blanket.

## Files
- `crates/boyko_ecs/src/ecs/memory/arena.rs` — `win` FFI module, cfg-gated `Backing`, `with_capacity`
  + `Drop` 3-arm split, cfg-scoped imports.
- `crates/boyko_ecs/Cargo.toml` — `[target.'cfg(unix)'.dependencies] libc = "0.2"`.
- `crates/boyko_ecs/src/ecs/core/ecs_master/ecs_master.rs` — O1 doc fix.
- `crates/boyko_ecs/benches/allocator.rs` — new `bench_arena_new` + `bench_ecs_master_new` (the gate).

## Follow-up surfaced
`EcsMaster::new`'s ~6 µs non-Arena residual (registries / EntityMaster 3-Vec / resources slab) is now
the dominant `new` cost — a candidate for a future micro-opt (X.B eliminates the ComponentPool
`Vec<Unit>`; X.D reduces EntityMaster's 3 per-spawn slots — both adjacent).
