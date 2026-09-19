# Schedule determinism (lane K) - review of rev 1

VERDICT: REVISE; BLOCKING=1; IMPORTANT=5

# Architecture review: kernel lane K (deterministic apply, access completeness, ambiguity detection)

## Verdict
[ ] APPROVED
[X] CHANGES REQUESTED

The plan needs one blocking fix (C1) and five important ones. K1a and K1b do not depend on C1. They could start once W3 (for K1a) and W4 (for K1b) are resolved.

## Remarks

### 🔴 Critical

#### C1. K2 covers two of the four ways the enable column is read, and the missing ones are used on the shipped host and in the Fixed schedule
**Where**: the K2 goal ("the two ways the enable column is touched"), D5, D8's open rule, K2a's files and tests.

**Problem**: D5 adds a read only to `Enabled<T>` and `Disabled<T>` (`iters/query/filter_enable.rs:181`, `:357`). Two more routes read the same column and also declare no access:
- **`IsEnabled<T>`**, a query data item. Its `init_access` is a no-op too (`iters/query/data_is_enabled.rs:170-176`).
- **Runtime enable terms** added with `Query::with_enabled` / `without_enabled` and the `QueryView` mirrors (`iters/query/enable_terms.rs:1-9`). Their tag id is supplied at call time, so it cannot be declared at init.

**Consequence**: `IsEnabled<T>` is used on the host this plan triages:
- `collect_lights` (`boyko_render/src/light_system.rs:593-596`) and `select_lighting_cull` (`light_policy.rs:199-200`) read `LightEnabled`.
- `pack_gpu_transforms` in Fixed reads `SnapInterpolation` (`gpu_transform_pack.rs:74`).
- The physics Fixed chain reads `IsEnabled<Simulated>` and `IsEnabled<Kinematic>` (`boyko_physics/src/scene_sync.rs:83,145,193`; `systems.rs:142`). The documented way to spawn a body turns `Simulated` on through `Commands` (`bundles.rs:69-73`).

Under D8, any open `Commands` holder with no ordering path to one of these readers should be a `DeferredOpen` pair. As planned, none of those pairs is reported. K3b would then pin a baseline from a detector that under-reports, and the ratchet would certify that F1's exact class is absent from the replay schedule. K2a's red-first test (1) only exercises `Enabled<R>`, so it cannot catch the omission.

**Confidence**: CONFIRMED (file:line above; `IsEnabled` does not appear anywhere in the plan).

**What is needed**:
- Add `IsEnabled<T>` to D5. The same dispatch-invariance argument applies.
- Decide what happens to runtime enable terms. Options: declare them at init through a typed form; treat a query that has them as reading every enable tag (the mirror of the open-writer rule); or name them as a limit and guard them with a census.
- Extend K2a test (1) to cover every static read route.
- Re-derive the "new pairs K2 exposes" list to include the `IsEnabled` readers.

### 🟡 Important

#### W1. Two ways a system's analysed access can be narrower than what it actually does
**Where**: D7's effective-status rule; D10's "anything else: its `Access`".

- **(a) The rule is folded per system.** A system is "open" if it is untyped or declares nothing, otherwise "closed". Now take a system with `EnableCommands<A>` plus any other deferring param that neither declares nor marks itself untyped. That system is classified as closed on {A}, and the other param's effects disappear from analysis.
  - `SystemParam` is a public unsafe trait that code outside the kernel implements (`tests/into_system_closure_inference.rs:66`).
  - This is the same lying declaration D7 rejects for `.defers_write`, brought back by aggregation.
  - There is no instance today: `Commands` is the only `HAS_DEFERRED = true` param (`commands.rs:391-394`).
- **(b) `GpuCompute` systems fall into D10's "its `Access`" branch, but their `Access` is empty by design** (`boyko_render/src/gpu_system.rs:13-25`; `ui/upload.rs:293-309`).
  - They reach the world through `DispatcherToken::world()`, which offers `resource`, `get_component_raw` and `query_entities_buf` (`system/dispatcher_token.rs:198-349`).
  - `UiUploadSystem` is documented to become the in-schedule upload that reads `ComputedRect` and the UI scratch through that view (`ui/upload.rs:333-349`).
  - Once it does, every pair it forms, including one with a UI layout writer, is silently dropped.

**Consequence**: in both cases the ratchet stays green over a real conflict.

**Confidence**: CONFIRMED for the mechanism; no instance on today's host.

**Direction**:
- For (a): account per param. A deferring param counts as open unless it declares for itself.
- For (b): classify `GpuCompute` as unknown (the same class as world-exclusive) unless it declares its access, or list it in "What this cannot claim".

#### W2. The obligation stated for KE17 is not enough to keep determinism, and nothing checks it
**Where**: D1 ("If KC-35's split window is ever built, it must keep D1"); the `may_defer` doc change at `schedule.rs:141-169`.

**Problem**: D1's determinism argument depends on the full barrier. The set of systems in a window is fixed only because the window waits for every running system. The split retire documented at `schedule.rs:147-153` lets a non-deferring system release its successors early.

**Consequence**:
- Suppose non-deferring N releases a deferring successor S (low index) while an unrelated deferring D (higher index) from the same wave is still running.
- Whether S and D land in the same window (applied S then D) or in consecutive windows (D then S) depends on completion timing.
- So the order depends on W even with ascending order inside each window.
- `k1_apply_order_w_sweep` uses 8 mutually non-conflicting deferring systems in a single wave, with no non-deferring predecessor. A split that broke this would leave the sweep green.

**Confidence**: CONFIRMED in shape, from the documented split.

**Direction**:
- State the obligation over the whole frame: the sequence of applies per frame must be a function of the build, not only the order inside one window.
- Add a sweep case with a non-deferring → deferring chain next to an independent deferring system, so a future KE17 change is checked.

#### W3. The dispatcher lane's index is not stable when lanes grow, and the unattached-thread route is unspecified
**Where**: D2 (the dispatcher lane "stays last"; states initialized outside a builder "use the dispatcher lane"; lanes grow at init).

**Problem**:
- If a one-shot or observer `EventWriterState` caches the numeric index of what was the last lane, a later build that adds a writer for `E` moves the dispatcher lane.
- `App::finish` builds Main and then Fixed (`app/app.rs:614-621`), so this can happen inside a single `finish`.
- `send_event` today derives the dispatcher lane from the dispatcher-wide `default_thread_count` (`event_dispatcher.rs:290-292`).
- It sends an unattached thread (for example host code outside `install`) to lane 0 (`tests/event_send_from_worker.rs:114-139`). Under writer-keyed lanes, lane 0 is the first writer's lane.

**Consequence**: observer, one-shot and host events land inside a writer's block instead of last. They also use up that writer's capacity, so its refusals depend on unrelated traffic, the H-02 class. This breaks D2's "each lane has one writer".

**Confidence**: CONFIRMED from the plan text and the code.

**Direction**:
- Make the dispatcher lane index invariant under growth: either a fixed index, or a sentinel resolved per type at send time.
- Route unattached threads to that lane.
- Add a test that builds a second schedule after a one-shot or observer has initialized.

#### W4. No standing gate runs the hwrt pins
**Where**: K3b ("run both … and the same with `--features hwrt`").

**Problem**:
- The CI test legs pass only `boyko-ecs/profiling-analysis` (`.github/workflows/ci.yml:101-112`).
- The feature-matrix leg runs `cargo check`, not tests (`ci.yml:389-394`).
- So the hwrt section is compiled and never run.
- F1 was an hwrt-only topology effect (`boyko_app/src/plugins.rs:682-683`).

**Consequence**: an hwrt-only edge or system change can bring back an F1-class pair while the standard gate stays green. The hwrt pin files rot until someone runs the leg by hand.

**Confidence**: CONFIRMED.

**Direction**: this test binary needs no device. Name a standing leg that runs it with `--features hwrt`, for example a CI test step or a leg in CLAUDE.md's leg list.

#### W5. The ratchet limits how many pairs are pinned, not which ones
**Where**: D14 (multiset comparison; `BASELINE_PAIRS` may only go down; "may grow only in a detector commit").

**Problem**:
- Renaming a system forces removing and re-adding lines with the same count, which both checks accept.
- A commit that fixes pair P and pins a new pair Q is indistinguishable from that rename.
- Closures that share a `type_name` make substitution invisible even in the text.
- "Only in a detector commit" is a process rule, not a check.

**Consequence**: the refactor-last campaign (bulk renames) is where a new F1-class pair can be swapped in with the count unchanged and the ratchet green.

**Confidence**: PLAUSIBLE.

**Direction**:
- Make the pin set only able to lose lines. For example, embed the frozen baseline in the test and handle renames through an explicit alias table.
- Force every new ambiguity through an edge or an in-code `ambiguous_with(reason)`, both of which are already census-visible.

### 🟢 Optional

- **O1. The W-sweep's expected row order is wrong.** The analytic value "rows ascending by (sys, ord)" ignores swap-remove on despawn and on migration (`archetype/archetype.rs`, 32 swap-remove sites). The test would be red at W=1, which invites falling back to a W=1 oracle. Compute the expected rows with a swap-remove model.
- **O2. K2a test (3) fails in release.** It is `#[should_panic]` on a `debug_assert!`, and CI has a release test leg (`ci.yml:101-109`). Mark it `#[cfg(debug_assertions)]`.
- **O3. No ratchet row covers the replay schedule.** The physics Fixed composition (`boyko_physics/src/plugin.rs:600-761`) is not a row. `EnginePlugins`' Fixed schedule holds only `pack_gpu_transforms`.
- **O4. `EnableCommands` uses its own queue.** Order within one system then follows parameter order, not call order. A `set` on an entity spawned via `Commands` in the same system silently does nothing when the params are listed the other way round (`enable_tag_api.rs:157-160`). Document this, or `debug_assert` on a reserved but not-yet-spawned id.
- **O5. `boyko_ui` sorts by `Entity` id** for flow and paint order (`layout.rs:537-545`, `:567-570`; `focus.rs:212-214`). After D1, `Children` order is deterministic while ids stay W-dependent (D3). This is outside KC-37's Fixed scope, but worth a line in "What this cannot claim".

## Positive (keep these)

- **D1 is right.**
  - Phase 1 pops without running user code, so ADG1 holds exactly.
  - Reusing `ready_scratch` costs nothing: it is only cleared (`executor_scratch.rs:612`) and never read.
  - `reset_for_frame` also cleans up the window if phase 2 panics.
  - Rejecting `running.ones()` is correct.
  - It matches Bevy's ascending-index apply.
- **D5's "no dispatch change" argument checks out.** `conflicts_with(universal, x)` is false for an empty `x` (`access.rs:214-222`). EXC2 (`schedule.rs:1326`) plus the `break` (`:1334`) mean the new bits are never consulted.
- **The D8 layout arithmetic is right.** `SystemMeta` is `repr(C)` and `zone` ends at 243 (`system_meta.rs:85,173`), so offset 248 + 8 = 256.
- **D2 also fixes a KE16 issue.** It removes the App-8 interleave, where a helping joiner mixes two systems' events in one lane (`event_dispatcher.rs:274-283`).
- **Excluding events in D10 is correct.** Readers see only the post-swap flat buffer (`event_reader.rs:78-80`, `:115-154`), and the swap runs once per frame (`app.rs:713-720`).
- **The ratchet design has several good points:**
  - allowed pairs are pinned, which catches set-level allow-lists growing;
  - the comparison is exact in both directions;
  - topological indices are not pinned;
  - K1a's expected values are analytic and it has an anti-vacuity ticket;
  - K3a's `Off` counter;
  - K3b's two red controls.
- **Every `file:line` in the plan that I checked matches the tree.**

## Open questions

1. **Group B:** does `select_lighting_cull` write an `_armed` field? This can be read now (`light_policy.rs:202-207`). Make the edge-or-allow decision in the plan.
2. **Interior mutability through `Res<T>`** (atomics shared by concurrent readers) is invisible to both D1 and K3. Is there an instance on the sim path, or should it be listed in "What this cannot claim"?
3. **How does `EventWriter::init_state` know** it is running inside a `ScheduleBuilder` rather than a one-shot or observer?
4. **What is the initial lane count at `preregister_event`?** It is W+1 today (`EventConfig::default_for(worker_count + 1)`). D2's memory figure of (writers + 1) × cap assumes it no longer depends on W.

Nothing was flagged for SIMD, prefetching, PGO or false sharing: the plan adds no hot loop and no cross-thread state. Loom does not apply either, since there are no new atomics. The plan's own N/A list is correct.