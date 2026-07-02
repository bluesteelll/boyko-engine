> STATUS: COMPLETED — archived 2026-07; implemented on branch `ecs`. See git history + the phase/feature RESULTS docs for the authoritative record.

# Phase 14a — F4 deferred finding: pre-existing remove-migration Tree-Borrows UB

**Status:** ✅ RESOLVED (Phase F4 fix — `UnsafeCell`-rooted archetype slab). The
archetype slab is now `Box<[UnsafeCell<MaybeUninit<Archetype>>; N]>`, every pointer
minted via `UnsafeCell::raw_get` (interior-mutable `SharedReadWrite` root), so a
stored archetype pointer survives a sibling `current_index` write. The
`miri_dual_presence_window_swap_remove` test was un-ignored and passes under BOTH
Tree Borrows AND Stacked Borrows; the ~12 reborrow sites are now sound with zero
hot-path change (0%-regression). See `docs/PHASE-F4-FIX-PLAN.md`. The original
finding is preserved below for the root-cause record.

---

_Original finding (now resolved):_

**Status (historical):** DEFERRED (pre-existing, not introduced by Phase 14a). Same class as
the Phase 9.1 multi-thread-Miri deferral: a sound-by-design raw-pointer pattern
that the *experimental* Tree-Borrows rules flag.

**Affected test:** `crates/boyko_ecs/tests/miri_phase14a.rs::miri_dual_presence_window_swap_remove`,
marked `#[ignore = "F4: ..."]` so the Miri suite no longer aborts on this UB
(the other three `miri_phase14a` tests pass; the native `phase14a_hooks_firing`
/ `phase14a_hooks_deferred` suites cover the same semantics end-to-end).

> **Secondary, unrelated finding** surfaced by `#[ignore]`-ing F4 — a benign
> *by-design* `Box::leak` cache allocation. See §7; it requires
> `-Zmiri-ignore-leaks` on the harness and is NOT part of F4.

---

## 1. Summary

Under `MIRIFLAGS=-Zmiri-tree-borrows` (set in `.cargo/config.toml` since Phase
3a), the remove-migration path trips a Tree-Borrows (TB) reborrow violation:

```
error: Undefined Behavior: reborrow through <tag> is forbidden
  --> crates/boyko_ecs/src/ecs/core/commands/remove_command.rs:78:43
  = help: the accessed tag <tag> has state Disabled which forbids this reborrow
help: the accessed tag <tag> was created here, in the initial state Reserved
  --> crates/boyko_ecs/src/ecs/core/archetype/archetype_bundle.rs:227:53
help: the accessed tag <tag> later transitioned to Disabled due to a foreign
      write access at offsets [0x2068..0x2070]
  --> crates/boyko_ecs/src/ecs/core/archetype/archetype.rs:481:8   (`current_index += 1`)
```

The UB is **NOT** in any Phase 14a hook code. The backtrace bottoms out in
`RemoveCommand::apply` at `remove_command.rs:78` — the reborrow runs at the TOP
of the command apply, BEFORE `migrate_entity_remove` is ever called and BEFORE
any hook fires.

---

## 2. The tag chain

1. **Birth (`archetype_bundle.rs:227`).** `ArchetypeBundle::get_archetype_ptr_mut`
   (and its sibling `archetype_ptr_for`) mints a `*mut Archetype` via
   `self.slots.as_mut_ptr().add(slot_idx)`. The `as_mut_ptr()` mint is
   *load-bearing* for write capability (documented in that method): a
   `*const → *mut` launder would not grant write permission under TB. Every such
   mint produces a **fresh** TB tag rooted at the slab `Box`'s allocation.

2. **Store.** `EcsMaster::create_entity` (`ecs_master.rs:~636`) calls
   `register_entity_with_ptr(entity, archetype_ptr, ...)`, persisting that one
   minted pointer — and therefore its one TB tag — into
   `EntityInland.archetype_ptr` for the lifetime of the entity.

3. **Foreign write (`archetype.rs:481`).** Each *subsequent* spawn into the same
   archetype calls `archetype_ptr_for` again → a NEW `as_mut_ptr()` tag, a
   **sibling** of the stored one (both children of the slab allocation root, not
   parent/child of each other). `Archetype::create_entity` then writes
   `self.current_index += 1` (and the `entity_ids.push`, pool writes, …) through
   that new sibling tag. Under TB a write through one sibling is a *foreign
   write* to the other, so the stored tag transitions Reserved → **Disabled**
   (loss of read permission) at offset `[0x2068..0x2070]` (the `current_index`
   field).

4. **Reborrow (`remove_command.rs:78`).** `RemoveCommand::apply` later reads the
   source archetype id via `(*inland.archetype_ptr()).id()`, reborrowing the now
   **Disabled** stored tag → TB UB.

The exact same chain underlies every fast-path read through
`inland.archetype_ptr()`: `EcsMaster::get_component_raw` (the `&*archetype`
reborrow), `has_component`, `has_entity`, `get_components_raw`, and the
`&mut *inland.archetype_ptr()` reborrows in the `*_mut` variants. The
remove-migration test merely reaches the first such reborrow with three entities
in one archetype, which guarantees a sibling `current_index += 1` write ran
between the store (step 2) and the read (step 4).

---

## 3. Why native execution is correct

The slab is a `Box<[MaybeUninit<Archetype>; MAX_ARCHETYPES]>` whose heap address
is stable for the `EcsMaster`'s lifetime (invariant U1). Every stored
`EntityInland.archetype_ptr` remains a valid, correctly-aligned, in-bounds
address into a live, initialised `Archetype` slot. No byte is ever read after
free, no aliasing `&mut` into the *same* archetype is ever live across the read,
and the engine is single-threaded on this path (SCH7). The defect is purely
that TB's experimental sibling-aliasing model treats two `as_mut_ptr()`
derivations of one allocation as mutually-foreign, so a write through the newer
one strips read permission from the older one even though both denote the same
stable storage. This is the textbook "stored raw pointer minted from
`as_mut_ptr()`, then read after a sibling mutation of the same allocation"
pattern that TB rejects and Stacked Borrows / native codegen do not.

---

## 4. Verdict: PRE-EXISTING (not introduced by Phase 14a Wave 4 §3.5)

Two independent lines of evidence:

### 4a. Diff evidence

`git diff b223350 -- <the four F4 sites>` shows:

* **`remove_command.rs` is UNCHANGED** since b223350 (the pre-Phase-14a HEAD) —
  the empty diff confirms line 78's reborrow `(*inland.archetype_ptr()).id()` is
  byte-identical to baseline.
* `archetype.rs:481`'s `current_index += 1` write is untouched (Wave 2 only
  inserted the `flags` field + its OR-compute; the line moved from 439 → 481
  purely from added lines above it).
* `archetype_bundle.rs:227`'s `self.slots.as_mut_ptr()` mint is untouched (Wave 2
  only inserted a `flags` field initialiser at :347; the mint moved from 226 →
  227 from the added field).

Wave 4's §3.5 restructure of `migrate_entity_remove` (Phase 1 block → Phase 2
hooks → Phase 3 re-resolve) changed code that runs **after** `remove_command.rs:78`.
It did not touch the line-78 reborrow, the `EntityInland` storage model, or the
`archetype_ptr_for` minting recipe that produce the conflicting tag.

### 4b. Baseline Miri reproduction

Stashing the entire uncommitted Phase 14a work to expose a clean b223350, a
minimal **no-hook** probe — spawn three entities into one 2-component archetype,
then `cmds.entity(e0).remove::<P4A>()` — reproduces the **byte-identical** UB on
baseline:

| Aspect                  | baseline b223350 (no hooks) | Phase 14a (with hooks) |
|-------------------------|-----------------------------|------------------------|
| Reborrow site           | `remove_command.rs:78:43`   | `remove_command.rs:78:43` |
| Tag birth               | `archetype_bundle.rs:226:53` (`as_mut_ptr`) | `archetype_bundle.rs:227:53` (`as_mut_ptr`) |
| Foreign-write Disabler  | `archetype.rs:439:8` (`current_index += 1`) | `archetype.rs:481:8` (`current_index += 1`) |
| Disabled offset         | `[0x2068..0x2070]`          | `[0x2068..0x2070]` |
| State transition        | Reserved → Disabled         | Reserved → Disabled |

The only deltas are line numbers (shifted by the Wave-2 `flags` field). The tag
chain, reborrow, Disabler, offset, and state transition are identical. The
remove-migration path has been TB-unsound here since before Phase 14a; it was
simply never exercised under `-Zmiri-tree-borrows` until the Phase 14a tester
added `miri_dual_presence_window_swap_remove`. (Phase 14a's F2 TLS-depth-counter
fix is unrelated — it removed a different re-entrancy UB that was masking this
one in the test ordering.)

---

## 5. Why deferred rather than locally patched

The task's hinted "re-derive the pointer after the push" fix targets a reborrow
*inside* `migrate_entity_remove` across the target push. But this UB fires at
`remove_command.rs:78`, **before** migration, and is one instance of a
storage-model property shared by **every** read through
`EntityInland.archetype_ptr`. A localized re-derive at line 78 (e.g. calling
`archetype_ptr_for` again instead of reading `inland.archetype_ptr()`) would:

* patch exactly one of a dozen-plus reborrow sites, and
* merely move the same UB to the next stored-pointer reborrow — the hook's own
  `get_component_raw` (`ecs_master.rs:~1155`) or the Phase-3 `move_out_entity`
  reborrow — without making the path TB-clean.

A real fix must change the slab-pointer storage discipline across the archetype
subsystem so that all reads and writes share a single TB root tag, e.g. one of:

* store the slab as `Box<[UnsafeCell<MaybeUninit<Archetype>>; N]>` and mint every
  pointer via `UnsafeCell::get()` (interior-mutability root — reads and writes no
  longer conflict under TB); or
* round-trip provenance through an exposed integer at the storage boundary
  (`ptr.expose_provenance()` on store, `with_exposed_provenance_mut` on read), so
  the stored address carries wildcard provenance that TB does not Disable; or
* re-mint the `*mut Archetype` from `archetype_ptr_for(id)` at each fast-path read
  instead of caching it in `EntityInland` (defeats the Phase-7 fast-path design
  goal — the whole point of storing the pointer was to skip the `id → slot`
  indirection).

All three touch core, hot, performance-critical archetype mechanics
(`ArchetypeBundle::slots`, `archetype_ptr_for`, `EntityInland`, and every
fast-path accessor in `EcsMaster`) and warrant their own architected phase with
benchmark + Miri gating. That is out of scope for the Phase 14a hook feature and
is exactly the "invasive/risky to core archetype mechanics" category the
investigation brief directs to defer.

---

## 6. Recommended fix (for a future phase)

Adopt the `UnsafeCell`-rooted slab (`Box<[UnsafeCell<MaybeUninit<Archetype>>; N]>`):

1. `ArchetypeBundle::slots: Box<[UnsafeCell<MaybeUninit<Archetype>>; MAX_ARCHETYPES]>`.
2. `get_archetype_ptr_mut` / `archetype_ptr_for` mint via
   `self.slots.get_unchecked(slot_idx).get() as *mut Archetype` — an
   `UnsafeCell::get()` root permits coexisting reads and writes under TB without
   the sibling-foreign-write Disable.
3. `get_archetype_ptr` (read-only) mints the same way and casts to `*const`.
4. Re-run the full Miri suite under both `-Zmiri-tree-borrows` and Stacked
   Borrows, plus `cargo bench` on the fast-path read targets, to confirm zero
   perf regression and TB cleanliness.

This both eliminates F4 and hardens every `EntityInland.archetype_ptr` read
across the engine.

---

## 7. Secondary finding: by-design `Box::leak` cache leak (NOT F4)

`#[ignore]`-ing `miri_dual_presence_window_swap_remove` lets the suite process
run to completion instead of aborting on F4's UB. Miri's end-of-process leak
check (it runs at process exit, AFTER all tests) then fires — previously masked
because F4's UB aborted the process before exit:

```
running 4 tests
test miri_deferred_command_enqueue_then_drain ... ok
test miri_dispatch_resource_mut ... ok
test miri_dual_presence_window_swap_remove ... ignored, F4: ...
test miri_pre_drop_remove_reads_dying_value ... ok
test result: ok. 3 passed; 0 failed; 1 ignored; 0 measured; 0 filtered out

error: memory leaked: alloc... (Rust heap, size: 8, align: 8), allocated here:
  ...
  at crates/boyko_ecs/src/ecs/core/bundle/bundle_column_cache.rs:231   (Vec<InlandPoolId>)
  at crates/boyko_ecs/src/ecs/core/commands/spawn_at_command.rs:151    (SpawnAtCommand::apply)
  at .../command_queue.rs:522 / :308 / :311                            (apply_via_raw_twin)
  at .../ecs_master.rs:2065                                            (drain_deferred_hook_queue)
  at .../ecs_master.rs:677 (create_entity) / :934 (spawn_one::<M2Parent>)
  at tests/miri_phase14a.rs:107 (miri_deferred_command_enqueue_then_drain)
```

### This is NOT F4 and NOT a defect

* The reported test (`miri_deferred_command_enqueue_then_drain`, test 2) PASSES
  in isolation — no UB, no TB violation. The 8-byte allocation is leaked at
  *process exit*, not during the test.
* The allocation is an `&'static [InlandPoolId]` produced by
  [`BundleColumnCache::resolve_and_cache`](../../crates/boyko_ecs/src/ecs/core/bundle/bundle_column_cache.rs)
  via an **intentional `Box::leak`** (the method's own doc: *"Leak the
  canonical-sorted slice to `&'static` … memory leak by design; bounded by
  SBO6"*). The Phase-12.5 static bundle cache deliberately leaks one slice per
  `(BundleTypeId, ArchetypeId)` pair for the process lifetime to obtain a stable
  `&'static` for the warm path. For `M2ChildBundle` (one component) that is
  exactly one `InlandPoolId` → 8 bytes.
* It reproduces here only because test 2 is the first test whose deferred-spawn
  drain warms a *new* bundle's column cache; Miri reports the first such leaked
  allocation. It is unrelated to the Phase 14a hook restructure and to F4.

### Resolution for the Miri harness

`Box::leak`'d process-lifetime caches are the textbook case for
`-Zmiri-ignore-leaks`. The `miri_phase14a` suite exercises the deferred-spawn
path (test 2), which legitimately warms the `BundleColumnCache`, so the suite
should run with leak-checking disabled:

```
MIRIFLAGS="-Zmiri-tree-borrows -Zmiri-ignore-leaks" \
    cargo +nightly miri test -p boyko-ecs --test miri_phase14a
```

This is a tester/harness configuration choice (owned by the `tester`), not a
code fix — patching `BundleColumnCache` to free its cache is out of scope for
Phase 14a and would defeat the Phase-12.5 `&'static` warm-path design. With
both flags, the suite reports `3 passed; 1 ignored` and exits cleanly.
