> STATUS: COMPLETED — archived 2026-07; implemented on branch `ecs`. See git history + the phase/feature RESULTS docs for the authoritative record.

# Phase F4 — `UnsafeCell`-rooted archetype slab (Tree-Borrows soundness fix)

Architecture plan for bug A-2/F4. Soundness-only (no behavior change). Acceptance:
0%-regression on Phase-7 random access + Miri-clean under BOTH Tree Borrows AND
Stacked Borrows. Full root-cause: `docs/PHASE-14-F4-FINDING.md`.

## §1 Root cause (recap)
`ArchetypeBundle::slots: Box<[MaybeUninit<Archetype>; MAX_ARCHETYPES]>`
(`archetype_bundle.rs:76`). Every pointer is minted via `slots.as_mut_ptr()` —
each call a FRESH child tag rooted at the `Box` allocation, so two mints are TB
**siblings**. `EntityInland.archetype_ptr` caches one such pointer `T0`. A later
spawn into the same archetype mints a sibling `T1` and writes `current_index += 1`
(`archetype.rs:481`) through it → foreign write → `T0` transitions Reserved→Disabled
→ the next reborrow of `T0` (e.g. `remove_command.rs:78`) is TB-UB. Native-correct
(slab base heap-stable, all in-bounds, single-threaded); TB-only experimental
sibling-aliasing strictness. Affects ~12 stored-pointer reborrow sites (it's a
storage-discipline property, not a hook bug) — a localized patch just moves the UB.

## §2 Decision — Option A: `UnsafeCell`-rooted slab
Change `slots` to `Box<[UnsafeCell<MaybeUninit<Archetype>>; MAX_ARCHETYPES]>`; mint
every pointer via `UnsafeCell::raw_get` (forms NO reference). A pointer from an
`UnsafeCell` root carries Cell-permission: coexisting sibling reads/writes do NOT
Disable each other under TB, and carry `SharedReadWrite` under SB (not popped by
sibling writes). **This is the EXACT in-repo pattern `ComponentPool` already uses**
for `Box<[UnsafeCell<Tick>]>` (`component_pool.rs:80,1038,1347-1359`) — F4 applies it
one level up, at `Archetype` granularity.
- **Rejected B (exposed-provenance round-trip):** escapes provenance to integer
  space → pessimizes the optimizer on the hot read path (violates 0%-regression);
  only patches reads; introduces a 2nd weaker idiom vs the established `UnsafeCell` one.
- **Rejected C (re-mint per read):** defeats the Phase-7 ~3 ns/lookup cached-pointer
  design AND doesn't even fix it (fresh sibling re-Disabled by the next write).

## §3 Storage + mint recipe
- Field: `slots: Box<[UnsafeCell<MaybeUninit<Archetype>>; MAX_ARCHETYPES]>`. Order
  `UnsafeCell<MaybeUninit<_>>` (cell OUTERMOST = provenance root; MaybeUninit inside
  tracks init). `UnsafeCell<T>` + `MaybeUninit<T>` both `#[repr(transparent)]` ⇒
  size/align/stride IDENTICAL to `Archetype` (8480 B) ⇒ slab alloc + `slot*stride`
  arithmetic unchanged. Per-element cell (mirrors the `Tick` precedent; localizes
  interior mutation to the slot being written).
- `new()`: `Box::<[UnsafeCell<MaybeUninit<Archetype>>; N]>::new_uninit().assume_init()`
  — sound because the element has no validity invariant (transparent over MaybeUninit).
  Per-slot lazy init via `self.occupied` unchanged.
- Single private helper (centralizes the recipe + SAFETY):
  ```rust
  #[inline(always)]
  fn slot_ptr_mut(&self, slot_idx: usize) -> *mut Archetype {
      debug_assert!(slot_idx < MAX_ARCHETYPES);
      let cell: *const UnsafeCell<MaybeUninit<Archetype>> =
          unsafe { self.slots.as_ptr().add(slot_idx) };
      UnsafeCell::raw_get(cell).cast::<Archetype>()  // raw_get forms NO reference
  }
  ```
  `get_archetype_ptr` returns `slot_ptr_mut(..) as *const` (read-only = caller
  contract, as today). Read pointers are now ALSO Cell-rooted → survive sibling writes.

## §4 Site migration
- **9 mint sites in `archetype_bundle.rs`** (`:227, :274, :317, :432, :487, :525, :575,
  :644, :703` + the two `ArchetypeBundleIter`/`IterMut` constructors caching the base +
  `Drop`) → all delegate to `slot_ptr_mut` (iterators cache the CELL-array base + `raw_get`
  per `next()`). Found via grep of `self.slots.(as_ptr|as_mut_ptr)`.
- **12 reborrow sites BODIES UNCHANGED** (`remove_command.rs:78`, `insert_command.rs:68/112`,
  `ecs_master.rs:1079/1091/1155/1201/1316/1335/1402/1442`, `query_view.rs:359/409`,
  `migration_helpers.rs` `&mut *source_ptr` sites, `fire_despawn_hooks`). They keep
  dereferencing the stored `*mut Archetype`; the stored pointer is now Cell-rooted so
  the reborrows become legal with ZERO textual change. Only SAFETY comments amended
  ("stable, interior-mutable (F1-rooted) slab provenance — survives sibling structural
  writes"). **`EntityInland` shape unchanged** (`*mut Archetype`, 16 B, offset 0).

## §5 Soundness (why read-after-sibling-write is now legal)
TB foreign-write rule: a write through node N Disables every non-ancestor node M.
Old: `T0` (stored) and `T1` (writer) are sibling children of the Box root → `T1`'s
write Disables `T0`. New: both are children of the SAME per-slot `UnsafeCell` node C,
which carries Cell-permission ⇒ interior mutation through one child does NOT Disable
the other ⇒ `T0` stays readable ⇒ reborrow legal. (Exactly `ComponentPool`'s tick
rationale.) SB: `raw_get` → `SharedReadWrite` (not `SharedReadOnly` a `&` would give);
sibling `SharedReadWrite` writes don't pop it; no `&mut [UnsafeCell;N]` retag ever
forms above. Clean under both models.

## §6 Invariants preserved
- `offset_of!(Archetype, columns)==0` + `size_of::<Archetype>()==8480` untouched (the
  cell wraps the SLAB ELEMENT, not Archetype's fields). New const-assert (I8):
  `size_of/align_of::<UnsafeCell<MaybeUninit<Archetype>>>() == that of Archetype`.
- **R1 — Send/Sync (THE risk):** `UnsafeCell` is `!Sync`, so the slab becomes `!Sync`,
  removing the auto impl `ArchetypeBundle`/`ArchetypeMaster`/`EcsMaster` rely on for
  Phase-9 cross-thread `&EcsMaster`. **MANDATORY:** add `unsafe impl Send + Sync for
  ArchetypeBundle {}` with SAFETY mirroring `ComponentPool` SEND10 (`&mut self`/apply-
  window-only mutation per SCH3; worker `&self` reads of immutable archetypes; distinct
  cell per slot; dispatcher aliasing contract SCH7). Wave-1 places a `cargo check` gate
  immediately after the field change to catch any Send/Sync regression in the scheduler.
- **0%-regression:** transparent layout + `raw_get` lowers to identity + the 12 reborrow
  bodies + `EntityInland` + `get_component_raw` (`:1136`) are byte-identical. Only cold
  structural-op methods change (a `raw_get` no-op replaces an `as_mut_ptr` no-op).

## §7 SAFETY invariants
F1 (every slab pointer minted via `raw_get` on a `*const UnsafeCell` element — Cell-
rooted), F2 (no reference materialised — preserves U11 no-`&MaybeUninit`-retag), F3
(`*mut MaybeUninit<Archetype> as *mut Archetype` is a transparent no-op). Blocks: the
`slot_ptr_mut` helper, `new()` (amend C3), the 9 mint sites (shrink to "delegates to
slot_ptr_mut"), in-place `addr_of_mut!` writes (U13 + F1 note), `clear`/`Drop`
`drop_in_place` (U12 + F1 note), the 12 reborrow sites (amend "interior-mutable"), and
the new `unsafe impl Send/Sync`.

## §8 Wave plan
1. **Wave 1:** field type + `slot_ptr_mut` helper + const-asserts + `new()` + the
   `unsafe impl Send/Sync` → **immediate `cargo check -p boyko-ecs`** (R1 gate).
2. **Wave 2:** migrate the 9 mint sites + the 2 iterators + `Drop` to the helper.
3. **Wave 3:** amend the 12 reborrow-site SAFETY comments (CODE UNCHANGED).
4. **Wave 4:** un-ignore `tests/miri_phase14a.rs:191` (`miri_dual_presence_window_swap_remove`);
   run the validation surface; mark the F4 finding resolved.

## §9 Test + Miri + bench surface
- **Miri (deliverable):** un-ignore the dual-presence test; run `miri_phase14a` under TB
  (`-Zmiri-tree-borrows -Zmiri-ignore-leaks` — the ignore-leaks is for the UNRELATED
  by-design Box::leak cache, not F4) AND under SB (drop `-Zmiri-tree-borrows`). Re-run
  the in-module `phase7_miri_*` slab tests (the canonical mint-recipe guards) under both.
  Run the broader Phase-9/10/11/12 Miri suites (every stored-ptr reborrow is now Cell-rooted).
- **New native regression** (`archetype_bundle.rs` tests): `f4_stored_ptr_survives_sibling_spawn`
  — spawn A (store its inland ptr), spawn B+C into the same archetype (the foreign writes),
  read through A's stored ptr. The minimal UB→clean reproducer (cheap Miri regression).
- **Native:** full ~667-test suite (debug+release) unchanged (no-behavior-change fix).
- **Bench (0%-gate):** Phase-7 `get_component` random-access (~3 ns/lookup) within ±2%;
  capture a `cargo asm`/disasm diff of `get_component_raw` to PROVE byte-identical hot codegen.
- `cargo clippy --all-targets -- -D warnings` (the new `UnsafeCell` field + manual Send/Sync
  must not trip lints).

## §10 Risk register
| # | Risk | Mit |
|---|------|-----|
| R1 | `UnsafeCell` drops auto Send/Sync → scheduler build break | mandatory `unsafe impl Send/Sync`; Wave-1 cargo-check gate FIRST |
| R2 | a mint site missed → F4 persists at one reborrow | single-helper recipe (any leftover `as_mut_ptr` is a visible outlier); the new regression test + full Miri catch it |
| R3 | a `&UnsafeCell` retag sneaks in | mandate `raw_get` (no reference); `phase7_miri_archetype_ptr_no_retag_ub` guards |
| R4 | wrong nesting (`MaybeUninit<UnsafeCell>`) | §3 fixes `UnsafeCell<MaybeUninit>` (cell outermost) + const-assert |
| R5 | hidden hot-path codegen change | disasm diff of `get_component_raw` + ±2% bench; reborrow bodies byte-identical |
| R6 | by-design Box::leak mistaken for F4 regression | `-Zmiri-ignore-leaks` + document |
| R7 | SB-cleanliness assumed not verified | explicit SB Miri pass (drop the TB flag) |

**Open questions (for the critic):** OQ1 — collapse `get_archetype_ptr`/`_mut` now that both
mint identical Cell-rooted pointers? (defer — API change, soundness-only phase). OQ2 —
per-element (chosen) vs whole-array cell? (both TB-legal; per-element mirrors the Tick
precedent + localizes interior mutation).

## §11 — Round 2 patches (resolve critic Round 1; supersede on conflict)

> Critic R1: **REVISE, no design change** — Option A + the TB/SB/Send-Sync/0%-regression
> arguments all VERIFIED against the code + the `ComponentPool` precedent. Must-fix =
> completeness/precision + the W4 mint-discipline (load-bearing). These patches make the
> plan dev-ready.

**P1 (CRITICAL — complete the site lists).**
- **12 mint sites** in `archetype_bundle.rs` (grep `self.slots.(as_ptr|as_mut_ptr)`):
  `227, 274, 317, 432, 487, 525, 575, 610, 644, 671, 689, 703` — the §4 list OMITTED
  **610** (`iter_occupied_ptrs`, a read `as_ptr` mint). ALL 12 route through the helpers.
  Read sites (274 `get_archetype_ptr`, 610 `iter_occupied_ptrs`, 671 `iter`) use a new
  `slot_ptr(&self) -> *const Archetype` (= `slot_ptr_mut(idx) as *const`, same cell root).
- **Reborrow sites — PARTITION into two classes** (only class (a) needs a SAFETY amend):
  - **(a) stored-pointer SURVIVORS (amend SAFETY — now interior-mutable/F1-rooted):**
    `remove_command.rs:78`, `insert_command.rs:68, 112`, `ecs_master.rs:1017, 1022, 1079,
    1091, 1155, 1201, 1316, 1335, 1402, 1442`, `query_view.rs:359, 409`, and the
    `migration_helpers.rs` RE-RESOLVED `&mut *source_ptr` at `:608` (and the cross-hook
    reborrows `:214, :215, :519, :520` that survive across pool writes).
  - **(b) fresh-minted same-frame LOCALS (already legal pre-fix — NO change, optionally a
    one-line "fresh local, same-cell" note):** `ecs_master.rs:652, 658, 665` (create_entity),
    `:790, 795, 802` (create_entity_at), `:875` (create_entity_at_with_pool_ids).

**P2 (W2 — the REAL Send/Sync gate).** The `cargo check` gate is a false-green: `Archetype`
(`archetype.rs:880`), `ArchetypeMaster` (`archetype_master.rs:653`), `EcsMaster`
(`ecs_master.rs:2511`) all use MANUAL `unsafe impl Send/Sync`, so they compile whether or
not `ArchetypeBundle` is Send/Sync. ADD a positive static assertion so the build FAILS if
the manual `unsafe impl Send + Sync for ArchetypeBundle` is forgotten:
`assert_impl_all!(ArchetypeBundle: Send, Sync);` (the crate already uses `static_assertions`
— cf. the `QueryView` Send/Sync assert). Keep the `cargo check` step too, but the
`assert_impl_all!` is the load-bearing R1 gate.

**P3 (W3 — accurate SAFETY wording).** Current Miri TB does NOT mint a distinct
"Cell-permission child node C" (that's a PROPOSED future refinement, UCG #403). Reword the
§5 / §7-F1 rationale (and every SAFETY comment the dev writes) to: *pointers derived from
the `UnsafeCell` address interior-mutable (`SharedReadWrite`) locations; the ENTIRE slab
element is wrapped (`UnsafeCell<MaybeUninit<Archetype>>`), so all bytes incl.
`current_index` are interior-mutable; same-cell-derived sibling pointers do not Disable one
another under current TB/SB — identical to the `Box<[UnsafeCell<Tick>]>` precedent
(`component_pool.rs:1347-1359`).* Drop the "distinct Cell node C" framing.

**P4 (W4 — the LOAD-BEARING mint discipline).** `<[T;N]>::as_mut_ptr` forms a transient
`&mut [UnsafeCell<…>;N]` over the WHOLE array — an array-level (NON-interior-mutable) retag
whose children could re-introduce the sibling relationship one level up, defeating the fix.
**Therefore NO site may call `self.slots.as_mut_ptr()`.** EVERY mint — including the
`&mut self` structural-op methods (227, 317, 432, 487, 525, 575, 644, 689, 703) — routes
through the single `&self`-based helper:
`slot_ptr_mut(&self, idx) = UnsafeCell::raw_get(self.slots.as_ptr().add(idx)).cast::<Archetype>()`
(`self.slots.as_ptr()` under `&self` forms only a shared `&[UnsafeCell;N]`, whose elements'
CONTENTS are interior-mutable — no `&mut`-array retag). A `&mut self` method calling a
`&self` helper is fine. This mirrors `ComponentPool` minting via `get_unchecked(i).get()`
(never `&mut [UnsafeCell;N]`). Make "no `as_mut_ptr` on the slab; the `&self` `raw_get`
helper is the SOLE mint entry" an explicit F4 invariant; `phase7_miri_archetype_ptr_no_retag_ub`
guards it under both models.

**P5 (OQ1 — update the now-false provenance doc).** The doc at `archetype_bundle.rs:244-248`
asserts `get_archetype_ptr` returns a `SharedReadOnly`-provenance pointer and that casting
it to `*mut` for write is UB. After the fix a read pointer cast from the `raw_get` `*mut`
root is `SharedReadWrite` (more permissive; still read-only by CALLER CONTRACT). UPDATE that
doc comment to: "read-only by caller contract; provenance is interior-mutable
(`SharedReadWrite`) post-F4, so the read/write split is a contract, not a provenance,
distinction." Do NOT collapse the two public methods (OQ1 deferred — API change).

**P6 (OQ2 — iterator base provenance).** `ArchetypeBundleIter`/`IterMut` must NOT cache a
single per-element `*Archetype` base and stride across elements (that pointer has provenance
over ONE cell → `add` into other cells is out-of-bounds UB). Instead cache the **cell-array
base** `*const UnsafeCell<MaybeUninit<Archetype>>` (= `self.slots.as_ptr()`, array provenance)
and mint each element in `next()` via `UnsafeCell::raw_get(base.add(slot_idx)).cast()`. The
`add` strides over the array (in-bounds, array provenance); `raw_get` then yields the
per-element interior-mutable pointer. (This is the §4.1.1 resolution — make it explicit.)

**Round 2 changelog:** P1 (12 mint sites incl. 610 + partitioned reborrow list + `slot_ptr`
read helper), P2 (`assert_impl_all!` R1 gate), P3 (SharedReadWrite SAFETY wording), P4 (no
`as_mut_ptr`; single `&self` `raw_get` mint entry — load-bearing), P5 (update :244-248 doc),
P6 (iterator caches array base, `raw_get` per `next()`). Wave 1 Step 2 gains the
`assert_impl_all!`. **APPROVED-in-substance** (Option A verified; must-fixes are
completeness/precision, now patched) — ready for the developer.
