# Kernel backlog — engine-side work, grouped by crate

Every engine addition the campaign needs, with the zero-cost-when-unused status and the oracle that
gates it. Aether-side work lives in [`CONSTRUCTS.md`](CONSTRUCTS.md) / [`MACHINES.md`](MACHINES.md).

## `boyko_ecs`

| # | Item | Why | Zero when unused | Oracle |
|---|---|---|---|---|
| E1 | **Fix `Or` dense blindness** — `impl_or_filter_tuple` (iters/query/filter.rs) forwards none of the dense plumbing the AND tuple has; a dense arm inside `Or<...>` silently matches nothing. ~117 production `Or<(` sites | R0; known silent-wrong-answer; precondition for any `or(...)` grammar | n/a (bug fix) | **red-first** test: `Or<(Changed<TableC>, Changed<DenseC>)>` vs a hand oracle |
| E2 | **`Entities<'w>` read-only SystemParam** with `get(EntityId) -> Option<Entity>`; carrier pointer is `*const InlandStore`, NOT `*const EntityMaster` (type-enforced narrowness, the `EntityCounter` precedent) | machines need `me`; iterators yield `EntityId`, events carry `Entity` with generation; today the only resolver sits behind `&EcsMaster` (exclusive) | yes — pure type addition | unit tests + a debug assert mirroring the writer guard |
| E3 | **`Query::{get, get_mut, single, single_mut, contains, first}`** — port from `QueryView`, which has them all; `get_mut` stamps the changed tick via the `SystemMeta` the `Query` param uniquely holds | "attack reads the TARGET's Health" today needs an exclusive system or an O(N) id scan | yes | port `QueryView`'s tests |
| E4 | **`Option<Res<R>>` / `Option<ResMut<R>>` params** — the null branch returns `None` instead of the `#[cold]` missing-resource panic; access declaration unchanged | unlocks the deferred `resource_exists` / `on_event` conditions; the one place the engine answers optionality with a panic | yes | unit test |
| E5 | **Run-condition combinators** `CombinedSystem<A, B, OP>` + `Not` — **EAGER fold** (ruling D5: a skipped RHS freezes its change-tick window), access union, tick maintenance forwarded to BOTH children | `when (A or B)` / `unless C` | yes | the 5 semantics pin tests from D5, incl. `run_once` inside `or` |
| E6 | **`ArchAdded` structural stamp** (ruling D2) — non-atomic, written only at cold sites already holding `&mut Archetype`; `Added<C>` consumers skip clean archetypes wholesale. The per-row value half is REJECTED; the amortized variant sits behind the specified bench | spawn-burst reactivity | yes (NCD const-fold) | stamp unit tests + the check-ticks clamp |
| E7 | **`CommandQueue::{mark, rewind}`** — cursor save + drop-glue replay over the byte arena | all-or-nothing for the structural half; must DOCUMENT that `reserve_entity` ids are not reclaimed | yes | unit test incl. the reserved-id caveat |
| E8 | **Event lanes**: `send`/`send_default` → `&self`; new `send_slice`; `MAX_EVENT_THREADS` → 65 + `MAX_WORKERS + 1 <= MAX_EVENT_THREADS` const-assert; debug-assert re-aim + TLS depth counter; `ordered` chunk-keyed lanes + boot-time sender-exclusivity refusal | [`EVENTS.md`](EVENTS.md) | yes (default path unchanged for serial users) | loom/stress on the lane path; two-run determinism gate for `ordered` |
| E9 | **`par_for_each_chunk_entities`** — the parallel twin does not exist (the serial one does; `par_for_each_chunk` has no entity slice) | parallel machine passes; spatial Phase 3 | yes | new-unsafe review gate (entity-slice aliasing) + `build(1) == build(W)` consumers |
| E10 | **`FLAGS_DIRECT`** — the enable-bit twin of `REQUIRES_DIRECT`: per-component initial flag states applied on attach (no transitive closure — a bit pulls no other bits; no ctor — a bit, not bytes) | the component `flags (…)` group | yes (`HAS_FLAGS` gate, same discipline as `HAS_REQUIRES`) | attach-path unit tests |
| E11 | **Refuse (or filter) `#[require]` of a bitset tag** — today the required-expansion path reaches `pool_id_for(...).expect(...)` for an id that owns no pool: read as a guaranteed panic, **unconfirmed by a test** | today's `requires` of a `flag` is a boot-time crash with a misleading invariant message | n/a | write the red test FIRST — this is still a code-reading claim |
| E12 | *(deferred, measure first)* `VmColumn` promotion to pub; the g6-style A/B of `iter` vs `for_each_chunk` on the machine-pass shape; the D2 value-half bench | — | — | criterion |

## `boyko_macros`

| # | Item | Why | Oracle |
|---|---|---|---|
| M1 | **`state_chart!`** — the machine flattening moves here (leaf enum, LCA chains, innermost-wins, **per-leaf route merge**). The merge fixes the both-chains-run defect and lands first-declared-wins arbitration (owner: align) | Aether stops being a codegen authority; hand-written Rust gains hierarchical charts | red-first: two events, one leaf, one frame → exactly ONE exit/action/enter chain; reachability/dead-state analysis tests |
| M2 | **Unlock `on_despawn`** in `#[derive(Component)]` — the derive still refuses the key ("deferred to Phase 14b") while the kernel hook field exists and fires | the one push-mandatory case (despawn has no pull carrier) is unreachable from the derive | derive test + hook-fire test |
| M3 | `#[event]`: the `ordered` registration surface; the flat-constructor emission if it lands macro-side rather than Aether-side | [`EVENTS.md`](EVENTS.md), [`CONSTRUCTS.md`](CONSTRUCTS.md) | token pins |

## `aether_lang` / `aether`

The v2 construct rewrite (R3), the per-entity `machine` front-end (R5), `each`, `resource`, all
refusals, the domain-mismatch diagnostic, the cost-model note, and the two authored-scene emission
fixes (extras-bundle collapse; `spawn_batch` grouping) — specified in
[`CONSTRUCTS.md`](CONSTRUCTS.md) and [`MACHINES.md`](MACHINES.md). Gate discipline is v1's three
lanes: token pins in `aether_lang`, trybuild goldens, and the behaviour lane in `aether_tests`
against the real engine.

## New crate `boyko_spatial`

Phases 1–4 in [`SPATIAL.md`](SPATIAL.md). Phase 1 needs **zero kernel changes**; Phase 3 needs E9.
The cell-hash + CSR + key-range-scatter internals are written as reusable building blocks (three
intended consumers — gameplay, physics-broadphase convergence, streaming cells).
