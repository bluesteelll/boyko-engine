> STATUS: COMPLETED — archived 2026-07; implemented on branch `ecs`. See git history + the phase/feature RESULTS docs for the authoritative record.

# Phase X.G — entities_inland Address-Stable Growth: Results

Branch `ecs`. Plan: [PHASE-XG-PLAN.md](PHASE-XG-PLAN.md) (R2-final). Research/inventory:
[PHASE-XG-RESEARCH.md](PHASE-XG-RESEARCH.md). Pipeline: project-analyst inventory →
architect → critic (CHANGES-REQUESTED: the multi-clear I-Z proof hole, W1 de-jure framing,
with_capacity over-ceiling semantics — all folded as R2) → implementation (orchestrator
finished forward after the dev agent hit a session limit; compiler as oracle) → gates.

## What landed

`EntityMaster::entities_inland` moved from `Vec<EntityInland>` to **`InlandStore`** — the
X.F reserve/commit pattern applied to entity metadata:

- NEW `memory/vm.rs`: `VmReservation` — the per-OS reserve/commit/release primitive
  extracted as a TWIN of the arena's arms (arena.rs deliberately untouched — X.F gates
  stay valid; unification filed as Phase X.H). Fallback arm uses **`alloc_zeroed`** (the
  store READS never-program-written memory by design).
- NEW `entity/inland_store.rs`: `#[repr(C)] { base, len, committed_slots,
  reserve_request, vm: Option<VmReservation> }` with `Deref/DerefMut<Target =
  [EntityInland]>` — every existing read/indexed-write site across the crate compiled
  UNCHANGED; the hot `Vec::get`-shaped lookup is preserved by construction.
- **`ensure(n)`** replaces every `resize(n, NULL)`: commit-to-cover + `len = max(len, n)`
  with **ZERO bytes copied and ZERO bytes written** — `EntityInland::NULL` is all-zero
  16 B (no padding; transmute-pinned) and freshly committed pages read zero (invariant J:
  `[len, committed)` is always all-zero, maintained by induction across clear cycles —
  the critic's R2-C1 multi-clear fix).
- **Lazy reservation (XG-B4 fix)**: the store starts with a DANGLING base + `len 0`
  (`from_raw_parts(dangling, 0)` is explicitly valid; the bounds check fails before the
  base could be touched); the 1 GiB reservation syscall happens inside the first
  `#[cold] grow_to` — `EcsMaster::new` pays nothing.
- `clear()` = memset `[0, len)` + `len = 0` (the stale-bytes hazard: a no-write regrow
  would otherwise re-expose dead records — dangling archetype ptrs / stale generations).
- Slab policy: 256 KiB → ×2 → 16 MiB clamp, request-dominant; defaults: reserve 1 GiB
  (64-bit syscall arms) / 16 MiB (Miri/wasm fallback, eager-zeroed); granule reused from
  the arena (64 KiB). Dead `INITIAL_ENTITY_CAPACITY` deleted.
- `EntityMaster` got `#[repr(C)]` with the hot scalar cluster (store 32 B + atomic 8 B +
  live_count 8 B = 48 B) on line 0; SEND5 updated ("no mid-flight realloc" is now
  STRUCTURAL — defense-in-depth, not a relaxation: SCH7 exclusivity remains normative).

## Gates

### Correctness (XG-B3)

| Gate | Result |
|---|---|
| boyko-ecs debug suites | **78 suites, 961+ passed, 0 failed** (+20 over X.F: U-V1..5, U-S1..9, P1, I1-I3, M-XG, the in-crate XG-B6 witness) |
| clippy `--workspace -D warnings` | clean |
| Miri-TB fast set | `miri_entity_store` 1/1 (with `-Zmiri-ignore-leaks` — the #53 CommandQueue Box::leak class, documented in the suite header); in-crate inland_store+vm 15/15; native 10/10 |
| Miri-TB churn controls (phase8cd/14a/14b/19) | **all clean** — 11/11, 4/4, 10/10, 9/9 (every world-creating test traverses the new grow path under Tree Borrows; ~2.6 h interpreted) |
| Key new tests | U-S2/I2 + `xg_b6_slot_address_stable_across_growth` (address-stability witnesses — impossible with `Vec`); U-S4 two-clear-cycle stale-bytes net; U-S6 exhaustion-no-state-change; U-S10-class proptest vs a `Vec` reference model |

### Performance

| Gate | Target | Measured | Verdict |
|---|---|---|---|
| XG-B1 asm (hot lookup) | instruction-identical mod displacement | 134/149 `shl$4` lookup regions byte-identical; deltas = register renames + **2 DELETED regions (the `Vec::resize` codepath)**; function-level diffs confined to legitimately-changed setup/growth fns (create/delete closures got SHORTER: −4 instr + unwind pads gone) | ✅ |
| XG-B2 hot lookups | ±2% (asm = oracle at ns scale) | `get_component_raw_hot` −1.9…+1.6% across 4 runs; `has_entity` +0.2…+3.6%; all groups swing BOTH directions run-over-run on identical lookup asm ⇒ thermal drift, asm controls (X.B law) | ✅ |
| XG-B2 production spawn path (vs xfbase) | no regression | **spawn_command_apply −53.6%, spawn_batch_10k_1comp −47.9%, commands_spawn_enqueue −37.6%, batch_10k_spawn_apply −25.4%, 3comp −29.2%, direct −15.1%** — `ensure` deleted the per-batch resize-fill (~16 KB of NULL writes × every `ensure_capacity(end+8192)` call) | ✅✅ |
| XG-B2 direct one-at-a-time API | create ≤5%, delete A/B | create_entity_10k **+6.2%**, delete_entity_10k **+10…12%**, set_component_raw **+5…8%** — STABLE across runs (not noise); see the honest analysis below | ⚠ documented |
| XG-B2 iter_entities (cold API) | baselines | dense **+23%** / sparse **−36%** — opposite stable swings on the zero-hot-caller inspection API (X.D acceptance class) | ✅ documented |
| XG-B4 `EcsMaster::new` | ≤7.5 µs | initially 7.78 µs (eager reservation) → **5.94 µs after the lazy-reservation fix** (better than X.F's 6.32) | ✅ |
| **XG-B5a g7 total vs Bevy** | ≥1.5× | **1.88×** (boyko 152.1 ms vs Bevy 286.2 ms; was 1.75× at X.F — X.G ALSO deleted ~90 ms of per-batch resize-fill the model had under-attributed) | ✅✅ |
| **XG-B5b attribution (binding)** | #285/#580 chain ABSENT from argmax | boyko argmax mode = **sub-batch #0 (×35/141, range scattered 0..646)** — no deterministic spike remains at all; Bevy's = #524 ×125/125 (its own table doubling) | ✅ |
| XG-B5c composite spike | reported (≤0.1× NOT promised) | boyko 1.47 ms (halved from 2.98) vs Bevy 9.04 ms = **0.163×** — inside the predicted 0.06–0.17 envelope; residual = X.F pool-creation class | ✅ honest |
| XG-B6 no-memcpy witness | address stability | slot addresses bit-identical across ≥3 slab growths (store, EntityMaster, and integration levels) | ✅ |

### The direct-API regression — honest analysis

`create_entity_10k` (+6%), `delete_entity_10k` (+10–12%), `set_component_raw` (+5–8%)
are STABLE, not noise. They are confined to the one-at-a-time direct API; the
**production path (Commands / spawn_batch) improved 15–54%** on the same code, and the
hot lookups are asm-identical. The `#[repr(C)]` hot-cluster reorder did NOT recover them
(hypothesis falsified by measurement), so the residual attribution candidates are
(a) demand-zero soft-fault timing inside fresh-world `iter_batched` harnesses (the VM
store re-faults pages every batch where malloc reused warm heap pages — a constant
amplified into a percentage by the harness shape; real applications create one world),
and (b) µ-architectural second-order effects of the 24→32 B field growth. Trade
accepted X.D-style: the engine's LAST realloc-doubling class is deleted, the production
spawn path is dramatically faster, and the worst-event profile is now flat.

## Residual risks / follow-ups

- **W5 (carried from X.F, widened to vm.rs):** the unix `mprotect` commit path has never
  executed on real Linux (no WSL on the dev host) — `cargo check` cross-target + review
  only. Close via WSL/CI for arena + vm at once.
- **I-Z de-jure note (R2-W1):** the syscall arms' "OS-zeroed pages are initialized"
  position rests on the documented OS zero-fill contracts + equivalence with
  `alloc_zeroed` (calloc fresh-page paths); Miri validates only the fallback arm; the
  native U-V3/U-S3 witnesses cover the syscall arms.
- **Pre-existing bug discovered by I3 (NOT an X.G regression) — FIXED post-X.G:**
  `EcsMaster::clear()` left the Phase-8.5 bundle caches stale — spawning a bundle type
  whose cache was populated BEFORE the clear panicked ("cached_archetype_id returns a
  registered id"). Investigation showed the caches are per-WORLD `EcsMaster` fields
  (not process-global as first assumed; only the `BundleTypeId` mint is global), so the
  fix is a per-world cache reset inside `clear()` (zero warm-path cost — no epoch
  compare needed) and the multi-world case was never broken (`EcsMaster::new()` starts
  cold). FreshBundle/MFreshBundle workarounds removed; regression suite:
  `tests/clear_respawn.rs` (clear+respawn both spawn paths + the multi-world pin).
- Phase X.H: migrate arena.rs onto vm.rs (twin-comment cross-references in place;
  the arena-side reciprocal comment was deferred to X.H to keep arena.rs byte-stable).
- `iter_entities` dense +23% — cold inspection API; revisit only if a hot consumer
  emerges (X.D guidance: walk archetype rows instead).

## Lessons

- **The bench harness shape can turn constants into percentages**: fresh-world
  `iter_batched` charges per-world page faults to the timed body; the same engine change
  measured −15…−54% on production-path benches. Always classify which harness family a
  regression lives in before reacting.
- The model under-counted X.G's win: the per-batch `resize` FILL (not just the doubling
  copies) was ~90 ms of the g7 workload. Deleting writes beats optimizing them.
- A `#[cold]`-path lazy reservation recovered an entire constructor-gate miss (7.78 →
  5.94 µs) for one Option that the hot path never reads — dangling-base + len-0 slices
  are the idiomatic zero-cost deferral.
- The critic's R2-C1 catch (the multi-clear induction hole) was a PROOF-text bug that
  the prescribed code already satisfied — but the two-clear-cycle test it mandated
  (U-S4) is the only thing standing between a future "optimize the memset" patch and an
  entity-aliasing bug.
