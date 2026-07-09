> STATUS: COMPLETED — archived 2026-07; implemented on branch `ecs`. See git history + the phase/feature RESULTS docs for the authoritative record.

# Physics Stage P — P2 Plan: Rigid Colored Solver `ContactColumns` -> `ScratchColumn` Migration (C3 BuildView/SolveView split)

Status: APPROVED (architect -> architecture-critic loop closed; two non-blocking notes O1/O2 recorded, two open implementation items tracked).
Scope: `crates/boyko_physics` rigid colored contact solver only. No ECS kernel changes. No public API changes.
Branch: `ecs`.

## 1. Goal

Eliminate a real Tree-Borrows (TB) Undefined Behavior in the parallel rigid colored contact solver and move the solver's contact working set off the bespoke `ContactColumns` side-store onto kernel-native dense `ScratchColumn` storage, in conformance with Principle 0 (ECS is THE SDK; no parallel data system glued on the side).

Concretely:
- Remove the whole-struct reborrow (`&mut *self.cols`) that each worker performs through `ColorSolvePtrs::columns()`, which aliases the entire `ContactColumns` across threads even though workers touch disjoint color bands. This is the UB exposed by `colored_rigid_scratch_miri.rs`.
- Replace `ContactColumns` (a `std::Vec`-backed parallel data system) with 31 kernel-native `ScratchColumn`s addressed by stable band ids, split into two capability-typed views:
  - `ContactBuildView<'a>` — single-thread build/push phase, `!Send`.
  - `ContactSolveView` — parallel solve phase, `Copy + Send + Sync`, exposes only the disjoint-by-color element accessors the kernels need.
- Preserve bit-identical solver output (scalar and AVX2) and 0% perf regression.

Performance contract:
- Hot solve loop unchanged in instruction count and layout (`cargo asm` parity, modulo a recorded benign spill — see O2).
- Zero steady-state allocations in the frame loop (all reserves at setup; `manifold_fill` refill staged into a setup-allocated, capacity-reused buffer — see O1).
- Solve phase data-race-free and TB-clean under the C2 coloring disjointness invariant.

## 2. The Bug Statement

### 2.1 Symptom
`crates/boyko_physics/tests/colored_rigid_scratch_miri.rs` trips Tree-Borrows UB under `cargo +nightly miri test` (with `MIRIFLAGS=-Zmiri-tree-borrows`).

### 2.2 Root cause
Each solver worker, for its assigned color band, obtains a pointer bundle `ColorSolvePtrs` and calls `ptrs.columns()`:

- `colored.rs:2053` — the call site inside the per-color worker kernel.
- `colored.rs:297-301` — the `ColorSolvePtrs::columns()` body, which performs `&mut *self.cols`, i.e. a reborrow of the ENTIRE `ContactColumns` struct from a raw pointer to a `&mut ContactColumns`.

The reborrow creates a unique (mutable) TB protector covering the whole `ContactColumns` for the duration of the worker call. Multiple workers run concurrently, each materializing `&mut *self.cols` over the SAME `ContactColumns` instance. Even though the workers only ever read/write rows belonging to their own disjoint color band, the `&mut` reborrow asserts exclusive access to the whole struct. That is two live `&mut` to overlapping memory across threads => aliasing violation. TB flags it. (Stacked Borrows happens to retire the tag fast enough to miss it in some runs; TB is the correct oracle here, consistent with the Phase-9 soundness series lesson that the compiler + Miri exit codes are the only soundness oracles.)

### 2.3 Why a narrower patch is insufficient
Reborrowing per-field instead of whole-struct would still hand each worker a `&mut Vec<_>` (or `&mut [_]`) spanning ALL rows of a column, not just its color band — the disjointness is by ROW (color), not by column. The fix must expose per-element access whose provenance is a base pointer + index, so that two workers writing different rows of the same column never materialize overlapping `&mut`. That is exactly what `ContactSolveView` (raw-base + indexed accessors) provides, and it is also the Principle-0-correct move (kernel `ScratchColumn` rows with `row_ptr` provenance instead of a `Vec` side-store).

## 3. Per-Column Classification (all 31 columns)

Legend:
- Element type: the per-row payload type.
- W-Read / W-Write: accessed by the parallel solve workers (read / written).
- ST-only: touched only on the single-thread build/push phase (never by workers).
- In SolveView: appears in `ContactSolveView` (i.e. has a worker-facing accessor). Columns that are ST-only do NOT appear in `ContactSolveView`.

| #  | Column                | Element type | W-Read | W-Write | ST-only | In SolveView |
|----|-----------------------|--------------|:------:|:-------:|:-------:|:------------:|
| 1  | body_a                | u32          |   Y    |    -    |    -    |      Y       |
| 2  | body_b                | u32          |   Y    |    -    |    -    |      Y       |
| 3  | normal_x              | f32          |   Y    |    -    |    -    |      Y       |
| 4  | normal_y              | f32          |   Y    |    -    |    -    |      Y       |
| 5  | normal_z              | f32          |   Y    |    -    |    -    |      Y       |
| 6  | ra_x                  | f32          |   Y    |    -    |    -    |      Y       |
| 7  | ra_y                  | f32          |   Y    |    -    |    -    |      Y       |
| 8  | ra_z                  | f32          |   Y    |    -    |    -    |      Y       |
| 9  | rb_x                  | f32          |   Y    |    -    |    -    |      Y       |
| 10 | rb_y                  | f32          |   Y    |    -    |    -    |      Y       |
| 11 | rb_z                  | f32          |   Y    |    -    |    -    |      Y       |
| 12 | normal_mass          | f32          |   Y    |    -    |    -    |      Y       |
| 13 | tangent1_x           | f32          |   Y    |    -    |    -    |      Y       |
| 14 | tangent1_y           | f32          |   Y    |    -    |    -    |      Y       |
| 15 | tangent1_z           | f32          |   Y    |    -    |    -    |      Y       |
| 16 | tangent2_x           | f32          |   Y    |    -    |    -    |      Y       |
| 17 | tangent2_y           | f32          |   Y    |    -    |    -    |      Y       |
| 18 | tangent2_z           | f32          |   Y    |    -    |    -    |      Y       |
| 19 | tangent_mass1        | f32          |   Y    |    -    |    -    |      Y       |
| 20 | tangent_mass2        | f32          |   Y    |    -    |    -    |      Y       |
| 21 | bias_rate            | f32          |   Y    |    -    |    -    |      Y       |
| 22 | mass_coeff           | f32          |   Y    |    -    |    -    |      Y       |
| 23 | impulse_scale        | f32          |   Y    |    -    |    -    |      Y       |
| 24 | normal_impulse       | f32          |   Y    |    Y    |    -    |      Y       |
| 25 | tangent_impulse1     | f32          |   Y    |    Y    |    -    |      Y       |
| 26 | tangent_impulse2     | f32          |   Y    |    Y    |    -    |      Y       |
| 27 | friction             | f32          |   Y    |    -    |    -    |      Y       |
| 28 | restitution          | f32          |   Y    |    -    |    -    |      Y       |
| 29 | rel_velocity_n       | f32          |   Y    |    -    |    -    |      Y       |
| 30 | group_start          | u32          |   Y    |    -    |    -    |      Y       |
| 31 | manifold_base        | (u32,u32)    |   -    |    -    |    Y    |      N       |

Counts: 31 total columns; 30 appear in `ContactSolveView`; 1 (`manifold_base`) is ST-only and excluded. Of the SolveView columns, 3 are worker-written (`normal_impulse`, `tangent_impulse1`, `tangent_impulse2`) and 24 are read-only contact-prep bases; `group_start` is a read-only band index column; `body_a`/`body_b` are read-only body indices.

Note on "24 bases": `ContactSolveView` stores 24 read-only base pointers (columns 1-23 plus `group_start` = column 30) PLUS 3 mutable impulse base pointers (columns 24-26) = 27 worker-facing column bases; the read-only-base count quoted as "24 bases" in the view definition (Section 5) is the 24 contact-prep/body/index read-only bases excluding the 3 impulse bases which are carried as mutable bases. `rel_velocity_n` (29), `friction` (27), `restitution` (28) are included among the read-only bases. (See exact field list in Section 5.1.)

## 4. Per-Column Lifecycle (push vs resize-overwrite)

Two distinct fill disciplines exist in the current `ContactColumns` build phase; both must be reproduced byte-identically on `ScratchColumn`s.

### 4.1 Push-filled columns (30 of 31)
Columns 1-30 are filled by sequential `push` during manifold iteration: for each contact row, each column gets exactly one appended element, in lockstep. Final length == contact count `n`. On `ScratchColumn` this becomes: `ensure_capacity(n)` once at the top of the build phase, then per-row `push_unchecked`/indexed write at the running cursor. Order is identical to the current `Vec::push` order, so byte layout is identical.

### 4.2 Resize-overwrite column (`manifold_base`, column 31)
`manifold_base` is NOT push-filled. Current mechanism:

1. `manifold_base.resize(n, (u32::MAX, 0))` — grow/shrink to `n` rows, initializing every new slot to the sentinel `(u32::MAX, 0)`.
2. Sparse indexed write: a subsequent pass walks manifolds and writes `manifold_base[row] = (base, count)` only at the rows that begin a manifold group; all other rows retain the `(u32::MAX, 0)` sentinel.

This is a "fill-then-sparse-overwrite" pattern, not an append. It is consumed ONLY on the single-thread phase (warm-start scatter / manifold bookkeeping), never by solve workers — hence column 31 is ST-only and absent from `ContactSolveView`.

Byte-identical reproduction (`manifold_fill`): on the `ScratchColumn` backend we cannot call `Vec::resize` with a non-zero sentinel directly. We reproduce it with a setup-allocated, capacity-reused staging buffer `manifold_fill`:

1. `ensure_capacity(n)` on the `manifold_base` ScratchColumn.
2. Fill the column's first `n` rows with the sentinel `(u32::MAX, 0)` (a straight memset-equivalent broadcast of the sentinel pair; `0u32::MAX` low word + `0` high word).
3. Run the identical sparse indexed-write pass writing `(base, count)` at manifold-start rows.

`manifold_fill` is the name of the staging discipline reproducing `resize(n, (u32::MAX,0))`. Per critic note O1 it MUST be setup-allocated, capacity-reused, and zero-steady-state-alloc, and MUST be commented as a transient refill stager — NOT durable per-entity data (so it does not itself become a parallel data system). See Section 13 (O1).

Byte-identity requirement: after build, the `manifold_base` ScratchColumn must be bit-for-bit equal to what `ContactColumns.manifold_base` produced via `resize` + sparse write, including all retained sentinels at non-manifold-start rows.

## 5. View Type Definitions (exact signatures)

### 5.1 `ContactSolveView` (Copy + Send + Sync)

```rust
/// Worker-facing, color-disjoint contact solve view.
///
/// SAFETY/SOUNDNESS: `Send + Sync` is sound ONLY because the colored solver's
/// C2 coloring invariant guarantees that any two rows touched concurrently by
/// distinct workers belong to distinct color bands and therefore distinct rows.
/// All mutation goes through `set_*_impulse*` which writes a single row via a
/// base-pointer + index (no `&mut` ever spans more than one element), so two
/// workers never materialize overlapping `&mut`. Read accessors form `&T` to a
/// single element only.
#[derive(Clone, Copy)]
pub(crate) struct ContactSolveView {
    // ---- read-only contact-prep / body / index bases (24) ----
    body_a: *const u32,
    body_b: *const u32,
    normal_x: *const f32,
    normal_y: *const f32,
    normal_z: *const f32,
    ra_x: *const f32,
    ra_y: *const f32,
    ra_z: *const f32,
    rb_x: *const f32,
    rb_y: *const f32,
    rb_z: *const f32,
    normal_mass: *const f32,
    tangent1_x: *const f32,
    tangent1_y: *const f32,
    tangent1_z: *const f32,
    tangent2_x: *const f32,
    tangent2_y: *const f32,
    tangent2_z: *const f32,
    tangent_mass1: *const f32,
    tangent_mass2: *const f32,
    bias_rate: *const f32,
    mass_coeff: *const f32,
    impulse_scale: *const f32,
    friction: *const f32,
    restitution: *const f32,
    rel_velocity_n: *const f32,
    group_start: *const u32,
    // ---- worker-mutable impulse bases (3) ----
    normal_impulse: *mut f32,
    tangent_impulse1: *mut f32,
    tangent_impulse2: *mut f32,
    len: usize,
}

// SAFETY: see struct-level soundness note (C2 coloring disjointness).
unsafe impl Send for ContactSolveView {}
// SAFETY: see struct-level soundness note (C2 coloring disjointness).
unsafe impl Sync for ContactSolveView {}

impl ContactSolveView {
    #[inline] pub(crate) fn len(&self) -> usize { self.len }

    // ---- scalar per-element reads (single-row &T provenance) ----
    #[inline] pub(crate) fn body_a(&self, i: usize) -> u32;
    #[inline] pub(crate) fn body_b(&self, i: usize) -> u32;
    #[inline] pub(crate) fn normal_mass(&self, i: usize) -> f32;
    #[inline] pub(crate) fn tangent_mass1(&self, i: usize) -> f32;
    #[inline] pub(crate) fn tangent_mass2(&self, i: usize) -> f32;
    #[inline] pub(crate) fn bias_rate(&self, i: usize) -> f32;
    #[inline] pub(crate) fn mass_coeff(&self, i: usize) -> f32;
    #[inline] pub(crate) fn impulse_scale(&self, i: usize) -> f32;
    #[inline] pub(crate) fn friction(&self, i: usize) -> f32;
    #[inline] pub(crate) fn restitution(&self, i: usize) -> f32;
    #[inline] pub(crate) fn rel_velocity_n(&self, i: usize) -> f32;
    #[inline] pub(crate) fn group_start(&self, i: usize) -> u32;

    // ---- Vec3 read helpers (3 scalar reads -> glam::Vec3) ----
    #[inline] pub(crate) fn normal(&self, i: usize) -> Vec3;
    #[inline] pub(crate) fn ra(&self, i: usize) -> Vec3;
    #[inline] pub(crate) fn rb(&self, i: usize) -> Vec3;
    #[inline] pub(crate) fn tangent1(&self, i: usize) -> Vec3;
    #[inline] pub(crate) fn tangent2(&self, i: usize) -> Vec3;

    // ---- impulse reads (single-row &T) ----
    #[inline] pub(crate) fn normal_impulse(&self, i: usize) -> f32;
    #[inline] pub(crate) fn tangent_impulse1(&self, i: usize) -> f32;
    #[inline] pub(crate) fn tangent_impulse2(&self, i: usize) -> f32;

    // ---- impulse writes (single-row, base+index, never spanning rows) ----
    #[inline] pub(crate) fn set_normal_impulse(&self, i: usize, v: f32);
    #[inline] pub(crate) fn set_tangent_impulse1(&self, i: usize, v: f32);
    #[inline] pub(crate) fn set_tangent_impulse2(&self, i: usize, v: f32);

    // ---- AVX2 gathers: load 8 lanes from a column at scalar stride ----
    // (one per read-only column the avx2 kernel consumes)
    #[inline] pub(crate) unsafe fn body_a_at(&self, i: usize) -> __m256i;
    #[inline] pub(crate) unsafe fn body_b_at(&self, i: usize) -> __m256i;
    #[inline] pub(crate) unsafe fn normal_x_at(&self, i: usize) -> __m256;
    #[inline] pub(crate) unsafe fn normal_y_at(&self, i: usize) -> __m256;
    #[inline] pub(crate) unsafe fn normal_z_at(&self, i: usize) -> __m256;
    #[inline] pub(crate) unsafe fn ra_x_at(&self, i: usize) -> __m256;
    #[inline] pub(crate) unsafe fn ra_y_at(&self, i: usize) -> __m256;
    #[inline] pub(crate) unsafe fn ra_z_at(&self, i: usize) -> __m256;
    #[inline] pub(crate) unsafe fn rb_x_at(&self, i: usize) -> __m256;
    #[inline] pub(crate) unsafe fn rb_y_at(&self, i: usize) -> __m256;
    #[inline] pub(crate) unsafe fn rb_z_at(&self, i: usize) -> __m256;
    #[inline] pub(crate) unsafe fn normal_mass_at(&self, i: usize) -> __m256;
    #[inline] pub(crate) unsafe fn tangent1_x_at(&self, i: usize) -> __m256;
    #[inline] pub(crate) unsafe fn tangent1_y_at(&self, i: usize) -> __m256;
    #[inline] pub(crate) unsafe fn tangent1_z_at(&self, i: usize) -> __m256;
    #[inline] pub(crate) unsafe fn tangent2_x_at(&self, i: usize) -> __m256;
    #[inline] pub(crate) unsafe fn tangent2_y_at(&self, i: usize) -> __m256;
    #[inline] pub(crate) unsafe fn tangent2_z_at(&self, i: usize) -> __m256;
    #[inline] pub(crate) unsafe fn tangent_mass1_at(&self, i: usize) -> __m256;
    #[inline] pub(crate) unsafe fn tangent_mass2_at(&self, i: usize) -> __m256;
    #[inline] pub(crate) unsafe fn bias_rate_at(&self, i: usize) -> __m256;
    #[inline] pub(crate) unsafe fn mass_coeff_at(&self, i: usize) -> __m256;
    #[inline] pub(crate) unsafe fn impulse_scale_at(&self, i: usize) -> __m256;
    #[inline] pub(crate) unsafe fn friction_at(&self, i: usize) -> __m256;
    #[inline] pub(crate) unsafe fn restitution_at(&self, i: usize) -> __m256;
    #[inline] pub(crate) unsafe fn rel_velocity_n_at(&self, i: usize) -> __m256;
    #[inline] pub(crate) unsafe fn group_start_at(&self, i: usize) -> __m256i;

    // ---- AVX2 impulse gathers + scatters ----
    #[inline] pub(crate) unsafe fn normal_impulse_at(&self, i: usize) -> __m256;
    #[inline] pub(crate) unsafe fn tangent_impulse1_at(&self, i: usize) -> __m256;
    #[inline] pub(crate) unsafe fn tangent_impulse2_at(&self, i: usize) -> __m256;
    #[inline] pub(crate) unsafe fn set_normal_impulse_at(&self, i: usize, v: __m256);
    #[inline] pub(crate) unsafe fn set_tangent_impulse1_at(&self, i: usize, v: __m256);
    #[inline] pub(crate) unsafe fn set_tangent_impulse2_at(&self, i: usize, v: __m256);
}
```

Notes on the AVX2 `<col>_at` accessors: each loads 8 contiguous lanes starting at row `i` (`_mm256_loadu_ps` / `_mm256_loadu_si256` from `base.add(i)`). The `set_*_impulse_at` accessors `_mm256_storeu_ps` 8 contiguous lanes at `base.add(i)`. These are the SoA column-contiguous loads/stores the current AVX2 kernel already performs against `ContactColumns` slices — provenance simply moves from `&[f32]` to a raw base pointer. `group_start_at` returns `__m256i` (u32 lanes).

### 5.2 `ContactBuildView<'a>` (!Send)

```rust
/// Single-thread build/push view over all 31 scratch columns.
/// !Send: build runs on one thread only; holding `&mut` to whole columns is
/// fine here because no other thread is live during build.
pub(crate) struct ContactBuildView<'a> {
    // 31 mutable ScratchBuildView handles, one per column.
    body_a: ScratchBuildView<'a, u32>,
    body_b: ScratchBuildView<'a, u32>,
    normal_x: ScratchBuildView<'a, f32>,
    normal_y: ScratchBuildView<'a, f32>,
    normal_z: ScratchBuildView<'a, f32>,
    ra_x: ScratchBuildView<'a, f32>,
    ra_y: ScratchBuildView<'a, f32>,
    ra_z: ScratchBuildView<'a, f32>,
    rb_x: ScratchBuildView<'a, f32>,
    rb_y: ScratchBuildView<'a, f32>,
    rb_z: ScratchBuildView<'a, f32>,
    normal_mass: ScratchBuildView<'a, f32>,
    tangent1_x: ScratchBuildView<'a, f32>,
    tangent1_y: ScratchBuildView<'a, f32>,
    tangent1_z: ScratchBuildView<'a, f32>,
    tangent2_x: ScratchBuildView<'a, f32>,
    tangent2_y: ScratchBuildView<'a, f32>,
    tangent2_z: ScratchBuildView<'a, f32>,
    tangent_mass1: ScratchBuildView<'a, f32>,
    tangent_mass2: ScratchBuildView<'a, f32>,
    bias_rate: ScratchBuildView<'a, f32>,
    mass_coeff: ScratchBuildView<'a, f32>,
    impulse_scale: ScratchBuildView<'a, f32>,
    normal_impulse: ScratchBuildView<'a, f32>,
    tangent_impulse1: ScratchBuildView<'a, f32>,
    tangent_impulse2: ScratchBuildView<'a, f32>,
    friction: ScratchBuildView<'a, f32>,
    restitution: ScratchBuildView<'a, f32>,
    rel_velocity_n: ScratchBuildView<'a, f32>,
    group_start: ScratchBuildView<'a, u32>,
    manifold_base: ScratchBuildView<'a, (u32, u32)>,
    _not_send: PhantomData<*mut ()>,
}

impl<'a> ContactBuildView<'a> {
    /// Reserve capacity for `n` rows across all 31 columns (setup-time).
    pub(crate) fn ensure_capacity(&mut self, n: usize);

    /// Append one contact row (push discipline, columns 1-30).
    /// `manifold_base` is filled separately via `manifold_fill` (Section 4.2).
    pub(crate) fn push_row(&mut self, row: ContactRowInit);

    /// Reproduce `resize(n, (u32::MAX, 0))` then run the sparse manifold write.
    pub(crate) fn manifold_fill(&mut self, n: usize, manifolds: &Manifolds);

    /// Freeze build state into a worker-facing `ContactSolveView`.
    pub(crate) fn solve_view(&self) -> ContactSolveView;
}
```

`PhantomData<*mut ()>` makes `ContactBuildView` `!Send` and `!Sync`, statically forbidding it from crossing into the worker pool. The `solve_view()` method captures raw bases + `len` from the (now frozen) columns to construct the `Copy` `ContactSolveView`.

## 6. `ColorSolvePtrs` Delta

Current `ColorSolvePtrs` (the bundle handed to each worker) carries:
- `cols: *mut ContactColumns`
- `group_start: ...` (a redundant cached pointer into the group_start column)
- `columns()` method (`&mut *self.cols` — the UB site at :297-301)
- `group_start_at(...)` accessor

Delta:
- REMOVE field `cols: *mut ContactColumns`.
- REMOVE field/cached `group_start` pointer.
- REMOVE method `columns()` (kills the :297-301 whole-struct reborrow / the :2053 call site).
- REMOVE method `group_start_at(...)` (folded into `ContactSolveView::group_start` / `group_start_at`).
- ADD field `view: ContactSolveView` (a `Copy` value, embedded by value — no indirection, no reborrow).

Workers now read/write exclusively through `self.view.<accessor>(i)`, whose provenance is a per-element base+index. No worker ever forms a `&mut` spanning more than one row.

New `Send`/`Sync` SAFETY argument for `ColorSolvePtrs`:

```rust
// SAFETY: ColorSolvePtrs is Send+Sync because its only shared-mutable state is
// `view: ContactSolveView`, whose mutation surface is the three impulse columns
// accessed strictly by single-row base+index writes. The C2 coloring invariant
// guarantees that two workers never share a contact row (distinct colors =>
// distinct rows => distinct body pairs), so concurrent writes target disjoint
// addresses and concurrent reads never alias a concurrent write. No `&mut` ever
// spans more than one element, so no overlapping unique protector is created.
unsafe impl Send for ColorSolvePtrs {}
unsafe impl Sync for ColorSolvePtrs {}
```

This replaces the previous (unsound) whole-struct-reborrow basis with one grounded directly in the C2 coloring disjointness invariant.

## 7. `scratch_ids.rs` Delta

Allocate a contiguous descending band of 31 scratch ids for the rigid colored contact columns.

- Band: ids `508 ..= 478` (31 ids; `508 - 478 + 1 = 31`).
- Assignment order: id `508` -> column 1 (`body_a`), descending to id `478` -> column 31 (`manifold_base`). (Descending to match the existing scratch-id convention in `scratch_ids.rs`.)
- 31 `register_layout` calls, one per column, each registering the column's element `Layout` (size+align) under its band id.
- Idempotency: registration is `OnceLock`/first-touch guarded exactly like the existing scratch ids; re-entry is a no-op returning the same id. No per-frame registration.
- Collision headroom: the band `478..=508` sits in unused scratch-id space below the existing allocated ids; verified non-overlapping with all currently registered bands. Headroom below 478 remains free for future physics scratch columns.

Concrete id map (descending):

| id  | column           | id  | column           | id  | column           |
|-----|------------------|-----|------------------|-----|------------------|
| 508 | body_a           | 497 | tangent1_z       | 486 | mass_coeff       |
| 507 | body_b           | 496 | tangent2_x       | 485 | impulse_scale    |
| 506 | normal_x         | 495 | tangent2_y       | 484 | normal_impulse   |
| 505 | normal_y         | 494 | tangent2_z       | 483 | tangent_impulse1 |
| 504 | normal_z         | 493 | tangent_mass1    | 482 | tangent_impulse2 |
| 503 | ra_x             | 492 | tangent_mass2    | 481 | friction         |
| 502 | ra_y             | 491 | bias_rate        | 480 | restitution      |
| 501 | ra_z             | 490 | mass_coeff?      | 479 | rel_velocity_n   |
| 500 | rb_x             |     |                  | 478 | manifold_base    |
| 499 | rb_y             |     |                  |     |                  |
| 498 | rb_z             |     |                  |     |                  |

(The table above is illustrative of the descending packing; the binding rule is authoritative: id `508 - (k-1)` -> column `k` for `k = 1..=31`, with `group_start` = column 30 -> id `479+? `. Authoritative mapping: column index `k` (1-based, per Section 3 table order) maps to id `508 - (k - 1)`; thus `group_start` (k=30) -> id `479`, `manifold_base` (k=31) -> id `478`, and `rel_velocity_n` (k=29) -> id `480`. The illustrative cell text is superseded by this formula.)

## 8. Reserve Sizing + `ensure_capacity` Grow-Guard

### 8.1 Constants ground truth (from `constants.rs`)
Per-arm pool sizing constants (adaptive by element size), used to derive scratch reserve rows:

- `POOL_TARGET_DATA_BYTES` — the target bytes-per-pool budget that drives the initial reserve.
- `POOL_MIN_ROWS` — floor on reserved rows per arm (so tiny worlds still get a usable column).
- `POOL_MAX_ROWS` — ceiling on reserved rows per arm (so a huge element type cannot blow the reserve).

Per-arm derivation: `reserve_rows = clamp(POOL_TARGET_DATA_BYTES / size_of::<Elem>(), POOL_MIN_ROWS, POOL_MAX_ROWS)`. The arm is selected by element size exactly like the existing `ComponentPool` adaptive chunking (tiny <=16B, small <=64B, medium <=256B, large >256B), so each of the 31 columns reserves according to its own element size.

### 8.2 B4 grow-guard rationale
The solve workers hold a `ContactSolveView` of raw bases captured at `solve_view()` time. If any column reallocated (grew) WHILE a view is live, the captured bases would dangle => UB. The B4 guard enforces: capacity is finalized BEFORE any `ContactSolveView` is created, and never grows while a view is live.

### 8.3 `ensure_capacity` design
- Predicate: at build top, compute required `n` (contact count). If `n > current_capacity`, grow (reallocate the column's reservation) NOW, before `solve_view()`. The doubling predicate (see open item 16.2) governs the new capacity: grow to `max(n, next_capacity)` where `next_capacity` is the doubling target — to amortize and avoid per-frame regrow churn.
- Re-create-before-view-live discipline: ALL `ensure_capacity` growth happens during the single-thread build phase. `solve_view()` is called only after the last possible growth. The worker dispatch happens only after `solve_view()`. Therefore no column can reallocate while a `ContactSolveView` (and its raw bases) is live. This is the soundness backbone for the `Send + Sync` bases.
- Steady state: after warm-up, `n <= current_capacity` holds frame-over-frame, so `ensure_capacity` takes the no-grow fast path and performs ZERO allocations in the frame loop.

## 9. Per-Kernel Access-Conversion Recipe

The conversion is mechanical and provenance-only; the arithmetic is untouched, guaranteeing bit-identity.

### 9.1 `solve_color` (scalar)
For each read `cols.<col>[i]` -> `view.<col>(i)`.
For each Vec3 read assembled from three columns -> `view.<vec3helper>(i)` (e.g. `view.normal(i)`).
For each impulse read `cols.normal_impulse[i]` -> `view.normal_impulse(i)`.
For each impulse write `cols.normal_impulse[i] = v` -> `view.set_normal_impulse(i, v)`.
`group_start` read -> `view.group_start(i)`.

Bit-identity: the loaded scalar values are identical (same bytes at the same offsets), and the FP/integer ops between load and store are unchanged. Output is therefore bit-identical.

0%-regression (`cargo asm`): each `view.<col>(i)` lowers to `*base.add(i)` — the SAME load the compiler emitted for `slice[i]` after bounds-check elision (the kernel already used unchecked indexing). The accessor is `#[inline]`, so post-inlining the codegen is the same `mov`/`movss`. Expected `cargo asm` parity modulo a benign register spill (O2).

### 9.2 `solve_color_avx2` (gather/scatter)
For each 8-lane column load `_mm256_loadu_ps(cols.<col>.as_ptr().add(i))` -> `view.<col>_at(i)`.
For each integer column load (`body_a`/`body_b`/`group_start`) `_mm256_loadu_si256(...)` -> `view.<col>_at(i)` (returns `__m256i`).
For each 8-lane impulse load -> `view.<col>_impulse_at(i)`.
For each 8-lane impulse store `_mm256_storeu_ps(cols.<col>.as_mut_ptr().add(i), v)` -> `view.set_<col>_impulse_at(i, v)`.

Bit-identity: identical SIMD intrinsics over identical memory => identical lanes. The view accessors are thin `#[inline] unsafe` wrappers around the SAME `_mm256_loadu_*` / `_mm256_storeu_*` against `base.add(i)`.

0%-regression (`cargo asm`): post-inline, `view.<col>_at(i)` is the same `vmovups`/`vmovdqu` from `base + i*4`. No extra instructions. The `group_start_at` returns `__m256i` exactly as the prior inline load did. Expected parity modulo the O2 spill.

## 10. Data-Race-Freedom / Tree-Borrows Soundness Proof (parallel path)

Claim: the parallel solve phase is data-race-free and Tree-Borrows-clean.

Premises:
1. C2 coloring invariant: within a single color band dispatched to workers in one parallel step, no two contact rows share a body. Equivalently, distinct rows touched concurrently reference disjoint `body_a`/`body_b` and are themselves distinct row indices. (Established by the graph-coloring pass prior to solve; this plan does not modify it.)
2. View construction happens after the last `ensure_capacity` growth (B4 / re-create-before-view-live). No reallocation occurs while any `ContactSolveView` is live. Bases are therefore stable for the worker lifetime.
3. Every worker mutation is `set_*_impulse*(i, v)` (scalar) or `set_*_impulse_at(i, v)` (8 contiguous lanes), writing exclusively to row(s) of the worker's assigned color band.
4. No accessor forms a `&mut` (or `&`) spanning more than the element(s) it touches: scalar accessors do `*base.add(i)` (single element); AVX2 accessors do `loadu/storeu` over 8 contiguous lanes via raw pointers (no Rust reference materialized at all).

Proof of race-freedom:
- Two workers in the same parallel step have distinct color bands => disjoint row sets (premise 1) => their `set_*_impulse*` writes target disjoint addresses. No write-write race.
- A read accessor in worker A reads row `i` in A's band; any concurrent write in worker B targets a row in B's band (disjoint from A's). No read-write race.
- The body velocity arrays read/updated by the solver are likewise partitioned by the C2 invariant (no shared body within a step), so no body-array race. (Unchanged from pre-existing design; the migration does not alter body access.)

Proof of TB-cleanliness:
- The eliminated UB (`&mut *self.cols` whole-struct reborrow) is gone: `ColorSolvePtrs` no longer holds `*mut ContactColumns` and has no `columns()` method.
- The remaining mutation path creates, per write, at most a transient unique access to a SINGLE element (or, for AVX2, a raw-pointer store with no reference). Per premise 1 these never overlap across threads, so no two live unique protectors cover overlapping memory. TB has no aliasing violation to flag.
- Read accessors form `&T` to a single element (or raw loads), never overlapping a concurrent unique write (premise: disjoint bands). No TB read-under-unique violation.
- The one remaining accepted TB diagnostic is the crossbeam-deque over-approximation (work-stealing internals), identical to the Phase-9 accepted result; it is not caused by this code.

Therefore the parallel path is data-race-free and TB-clean (modulo the accepted crossbeam-deque over-approximation).

## 11. Gates

All must pass before P2 is considered done.

1. Byte-identical `ColumnsSnapshot` over ALL 31 columns. CRITICAL: the current test helper silently snapshots only 26 columns — it MUST be extended to all 31 (it currently omits 5, including the ST-only `manifold_base` and the impulse columns or contact-prep tail — extend to the full set). The snapshot taken with the `ScratchColumn` backend must be bit-for-bit equal to the `ContactColumns` backend snapshot for an identical input scene, across scalar and AVX2 solve paths.
2. Miri Tree-Borrows fully clean (`MIRIFLAGS=-Zmiri-tree-borrows cargo +nightly miri test`) for the rigid colored solver tests, EXCEPT the allowed crossbeam-deque work-stealing over-approximation (documented, identical to Phase-9). `colored_rigid_scratch_miri.rs` must now PASS.
3. 0%-regression on the rigid colored solve benchmark (criterion, median-of-N per `bench.ps1`) AND zero steady-state allocations (alloc-count bench) in the frame loop. `manifold_fill` staging must show zero per-frame allocs after warm-up.
4. `cargo clippy --all-targets -- -D warnings` clean.
5. `cargo asm` parity on `solve_color` and `solve_color_avx2` hot loops, modulo the benign spill recorded per O2.

## 12. The 16 Stepwise Edit Sites (file:line)

1. `scratch_ids.rs` — add the 31-id band `508..=478` + 31 `register_layout` calls (Section 7). [near the existing scratch-id registrations]
2. `colored.rs:297-301` — DELETE `ColorSolvePtrs::columns()` (the `&mut *self.cols` UB body).
3. `colored.rs:2053` — DELETE the `ptrs.columns()` call site; replace downstream uses with `ptrs.view`.
4. `colored.rs` (ColorSolvePtrs struct def) — remove `cols`, remove cached `group_start`, remove `group_start_at`; add `view: ContactSolveView`; add the new `Send`/`Sync` SAFETY impls (Section 6).
5. `colored.rs` — define `ContactSolveView` (Section 5.1) with all reads, Vec3 helpers, scalar/avx2 accessors, impulse set/gather/scatter, `group_start`/`group_start_at`.
6. `colored.rs` — define `ContactBuildView<'a>` (Section 5.2) with the 31 `ScratchBuildView` fields, `ensure_capacity`, `push_row`, `manifold_fill`, `solve_view`.
7. `colored.rs` (build phase entry) — allocate/obtain the 31 `ScratchColumn`s by band id; construct `ContactBuildView`; call `ensure_capacity(n)` (Section 8.3, B4 discipline).
8. `colored.rs` (build push loop) — replace the 30 push-filled `ContactColumns` pushes with `ContactBuildView::push_row` (Section 4.1).
9. `colored.rs` (manifold pass) — replace `manifold_base.resize(n,(u32::MAX,0))` + sparse write with `ContactBuildView::manifold_fill` (Section 4.2, O1 discipline + comment).
10. `colored.rs` (pre-dispatch) — call `solve_view()` AFTER the last `ensure_capacity`; embed the resulting `ContactSolveView` into each `ColorSolvePtrs` (B4 re-create-before-view-live).
11. `colored.rs::solve_color` — apply the scalar access-conversion recipe (Section 9.1).
12. `colored.rs::solve_color_avx2` — apply the AVX2 access-conversion recipe (Section 9.2).
13. `colored.rs` (struct removals) — delete the `ContactColumns` struct + its impl (now fully superseded) OR retain only if referenced by a non-rigid path; verify no other referent (grep). [authoritative: remove if rigid-colored-only].
14. `colored.rs:685` — name the scratch reserve wrapper (open item 16.1: `scratch_reserve_rows` vs `pool_reserve_rows`); wire it to the constants-derived sizing (Section 8.1).
15. Test helper (`ColumnsSnapshot`) — extend from 26 to ALL 31 columns (Gate 1).
16. `colored_rigid_scratch_miri.rs` — confirm/adjust the test now exercises the view path and PASSES under TB (Gate 2).

## 13. Non-Blocking Critic Notes

### O1 — `manifold_fill` must be a setup-allocated, capacity-reused, zero-steady-alloc refill stager (and commented as such)
The buffer/discipline reproducing `resize(n, (u32::MAX,0))` MUST: (a) allocate its staging capacity at setup, (b) reuse that capacity across frames (grow only on the B4 grow path, never per-frame in steady state), (c) perform zero steady-state allocations, and (d) carry a code comment explicitly stating it is a TRANSIENT refill stager reproducing the resize-sentinel pattern — NOT durable per-entity data. This keeps it from becoming a parallel data system in violation of Principle 0. (Non-blocking: design above already specifies this; the note pins it as a hard implementation requirement + mandatory comment.)

### O2 — record the `cargo asm` spill observation in RESULTS
A benign register spill may appear in the hot loop `cargo asm` diff after the view conversion (one extra base pointer kept live). It is performance-neutral within bench noise. RESULTS must record the observed spill (or its absence) explicitly so the 0%-regression claim is auditable. (Non-blocking.)

## 14. Open Implementation Items

### 16.1 — scratch reserve wrapper name at `colored.rs:685`
Decide the wrapper name for the constants-derived per-column reserve: `scratch_reserve_rows` vs `pool_reserve_rows`. (Leaning `scratch_reserve_rows` for clarity that it sizes scratch columns, not `ComponentPool` arms — but the developer picks at implementation, consistent with the surrounding naming.)

### 16.2 — `ensure_capacity` doubling predicate
Finalize the exact doubling target in `ensure_capacity`: grow to `max(n, current_capacity * 2)` vs grow to `max(n, next_pow2(n))`. Both satisfy B4 + amortized-O(1) + zero-steady-alloc; pick the one matching the existing pool growth convention to keep warm-up reserve behavior consistent. Record the chosen predicate in RESULTS.

## 15. Summary of Conformance

- Principle 0: contact working set moves from the `ContactColumns` `std::Vec` side-store onto kernel-native `ScratchColumn`s; `manifold_fill` is an explicitly-commented transient stager, not durable side data.
- Principle 1/4: no `dyn`, no `Mutex`/`RwLock`/`RefCell` on the solve path; mutation is lock-free per-row base+index under the C2 disjointness invariant.
- Principle 3/6: SoA column layout preserved; AVX2 contiguous loads/stores preserved; bit-identical SIMD.
- Principle 7: accessors `#[inline]`, no blind `#[inline(always)]`; `cargo asm` parity is a gate.
- Principle 8: every `unsafe` (Send/Sync impls, raw accessors) carries a `// SAFETY:` comment grounded in the C2 coloring disjointness invariant and the B4 re-create-before-view-live discipline.
- Soundness: the `&mut *self.cols` whole-struct reborrow UB is eliminated; the parallel path is proven data-race-free and TB-clean (modulo the accepted crossbeam-deque over-approximation).
