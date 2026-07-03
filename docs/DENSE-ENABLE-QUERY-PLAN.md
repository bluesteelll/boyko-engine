# Dense-include × enable-term query support (kernel)

**Status:** design (architect synthesis; pending critic → developer → reviewer → tester).
**Owner directive (2026-07-03):** foundation-first — fix the kernel before returning to the
app-host R5 rung. Once landed, `boyko_render::SnapInterpolation` migrates from its structural
ZST-table-tag fallback back to a real `EnableTag`.

## Problem

`Query<&mut DenseComponent, Enabled<Tag>>` / `Disabled<Tag>` — a query that combines a term over
a **dense-stored** component with an **enable-term** — compiles but silently yields **zero rows**.
No shape-assert rejects it (`enable_tuple_no_positive_rejected` only rejects a positive-term-less
enable tuple; this query HAS a positive data term), so it is a "compile-but-lie" bug — the exact
hazard the enable module warns about. It forced R5's teleport marker onto a structural table tag
instead of the intended `EnableTag` bit, and it is a latent trap for any future dense × enable query.

## Root cause (verified against live code)

The query planner has three mutually-exclusive candidate-seed branches in
[`QueryDataState::new`](../crates/boyko_ecs/src/ecs/core/iters/query/state.rs) (~L247-284):

1. `if IS_CANDIDATE_SEEDED` — the *sole single enable* shape (`Query<(), Enabled<Tag>>`): seed from
   `EnablePresence::snapshot_present(tag)`.
2. `else if HAS_DENSE_INCLUDE && is_empty_include()` — a dense include with no table positive bound:
   `dense_seed(...)` from `DenseStore::arch_presence`. **Does not run `recull`.**
3. `else` — the table path: `update_archetypes` + `post_filter_matched` + (if `HAS_ENABLE_TERM`)
   `recull(...)` populating `enable_cull.culled_ids`.

For `Query<&mut Dense, Enabled<Tag>>`: `HAS_DATA_COMPONENT = true` ⇒ `IS_CANDIDATE_SEEDED = false`;
`HAS_DENSE_INCLUDE = true` + empty table include ⇒ it takes branch **(2)**, so `culled_ids` stays
empty. Then [`enable_driver_ids`](../crates/boyko_ecs/src/ecs/core/iters/query/state.rs) (~L602) for
the `HAS_ENABLE_TERM && !IS_CANDIDATE_SEEDED` shape returns `&self.enable_cull.culled_ids` = **empty**
⇒ zero driver archetypes ⇒ zero rows. Compounding it: the dense shape routes its per-frame refresh to
`dense_update` (query.rs ~L760), which **bypasses `update()` entirely** — so even the epoch-gated
recull that `update()` runs for positive-term enable shapes is unreachable for the dense shape.

**Load-bearing finding (scoped — see the dense_iter exception below):** on the **archetype-walking
cursors**, the per-row enable bit is **already enforced** during iteration —
[`iter.rs`](../crates/boyko_ecs/src/ecs/core/iters/query/iter.rs) L241/L539/L796 call
`F::filter_fetch` under `!const { F::IS_ARCHETYPAL }`, and `Enabled`/`Disabled` set
`IS_ARCHETYPAL = false`. The critic verified this holds for `iter` / `iter_mut` / `iter_entities*`
/ `par_iter` (which const-rejects dense at its bound) / `for_each_chunk` (rejects non-archetypal
filters) / `get` / `get_mut`. For those the archetype-level cull is **purely an optimization** and
the bug is exclusively driver-list starvation — a **routing + invalidation repair localized to
`state.rs`**.

**THE EXCEPTION (critic C1 — a second false-include surface the routing fix does NOT reach):** the
dense fast path [`Query::dense_iter` / `dense_iter_mut`](../crates/boyko_ecs/src/ecs/core/iters/query/query.rs)
(~L404-435) and the `QueryView` twins (query_view.rs ~L467-492) stride the `DenseStore` column
directly via `DenseCursor::next_live` (dense_iter.rs ~L164-189): they consult **neither**
`driver_ids`/`culled_ids` **nor** `filter_fetch`, and are bound only on `D: DenseQueryData` with `F`
**entirely unconstrained**. So `Query<&mut Dense, Enabled<Tag>>::dense_iter_mut()` compiles today and
yields **every live slot — disabled rows included** (a `&mut` write to rows the enable term must
exclude; `Disabled` leaks identically). The migration target (`SnapInterpolation` → `EnableTag`,
a dense component) is exactly the solver-driven `dense_iter` consumer the module names, so this is
not theoretical. This must be closed as part of the feature — see **D0**.

## Industry grounding (researcher)

Unity DOTS `IEnableableComponent`, flecs `CanToggle`/`TOGGLE`, and boyko's own `EnableTag` all use a
**per-entity bit, no structural change**, and all use **two-level filtering**: a coarse candidate
cull that *bounds* the set (never drops a possible match) + an *exact* per-entity bit test. boyko
already has both levels (`EnablePresence` archetype oracle + `EnableColumn::for_each_run` per-row
summary/bit — the direct analogue of DOTS's `chunkEnabledMask` + `ChunkEntityEnumerator`). The one
structural difference driving this bug: boyko deliberately keeps the `EnableTag` id **out of the
archetype signature** (Decision D5 — it has no `ComponentPool`), so an enable term contributes no
positive include bit and must ride a *separate* candidate seed. DOTS/flecs sidestep the problem by
keeping the toggleable component *in* the signature; boyko cannot without reversing D5, so the
presence-index seed is the necessary substitute — and it must **compose** with the dense seed instead
of being mutually exclusive with it.

## Design

Adopt the **full fix** (not the minimal driver-only reroute): keep the coarse archetype cull so a
dense query with a sparse enable stays O(enabled archetypes), not O(all dense archetypes trimmed
per-row). Concretely:

### D0 — `dense_iter*` compile-rejects an enable-bearing filter (critic C1)
The dense fast path cannot honor a per-row enable term without reconstructing per-slot
`(archetype, row)` context — the `DenseStore` column is **archetype-agnostic** (one flat buffer
across archetypes) while the enable bit is keyed by `(archetype, row)`, so applying the bit inside
`DenseCursor` would defeat the contiguous-stride purpose the fast path exists for. Therefore
`dense_iter` / `dense_iter_mut` (on both `Query` and `QueryView`) must **compile-reject** any `F`
that carries an enable term, mirroring `par_iter`'s dense const-reject and `for_each_chunk`'s
`ArchetypalQueryFilter` reject. Mechanism: an inline
`const { assert!(!F::CONTAINS_ENABLE_TERM, "dense_iter cannot honor an enable term — use iter_mut()") }`
at the top of each of the four methods (fires at monomorphization; the existing `eval_shape_asserts`
`const {}` idiom). A user who wants `Query<&mut Dense, Enabled<Tag>>` iteration uses `iter_mut()` —
the archetype-walking cursor whose driver D2/D4 fix and whose per-row `filter_fetch` already enforces
the bit. No existing (non-enable) `dense_iter` call breaks (`F` is `()`/archetypal there). A
**trybuild `compile_fail`** pinning the reject is mandatory. *(Scope note: this rejects an
ENABLE-bearing `F` only; whether `dense_iter` also silently skips a `Changed`/`Added` per-row filter
is a pre-existing, separate question — flag it for the reviewer, do not widen this fix.)*

### D1 — New classification const, `IS_DENSE_ENABLE`
Add to `QueryDataState` (near `HAS_DENSE_INCLUDE`, state.rs ~L134):
```rust
const IS_DENSE_ENABLE: bool = Self::HAS_DENSE_INCLUDE && Self::HAS_ENABLE_TERM;
```
It is disjoint from `IS_CANDIDATE_SEEDED` (which requires `!HAS_DATA_COMPONENT`; a `&mut Dense` has
`HAS_DATA_COMPONENT = true`, and a sole `Query<(), Enabled<Tag>>` has no dense include). Every
non-dense/non-enable query folds it to `false`, so the new branches are dead-code-eliminated — the
**0%-gate stays byte-identical** (const-assert this, mirroring `shape_consts_classification`).

### D2 — `new()` dense-seed branch: recull after the dense seed
In branch (2), after `Self::dense_seed(...)`, when `const { Self::HAS_ENABLE_TERM }`:
```rust
Self::recull(archetype_state.matched_ids_pre_terms(), &filter_state, master, &mut culled_ids);
last_observed_enable_epoch = master.enable_presence().epoch();
```
This bounds the driver to enable-kept archetypes over the dense-seeded candidate set. `recull` uses
`F::enable_cull_keeps_archetype`, which already encodes the **polarity** correctly (see D4).

### D3 — `dense_update()`: re-home the enable recull (the invalidation fix)
The dense shape refreshes via `dense_update` (state.rs ~L564) every frame, **not** `update()`.
`dense_update` reseeds `matched_ids` from live `arch_presence` on every call (so dense inserts,
archetype churn, and removals are always fresh). Add, when `const { Self::HAS_ENABLE_TERM }`, after
`seed_from_candidates`: recull the freshly-seeded `matched_ids` into `culled_ids` and re-stamp the
enable epoch. Because the reseed already rebuilds `matched_ids`, a recull-after-reseed re-adds any
archetype that gained a column (the Decision-5 re-add invariant).

**The epoch-gate is REQUIRED, not optional (critic O1).** `dense_update` runs on **every** per-frame
query resolution (`update_with_world` → `dense_update`, unconditional), so an *unconditional* recull
would run a per-frame O(matched archetypes) `enable_cull_keeps_archetype` scan — a regression versus
the table path, whose warm branch (state.rs ~L437-477) skips the recull when the epoch is unchanged.
Gate the recull on `dense_generation_changed || enable_presence().epoch() != last_observed_enable_epoch`
(the reseed of `matched_ids` stays unconditional — only the recull is gated), matching the table
path's discipline so the dense shape reaches parity, not a per-frame scan. Correctness of the gate:
`note_column_alloc` bumps `epoch` (Release), and a presence-bit CLEAR is documented+enforced to always
pair with a `structural_generation` bump (which changes the dense reseed result → `dense_generation_changed`),
so every event that could change the cull membership trips one of the two gate terms. A pure
enable-toggle of an existing row needs **no** recull (its archetype membership is unchanged; the
per-row `filter_fetch` reflects the flipped bit at iteration) — so the gate correctly skips it.

### D4 — Driver routing + polarity
`enable_driver_ids` (state.rs ~L602): the `HAS_ENABLE_TERM && !IS_CANDIDATE_SEEDED` arm already
returns `&self.enable_cull.culled_ids`. With D2/D3 populating `culled_ids` for the dense+enable shape,
**no change to `enable_driver_ids` is required** — the existing return is now correct. Add two
debug-asserts for the `IS_DENSE_ENABLE` shape: `culled_ids` is the intended driver, and (critic O2,
mirroring the `qs1_after_cull` invariant) `culled_ids ⊆ matched_ids` (the driver subset never desyncs
from the dense-seeded bitset).

**Why the epoch-gate is sound (the linchpin — verified in `enable_presence.rs:157-183`):**
`contains(tag, arch)` returns true iff the archetype owns an **allocated `EnableColumn`** for the tag
— it is **column-presence, not has-an-enabled-row**. It flips only on `note_column_alloc` (which bumps
`epoch`) / column removal (which pairs with a structural bump), **never on a per-row enable/disable
toggle**. So `culled_ids` membership is a function of column presence + the dense reseed only; a
per-row toggle leaves the cull unchanged (the archetype stays kept) and is reflected solely by
per-row `filter_fetch`. The `enable-a-row-in-a-previously-all-disabled-archetype` case is NOT a
false-drop: that archetype has the column (`contains` = true), so it was never culled — it sits in
`culled_ids`, per-row-rejects while all-disabled, and per-row-admits the newly-enabled row with no
recull.

**Polarity is handled by `enable_cull_keeps_archetype`, and this is load-bearing:**
- `Enabled<Tag>` keeps iff `enable_presence().contains(tag, arch)` (column present) — tightens to
  column-bearing archetypes; per-row `filter_fetch` (NULL column ⇒ `false`) trims disabled rows.
- `Disabled<Tag>` keeps **all** archetypes (amendment A1.1) — a no-column dense archetype has every
  row "disabled", so it MUST NOT be dropped; per-row `filter_fetch` (NULL column ⇒ `true`) admits
  those rows. This is exactly why the fix uses `recull` (cull-after-seed) rather than intersecting the
  dense seed with `EnablePresence` up front: an up-front intersection would false-empty the
  `Disabled` polarity by dropping no-column dense archetypes. Do **not** "optimize" the disabled
  polarity into a presence intersection.

### D5 — Both presence oracles are independent and in scope
Dense presence (`DenseStore::arch_presence`, keyed by the dense component's `ComponentId`) and enable
presence (`EnablePresence`, keyed by the tag's `ComponentId`) are separate registries with separate
keys; `dense_seed`/`dense_update` already borrow the dense registry, `recull` already borrows the
master's `enable_presence` — both reachable at cull time with no new plumbing.

### D6 — Shape coverage (critic W1 — the fix is complete only if every dense+enable shape is accounted for)
`IS_DENSE_ENABLE` keys off `HAS_DENSE_INCLUDE`, but `use_dense_seed()` also requires
`is_empty_include()` at runtime, so the shapes partition across THREE handlers. Each row must be
verified by the developer; the table is the completeness contract:

| Query shape | Seed branch | Fixed by | Notes |
|---|---|---|---|
| `Query<&mut Dense, Enabled/Disabled<Tag>>` | dense-seed (empty include) | **D2 + D3** | the target; `use_dense_seed()==true` |
| `Query<&Dense, (With<Table>, Enabled<Tag>)>` | table (`is_empty_include()==false`) | **pre-existing positive-term recull** (`update()` path) | D3 is dead code here — the `With<Table>` include bit routes it to `update()`, which already reculls |
| `Query<(&Dense, &Table), Enabled<Tag>>` | table (`&Table` sets an include bit) | **pre-existing positive-term recull** | same as above |
| `Query<(), Enabled<Tag>>` (sole) | candidate-seed | **pre-existing** IS_CANDIDATE_SEEDED path | no dense include; unaffected |
| `Query<&Dense, Changed<Dense>>` etc. (no enable) | dense-seed | **unaffected** (0%-gate; `HAS_ENABLE_TERM==false`) | byte-identical |
| `Query<AnyOf<(&Dense, &B)>, Enabled<Tag>>` | table (`AnyOf` has `HAS_DENSE=true` but `HAS_DENSE_INCLUDE=false` + `REQUIRES_POST_FILTER_TRIM=true`) | **VERIFY** | `IS_DENSE_ENABLE==false` and not candidate-seeded → `else` table branch → reculls, but the cursor also runs `dense_row_passes`; the recull-vs-OR-trim interaction is unanalyzed. Developer MUST add a behavioral test; if it mis-culls, add a shape-assert rejecting `AnyOf`-dense + enable as unsupported (documented) rather than silently mis-render. |
| any `dense_iter*` call with an enable-bearing `F` | (fast path — no seed) | **D0 compile-reject** | trybuild `compile_fail` |

The developer must confirm each "pre-existing" row actually yields correct rows today (they should —
they route through the table recull that already works — but the completeness of THIS fix depends on
it, so each gets a smoke assertion). The `AnyOf`-dense row is the one genuine open question:
in-scope-with-test or explicitly-unsupported-with-shape-assert — no third option.

## What does NOT change (0%-gate + soundness surface)
- Per-row `filter_fetch` / `for_each_run` / `EnableColumn` — untouched (already exact).
- `iter.rs` archetype-walking cursors — untouched (they already run the per-row gate).
- Non-dense / non-enable queries — byte-identical (all new code behind `const IS_DENSE_ENABLE` /
  `const HAS_ENABLE_TERM` folds out).
- The sole-single-enable candidate path and the positive-term table path — untouched.
- No new `unsafe`: `recull` is single-threaded `new`/`update`-time, reads the Acquire presence oracle,
  writes a plain `Vec` (the existing pattern).
- **`dense_iter*` DOES gain a compile-time guard (D0)** — the only touch outside `state.rs`. It adds a
  `const {}` assert (no runtime code, no effect on any existing non-enable `dense_iter` call) + a
  trybuild `compile_fail`; the dense cursor's runtime path is otherwise unchanged.

## Tests (mandatory)
Headless, in `state.rs::enable_global_scan` (or a new sibling mod), mirroring `cull_then_enable_readds`.
**Every fixture that means to exercise the dense-seed path MUST assert `state.use_dense_seed() == true`**
(critic W2) — otherwise it silently routes through the `update()` table recull and leaves D2/D3
unverified. A dense component with an empty table include is required (`#[component(storage = "dense")]`
+ no `With`/table-data term).
1. **Positive behavioral, `Enabled`:** `Query<&mut Dense, Enabled<Tag>>` over a world with
   some-enabled / all-enabled / all-disabled dense rows yields exactly the enabled dense rows (the
   direct zero-row regression witness). Assert `use_dense_seed()`.
2. **Polarity, `Disabled`:** `Query<&mut Dense, Disabled<Tag>>` yields the disabled dense rows
   **including** rows in dense archetypes that never had the tag column (the A1.1 no-column-is-all-
   disabled trap) — the test that would fail an up-front-intersection design.
3. **Per-row toggle (exact path, NOT recull):** in a column-present dense archetype, disable then
   re-enable a row → it leaves then re-enters the result across `update`s. Per D4, `contains` is
   column-presence, so the archetype stays in `culled_ids` throughout and ONLY `filter_fetch` changes —
   this pins the per-row exact trim over the dense-seeded driver (a regression that dropped per-row
   enforcement would fail here).
4. **Re-seed on dense-insert:** insert the dense component into a new archetype that has the tag →
   picked up (exercises `dense_update`'s reseed + recull; the `dense_generation_changed` gate term).
5. **Column-alloc epoch (the D3 recull invalidation):** a dense archetype gains the tag column for the
   first time mid-run (`note_column_alloc` bumps `epoch`) → the epoch-gated recull re-adds it exactly
   once. This is the test that fails if the recull is gated on the dense generation ALONE (the epoch
   term is load-bearing).
6. **`get`/`iter` agreement (critic W3):** for `Query<&mut Dense, Enabled<Tag>>`, `get(entity)` on an
   enabled dense entity returns `Some` and on a disabled one returns `None`, and the set of `get`-Some
   entities equals the `iter` set (the `get_iter_agree` invariant extended to the dense shape —
   `get`/`get_mut` route through `matched_archetypes_bitset` + `query_view_enable_passes`, a different
   path than `culled_ids`, so it needs its own witness).
7. **`const` classification:** `IS_DENSE_ENABLE` true for the dense+enable shape, false for dense-only,
   enable-only, and plain queries; `IS_CANDIDATE_SEEDED` stays false for the dense+enable shape;
   0%-gate const-assert that a plain `Query<&P, With<P>>` is unaffected.
8. **`dense_iter*` reject (critic C1):** a trybuild `compile_fail` proving
   `Query<&mut Dense, Enabled<Tag>>::dense_iter_mut()` (and the `Query`/`QueryView` `dense_iter` twins)
   fail to compile with the D0 assert message. A companion positive test that the SAME query iterates
   correctly via `iter_mut()`.
9. **`AnyOf`-dense + enable (D6 open row):** a behavioral test of
   `Query<AnyOf<(&Dense, &B)>, Enabled<Tag>>` — either it yields correct rows (in-scope) or the test
   documents the shape-assert reject (unsupported). No silent mis-render.
10. **Miri (TB):** run the dense+enable iteration (both polarities) + `get`/`get_mut` under
    `cargo +nightly miri test` — the dense column access + per-row fetch is the unsafe surface; a
    stranded/aliased pointer would surface here (the phase-14a precedent: Miri-TB has caught real bugs
    in this kernel before).

## Downstream (after this lands)
`boyko_render::SnapInterpolation` migrates from the ZST table tag back to an `EnableTag`:
`snap_apply`'s `With<SnapInterpolation>` → `Enabled<SnapInterpolation>`, and
`pack_gpu_transforms`'s `Option<&SnapInterpolation>` presence read → the enable read (the two
`// KERNEL-TODO: dense×enable ⇒ With→Enabled` sites the R5 design flags). This removes the per-teleport
archetype move and resolves the "EnableTag" doc drift for real. Folded into the R5 review-fix pass.
