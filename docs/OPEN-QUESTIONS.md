# Open questions for the owner

Difficulties, disputable calls and things I did not understand — written down as they arise so the
owner can read them later and weigh in, rather than finding them buried in a report after the
decision was already made.

> Russian version: [`ru/OPEN-QUESTIONS.md`](ru/OPEN-QUESTIONS.md). **This file is the source of
> truth**; editing either side updates the other in the same commit. See [`ru/README.md`](ru/README.md).

**Convention.** Newest first. Each item states the situation, the options, and what it blocks. An
item is marked `RESOLVED` with the date and the owner's decision rather than deleted — the record of
*why* a call was made outlives the call. Perf and architecture forks are decided without asking, with
numbers; what lands here is VALUES, SCOPE, and anything genuinely unclear.

---

## 2026-09-22 — SCOPE: the A1 tween scratch's generation-free key lost its load-bearing fact when EM2′ landed, and closing it properly is a KERNEL change

Surfaced by the A7 merge (`feat/ui-advanced` into the integration line), where the
lane's own tripwire fired on contact.

`UiTweenScratch` (`crates/boyko_ui/src/animation.rs`) queues completions as
`(EntityId, ComponentId)` — a key with NO generation — and `ui_tween_reap` hands
that bare id to `DenseStore::remove`, which performs no liveness check (MEASURED
at A1: it removed a live, unrelated row and returned `true`). A1 argued the key
safe from three independently-measured facts. **Fact 2 — "this kernel does not
recycle `EntityId`" — is dead**: EM2′ (`0afcbd7d`, on this line) landed id
recycling on the deferred route.

The lane anticipated exactly this. Its gate `entity_ids_are_not_recycled_today`
carried the instruction *"Do not 'fix' it as a gate with no red. Its red IS the
fact changing … `ui_tween_reap` must carry `Entity` (generation included) from
here on"*, and it went red the moment the two branches met. It is succeeded in
this merge by `entity_ids_are_recycled_on_this_kernel` (pins the new fact) and
`a_completion_pair_never_outlives_its_frame_so_a_recycle_cannot_replay_it` (gates
the consequence directly, and constructs the id collision rather than assuming
it).

**What is NOT done, and why it is yours.** The key is still safe — on facts 1 and
3 ALONE. A despawn removes the entity's dense rows itself, and the completion list
is filled and drained inside one frame with nothing in the shipped schedule
occupying the window between `ui_visual_tick` and `ui_tween_reap`. That is a
SCHEDULE-shaped guarantee where fact 2 was a KERNEL-shaped one: an exclusive
system a host deliberately schedules between the tick and the reap can now despawn
an entity, have its id recycled onto a new owner carrying the same channel, and
watch the reap delete the new owner's row — damage that was unreachable before
`0afcbd7d` and is reachable now.

The options, with their prices:

- **(a) Carry the generation in the pair.** `(Entity, ComponentId)` is 12 B against
  8 B (+50 % on a buffer that peaks at one entry per completion) — the price A1
  already computed and declined. The blocker is not the bytes: `Query::iter_entities_mut`
  yields a bare `EntityId`, so the tick cannot mint a generation at all. This needs
  an `Entity`-yielding query iterator in `boyko_ecs`, which is a kernel API
  addition and is why it is not taken here.
- **(b) Make `ui_tween_reap` refuse an unfinished row.** The reap has
  `&mut EcsMaster`; a row whose `elapsed` has not reached its duration is by
  construction not the row the tick completed, and a freshly started tween on a
  recycled id has `elapsed == 0`. Generation-free and local, but it changes the
  reap's contract from "remove what the tick named" to "remove what the tick named
  IF it is still finished", and the removal currently goes through the type-erased
  `dense_registry_mut()`, which cannot read `elapsed` without naming the four
  concrete channel types.
- **(c) Leave it on facts 1 + 3 and say so at the site.** What this merge does. The
  source no longer claims a false fact and the gates pin what is actually true; the
  residual exposure is the exclusive-system window above.

Blocks nothing — the shipped schedule is safe today. What it blocks is anyone
scheduling an exclusive system between the tick and the reap without knowing the
window is now live.

---

## 2026-09-21 — D2: the b5 basis shear points the ray the WRONG way against the raster jitter; fixing it moves two software goldens (owner decision)

Lane `fix/hwrt-shadow-ray-origin`. `composite_perspective_from_view_sheared`
(`crates/boyko_render/src/view.rs`) shears the b5 forward to `fwd + right*sx - up*sy`, i.e. the
ray through raster-NDC `q + j`; the raster (`marcher_view_proj_rows_jittered`, `row0 += jx*row3;
row1 += jy*row3`) puts at pixel `q` the content of unjittered `q - j`. Under the DEFAULT
`JitterScope::RasterAndBasis` the two producers therefore sample sub-pixel positions `2j` apart —
the mechanism note's model predicts 132,510 false-shadow px at phase 4 with the shear as coded vs
53,531 for `RasterOnly`, and the probe measured 134,055 / 55,508; a correctly-signed shear predicts
0. The industry convention (Falcor `p += (-jitterX, +jitterY)`, Bevy / HDRP / donut inverting the
JITTERED projection) is the raster's direction. Pinned now by
`view::tests::basis_shear_mirrors_the_raster_jitter_pinned_until_owner_ruling` (the shear equals
the new `raster_ray_forward` at the NEGATED jitter), and Gate 1b
(`raster_ray_forward_passes_through_the_raster_sample`) proves the correct sign against the
raster rows — the `docs/TAA-PLAN.md` Decision 1 "marcher-sample-pos == raster-sample-pos" check
that was never written.

**Why not fixed on the lane:** the lane's gate is that no software golden moves, and correcting
the shear moves `taa_armed_basis` and `taa_rcas` on the SOFTWARE leg (the CSM receiver `P` shifts
sub-pixel; the SDF sphere is sampled at the mirrored offset). The HWRT origin fix is independent
of it (`SHADOW_RASTER_FWD` is derived from the UNJITTERED view, exact under either sign).

**Options.** (a) Flip the shear: `composite_perspective_from_view_sheared` calls
`raster_ray_forward`, Gate 1 (`view.rs`) flips its `(jx, -jy)` to `(-jx, +jy)`, the D2 pin test is
deleted, `taa_armed_basis` + `taa_rcas` are re-blessed on BOTH legs (hwrt: the SDF-owned pixels'
sample position moves; the mesh-owned origin is already exact). (b) Flip the raster instead —
moves every TAA pin on both legs. (c) Leave it — the marcher's SDF/mesh depth composite
(`sdf_gbuffer_composite.hlsl`, `own_pixel`) keeps comparing distances along rays `2j` apart, and
`docs/TAA-PLAN.md`'s consumer-audit row "sdf_depth_composite EXACT to fp" stays false.
Recommendation: (a). **Decision needed:** whether re-blessing two software goldens is granted.

## 2026-09-21 — H-blink1: the light header's CSM bit trails the host by two frames, so the `taa_jitter_eval` pins blink ON/OFF/ON at frames 0..2 (owner decision)

Lane `fix/hwrt-shadow-ray-origin`, measured with the new burst dump
(`BOYKO_HOST_DUMP_SETTLE=0 BOYKO_HOST_DUMP_FRAMES=3` on `vb_mesh_shadows`, software leg): the
state lines read `csm_armed=1 header_csm=1` / `csm_armed=1 header_csm=0` / `csm_armed=1
header_csm=1` on frames 0, 1, 2. `sync_csm_light_gate` (`crates/boyko_render/src/csm_caster.rs`)
has no ordering edge against `resolve_csm_cascades` nor against `collect_lights`
(`crates/boyko_app/src/plugins.rs`; the order dumps show `gate < fit` and `collect < gate`, both
NO PATH, 34/34 frames), so at frame k it reads frame k-1's fit and its write is folded at k+1:
`header(k+1) = armed(fit(k-1))` while `host(k+1) = fit(k+1)`. The fixture hand-seeds
`LightingConfig::csm_shadows = true` (`taa_jitter_eval.rs`), so frame 0 is ON (the seed), frame 1
OFF (the gate wrote `false` off the DISABLED seed fit), frame 2 ON. Pre-R4 the host was unarmed
on frame 0 (0 casters), so the sequence was OFF/OFF/ON and no blink; R4's frame-0 edges exposed
the pre-existing stagger.

**Fix shape:** `sync_csm_light_gate.after_set(CsmResolveSet).before_set(LightCollectSet)` (the
`sync_sv0_light_gate` / `sync_cluster_light_gate` shape in `plugins.rs`), or derive the header bit
from the runner's own `csm_armed`. Either re-blesses the seven pins with `csm_shadows` hand-seeded
and mesh casters, on BOTH legs (`taa_armed`, `taa_armed_basis`, `taa_rcas`, `vb_taa`,
`vb_taa_rcas`, `vb_both_taa`, `vb_mesh_shadows` — frame 1 enters the TAA history at 10 %), which
is why it is not on this lane. The follow-up gate: `header_csm == csm_armed` on every burst line
with `frame >= 1`. **Decision needed:** grant the seven re-blesses (a one-frame start-up blink is
the cost of leaving it).

## 2026-09-21 — VB D7 (latent): `vb_shadow_vis` has the same b5-ray origin the Deferred HWRT resolve just fixed

`crates/boyko_rhi_vulkan/shaders/vb_shadow_vis.comp.hlsl` casts its cone from
`P = ro + rd*view_t` on the b5 ray; VB's `viewt_from_depth_rz` puts `P` on the right view-z plane
but laterally `t*Δθ` off under TAA — the same self-hit class the Deferred fix removed, once the
VB geo/shade split is armed (no pin arms it today; `vb_taa` renders identically on both legs). The
`RayShadowUbo` fields the lane added (`SHADOW_ORIGIN_MODE`, `SHADOW_RASTER_FWD`) are uploaded on
VB × TAA hwrt frames too and are the forward seam: every VB pixel with hardware depth is
raster-owned, so no depth producer test is needed there. Do it when the split is armed. Two
related notes, not decisions: the RTG ch. 6 ulp offset stays deferred until a large-world
(|P| ≳ 1e3) need appears (it needs the geometric normal the G-buffer does not carry); the
SDF-cast shadow onto mesh pixels (`sdf_gbuffer_composite.hlsl`, `P_mesh = ro + rd*t_mesh`,
software-shared) carries the same reconstruction error under a 20-mm normal lift — safe at
≥ 512 px (`1/h` scaling), worth a row in `docs/TAA-PLAN.md`'s consumer-audit table, whose
"sdf_depth_composite EXACT to fp" row is false under D2. The software leg's
`csm_visibility(P, ...)` receives the same b5-reconstructed `P` on raster-owned pixels (≤ 11 mm
at 512 px), swallowed today by the CSM normal offset / PCF — the same table row.

---

## RESOLVED 2026-09-21 — Rung A0: both `claude/trusting-ramanujan-0f8927` commits are OBSOLETE; nothing is cherry-picked, one idea is owed in E4's shape

Rung A0 of the unified plan asked whether the two non-equivalent commits on
`claude/trusting-ramanujan-0f8927` — `6a9871bb` "fix(ecs): event-lane width hazard + worker-panic
propagation" and `867dd734` "docs: OPEN-QUESTIONS.md seed", base `e65a5673` — merge into this line
or are ruled obsolete. The code-reviewer read the worktree, this line (`D:/wt/joltab` @ `ff64c6be`)
and the plan; no cargo was run (timing window). Ancestry by `git`: `e65a5673` IS an ancestor of
`ff64c6be`; the two commits are NOT; A2 (`1693234d`, `fix/pool-panic-and-reservoir-loom`) IS, via
`e3d077c9` → `cb5fbf4c`.

**Ruling: both OBSOLETE.** Nothing is cherry-picked. The branch is pushed, so the commits stay
reachable after the worktree is pruned; the prune is allowed once the orchestrator accepts the two
verdicts (04 s4).

**`6a9871bb` — obsolete in all three halves, each for a different reason.**

* *Worker-panic half — superseded by A2, a strict superset.* The commit wraps the system body in
  `catch_unwind(AssertUnwindSafe(..))`, publishes completion unconditionally, then `resume_unwind`s
  into `Scope`; A2 does it with `SystemRunGuard` (a Drop guard: `finish()` on the normal path, the
  unwind path publishes, restores the TLS depth first, claims `panicked` first-wins), adds
  `cancel_after_panic` so the FIRST panic is the one delivered and the apply window does not run
  over a panicked frame (the commit leaves it free to — its dispatcher keeps going and applies the
  panicked system's commands), and `Scope::drop` decides by `body_returned`, not
  `thread::panicking()`. Tests: the commit's one test with no watchdog (a regression HANGS it red)
  against A2's 18 under an in-test 10 s `DEADLINE` in
  `crates/boyko_ecs/tests/a6_schedule_panic_propagation.rs` plus the threadpool's 14 + loom + Miri
  receipts; the commit's test is contained in two of them. Both hunks edit the same `scope.spawn`
  closure, so a cherry-pick conflicts and the resolution is "take the line's side" in full.
* *Constants half — superseded by KE8.* `MAX_EVENT_THREADS = 65` plus
  `const _: () = assert!(MAX_WORKERS < MAX_EVENT_THREADS)` is already on the line
  (`crates/boyko_ecs/src/ecs/constants.rs`); the commit's `MAX_WORKERS as u32 + 1` is the same value
  on the same lines — conflict, redundant.
* *Lane-gate half — the hazard is CONFIRMED live on the line, and the commit's CONTRACT contradicts
  ruling E4.* Live: `EcsMaster::new` hard-wires `EventDispatcher::new(1)`, `default_thread_count`
  has no setter, `App::with_pool` does not tell the dispatcher the pool width; an `EventWriter` on
  worker `k >= n` indexes lane `k` of `n` — debug assert, release out-of-bounds panic, and which
  worker the scheduler picks decides whether it fires (the flaky-red class; before A2 a silent
  hang). Four `aether_tests` files (`a1_bundle_event.rs`, `a3_machine.rs`,
  `a4_machine_hierarchy.rs`, `r2_chart_arbitration.rs`) work around it with
  `MAX_EVENT_LANES = 64`, which fails the same way on a 64-worker box. Ruling **E4**
  (`docs/aether-v2/DECISIONS.md:482-537`, 2026-08-30, delegated from ballot AB-2) measured the same
  hazard (release, 4-worker pool, `preregister_event_default`, 64 sends → 3 panicked) and ruled
  RAISE: effective lanes `max(N, worker_count + 1)`, resolved where the pool is first known,
  reported once at boot — and REJECTED boot refusal by name ("one source that is correct on a
  4-core laptop becomes unbootable on a 64-core server, machine-dependently … strictly worse").
  The commit refuses boot: `preregister` returns `Err(EventConfigTooFewLanes { lanes, required })`,
  and its two gate tests assert exactly that, so they cannot be kept either. Its argument against a
  clamp ("silently reintroduce contention") does not apply to E4's shape: raising ADDS lanes and is
  reported. Merging it would settle a fork silently against the line's own decisions record. Of the
  commit's 13 files, five conflict textually with the line, one hunk drops wholesale, and
  `docs/FEATURE_MAP.md` / `docs/SYSTEMS.md` are anchor docs to be redone by hand, never UNION.

**Carried forward as owed work in E4's shape — NOT a merge (rung unassigned; question 1 below):**

1. `EventDispatcher::set_default_thread_count(n)` in `event_dispatcher.rs`; keep the commit's
   ordering rule "refuse after a registration" — it makes the `App` wiring site the only site, and
   E4's feasibility argument depends on that order.
2. `App::with_pool` wires `pool.worker_count() + 1` before any plugin runs. `worker_count()` is
   `u32`, clamped to `[1, 64]`, so `+1 <= 65 = MAX_EVENT_THREADS` holds; the commit's
   `expect("invariant: …")` form is honest.
3. In `preregister`: RAISE `cfg.thread_count` to `max(cfg.thread_count, default_thread_count)` and
   report once — not `Err`. The raise also puts `send_event`'s dispatcher lane
   (`default_thread_count - 1`) on a real lane in `App`-hosted worlds, which E4's probe showed it
   never reaches today.
4. Red-first tests rewritten, not copied: `App::with_threads(2)` + `preregister_event_default` →
   `events().default_thread_count() == 3` and events flow from both workers
   (`preregister_event_default_sizes_to_pool_and_flows` is reusable as-is); `App::with_threads(2)`
   + `default_for(2)` → registers with 3 lanes and one boot log line, not an `Err`. The A2 suite's
   `DEADLINE` form, not a "run with a timeout wrapper" header.
5. Sweep: the four `aether_tests` `MAX_EVENT_LANES = 64` sites become
   `default_thread_count()`-derived; `app_multi_schedule.rs:217`, `:332` →
   `preregister_event_default`. There is NO production `preregister_event` site on the line (grep
   of `crates/*/src`, `src/`): the hazard bites test authors, not the shipped engine, and A2 already
   makes their failure loud.

Scope note the assignee carries: D-E20 (`02-ORDER-OF-WORK.md:181`; U-21; H-02) keys lanes by
WRITER at `init_state` in registration order and makes `send_event` dispatcher-only — under it
there is no `worker_count + 1` requirement, so items 1–3 are interim and D-E20 deletes them (the
plan's A1/A1b "interim code, U5/U7 delete it" pattern). `app.rs` is locked by D-E8/D-E9/D-E22, not
D-E20; landing item 2 adds `app.rs` to D-E20's deletion surface and must be named there.

**`867dd734` — obsolete.** This file already cites the seed by hash and names both its entries
(the 2026-08-21 entry "The owner channel exists twice", below, now marked resolved); an add/add
merge of a 40-line file with its own H1 and its own convention ("resolved entries move to the
bottom") into a 7,700-line descending-chronology file is exactly the "union re-imports pre-ruling
text" hazard (04 s2). Entry 1's DECISION was closed by history — `master` fast-forwarded on
2026-09-07 carrying `clippy.toml` — and its MECHANISM paragraph, the only thing at risk of loss, is
now appended to the 2026-08-21 entry. Entry 2, the observation that `send_event` routes the
dispatcher to `default_thread_count - 1` while `EventWriter::send` routes it to
`buffer.thread_count - 1` — two lanes when a type is registered wider than required, both
single-writer, no race — is not on the line in this form; E4's probe recorded the adjacent,
stronger fact (`send_event` never reaches the reserved lane at all today). After E4 it stays true
for over-wide registrations (the `aether_tests` 64-lane sites on an 8-worker pool: lanes 8 and 63);
D-E20 dissolves it. Worth one sentence in the E4 implementation notes, nothing else.

**Two questions this ruling leaves open — orchestrator's, not the reviewer's:**

1. **Which rung owns E4's setter + wiring (items 1–3)?** The unified plan does not reference
   E4 / AB-2 / KE8, and D-E20 retires the worker-id lane model the floor exists for. Three
   consistent options: (a) fold E4 into D-E20 and let writer lanes make the floor moot (the aether
   `MAX_EVENT_LANES = 64` sites stay as they are until then); (b) land items 1–3 as an interim S
   rung in A5's batch, deletion named in D-E20, `app.rs` added to its lock set; (c) leave it on
   Aether's KE8 row (`docs/aether-v2/KERNEL-BACKLOG.md:47`) outside the plan. The fact that settles
   it: whether any rung before D-E20 adds a PRODUCTION `preregister_event` site on an `App` with
   W >= 2 — today there is none.
2. **Does ruling E4 survive rev 6.1 of the plan at all, given D-E20?** If "D-E20 supersedes E4",
   `docs/aether-v2/KERNEL-BACKLOG.md:47` and `docs/aether-v2/DECISIONS.md:482` should say so, or a
   later reader implements E4 into a lane model that no longer exists.

The plan's A0 row (`UNIFIED-SYSTEM-PLAN-02-ORDER-OF-WORK.md`) lives on the main checkout, not on
this line, and is updated at A8. The review itself is on the orchestrator's board, not in the tree;
everything load-bearing from it is reproduced above.

---

## 2026-09-21 — What R4 / R4b leave to the kernel lane K: the pairs the light-table lane did not order, and why

Lane `fix/light-table-defects`, R4-frame-order and R4b-open-edges. The diagnosis behind R4 (an
ordering edge that carried no data moved six hwrt TAA goldens, because `gather_mesh_draws` and
`visibility_sync` had no ordering path and frame 0 drew no meshes) listed every unordered pair of
the hwrt `taa_jitter_eval` host's `Main` schedule: 236 pairs, 33 of them with a declared access
conflict. This lane declared the cross-plugin reader → writer pairs it could show in code, and
hands the rest to lane K.

**Declared here**, each as a named set edge in `EnginePlugins::build`
(`crates/boyko_app/src/plugins.rs`) with a `crates/boyko_app/tests/host_orders_*.rs` cycle gate:
the `RenderEnabled` readers after `visibility_sync` and after `validate_asset_refs`
(`VisibilitySet::{Sync, Validate, Read}`; the validation both reads the bit and clears it on a
stale row), the instance packs after propagation, `light_reconcile` after propagation, and the CSM
fit and the punctual atlas resolve after the camera resolve and after `light_reconcile`.

Two of these edges moved goldens: the 14 TAA pin-legs re-blessed on 2026-09-21 (`goldens/PINS.toml`,
the `[taa_armed]` note and its siblings) had pinned a frame 0 that drew no meshes (the
`visibility_sync` / reader order, R4's E1) AND was lit from the sun's identity `GlobalTransform`
(`light_reconcile` before `propagate_transforms`, the 2026-09-19 (b) measurement below, R4b's edge
C); every note states both causes, the pixel count, and, for hwrt, the mesh-only value the G2 edge
alone had produced.

**Left unordered, by ruling (2026-09-20, R4 §6), with the reason:**

- `select_lighting_cull` vs `light_reconcile`: **unordered, no shared data.** The cull
  (`crates/boyko_render/src/light_policy.rs`, `select_lighting_cull`'s two queries) reads
  `IsEnabled<LightEnabled>` under structural `With<PointLight>` / `With<SpotLight>` filters and
  reads no field of either component; the reconcile (`crates/boyko_render/src/light_reconcile.rs`,
  `light_reconcile`'s three loops) writes only `position` / `direction` and never touches
  `LightEnabled`. The diagnosis flagged the pair only because `With<C>` conservatively declares a
  read of `C` (`crates/boyko_ecs/src/ecs/core/iters/query/filter.rs`, `With`'s "Access surface"
  doc); the bytes the two systems touch are disjoint, so no order between them changes any output.
- Every other pair in that list — the `LightingConfig` / `LightTableDirty` write/write cluster
  among the lighting gates, `sync_csm_light_gate` vs the CSM fit on `ResolvedCsm`, `snap_apply`
  vs the camera and caster controllers on `Transform`, the particle pack vs tick on
  `ParticleClock`, and every pair one side of which is an exclusive system — is the class, not
  the instance. **Lane K** makes the class a kernel gate (ambiguity detection, deterministic
  apply, complete access declarations): `docs/scheduler/determinism-K/` (`00-DIAGNOSIS.md` is the
  bisection, `00-RULINGS.md` the rulings; its "Lane order" cuts K after this lane merges, so the
  ratchet's frozen set is taken with R4's edges in). Those documents landed on the integration
  line as commit `d4213813` (`merge/ke16-into-ecsnative`) and are not on this lane's base
  `1c31aeac`.

**Nothing here is a question for the owner**; the entry is the hand-off record, so the next reader
of an ambiguous pair in this schedule finds the ruling that left it and the lane that owns it.

---

## 2026-09-19 — Three pre-existing findings the light-table lane surfaced and did not fix

Lane `fix/light-table-defects` (R1, R2, R2b, R3, and the R2b seed → fit edge). All three predate
the lane and none is fixed by it. (a) and (b) are questions; (c) adds a failure mode and a rate to an
entry that already exists.

### (a) A light spawned between frames through a world-level `run_system` is never enabled, and nothing says so

**Measured.** `crates/boyko_render/tests/primary_directional.rs`, as first written for R2, spawned
its suns with `app.world_mut().run_system(|mut cmds: Commands| …)`. T1's sun was spawned before the
first `app.update()`, and the seed enabled it. T2's two suns were spawned between `app.update()`
calls several frames later, and the seed never enabled them, so neither reached the light table and
the test panicked at `:272` ("T2: two enabled suns") for both the R2 and the R2b tester. A copy that
set the bits itself passed. The test now sets them itself (review C1), so it no longer shows the gap.

**Why, read from the code (not separately measured).** After its one first-run full scan,
`LightSeedState::seed` (`crates/boyko_render/src/light_system.rs`) finds new lights only through
four `Added<*Light>` sub-systems, and `run_added` stamps their window itself:
`(previous pass's end, world.current_tick()]`. The second pass is the exception: the sub-systems
are first initialized there, with a start `MAX_CHANGE_AGE` ticks back, so it catches everything
spawned since the first pass. From the third pass on the window is as stated. Inside
`Schedule::run` the world tick is bumped
twice before any system runs — `this_run` at `schedule.rs:383`, then the apply-window bump at
`schedule.rs:478` — so each seed pass ends its window at `this_run + 1`, and the world keeps that
tick until the next `Schedule::run`. A `run_system` between frames applies its commands at that same
tick (`EcsMaster::run_cached_system` → `System::apply`, `system_api.rs:142-157`), and
`Tick::is_newer_than` (`tick.rs:168`) excludes a tick equal to the window's start. So the next pass
starts its window exactly at the spawn's tick and skips it; no later pass looks again. The seed's
own doc (`collect_added_light_ids`) says `current_tick()` is the frame's `this_run`; after the
apply-window bump it is `this_run + 1`.

Two consequences of the same arithmetic, neither measured:
- A light spawned by a `Main` system whose commands are applied AFTER the seed has run that frame
  lands on the same tick and should be skipped the same way. Ordered or applied before the seed, it
  is caught.
- A schedule that runs between the spawn and the next `Main` run (`Fixed`, on a frame where it runs
  a substep) advances the tick and should make the spawn visible. If so, whether such a light ever
  renders depends on the frame rate.

**Who is exposed.** Every in-tree example spawns its lights in a startup system, which runs before
the seed's first pass, and the full scan catches them. Not checked: scene load, deserialisation
(`boyko_serialize`), Aether-expanded spawns (`crates/aether_lang/src/expand.rs` emits light
spawns), and any editor that spawns into a running world.

**Question.** Does any production path spawn lights between frames, or from a `Main` system that
runs after the seed? If one does, those lights are never rendered and no warning, counter or assert
reports it. A probe that spawns a light through each such path and checks its `LightEnabled` bit
one frame later would answer it.

### (b) The frame-0 light table carries `[0, 0, -1]` for a sun spawned with an identity `GlobalTransform`

**Measured** (R2 tester, the behaviour-neutral M7p probe in `crates/boyko_app/tests/sdf_marcher_sun.rs`,
under `EnginePlugins::window`). The suns were spawned in a startup system with a posed `Transform`
and `GlobalTransform::IDENTITY`. The first `scene()` call — the frame-0 table — received
`Some([0.0, 0.0, -1.0])`, the identity pose's direction; the second call and every later one
received the posed direction.

**What the code says.** `LightingPlugin`'s doc ("Add-order contract", `light_plugin.rs`) requires
`light_reconcile` to run after `propagate_transforms` and says the plugin cannot express that edge.
`EnginePlugins` adds `CameraPlugin` (`plugins.rs:430`) before `LightingPlugin` (`plugins.rs:476`)
and declares no edge between `CameraSet::Resolve` and `light_reconcile`, so the order is add-order
alone. The same file's `ParticleTickSet` comment records that add-order is not a pin: a pose written
in `Main` was measured drawn one frame late for exactly that reason.

**Not known.** Whether `light_reconcile` runs before propagation only on frame 0 or on every frame.
If every frame, a moving light's direction or position trails its transform by one frame for as long
as it moves; the `Changed<GlobalTransform>` gate makes a static light catch up, not a moving one.
The gates in this lane now spawn their suns with a posed `GlobalTransform`
(`sdf_marcher_sun.rs`, `csm_primary_sun_agreement.rs`, `primary_directional.rs`), so none of them
sees frame 0 any more.

**Question.** Is this an accepted one-frame lag, or a missing edge? The workspace already has a
precedent for the edge: `ParticleTickSet`, where the plugin declares set membership and the
composing app declares `.after(CameraSet::Resolve)`. It blocks nothing today: the pinned dumps are
taken well after frame 0.

**RESOLVED 2026-09-20 — a missing edge** (the orchestrator's R4 ruling §6(c): add it where the
`GlobalTransform` read is shown in code, which it is — the three `&GlobalTransform` queries in
`light_reconcile`'s signature). R4b-open-edges declares `LightReconcileSet.after(CameraSet::Resolve)`
in `EnginePlugins::build`, gated by
`crates/boyko_app/tests/host_orders_light_reconcile_after_propagation.rs`. The "only frame 0 or
every frame" question above was never measured and is moot under the edge. The measurement above
turned out to be in the goldens as well: the 14 TAA pin-legs re-blessed on 2026-09-21 had carried
that identity-lit frame 0 in their TAA history (the second of the two causes in each
`goldens/PINS.toml` note; see the 2026-09-21 entry above), and with the edge in place they render
the posed sun on frame 0 too.

### (c) `boyko_rhi_vulkan --test sdf_gbuffer_hybrid` crashes with `0xC0000005` under a six-thread run

This binary is already recorded under **KNOWN FRICTIONS** ("⚠️ A KNOWN-RED TARGET IS A SHADOW",
2026-08-10), in the bullet "Not red at all, but FAILING UNDER THE FULL PARALLEL SWEEP" (43/43 in
isolation). That entry stands; this adds what the failure is and how often it happens.

- **Mode:** the process exits `0xC0000005` (STATUS_ACCESS_VIOLATION) about 3–4 s in, with
  `RUST_TEST_THREADS=6`, while six GPU tests boot at once.
- **Rate (R2b/R3 tester, interleaved, 20 runs per arm):** 1 of 20 with R3's `ddgi.rs` reverted to
  HEAD, 2 of 20 on the lane tree. Earlier reruns crashed 2 of 8.
- **Every run that did not crash:** 43 passed, 0 failed, 8 ignored.
- The lane's R1 and R2 workspace runs, at the default thread count, did not hit it.
- **Root cause:** unknown.

**Question.** For the lane-wide gate: run this binary with `--test-threads=1`, or root-cause the
concurrent boot first?

---

## 2026-09-19 — What R2 leaves behind: sibling defect R2c, and what the `grand_showcase_2mat` re-bless carried

R2 (`docs/render/light-table-defects/R2-DESIGN.md`, lane `fix/light-table-defects`) makes the
Deferred SDF marcher shadow toward the light table's primary directional, read every frame
(`LightTableStaging::primary_directional_dir`, turned into the push by `gpu_scene`'s `MarcherSun`),
and bake no sun shadow at all on a frame without a directional. Four things are left over. None of
them is R2's to do, and none is a question to the owner yet; they are written down here so they are
not lost. Two of them, the stale digest citations and the two one-pixel pins, were closed by the
re-bless on 2026-09-19 and stay below as the record.

### R2c — on a sunlit frame, other lights take the PRIMARY sun's shadow mask (open)

On Deferred, `gMaterial.r` holds the marcher's shadow toward the primary sun, and the resolve
applies it to lights that are not that sun:

- **Every extra directional** gets it by default (`deferred_pbr.hlsl:982`, `float vis = shadow;`).
  Only an extra directional flagged as an SDF caster, in multi-light mode, marches its own shadow.
- **Every point and spot light** gets it while punctual shadows are off (`deferred_pbr.hlsl:1371`,
  `vis = (punctual_shadow_mode != PUNCTUAL_SHADOW_MODE_OFF) ? 1.0 : shadow`).

So a point light goes dark wherever the sun is occluded, and a second sun is shadowed toward the
first. R2 fixed only the sunless half: with no directional the marcher clears `SHADOWS`, so
`gMaterial.r` is `1` and the punctual lights are unmasked. On a sunlit frame the mask is now the real
sun's instead of the boot constant's, so in an unpinned scene with punctual lights the masked region
MOVES rather than disappears. The punctual half is the same rule `unwritten_shadow_map_gate.rs`'s F3
note describes, from the other side: header bit 3 decides whether a punctual light takes `shadow` or
owns its visibility. Not designed; whoever takes it starts from those two lines.

### The hand-copied `f6147f90` citations, repaired at the re-bless (2026-09-19)

`f6147f90` is the pin's pre-R2 digest. R2 moves the pin by design (a thin terminator crescent on
four of the five spheres); the owner signed off the change on 2026-09-19 and both legs were
re-blessed to `7e71e2a6`. Every hit was re-derived by grep on the lane and sorted by what it claims:

- **Re-pointed to `7e71e2a6`**, because each names the pin as the CURRENT Deferred golden:
  `crates/boyko_app/tests/grand_showcase_2mat.rs:191`, `:226`;
  `crates/boyko_app/tests/forward_mesh.rs:16`, `:85`; `crates/boyko_app/tests/vb_mesh.rs:6`;
  `crates/boyko_app/tests/vb_mesh_ssao.rs:7`; and `[forward_mesh]`'s two comments in
  `goldens/PINS.toml` (now `:177` and `:185`). The second says Forward's sky matches the Deferred
  golden's background; R2 changed no sky pixel, so it holds for the new digest.
- **Left as written**, because each is a dated plan record, not a current value:
  `docs/MULTI-PARADIGM-RENDER-PLAN.md:26`, `:480` and `docs/RENDER-PARITY-PLAN.md:46`, `:362` give
  their rungs' gate set next to `58f6c6c3`, a digest retired on 2026-07-12 (`8e48f7fe`), so the set
  was already history before R2; `docs/RENDER-AA-AND-TAILS-PLAN.md:33` names "the current goldens"
  of its 2026-07-12 mandate, the day `f6147f90` was blessed. Re-pointing the one digest would put a
  2026-09-19 value into a July record. The light-table design documents
  (`docs/render/light-table-defects/`) cite `f6147f90` as the pre-R2 value, which it is.

The review also named `PARTICLES-PLAN`; that file carries no `f6147f90` on this tree.

### Two more pins R2 moves, one pixel each: `taa_armed` and `particle_sdf_collide` (re-blessed 2026-09-19)

`grand_showcase_2mat` is not the only pin the re-bless had to carry. In both of these scenes the sun
IS the old boot constant, `[-0.45, 0.82, 0.36]` (`crates/boyko_app/tests/taa_jitter_eval.rs:171`,
`crates/boyko_app/tests/particle_scene/mod.rs:196`), so the marcher shadowed toward that direction
before R2 as well. What changed is its bits. The old push was the raw, non-unit constant, which the
shader normalises. After R2 the push is the table's value: the pose's direction after the
quaternion round-trip, normalised by `GpuLight::from_directional`. The two are the same direction to
rounding, not bit for bit, and that difference crosses one 8-bit rounding step in each frame.

Measured on the lane on 2026-09-19 with `golden.ps1 -Pin <pin> [-Hwrt]` (never `-Bless`):

| pin | leg | before (the pin) | after R2 | diff |
|---|---|---|---|---|
| `taa_armed` | software | `765de1d9866f37748bcf4182a21cd4d3307f94c140fb54300d6fd85adefa1b62` | `31ce517879f042eb2579a5f666155193cf488f1f4da66e9ece12147fc65ffb31` | 1 px at (236, 191), 1 LSB darker |
| `taa_armed` | hwrt | `0acc66b55dcbf6ccdbdade9b55e1861ac24a649df179bb1bbb74437058026b45` | `c6429c3ed8d87ac27b1603404f787e7f2cca275a683ee011517ac2fda91c9235` | 1 px at (236, 191), 1 LSB darker |
| `particle_sdf_collide` | software | `729f5ad69846b146dcd07dc840943234af3781b8b127fd5da760b918a6784704` | `c6a055a1518ca4deef97dd942a1b3592ff68e471733643f417b6336135b23bc3` | 1 px at (463, 385), 1 LSB brighter |
| `particle_sdf_collide` | hwrt (`PENDING` in `PINS.toml` until the re-bless; "before" is a pre-R2 run) | `729f5ad6…` | `c6a055a1…` | the same pixel |

**The A/B that attributes it.** The same R2 build was run with the old raw constant pushed through
the new path whenever the table has a sun. It restored every old digest: `taa_armed` `765de1d9…`
(software) and `0acc66b5…` (hwrt), `particle_sdf_collide` `729f5ad6…` on both legs, and
`grand_showcase_2mat` `f6147f90…` on both legs. So on a sunlit frame the sun's bits are the only
difference, and these two moves are rounding, not a behaviour change. This is the class the rulings
put `deferred_sdf_casters` in (`docs/render/light-table-defects/00-RULINGS.md`, R2 W3). That pin did
not move.

Re-blessed on this basis on 2026-09-19, together with `grand_showcase_2mat`: `taa_armed` on both
legs, and `particle_sdf_collide` on both legs, its hwrt leg pinned for the first time, at the
software value the table above measured (the re-bless's own run rendered `c6a055a1…` again).
Each pin's note in `goldens/PINS.toml` records the reason. `particle_sdf_collide` still waits for
the owner's look at its image, as it did before R2.

### Unpinned owner reference dumps that change by design

These scenes render Deferred × Both (the default) with a sun that is not the old boot constant, and
none of them asserts anything. A new image from one of them after R2 is the fix, not a regression:

- `crates/boyko_app/tests/pbr_material_showcase.rs:77` — sun `[-0.55, 0.30, 0.42]`, about 31° from
  the constant, so its change is the largest.
- `crates/boyko_app/tests/pbr_showcase.rs:30`, `crates/boyko_app/tests/textured_smoke.rs:34`,
  `crates/boyko_app/tests/grand_showcase_mvpm.rs:32` — sun `[-0.40, 0.78, 0.48]`, 7.8° off, the same
  thin crescent `grand_showcase_2mat` shows.

---

## RESOLVED 2026-09-18 — An un-slotted punctual light samples another light's shadow map on armed frames

✅ **RESOLVED 2026-09-18 by the light-table-defects lane (`fix/light-table-defects`), as an
architecture call — not an owner decision, and not a question to the owner.** The fork below
("which side owns the value") was taken on cost and on whether a gate could be shown able to fail:

- **Taken: the host builds every point/spot row with `SLOT_NONE` in the slot field.**
  `GpuLight::from_point` / `from_spot` and their golden mirrors `GoldenLight::point` / `spot` OR in
  `SLOT_NONE_FIELD` (`0x003E_0000`, defined next to `GpuLight` in
  `crates/boyko_render/src/light.rs`; `shadow_atlas.rs` pins it to `SLOT_NONE << ATLAS_SLOT_SHIFT`
  at compile time). `slot_pack` keeps its guard, so the constructor is the value's single owner and
  the fold only ever adds a real assignment on top. The six shader sites already reject the
  sentinel: no shader code changed, no `.spv` was re-emitted, no manifest row moved;
  `light_table.hlsli`'s false comment was corrected comment-only. On an armed frame an un-slotted
  light now skips its PCF instead of paying for a wrong one.
- **Rejected: the six sites also test bit 16.** Two more ALU ops per light per pixel on armed
  frames, six hand-edited sites and up to 20 `.spv` re-emitted — and it does not fix the class,
  because `GoldenLight::with_sdf_shadow()` sets bit 16 with no slot.
- **Rejected: store `slot + 1`, so 0 means "none".** One more integer add per light per pixel,
  the same `.spv` churn, and a new encoding for every slotted row, to protect rows only `from_*` /
  `GoldenLight::*` ever build.
- **Rejected: the fold always packs.** A row built outside the fold would still decode slot 0,
  and the value would have two owners — so no single mutation could reintroduce the defect, and
  no gate could be shown able to fail.

What the call cost, on purpose: an un-slotted table is no longer byte-identical to the pre-Inc-1
fold. In the slotted encoding the old word MEANT "slot 0", so that identity was the defect; the
pixel 0%-gate is what the pins check, and it is unaffected by design.

**The exposure claim was refuted by reading, before any run.** The brief said the two froxel pins
were exposed; they are not. Neither `vb_mesh_froxel` nor `vb_mesh_tex_froxel` spawns a
`ShadowCaster`, so the punctual depth pass never arms and header bit 3 stays off there (the section
"Who is exposed" below). The only exposed scene is the unpinned
`crates/boyko_app/examples/vb_lab.rs`, whose un-flagged blue point read the spot's face record.

The gates that pin it (`docs/render/light-table-defects/R1-DESIGN.md` section 4):
`light_system.rs`'s `every_punctual_row_decodes_exactly_its_assignment` (every assignment of three
points and three spots), `mesh_shadow_arming_agreement.rs` (the production resolve → assignment →
`collect_lights` path, with an un-flagged point, an un-flagged spot and a budget loser),
`lighting_l1_host_oracle.rs`'s `every_light_row_producer_emits_the_slot_none_sentinel` (both
producers, plus the shader's own spelling of the field, with a negative control), and the device
gate `unwritten_shadow_map_gate.rs`'s `unslotted_punctual_lights_never_sample_the_atlas`. The
poison probe now also records `sampled_rows` — the rows the shader's own predicate
`light_atlas_slot(kind) != SLOT_NONE` would sample — beside the `CASTS_SHADOW_BIT` count the last
paragraph of this entry explains.

Out of scope and not an owner question: bit 16 means "slotted" to the host and "casts an SDF
shadow" to the shader (follow-up R1-F1 in the design); it moves no pixel today.

### The record as written before the fix

Found by the vkval lane (round 3, the shadow-gate stage) while writing the punctual half of the
unwritten-shadow-map gate. **It is pre-existing and that lane does not fix it.** The fix is
decided (below): it is host-side only, and it lands in its own lane after this one. Line numbers
below were verified on 2026-09-18.

### What happens

- A punctual light's atlas layer lives in its own light-table row: the slot field of the kind word,
  read by `light_atlas_slot`. A row whose light got no slot — no `CastsPunctualShadow`, or it lost
  the slot budget — should carry `SLOT_NONE` (`0x1F`). **It carries 0.** `collect_lights` builds
  the row through `slot_pack` (`crates/boyko_render/src/light_system.rs`), which skips the pack
  for a `SLOT_NONE` base to keep the 0%-gate bytes, so the field stays as
  `GpuLight::from_point` / `from_spot` left it.
- There are six shader sites, and every one checks the header's punctual bit and then ONLY
  `slot != SLOT_NONE`:
  - `crates/boyko_rhi_vulkan/shaders/deferred_pbr.hlsl:1332-1334`
  - `crates/boyko_rhi_vulkan/shaders/forward_opaque.fs.hlsl:414-416`
  - `crates/boyko_rhi_vulkan/shaders/sdf_forward_march.comp.hlsl:1108-1110`
  - `crates/boyko_rhi_vulkan/shaders/vb_resolve.comp.hlsl:464-466`
  - `crates/boyko_rhi_vulkan/shaders/vb_shade.comp.hlsl:619-621`
  - `crates/boyko_rhi_vulkan/shaders/vb_shade_split.comp.hlsl:586-588`
- So on an ARMED frame (header bit 3 on: some light holds a slot and casters exist), a light
  without `CastsPunctualShadow`, or one that lost the slot budget, is read as slot 0. A spot
  samples atlas **layer 0**: rendered that frame, but another light's map. A point takes
  `gFaces[0]`, the slot-0 light's face record, and samples layers 0..5; any of those layers that
  no pass rendered holds memory nothing wrote.
- `crates/boyko_rhi_vulkan/shaders/light_table.hlsli:223-224` says the resolve reads the slot
  only under `punctual_shadow_mode != 0` AND `casts_shadow`, "so a 0 slot is never sampled". None
  of the six sites checks `casts_shadow`; the comment is false.

### Who is exposed

**No pinned scene.** This is static reading by two reviewers (the R1 fix design and its critique,
2026-09-18); the fix lane confirms it on the device, where the two froxel pins must not move.

- `vb_mesh_froxel` and `vb_mesh_tex_froxel` are NOT exposed. They do have 12 un-slotted rows among
  14 point/spot lights, with `ShadowConfig` enabled. But neither spawns a `ShadowCaster`: their
  meshes are plain `MeshBundle`s (`crates/boyko_app/tests/vb_mesh_froxel.rs:206`,
  `crates/boyko_app/tests/vb_mesh_tex_froxel.rs:225`). So `CsmCasterScratch::batch_count() == 0`,
  the punctual depth pass is not armed (`sync_punctual_light_gate` requires
  `casters.batch_count() > 0`), and header bit 3 never arms there.
- The one exposed scene is the unpinned `crates/boyko_app/examples/vb_lab.rs`. It has a flagged
  spot (`:288-306`), an un-flagged point (`:309-313`) and `ShadowCaster` meshes (`:223`, `:240`).
  The point's row decodes as slot 0, so it reads the spot's face record (`gFaces[0]`) and samples
  layers 0..5. Layer 0 is the spot's map; layers 1..5 are never rendered.

### The decided fix, and why it is its own lane

The R1 fix design took this as an architecture call. The fix is **host-side**: an un-slotted
punctual row is born `SLOT_NONE`.

- `GpuLight::from_point` / `from_spot` (`crates/boyko_render/src/light.rs`) and their golden
  mirrors `GoldenLight::point` / `spot` (`crates/boyko_rhi_vulkan/src/goldens.rs:1310`, `:1326`)
  build the kind word with `SLOT_NONE` already in the slot field.
- `slot_pack` (`crates/boyko_render/src/light_system.rs:451`) keeps its guard, and it only ever
  writes a real assignment over that field.
- The six existing checks then reject the row. No shader code changes, no `.spv` is re-emitted and
  no pin moves. `light_table.hlsli`'s false comment is corrected in the same change, comment-only.

The cost is intended. An un-slotted table stops being byte-identical to the pre-Inc-1 fold, so the
0%-gate's byte identity ends. In the slotted encoding the old word means "slot 0", so that identity
was the defect. The pixel 0%-gate still holds, and that is what the pins check.

The shader-side alternative was rejected: the six sites would also test bit 16. That edits six sites,
re-emits their `.spv` and adds ALU per light per pixel on armed frames. It also does not fix the
class:

- `GoldenLight::with_sdf_shadow()` (`goldens.rs:1371`) sets bit 16 without a slot.
- `light_table.hlsli:56` defines bit 16 as "this light casts an SDF shadow", not as "slotted".

The vkval lane's commits are split into a pixel-neutral one and a pixel-changing one, and this fix
belongs to neither. It lands in its own lane after this one, and this entry is closed `RESOLVED`
then.

The shadow gate's own record of the case is case 2 of `sync_punctual_light_gate`'s "What a
punctual sample can read" (`crates/boyko_render/src/shadow_atlas.rs`); its probe and its
device-free sweep count slotted rows by `CASTS_SHADOW_BIT` rather than by
`light_atlas_slot(kind) == SLOT_NONE` for exactly this reason.

---

## 2026-09-17 — Physics defects A7 and A7a: a resting box pyramid creeps sideways, and a quarter-overlap face contact repeats a feature id

Found while closing defect A4 (the sleep latch that survived the loss of support) and defect A5
(`physics_apply` writing into another body's row). **Neither is A4's or A5's to fix**, and both were
about to exist only in `#[ignore]` reason strings, so they are written down here before the lane
closes. This entry is the record a future session finds by grepping `docs/` for open physics
defects; the four tests below were red BY DESIGN until the A7 lane landed. A7a (2026-09-18) greened
A7-R0 and G8; A7b (S5, the next commit) greened A7-R1 and A7-R2, so all four are green. What stays
open is under "What is NOT decided" below — first of all a residual drift RATE, which is why no
document may call A7 fixed without that rate beside it.

### A7 — a resting box pyramid creeps sideways, and tall piles never come to rest

**What is measured** (msvc release, 2026-09-17; displacements and step counts, no timing):

- Jolt's height-15 pyramid (1240 boxes, layer gap 0, `PhysicsConfig::sleeping = false`) creeps with
  a fixed (−x, −z) bias. Largest horizontal displacement of any box between step 600 and step 3000,
  after the vertical settle is over: **D_max = 0.6361069 m**, against the test's bound of 0.01 m
  (`CREEP_BOUND_M`, two of Box2D's `B2_LINEAR_SLOP`). Sixty-three times the bound is not slop.
- Piles of height **7 or more never reach a 60-step quiet window**: the contact set keeps changing
  at rest, so the sleep debounce never completes. Heights 4, 5 and 6 do settle (6 only with the
  flicker budget, G8).
- **This clears A4 of it.** Those non-settling runs record `contact_wakes == 0`: no contact-change
  wake fires at all, so the wake-on-contact-change comparison A4 added is not what keeps them
  awake. A future session must NOT re-open A4 for the height ≥ 7 non-settling; that reading was
  taken and it excludes A4.

**Red-first tests** — all in `crates/boyko_physics/tests/sleep_settles_box_piles.rs`. A7-R1 and
A7-R2 carried `deferred:` ignores until S5; they now take the file's release-only `slow:` form and run
in the physics release run, `cargo test --release -p boyko-physics --no-fail-fast`, which `CLAUDE.md`
names as the leg of the debug-ignored `slow:` tests:

- `a_resting_jolt_pyramid_does_not_creep_with_sleeping_off` (A7-R1) — the creep bound above. Its
  window opens with a standing guard (`STANDING_DROP_M`) so a collapsed-and-resting pile cannot
  satisfy the bound by having nothing left to move.
  **Green since A7b.** D_max over steps 600-3000: 0.6361069 m at base, 0.0972966 m with A7a,
  0.0008583 m with A7b as first implemented (box 1240, the apex) and 0.0007180 m with A7b as
  committed (box 1227, layer 12).
- `a_jolt_scale_box_pyramid_freezes` (A7-R2) — the height-15 pile freezes and holds.
  **Green since A7b.** With A7a alone it did not freeze in 6000 steps (largest `|v|² + |ω|²`
  0.0069859); with A7b it froze at step 188 as first implemented (4.95 s) and at step 248 as
  committed.
- `a_height_6_box_pyramid_freezes` (G8) — the height-6 pile freezes and holds; red before A7a
  because onset flicker kept it awake.
  **Green since A7a (2026-09-18, msvc release).** Base `9f712204` did not freeze in 6000 steps;
  with A7a it froze at step 128, and with A7b at step 185. Its `deferred:` ignore became the file's
  release-only `slow:` form with A7a; with A7b it takes 2.4 s in debug, so it runs in both profiles
  and only Miri skips it.

### A7a — a quarter-overlap face contact repeats a feature id, so two warm-start keys collide

**What is measured:** of the four quarter-overlap face-contact offsets a box pile produces
(`(±1, ±1)` between two unit boxes overlapping by 1 mm), **3 of 4 repeat a feature id** inside one
manifold. The warm-start key is `pack(a, b, feature_id)`, so two points of the same manifold hash to
the same key and `WarmStartTable::insert` overwrites one with the other — the pile's layer-to-layer
contact loses a point's accumulated impulse every step. Clipped points inherit `min(prev, cur)`
corner ids, which is where the collision comes from.

**Red-first test:** `a_quarter_overlap_face_contact_has_distinct_feature_ids` (A7-R0), same file.
It is device-free and schedule-free, so it runs in either profile and under Miri.
**Green since A7a (2026-09-18).** A clipped point is now named by the two features that created it
(`narrowphase::feature_face_clip`). The test has no `#[ignore]` any more.

### A7b — a resting face pair takes the 1-point edge path on jitter

**What is measured** (the A7 lane's C2 probe, msvc release, 2026-09-18, on the A7a tree): on the
resting height-15 pile, 33.3 % of load-bearing support manifold-steps were on the edge-edge path —
one point instead of a clipped patch of four — and the supports changed point count 407 373 times
over steps 600-3000. The edge axes taken were exclusively `A.x × B.z` and `A.z × B.x`
(3 238 124 of 3 238 124): axes that nearly duplicate the face normal. Their SAT depth differs from
the face axis's by about the relative tilt times the lateral centre offset — ~1.4e-5..2.8e-5 m on a
quarter overlap at rest — which beat the old `SAT_EPS` = 1e-5 m face preference on jitter, and the
5 % hysteresis then held whichever axis won.

**The fix (S5, `narrowphase/box_box.rs`):** Box3D's rule as its live `convex_manifold.c` ships it,
with two differences that predate S5: Box3D treats a clip left with fewer than 3 vertices as no face
contact, where here only an empty clip counts, and Box3D keeps speculative points, where here only
points with `separation <= 0` are kept. The face contact is built, and the edge replaces it only if
the edge is shallower than the face's REALIZED clipped patch by more than `FACE_AXIS_PREFERENCE` =
0.005 m, or if the face realizes no patch; an edge hint never holds a face; a held face hint that
realizes no patch yields to the best face's patch when that was already built (Box3D re-runs its full
query when a cached feature fails); any other chosen face whose clip is empty or has no penetrating
point falls back to the edge contact. As a result a box pair's manifold exists exactly when the two
poses overlap, whatever the axis hint, except on a reference face with a zero in-plane extent, which
retires A4's Known behaviour 2 for box pairs. Gated by A7-N9, A7-N10, A7-N11 and
`a_degenerate_reference_face_is_no_contact` in that file. Its rung was pre-registered in the
pile-test header before the first run and read 0.0008583 m on the form first implemented and
0.0007180 m on the form committed, both in the "confirms" band. On that pile the 5 mm comparison chose
the edge 0 times in 3 531 050 SAT edge answers over steps 600-3000; every one of the 264 edges taken
was a face that realized no patch, so the constant's value does nothing there.

### What is NOT decided

1. **Answered: A7 was two defects, A7a and A7b.** Measured on A7-R1's D_max: 0.6361069 m →
   0.0972966 m with A7a (6.54×) → 0.0007180 m with A7b as committed (135× more; 0.0008583 m, 113×,
   as first implemented). Only the residue below is left.
2. **Answered: the feature-id repair belongs in the clipper** (A7a names a clipped point by the two
   features that created it; the key packing is unchanged).
3. **A4's Decision 3 (the onset-flicker comparison) is re-decided only AFTER A7**, per the A4
   round-2 ruling §3.4. A7 has landed and it has NOT been re-decided. Until it is, a red G2 or G7 is
   triaged by the four steps in that file's module header and in its failure message — never waived,
   and never silenced by raising a budget.
4. **Done with A7b: the flicker budgets were re-sized from a generator run on the A7b tree** (msvc
   release, 2026-09-18; all 30 height-4 and all 56 height-5 draws froze, the latest at steps 127 and
   243). The pooled wake probability fell from 48/78 to 2/32 at height 4 and from 160/216 to 20/76 at
   height 5, and by the file's rule — the smallest budget whose tail is below 1 % at the measured p
   plus one standard error — `G2_MAX_EVENTS` went 18 → 3 and `G7_MAX_EVENTS` 56 → 6;
   `MEDIUM_SETTLE_LIMIT` went 20 000 → 486 (twice the latest height-5 freeze) and G7's per-draw limit
   6000 → 1016 (eight times the latest height-4 freeze). Every one went DOWN. One height-5 draw took 4
   events, one more than G2's new budget: the sample puts that tail at 1 in 56 where the pooled
   geometric model puts it at 0.48 %.
5. **No price is attached to any of this.** The A4 bench exists (`§8` of
   [MEASUREMENT-QUEUE.md](MEASUREMENT-QUEUE.md)) and has never been run. S5's own price is queued as
   `§9`: on identical poses of a resting height-15 pile the rule alone adds 17.1 % contact points,
   every step, in a default world (sleeping off). The +57 % at step 600 compares two different
   trajectories and is not the rule's price.
6. **The residue is a drift RATE, and it is unexplained.** One run of A7-R1's scene extended to 5400
   steps (msvc release, 2026-09-18, the kernel as committed) read D_max = 0.7180 mm over steps
   600-3000 and 1.0965 mm over 3000-5400, on the same box (1227, layer 12) in both windows, with layers
   10-14 moving toward (−x, −z) on average in both; 1.8138 mm over 600-5400. (The form first
   implemented read 0.8583 mm, 0.9262 mm and 1.7841 mm, on box 1240.) The windows add: ~0.38 mm per
   1000 steps, ~2.3e-5 m/s, ~8 cm per hour of simulated time with sleeping off, the default. It is
   14× under A7-R1's bound over A7-R1's window. It is not A7b: over steps 600-3000, 0 of 9 743 983
   support manifold-steps were on the edge path (1 of 9 743 988 on the first form; 33.3 % with A7a
   alone). **Test that decides the next step:** mirror the scene in x. If the drift mirrors too, a
   world-anchored tie-break — a `>= 0.0` sign test such as `support_edge_point`'s, or a lowest-index
   rule — is steering it.
7. **T-h, the substep dependence: not measured.** On the A7a tree without S5, the support edge-path
   share at substeps 4, 8, 16 and 32; on S5, D_max at substeps 4 and 8. If the edge-path flips
   explain the pre-fix substep curve, the share falls monotonically with substeps and D_max(8) is not
   2× or more below D_max(4).
8. **Where a chosen face realizes no patch and no best-face patch was built, S5 builds the edge
   contact** (the cold fallback). Before S5 a chosen face that realized nothing gave no manifold, but
   most of these pairs still had one then, because the pre-S5 rule held an edge hint that S5's class
   clause refuses. On the resting height-15 pile the fallback ran 12 572 times over steps 600-3000
   (13 615 on the first form; 1133 at height 7): on 10 446 of those calls the pre-S5 rule, given the
   same poses and hint, built an edge contact too, and 2126 — under one a step — are manifolds S5
   adds. All are no-load same-layer knife-edge pairs, face and diagonal neighbours; 3588 of their
   points lie at a separation above 0 (at most 1.96e-6 m). 12 567 were pairs whose own best face
   realized no patch (A7-N11's family H); 5 were a held face with no built patch to yield to, where
   Box3D would build the best face's patch instead — the one remaining difference on the held-hint
   path. The C2 probe's form had no fallback, so this is where the shipped form differs from the
   measured one. On generic poses a face hint held by the hysteresis can clip to nothing where the
   pre-S5 rule had an edge contact (13 cells in a random census of 154 423 overlapping poses × 16
   hints; A7-N11's family G); there S5 now yields to the best face's already-built patch, as Box3D
   does, and the yield ran once on the pile, before step 600.
9. **The clip id does not encode the incident face** (A7a review, open question 1). A cross-step
   alias needs the incident face to change while the reference face is held, which takes a relative
   rotation near 45°; a resting contact never makes one. Promotion trigger: a census of manifold-steps
   that keep `ref_face` but change the incident face, over A7-R2 and G1-G8, reading > 0. The fix would
   not change the 4-bit field: map ring edge k to box edge 0..11 through a 24-byte table, with
   plane-created edges at 12..15.
10. **The reference-axis hysteresis is relative** — 5 % of a resting depth near 1e-4 m is ~5e-6 m —
    while Box3D keeps its cached feature within an absolute `linearSlop`. The live manifold count
    still changes on 1739 of 2400 steps of the resting pile after S5 (0.725; 0.759 on the first
    form; the SAT-form probe read 0.7338).
11. **Parked, with their numbers:** S2 (the full 2×2 tangent block in the friction solve) — Step 0
    read D = 0.6183357 m, 2.79 %, inside the ~10 % chaos band that gravity perturbations of 1e-6
    measure on this scene. S3 — deferred; its triggers were defined on S4 (a speculative margin),
    which the C2 probe withdrew.

---

## 2026-09-09 — Stage 4 reaches an arm the tree says not to touch, and a `ScratchBuildView` that cannot express what the buffer does

Six cohorts have moved this session (`vn_initial` + both `TouchedMask`s, the four warm-start tables,
the narrowphase output, the serial solvers' constraint buffers). The next buffer in the census,
`ContactPairs::pairs`, raises two things that are worth writing down before they are decided
silently.

### 1. The all-pairs broadphase arm carries an explicit DO-NOT

`systems.rs:302-303` reads, verbatim:

> The shipped all-pairs loop, kept VERBATIM so the default path's asm is byte-identical to before O2
> (the 0%-gate). DO NOT refactor this arm.

Migrating `pairs` off `std::Vec` changes that arm's codegen — that is what the migration IS, and no
container swap can leave the asm identical. So the note either applies and Stage 4 stops one buffer
short of finishing, or it was written to fence off *incidental* churn during the O2 grid work and
does not bind the deliberate storage change.

The reading taken here is the second one, on two grounds: the note's stated purpose is protecting the
default path from an unrelated change, and the same migration on `BroadphaseGrid`'s thirteen buffers
measured **faster**, not slower (18.27 vs 19.20 ms on one worker, 20.45 vs 21.60 on eight, 25.07 vs
26.99 on sixteen, on the Jolt-parity pyramid). The arm's SHAPE is kept exactly — only `pairs.clear()`
and `pairs.push(..)` change receiver; the loop body, the bound test and the emit order are untouched.

**What is missing is a measurement of THIS arm**, and taking one means running the broadphase
benchmark on the owner's workstation, which is a timings request rather than a call to make alone.
Until it is taken the claim is an inference from a neighbouring buffer, and it is labelled as one.

✅ **MEASURED 2026-09-09 in the owner's quiet window, and the answer is BOTH.** Release/`bench`
profile, medians, each arm in its own worktree, runs taken back to back.

### The arm IS covered end-to-end, and there it costs nothing

The first thing the measurement found was that no new bench was needed:
**`BroadphaseKind::AllPairs` is the DEFAULT** (`resources.rs`, `#[default]`), and
`jolt_parity_pyramid` never sets `PhysicsConfig::broadphase` — so that bench has been running the
DO-NOT arm through the real system all along, at 1240 bodies, on every worker row.

Full physics step, base `e665ccd6` (this session's start) → HEAD `627ca762` — the whole of Stage 4,
seven cohorts and the kernel addition:

| W | base | HEAD | Δ |
|---|---|---|---|
| 1 | 18.671 ms | 18.681 ms | +0.05 % |
| 2 | 19.240 ms | 18.992 ms | −1.29 % |
| 4 | 19.394 ms | 19.390 ms | −0.02 % |
| 8 | 21.017 ms | 20.970 ms | −0.22 % |
| 16 | 26.073 ms | 25.895 ms | −0.68 % |

Mixed in direction and all of it under the instrument's own resolution: criterion's comparison
against a stored baseline from an earlier session drifts **+2…4 %** on the same rows, so this
machine's session-to-session noise is larger than every number in the table. The honest reading is
**no cost detected, and none below ~2–4 % could have been.**

### The LOOP, in isolation, is 7–17 % slower — the DO-NOT was pointing at something real

The `broadphase` bench's `all_pairs` row, container-A/B on the identical loop (base `&mut Vec`,
HEAD `ContactPairs` refill view):

| n | base | HEAD | Δ |
|---|---|---|---|
| 100 | 5.596 µs | 6.545 µs | **+17.0 %** |
| 1 000 | 600.3 µs | 637.4 µs | **+6.2 %** |
| 10 000 | 77.42 ms | 85.17 ms | **+10.0 %** |

A repeat run at HEAD landed within 0.7 %, so this is far outside noise. The two results reconcile
arithmetically rather than contradicting: at 1240 bodies the all-pairs pass is ≈0.9 ms of an 18.7 ms
step, and +7 % of that is +0.06 ms — **+0.35 % end-to-end**, under the noise floor of the first
table. Both are true; only the second one can see it.

### The obvious explanation is WRONG, and it was tested rather than assumed

Hypothesis: `ComponentPool::row_ptr` scales the index by the pool's RUNTIME
`component_layout.size()`, where `Vec` scales by a compile-time constant — a field load plus a
multiply per push. Replacing it with typed `size_of::<T>()` arithmetic measured
**6.645 µs / 648.9 µs / 83.31 ms**: no consistent direction, inside ±2 %. It was reverted rather than
kept, because a neutral change that adds an unsafe assumption (stride ≡ `size_of::<T>()`, guarded
only by a `debug_assert`) is a cost with no benefit.

So the cost is NOT the address arithmetic. The remaining candidate is the per-push reload: `push`
reaches the frontier through `&mut ScratchBuildView` → `&mut ScratchColumn` → `&mut ComponentPool`,
and the write through the resulting `*mut u8` leaves the compiler unable to keep `len` /
`committed_rows` / `buffer` in registers across the loop, where `Vec`'s three fields stay live.
**The fix that follows from that diagnosis is a `ScratchBuildView` that CACHES `(base, len,
committed)` and writes the length back on `Drop`** — push becomes a compare, a store and an
increment on registers, with a `#[cold]` slow path for the grow.

**Question: fund the cached-frontier `ScratchBuildView` now, or leave the 7–17 % on the all-pairs
loop?** The case for leaving it: the loop is O(n²), it is the arm the engine's own density policy
exists to move OFF, and at any body count where the absolute cost matters the Grid arm is already
selected — and that arm measured FASTER after its own migration. The case against: `AllPairs` is the
DEFAULT, so this is the path every world that never opts in takes, and the regression was introduced
by this lane.

✅ **FUNDED AND SHIPPED the same day** (`0a803cfc`), and the numbers say the fix works while the
step-level claim does NOT.

The view now caches `(base, len, committed)` and publishes the length on `Drop`. The loop reads
**5.619 µs / 600.2 µs / 74.78 ms** — the regression is gone, and at n = 10 000 the column is **3.4 %
FASTER than `Vec`**. End-to-end it is invisible: the pyramid reads 18.551 / 19.348 / 19.397 / 21.218
/ 26.091 ms against 18.681 / 18.992 / 19.390 / 20.970 / 25.895 before, mixed in direction and inside
the same +2…4 % drift. That is exactly what the arithmetic predicted, and it is why the case for
shipping is "the lane introduced the regression and the loop is now better than what it replaced",
not "the step got faster".

Validated under Miri with Tree Borrows (12 tests). ⚠ **The commit message for `0a803cfc` says the
recorded note about Miri on this machine "was WRONG". That sentence is itself wrong**: the note
`reference-miri-nightly-resolves-to-msvc` names `+nightly-x86_64-pc-windows-gnu` as the working form
and only reports the bare `+nightly` invocation failing. What was wrong was reading the compressed
index line for the note instead of the note. Recorded here because the commit is pushed and its
message cannot be amended without a force-push.

### 2. `ScratchBuildView` cannot express a pre-sized, parallel-filled, compacted buffer

`BroadphaseGrid::build_parallel` (`resources.rs:1782-1853`) does `out.clear()` → `out.resize(m +
reserve, default)` → parallel fill by index → `out.truncate(w)` → `sort_unstable`. The refill view
has `clear` / `push` / `extend_from_slice` / `as_mut_slice` and nothing else, so `resize` becomes a
push loop (fine, but it is a store plus a grow check per element where a `Vec` did one length write
and a fill) and `truncate` **cannot be written at all** — there is no way to shrink the live length
short of clearing and re-pushing every survivor.

This is not only this caller's problem. The same push loop was hand-written three times already in
this session — `TouchedMask::reset`, `WarmStartTable::rebuild` and `::with_capacity`,
`BoxAxisCache::begin_frame` — each time because `resize` did not exist.

Principle 0 says a capability a subsystem needs becomes a first-class kernel feature rather than a
per-crate adapter, so the intended answer is `ScratchBuildView::resize(len, value)` and
`::truncate(len)`, backed by a `ComponentPool` no-drop count set (sound for exactly the reason
`clear_no_drop` is: `T: Copy` is asserted at `ScratchColumn::new`). That is a `boyko_ecs` change made
from a physics lane, which is why it is recorded here rather than just done — it is additive, but it
is the kernel.

**Question: take the kernel addition from this lane, or leave `ContactPairs` as the one unmigrated
rigid buffer until the kernel is touched for another reason?**

✅ **RESOLVED 2026-09-09, same day, and the question as posed was WRONG.** It framed the addition as
a choice about one buffer. Counting the remaining Stage 4 census afterwards: `soft/coupling.rs` uses
`resize` twice and `soft/colored.rs` six times, on top of `build_parallel`'s. **Every single
remaining buffer in this lane's scope needs it** — so the real alternative was not "leave one buffer"
but "stop Stage 4 here", which is not a trade anyone would take. Recording that because the framing
survived into a commit message before the census corrected it, and a question that misstates its own
stakes is worse than no question.

Taken as `ScratchBuildView::resize` / `::truncate` over a `ComponentPool` no-drop count set, with the
`extend_fill_copy` grow hoisted out of the loop. Verified by red mutation: removing the fill turns
three of the five new kernel tests red, including the one written for the failure mode that would
otherwise pass every length assertion while handing back last frame's bytes.

## 2026-09-03 — the Gaia register catches up with the owner: TWELVE of the fourteen ballots were ruled on 2026-08-30/31, on a branch this one never received

**Why this entry exists.** The owner remembers deciding the Gaia ballots, and he is right. They were
put to him on 2026-08-30 and he answered twelve of them the same day; four more were settled by the
standing perf/architecture rule. **The rulings landed on `feat/threadpool-ke16` and this branch
never received them**, so until this entry the register the repository's own rules name as primary
still listed all fourteen as OPEN — and printed one of them **backwards**.

**Established from git history, not from reading order.** `97c504c8` (2026-08-29), the head of
`feat/multi-paradigm-render` when this sync was written, **is the merge-base** with
`feat/threadpool-ke16` and is an ancestor of it: that branch is **28 commits ahead** and carries
nothing this one does not. The ruling commits are `6f75ee9e` (2026-08-30, *the owner's rulings, and
the Aether ladder closes*), `c1a9b8ec` (2026-08-31, *the delegated rulings*), `b7a9931d`
(2026-08-31, four owner directions recorded for a later research pass) and `b6c41237` (2026-08-31,
*GB-5 ruled*). **None of the four is reachable from this branch** (`git merge-base --is-ancestor`
returns false for each). Nothing here overrules the working branch; where the two disagree, KE16 is
newer by ancestry.

> ⚠ **2026-09-10 — the merge landed, and the two paragraphs above are now HISTORY, not status.**
> `merge/ke16-into-render` merges `feat/threadpool-ke16` (`02325b01`) into
> `feat/multi-paradigm-render` (`0c18acfc`). The four ruling commits this entry was written to
> report — `6f75ee9e`, `c1a9b8ec`, `b7a9931d` and `b6c41237` — **are reachable from this branch
> now, and their full bodies are in this file below and in**
> [`gaia/DECISIONS.md`](gaia/DECISIONS.md). The entry is kept exactly as written because it is the
> record of WHY the sync was needed: read *“this branch never received them”* and *“none of the
> four is reachable”* as statements about 2026-09-03, not about today.
>
> **Added 2026-09-10, while widening the anchors gate to this file:** the `docs/ru/` freeze's own receipt
> — *"both sides were last written by `efd7735f` (2026-09-03), verified by `git log -1` per path"* — no
> longer reproduces on this branch: `git log -1 -- docs/ru/OPEN-QUESTIONS.md` answers `d2c8c646`, the KE16
> merge, which rewrote that file by +1421/−50 as a conflict resolution (union of both sides' Russian
> entries, nothing translated, nothing added). Checked per parent, the last REAL writers are `efd7735f`
> (2026-09-03) and `778739f0` (2026-09-02), both before the freeze, so no Russian text was authored after
> it — the claim survives, the anchor does not, and divergence is measurable from `d2c8c646` forward.

⚠ **The single most dangerous cell, now repaired.** [`gaia/CAMPAIGN.md`](gaia/CAMPAIGN.md) and this
file printed F5's recommendation as **(a)** — *the option the owner rejected*. He chose **(b)**,
"Все сразу грамотно по списку с самого начала". A reader resuming from this branch would have built
the wrong half of the streaming design.

⚠ **And the column that made a recommendation readable as a decision.** The ballot table in
`gaia/CAMPAIGN.md` was headed **Recommendation** — the plan's own recommendation.
`git log -G"Recommendation" -- docs/gaia/CAMPAIGN.md` returns exactly two commits: `a4591c56`, whose
message says the ballots are **held**, and `c1a9b8ec`, which renamed the column on the working
branch. F1's cell there read "(b) GK-4 now" and happens to match what the owner later ruled — a cell
**wrong in status and right in content**, which is the shape nobody re-checks. The column is now
headed **Disposition (2026-08-30)**.

### What was ruled, and by whom

`RULED BY THE OWNER` and `[delegated]` are different things and are kept apart. The full ground of
each ruling lives on `feat/threadpool-ke16` at the site named. Where that site does not exist on
this branch the branch is named rather than linked, because a link that dangles here is the defect
this entry repairs.

| ballot | who | ruling |
|---|---|---|
| **F1** | owner | **Macro-time GK-4, NOW** — not sequenced behind EG2. *"Do it properly right away. But bear in mind the world must support streaming."* Both halves bind: no part of the bake design may foreclose streaming. ⚠ The ballot's own `RequiredCtor`-is-unbakeable inference is **refuted** — a GK-4 baker is a Rust program linked against the derive tables and calls the fn pointer trivially; only the *evaluator over text* cannot. Ground: KE16 `gaia/DECISIONS.md` §Inherited pipeline |
| **F2** | delegated | **(a) rows are ENTITIES — amended three ways**: storage is `StorageKind::Table`, **not dense**; rows materialize **eagerly at load**; a table is loaded by its own explicit load and **pinned**, so `unload_cell` never touches it. The runtime handle is an `Entity` captured at load, never a row index. ⚠ **Confirmed on CORRECTED grounds — see the F2 section below.** Ground: KE16 `gaia/DECISIONS.md` §Data tables |
| **F3** | owner | **Both file shapes over ONE schema.** A table file is a **spelling**, not a second type system. Rejected: a table dialect. Ground: KE16 `gaia/DECISIONS.md` §Data tables |
| **F4** | delegated | **(a), as SUPPRESS-THEN-FIXUP** — four ordered sub-passes on the clone path's own precedent. (b)'s *mechanism* is adopted as sub-pass 4; its *timing* is refuted at a measured 100 % mis-link rate (saved/fresh id overlap 8/8). (c) is worse than "untenable": it forbids every mesh, material, light, camera and particle effect. Carries the stable-asset-carrier addendum — a stable NAME resolved to `MeshHandle(u32)` at load, since `MeshRef` does not exist. ⚠ **Three amendments and a moved subject — see the F4 section below.** Ground: KE16 `gaia/DECISIONS.md` §Load semantics |
| **F5** | owner | **Option (b), EVERYTHING FROM THE START.** *"Все сразу грамотно по списку с самого начала."* `load_cell`/`unload_cell`, GK-1's cross-load map with a declared lifetime, and cross-cell reference resolution ship **with** the scene profile. ⚠ The largest change to the campaign's ground: **G6 absorbs the hardest half**, because a per-load `LoadEntityMap` cannot express references BETWEEN chunks. **The register printed (a) — the rejected option — until this entry.** Ground: KE16 `gaia/DECISIONS.md` §Streaming scope |
| **F6** | owner | **(a) migrate `.ui` to the Gaia `ui` profile and DELETE the old format in the same campaign.** Price accepted and recorded at G7: the migration is done **blind**, because G7's prerequisites do not exist yet. Ground: KE16 `gaia/DECISIONS.md` §Language shape |
| **F7** | owner | **RATIFIED — mods are NOT supported for now**, stated explicitly so that silence stops being a default decision. The reflection constraint is recorded in its **narrow** form (no reflection in the **game binary**), so a later mod campaign is not blocked by a sentence that never meant to block it. Ground: KE16 `gaia/DECISIONS.md` §Refusals |
| **F8** | — | ⚠ **UNANSWERED — still open**, and deliberately untouched by every ruling of 2026-08-30. GB-3's Part 4 and F4's carrier addendum are both **F8-neutral by construction** and say so at their own sites: a *carrier form* (a stable name used as a key) is not a *name-vs-id ruling* |
| **F9** | delegated, **partly** | ⚠ **Three eliminations RULED, the residual escalated back.** (1) `FLAGS_DIRECT` must **not** fire on the load path — it would overwrite every saved bit with its attach-time value; (2) the refusal option **may not be chosen**, because it contradicts ratified **AB-6** (*a language may not refuse what the derive accepts*); (3) if a carrier lands, its spelling is `flags (X = true)`, shared verbatim with Aether, keyed on GK-1's global object id and riding F4's fixup pass. ⚠ **STILL THE OWNER'S:** may a document set a flag on an **individual authored object**? Blocks G2 and nothing else. Ground: KE16 `gaia/DECISIONS.md` §Flags in the byte format |
| **F10** | — | ⚠ **UNANSWERED — still open**, and deliberately untouched. Every ruling that could have brushed it states its F10-neutrality at its own site rather than settling it by proximity |
| **GB-1** | standing rule | **RESOLVED — and the owner resolved it by sending it back**: it is an architecture fork, not a values call. Neither a template EXPANSION nor an UNLABELED write gets a ladder layer, because the ladder is the wrong axis: expansion is one step on the inheritance-**DEPTH** axis, ranked where `extends` ranks, resolution depth-then-ladder; an unlabeled write is `base`. The ratified ladder is untouched and finding K2 is **dissolved**, not diagnosed. Ground: KE16 `gaia/DECISIONS.md` §Layers and depth |
| **GB-2** | owner | **`#RRGGBB` is `#RRGGBBAA` with maximal `AA`** — a defaulted field, not a second literal kind, which dissolves the reopen rather than answering it. Plus the arity direction the clause never covered: an 8-digit literal at a 3-component field (`PointLight::color` is `[f32; 3]`, [`light.rs:309-310`](../crates/boyko_render/src/light.rs)) is a **coded bake refusal naming the field**, never a silent alpha drop. Ground: KE16 `gaia/DECISIONS.md` §Language shape |
| **GB-3** | owner (adoption) + delegated (the rewrite) | **The THIRD reference kind is ADOPTED**, and the four-part rewrite is ruled: the spelling is a **sigil**, uniform and position-independent, so *a bare word is never a reference*; style references are the **first** kind; `$hole` **stays**, reclassified lexical in every position; *"declared node ⇒ `@`"* is **withdrawn**. ⚠ **The glyph is NOT minted** — it is a **G2 deliverable, closed jointly with Gaia's arithmetic operator vocabulary**, because R3 makes whitespace insignificant and a glyph that can open a binary operator is ambiguous (recommendation `~`). Ground: KE16 `gaia/DECISIONS.md` §Identity and references |
| **GB-4** | delegated | **(a) linkage-in-slot, and the linkage word is MANDATORY**: `instance <name> extends\|copy <base-ref>`, no default. `from` is deleted — measured over code fences only, 1 `from` hit in 44 fenced lines and it is `from=` as a field key on `bind`, so head-position `from` counts **zero**. Price, stated: one mandatory word per instance line. ⚠ Rider owed to G2: the corpus carries an **undeclared `from=` → `source=` rename on `bind`**. Ground: KE16 `gaia/DECISIONS.md` §The instance spelling |
| **GB-5** | owner | **PERMIT AS SEED** — ruled 2026-08-30, recorded 2026-08-31. ⚠ **See the GB-5 section below: this is the one answer the register lost outright, and four registers on the working branch itself still say it is open** |
| **GB-6** | owner | ✅ **DISPOSED 2026-09-03, both halves — it is no longer an open ballot.** (a) The scene-form narrowing is **RULED**: the Gaia TEXT is the source the editor saves, the BINARY is the compiled artifact that ships, and the alternative the ballot never named — shipping a scene as TEXT and parsing it at runtime, Godot's `.tscn` shape — is declined. (b) The conditional-visibility valve is **NOT a Gaia ballot**: *«gaia это не логика а данные»*, so it MOVES to **Aether R3** and is struck from this series. See the ruling section below |
| **GB-7** | standing rule | **RESOLVED — no wall-clock companion; G7's still-frame COUNT gate stands alone.** Measured, not asserted: the timed loop's inputs are archetype count, bound-**type** count and row count and **not** the binding count (`dynamic_bound_ids` is a deduplicated type set, [`bind_system.rs:56-60`](../crates/boyko_ui/src/binding/bind_system.rs)), so the still frame is **332 ns at +0 % for 10× the bindings** but **+82 % for 2× the rows** and **+101 % for 2× the bound types**. `Instant::now()`'s step is 100 ns, so the frame is ~3.3 ticks with a 15-25× single-call tail, and the red-first delta (0 → 1 sink write) sits at signal-to-noise **0.05-0.50** — which the count gate resolves exactly and no clock does. A clock over the **scan itself**, world pinned, belongs to GK-2's design pass |
| **GB-8** | delegated | **Three parts.** (1) **No per-site waivers**, at any of the four censuses, ever; where a property is not decidable as written, narrow the **predicate** and print the narrowing in the failure message. (2) The AIR census **widens to all of `docs/` and lands at G0** — a one-constant change, green at zero remediation. (3) The **LINK census is a separate deliverable at Aether R8** (1636 relative targets under `docs/`, **59 dead across 11 files, 44 of them in `docs/AUDIT-2026-05-23.md`**); landing it with the id half would hold the free half hostage to 59 repairs. ⚠ The census file itself, `tests/gaia_g0_citation_census.rs`, exists on `feat/threadpool-ke16` and **not on this branch**, and the G0 widening edit has landed on neither |
| **GB-9** | standing rule | **RESOLVED — option (b): the `Or<(Changed<A>, Changed<B>)>`-over-dense emission ban is DELETED, with a record.** Its original ground was fixed and gated by Aether R0 / KE1; the fallback ground (D4) was measured and rejected — **D4 reserves the Aether *surface* `or(...)` while the ban governed *generated code*, and ratified GN2 says the baker emits no Rust**, so the ban had no subject. It could not be re-grounded on KE13 either: `Or` folds `NEEDS_CHANGE_DETECTION` and `EcsMaster::query` const-refuses it, so the banned shape cannot reach a `QueryView` at all. ⚠ **Its deadline expired unanswered** — R0 landed with GB-9 open |

**The Aether `AB` series closed on the same day and on the same branch.** Every `AB` ballot except
**AB-12** (which blocks nothing) is ruled — **six by the owner** (AB-1, AB-6, AB-7, AB-10, AB-11,
AB-13) and **six delegated** (AB-2, AB-3, AB-4, AB-5, AB-8, AB-9); index ported verbatim, citations
verified against this branch, in §*2026-09-10* below. ⚠ Until 2026-09-10 this sentence read *"seven
by the owner, eleven under the standing rule"* — the source section's SESSION count (its seven owner
answers include the Gaia ballot GB-2; only six AB ballots are delegated), copied here as if it were
an AB count: 7 + 11 + 1 ≠ 13. The four with direct Gaia consequences are marked at their bodies in
the 2026-08-29 entry below (**AB-6**, **AB-10**, **AB-11**, **AB-13**). The 2026-09-10 pass ported
the index and verified each cited site on this branch; it did not re-check the Aether grounds
themselves.

### 2026-09-10 — the Aether AB index, ported verbatim from `feat/threadpool-ke16` and checked against this branch

> *[A5 batch merge, 2026-09-22.* This sub-entry was written on `feat/multi-paradigm-render` at
> 2026-09-10, a branch that lacked ke16's 2026-08-29 ballot bodies and `docs/aether-v2/`. The line it
> now sits on descends from `feat/threadpool-ke16` (`5550a3da` is an ancestor of the merge), so every
> site this entry names as "not on this branch" / "no … here" IS present here, at the ke16 line
> numbers it cites "there" (`aether-v2/DECISIONS.md` **E4** at `:482`, **C5a** at `:165`, the
> §Sequencing rulings heading at `:722` — verified by grep at the merge), and the bodies Table B quotes
> stand in full in the 2026-08-29 entry below. Read its "this branch" clauses as the 2026-09-10 record
> of the render branch, not of this tree. Its ke16 `docs/OPEN-QUESTIONS.md` spans are written
> `lines N-M` rather than `:N-M` here, and its `(:N there)` cells `(line N there)`, so the anchors gate does not bind them to the nearest file a row
> cites; nothing else in the entry was changed. What the entry adds on this line is Table A's `who`
> column, which is what lets the ruled-vs-open census read AB-1, AB-6, AB-7, AB-10, AB-11 and AB-13
> as ruled — the ke16 table at §*2026-08-30* has no such column.*]*

**Source.** `feat/threadpool-ke16`, `docs/OPEN-QUESTIONS.md` §*2026-08-30 — the owner's rulings, and
the Aether ladder closes* (lines 201-252 of that worktree, read 2026-09-10). **Table A** is that
section's table **verbatim**, with one added `who` column: the source has no `who` column; its seven
rows ARE the seven owner answers its own preamble counts (ke16 lines 203); the AB-7 and AB-11 cells
additionally say so themselves (*Owner: "…"*, *"what made it the owner's"*), and AB-10's attribution
is also carried by this file's own 2026-09-03 annotation of its body (*"on the owner's own ground"*).
**Table B** indexes the six delegated `AB` rulings, which have
NO row on ke16 — their rulings live only in that branch's 2026-08-29 bodies — quoting each body's
opening ruling sentences verbatim with `[…]` marking every elision (the ke16 line span is named per
row). Every *where the ground lives* cell was opened on THIS branch; where a site does not exist
here, the branch is named instead of linked — the 2026-09-03 rule above, now applied to the Aether
half it had left unapplied. The ke16 ground cells that were replaced by a branch name read:
AB-1: `[aether-v2/DECISIONS.md](aether-v2/DECISIONS.md) C3`; AB-6: `[aether-v2/KERNEL-BACKLOG.md](aether-v2/KERNEL-BACKLOG.md) AB-6 + **KE14**`; AB-7: `[aether-v2/KERNEL-BACKLOG.md](aether-v2/KERNEL-BACKLOG.md) **KE15**`; AB-10: `[aether-v2/AI-ORIENTATION.md](aether-v2/AI-ORIENTATION.md) AIR-10 / AB-10`; AB-11: `[aether-v2/CAMPAIGN.md](aether-v2/CAMPAIGN.md) AB-11 row; red test ab11_flag_filter_polarity.rs`; AB-13: `[aether-v2/CAMPAIGN.md](aether-v2/CAMPAIGN.md) AB-13 row`; GB-2: `[gaia/DECISIONS.md](gaia/DECISIONS.md) §Colour`.

*The source section's own preamble, verbatim (ke16 lines 203-208) — a SESSION count, whose "seven" includes
the Gaia ballot GB-2, see the correction above:*

> Seven ballots answered by the owner in one session, plus eleven decided by the orchestrator under
> the standing rule that performance and architecture forks are settled with numbers. **Every `AB`
> ballot is now closed except AB-12**, which blocks nothing.
>
> Recorded here as the single index; each ruling's full ground and rejected alternative live at the
> site named in its row.

**Table A — the ke16 index, verbatim, plus `who`.**

| ballot | who | ruling | where the ground lives |
|---|---|---|---|
| **AB-1** | owner | **RATIFIED as specified** — event auto-registration adopted on ergonomic grounds; the STAGE-under-D4 alternative rejected. ⚠ The reopen was licensed because C3's recorded ground — *"the unregistered case fails silently on both ends"* — was **refuted by measurement**: both generated ends are a loud init-time panic. The grant now stands on a stated ground (ergonomics), not on the refuted one | [`aether-v2/DECISIONS.md`](aether-v2/DECISIONS.md) C3 |
| **AB-6** | owner | **Option (b), and the class split in two.** The ballot's premise was refuted first: hand-written Rust could not do it either, so there was no ratified surface to narrow — only a promise the kernel did not keep. **Dense**: build the construct-and-commit route. **Bitset/flag**: refuse in the **derive**, since a flag has no bytes and `FLAGS_DIRECT` is the mechanism. Refusing in Aether alone was ruled out because it would make Aether reject what the derive accepts | [`aether-v2/KERNEL-BACKLOG.md`](aether-v2/KERNEL-BACKLOG.md) AB-6 row; **KE14** exists on `feat/threadpool-ke16` only — this branch's backlog ends at KE12 |
| **AB-7** | owner | **R-DENSE LIFTED, and the ballot's framing rejected.** Owner: *"that there is no parallelism is just wrong."* The candidate driver-independent ground had already been measured and **refuted on both conjuncts**; the owner then declined to treat the driver limitation as a constraint at all. Measured: the refusal is a `const assert` whose own comment says the chunk runner *"has no world cell"* — unwired plumbing, not a design limit, and the same gap blocks `Related` joins | **KE15** — a `feat/threadpool-ke16` `aether-v2/KERNEL-BACKLOG.md` row; not on this branch (backlog ends at KE12) |
| **AB-10** | owner | **(a) the measurement script IS required** when the audit branch is taken. **(b) the widening to the joint Aether+Gaia vocabulary is RATIFIED** — if the two languages are one body of work, the vocabulary is one vocabulary, and a word meaning different things across them is a false friend between the author's own languages. Price accepted: the risk list grows with Gaia's vocabulary, and a collision there may force a rename in Aether | [`aether-v2/AI-ORIENTATION.md`](aether-v2/AI-ORIENTATION.md) AIR-10 / AB-10 |
| **AB-11** | owner | **The recommended option — a parse refusal** with a did-you-mean pointing at `enabled` / `disabled`. ⚠ Adds a refusal where v1 documents non-refusal, which is what made it the owner's. Ground, measured with R0's fix already in the tree: over a `flag`, `with F` matches nothing and `without F` excludes nothing — two different silent wrong answers — and the defect is in the **leaves**, so R0's `Or` fix does not reach it | [`aether-v2/CAMPAIGN.md`](aether-v2/CAMPAIGN.md) AB-11 row; red test: `feat/threadpool-ke16` `crates/boyko_ecs/tests/ab11_flag_filter_polarity.rs` — absent on this branch |
| **AB-13** | owner | **`true` / `false`.** Settles all four interacting parts at once: the values are Rust keywords already, so nothing new is reserved; the three-way `on` collision (`machine … on entity`, `on E => T`, `flags (X = on)`) **does not arise**, so neither a reader nor a generator needs lookahead; and the group keeps the name `flags`, PENDING Tier 3's withdrawal of the `flags → initial` rename standing on its own sound ground. Spelling: `flags (Stunned = false, Burning = true)` | [`aether-v2/CAMPAIGN.md`](aether-v2/CAMPAIGN.md) AB-13 row |
| **GB-2** | owner | **`#RRGGBB` is `#RRGGBBAA` with maximal `AA`** — a defaulted field, not a second literal kind, which dissolves the reopen question rather than answering it. **And the arity direction the clause did not cover: REFUSE AT BAKE** — an 8-digit literal at a 3-component field (`PointLight.color` is `[f32; 3]`) is a coded refusal naming the field, never a silent alpha drop | `feat/threadpool-ke16` `gaia/DECISIONS.md` §Language shape, the two *Colour* bullets — this branch's `gaia/DECISIONS.md` has no colour line (this file's own F/GB table above already cites §Language shape) |

**Table B — the six delegated `AB` rulings that have no index row on ke16, plus AB-12.** None of
the six rulings named in the ground column (E4, E5, E6, M4a/D6a, C5a, §Sequencing rulings) exists in
this branch's `aether-v2/DECISIONS.md` — verified by grep: no `**E4.` … `**C5a.` ruling lines and no
§Sequencing rulings heading here.

| ballot | who | ruling (verbatim from the ke16 body; `[…]` = elided) | where the ground lives |
|---|---|---|---|
| **AB-2** | delegated | ✅ **RESOLVED 2026-08-30 [delegated]** → ruling **E4** […] **`lanes N` is redefined from a count to a MINIMUM**: the effective count is `max(N, worker_count + 1)`, resolved where the worker count is first known, and the raise is **reported once at boot**, not silent. (ke16 lines 1086-1089) | `feat/threadpool-ke16` `aether-v2/DECISIONS.md` **E4** (line 482 there) |
| **AB-3** | delegated | ✅ **RESOLVED 2026-08-30 [delegated]** → ruling **E5**. **Per-thread claimed host lane, one claimer enforced.** `MAX_EVENT_THREADS` 65 → **66**, const-assert strengthening to `MAX_WORKERS + 2 <= MAX_EVENT_THREADS`. (ke16 lines 1104-1106) | `feat/threadpool-ke16` `aether-v2/DECISIONS.md` **E5** (line 539 there) |
| **AB-4** | delegated | ✅ **RESOLVED 2026-08-30 [delegated]** → ruling **E6**. **Registrant: the generated path**, calling a new `register_ordered_emitter(event_id, system_id)` at plugin build, beside the `preregister_event[_default]` it already emits. **Predicate** (the half the ballot said was missing): at build end, any `ordered` event id with an emitter count **> 1** is a hard boot failure naming both systems. **Verbatim escape: refused at the param list** — a hand-written `EventWriter<E>` for an `ordered` `E` is refused at `EventWriter::init_state`, the site that already panics loudly for an unregistered event and already has `E::event_id()` and the dispatcher in hand. (ke16 lines 1127-1134) | `feat/threadpool-ke16` `aether-v2/DECISIONS.md` **E6** (line 582 there) |
| **AB-5** | delegated | **RESOLVED 2026-08-30** (orchestrator ruling under the owner's delegation of the perf/architecture forks). Original question: the machine event router's random-access mechanism and the tick visibility following from it — (a) amend M4 to `Query::get_mut`; (b) keep `get_component_mut`. **Decision: (a).** Full ruling with the measurements: […] **M4a**, with M7 and **D6a** amended in the same edit and […] §Event routing + CAMPAIGN R5's Depends cell moved with them. (ke16 lines 1145-1151) | `feat/threadpool-ke16` `aether-v2/DECISIONS.md` **M4a** (line 264 there) with **D6a** (line 437 there, a blockquote rather than a numbered ruling); [`aether-v2/MACHINES.md`](aether-v2/MACHINES.md) §Event routing — the heading exists on this branch (:63), the amendment is on ke16 |
| **AB-8** | delegated | **RESOLVED 2026-08-30** (orchestrator ruling; a performance fork, so it is decided with numbers rather than escalated). Original question: the `each par` driver, plus three riders. Full ruling: […] **C5a**. […] **Decision: `each par` → `par_iter_mut`.** (ke16 lines 1226-1231) | `feat/threadpool-ke16` `aether-v2/DECISIONS.md` **C5a** (line 165 there) |
| **AB-9** | delegated | **RESOLVED 2026-08-30** (orchestrator ruling under the same delegation). Original question: `boyko_reflect` sequencing for AIR-06(b). Options were: R8 waits on the merge / AIR-06(b) is descoped to the reflection-free halves (asserted to still satisfy the oracle) / the merge is pulled forward. Ruling recorded in the campaign's own decision log: […] §Sequencing rulings, entry **AB-9**. […] **RULING, three parts.** 1. **Pull the merge forward** […] 2. **AIR-06(b) is NOT descoped.** The descope buys a vacuous green. 3. **A fourth item, which no option on the ballot named, is the real precondition and is attached to R8's Lands**: engine components must opt into reflection […] (ke16 lines 1258-1262, 1295-1303) | `feat/threadpool-ke16` `aether-v2/DECISIONS.md` §Sequencing rulings, entry **AB-9** (lines 722-728 there) |
| **AB-12** | — | ⚠ **STILL OPEN — a query, not a values call; blocks nothing.** Evidence gathered 2026-08-30 points at option (b) — see the paragraph below | the AB-12 body in §2026-08-29 |

*The source section's two closing paragraphs, verbatim (ke16 lines 220-228):*

> **Still open on the Aether side: AB-12 only**, and it blocks nothing. Evidence gathered 2026-08-30
> points at option (b): every `G1`/`G2` occurrence in the 2026-08-29 plan session is a **Gaia rung**,
> not a defect body, and the surviving `AD1..AD4` all have subjects. That is evidence, not proof — the
> search was a grep over a 7 MB transcript, an instrument this campaign has had lie to it three times
> in one day.
>
> **Not ballots, and not the owner's: K8 and K9** — the bundle-with-`link Entity` spawn spelling and
> the relationship macro's private-field demand — are architecture gaps routed to R3's own design
> pass. R3's `bundle` and `relation` constructs may not be declared done while they stand.

**Found while porting, recorded and not repaired.** (i) ke16's own AB-7 body (lines 1194-1225 there)
still opens with the words "STILL OPEN — STILL THE OWNER'S" against its own index row (:214), which <!-- doc-anchor-ignore -->
says the owner lifted R-DENSE — the GB-5 defect class on the source branch, in the Aether half; this
pass writes nothing on that branch. (ii) This branch's `aether-v2/DECISIONS.md:32` still says
"Ballot AB-13 (open — do not settle by edit)" and `aether-v2/CONSTRUCTS.md:135` still points AB-11
at "§Open"; both are outside the census's target files and are owed to whoever owns
`docs/aether-v2/` here. (iii) This branch's AB-6 body below already cites KE14, a backlog row that
does not exist here.

### GB-5 — the one answer the register lost outright

**RULED BY THE OWNER 2026-08-30, recorded 2026-08-31 (`b6c41237`): a scene document MAY declare an
engine-derived field. The authored value is the INITIAL value; the engine takes it over if and when
its condition holds.** The owner's ground, verbatim, is stronger than the ballot's own framing:
*"the third is the most logical — it is simply a starting point in space; obviously this data exists
to be manipulated and will not be static."*

**Required form, and it is not a boolean.** All twelve measured engine-derived field members are
*conditionally* derived, so a per-field boolean pins a predicate that is false for most entities
carrying the field. **The disposition column records the CONDITION and the WRITER** — *this field
may be taken over by `light_reconcile` when the entity has `GlobalTransform`* — which claims nothing
about a particular entity and so cannot be false. The engine already ships the semantics: a spot
light's `direction` is documented as a SEED that `light_reconcile` overwrites
([`light.rs:1207`](../crates/boyko_render/src/light.rs)). For `ContentSize.width`/`.height` a seed is
the **only** correct answer — there is no measurement until the font loads. And it dissolves the
case a refusal could not answer: `Transform` and `RigidBody` are a polarity pair whose author-owned
side flips on `Simulated`, a bit gameplay toggles at RUNTIME.

**The owner's acceptance condition is discharged by construction, so this does NOT return to him:**
*"if the marking costs nothing at runtime and is purely for convenience, then yes"* — the column is
born behind the same default-off bake feature as the rest of the bake machinery and never reaches
the game binary. If it turns out it cannot be kept out of the game binary, the condition is not met
and it does return to him.

⚠ **The mechanism that lost the answer, named so it is not repeated.**
`git show --name-only b6c41237` touches `docs/gaia/DECISIONS.md` **and nothing else** — not
`OPEN-QUESTIONS.md`, not its Russian twin, not `gaia/CAMPAIGN.md`. The EN/RU same-commit rule held
for the commits it covers and simply does not reach `DECISIONS.md`, which has no Russian twin.
**Four registers on `feat/threadpool-ke16` still say GB-5 is open** — that branch's
`docs/OPEN-QUESTIONS.md:250`, `:282` and `:198`, and its `docs/gaia/CAMPAIGN.md:218`, the last of
which reads *"a ruling here would be a defect"*. This pass writes nothing on that branch; the repair
there belongs to whoever owns it. **The consequence is not cosmetic: G1's field-table freeze reads
as BLOCKED in every index and is UNBLOCKED in the log**, and G1 is unstarted —
`field_table|FieldTable|field_by_name` greps **empty** across `boyko_macros` and `boyko_ecs` on this
branch, so the cost is one attribute and one column with zero consumers to update. That cost only
rises.

**The standing cure this argues for** is one grep: a census asserting that no ballot id appearing
under a RULED heading in [`gaia/DECISIONS.md`](gaia/DECISIONS.md) may appear as OPEN in this file,
its Russian twin, or [`gaia/CAMPAIGN.md`](gaia/CAMPAIGN.md). It would have caught GB-5 the day it
landed. GB-8's ruling already puts the census file on the G0 row — but that file
(`tests/gaia_g0_citation_census.rs`) exists on `feat/threadpool-ke16` and **not here**, so writing
the check on this branch would mean porting a gate across a branch boundary. Recorded as owed and
deliberately not done inside a register-sync commit.
✅ **Delivered 2026-09-10, on this branch, written here rather than ported:**
[`tests/gaia_ruled_vs_open_census.rs`](../tests/gaia_ruled_vs_open_census.rs) — root package,
hand-rolled, no per-site waivers (GB-8). Its RULED set is read from the RULED headings of
[`gaia/DECISIONS.md`](gaia/DECISIONS.md) and from the `who` column of this file's ruling tables; its
OPEN set is read from bold spans and headings in this file and [`gaia/CAMPAIGN.md`](gaia/CAMPAIGN.md).
The Russian twin is not scanned: `docs/ru/` is frozen by owner decision of 2026-09-07. Proven red by
mutation on both a table row and a ballot body; the red output is in the file's header.

### F2 — the ruling is CONFIRMED and its original ground is SUPERSEDED

A correction was filed against F2 on the working branch (`docs/OPEN-QUESTIONS.md:146-164` there) and <!-- doc-anchor-ignore -->
must not be left standing as though unanswered: *"the ruling stands but its decisive number was
taken at the size that flatters it."* Its arithmetic: `constants.rs:55` documents a resident floor of
one `POOL_MIN_SLAB` (64 KiB) commit per NON-EMPTY column; a table archetype has four non-empty
columns ⇒ **256 KiB per table regardless of row count**; so twenty 50-row tables cost **~5 MiB
resident for 40 KB of payload**, and the rejected resource region *"wins by two orders of magnitude
at 50 rows"*.

**The ruling stands. That ground does not, and is marked SUPERSEDED.** Three reads on THIS branch:

1. **The floor is symmetric across the fork.** `COMMIT_GRANULE = 64 * 1024`
   ([`constants.rs:7`](../crates/boyko_ecs/src/ecs/constants.rs)) is the commit unit on **both**
   sides — a resource region commits in the same granule. At the correction's own worst case the
   margin is therefore **≤ 4× and ≤ 3.75 MiB**, not two orders of magnitude. One 1024² RGBA8 texture
   is 4 MiB.
2. **The floor keys on SCHEMA count, not FILE count.** `create_archetype` **dedups** by id set and
   `load_archetype` **appends** at the archetype's current row head — the loader says so itself at
   [`load_writer.rs:325-335`](../crates/boyko_ecs/src/ecs/core/serialize/load_writer.rs). N one-row
   documents and one N-row document therefore land in the SAME archetype, and F3's owner-ruled
   one-schema-both-shapes form is exactly the mitigation. The correction's "twenty tables" counts
   files.
3. **Option (b) cannot be built with the engine's own primitive.** `VmColumn::new` asserts
   `COMMIT_GRANULE.is_multiple_of(size_of::<T>())`
   ([`vm_column.rs:144-149`](../crates/boyko_ecs/src/ecs/memory/vm_column.rs)) and 40 ∤ 65536, so
   the correction's "flat column" of a 40 B row type collapses into a pool-reusing variant that
   saves 25 % while paying 100 %. And `grep -rniw "resource" crates/boyko_serialize/src/` returns
   **0** on this branch — (b) writes an entire per-`ResourceId` serialize seam from scratch rather
   than adding a region.

⚠ **What the correction is RIGHT about, carried forward rather than dismissed.** F2 was partly argued
on "dense does not parallelise", and the KE16 kernel finding — every `par_iter` inside a system body
runs on one thread — removes that axis for **both** options: it now discriminates nothing. F2 must
be read on resident memory and access cost alone, which is what the three reads above do. The
internal tension the correction names is also unresolved and is recorded, not closed: F2(ii) rejects
laziness because *"row constants exist to delete a lookup"* while F2(iii) makes the runtime handle
an `Entity`, and reading a row from an `Entity` **is** a lookup. Access cost is priced on neither
option. **Marginal cost:** the honest figure is **511-704 KiB**, not a flat 511 KiB.

### F4 — the ruling stands with three amendments, and its subject MOVED

The 2026-08-30 [delegated] ruling — option (a), suppress-then-fixup, four ordered sub-passes inside
`load_world` — is confirmed. Three amendments follow from measurement and are part of the record:

1. **Sub-pass 4 SPLITS.** **4a** = relationship relink through the already-installed fn-pointer
   table (fires no user code, closes the reverse index and therefore the hierarchy); **4b** = user
   hook dispatch, needed only for refcounts and genuine user hooks. 4a is nearly free: the relink
   table already exists and is FK-driven rather than map-driven (`install_relationship_relink` /
   `get_relationship_relink_fn`,
   [`clone.rs:238`](../crates/boyko_ecs/src/ecs/core/component/component_registry/clone.rs)), and
   the suppression bracket already exists (`LinkSuppressGuard`,
   [`relationship/mod.rs:111`](../crates/boyko_ecs/src/ecs/core/relationship/mod.rs)).
2. **The coverage census is 5 mechanisms × 3 STORAGE KINDS** {table, dense, bitset}, not 5 × 1.
3. **Sub-pass 4b reports per hook class** on `LoadReport`, and a refcount hook that found no
   `RefcountDeltas` must say so.

The fixup runs INSIDE `load_world` and is not optional; it is additionally exposed `pub` over a
narrowed iteration domain so that F5's `load_cell` needs no second implementation.

**Two of F4's five mechanisms were measured this week, and one of them was FIXED — on the other
branch, not on this one.**

- ⚠ **A dense component's `#[entities]` field was never remapped on load. FIXED on
  `feat/threadpool-ke16` by `8d351b2e` (2026-09-03)**: `remap_loaded_entities` gathered its columns
  from `archetype.component_ids()` and had no dense arm at all, so the loader remapped the store's
  OWNER but never the component's INTERIOR — measured as a loaded `DenseRef.target` still equal to
  the SAVED id, with `dense_stores_skipped = 0` and `load_world` returning `Ok`. **The defect is
  still live on this branch**: `remap_loaded_entities`
  ([`load_writer.rs:912`](../crates/boyko_ecs/src/ecs/core/serialize/load_writer.rs)) iterates
  archetypes only, and `LoadReport` here carries neither `remapped_table_rows` nor
  `remapped_dense_rows`.
- ⚠ **Relation reverse indexes are still never rebuilt on load, on BOTH branches**, and the
  consequence is render-visible rather than a query-hygiene nicety. Measured on this branch:
  `grep -rE "trigger_on_[a-z]+\(" crates/boyko_ecs/src/ecs/core/serialize/` returns **0**, and no
  relink call reaches `crates/boyko_serialize/src/`. `Children` classifies `Serializability::Ignore`
  with no rebuilder — the loader's own comment says so at `load_writer.rs:326-330` — and transform
  propagation descends **only** through `Children`:
  [`propagation.rs:349-350`](../crates/boyko_scene/src/propagation.rs) holds the sole `stack.extend`
  edge in the file. **So a reloaded scene's children never compose their parent's pose.** Under the
  2026-09-03 editor ruling that is the editor's very first useful frame.
- ⚠ **A code comment asserts the mechanism that does not exist.**
  [`component.rs:137-142`](../crates/boyko_macros/src/component.rs) states that a relationship
  TARGET's reverse index *"is rebuilt from the sources' `Relationship` on load, exactly as it is
  rebuilt on clone"*. The clone half is true; the load half is the measurement above. The comment is
  identical on both branches and is **deliberately not edited here** — the honest place for it is
  the commit that lands sub-pass 4a and makes it true, and that file is 150 lines diverged between
  the branches. Recorded so the 4a implementer does not have to rediscover it.

Also recorded: the fresh-world contract guard a merge load walks into is a `debug_assert!` at **two**
sites (`load_writer.rs:703` and `:795`), so it vanishes in a shipping build. Editor undo is
structurally a merge load, which makes this an editor concern and not only a streaming one.

### RULED BY THE OWNER, 2026-09-03 — the rulings of today

None of these is a ballot from the 2026-08-28/29 series. All are the owner's own words in his own
session, recorded here because the register is where owner rulings live and because several of them
CLOSE things this entry would otherwise have listed as open. The editor design documents themselves
live on `feat/threadpool-ke16` under `docs/editor/`; they do not exist on this branch, so the
rulings are recorded here and the edits to those documents are owed on that branch.

**The editor edits Gaia DIRECTLY** — scenes, data assets **and** ui documents. His model, verbatim:
*"когда мы изменяем сцену мы можем ее сохранить, в таком случае меняются исходники gaia — ну типа
как json файл. После этого да, мы уже компилируем эту сцену в бинарь."* So the Gaia **TEXT** is the
SOURCE and is what the editor edits and saves; the **BINARY** is the compiled artifact and is what
ships. ⚠ **This SUPERSEDES the editor design's `E6.1`**, which decided the document's persistent
form in v1 to be *"the `boyko_serialize` binary written by `save_world`"*
(`feat/threadpool-ke16` `docs/editor/EDITOR-DOCUMENT.md` §1). The rest of E6.1 — *the document IS
the edit World, there is no retained document model beside it* — is untouched; only the persistent
FORM changes. Consequence carried rather than hidden: the Gaia TEXT printer then needs the same
derived/`Ignore` classification the byte format already has, or the first save-after-load writes
`Children`, GB-5's seed fields and hook-installed state back out as authored data — two sources of
truth in a hand-editable file.

**Gaia proceeds in PARALLEL with the KE16 threadpool work, in separate worktrees** (the standing
worktree-per-system rule). No rung moves; this decides who does the work and where, and it closes
the schedule question this pass had listed as open.

#### GB-6 is DISPOSED, both halves — it stops being an open Gaia ballot

**GB-6(a), the scene-form narrowing — RULED BY THE OWNER.** The §Relations line
([`gaia/CAMPAIGN.md`](gaia/CAMPAIGN.md)) is correct as written and is now **decided**. The
alternative the ballot never named is recorded here, because the ballot's own body observed that its
rejected alternative was its only witness and that this is why it could not be vetoed: the real
alternative was **shipping a scene as TEXT and parsing it at runtime** — Godot's `.tscn` shape — and
the owner has declined it. The four downstream reassignments already filed against the line
therefore **stand** rather than reverting to provisional. The rider that a veto point must state its
price is discharged by the ruling itself: the question is answered, so there is nothing left to veto.

**GB-6(b), the conditional-visibility valve — NOT A GAIA BALLOT AT ALL; it MOVES.** The owner's
objection, and it is correct: *"Не ясно вообще в чем вопрос и причем тут gaia. gaia это не логика а
данные. Логика у нас либо в расте либо в aether."* Traced, the ballot is **misfiled rather than
wrong**: Gaia's own ratified rule already says what he says — structure bakes, only VALUES react, and
a condition in a Gaia file is a token REFERENCE, never an expression — so the data side says only
*this node's visibility is controlled by token X*, and the logic that decides X is Rust or Aether.
The one missing piece is entirely on the Aether side: **Aether has no construct that toggles a
flag**, while the runtime ships three times over (`UiWorldCulled` / `UiWorldHidden` /
`UiWorldOccluded`). **Disposition: it moves to Aether rung R3**, where `tag`, `flag` and the `scene`
surface already live, and it is **struck from the Gaia `GB` series**. ⚠ Recorded because it was
visible and unresolved rather than newly discovered: the ballot was already filed against one rung
while its own body named another.

#### The editor's design questions, ruled today

- **Play window:** a SEPARATE OS window in v1; embedding is v2.
- **Edits made while playing:** discarded. *"Правки применяются только когда не запущен play in
  editor."*
- **The v1 demonstration document:** the **boyko playground** scene.
- **Editor v1 authors UI documents TOO — option (b).** Asked as a straight choice (*Авторит ли
  редактор v1 ещё и документы UI (профиль `ui`), или только сцены и data assets?*), answered
  **"И UI тоже в v1"** — so the editor's *assets are Gaia* rule has **no exception on day one**.
  ⚠ **The pricing pass had RECOMMENDED (a)** — scenes and data assets only, on the ground that it
  is cheap to reverse — **and was overruled.** The losing recommendation is kept visible for the
  same reason a ruling is. **What this does NOT change: F6's answer.** `.ui` is migrated and
  deleted either way, which was already ruled 2026-08-30. **What it changes is F6's SCHEDULE:**
  **G7 moves UP the ladder** rather than sitting after G6, while still waiting on the measured
  prerequisite — UI reaching the screen, which today is a wiring gap and not a missing plugin.
  The `.ui` deletion trigger (a coverage census over `UiTextComponent::ALL` ∪ `UiBackground` on
  the WORLD side) therefore becomes work that is written before the windowed pass lands, not
  after G6. The fact that binds under either answer, and now binds sooner: `.ui` **cannot author
  a background colour at all** — `UiBackground`
  ([`components.rs:219`](../crates/boyko_ui/src/components.rs)) is carried by the
  Panel/Button/Bar bundles and appears **nowhere** under `crates/boyko_ui/src/text/`, so an
  editor saving `.ui` would display a coloured panel and save a colourless one, under a green
  round-trip gate whose universe never contained it.
- **Destructive commands issued by an agent** (`entity.despawn`, `document.save` over an existing
  file, `play.stop`) require an explicit `confirm: true` over the transport — never when the same
  command is issued by the editor's own UI.
- **The edit viewport stays FROZEN, and a preview toggle is NOT in v1.** *"у нас же два режима во
  вьюпорте есть — редактирование, где всё заморожено, и play; ну как в unreal engine."*
  ⚠ **This REVERSES a recommendation made earlier the same day** — the analysis had argued for a
  preview toggle on the ground that the editor process still runs ENGINE systems (particles,
  animation, physics) even without game systems, so a preview would be meaningful. That
  recommendation is kept visible here with its disposition attached rather than deleted. The owner
  ruled two modes, and the consequence follows the ruling into the record: tuning a particle effect
  goes through Play or through a separate preview panel, and the main viewport keeps two states.
- **The 3D viewport has a FIXED render resolution taken from CONFIG; the UI scales, the render plane
  does not follow the window.** *"ты ставишь разрешение вьюпорта в конфиге и потом просто интерфейс
  скейлится, реальная плоскость рендеринга вьюпорта остается такой же."* ⚠ **This DISSOLVES the
  resize question rather than answering it**: the scene stops being something that must be reconciled
  with the window extent and becomes a fixed-size texture the UI composites into a panel rect. Two
  consequences follow, and they are recorded because one of them inverts a price everyone assumed:
  1. **The "editor viewport as a sub-rect" question is settled as SUB-RECT — and the sub-rect is
     CHEAPER than fullscreen, not more expensive.** The assumed price of a sub-rect was extent
     plumbing threaded through the renderer; under a config-fixed render target **no extent plumbing
     is threaded at all**, because nothing downstream of the target ever learns the window size.
  2. *Engineering detail decided under the standing rule, recorded as MINE and not his:* scaling into
     a panel of a different aspect ratio must **preserve aspect — bars at the edges — never stretch
     per axis.** Today the blit stretches, which is why a non-proportional window drag distorts the
     3D. Fixing that is part of the compositing step this ruling creates, not a separate campaign.

#### ⚠ Replay determinism — the owner CHANGED a design decision, not just answered a question

The editor replay design (`feat/threadpool-ke16` `docs/editor/EDITOR-REPLAY.md`, decision **E4.5**)
states that the sufficiency oracle is **EXPECTED RED on event-lane order** — *"events of two systems
interleave PER WORKER THREAD, so lane order follows scheduling"* — and offers "make lane order
deterministic", a scheduler change with a real cost, as the alternative. **The owner rejected that
framing and gave a better design:**

> *"можно сделать отдельный флаг чтобы в реплей записывать порядок воркеров и потом отдельный флаг
> воспроизведения чтобы это считывать. Тогда при дебаге это позволит нормально дебажить, а если цель
> просто посмотреть демо игры как в dota — производительность не будет страдать. В любом случае в
> 90% игр реплеев вообще не будет и производительность должна быть максимальной."*

His ground is the load-bearing part: **worker divergence between runs is not a defect, it is what a
work-stealing pool does** — so constraining the scheduler fights the mechanism for a property that
one mode of three needs. RECORD the outcome instead of constraining the engine, which is the same
principle the replay design already applies to input: record the seam.

**Two flags:** a RECORD flag that writes the lane order into the replay, and a PLAYBACK flag that
reads it back. A debug replay is exact; a spectator/demo replay does not pay for it; a game with no
replay pays nothing.

Consequences, written into the record rather than left to be rediscovered:

1. **E4.5's "expected RED" is WITHDRAWN.** The oracle is not red — it is GREEN under the debug flag
   and honestly NOT-APPLICABLE under the demo flag. Those are different states, and the document
   must stop conflating them. E4.5's old position is not deleted; it is superseded, with this date.
2. **The kernel scheduler change ("make lane order deterministic") is no longer a prerequisite of
   replay.** If it is wanted later it is wanted on its own merits, and it is no longer owner
   question 4 of that design.
3. *Engineering note, recorded as such and not as the owner's words:* what needs recording is the
   **merge order of the lanes per frame** — a permutation — not which worker took which task. That
   leaves the event WRITE path untouched and costs a few bytes per frame.
4. ⚠ **One verification is OPEN and is filed as open, not assumed:** recording an ORDER restores
   sequence but not any non-determinism in parallel-computed VALUES. Physics is covered — the
   colored solve's {1, N}-worker bit-identity is pinned — but any OTHER parallel float reduction
   needs its own answer. That check is outstanding.

### Still open, and NOT answered here

After today's rulings **no scoping question remains open — three ballots do.**

**Closed today, and recorded here so each change of state is visible rather than silent:**
**GB-6** is disposed in both halves (see the ruling section above) and is no longer a Gaia ballot.
**EDITOR-UI-SCOPE** — created by the editor ruling and open for the length of one section of this
entry — is **RULED (b)**, UI documents in v1 too; the pass had recommended (a) and was overruled,
and the bullet above records both. **CAMPAIGN-PRIORITY** is answered by the parallel-worktrees ruling;
what survives of the pass's recommendation there is its ordering constraint alone — the register
sync and F4's sub-pass 4a land FIRST, and the sync lands in the MAIN checkout.

**F8, F9's residual and F10 also remain unanswered, and are NOT settled here.** The pricing pass
offers a disposition for each under the standing architecture rule — F8: the object NAME is not the
id, the ratified file-local id slot stands, and the reopen never met AIR-10's own bar (it stated its
trigger and never its measurement). F9's residual: no per-object `flags (…)` group; per-object
authored state is a durable COMPONENT FIELD owned by exactly one deriving system. F10: not a ballot
at all — a bundle is a CODE fact, so it expands from the live derive-emitted manifest at load and is
never frozen at bake. **Each of those three is a recommendation offered as a veto line, not a
ruling**, and each is recorded as such at its ballot body below.

---

## RESOLVED 2026-09-02 — KE16's six owner calls, answered the same day they were asked

**Where the work is.** Worktree `D:/wt/threadpool`, branch `feat/threadpool-ke16`. The design is
`docs/threadpool/KE16-DESIGN.md` and its five axis files; the measurement plan is
`KE16-DESIGN-MEASUREMENT.md`. The catalogue behind it (`KE16-DESIGN-SPACE.md`, the `KE16-VARIANTS-*`
files, `KE16-EVIDENCE.md`) is 99 variants on 35 axes. Eighteen candidates are being built behind
mutually exclusive cargo features and will be measured one axis at a time; the winner becomes
unconditional and every feature is deleted in the same pass.

**What the harness established before any fix** (release, 16 workers, this box): a wave spawned from
inside a worker is *exactly* serial (max-in-flight 1, wall-clock on the serial floor within 1 %);
`par_iter` inside a scheduled system is 1.00× against 10.82× from the bench thread at 65 536 rows;
the colored solve on the route that ships is **36.02 ms against 31.52 ms single-threaded** — enabling
parallel physics as wired today costs 14 % more than leaving it off. Every O-series colored-solve
number on record was taken on the healthy route the engine does not use.

The six calls below were VALUES or SCOPE. All six are now ruled.

1. **Commit the harness — YES.** The five instrument files and the three `Cargo.toml` hunks carry
   every number above and ship with the pass. The documentation corpus is committed ahead of the
   implementation so the design and its evidence exist on a commit while the axes are still in
   flight; the instruments follow with the code they measure.

2. **The MSDF bake — OPTIMISE IT, do not merely choose a joiner policy for it.** Owner: *"if it can
   be optimised somehow, optimise it."* Reading the site changed the question.
   `generate_distance_field` (`crates/boyko_fontbake/src/msdf/distance.rs`) calls `pool.install` from
   an application thread and spawns `4 × workers` disjoint row bands — 64 tasks at W=16. That is the
   external-joiner route, and it is the one route where **defect B is live in production today**: the
   joining bake thread batch-steals up to 33 of those 64 bands into a private unregistered scratch
   deque and runs them one at a time. The measured signature of that shape on the harness is a
   33-of-64 serial residue. So the bake is not choosing between "keep a helper" and "lose a lane" —
   it is currently paying the defect in full, and **both** B candidates improve it. Two consequences,
   both now design requirements rather than notes:
   - **The bake gets its own bench row, and it decides B1 versus B3.** The B verdict is no longer
     settleable on code size: `crates/boyko_fontbake` gains a criterion row over a real glyph on both
     joiner policies, and whichever is faster on it wins the external arm.
   - **`spawn_batch` at the bake moves from out-of-scope into the pass.** It was recorded as "a
     one-line follow-up if `ke16-c-batch` ships"; under this ruling, if that candidate wins its step,
     the bake is converted in the same pass and re-measured on the same row.

3. **The scheduler-level lever is KE17 — CONFIRMED, "надо будет исправить отдельно".** Lanes draining
   at different times at the ECS level is the apply-window barrier (`schedule.rs`, the
   `pending == running` gate), not the pool; system assignment is already dynamic. The row is open in
   `docs/aether-v2/KERNEL-BACKLOG.md` as of 2026-09-02, work not started, and it is expected to be
   worked separately after this pass.

4. **Windows timer resolution — RAISE IT.** Owner, after hearing the cost: *"ну давай повысим тогда"*,
   which overrides the design's earlier refusal. The reasoning is kept so the record of *why*
   outlives the call: `timeBeginPeriod(1)` raises the timer-interrupt rate, so the CPU wakes more
   often and cannot settle into deep idle states — idle draw rises and battery life falls. It does
   **not** make running code faster; it makes a *timed wait* expire close to when it was asked to.
   Workers here are woken directly by `unpark`, so the timer governs only the lost-wakeup backstop —
   but at the default ~15.6 ms resolution a lost wakeup costs a whole frame instead of 50 µs, and
   that is the insurance the owner bought. It ships as **App-12**: a hand-declared `winmm` FFI pair
   behind an RAII guard that raises 1 ms at host boot and restores it on drop, `#[cfg(windows)]`,
   living in the host layer (`crates/boyko_app`) because a library must not silently change
   process-wide state — the pool and the ECS do not call it. It is measured both ways in
   `crates/boyko_app/tests/app12_timer_resolution.rs`, so the pass reports what the change bought
   rather than asserting it. The pass still removes the *dependency* on the backstop (count-gated
   completion); raising the resolution makes the residual window cheap instead of frame-sized. Two
   prose claims in the tree that nothing calls `timeBeginPeriod` are corrected in the same pass.

5. **A helping joiner may run a sibling system inline — ACCEPTABLE.** A worker joining its own wave
   can pick up a conflict-free sibling system and run it inside another system's body. Sound by the
   conflict graph; visible as two overlapping system spans on one lane and interleaved events within
   one event lane. The in-system guard becomes a depth counter (`App-8`).

6. **Scope of the O-series retake — CONFIRMED.** The shipping-route numbers go into a new
   `docs/threadpool/KE16-RESULTS.md`; the O6 tables get one dated line pointing there. The O-series
   documents are not rewritten.

---

## 2026-08-30 — what the adversarial pass found in the rulings above, and the kernel finding that outranks all of them

**Recorded, not repaired.** The rulings in this file landed and their gates are green; two independent
reviewers then found the items below. They are written here rather than fixed because a repair taken
on incomplete information generates a second repair — and one of these findings changes the axis
that F2 was decided on.

### ⚠⚠ KERNEL — every `par_iter` inside a system body runs on ONE thread

Measured 2026-08-30, three independent instruments, release, 16 workers, 16384 rows, ~50 ns/row body:

| arm | ns/row | speedup | max in flight |
|---|---|---|---|
| `for_each_chunk`, no pool (reference) | 50.10 | 1.00× | — |
| `par_for_each_chunk` under `pool.install` | **6.51** | **7.69×** | 16 |
| `par_iter()` **in a system body** | 49.36 | **1.01×** | — |
| `par_for_each_chunk` **in a system body** | 48.69 | **1.03×** | **1** |

**Mechanism, one line** *(as it stood before KE16; struck 2026-09-10 by the merge
`merge/ke16-into-render`)*. ~~`crates/boyko_threadpool/src/worker.rs:370-371` (`push_task`) sends a task
spawned *by a worker* into `injector_local[wid]`. Sibling stealing iterates `inner.stealers`, which
holds the worker **deques** only — **no thread ever polls another thread's local injector.** Work
spawned from inside a worker is reachable by that worker alone, serial by construction.~~ A system
body always runs on a worker. **Superseded:** `push_task` is now `worker.rs:750`, and under the shipped
`a1f` placement it pushes onto the worker's own REGISTERED Chase-Lev deque, which sibling stealing DOES
poll (`docs/threadpool/KE16-DESIGN.md`; the §App-2 take in `docs/threadpool/KE16-RESULTS.md` measures
`par_in_system / par_from_dispatcher = 1.004`). The struck sentence is the record of the defect KE16
closed, not a description of the tree. The anchor was moved with the strike rather than alone, because
a renumbered citation under a refuted claim reads as a live claim.

**The query drivers are innocent**: a raw `pool.scope` with zero ECS code, opened from a worker,
serialises identically.

**Second, independent occupancy defect on the same principle.** Even on the healthy path only
**4–5 of 16** tasks are ever simultaneously live: `Scope::drop` steals roughly half the wave into a
private `scratch` deque that has **no registered `Stealer`**, and runs it inline, one at a time.
Same shape — runnable work parked where nobody can steal it.

**Blast radius.** All four parallel physics sites are on the defect path — `solver/colored.rs:2667`,
`soft/colored.rs:1031`, `resources.rs:1688`, `:1760` — plus 7 `par_iter_mut` call sites, all in
system bodies. Render, UI and app have **zero** exposure. ⚠ **The O-series colored-solve speedups
were measured from a bench thread (dispatcher = the healthy path); production takes the serial one.
Those numbers must be RE-TAKEN, not re-read.**

**Why three layers of defence all missed it.** `tests/scheduler_par_iter_concurrent_systems.rs`
asserts no-deadlock and full row coverage and **nothing about threads** (two matches for
`num_threads`, both in a builder). The one bench on the path, `g3_boyko_par_iter_10k`, has an
absolute ≤41 µs ceiling with **no single-thread baseline** and a body that is a `fetch_add` on one
shared static — which runs *faster* serialised. And two doc comments (`thread_pool.rs:128-129`,
`worker.rs:356`) assert siblings *can* see these tasks "via stage 1.5 of `worker_main`" — **there is
no stage 1.5**; `colored.rs:2629-2631` likewise assumes a lane pool of `num_threads + 1` where it is
**1**.

**Inter-system parallelism is unaffected and measured healthy**: four conflict-free systems under
`Schedule::run` give `max_inflight=4`, `lanes_busy=4`, `top_lane=25.1%`. The two kinds of
parallelism this engine plans for are both implemented; the between-systems kind works, the
within-a-system kind does not.

### F2 — the ruling stands but its decisive number was taken at the size that flatters it

`constants.rs:55`: *"Resident floor: one `POOL_MIN_SLAB` (64 KiB) commit per NON-EMPTY column."* A
table archetype has four non-empty columns ⇒ **256 KiB per table, independent of row count.** F3
(owner-ruled) admits N table files through one schema, so twenty small tables of 50 rows × 40 B cost
**~5 MiB resident for 40 KB of payload (~128× overhead)** plus twenty extra archetype mask tests on
every query, every frame. The rejected resource region loses by ~25 % at 4000 rows and **wins by two
orders of magnitude at 50**. The ruling never says the choice is size-dependent, and the mechanical
rule it gives keys on a different property. Marginal cost also mixes units: honest figure is
**511–704 KiB**, not a flat 511 KiB.

⚠ **And the parallelism finding above removes the axis F2 was partly argued on** — "dense does not
parallelise" no longer discriminates between the options, because *nothing* parallelises inside a
system today. F2 must be re-argued on resident memory and access cost.

Internal tension to resolve with it: F2(ii) rejects laziness because *"row constants exist to delete
a lookup"*, and F2(iii) then makes the runtime handle an `Entity` captured at load — reading a row
from an `Entity` **is** a lookup (inland resolve + archetype pool). Access cost is priced on neither
option.

### GK-1 as specified violates Principle 0

`LoadEntityMap` is legitimate **today** because it is a stack local of `load_world` — the named
"truly transient function-local scratch" exception. GK-1 requirement 4 makes it world-owned in a
`Resource` with a lifetime spanning every loaded cell, keeping the `Vec` — which converts the
exception into exactly the durable parallel data system Principle 0 forbids. **The in-tree answer is
one line away and the ruling already cites the file**: `PathIndex { entries: VmColumn<PathEntry>,
sorted_len, … }` — VM-native, sorted prefix + unsorted tail, HashMap-free. `(u64, Entity)` is 16 B
and divides the granule.

### GB-3's dangle check is structurally unreachable downstream of the sigil pass

A dangling asset reference produces **one error-level log line and an otherwise fully-formed,
unvalidated, refcount-holding entity**: `AssetServer::load` logs `E0801` and returns a live handle in
the `Failed` state, which it inserts into the path index. `validate_asset_refs` early-returns unless
`free_epoch` advanced — a never-loaded asset frees nothing, the epoch never moves, and the row is
**never examined**. A reference kind whose only validator is owed is a silent-wrong-answer generator.

### The shape four of the five blocking findings share

> *A mechanism was verified to EXIST and was not verified to be REACHABLE, or to behave, on the path
> being ruled about.*

`Construct` exists — for pooled ids. `remap_loaded_entities` exists — for fresh worlds.
`relationship_on_insert` exists — with a destructive guard that removes the relationship component
rather than skipping. `apply_attach_flags_*` is absent from the loader and reachable from its drain.
**The rulings' citations are accurate; what they under-tested is the second question.** This is the
same question the kernel finding above answers for `par_iter`, from the other side.

### Owner ballots still open

⚠ **DATED NOTE, 2026-09-10 (`merge/ke16-into-render`) — this heading and the paragraph under it are `feat/threadpool-ke16`'s record of 2026-08-30, and are no longer a live list.** GB-5 was RULED BY THE OWNER on 2026-08-30 — PERMIT AS SEED, recorded 2026-08-31 by `b6c41237`; ground at `docs/gaia/DECISIONS.md:1262` and the who-table row `docs/OPEN-QUESTIONS.md:1031`. GB-6 was DISPOSED BY THE OWNER on 2026-09-03, both halves; ground at the who-table row `docs/OPEN-QUESTIONS.md:1032` and §*2026-09-03* (`docs/OPEN-QUESTIONS.md:1306`). `F8`, `F10` and `AB-12` are untouched by this note.

**F8** (name-vs-id) and **F10** (bundle expand-or-refuse) — untouched, and no ruling above settles
either. ~~**GB-5** and **GB-6** carry analyses, deliberately not rulings: the owner asked for
trade-offs.~~ **AB-12** remains, and blocks nothing.

## 2026-08-30 — the owner's rulings, and the Aether ladder closes

Seven ballots answered by the owner in one session, plus eleven decided by the orchestrator under
the standing rule that performance and architecture forks are settled with numbers. **Every `AB`
ballot is now closed except AB-12**, which blocks nothing.

Recorded here as the single index; each ruling's full ground and rejected alternative live at the
site named in its row.

| ballot | ruling | where the ground lives |
|---|---|---|
| **AB-1** | **RATIFIED as specified** — event auto-registration adopted on ergonomic grounds; the STAGE-under-D4 alternative rejected. ⚠ The reopen was licensed because C3's recorded ground — *"the unregistered case fails silently on both ends"* — was **refuted by measurement**: both generated ends are a loud init-time panic. The grant now stands on a stated ground (ergonomics), not on the refuted one | [`aether-v2/DECISIONS.md`](aether-v2/DECISIONS.md) C3 |
| **AB-6** | **Option (b), and the class split in two.** The ballot's premise was refuted first: hand-written Rust could not do it either, so there was no ratified surface to narrow — only a promise the kernel did not keep. **Dense**: build the construct-and-commit route. **Bitset/flag**: refuse in the **derive**, since a flag has no bytes and `FLAGS_DIRECT` is the mechanism. Refusing in Aether alone was ruled out because it would make Aether reject what the derive accepts | [`aether-v2/KERNEL-BACKLOG.md`](aether-v2/KERNEL-BACKLOG.md) AB-6 + **KE14** |
| **AB-7** | **R-DENSE LIFTED, and the ballot's framing rejected.** Owner: *"that there is no parallelism is just wrong."* The candidate driver-independent ground had already been measured and **refuted on both conjuncts**; the owner then declined to treat the driver limitation as a constraint at all. Measured: the refusal is a `const assert` whose own comment says the chunk runner *"has no world cell"* — unwired plumbing, not a design limit, and the same gap blocks `Related` joins | [`aether-v2/KERNEL-BACKLOG.md`](aether-v2/KERNEL-BACKLOG.md) **KE15** |
| **AB-10** | **(a) the measurement script IS required** when the audit branch is taken. **(b) the widening to the joint Aether+Gaia vocabulary is RATIFIED** — if the two languages are one body of work, the vocabulary is one vocabulary, and a word meaning different things across them is a false friend between the author's own languages. Price accepted: the risk list grows with Gaia's vocabulary, and a collision there may force a rename in Aether | [`aether-v2/AI-ORIENTATION.md`](aether-v2/AI-ORIENTATION.md) AIR-10 / AB-10 |
| **AB-11** | **The recommended option — a parse refusal** with a did-you-mean pointing at `enabled` / `disabled`. ⚠ Adds a refusal where v1 documents non-refusal, which is what made it the owner's. Ground, measured with R0's fix already in the tree: over a `flag`, `with F` matches nothing and `without F` excludes nothing — two different silent wrong answers — and the defect is in the **leaves**, so R0's `Or` fix does not reach it | [`aether-v2/CAMPAIGN.md`](aether-v2/CAMPAIGN.md) AB-11 row; red test `ab11_flag_filter_polarity.rs` |
| **AB-13** | **`true` / `false`.** Settles all four interacting parts at once: the values are Rust keywords already, so nothing new is reserved; the three-way `on` collision (`machine … on entity`, `on E => T`, `flags (X = on)`) **does not arise**, so neither a reader nor a generator needs lookahead; and the group keeps the name `flags`, PENDING Tier 3's withdrawal of the `flags → initial` rename standing on its own sound ground. Spelling: `flags (Stunned = false, Burning = true)` | [`aether-v2/CAMPAIGN.md`](aether-v2/CAMPAIGN.md) AB-13 row |
| **GB-2** | **`#RRGGBB` is `#RRGGBBAA` with maximal `AA`** — a defaulted field, not a second literal kind, which dissolves the reopen question rather than answering it. **And the arity direction the clause did not cover: REFUSE AT BAKE** — an 8-digit literal at a 3-component field (`PointLight.color` is `[f32; 3]`) is a coded refusal naming the field, never a silent alpha drop | [`gaia/DECISIONS.md`](gaia/DECISIONS.md) §Colour |

**Still open on the Aether side: AB-12 only**, and it blocks nothing. Evidence gathered 2026-08-30
points at option (b): every `G1`/`G2` occurrence in the 2026-08-29 plan session is a **Gaia rung**,
not a defect body, and the surviving `AD1..AD4` all have subjects. That is evidence, not proof — the
search was a grep over a 7 MB transcript, an instrument this campaign has had lie to it three times
in one day.

**Not ballots, and not the owner's: K8 and K9** — the bundle-with-`link Entity` spawn spelling and
the relationship macro's private-field demand — are architecture gaps routed to R3's own design
pass. R3's `bundle` and `relation` constructs may not be declared done while they stand.

### The Gaia side, same day — fourteen ballots put, TWELVE answered

Six the owner settled outright, four he **delegated** (ruled below and in
[`gaia/DECISIONS.md`](gaia/DECISIONS.md)), and two he asked to be **analysed, not decided** — those
two ~~stay OPEN~~, with the analysis attached to their bodies in §2026-08-29 so he can rule from one
reading. ~~**A ruling written for either would be a defect.**~~ Unanswered: **F8** and **F10**, plus
**F9's residual VALUES question**, which the delegated ruling deliberately did not settle.

⚠ **DATED NOTE, 2026-09-10 (`merge/ke16-into-render`).** The two ballots this paragraph and the table below send back for analysis have since been ruled, and every sentence here and in that table which says otherwise is struck in place: GB-5 was RULED BY THE OWNER on 2026-08-30 — PERMIT AS SEED, recorded 2026-08-31 by `b6c41237`; ground at `docs/gaia/DECISIONS.md:1262` and the who-table row `docs/OPEN-QUESTIONS.md:1031`; GB-6 was DISPOSED BY THE OWNER on 2026-09-03, both halves; ground at the who-table row `docs/OPEN-QUESTIONS.md:1032` and §*2026-09-03* (`docs/OPEN-QUESTIONS.md:1306`). `F8`, `F10` and F9's residual VALUES question are untouched by this note.

| ballot | who | ruling | where the ground lives |
|---|---|---|---|
| **F1** | owner | **Macro-time GK-4, NOW** — not sequenced behind EG2. *"Do it properly right away. But bear in mind the world must support streaming."* Both halves bind. ⚠ The ballot's own `RequiredCtor`-is-unbakeable inference is **refuted**: a GK-4 baker is a Rust program linked against the derive tables and calls the fn pointer trivially (measured through today's public `required_ctor_in_set`); only the *evaluator over text* cannot | [`gaia/DECISIONS.md`](gaia/DECISIONS.md) §Inherited pipeline |
| **F3** | owner | **ONE schema for both file shapes.** The table file is a spelling, not a second type system. Rejected: a table dialect | [`gaia/DECISIONS.md`](gaia/DECISIONS.md) §Data tables |
| **F5** | owner | **EVERYTHING FROM THE START — option (b), the one nobody had written down.** *"Все сразу грамотно по списку с самого начала."* `load_cell`/`unload_cell`, GK-1's cross-load map with a declared lifetime, and cross-cell reference resolution ship **with** the scene profile. ⚠ The single largest change to the campaign's ground: **G6 absorbs the hardest half**, because a per-load `LoadEntityMap` cannot express references BETWEEN chunks | [`gaia/DECISIONS.md`](gaia/DECISIONS.md) §Streaming scope; [`gaia/CAMPAIGN.md`](gaia/CAMPAIGN.md) rows G6 and G8 |
| **F6** | owner | **Migrate `.ui` to the Gaia `ui` profile and DELETE the old format in the same campaign.** Price accepted and recorded at G7: the migration is done **blind** | [`gaia/DECISIONS.md`](gaia/DECISIONS.md) §Language shape |
| **F7** | owner | **Mods are NOT supported for now**, ratified explicitly — the ballot existed because silence was itself a decision. The reflection constraint is recorded in its **narrow** form (no reflection in the game binary), so a later mod campaign is not blocked by a sentence that never meant to block it | [`gaia/DECISIONS.md`](gaia/DECISIONS.md) §Refusals |
| **GB-3** | owner (adoption) + delegated (the rewrite) | **The third reference kind is ADOPTED**, and the four-part rewrite is ruled: sigil / styles-are-kind-1 / `$hole` stays / "declared node ⇒ `@`" withdrawn. ⚠ The glyph itself is **not** minted — it cannot be chosen before Gaia's arithmetic operator vocabulary is closed | [`gaia/DECISIONS.md`](gaia/DECISIONS.md) §Identity and references |
| **F4** | delegated | **(a), as SUPPRESS-THEN-FIXUP** — four ordered sub-passes on the clone path's own precedent. (b)'s mechanism adopted as sub-pass 4; its timing refuted at a measured 100% mis-link rate. Carries the stable-asset-carrier addendum | [`gaia/DECISIONS.md`](gaia/DECISIONS.md) §Load semantics |
| **F2** | delegated | **(a) rows are entities — AMENDED**: `StorageKind::Table` (not dense), eager at load, own explicit load and pinned. (b) saves 0.13 MB and costs the entire per-`ResourceId` serialize seam from scratch | [`gaia/DECISIONS.md`](gaia/DECISIONS.md) §Data tables |
| **F9** | delegated, **partly** | **Three eliminations RULED** — do not fire `FLAGS_DIRECT` on load; the refusal option contradicts ratified **AB-6**; the spelling, if a carrier lands, is `flags (X = true)` shared verbatim with Aether. ⚠ **The residual is a VALUES call and is escalated back:** may a document set a flag on an *individual authored object*? | [`gaia/DECISIONS.md`](gaia/DECISIONS.md) §Flags in the byte format |
| **GB-4** | delegated | **(a) linkage-in-slot, and the linkage word is MANDATORY**: `instance <name> extends|copy <base-ref>`. `from` deleted (head-position occurrences measured: **zero**) | [`gaia/DECISIONS.md`](gaia/DECISIONS.md) §The instance spelling |
| **GB-5** | **owner asked for ANALYSIS** | ⚠ ~~STAYS OPEN.~~ **Struck 2026-09-10 by the merge `merge/ke16-into-render`: GB-5 was RULED BY THE OWNER on 2026-08-30 — PERMIT AS SEED, recorded 2026-08-31 by `b6c41237`; ground at `docs/gaia/DECISIONS.md:1262` and the who-table row `docs/OPEN-QUESTIONS.md:1031`. This row is the record of 2026-08-30, not a live status.** The analysis is attached to the ballot body below. It settles one thing only, and it is a refutation of the ballot's own mechanism: **all 12 measured engine-derived field members are *conditionally* derived**, and both dispositions are pinned by green committed tests today | §2026-08-29, the GB-5 body |
| **GB-6** | **owner asked for ANALYSIS** | ⚠ ~~STAYS OPEN.~~ **Struck 2026-09-10 by the merge `merge/ke16-into-render`: GB-6 was DISPOSED BY THE OWNER on 2026-09-03, both halves; ground at the who-table row `docs/OPEN-QUESTIONS.md:1032` and §*2026-09-03* (`docs/OPEN-QUESTIONS.md:1306`). This row is the record of 2026-08-30, not a live status.** Analysis attached below: (a) the disposition is already declarative in four documents with four downstream reassignments filed against it, and its rejected alternative is **its own only witness**; (b) the valve's runtime already ships three times over and the whole gap is **one Aether construct** | §2026-08-29, the GB-6 body |


## 2026-08-29 — Corpus audit of the aether-v2 + gaia plans: THIRTY-TWO open ballots, listed here because a plan that settles a fork silently is the defect

A multi-lens review of the two plan corpora ([`aether-v2/`](aether-v2/CAMPAIGN.md),
[`gaia/`](gaia/CAMPAIGN.md)) returned 57 adjudicated edits. The determinate ones — false engine
claims, phantom artifacts, gates that cannot fail — are written into the documents themselves. What
an edit cannot do is decide a fork, and the review found the corpus quietly deciding them: a
recommendation printed as if ratified, an alternative never named, a refusal re-grounded in passing.
Every such fork is below, with what it blocks.

**Nine of them REOPEN something already ratified** — F7, F8, GB-3, AB-1, AB-6, AB-7, AB-10 (the
PENDING Tier 1 AIR-10 widening), AB-11, and AB-13's rename option. Those nine carry ⚠ and name what
they touch; no other item on this list does, so the marks and the count check each other. None may
be settled by an edit; where a reopen is licensed it is because a measurement refuted the original
premise, and that is said at the item.

**Nothing here blocks R0, R1 or R2 on the Aether ladder** — those are buildable immediately (KERNEL-BACKLOG **KE11**'s
red tests land regardless of its disposition). Everything above them waits on a line from this list.

> ✅ **STATUS, 2026-09-03 — read this before any ballot body below.** The bodies in this section are
> kept verbatim as the record of what was open and why; **they are no longer a to-do list.** Of the
> fourteen Gaia ballots, **twelve are answered** — six by the owner outright (F1, F3, F5, F6, F7 and
> GB-3's adoption), four **delegated** and since ruled (F2, F4, F9 in part, GB-4), and four by
> standing rule (GB-1, GB-7, GB-8, GB-9). **Still open: F8, F10 and F9's residual VALUES
> question** — with two corrections: **GB-5 is RULED** (permit as seed, 2026-08-30) and only the
> registers said otherwise, and **GB-6 was DISPOSED 2026-09-03** in both halves, its (b) half
> moving to Aether R3. Every `AB` ballot except AB-12 is closed. The
> per-ballot index, the git provenance and the F2/F4 corrections are in
> §*2026-09-03 — the Gaia register catches up with the owner*, above. Each body below now carries its
> own disposition line, so no body has to be read against a stale header.
>
> *— and the STATUS as `feat/threadpool-ke16` wrote it on 2026-08-30, kept by the merge `merge/ke16-into-render`, 2026-09-10. It is the earlier of the two; where they disagree — GB-5 and GB-6, which it lists as open — the 2026-09-03 paragraph above is the later state.* ⚠ **Both have since been ruled, and every sentence below that says otherwise is struck in place:** GB-5 was RULED BY THE OWNER on 2026-08-30 — PERMIT AS SEED, recorded 2026-08-31 by `b6c41237`; ground at `docs/gaia/DECISIONS.md:1262` and the who-table row `docs/OPEN-QUESTIONS.md:1031`; GB-6 was DISPOSED BY THE OWNER on 2026-09-03, both halves; ground at the who-table row `docs/OPEN-QUESTIONS.md:1032` and §*2026-09-03* (`docs/OPEN-QUESTIONS.md:1306`).
>
> ✅ **STATUS, 2026-08-30 — read this before any ballot body below.** The bodies in this section are
> kept verbatim as the record of what was open and why; **they are not a to-do list any more.**
> **Every `AB` ballot except AB-12 is now closed**, along with `GB-1`, `GB-2`, `GB-7`, `GB-8` and
> `GB-9`. Seven were answered by the owner and eleven by the orchestrator under the standing
> perf/architecture rule. The index with each ruling and where its ground lives is
> §*2026-08-30 — the owner's rulings, and the Aether ladder closes*, above.
>
> ✅ **UPDATED LATER THE SAME DAY — the Gaia fourteen were put to the owner and TWELVE are
> answered.** Settled by him: **F1, F3, F5, F6, F7** and **GB-3**'s adoption. Delegated and now
> ruled: **F2, F4, GB-4**, and **F9 in part**. Sent back for **analysis, not decision**: **GB-5**
> and **GB-6** — both ~~stay OPEN~~, and the analysis is attached to each body below.
> **Still open and genuinely awaiting an answer: `F8`, `F10`,** ~~`GB-5`, `GB-6`,~~ **and F9's residual
> VALUES question** (may a document set a flag on an individual authored object). Plus **AB-12**,
> which blocks nothing. The per-ballot index is the table in
> §*2026-08-30* above, subsection *The Gaia side, same day*.
> ⚠ *Struck 2026-09-10 by the merge `merge/ke16-into-render`, which imported this 2026-08-30 paragraph from `feat/threadpool-ke16`.* GB-5 was RULED BY THE OWNER on 2026-08-30 — PERMIT AS SEED, recorded 2026-08-31 by `b6c41237`; ground at `docs/gaia/DECISIONS.md:1262` and the who-table row `docs/OPEN-QUESTIONS.md:1031`; GB-6 was DISPOSED BY THE OWNER on 2026-09-03, both halves; ground at the who-table row `docs/OPEN-QUESTIONS.md:1032` and §*2026-09-03* (`docs/OPEN-QUESTIONS.md:1306`). `F8`, `F10` and F9's residual VALUES question keep the status the 2026-09-03 STATUS paragraph at the head of this blockquote gives them.

### Gaia — the `F` series (F1..F7 raised 2026-08-28, full bodies in the entry below; F8..F10 born in this review)

- **F1 — the bake route.** EG2 reflection seam (already rejected by the 2026-08-27 audit ballot) vs
  macro-time GK-4 now with EG2 as a later upgrade. The real question is SEQUENCING. Blocks **G1**.
  ✅ **RESOLVED 2026-08-30 BY THE OWNER — macro-time GK-4, NOW, with streaming not foreclosed.**
  Ground: `feat/threadpool-ke16` `gaia/DECISIONS.md` §Inherited pipeline.
- **F2 — where a DataAsset's rows live at runtime.** Entity-shaped dense columns with generated row
  constants, vs a new resource region in the byte format. Blocks **G5**.
  ✅ **RESOLVED 2026-08-30 [delegated] — (a) rows are entities, AMENDED**: `StorageKind::Table` not
  dense, eager at load, own explicit load and pinned. ⚠ **Confirmed on CORRECTED grounds
  2026-09-03**; the original size argument is superseded — see the 2026-09-03 entry above.
- **F3 — the shape of a table file.** Single-file-per-asset only, vs also a table file baking N rows
  into ONE ARCHETYPE COLUMN — over ONE schema either way. Blocks the **grammar** (G5's authoring
  surface). ⚠ *This body used to read "into one dense column". F2's ruling refutes the phrase: a
  dense id is signature-excluded (`is_signature_storage` matches `StorageKind::Table` alone), so
  `get_or_create_archetype(&[DenseRow])` returns the EMPTY archetype and "dense-column archetype"
  denotes nothing in this engine.*
  ✅ **RESOLVED 2026-08-30 BY THE OWNER — both shapes, ONE schema.** A table file is a spelling, not
  a second type system. Ground: `feat/threadpool-ke16` `gaia/DECISIONS.md` §Data tables.
- **F4 — what "loaded" means, WIDENED** to every insert-path mechanism the load path misses (hooks,
  the `#[require]` closure, flag initial state, relation reverse indexes, asset refcounts). Blocks
  **G6** and the engine's load semantics generally; carries the G6 stable-asset-carrier addendum.
  ✅ **RESOLVED 2026-08-30 [delegated] — (a), as SUPPRESS-THEN-FIXUP: four ordered sub-passes**, with
  **three amendments recorded 2026-09-03** (sub-pass 4 splits into 4a relink / 4b hooks; the census
  is 5 mechanisms × 3 storage kinds; 4b reports per hook class). ⚠ Its subject MOVED: one of the
  five mechanisms was fixed on `feat/threadpool-ke16` and another is still live on both branches —
  see the 2026-09-03 entry above.
- **F5 — streaming scope.** Catalog now / loader later, vs the whole streaming half inside G6. The
  second option had never been written down. Blocks **G6**.
  ✅ **RESOLVED 2026-08-30 BY THE OWNER — option (b), EVERYTHING FROM THE START.** *"Все сразу
  грамотно по списку с самого начала."* ⚠ **This register printed the recommendation as (a) — the
  option he rejected — until 2026-09-03.** G6's scope is rewritten accordingly. Ground:
  `feat/threadpool-ke16` `gaia/DECISIONS.md` §Streaming scope.
- **F6 — the fate of `.ui`.** Migrate and delete in the same campaign, vs freeze until an owner-eval
  of a real Gaia HUD. Blocks **G7**.
  ✅ **RESOLVED 2026-08-30 BY THE OWNER — (a), migrate and delete in the same campaign.** Ground:
  `feat/threadpool-ke16` `gaia/DECISIONS.md` §Language shape.
- **F7 — mods.** ⚠ Borders the ratified reflection-only-at-bake refusal; must be framed against it
  rather than asked fresh. Blocks nothing today; silence is itself a decision.
  ✅ **RESOLVED 2026-08-30 BY THE OWNER — mods are NOT supported for now, ratified explicitly**, and
  the reflection constraint is recorded in its narrow form (no reflection in the game binary).
  Ground: `feat/threadpool-ke16` `gaia/DECISIONS.md` §Refusals; the ratified line in this branch's
  [`gaia/DECISIONS.md`](gaia/DECISIONS.md) §Refusals now carries that date.
  ⚠ *Merge note, 2026-09-10 (`merge/ke16-into-render`): the `Ground:` lines in F1–F7 above name
  "`feat/threadpool-ke16` `gaia/DECISIONS.md` §…" because they were written on 2026-09-03, when
  that ground sat on another branch. The merge landed it — every one of those sections is now in
  this tree's [`gaia/DECISIONS.md`](gaia/DECISIONS.md), under the same §. That branch's own F1–F7
  bodies stated the same rulings with the local link and nothing these do not say, so they are
  reduced to this note — except the two `Ground:` pointers only it carried, kept here: **F2** →
  [`gaia/DECISIONS.md`](gaia/DECISIONS.md) §Data tables, and **F4** →
  [`gaia/DECISIONS.md`](gaia/DECISIONS.md) §Load semantics.*
- **F8 — name-vs-id for objects.** Does an authored object carry a human NAME that bakes to an id,
  or a minted ID with the name as commentary? ⚠ **REOPENS the ratified `gaia fmt --assign-ids`
  identity ruling** — which was written before the third reference kind (assets, GB-3) surfaced.
  AIR-10's bar applies: a reopen states its TRIGGER and its MEASUREMENT, it does not re-argue taste.
  Blocks: **TBD — owner scoping** (G3 is the candidate).
  ⚠ **STILL OPEN, and deliberately untouched by every ruling of 2026-08-30.** GB-3's Part 4 and F4's
  carrier addendum are both **F8-neutral by construction** and say so at their own sites: a *carrier
  form* (a stable name used as a key) is not a *name-vs-id ruling*.
  *RECOMMENDATION, not a ruling (2026-09-03):* the object NAME is **not** the id — the ratified
  file-local id slot stands and the reopen never met AIR-10's own bar, because it stated its trigger
  (the third reference kind) and never its measurement; GB-3's ruling then made asset references a
  position-independent sigil and declares its F8-neutrality at its own site, so the trigger does not
  reach object identity. Offered as a veto line: disagreeing reopens the whole grammar rung, so it is
  cheaper to say so now than at G2.
  *(Merge note, 2026-09-10: `feat/threadpool-ke16` carried these same three sentences with
  different line breaks and without the 2026-09-03 recommendation; reduced to this line.)*
- **F9 — the flags carrier.** Where a document's `flag` lands in the byte format: a carrier of its
  own, or a bake-time refusal with a `GA####` code telling the author to spell it as a component.
  Blocks **G2/G6**.
  ⚠ **PARTLY RESOLVED 2026-08-30 [delegated] — three eliminations ruled, the residual escalated
  back.** RULED: (1) `FLAGS_DIRECT` must **not** be fired on the load path — it would overwrite every
  saved bit with its attach-time value; (2) the refusal option **may not be chosen**, because it
  contradicts ratified **AB-6** (*a language may not refuse what the derive accepts*); (3) the
  spelling, if a carrier lands, is `flags (X = true)` shared verbatim with Aether, keyed on GK-1's
  global object id, riding F4's fixup pass. ⚠ **STILL THE OWNER'S:** may a document set a flag on an
  **individual authored object**? That is a VALUES call about the authoring surface; it blocks **G2**
  and nothing else.
  *RECOMMENDATION, not a ruling (2026-09-03):* no per-object `flags (…)` group in v1 — per-object
  authored state is a durable COMPONENT FIELD, and the flag stays owned by exactly one system that
  derives it (`Visibility` byte → `RenderEnabled` bit is the shipped, gated pattern). A flag bumps no
  change tick, so a document-set flag can race its owning system with no observable. Offered as a
  veto line: a level FILE outranking a SYSTEM over a runtime bit is a legitimate values position, and
  option (A) is fully priced (header 80→96 B, version 2→3, 5 016 B for 10 000 entities × 4 tags).
  Ground and the full price of a carrier:
  [`gaia/DECISIONS.md`](gaia/DECISIONS.md) §Flags in the byte format.
  *(Merge note, 2026-09-10: the pointer on the two lines above is `feat/threadpool-ke16`'s and is
  kept; the rest of its version of this item is the same three eliminations, differently wrapped.)*
- **F10 — bundles: expand or refuse.** Does the baker expand a bundle into its components at bake
  time, or refuse a bundle in a document and demand the components? **Pre-question: is this a ballot
  at all** — PENDING's Tier 3 and its Part D contradict each other on whether it was already
  decided. Blocks: **TBD — owner scoping**.
  ⚠ **STILL OPEN, and deliberately untouched.** Every ruling of 2026-08-30 that could have brushed it
  — GB-1's depth rule, GB-3's Part 4, GB-4's instance form — states its F10-neutrality at its own
  site rather than settling it by proximity. ⚠ **The pre-question is UNANSWERABLE BY READING:** both
  sides of the alleged contradiction were rewritten into mutual notices of it, neither now states a
  POSITION, and the original text was never in git — `git log --all -- docs/gaia/PENDING-SYNTAX-PLAN.md`
  returns the commit that ADDED the file in its present form and one later touch. It can only be
  settled by ruling.
  *RECOMMENDATION, not a ruling (2026-09-03):* not a ballot — an architecture fork, decided by F4's
  own ratified provenance criterion (a datum that is a function of the CODE is re-read live and never
  frozen into the artifact). A bundle is a code fact, so a document MAY name one and it is expanded
  from the LIVE derive-emitted manifest on the load path, never at bake — a bake-time expansion is
  the version-skew class F4 rejected, where a level baked before `Pawn` gained a member loads without
  it forever, silently. The refuse-arm survives narrowed to the K8 shape, with a `GA####` code at G2.
  *(Merge note, 2026-09-10: `feat/threadpool-ke16` carried the first three sentences above with
  different line breaks and without the pre-question finding or the recommendation; reduced to
  this line.)*

### Gaia — the `GB` series (born in this review)

- **GB-1** — which layer of the priority ladder a template EXPANSION and an UNLABELED write occupy.
  Blocks **G4**.
  ✅ **RESOLVED 2026-08-30 by STANDING RULE — and the owner resolved it by sending it back**, which is
  the opposite failure from the one this list hunts: an architecture fork over-escalated. **Neither
  case gets a ladder layer**, because the ladder is the wrong axis: a template expansion is one step
  on the inheritance-DEPTH axis, ranked exactly where `extends` ranks, and resolution is depth first,
  then ladder; an UNLABELED write occupies `base`. The ratified ladder is untouched, and finding K2
  is dissolved rather than diagnosed. Ground: `feat/threadpool-ke16` `gaia/DECISIONS.md`
  §Layers and depth.

  > *Body kept by the merge `merge/ke16-into-render`, 2026-09-10, from `feat/threadpool-ke16` — the record as it stood when the ruling was taken. Where it says a ballot is STILL OPEN, the disposition line above it is the later state.*
  >
  > **RESOLVED 2026-08-30 — decided by standing rule** (`CLAUDE.md`: perf and architecture forks are
  > decided without asking; only VALUES and SCOPE go to the owner). This ballot was mis-routed here;
  > the owner re-routed it back. **Neither case gets a ladder layer, because the ladder is the wrong
  > axis for one of them.**
  >
  > **The ratified ordering, quoted exactly** ([`gaia/DECISIONS.md`](gaia/DECISIONS.md) §The logic
  > line): *"defaults + a **closed priority ladder** `base < variant < tuning < debug` (named layers,
  > never free numbers — the `!important` race; two writes of one field on one layer = error with
  > both files; bake is order-independent and byte-identical in any file order)"*. Nothing below
  > disturbs it: no rung is added, removed, renamed or reordered.
  >
  > **(a) A template EXPANSION occupies no ladder layer. It is one step on the INHERITANCE-DEPTH
  > axis, which is already ratified and already separate.** The corpus carries depth as its own
  > mechanism in three places — *"Single parent, no diamonds"*, *"Variant chains: a patch target
  > resolves against the **immediate** base — one sentence plus one pinned test"*, and the bake
  > budget over *"expansion steps, spawned fields, **template depth**, output bytes"*. A template
  > application ranks exactly where `extends` ranks: below the applying node's own writes, at the
  > same layer. Resolution order is **depth first, then ladder**: within one layer a node's own write
  > beats what it expanded; across layers the flattened node from the lower layer loses field-wise to
  > the higher one.
  >
  > **The decisive ground is that the alternative is not expressible.** The rejected option is to
  > mint a rung for expansion (`template < base < …`). Its price is not taste: **`template depth` is
  > a BUDGETED quantity in the ratified text, and a budget bounds a number that is otherwise
  > unbounded.** A template that applies a template needs one rung per level; a closed four-name
  > enum cannot carry an unbounded count. The rung would also break the ladder's own closure, which
  > AIR-10's bar protects. So the option fails twice — on the ratified text and on arithmetic — and
  > this is why the ruling routes expansion to the axis that is already unbounded.
  >
  > **This dissolves K2 without new machinery.** K2 is *"template application double-writes a field
  > on one unnamed ladder rung"*: `apply Torch` plus a local `power=2.5` in the same node looked like
  > the ratified *two-writes-one-layer = error*, which would have made the commonest authoring act in
  > the language — apply a template, override one field — a bake error. Under the ruling they sit at
  > different DEPTHS, so it is an override, not a collision. The error keeps a real referent and
  > narrows to what is genuinely ambiguous: **same depth AND same layer.**
  >
  > **(b) An UNLABELED write occupies `base`.** Ground: `base` is the layer whose name already means
  > "the thing itself", and it is the ladder's bottom, so authored content stays overridable by every
  > pack above it. The rejected alternative is an implicit fifth "unlabeled" rung, and its price is
  > the exact race the ladder was ratified to prevent — an unranked write makes *"which write won"* a
  > computation over file order, which would make G4's own gate (`bake(files) == bake(shuffle(files))`
  > byte-identical) either red or, worse, vacuously green on a fixture that happens not to collide.
  >
  > **Recorded, NOT decided here — the consequence (b) buys.** In a non-`base` pack every line must
  > repeat `layer=tuning`, and one forgotten `layer=` lands silently at `base`: a silent wrong-layer
  > write, which is this repo's standing defect class. A file-level layer default is the obvious cure
  > and is a **grammar** question belonging to **G4**, not to this ballot; naming it here so it is not
  > lost, and deliberately not ruling it.
  >
  > **Spelling-independent, and deliberately so.** The ruling is about precedence, not words. It
  > holds under either option of **GB-4** (`instance … extends|copy` in slot vs in head) and under
  > whatever anchor the template-application fix lands on (PENDING Tier 1 proposes `apply Torch:`).
  > It is scoped to **template** expansion, the construct GB-1 names. Whether a **bundle** expands at
  > bake at all is **F10**, which is the owner's and is untouched; if F10 rules "expand", the depth
  > rule above applies to it unchanged, and that is a consequence, not a pre-decision.
  >
  > **Riding lines moved in the same edit:** [`gaia/DECISIONS.md`](gaia/DECISIONS.md) §The logic line
  > and §Inheritance / variants, [`gaia/PENDING-SYNTAX-PLAN.md`](gaia/PENDING-SYNTAX-PLAN.md)
  > Tier 1 (template use), Part D and Part E (K2),
  > [`AETHER-GAIA-REVISION-2026-08-29.md`](AETHER-GAIA-REVISION-2026-08-29.md) §Work order,
  > and [`ru/OPEN-QUESTIONS.md`](ru/OPEN-QUESTIONS.md).

- **GB-2** — the colour transfer function: `#RRGGBBAA` sRGB-decoded at bake, vs raw bytes carried
  into a LINEAR field. Blocks **G2/G5**.
  ✅ **RESOLVED 2026-08-30 BY THE OWNER — `#RRGGBB` is `#RRGGBBAA` with maximal `AA`**: a DEFAULTED
  FIELD, not a second literal kind, which dissolves the reopen rather than answering it. Plus the
  arity direction the original clause never covered: an 8-digit literal at a 3-component field
  (`PointLight::color` is `[f32; 3]`) is a **coded bake refusal naming the field**, never a silent
  alpha drop.

  > *Body kept by the merge `merge/ke16-into-render`, 2026-09-10, from `feat/threadpool-ke16` — the record as it stood when the ruling was taken. Where it says a ballot is STILL OPEN, the disposition line above it is the later state.*
  >
  > **RESOLVED 2026-08-30 — decided by standing rule.** This is a correctness question with a
  > measured answer, and the answer is **neither of the two options as posed**: both assume ONE rule
  > for the literal. **The transfer function is a property of the DESTINATION FIELD, not of the
  > literal**, and it is declared in the GK-4 field table.
  >
  > **What is true at source.** Every colour-typed field in the workspace was read, not inferred.
  > There are three carrier classes and they disagree with each other:
  >
  > | carrier | fields | what the source says |
  > |---|---|---|
  > | LINEAR float | `PointLight::color` (`boyko_render/src/light.rs:309-310`), `DirectionalLight::color` (`:289-290`), `SpotLight::color` (`:336-337`), `SkyLight::sky_color`/`ground_color` (`:357-360`), `MaterialGpu::base_color`/`emissive` (`material.rs:51,54,64-69`) | `/// LINEAR rgb color`; `light.rs:114` *"All radiometric values are LINEAR"*; `material.rs:56` *"All values are LINEAR"* |
  > | 8-bit encoded RGBA | `UiBackground::color`/`border_color` (`boyko_ui/src/components.rs:211,220-223`), `UiText::color` (`text/components.rs:39-47`) | `u32`, *"authored STRAIGHT RGBA8 (`byte0=R .. byte3=A`)"* — **"straight" here means NON-PREMULTIPLIED, not "not decoded"**: `boyko_ui/src/components.rs:211` continues *"the pack system premultiplies them"*, and `premultiply_rgba8` (`boyko_render/src/ui/instance.rs:330`) is the operation named. No sRGB decode exists anywhere on the UI path — `ui_rect.fs.hlsl:252` unpacks the byte word and composites it directly |
  > | device-encoded packed | `ParticleEffect::color_keys: [u32; 4]` (`boyko_render/src/particle_effect.rs:120-143`) | byte order is **`0xAABBGGRR`**, *"the opposite of the `0xRRGGBBAA` an author reaches for by habit"* — and its own doc records that both in-tree presets were authored wrong exactly that way, *"and neither was caught by anything"* |
  >
  > That table settles the ratified-text worry before it starts: the ratified *"`#RRGGBBAA` →
  > straight-RGBA8"* line ([`gaia/DECISIONS.md`](gaia/DECISIONS.md) §Language shape) is a statement
  > about **alpha association**, not about a transfer function. It is not being reopened; it is being
  > read against the field whose doc comment defines the word.
  >
  > **The ruling.**
  > 1. **Destination is a LINEAR float colour field → the sRGB EOTF is applied at bake.** The
  >    authored byte is display-referred; the field is scene-referred; a converter that skips the
  >    conversion is simply wrong.
  > 2. **Destination is an 8-bit encoded RGBA carrier → the transfer function is the IDENTITY.** Not
  >    a special case bolted on: source space and destination space are the same space, so the
  >    general rule yields the identity here. **Measured:** decode-then-re-encode over the whole
  >    domain drifts **0 of 256 bytes**, so stating the rule uniformly changes no existing UI colour.
  > 3. **Destination is a device-encoded packed carrier → a coded `GA####` bake refusal**, naming the
  >    field's own byte order. Gaia must not be the fourth way to author `0xAABBGGRR` by habit.
  >
  > **Rejected alternative, and its price — measured, not asserted.** Carrying raw bytes into a
  > LINEAR field (the ballot's option (b), and what
  > [`gaia/LANGUAGE.md`](gaia/LANGUAGE.md) does today) is wrong by up to **12.92×** multiplicatively
  > (byte 3: `0.011765` raw vs `0.000911` decoded) and **0.287** absolutely (byte 136). On the
  > corpus's own literal `#FFB35CFF` into `PointLight::color` it is **1.557× on green** (`0.702` vs
  > `0.451`) and **3.371× on blue** (`0.361` vs `0.107`) — a large hue shift toward desaturation,
  > not a brightness nudge, and nothing downstream attributes it to the baker. The mirror-image
  > alternative — one global "always decode" rule — is wrong in the other direction: applied to a
  > `u32` field by storing the decoded value it corrupts every UI colour, which is why the rule is
  > keyed on the destination rather than on the literal.
  >
  > **⚠ A gate constraint that falls out of the same measurement, and must be obeyed by G2's
  > fixtures: the two routes have exactly TWO fixed points, byte 0 and byte 255.** A red fixture
  > written with `#FFFFFFFF` or `#000000FF` is satisfied identically by both routes — **a gate that
  > cannot fail**, the class this corpus exists to repair. Every colour fixture must use a
  > mid-domain channel; `128` (raw `0.501961` vs decoded `0.215861`) is the recommended witness.
  >
  > **The ARITY half — settled here, and it needed settling because nothing carried it.**
  > `LANGUAGE.md:71-74` flags a 4-component `#RRGGBBAA` written into a 3-component `[f32; 3]` and
  > records that *"the arity half is carried by nothing"*. **The precedent is already shipped in this
  > repo**: Aether's `ColorLit` (`crates/aether_lang/src/parse.rs:1177-1213`) checks arity against
  > the TARGET's arity — 3 or 4 for `base`, exactly 3 for `emissive` — synthesizes the missing alpha
  > as `1.0` when widening (`expand.rs:891-898`), refuses when narrowing with a message naming the
  > target type (*"`Material::new` takes `emissive: [f32; 3]`, emitted radiance has no alpha"*), and
  > lands the blame on the **tuple's own span**, *"because neither the key nor any single component is
  > the thing that is wrong"*. Gaia mirrors that rule verbatim: **widening 3 → 4 supplies alpha
  > `1.0`; narrowing 4 → 3 is a coded refusal, never a silent alpha drop** (a silent drop is Unity's
  > class, which §Inheritance bans by name). Consequence: the 6-digit **`#RRGGBB`** form is admitted
  > as the 3-component spelling, and `LANGUAGE.md`'s `color=#FFB35CFF` on a `PointLight` is a bake
  > error whose correct form is `#FFB35C`.
  >
  > **Why admitting `#RRGGBB` is not a reopen.** §Language shape's literal list is introduced as
  > *"Literals are engine-native (…)"* — an illustrative list. The one list in this corpus that is
  > closed says so in as many words (§The logic line: *"the list is exhaustive"*), and this is not
  > that list. The alternative to admitting the 6-digit form is to make every light colour a float
  > tuple, which prices the commonest scene edit in the least readable form for no correctness gain.
  >
  > **Not touched:** whether `PointLight::position` is declarable at all is **GB-5**, the owner's,
  > and nothing above bears on it.
  >
  > **Riding lines moved in the same edit:** [`gaia/DECISIONS.md`](gaia/DECISIONS.md) §Language
  > shape, [`gaia/LANGUAGE.md`](gaia/LANGUAGE.md), [`gaia/PENDING-SYNTAX-PLAN.md`](gaia/PENDING-SYNTAX-PLAN.md)
  > Tier 1 (`PointLight` field names), Tier 4, Part D and Part E (K12),
  > [`AETHER-GAIA-REVISION-2026-08-29.md`](AETHER-GAIA-REVISION-2026-08-29.md) §Work order,
  > and [`ru/OPEN-QUESTIONS.md`](ru/OPEN-QUESTIONS.md).
- **GB-3** — the reference taxonomy needs a THIRD kind. Ratified §Identity has two (lexical/copy,
  entity identity via `@` with remap); asset references are neither. On one ballot because R4 should
  be rewritten once: the asset-ref spelling (sigil / typed head / bare strings — the bare-string
  option needs its own dangle-check answer), the style-reference kind, the `$hole` sigil
  disposition, and withdrawal of the "declared node ⇒ `@`" rule. ⚠ **amends ratified §Identity** —
  an extension consistent with its own one-spelling-one-behaviour rationale, not a reversal. Blocks
  **G2/G3** and PENDING Tiers 1-2.
  ✅ **RESOLVED 2026-08-30 — THIRD KIND ADOPTED BY THE OWNER; the four-part rewrite ruled
  [delegated].** (1) the spelling is a **SIGIL**, uniform and position-independent, so a bare word is
  never a reference — but ⚠ **the glyph is NOT minted**: it is a **G2 deliverable, closed jointly
  with Gaia's arithmetic operator vocabulary**, because R3 makes whitespace insignificant and a glyph
  that can open a binary operator is ambiguous (recommendation `~`). (2) style references are the
  FIRST kind, and the ballot's example list drops "a style record". (3) `$hole` STAYS, reclassified
  lexical in every position. (4) "declared node ⇒ `@`" is WITHDRAWN, replaced by *what does bake do
  with this reference?*. ⚠ The ballot's GN1 premise is refuted: GN1's first resolution is a load
  remap, so GN1 is the law over all three kinds. The ruling now sits in this branch's
  [`gaia/DECISIONS.md`](gaia/DECISIONS.md) §Identity and references, where the open ballot body used
  to sit inline. **Follow-on finding, unresolved and NOT a ballot:** the asset-ref dangle check is
  structurally unreachable downstream of the sigil pass — `AssetServer::load` logs `E0801` and
  returns a LIVE handle in the `Failed` state which it inserts into the path index, and
  `validate_asset_refs` early-returns unless `free_epoch` advanced, which a never-loaded asset never
  makes happen.

  > *Body kept by the merge `merge/ke16-into-render`, 2026-09-10, from `feat/threadpool-ke16` — the record as it stood when the ruling was taken. Where it says a ballot is STILL OPEN, the disposition line above it is the later state.*
  >
  > ✅ **RESOLVED 2026-08-30 — the THIRD KIND IS ADOPTED BY THE OWNER; the four-part rewrite ruled
  > the same day [delegated].** Full ruling: [`gaia/DECISIONS.md`](gaia/DECISIONS.md) §Identity and
  > references. Summary, with the measured ground and the rejected alternative at each part:
  >
  > **The kind is real, and the deciding column is one the other two do not have.** An asset
  > reference is the only kind whose referent can **stop being valid after load**, because streaming
  > retires slots — and under the owner's F5 ruling that is the normal case. At source:
  > `PathIndex::lookup(hash: u64) -> Option<(u32, u32)>` = `(slot, generation)`
  > (`crates/boyko_ecs/src/ecs/core/asset/path_index.rs:112`), then refcounted and revalidated every
  > frame by `validate_asset_refs` against the `MeshRefGen`/`MaterialRefGen` lanes. Both collapses
  > are refuted by that: there is no `Entity` and no load-map row (and `PathIndex` is globally
  > interned, first-insert-wins, **not** per-load), and a lexical reference does not survive into the
  > binary at all while this one must. ⚠ It is also the **only reference kind that already works
  > across a cell boundary** — two cells referencing one path share `(slot, generation)` and the
  > refcount keeps it alive while either holds it — so folding assets into `@` would give the
  > working kind the lifetime of the one F5 just made hard.
  >
  > **Part 1 — spelling: (a) A SIGIL**, uniform and position-independent, buying the invariant *a
  > bare word is never a reference*. **Rejected (c) bare strings**, on this ballot's own ratified
  > rationale: a string literal is already a **non-reference value** in this language, so (c)
  > collides kind 3 with plain values, not merely with another reference kind — and its dangle check
  > needs **three** mechanisms (field table, head grammar, closed valve signature) because asset
  > references occur at positions with no destination field, which is three places the check can be
  > structurally unreachable. **Rejected (b) a typed head**, on the measured ground that already
  > killed widget sugar (*the vocabulary is unclosable*) and on R7's requirement that the operation
  > list be closed and generated. ⚠ **The glyph is NOT minted here, and that is a finding.** Measured
  > over Gaia code fences (44 fenced lines, 4 files): `/` 21, `@` 10, `:` 4, `#` 3, `|` 2, `-` 2,
  > `$` 1, `;` 1, `+` 1, `>` 1; absent `!  %  &  *  <  ?  \  ^  ` ~`. Excluded: `@`/`#`/`$` (taken),
  > `&` (ratified-rejected, and reads as *borrow* for an owned refcounted handle), `?` (reads as
  > fallible, which contradicts §Refusals' *no construct with a fallback*), backtick
  > (measured-hostile in this toolchain), **and any glyph that can open a binary operator** — R3
  > makes whitespace insignificant, so `(a %b)` and `(a % b)` must parse identically and `%` is what
  > the ratified `for … if` guard wants. **Therefore the sigil cannot be chosen before Gaia's
  > arithmetic operator vocabulary is closed; the two are one question and G2 closes them together.**
  > Recommendation `~`. The dangle check is named in the ruling: one lexical pass over sigil-marked
  > tokens, a coded `GA####` refusal naming pack and path, blamed on the reference's own span.
  >
  > **Part 2 — style references are the FIRST kind (lexical/copy).** §Refusals already ratifies *no
  > cascade — styles are flattened by bake*, and a thing flattened by bake leaves no reference in the
  > binary. ⚠ **A contradiction inside the ballot's own text is resolved rather than carried
  > forward:** it listed *"a style record"* among the asset kind's examples, against §Refusals on the
  > same page. §Refusals is ratified and the parenthetical was not; the example list **drops** it.
  >
  > **Part 3 — `$hole` STAYS, and the ballot's premise is false.** It is not a third *kind*: §Identity
  > already names templates and `let` as the lexical kind, so `$power` was ratified all along. What
  > `$` does is stop a real ambiguity: R6's case-gate was refuted (M12), and at value position the
  > grammar admits bare lowercase words that are **not** references (RON-style enums, the ladder
  > names, `rarity=common`), so a template parameter named `common` would silently flip
  > `rarity=common` from an enum value into a substitution. Ruled: `$` marks the lexical kind **in
  > every position**, including `extends $sword_base` and `apply $Torch`, which is what lets a
  > generator pick the sigil from the kind alone. Price: one character per reference.
  >
  > **Part 4 — "declared node ⇒ `@`" is WITHDRAWN**, replaced by *what does bake do with this
  > reference?* — substitutes it (`$`), records an object id for remap (`@`), or records a path hash
  > for lookup + refcount (the new glyph). The corpus already shows the withdrawn rule's damage:
  > `LANGUAGE.md` spells a **lexical** inheritance base as `extends @iron_sword`.
  >
  > **The ballot's second item is REFUTED, not answered.** It asserted *"GN1 resolves every reference
  > form … WITHOUT remap. That is the asset kind's behaviour."* GN1's own four resolutions open with
  > *"object id → `Entity` (**load remap**)"*, so GN1 spans a remapping resolution. GN1 is the uniform
  > law over all three kinds; **only kind 3 is lookup-without-remap.**
  >
  > ⚠ **Two findings recorded, neither ruled here** (each is a different ballot's or rung's):
  > (1) **`@asset/object` is ambiguous under the `/` separator** — every asset path in a Gaia fence
  > has ≥2 slash-separated segments (**8 of 8**), so `@a/b/c` cannot be split without an out-of-band
  > rule; the one ratified example reads unambiguously only because it uses a 1-segment asset name, a
  > shape occurring nowhere else. R4's amendment already flags the `/`-vs-`#` drift; the measurement
  > says only `#` survives contact. (2) **The asset-ref target registry does not exist** —
  > `grep -rnE "get_by_name|by_path|load_path|handle_for_name|name_to_handle"` over the asset and
  > render crates returns **0**. That is a build item for **G3**, not a spelling item.
  >
  > **Riding lines this ruling OWES and does not make:** PENDING R4/R5/R7 and Tiers 1–2 are
  > regenerated by it, and `LANGUAGE.md` still carries `extends @iron_sword` and bare-string asset
  > refs. Both files are already marked — PENDING as owner-gated, `LANGUAGE.md` as `ratified-stale`
  > in its own head — so the divergence is *marked*, not silent. The edits belong to **G2**.
- **GB-4** — instance re-spelling: (a) linkage-in-slot (`instance wall_east extends|copy "…"`) vs
  (b) linkage-in-head (`instance` ≡ live link, `copy` its own head). `from` is deleted either way —
  it was minted by that one line and appears in no ratified vocabulary. Blocks **G4** / Tier 2.
  ✅ **RESOLVED 2026-08-30 [delegated] — (a) linkage-in-slot, and THE LINKAGE WORD IS MANDATORY**:
  `instance <name> extends|copy <base-ref>`, no default. (b) was rejected on four grounds, the
  sharpest being that it has NO refusal for the OMITTED case while (a) does — and on the generator
  axis (b)'s omission error is the invisible one. Price, stated: one mandatory word per instance
  line. ⚠ Rider owed to G2: the corpus contains an UNDECLARED `from=` → `source=` rename on `bind`.

  > *Body kept by the merge `merge/ke16-into-render`, 2026-09-10, from `feat/threadpool-ke16` — the record as it stood when the ruling was taken. Where it says a ballot is STILL OPEN, the disposition line above it is the later state.*
  >
  > ✅ **RESOLVED 2026-08-30 [delegated] — (a) linkage-in-slot, with the linkage word MANDATORY.**
  > Full ruling: [`gaia/DECISIONS.md`](gaia/DECISIONS.md) §The instance spelling.
  > `instance <name> extends|copy <base-ref>`, no default linkage, `from` deleted.
  >
  > ⚠ **How the owner's word was read, said plainly rather than assumed.** He answered *"давай"* —
  > **delegation, not a selection**: he named no option, and the ballot text carries no recommendation
  > for the word to point at. Nothing in the evidence makes one option obviously his intent either,
  > so it is decided under the standing perf/architecture rule and recorded as the orchestrator's
  > call, not attributed to him.
  >
  > **Four grounds.** (1) **(b) mints two grammars for one ratified construct** — this corpus already
  > ratifies *"Instance = variant = derived asset = one construct"*, and **AIR-15** rejects
  > *"two grammars per construct — the `at` lesson"* by name, on the AI-orientation axis this ballot
  > was to be judged on. (2) **(b) turns a modifier change into an anchor mutation**: under (a),
  > "make `wall_east` a snapshot" rewrites one token in a fixed slot and every patch/diff/tool edit
  > keyed on *the `instance` node named `wall_east`* still resolves; under (b) it rewrites the
  > **head**, which is neither a key nor a value and so lies outside **AIR-09**'s ratified
  > `{node_id, key, value}` patch model. (3) **(b) has no refusal for the omitted case; (a) does** —
  > `instance …` alone is legal under (b) and means *live*, so forgetting the word yields a silent
  > wrong-linkage node, the identical consequence the ladder ruling already bought once for a
  > forgotten `layer=`; and defaulting is unavailable on the section's own ground, since RimWorld and
  > Factorio each shipped one form and their ecosystems invented the other. (4) **The two axes stay
  > orthogonal**: the data profile already spells a linkage word in a slot on `row`, so under (b)
  > `copy` is a *head* in one profile while `extends` is a *slot word* in the other — the "one word,
  > N positions" class the syntax plan catalogues against Aether — and R7's closed operation list
  > grows per construct × linkage under (b) versus by two entries under (a).
  >
  > **Rejected, with its price stated rather than dismissed.** (b) is genuinely cheaper to read in the
  > common case and maximally loud on scan; **(a)'s price is one mandatory word on every instance
  > line.** The trade is taken because on the generator axis (a) has **no silent failure** — both
  > wrong answers are visible, the token is there and it is the other one — while (b) has exactly
  > one, and it is the omission case, the commonest generator error.
  >
  > **`from` — measured, and the qualification strengthens the deletion.** A raw grep counts prose, so
  > the measurement is over **code fences only**: extract fenced blocks from `docs/gaia/*.md` and
  > match `\bfrom\b` → **1 hit in 44 fenced lines across 4 files**, and it is
  > `bind value from=@player/unit …` — `from=` as a **field key**, not the head-position `from` being
  > deleted. Head-`from`'s count is therefore **zero**, and `grep -rn '"from"' crates/aether_lang/src/`
  > exits 1, so it is not an Aether keyword either. ⚠ **Rider:** PENDING's own R1 amendment spells
  > that construct `bind: value source=…`, so the corpus contains an undeclared `from=` → `source=`
  > rename. GB-4 deletes head-`from`; the surviving role must not be left half-renamed. That edit is
  > **owed to G2**.
- **GB-5** — may a scene document declare an ENGINE-DERIVED field (`PointLight.position`)? The `ui`
  profile already refuses this; the ballot is extending the rule to `scene` as a DECISIONS line
  BEFORE G1 freezes the GK-4 field table, which then marks such fields. Blocks the **G1 table
  freeze**.
  ✅ **RESOLVED 2026-08-30 BY THE OWNER (recorded 2026-08-31, `b6c41237`) — PERMIT AS SEED.** *"the
  third is the most logical — it is simply a starting point in space; obviously this data exists to
  be manipulated and will not be static."* Required form: the field-table disposition column MUST NOT
  be a boolean — all twelve measured members are CONDITIONALLY derived, so the column records the
  CONDITION and the WRITER. His acceptance condition (*"if the marking costs nothing at runtime and
  is purely for convenience, then yes"*) is DISCHARGED BY CONSTRUCTION: the column is born behind the
  same default-off bake feature and never reaches the game binary, so this does NOT return to him.
  ⚠ **This is the clearest case of a register losing an answer the owner gave** — see the 2026-09-03
  entry above for the mechanism and for the four registers on `feat/threadpool-ke16` that still say
  it is open. **G1's table freeze is UNBLOCKED.**

  > *Body kept by the merge `merge/ke16-into-render`, 2026-09-10, from `feat/threadpool-ke16` — the record as it stood when the ruling was taken. Where it says a ballot is STILL OPEN, the disposition line above it is the later state.*
  >
  > ⚠ **HEADER NOTE, 2026-09-10 (`merge/ke16-into-render`) — nothing in this block is a live status.** Every sentence in it that calls the ballot open is struck in place and dated. GB-5 was RULED BY THE OWNER on 2026-08-30 — PERMIT AS SEED, recorded 2026-08-31 by `b6c41237`; ground at `docs/gaia/DECISIONS.md:1262` and the who-table row `docs/OPEN-QUESTIONS.md:1031`.
  >
  > ⚠ ~~STILL OPEN — 2026-08-30. The owner asked for the TRADE-OFFS, not a ruling, and a ruling
  > written here would be a defect.~~ What follows is the analysis, written so he can rule from one
  > reading. Nothing in it is a decision.
  >
  > **1. The ground at source.** `light_reconcile`
  > (`crates/boyko_render/src/light_reconcile.rs:105-125`) takes
  > `Query<(&GlobalTransform, Mut<PointLight>), Changed<GlobalTransform>>` and writes
  > `l.position = g.translation()` behind a bit-gate. `PointLight` carries `position`, `color`,
  > `power`, `range` and is `#[require(Transform, GlobalTransform)]`; its own doc says
  > `light_reconcile` *"derives `position` from the `GlobalTransform` translation when one is
  > present."*
  >
  > **⚠ What happens today is not one answer but TWO, and both are pinned by green committed tests.**
  > Run live (`cargo test -p boyko-render --test render_upload_s4`, `running 2 tests`, `EXIT=0`):
  >
  > | test (`crates/boyko_render/tests/render_upload_s4.rs`) | result | what it pins |
  > |---|---|---|
  > | `point_light_position_tracks_global_translation` (`:261`) | ok | authored `position` is **overwritten** by the transform translation — **the overwrite is a contract** |
  > | `light_without_global_transform_is_untouched` (`:336`) | ok | *"the authored pose is exactly preserved"* — **non-overwrite is equally a contract** |
  >
  > The concrete failure the ballot is about: an author writes `PointLight.position = (3,5,2)` and
  > omits `Transform`. `#[require]` supplies the defaults, the first frame's `Changed<GlobalTransform>`
  > matches, and the light silently renders **at the origin**. Nothing logs; the authored value
  > survives zero frames. ⚠ **One seam decides whether that reproduces for a BAKED scene, and it is
  > an engineering fork, not the owner's:** F4(ii) measured that the load path adds no component the
  > file omitted, so whether a baked `PointLight` is overwritten depends on whether the **baker's**
  > world construction fires `#[require]` — which is G1's own unwritten design. Flagged so it is not
  > settled by accident.
  >
  > **2. The `ui` profile does NOT already refuse this — measured, and this weakens the ballot's
  > premise three ways.** The rule exists at exactly one site,
  > [`gaia/LANGUAGE.md`](gaia/LANGUAGE.md): *"Engine outputs (`ComputedRect`, `Interaction`, bitset
  > tags) are undeclarable — bake error."* (i) **It is not in the decision log** —
  > `grep -rn "ComputedRect\|Interaction" docs/gaia/DECISIONS.md docs/gaia/CAMPAIGN.md
  > docs/gaia/PENDING-SYNTAX-PLAN.md` → **zero hits** — and `LANGUAGE.md` is the corpus's standing
  > example of a file that resolves but is stale. (ii) **It gives no reasoning**: one sentence, three
  > examples, no ground, so there is nothing to extend *by argument*, only by repetition. (iii)
  > **All three named examples are WHOLE components; `PointLight.position` is a FIELD inside an
  > author-owned one** — refusing a component the author never wanted costs nothing, refusing one
  > field of a component whose other three they must write is a different trade. ⚠ And a name-shaped
  > reading of that rule is already wrong in-tree: `ComputedClip` is documented *"AUTHOR-OWNED in P1
  > (not computed)"* and `StackIndex` *"AUTHOR-OWNED in P1"* (`crates/boyko_ui/src/components.rs`),
  > so a rule keyed on the word "Computed" forbids two author-owned components on day one.
  >
  > **3. The class, enumerated — and it is not one field.** Two comment-proof commands over the four
  > scene-relevant crates (`boyko_scene`, `boyko_render`, `boyko_ui`, `boyko_physics`), re-run
  > 2026-08-30:
  > `grep -rEn "[ (,]Mut<[A-Z]|Query<[^>]*&mut [A-Z]" … | grep -vE ":[0-9]+:[[:space:]]*(//|/\*|\*)"`
  > → **17**; `grep -rEn "get_component_mut::<[A-Z]" … | grep -vE …` → **12**.
  > ⚠ **These two are a FLOOR, not a census**: they are different mechanisms (`ComputedRect` and
  > `Interaction` are written only through `get_component_mut`, so the first command misses them
  > entirely), and reading turned up a **third** — a `Command::apply` body, `SyncRefGenCommand`. No
  > single grep produces this class.
  >
  > **Shape A — whole engine-owned components (what the `ui` clause actually names): 14.**
  > `GlobalTransform` · `ComputedRect` · `Interaction` · `RelativeCursorPosition` ·
  > `UiWorldProjection` · `UiWorldCulled` / `UiWorldHidden` / `UiWorldOccluded` (three separate
  > owning authorities) · `InstanceModelCol` · `PrevInstanceModelCol` · `GpuTransform3D` ·
  > `Gpu3dInstance` · `MeshRefGen` · `MaterialRefGen`.
  >
  > **Shape B — engine-derived FIELDS inside author-owned components. This is GB-5's actual subject:
  > 12 members over 8 components, and EVERY ONE IS CONDITIONAL.**
  >
  > | field | overwritten iff | writer |
  > |---|---|---|
  > | `PointLight.position` | the entity has `GlobalTransform` | `light_reconcile` |
  > | `SpotLight.position`, `.direction` | same | `light_reconcile` |
  > | `DirectionalLight.direction` | same | `light_reconcile` |
  > | `Transform.translation`, `.rotation` (`.scale` **never**) | `Simulated` ON ∧ `inv_mass != 0` ∧ no `ChildOf` | `boyko_physics` `scene_sync.rs` |
  > | `Transform` (whole) | the entity has `OrbitCamera` | `boyko_scene` `camera.rs` |
  > | `Transform` (whole) | the entity has `FlyCamera` | `boyko_scene` `camera.rs` |
  > | `RigidBody.position`, `.rotation` | **opposite polarity**: `Simulated` OFF ∧ `inv_mass == 0` | `scene_sync.rs` |
  > | `ContentSize.width` / `.height` | the node has `UiText`+`UiTextBuffer` ∧ a font is loaded | `boyko_ui` `text/measure.rs` |
  > | `UiLayout.width` **or** `.height` (axis picked at runtime from the track's `LayoutType`) | the node is a `Bar`'s fill child | `boyko_ui` `widgets.rs` |
  > | `UiValue.0` | a `BindValue` record targets it | `binding/bind_system.rs` |
  > | `UiTextBuffer` | a `BindText` record targets it | `binding/bind_system.rs` |
  >
  > **The finding that governs any ruling: "engine-derived" is not a property of a FIELD.** It is a
  > property of *(field × the entity's other components, and for physics and `Bar` × their runtime
  > values)*. Three consequences: **`Transform` is in the class**, so a blanket per-field rule applied
  > honestly refuses `Transform.translation` on every entity; **`Transform` and `RigidBody` are a
  > polarity pair** whose author-owned side flips on `Simulated`, a bitset bit gameplay toggles at
  > runtime, which bake cannot decide; and **both green tests above are correct**, so any rule saying
  > "`PointLight.position` is engine-derived, full stop" contradicts a test that passes today.
  >
  > **4. Why the timing matters, and the honest price is not the ballot's.**
  > `grep -rn "field_table\|FieldTable\|field_by_name" crates/boyko_macros/src crates/boyko_ecs/src`
  > → **empty**: G1 is unstarted, so "before the freeze" means *before the table's shape is designed*.
  > **Bytes are indifferent** — G1's gate is `bake_one(...) == save_world(...)` byte-for-byte, and a
  > refusal is a bake-time diagnostic that changes no bytes, so **lateness does not turn a gate red**.
  > The real prices: **cheap before** — one attribute at the component definition site, one column in
  > the derive-emitted table, zero consumers to update; **expensive after** — the disposition has to
  > live in a hand-maintained side list keyed by (component, field), an object this corpus has already
  > priced (*four hand-maintained lists of one vocabulary* as the single cause of `.ui`'s measured
  > defects, and a printer losing 10 of 19 components under a gate structurally blind to the loss),
  > plus reopening a landed G1 under AIR-10's bar.
  > ⚠ **The same measurement cuts the other way, and this is the strongest argument for deciding
  > CAREFULLY rather than EARLY:** a per-field marker in the frozen table is the shape the ballot
  > assumes, and it is **wrong for 12 of 12 measured members**, because not one is unconditional.
  > Freezing a per-field boolean early locks a predicate that is false for every case it covers —
  > worse than deciding late.
  >
  > **5. The options, each in its strongest form.**
  > - **(a) REFUSE — a bake error on any write to a marked field.** *Strongest case:* it kills the
  >   silent wrong answer at authoring time, this repo's standing preference, reaffirmed by the owner
  >   on this very campaign — GB-2's alpha-drop ruling, whose recorded ground is *a refusal that
  >   dissolves a class beats a documented sharp edge*. *Price:* it needs a decidable per-field
  >   predicate and the measurement says none exists; applied to `Transform.translation` it refuses
  >   the commonest write in the language, and applied only to lights it is a rule for four fields
  >   pretending to be a class.
  > - **(b) PERMIT — the field is writable; the engine discards it.** *Strongest case:* no new
  >   machinery, no over-refusal, and no wrong predicate frozen into the table; it also keeps Aether's
  >   own emitted authored scenes legal, since the engine already writes a light pose and lets
  >   `light_reconcile` re-derive it. *Price:* the light-at-the-origin failure, silent, in the language
  >   whose whole §Refusals section exists to make such things inexpressible.
  > - **(c) PERMIT-AS-SEED — authorable, documented as a seed, warned about at bake.** *Strongest
  >   case:* it says what is true, and it is the engine's own vocabulary already (*"`SpotLight::new`'s
  >   `direction` is only a SEED"*). *Price:* a warning is not a gate, and Gaia's ratified evaluator
  >   is **closed** — *anything bake cannot fully resolve is a bake error, never a fallback* — so a
  >   warning-only disposition is the first fallback in a language that has none.
  > - **(d) REFUSE CONDITIONALLY — refuse iff the deriving sibling is on the same flattened entity.**
  >   *Strongest case:* it is **the only option consistent with both green tests**, and it is decidable
  >   at bake for every presence-conditioned member (**7 of 12** — the four light fields, the two
  >   camera cases, `ContentSize`), because the baker knows the entity's final column set. *Price:*
  >   the predicate runs over the *flattened* entity, so it is a **G4**-time check while the write is a
  >   G2/G3-time act — the blame span is a write in file A while the disqualifying sibling arrives from
  >   a template in file B; and it is **undecidable for the 5 value-conditioned members** (physics
  >   `Simulated`/`inv_mass`, `UiLayout` under `Bar`), which must then take (b) or (c) anyway, giving
  >   the language two rules for one class.
  >
  > **Where the evidence points, and where it stops.** It points hard at **rejecting the
  > per-field-flag mechanism the ballot assumes** — that is a measurement, not a preference: 12 of 12
  > members are conditional. It does **not** pick between (b), (c) and (d); that is a values call
  > about how much authoring expressiveness a refusal may cost.
  >
  > **⇒ The question, in one sentence.** Given that every engine-derived field measured is
  > *conditionally* derived — including `Transform.translation`, which is engine-owned only for a
  > simulated dynamic body — should the scene profile **refuse** such a write when the deriving
  > sibling is present on the same baked entity (accepting a post-composition blame span and a second
  > rule for the five cases bake cannot decide), or **permit** it and document the discard?
- **GB-6** — §Relations carries two unclassified "ratify" imperatives: the scene-form fork (blocks
  **G6** if it is a ballot; if delegated, it must be relabelled "decided, owner may veto"), and the
  EnableTag-toggling visibility valve, which carries a DEADLINE ("before the first designer asks")
  and therefore needs an F-id and a rung on the Aether ladder — **a deadline with no rung can never
  come due**. Recorded beside it: the two authored-scene emission fixes ride no rung at all.
  ✅ **DISPOSED 2026-09-03 BY THE OWNER, both halves — no longer an open ballot.** (a) The
  scene-form fork is **RULED**, and the line is decided rather than relabelled: the Gaia TEXT is the
  source the editor saves and the BINARY is the compiled artifact that ships. The alternative the
  ballot could not name — shipping a scene as TEXT and parsing it at runtime (Godot's `.tscn`
  shape) — is declined, so the four downstream reassignments filed against the line STAND.
  (b) The valve is **not a Gaia ballot at all** and MOVES to **Aether R3**: Gaia is data, the logic
  that decides the token is Rust or Aether, and the only missing piece is an Aether construct that
  toggles a flag. ⚠ The misfiling this ballot carried was visible rather than newly discovered:
  [`AETHER-GAIA-REVISION-2026-08-29.md`](AETHER-GAIA-REVISION-2026-08-29.md) files GB-6 against
  **G7** while its own body named **G6**.

  > *Body kept by the merge `merge/ke16-into-render`, 2026-09-10, from `feat/threadpool-ke16` — the record as it stood when the ruling was taken. Where it says a ballot is STILL OPEN, the disposition line above it is the later state.*
  >
  > ⚠ **HEADER NOTE, 2026-09-10 (`merge/ke16-into-render`) — nothing in this block is a live status.** Every sentence in it that calls the ballot open is struck in place and dated. GB-6 was DISPOSED BY THE OWNER on 2026-09-03, both halves; ground at the who-table row `docs/OPEN-QUESTIONS.md:1032` and §*2026-09-03* (`docs/OPEN-QUESTIONS.md:2343`).
  >
  > ⚠ ~~STILL OPEN — 2026-08-30. The owner asked for the ANALYSIS, not a ruling.~~ Below is what was
  > measured; nothing in it is a decision.
  >
  > **(a) The scene-form fork — the evidence says it was already decided, and already acted on.**
  > The imperative sits in [`gaia/CAMPAIGN.md`](gaia/CAMPAIGN.md) §Relations. The disposition appears
  > in **four other documents, all in the declarative voice, none interrogative**:
  >
  > | site | text |
  > |---|---|
  > | [`AETHER-GAIA-REVISION-2026-08-29.md`](AETHER-GAIA-REVISION-2026-08-29.md) | in the **landed-changes** table: *"Aether's `scene` narrows to dev-bootstrap and the shipped world form moves to Gaia"*, with carriers cited |
  > | [`AETHER-LANG-PLAN.md`](AETHER-LANG-PLAN.md) §3.7 | *"`scene` keeps only a **dev-bootstrap** role"* |
  > | [`AETHER-V1-SURFACE-REVIEW.md`](AETHER-V1-SURFACE-REVIEW.md) | *"`scene` narrows to a dev-bootstrap role; the shipped world form moves to Gaia"* |
  > | [`aether-v2/CONSTRUCTS.md`](aether-v2/CONSTRUCTS.md) §`scene` | *"`scene` keeps its narrowed authored-scene role, with the world moving to the baked asset format"* |
  >
  > **And downstream work has already been reassigned on its basis.**
  > [`AETHER-V1-SURFACE-REVIEW.md`](AETHER-V1-SURFACE-REVIEW.md) declares four v1 absences —
  > `N-14` (no node handle), `N-15` (no scene unload), `N-16` (mesh sources), `N-17` (no material
  > asset handle) — to be *"Gaia's ground now"*, each with a named carrier (AIR-08; ballot F5 / rung
  > G6; §Identity + the GN1 lint). *A question still open cannot have had four consequences filed
  > against it.*
  >
  > ⚠ *De-bolded 2026-09-10 by the merge `merge/ke16-into-render`: the sentence is the 2026-08-30 analysis's own argument, kept verbatim, and is no longer set in the bold disposition voice a reader — or the `gaia_ruled_vs_open_census` gate — reads a live status out of.* GB-6 was DISPOSED BY THE OWNER on 2026-09-03, both halves; ground at the who-table row `docs/OPEN-QUESTIONS.md:1032` and §*2026-09-03* (`docs/OPEN-QUESTIONS.md:2359`).
  >
  > ⚠ **The one thing missing is the price of the rejected alternative.** Measured:
  > `grep -rn "SceneModel" docs/ crates/` returns **exactly one line in the whole repository** — the
  > very line asserting *"the alternative, a shared SceneModel, is priced higher"*. **The claim is its
  > own only witness.**
  >
  > **What changes under each classification.** *As a ballot:* G6 is formally blocked — but G6 is
  > already held by F4's unimplemented ruling and by F5's newly widened streaming scope, so **no
  > schedule moves**; what does change is that the four `N-14`…`N-17` reassignments become
  > provisional and two documents are left asserting a decision the owner has not made — the
  > diverged-pair cost this corpus already prices. *As delegated (relabel "decided, owner may
  > veto"):* one phrase changes on one line, the reassignments stand, and the veto becomes explicit
  > instead of implicit — `CAMPAIGN.md` already carries exactly this shape for first-class `remove`.
  > ⚠ *The cost of the in-between state is visible in the tree right now*: G7's row records the
  > symptom itself — *"it lists **GB-6** against G7, while GB-6's own body names **G6**; not resolved
  > here."* A line that says "ratify" with no register reads as owed; a rung whose row names no ballot
  > reads as unblocked. Both readings are live.
  >
  > **Rider that must travel with a relabel, either way:** a veto point presumes the owner can see
  > what he is declining to veto, and today he cannot, because "priced higher" cites no price.
  > Writing that price is one paragraph and is the only thing that makes the veto meaningful.
  >
  > **⇒ Question (GB-6a), one sentence.** The scene-form narrowing is stated declaratively in four
  > documents and has already had four downstream reassignments filed against it — do you want the
  > §Relations line relabelled **"decided, owner may veto"** (the `remove` shape), or is this
  > genuinely a ballot you want to answer, in which case those four reassignments revert to
  > provisional?
  >
  > **(b) The conditional-visibility valve — the mechanism exists; ONE construct does not.**
  > What the valve is for: a designer wants *"show this element when health < 30%"*, which Gaia
  > refuses by ratified rule (§UI bindings item 5, *"Structure bakes; only VALUES react … a
  > structural change is an explicit document/subtree swap via an action"*; §Refusals, *no structural
  > reactivity*, *no out-of-document conditions*). The sanctioned escape: the document names an
  > action; the action toggles an EnableTag; the layout pass skips the node.
  >
  > **Three of the four pieces already ship, measured.** (1) `flag` is a ratified Aether construct —
  > [`aether-v2/CONSTRUCTS.md`](aether-v2/CONSTRUCTS.md), *"enable bit: O(1) toggle, takes
  > NOTHING"* — with `enabled`/`disabled` filter terms in the shipped v1 grammar. (2) **The engine
  > already runs this exact idiom in production, three times over**: `UiWorldCulled`,
  > `UiWorldHidden`, `UiWorldOccluded` (`crates/boyko_ui/src/world/components.rs`) are bitset tags
  > owned by three separate authorities, deliberately split so *"the three never race a shared
  > bit"*, and *"the layout pass skips a root with this bit set."* The valve's runtime is not
  > speculative; it is shipped, with a documented ownership discipline to copy. (3) Gaia's document
  > side is ratified already: *"A condition in a Gaia file is a **token reference, never an
  > expression** — that is the whole answer to how data references logic without becoming code"*.
  >
  > **The missing piece is exactly one thing: Aether has no construct that TOGGLES a flag.**
  > `CONSTRUCTS.md` has 16 construct sections; `flag` declares the bit, the filter grammar reads it,
  > and there is **no `action` construct at all** — the word "action" occurs once in that file, inside
  > a machine's drain-loop prose. A machine action block can write one, but `machine … on entity` is
  > **R5**, and R5 is blocked on owner ballot **AB-7**.
  >
  > **What rung could carry it.** **Aether R3 — the only honest candidate today**: it owns
  > `tag`/`flag`, it owns `system` and `set`, it is the rung CG-1/CG-2 are already nominated to, and
  > it is where the deadline's own failure mode — *"the surface hardens without it"* — actually
  > occurs. **R5** if the valve is spelled as a two-state machine, at the price of inheriting AB-7, a
  > blocker the valve does not need. **Gaia G7** can carry the document-side spelling but **not** the
  > action, which is an exported Aether symbol by ratified rule — and splitting it G7/R3 gives the
  > valve two homes, precisely the CG-1/CG-2 disease this section is about.
  >
  > **What it costs on R3:** one construct or one modifier that writes a named `flag` bit, its refusal
  > wording, and a trybuild golden. **No new storage kind, no new tag, no new runtime — all three
  > exist and ship.**
  >
  > ⚠ **What NOT scheduling it costs, and the asymmetry is the finding.** The corpus states the
  > mechanism itself: *"Logic creep has a documented trajectory (Paradox: literals → … → 'calculated
  > on every frame, massive lag'); what stops it is refusal plus a pressure valve (curves), not
  > discipline."* **That one sentence promises two valves, and only one was built.** The data
  > profile's valve (`curves` + `scalable`) is written into the *exhaustive* allowed list of §The
  > logic line; the document profile's valve got a deadline and no rung — and the corpus's own words
  > for that are *"a deadline with no rung can never come due, and is therefore not a schedule."*
  >
  > **⇒ Question (GB-6b), one sentence.** The valve's runtime already ships three times over in
  > `boyko_ui` and its Gaia-side spelling is already ratified, so the whole gap is one Aether
  > construct that toggles a named `flag` — do you want it attached to **R3** (where `tag`/`flag` and
  > the `scene` surface already live, and where the deadline's "surface hardens" failure actually
  > occurs), which is what converts the deadline into a schedule?
  >
  > **The carrier gap — CG-1 / CG-2, analysed with the same pass.** Both are assertions about
  > **Aether's emitted authored scenes**. The natural carrier is Aether **R3**'s `scene` surface,
  > which is also where the two Aether-side twins sit; the alternative is Gaia **G3**, beside PENDING
  > M3/M10. **The asymmetry decides it:** the Gaia side is already gated — **M3** owns the cross-check
  > (bake refuses a `link` mismatch *in either direction*, two red fixtures at G3) and **M10** owns
  > the bake refusal for an `Entity` field authored without `link` — and what is unowned is only the
  > **Aether emitter's** half. ⚠ **Which produces a concrete, checkable consequence: M3's fixture
  > cannot be authored honestly until CG-1 has a carrier**, because "in either direction"
  > presupposes the Aether side emits `link` at all; written today it is either red by construction or
  > vacuous — the "gate that cannot fail" class, arriving through an unassigned rung rather than
  > through bad wording. The precedent for what happens if nothing carries them is in the same
  > document: the `stable_name` ruling is ratified and its resolution axis specified at M8, while its
  > *application to Aether-emitted scenes* rides nowhere — so the ratified rule and the emitter can
  > diverge silently, and the only thing that would notice is a fixture that does not exist.
- **GB-7** — the wall-clock companion beside G7's count gate: kept or dropped, and if kept, its
  tolerance, run count and noise floor. **G7**, minor.
  ✅ **RESOLVED 2026-08-30 by STANDING RULE — NO wall-clock companion; the count gate stands alone.**
  The model case of a ballot a measurement settled and that should never have been asked: the timed
  loop's inputs are archetype count, bound-TYPE count and row count and NOT the binding count, so the
  still frame is 332 ns at +0 % for 10× the bindings but +82 % for 2× the rows and +101 % for 2× the
  bound types; at a 100 ns timer step the frame is ~3.3 ticks with a 15-25× single-call tail, and the
  red-first delta (0 → 1 sink write) sits at signal-to-noise 0.05-0.50. A clock over the SCAN itself,
  with the world pinned, belongs to GK-2's own design pass. The ruling now sits in this branch's
  [`gaia/DECISIONS.md`](gaia/DECISIONS.md) §UI bindings item 9, where the open ballot body used to
  sit inline.

  > *Body kept by the merge `merge/ke16-into-render`, 2026-09-10, from `feat/threadpool-ke16` — the record as it stood when the ruling was taken. Where it says a ballot is STILL OPEN, the disposition line above it is the later state.*
  >
  > **RESOLVED 2026-08-30 — decided by standing rule. DROPPED.** No wall-clock companion. G7's gate
  > is the count gate alone, exactly as re-axed. The ground is measured, and it is **not** "a clock
  > is noisy" — it is that a clock does not measure the quantity this gate is about.
  >
  > **What is true at source** (read, not inferred — the shapes decide the ruling):
  > - `ui_bind_discovery` (`crates/boyko_ui/src/binding/bind_system.rs:75-89`) makes **one** call:
  >   `world.any_changed_since(&scratch.dynamic_bound_ids, …)`.
  > - `dynamic_bound_ids` is a **deduplicated set of component TYPES** — `register_bound_id`
  >   (`:56-60`) pushes only `if !self.dynamic_bound_ids.contains(&id)`.
  > - `EcsMaster::any_changed_since` (`crates/boyko_ecs/src/ecs/core/ecs_master/component_api.rs:433-453`)
  >   is `for archetype in …iter_archetypes()` over **every archetype in the world**, then `for &id in
  >   ids`, then `for row in 0..pool.count()`.
  > - `ui_bind_apply` (`:98-105`) returns immediately when `!dirty`, so a still frame is **discovery
  >   only**.
  >
  > **Therefore the still frame's wall clock is a function of archetype count, bound-TYPE count and
  > live-row count — and of the binding count not at all.** "200 bindings" is the number in the
  > gate's own name and it is **not an input to the loop being timed**: 200 or 2000 bindings over the
  > same three component types produce the identical id set and the identical scan.
  >
  > **Measured on this machine** (`windows-gnu`, rustc 1.97.1, `-O`; the discovery loop modelled at
  > HUD scale — 40 archetypes, 3 bound types, 200 rows; medians of 60 × 2000-call batches, two runs
  > agreeing to <1%):
  >
  > | what changed | still-frame cost | vs the fixture |
  > |---|---|---|
  > | the fixture as written | **332 ns** | — |
  > | bindings 200 → 2000, same 3 types | **332 ns** | **+0%** — the gate's own number moves nothing |
  > | 2× archetypes (an unrelated feature adds types) | 360 ns | **+8%** |
  > | 5× archetypes | 484 ns | **+46%** |
  > | 2× live rows (an unrelated feature adds entities) | 604 ns | **+82%** |
  > | 2× bound component types | 670 ns | **+101%** |
  >
  > A threshold pinned on that fixture is pinned to **none** of what its title names and to **all** of
  > what the fixture does not pin. Spawning one more entity in an unrelated part of the fixture world
  > moves it by tens of percent, so the companion would fail for reasons that have nothing to do with
  > binding cost — and the standing repair for that is to loosen the tolerance until it cannot fail,
  > which is where every gate in this repo's failure ledger ended up.
  >
  > **Resolution, second and independent ground.** `Instant::now()`'s median non-zero step here is
  > **100 ns**, so the whole still frame is **~3.3 timer ticks**. Single-call jitter on an idle
  > machine: median 200-300 ns against a max of 4600-5800 ns — a **15-25× tail** over the entire
  > measured quantity. The only form that reaches a usable ±1.7% process-to-process is a batch of
  > **200 000** frames (25 processes, 222 700-226 400 ns per 1000-call batch) — and 200 000 frames is
  > not a still frame; it is a microbenchmark of `any_changed_since`, which belongs to **GK-2**, not
  > to G7. Even that form's in-run spread ranged **36%-83%** across six runs, reproducing on the CPU
  > the lesson this repo already recorded for GPU timing: **the noise floor is not a constant.**
  >
  > **The tolerance the ballot asked for cannot be honestly quoted.** A tolerance must come from a
  > measured noise floor; the measured floor here is a range, not a number, and the signal it would
  > have to resolve — G7's red-first is *"dirtying exactly one source"*, i.e. **0 → 1 sink write** —
  > sits at a single-frame signal-to-noise of **0.05-0.50** across six runs, never once above 0.5.
  > The count gate resolves that same delta **exactly** (0 vs 1) with no instrument at all. Quoting a
  > round number instead would be exactly the move this ballot exists to prevent.
  >
  > **Rejected alternative and its price.** Keeping a loose companion (say "must stay under 1 ms")
  > costs a line that can never go red — it is 3000× the measured cost, so it survives any regression
  > the count gate would catch, and it would be read by later maintainers as timing coverage that
  > does not exist. That is strictly worse than no clock: the corpus already struck the
  > delta-subtraction form as unfalsifiable, and re-admitting it in a looser dress re-creates the
  > defect with a fig leaf. **A gate that cannot fail is worse than an absent one, because it is
  > counted.**
  >
  > **The condition under which a clock returns, stated so the drop is not permanent by accident.**
  > A wall clock over the **scan itself** — `any_changed_since` benchmarked with archetype count,
  > bound-type count and row count all pinned, and the number reported per row rather than per frame
  > — is a legitimate instrument, and it is exactly what **GK-2**'s design pass needs to justify a
  > per-column tick (GK-2's row already calls today's scan *"an O(live rows) scan documented as
  > 'cheap'"*). That bench belongs to GK-2 and is not a companion to G7's count gate.
  >
  > **Corroborated in passing, not reopened:** the `any_changed_since` doc comment (`:416-419`) claims
  > the scan is *"Bounded to the archetypes that actually host a bound id (typically 1-few)"*. The
  > loop at `:434` is over **all** archetypes; only the inner work is bounded. The corpus already
  > owns this as GK-2's companion doc fix ([`gaia/DECISIONS.md`](gaia/DECISIONS.md) §UI bindings,
  > item 8) — recording that the reading holds, and changing nothing else.
  >
  > **Riding lines moved in the same edit:** [`gaia/CAMPAIGN.md`](gaia/CAMPAIGN.md) row G7,
  > [`gaia/DECISIONS.md`](gaia/DECISIONS.md) §UI bindings item 9,
  > [`AETHER-GAIA-REVISION-2026-08-29.md`](AETHER-GAIA-REVISION-2026-08-29.md) §Work order,
  > and [`ru/OPEN-QUESTIONS.md`](ru/OPEN-QUESTIONS.md).
- **GB-8** — the corpus-wide link/id census: which rung owns it, and whether it may carry per-site
  waivers. **Precedent, not a reopen** (no ratified item is touched): this repository's own
  anchor-census precedent says no. ⚠ *This body used to cite "188 abdications out of 302 sites".
  That is NOT this tree's number and no in-tree gate produces it:*
  [`tests/internal_docs_anchors.rs`](../tests/internal_docs_anchors.rs) *run live prints* **735
  anchors / 116 waived** *— and the precedent is STRONGER than that aggregate, because the waiver
  concentrated entirely in the one document admitted under it, which waives* **89 of 177 (52.3 %)**.
  **G0/R8**.
  ✅ **RESOLVED 2026-08-30 [delegated], in three parts.** (1) **NO PER-SITE WAIVERS** at any of the
  four censuses, ever; where a property is not decidable as written, narrow the PREDICATE and print
  the narrowing in the failure message. (2) The AIR census **widens to ALL of `docs/` and LANDS AT
  G0** — a one-constant change, green at zero remediation. (3) The **LINK census is a SEPARATE
  deliverable at Aether R8** (3186 relative targets under `docs/`, 59 dead across 11 files, 44 of
  them in `docs/AUDIT-2026-05-23.md`), because landing it with the id half would hold the free half
  hostage to 59 repairs. ⚠ **G0 is NOT done**: the widening edit has landed on no branch, and the
  census file `tests/gaia_g0_citation_census.rs` exists on `feat/threadpool-ke16` and not on this
  one. ⚠ Filing correction: the work order files GB-8 against **R8 only**, dropping the **G0** half
  that turned out to carry the work.
- **GB-9** — does the `Or`-over-dense generated-code ban survive the kernel fix (KERNEL-BACKLOG
  **KE1**)? Keep it with a stated ground, or delete it with a record. The body said **"Decide before
  R0 lands"**, because R0 is precisely what makes the ban's original ground false. Blocks **G7**'s
  codegen rules.
  ✅ **RESOLVED 2026-08-30 by STANDING RULE — option (b): the ban is DELETED WITH A RECORD.** Its
  original ground was fixed and gated by Aether R0 / KE1; the fallback ground (D4) was measured and
  rejected — D4 reserves the Aether SURFACE `or(...)` while the ban governed GENERATED CODE, and
  ratified GN2 says the baker emits no Rust, so the ban had no subject. It could not be re-grounded
  on KE13 either: `Or` folds `NEEDS_CHANGE_DETECTION` and `EcsMaster::query` const-refuses it, so the
  banned shape cannot reach a `QueryView` at all. What G7 owes INSTEAD: the red fixture asserts the
  GENERATOR does not emit a change-gate over a NON-SIGNATURE storage kind (GK-2's ground, still
  live). ⚠ **The deadline EXPIRED UNANSWERED — R0 landed with GB-9 open**, which is why the ruling
  reconstructs the record from prose rather than from a reproducible failure. It is recorded rather
  than smoothed away, and the same work order that carried the deadline declared R0 "buildable now,
  no ballot in the way" in the same commit.

> **Merge note, 2026-09-10 (`merge/ke16-into-render`).** The two entries that follow are
> `feat/threadpool-ke16`'s own **GB-8** and **GB-9** items — the rulings written where they were
> taken, with the measurements behind them. They repeat the two ballot ids above, which are this
> branch's 2026-09-03 summaries of the same two rulings. Both are kept: each carries numbers the
> other does not, and the summaries above additionally record what had NOT landed as of
> 2026-09-03.

- **GB-8** — **RESOLVED 2026-08-30** (orchestrator ruling; the owner delegated this ballot as one of
  the twelve mis-routed under CLAUDE.md's "perf and architecture forks are decided with numbers").
  Original question: the corpus-wide link/id census — which rung owns it, and whether it may carry
  per-site waivers. **Precedent, not a reopen** (no ratified item is touched). Ruling recorded in the
  campaign's own decision log: [`gaia/DECISIONS.md`](gaia/DECISIONS.md) §Census discipline.

  **Measured first, because the ballot's own ground is not this tree's.** `188 of 302` appears in
  four corpus files and in the census test's doc comment, and **no in-tree gate produces it** —
  `tests/internal_docs_anchors.rs` states neither number. Run live (2026-08-30) it prints **735
  anchors, 116 waived**: ARCHITECTURE.md 6/0, FEATURE_MAP.md 222/7, SYSTEMS.md 330/20,
  MESHLET-VIRTUAL-GEOMETRY-PLAN.md **177/89**. The aggregate is 15.8%, not 62% — **and the
  precedent is stronger than the aggregate, not weaker**: the waiver did not spread evenly, it
  concentrated entirely in the one document admitted under the allowance, which now waives
  **50.3%** of its anchors, and a waived anchor "keeps **neither** shape nor identity"
  (`internal_docs_anchors.rs` head, clause 3).

  **Also measured:** the census that landed at `01a4436e` is not one scope but four.
  `tests/gaia_g0_citation_census.rs` (460 lines, **4 tests, all green**): the **AIR** census covers
  `docs/gaia` + `docs/aether-v2` (18 definitions, **114 citations across 13 files**); the **KE/KM**
  census **already runs corpus-wide over all of `docs/`** (16 rows, 101 citations, 352 files); the
  retired-form census also runs over all 352 files under a narrowed predicate; the staleness census
  covers the 1 file the revision record marks `ratified-stale`. None carries a waiver list.
  Widening the AIR census corpus-wide is **green at zero remediation**: the `AIR-##` citations
  outside G0's two directories sit in **5** files (`OPEN-QUESTIONS.md`, `ru/OPEN-QUESTIONS.md`,
  `AETHER-GAIA-REVISION-2026-08-29.md`, `FEATURE_MAP.md`, `AETHER-V1-SURFACE-REVIEW.md`), and
  **every id cited lies inside the carrier's `AIR-01..AIR-18`** — so none of them is a repair.
  ⚠ **The count is deliberately not pinned here, and the reason is this ruling's own footprint.**
  It measured **21** before the ruling was written and **40** after, because the ruling text itself
  cites `AIR-06` a dozen times in this file and its Russian twin. A citation count is a moving
  target that goes stale on the next edit — which is the exact defect this ruling corrects two
  paragraphs above. **The property is the ruling; the number is an observation with a timestamp.**
  **The LINK half had never been measured and is the half with a cost**: over the two G0
  directories, 116 relative markdown targets, **0 dead**; over all of `docs/`, **1636 targets, 59
  dead across 11 files — 44 of the 59 in one file**, `docs/AUDIT-2026-05-23.md` (the other 15: 8 in
  `docs/plans/`, 5 in `docs/archive/`, 2 in `docs/diagnostics/`).

  **RULING, three parts.**
  1. **No per-site waivers, at any of the four censuses, ever.** The ground is the live measurement
     above, not the cited 188/302. Where a property is not decidable as written, the remedy is the
     one this census file already practises and states at its own site — **narrow the predicate and
     print the narrowing in the failure message** (its retired-form test replaces ~1400 per-site
     dispositions with one decidable rule). A **scope statement** ("this census covers directory X")
     is not a waiver; a per-site skip list is.
  2. **The AIR census widens to all of `docs/`, and it lands at G0 — not R8.** It is a one-constant
     change (`G0_DIRS` → `markdown_under("docs")`), it is green at zero remediation, the sibling
     census in the same file already runs corpus-wide from G0, and G0 is still open (its row carries
     no ✅ and is held by F1/F4), so it can still take work. R8 depends on R3 and would date a
     one-line edit months out.
  3. **The link census is a SEPARATE deliverable and it lands at R8**, with the 59 dead targets as
     its red-first evidence and with `docs/archive/` + `docs/plans/` **in scope** — excluding them
     would be the scope-statement route and it is not needed, since only 13 of the 59 live there.

  **Rejected, with the price.** *Per-site waivers so the whole thing lands in one commit* — price,
  measured on the sibling gate: 50.3% abdication on the document that used the allowance, with
  `check_anchor` returning at the waiver branch before the shape test, so a waived anchor that is
  simply **wrong** about which line holds the symbol still passes. *Give the whole census to R8* —
  price: a green, free, one-line widening waits behind R3 and R8, while the KE/KM half in the same
  file already contradicts that placement by running corpus-wide from G0 today. *Land id and link
  together at one rung* — price: the free half is held hostage by the half carrying 59 repairs,
  which is how a gate gets deferred until it is convenient.
- **GB-9** — **RESOLVED 2026-08-30** (orchestrator ruling under the same delegation). Original
  question: does the `Or`-over-dense generated-code ban ([`gaia/DECISIONS.md`](gaia/DECISIONS.md)
  §UI bindings, item 8, second row) survive the kernel fix (KERNEL-BACKLOG **KE1**)? Keep it with a
  stated ground, or delete it with a record. The deadline "decide before R0 lands" **expired
  unanswered** — R0 landed at `01a4436e` — so the original ground can no longer be observed in the
  tree and this ruling reconstructs it from the record.

  **Measured.** (1) KE1 is fixed and gated: `crates/boyko_ecs/tests/ke1_or_dense_blindness.rs`
  **8/8 green** (run 2026-08-30; 7 of 8 red before the fix), and `impl_or_filter_tuple` now folds
  `HAS_DENSE = false || $F::HAS_DENSE` and forwards `resolve_dense` per arm.
  (2) **The ban's ground CANNOT have "moved to KE13", and a ruling that said so would be false.**
  The same macro folds `NEEDS_CHANGE_DETECTION = false || $F::NEEDS_CHANGE_DETECTION`, and
  `EcsMaster::query<D, F>()` opens with `const { eval_query_no_change_detection::<D, F>() }` — a
  **compile error** whenever `F::NEEDS_CHANGE_DETECTION`. So `Or<(Changed<A>, Changed<B>)>` cannot
  reach a `QueryView` at all, and KE13 is a `QueryView::get`/`get_mut` defect over a dense
  `With`/`Without` — a different shape from the banned one.
  (3) What *does* survive R0: the three shapes R0's own landing note says it did not cover
  (`Query<Entity, Or<..dense..>>`, `Added<Dense>` inside `Or`, arity > 2 with more than one dense
  arm); `par_iter`/`for_each_chunk`, which now **compile-refuse** a dense-armed `Or` — loud, not
  silent; and D4, whose reserve R0 narrowed to the consumer clause alone.
  (4) **The banned shape is not generator-reachable.** Gaia's ratified GN2 (§UI bindings item 4)
  rejects "bake emits Rust into the game build"; the runtime is a **closed bindable-type set**
  (`register_bindable::<C>`, shipped at `boyko_ui/src/interaction/plugin.rs:189`) plus type-erased
  fn-pointer accessors. The `Or<(Changed<C1>, …)>` change-gate in the shipped precedent is
  **host-composed** — `boyko_ui/src/binding/bind_system.rs` says so in as many words — hand-written
  engine-side, not emitted per binding. All four production `Or<(` type positions in the tree are
  hand-written and none is over dense (the only production dense component is `GpuTransform3D`).

  **RULING: option (b) — DELETE the ban, and replace it with a rule that is not about `Or`.**
  Its recorded ground is fixed and gated. Option (a)'s named candidate ground — D4's coupling — does
  not survive contact: **D4 reserves the Aether *surface* `or(...)`, while the ban governs *generated
  code*, and GN2 says the baker emits no Rust** — a ban on a shape the generator cannot emit,
  grounded in a reserve on a surface the generator does not write, is a rule with no subject. The
  row is replaced by **"a bind source or change-gate over a dense (or bitset) component"**, which is
  item 8's *third* row and whose ground (`any_changed_since` / `get_component_changed_tick` blind to
  dense — **GK-2**) is still live and unfixed; the `Or` row was always the weaker statement of the
  same hazard, aimed at one filter shape instead of at the storage kind. The fixture discipline is
  kept and re-aimed: the red fixture asserts the **generator does not emit a change-gate over a
  non-signature storage kind**, which cannot go green on a kernel commit. The three uncovered
  `Or`-dense shapes and KE13 are filed against G7's codegen rules as *"if a generator ever emits
  `Or` over dense, these are the shapes with no oracle"* — deleting the ban must not delete the
  knowledge.

  **Rejected, with the price.** *(a) Keep the ban.* Price: a ratified line whose stated ground is a
  fixed defect, whose fallback ground (D4) is about a different artifact than the one the ban
  governs, and whose only remaining candidate ground (KE13) is factually unavailable — i.e. exactly
  the "recommendation printed as if ratified" defect this ballot list exists to catch, re-created by
  the act of resolving it. Second price: the red fixture stays aimed at a shape nothing emits, which
  is how a gate becomes unfalsifiable.

### Aether v2 — the `AB` series

> ✅ **STATUS, 2026-09-03.** Every `AB` ballot except **AB-12** was closed on 2026-08-30 — six by
> the owner and six delegated (see §*2026-09-10* above for the corrected count; this note read
> *"seven by the owner and eleven under the standing perf/architecture rule"* until 2026-09-10) — on
> `feat/threadpool-ke16`, which this branch had not received. The per-ballot index is now ported
> into this file at §*2026-09-10*; the original stays on that branch. **AB-12 blocks nothing.**
> Every body below now carries a one-line pointer to its index row; the four with a direct Gaia
> consequence (AB-6, AB-10, AB-11, AB-13) keep their fuller 2026-09-03 annotation.

- **AB-1** — auto-registration of events: ratify on ergonomics alone, or STAGE it under D4 until an
  in-tree consumer exists. ⚠ **reopens the ratified C3 grant, and the reopen is licensed by
  measurement**: the grant's recorded ground was that the unregistered case "fails silently on both
  ends", and both generated ends are in fact a loud init-time panic. Blocks **R3**'s event
  construct.
  ✅ **RULED 2026-08-30 BY THE OWNER — RATIFIED as specified**; text and ground: §*2026-09-10* row AB-1.
- **AB-2** — ✅ **RESOLVED 2026-08-30 [delegated]** → ruling **E4** in
  [`aether-v2/DECISIONS.md`](aether-v2/DECISIONS.md). **`lanes N` is redefined from a count to a
  MINIMUM**: the effective count is `max(N, worker_count + 1)`, resolved where the worker count is
  first known, and the raise is **reported once at boot**, not silent.
  *Measured, not reasoned:* the floor is unchecked on every path because
  `current_worker_id_or_dispatcher_lane` returns a worker's own id and **ignores** its
  `worker_count` argument — a correct denominator clamps nothing. The violation is a panic in both
  profiles (`thread_index 3 >= thread_count 1` in debug; `index out of bounds: the len is 1 but the
  index is 3` in release), never a `Result`, so the ballot's characterisation is **confirmed** — and
  it is a *safe* bounds panic, not UB, which is what makes raising affordable.
  ⚠ *The probe also found the floor is already violated by the DEFAULT path:* `EcsMaster::new()`
  hard-wires `EventDispatcher::new(1)` with no setter, so `preregister_event_default` allocates one
  lane on every machine — a 4-worker pool + default registration + 64 worker sends **panicked 3
  times** in release. Feasible because `App::new()` builds the pool before `add_plugin` runs.
  *Rejected:* **boot refusal** (a source correct on a 4-core laptop becomes unbootable on a 64-core
  server, for a number the author cannot know); **dropping the knob** — technically the cleanest
  answer, since `lanes` is not author-meaningful, but it **deletes a ratified language surface**,
  which is a SCOPE call: **escalated to the owner as a recommendation, deliberately not taken.**
- **AB-3** — ✅ **RESOLVED 2026-08-30 [delegated]** → ruling **E5**. **Per-thread claimed host lane,
  one claimer enforced.** `MAX_EVENT_THREADS` 65 → **66**, const-assert strengthening to
  `MAX_WORKERS + 2 <= MAX_EVENT_THREADS`.
  ⚠ **Two premises in this ballot were false and are corrected at source.** (1) `MAX_EVENT_THREADS`
  in the tree is **65, not 64** — `constants.rs:400`, raised together with its const-assert in
  `01a4436e`, the very commit this corpus was written alongside (`git log -L400,400:…` dates it).
  KE8's *lane-constant* half has **landed**; its `&self` send, `send_slice` and re-aimed debug
  assert have not. So this option buys **one** lane, not two. (2) It is not true that "every OS
  thread that is not a pool worker maps to lane 0": the mapping has **three** arms, and the
  **dispatcher gets its own reserved lane** (`worker_count`). Only `WORKER_ID_UNATTACHED` maps to 0.
  *The hazard, measured in release* — an unattached thread and worker 0 each sending 4000 events,
  released on a rendezvous barrier: **6 of 6 runs lost events** (1891, 2295, 2070, 183, 1473, 1940
  of 8000 = 2.3 %–28.7 %), and **every send returned `Ok`**. Without the barrier 6/6 runs lose
  nothing — which is precisely how a stress test that does not force overlap stays green over this.
  *Why "accepted hazard, documented" was unavailable:* the loss is silent **and** it is UB (two
  threads writing one `MaybeUninit<E>` slot through an `UnsafeCell`), and the tree already holds
  that option's output as a **false** `// SAFETY:` clause — `send_one`'s U4 clause 2 claims "only
  the worker pinned to `thread_index` accesses this UnsafeCell". Ratifying (c) would ratify an
  invariant measurement has refuted; principle 8 forbids it. *Rejected:* **`Err` on unattached** —
  breaks the escape hatch the engine itself documents (`EventWriter::send`'s doc sends main-thread
  and FFI callers to `send_event`, calling that route "safe"), and buys nothing the claimed lane
  does not. The chosen contract concedes that breakage for the **second** unattached claimant only,
  which is the genuinely unsound case.
- **AB-4** — ✅ **RESOLVED 2026-08-30 [delegated]** → ruling **E6**. **Registrant: the generated
  path**, calling a new `register_ordered_emitter(event_id, system_id)` at plugin build, beside the
  `preregister_event[_default]` it already emits. **Predicate** (the half the ballot said was
  missing): at build end, any `ordered` event id with an emitter count **> 1** is a hard boot
  failure naming both systems. **Verbatim escape: refused at the param list** — a hand-written
  `EventWriter<E>` for an `ordered` `E` is refused at `EventWriter::init_state`, the site that
  already panics loudly for an unregistered event and already has `E::event_id()` and the dispatcher
  in hand.
  *Rejected — `SystemMeta` emit-access:* measured, `EventWriter::init_access` is an **empty body**
  carrying "events stay OUTSIDE the conflict graph" (Phase 12 EW5 / Q2 Option A), so `SystemMeta`
  holds zero event information and there is no axis to read. Adding one as a *write* would make the
  scheduler serialise every pair of same-type emitters — surrendering exactly the parallel emission
  E1 exists to buy; adding a non-conflicting axis is option (a) with worse placement.
  *Rejected — the `#[event]` macro side:* structurally impossible, since exclusivity is a property
  of the emitter **set** and the macro sees one type and zero systems (it does not even read its
  attribute arguments today — `boyko_macros/src/lib.rs:292` takes `_args`).
  *Price of the refusal, stated:* a hand-written system cannot emit an `ordered` event even as sole
  emitter; the escape is to declare the emitter in Aether.
- **AB-5** — **RESOLVED 2026-08-30** (orchestrator ruling under the owner's delegation of the
  perf/architecture forks). Original question: the machine event router's random-access mechanism
  and the tick visibility following from it — (a) amend M4 to `Query::get_mut`; (b) keep
  `get_component_mut`. **Decision: (a).** Full ruling with the measurements:
  [`aether-v2/DECISIONS.md`](aether-v2/DECISIONS.md) **M4a**, with M7 and **D6a** amended in the
  same edit and [`aether-v2/MACHINES.md`](aether-v2/MACHINES.md) §Event routing + CAMPAIGN R5's
  Depends cell moved with them.

  *First, a correction to this item's own body.* It said the `Query` SystemParam "has **no**
  `get`/`get_mut` today". That was true when written and is **not** true now: both landed with R1 at
  `01a4436e` (`iters/query/query.rs`, `get` and `get_mut`), so (a) was a citation fix, not a new
  dependency.

  *The tick half, measured* under an explicit ordering edge — which the pre-existing tests lack, and
  without which "same frame" and "one frame later" are indistinguishable: `Query::get_mut` +
  `Mut<T>` is seen by a `Changed<T>` reader **in the same frame**; `get_component_mut` is **not**,
  even with the reader ordered after it.

  *But the decisive ground turned out not to be the tick at all.* `get_component_mut` takes
  `&mut self`, so its router can only be an **exclusive** system — and an exclusive body is
  `FnMut(&mut EcsMaster)` with, in the kernel's own words, "no param tuple, no per-param state"
  (`system/exclusive_function_system.rs`). Such a router **cannot take `EventReader<E>`**. It must
  read through `EcsMaster::events_of`, whose contract is "Returns an **empty slice** if `E` was not
  registered or if no events were sent last frame" — the one silent path ruling C3 already
  identified, collapsing *unregistered* and *nothing happened* into one value. Option (a)'s router is
  an ordinary system: it takes `EventReader<E>`, and a forgotten registration is a **loud boot
  panic**. `events_of` also serves the **previous** frame, making (b)'s latency **two** frames
  against the one MACHINES.md documents.

  *Rejected — (b), and its price:* a machine whose `inbox` event is un-preregistered routes nothing,
  silently, forever; doubled latency; plus a bypass mechanism that exists only to repair (b).

  *A ground I expected and did NOT get, recorded so nobody re-argues it.* Exclusive systems declare
  `Access::universal()` and are single-per-round, so (b) looked like a per-event-type scheduler
  barrier. **Measured against a control that proves the schedule had real concurrency to lose**
  (1 thread 235–250 µs vs 8 threads 79–133 µs, **2.7–3.2×**): an exclusive router cost **0.78–1.00×**
  a `Query` router at both 1 and 8 routers — below the ±20 µs noise, sign flipping between runs.
  **Scheduler cost grounds neither option.**

  *The two riding questions, answered here rather than left to drift:* `publish tracked` **does**
  now see the router's deposit in the same frame; and M7's tick bypass is **withdrawn, not
  re-derived** — measured, a plain `&mut T` through `get_mut` bumps **no** tick while `Mut<T>` bumps
  at `this_run`, so all-or-nothing falls out of the emitted term. (b) had no such lever:
  `get_component_mut` returns `Mut<T>` unconditionally, which is exactly why a bypass had to be
  invented for it.
- **AB-6** — `requires` of a dense-storage component: parse refusal / a dense required-ctor route /
  known-open plus a hook workaround. ⚠ the refusal option **narrows the ratified
  `storage = table|dense` × `requires` surface**. The red tests land now under any disposition —
  they demonstrate the panic either way. Blocks KERNEL-BACKLOG **KE11**'s disposition and **R3**'s wording.
  ✅ **RESOLVED 2026-08-30 — option (b), and the class SPLITS IN TWO.** **Dense** gets the
  construct-and-commit route (KE14); **bitset/flag** is refused IN THE DERIVE, since a flag has no
  bytes and `FLAGS_DIRECT` is the mechanism. Refusing in Aether alone was ruled out on the owner's
  principle that *a language may not refuse what the derive accepts*. The ballot's premise was
  refuted first: hand-written Rust could not do it either, so there was no ratified surface to
  narrow — only a promise the kernel did not keep. **Landed as code on `feat/threadpool-ke16`**,
  commit `ef81ecaa` (2026-08-30). Gaia-relevant: **F9's refusal option is off the table verbatim
  because of it.**
- **AB-7** — R-DENSE: unconditional with a driver-independent ground that must be ESTABLISHED rather
  than asserted, or lifted by `publish tracked`. ⚠ **re-grounds a ratified refusal**. Blocks **R5**.
  ~~STILL OPEN — STILL THE OWNER'S.~~ **Struck 2026-09-10 by the merge `merge/ke16-into-render`:
  the owner LIFTED it — see the ruling table above, `docs/OPEN-QUESTIONS.md:1641`, "R-DENSE LIFTED,
  and the ballot's framing rejected", with his own words. This paragraph is the record of the state
  BEFORE that ruling, not a live status.** ⚠ When it was struck, the ruled-vs-open census could not
  see this one: the table at `docs/OPEN-QUESTIONS.md:1637` is headed `| ballot | ruling | where the ground lives |` with no
  `who` column, so it was not a ruling SOURCE by the predicate, and AB-7 sat in the census's `open`
  set without reddening it — found by hand while repairing the ten sites the census did catch. Since
  the A5 batch merge (2026-09-22) the §*2026-09-10* index above carries the same ruling in a `who`
  row, so the census reads AB-7 as ruled and this bullet may not spell the marker in bold: the
  sentence that did is the one you are reading, un-bolded. What was delegated was not the choice but the *measurement*
  underneath it, and it was taken on **2026-08-30**: **the candidate driver-independent ground is
  REFUTED, on both of its conjuncts.** The ballot therefore goes to the owner with a refuted premise
  rather than an open question. Evidence, all read or run in this tree at `01a4436e`:

  1. *"the router's deposit path assumes a table row"* — **false.** Both random-access deposit APIs
     carry working dense arms: `EcsMaster::get_component_mut` resolves the global `DenseStore` and
     bumps the per-slot `changed_tick` (`ecs_master/component_api.rs`, `StorageKind::Dense` branch),
     and `Query::get`/`get_mut` carry `HAS_DENSE` gates. Pinned green in-tree by
     `tests/ke3_query_random_access.rs::{get_applies_a_dense_with_filter,
     get_mut_applies_a_dense_with_filter}`.
  2. *"the layout const-assert assumes a table row"* — **there is no such assert.** Every dense
     `const { assert!(…) }` in the kernel belongs to a **driver** (`for_each_chunk`,
     `par_for_each_chunk`, `par_iter`, `Query::contains`; `dense_iter` asserts the converse). Every
     other `StorageKind::Dense` site is a branch that *handles* dense, not one that rejects it.
  3. Measured positively: a dense component is fully iterable **and** change-tracked under the
     sequential `iter_mut` driver — 64 of 64 rows reached a `Changed<>` reader — which is exactly the
     lowering `publish tracked` selects.

  **What the measurement did find** is a real constraint, but narrower than "driver-independent":
  dense is compile-rejected (`E0080`, reproduced) by **all three non-sequential drivers** —
  `for_each_chunk`, `par_for_each_chunk` **and `par_iter_mut`** — and `par_iter`'s own message gives
  the reason as *"the parallel path does not resolve the dense store into each worker chunk's
  `Fetch` (the chunk runner has no world cell)"*. The rejection tracks **parallelism and chunking**,
  not chunking alone. Two consequences for whoever takes this: **(i)** option (b) is what the engine
  supports today, and lifting the refusal does **not** make `parallel` machines work over dense,
  because both parallel drivers reject it too; **(ii)** a genuinely driver-independent ground may
  exist but lives in a **different ballot** — `requires` over a dense component panics
  (KERNEL-BACKLOG **KE11** / ballot **AB-6**). That is also the owner's and is **neither settled nor
  assumed** here.
- **AB-8** — **RESOLVED 2026-08-30** (orchestrator ruling; a performance fork, so it is decided with
  numbers rather than escalated). Original question: the `each par` driver, plus three riders. Full
  ruling: [`aether-v2/DECISIONS.md`](aether-v2/DECISIONS.md) **C5a**.

  **Decision: `each par` → `par_iter_mut`.** Measured rather than taken from the labels — over 2048
  rows (clear of the 1024-row inline floor), with the writer keyed `.before` the reader: a
  `Changed<>` reader sees **2048 of 2048** `par_iter_mut` writes and **0 of 2048**
  `par_for_each_chunk` writes. Cost, release, 200 000 rows × 200 frames, 8 workers, five samples:
  `par_iter_mut` is **1.17–1.47×** `par_for_each_chunk` (median ≈1.19), a delta of **0.03–0.07
  ns/row**. Absolute ns/row is **not** reproducible run to run (0.13→0.34, machine noise) — the
  *ratio* is, which is why the ratio is what is recorded. At the machine cost model's own 10 000 rows
  that delta is **0.3–0.7 µs/frame**, against the **34–40 µs** ruling D1 measured for the 5-arm jump
  table at the same row count: **≈1–2 % of the pass**. The measurement carried a falsification guard
  (row count and per-row increment count asserted after timing), so a no-op could not have produced
  the numbers.

  *Rejected: `par_for_each_chunk` as the `par` driver.* Price — the most inviting parallel spelling
  in the language would be structurally tick-blind, silently killing every downstream `Changed<>`.
  That is C5's own recorded defect, and the 0-of-2048 above is it reproduced.

  The three riders, answered with it: **`soa par` EXISTS**, and is the only route to
  `par_for_each_chunk` (tick-blindness stays behind the word that announces it). **The batching key
  is NOT author-visible in v1** — measured ground, not taste: `BatchingStrategy` has **three** fields
  (`batches_per_thread`, `min_batch_size`, `max_batch_size`), so `parallel (batch = N)` cannot name
  what it appears to name; *rejected alternative* — mapping `N` to `batches_per_thread`, whose price
  is that an author writing `batch = 64` expecting 64 rows per chunk gets 64 chunks **per thread**.
  The knob stays reachable via `batching_strategy(…)` from the verbatim escape, and the form is built
  when a measured in-tree consumer appears (D4's rule). **Machines and `each` share the LADDER, not
  the default** — both select tracking by term and climb `iter_mut` → `soa` → `par`, but `each`
  defaults to `iter_mut` (C5) and the machine pass to the chunked driver (M7/D6), each measured
  separately; unifying the defaults would reopen M7/D6, which AB-5 licensed only for the citation and
  the withdrawn bypass. Left standing deliberately.
- **AB-9** — **RESOLVED 2026-08-30** (orchestrator ruling under the same delegation). Original
  question: `boyko_reflect` sequencing for AIR-06(b). Options were: R8 waits on the merge /
  AIR-06(b) is descoped to the reflection-free halves (asserted to still satisfy the oracle) / the
  merge is pulled forward. Ruling recorded in the campaign's own decision log:
  [`aether-v2/DECISIONS.md`](aether-v2/DECISIONS.md) §Sequencing rulings, entry **AB-9**.

  **Measured, and the ballot's figures were stale in both directions.**
  1. `feat/reflection` is **15 commits ahead of and 20 BEHIND** `feat/aether-v2` (merge-base
     `5ec1699f`). "18 commits ahead" was measured against a different base and before
     `f7c46c76`/`d272e1fd`/`0e0b4c68` landed. The behind-count is the number the ballot never
     carried and it is the one that grows.
  2. Merge cost, from `git merge-tree --write-tree` (**no merge performed**): **7 conflicted
     paths** — 4 docs (`FEATURE_MAP.md`, `SYSTEMS.md`, `OPEN-QUESTIONS.md`, `ru/OPEN-QUESTIONS.md`),
     2 trybuild `.stderr` (`unknown_key_rejected.stderr` content; `on_despawn_rejected.stderr` a
     **modify/delete** — `01a4436e` deleted it on this branch while reflection modified it, so it
     needs a decision, not a re-bless), and **one source file**,
     `crates/boyko_macros/src/component.rs`, at **2 hunks / 23 lines**. The merge brings 15 commits,
     124 files, +44 956/−214, and **3 new workspace members** (`boyko_reflect`, `reflect_fixture`,
     `reflect_dogfood`).
  3. **The ballot's load-bearing claim — that the reflection-free halves "still satisfy its oracle"
     — is false as it would be used.** AIR-06's oracle has three clauses; the second is *"a
     probe-crate component must appear in the dump"*, and "the dump" is (b)'s **project** schema — a
     *grammar* manifest generated from parser dispatch tables cannot contain a workspace component.
     Descoping (b) does not leave that clause satisfied; it removes its subject. It is *satisfiable*
     reflection-free only by declaring the probe component in an `aether!` block — which makes the
     gate green over a dump covering **0 of the 138 `#[derive(Component)]` sites in the engine
     crates** (`boyko_ecs` 43, `boyko_ui` 36, `boyko_render` 21, `boyko_physics` 15, `boyko_scene`
     14, `boyko_demo` 8, `boyko_input` 1; 185 across all of `crates/*/src`). Measured: **zero
     production `aether!` declaration blocks exist** — all 50 live under test files, and the 6
     `*/src` hits are the macro's own implementation and doc comments. Gaia declares nothing (no
     baker).
  4. **And the merge alone does not fix that either.** `boyko_reflect::registry::type_info_of`
     returns `None` for "a component without `#[component(reflect)]`" — reflection is **opt-in per
     component**. On `feat/reflection`, `#[component(reflect)]` appears at **106 lines across 33
     files, and not one is in an engine crate** (23 files `reflect_fixture`, 4 `boyko_reflect`, 3
     `reflect_dogfood`, 3 `boyko_macros`).

  **RULING, three parts.**
  1. **Pull the merge forward** — `feat/reflection` merges into `feat/aether-v2` as its own rung
     before R8, at the measured cost above. "Waiting" is rejected as an option with no event to wait
     for: nobody else is merging the branch, and it is already 20 commits behind, with the lag
     concentrating in `crates/boyko_macros/src/component.rs` — the one file both the derive work and
     the reflect work touch.
  2. **AIR-06(b) is NOT descoped.** The descope buys a vacuous green.
  3. **A fourth item, which no option on the ballot named, is the real precondition and is attached
     to R8's Lands**: engine components must opt into reflection — either a sweep marking them
     `#[component(reflect)]`, or a decision that the derive opts in by default. Until one exists,
     AIR-06(b)'s dump covers the empty set under **all three** of the ballot's options. **Which of
     the two routes is taken is R8's own design pass and is NOT decided here** — it has a
     per-component static + `OnceLock` cost that has to be measured, not argued. In the same edit,
     AIR-06's oracle clause is corrected from "a probe-crate component" to "a hand-written
     `#[derive(Component)]` component **from an engine crate**", because the probe-crate form is
     satisfiable by a route that measures nothing.

  **Rejected, with the price.** *R8 waits on the merge* — price: R8's gate goes green on a dump
  covering 0 of 138 engine components (the opt-in set is empty), while the branch keeps diverging
  and the one real source conflict keeps growing in exactly the file both campaigns edit.
  *Descope AIR-06(b) to the reflection-free halves* — price: the same zero coverage **plus** the
  loss of the oracle's only workspace-facing clause, i.e. a gate that cannot fail.
- **AB-10** — AIR-10 residue: is the measurement script required when the audit branch is taken, and
  is PENDING's widening to the joint Aether+Gaia vocabulary ratified? ⚠ the second is a **scope
  change to a ratified requirement** and must not arrive as a side effect of an edit.
  ✅ **RESOLVED 2026-08-30 — (a) the measurement script IS required** when the audit branch is
  taken; **(b) the widening to the JOINT Aether+Gaia vocabulary is RATIFIED**, on the owner's own
  ground that the two languages are ONE BODY OF WORK, so a word meaning different things across
  them is a false friend between the author's own languages. Price accepted: a Gaia collision may
  force a rename in AETHER. Direct Gaia consequence for G2: every Gaia keyword ruling (PENDING
  Tier 1's `key` → `keyframe`, `table` → `defs`/`catalog`, the `text:` deletion) is now subject to
  AIR-10's bar over the joint vocabulary.
- **AB-11** — `with`/`without` over a `flag`. Recommended: a parse refusal with a did-you-mean
  pointing at `enabled`/`disabled`, which dissolves the whole class (`with Flag` matches nothing,
  `without Flag` excludes nothing; both silent). The alternative is forbidden-form-only, caught at
  doc-generation. ⚠ **adds a refusal where v1 documents non-refusal**. Blocks **R3**'s filter
  goldens.
  ✅ **RESOLVED 2026-08-30 — a PARSE REFUSAL with a did-you-mean.** Ground, measured with R0's fix
  in the tree: over a `flag`, `with F` matches NOTHING and `without F` excludes NOTHING — two
  different silent wrong answers — and the defect is in the LEAVES, so R0's `Or` fix does not reach
  it. Red test `ab11_flag_filter_polarity.rs`. ⚠ Rider, unreconciled and now cheap to close: AB-11
  records the did-you-mean as `enabled`/`disabled` while AB-6's ruling text records
  `flags (X = true)`, so an author who hits both is told two spellings for one concept. Under F9's
  recommended disposition all three flag-adjacent refusals point at ONE target — *set the component
  field* — and the reconciling line belongs in G2's diagnostics list.
- **AB-12** — *(a query, not a values call)* do the two dropped measured defects — the old G1/G2 of
  the AI-orientation defect series — exist in the session record? If they do, they return as AD5/AD6
  with repros; if not, the renumbered record stands. Only the owner's session archive can answer.
  Non-blocking.
- **AB-13** — the `flag` initial-value vocabulary, four parts that interact and are answered
  together: (1) the values themselves — `on | off` vs `true | false` / `set | clear` /
  `enabled | disabled`; (2) whether the chosen words are RESERVED keywords or contextual
  (contextual keeps them usable as identifiers, at the price of a grammar that reads differently in
  two places); (3) disambiguation across all THREE `on` positions already in the surface
  (`machine … on entity`, `on E => T`, `flags (X = on)`) — a reader and a generator must tell them
  apart without lookahead; (4) the group's NAME. On (4), read what PENDING says now: Tier 3
  **withdrew** its own `flags` → `initial` rename, on the ground that `initial` is already the
  machine's initial-state keyword, so the rename recreates the collision it was meant to fix
  ([`gaia/PENDING-SYNTAX-PLAN.md`](gaia/PENDING-SYNTAX-PLAN.md) Tier 3). The open question is
  therefore whether that withdrawal STANDS, or a different rename is wanted. ⚠ any rename here
  touches a ratified keyword, so AIR-10's bar applies and the offered measurement is the collision
  audit over the three `on` positions. Blocks **R3** (the `flag` construct surface and its filter
  goldens).
  ✅ **RESOLVED 2026-08-30 — `true`/`false`**, which settles all four parts at once: they are Rust
  keywords already, so nothing new is reserved; the three-way `on` collision does not arise, so
  neither a reader nor a generator needs lookahead; and the group KEEPS THE NAME `flags`, PENDING
  Tier 3's withdrawal of the `flags` → `initial` rename standing on its own ground. Spelling:
  `flags (Stunned = false, Burning = true)`. **Gaia shares this vocabulary VERBATIM** — but, under
  F9's recommended disposition, only for a component TYPE's initial state, never for an individual
  authored object.

**Two items on this list are NOT ballots, and are named so they are not mistaken for one.** K8 (no
honest spawn spelling for a bundle carrying `link Entity`) and K9 (the kernel's relates/related
macro demands a private collection field plus `retain_empty`, inexpressible on a pub-field Aether
group) are architecture gaps, routed to R3's design pass rather than to the owner. R3's `bundle` and
relation constructs must not be declared done while they stand open.

---

## 2026-08-28 — Gaia: the owner ballots, two of them blocking

The Gaia research (two multi-agent passes, ~25 systems surveyed; plan corpus now at
[`gaia/`](gaia/CAMPAIGN.md)) closed every architecture/perf fork by standing rule, and left seven
VALUES/SCOPE ballots. The 2026-08-29 corpus audit added three more (F8-F10, logged in the entry
above), so the Gaia `F` series now numbers **ten**; the seven raised on this date carry their full
bodies below. [`gaia/CAMPAIGN.md`](gaia/CAMPAIGN.md) §Owner ballots is the one-line INDEX only — it
used to point here for the bodies while this file pointed back at it, which is the circle that kept
five of them from ever being written.

The two that BLOCK rungs:

- **F1 — the bake route into the byte format.** `save_world` is the only legal printer, so the
  baker must build a live world from text. Route (a), the EG2 reflection seam, is already rejected
  by the owner's own audit ballot of 2026-08-27; route (b), macro-time GK-4 (derive-emitted
  name-keyed field tables + typed constructors), needs no reflection at all and detaches Gaia from
  both EG2 and the unlanded C11. Recommendation: (b) now, (a) as a later upgrade. The real ballot
  is SEQUENCING: decide EG2 first (it unblocks more than Gaia), or detach now. Blocks G1.

  > ✅ **RESOLVED 2026-08-30 BY THE OWNER — (b), macro-time GK-4, taken NOW.** His words: *"Do it
  > properly right away. But bear in mind the world must support streaming."* Both halves bind —
  > G1 takes the GK-4 route, and no part of the bake design may foreclose streaming, which F5's
  > ruling then made concrete. Rejected: sequence EG2 first. ⚠ **This ballot's own decisive
  > inference is REFUTED**: `RequiredCtor` being an `unsafe fn(*mut u8)` does *not* make ctor-form
  > `requires` unbakeable — a GK-4 baker is a Rust program linked against the derive tables and
  > calls the fn pointer trivially. Only the *evaluator over Gaia text*, a different program,
  > cannot. Ground: `feat/threadpool-ke16` `gaia/DECISIONS.md` §Inherited pipeline.
  >
  > *— and the same ruling as `feat/threadpool-ke16` recorded it on the day, kept by the merge `merge/ke16-into-render`, 2026-09-10. Both are kept because each carries ground the other does not; where the two differ in status, the paragraph above is the later one.*
  >
  > ✅ **RESOLVED 2026-08-30 BY THE OWNER — (b), macro-time GK-4, taken NOW.** *"Do it properly right
  > away. But bear in mind the world must support streaming."* Both halves bind: G1 takes the GK-4
  > route, and no part of the bake design may foreclose streaming — which is why F5 lands with the
  > scene profile rather than after it. **Rejected: decide EG2 first.** Price: G1 waits on a seam
  > this campaign does not need. ⚠ **One inference in the body above is refuted by the ruling** — the
  > baker is a Rust program linked against the derive-emitted tables, so it calls a `RequiredCtor`
  > fn pointer trivially (measured through today's public `required_ctor_in_set`); what cannot call
  > one is the *evaluator over Gaia text*, a different program. Ground:
  > [`gaia/DECISIONS.md`](gaia/DECISIONS.md) §Inherited pipeline.
- **F4 — what "loaded" means. WIDENED: the loader misses more of the insert path than hooks.**
  The finding that opened this ballot was "the loader runs no hooks". That is ONE mechanism of
  several, and the ballot as first written would have bought a hook-coverage census and still left
  a loaded world differing from a spawned one. Everything the insert path does that the load path
  does not:
  - **Hooks.** Reverse indexes (`Children`, `LikedBy`) are absent after a load.
  - **The `#[require]` closure. MEASURED: nothing on the load path adds a component the file
    omitted.** A document naming `Foo` but not its required `Bar` loads an entity with no `Bar`,
    where `Commands::spawn` of the same bundle would have both. Nor can the baker close the gap:
    `RequiredCtor` is `unsafe fn(dst: *mut u8)`
    ([`required.rs`](../crates/boyko_ecs/src/ecs/core/component/component_registry/required.rs)) —
    a raw constructor pointer, which a total, closed, build-time evaluator cannot call and
    therefore cannot bake into the file. This is the half that decides the answer.
  - **Flag / `EnableTag` initial state**, whatever an insert would have stamped.
  - **Relation reverse indexes** — hook-built, and what makes a relation queryable from the other
    end at all.
  - **Asset refcounts stay at 0**, so every mesh of a loaded scene is subject to retirement
    mid-game.

  The same document is therefore correct in the dev loop (`Commands` spawn fires all of it) and
  broken in the shipped game. Options: (a) a specified post-load fixup pass whose coverage census
  spans ALL FIVE mechanisms, not hooks alone — recommended; (b) the load path runs the real insert
  path (correct by construction, but a hook then mutates the world mid-load — a real semantic fork
  plus a per-row price); (c) forbid load-incomplete components in baked assets — untenable, it
  forbids `MeshHandle` and every `#[require]`d component at once. **Addendum (the G6 carrier):**
  whichever option wins must also NAME the stable asset-id carrier a loaded scene's references use.
  `MeshHandle(u32)` blits a process-local slot, and `MeshRef` — the name the language sketch
  writes — does not exist in the engine, so the carrier is unlanded on both spellings; F4's answer
  is what makes a loaded reference mean anything after a restart. This defines the engine's load
  semantics generally; decide before the first Gaia scene. Blocks G6.

  > ✅ **RESOLVED 2026-08-30 [delegated] — option (a), specified as SUPPRESS-THEN-FIXUP: four
  > ordered sub-passes inside `load_world`.** (b)'s *mechanism* is adopted as sub-pass 4; its
  > *timing* is refuted at a measured 100 % mis-link rate (saved/fresh id overlap 8/8), which is
  > worse than today's empty index. (c) is worse than "untenable": it forbids every mesh,
  > material, light, camera and particle effect. The addendum is answered with it — the stable
  > asset-id carrier is a stable NAME in the file, resolved to the existing `MeshHandle(u32)` at
  > load, with no new component type. **Three amendments recorded 2026-09-03** (sub-pass 4 splits
  > into 4a relink / 4b hooks; the coverage census is 5 mechanisms × 3 storage kinds; 4b reports
  > per hook class), together with what MOVED since the ballot: a reloaded scene's children never
  > compose their parent's pose, and a dense component's `#[entities]` field was fixed on the
  > other branch. See §*2026-09-03* above. Ground: `feat/threadpool-ke16` `gaia/DECISIONS.md`
  > §Load semantics.
  >
  > *— and the same ruling as `feat/threadpool-ke16` recorded it on the day, kept by the merge `merge/ke16-into-render`, 2026-09-10. Both are kept because each carries ground the other does not; where the two differ in status, the paragraph above is the later one.*
  >
  > ✅ **RESOLVED 2026-08-30 [delegated] — option (a), specified as SUPPRESS-THEN-FIXUP: four ordered
  > sub-passes, on the clone path's own precedent.** Full ruling and every measurement:
  > [`gaia/DECISIONS.md`](gaia/DECISIONS.md) §Load semantics. In brief:
  >
  > **All five reproduce at `6f75ee9e`** (`cargo test -p boyko-serialize --test
  > gaia_f4_load_path_fixups -- --ignored --test-threads=1` → `0 passed; 5 failed`, controls green),
  > and the hook census is `grep -rEn "trigger_on_[a-z]+\("` → **0** dispatch sites on the load path
  > against **38** on the command/master path.
  >
  > **The order is the ruling:** (1) require closure at **parse** time — widen the id list with
  > `for_each_required_id_excluding` and emit the **existing** `LoadColumn::Construct`, which already
  > takes a `RequiredCtor` and is simply never reached for a component the file does not mention;
  > (2) declared flags via `flags_direct_for`; (3) the existing remap; (4) hooks, then drain deferred
  > commands to a fixpoint. Requires must precede hooks; **hooks must follow the remap.**
  >
  > **(b)'s MECHANISM is adopted as sub-pass 4; only its TIMING is refuted — and the ballot's stated
  > price for (b) was wrong.** `DeferredEcsMaster` statically withholds every structural-change
  > method, so a hook *cannot* mutate the world mid-load. The real hazard is ordering against the
  > remap: `relationship_on_insert` reads a **raw saved FK** and guards on `is_alive`, and the
  > measured saved/fresh id overlap across a normal round trip is **8/8, 100%** — so the guard does
  > not fire and the hook builds a reverse index pointing at a **live but wrong** entity, strictly
  > worse than today's empty one. **This bug already has a name in this kernel: BUG-EDGE-CLONE-1**,
  > and the clone path's shipped fix is exactly a suppression bracket plus a relink after the FK is
  > remapped. **The kernel has already answered F4 once, on the sibling path, and it answered (a).**
  >
  > **(c) verified, not inherited, and worse than "untenable":** an attribute-line census over
  > `crates/*/src/`, hand-audited, finds **10 require-bearing shipped components** (three light
  > types, `ParticleEffectHandle`, three camera types, `MeshHandle`, `MaterialHandle`,
  > `UiWorldProjection`'s owner) plus **3 explicit hook-bearing** and the whole
  > `Relationship`/`RelationshipTarget` family. (c) forbids **every mesh, material, light, camera and
  > particle effect**; `MeshHandle` alone is load-incomplete on three counts at once.
  >
  > **Two corrections to the body above.** (ii) is **not a load-path defect** — it is the *dynamic
  > by-id entrance*'s, which the loader shares: `EcsMaster::create_entity` is public, shipped, and
  > produces no required component where `Commands::spawn` does, so the silent wrong answer is live
  > one call from gameplay code, and a GK-4 baker built on it would bake the defect **into the file**.
  > (iii) **splits in two**: the *declared* half is reconstructible with **no format change at all**,
  > and only the *authored per-entity* half is F9's.
  >
  > **The addendum is answered too: the carrier is a stable asset NAME in the file, resolved to the
  > existing `MeshHandle(u32)` at load — no new component type.** `grep -rn "\bMeshRef\b" crates/` →
  > **0** (all 7 repository hits are corpus prose), while `MeshRefGen`/`MaterialRefGen` is a
  > **generation counter** — the name is one suffix from a live type meaning something else, so
  > **`MeshRef` must not be minted**. And there is no name- or path-keyed asset lookup in the engine
  > at all (**0** hits), so the registry is a **G3** build item. ⚠ This names a *carrier form*, not a
  > name-vs-id ruling: **F8** stays free.
  >
  > ⚠ **Two escalations the ruling does not own.** The `create_entity` require-drop is an **engine**
  > defect independent of Gaia and needs its own carrier — filing it under F4 would let a
  > Gaia-scoped fix leave the gameplay-facing hole open. And `remap_loaded_entities` is **O(whole
  > world) per load**, which under F5 now runs on every `load_cell`.

The remaining five, in the same shape:

- **F2 — where a DataAsset's rows live at runtime.** (a) rows are ENTITIES: each row lands in a
  dense-column archetype, with derive-emitted row constants (a `[DefOf]`-shaped surface) so Rust
  names a row without a lookup. Price: a row costs an `EntityId` and its archetype slot — a
  4000-row item table is 4000 entities the world carries from boot. (b) a new RESOURCE region in
  the byte format: rows are a flat column owned by a resource and addressed by index. Price: a
  second load path in `boyko_serialize` beside `load_archetype`/`load_dense_store`, and the format
  grows a region it does not have today (inventory finding 3). Recommendation (a); its VALUES half
  is the sentence *"a table is entities"* — everything stays queryable, one storage, Principle 0
  intact. Rejected without a ballot: a `HashMap<Name, Row>` side store — a parallel data system,
  which the standing rule forbids outright. Blocks G5.

  > ✅ **RESOLVED 2026-08-30 [delegated] — (a) rows are entities, AMENDED in three ways the ballot
  > did not carry.** Storage is **`StorageKind::Table`**, not dense — the ballot's "dense-column
  > archetype" denotes nothing here, because a dense id is signature-excluded and
  > `get_or_create_archetype(&[DenseRow])` returns the EMPTY archetype. Rows materialize **eagerly
  > at load**, never lazily. A table is loaded by its own explicit load and **pinned**, so
  > `unload_cell` never touches it; the runtime handle is an `Entity` captured at load, never a
  > row index. ⚠ **CONFIRMED 2026-09-03 on CORRECTED GROUNDS, and the original size argument is
  > marked SUPERSEDED** — the commit granule is symmetric across both options, the resident floor
  > keys on SCHEMA count rather than FILE count, and `VmColumn` refuses a 40-byte element
  > outright. The arithmetic is in §*2026-09-03* above. Ground: `feat/threadpool-ke16`
  > `gaia/DECISIONS.md` §Data tables.
  >
  > *— and the same ruling as `feat/threadpool-ke16` recorded it on the day, kept by the merge `merge/ke16-into-render`, 2026-09-10. Both are kept because each carries ground the other does not; where the two differ in status, the paragraph above is the later one.*
  >
  > ✅ **RESOLVED 2026-08-30 [delegated] — (a) rows are entities, AMENDED three ways the ballot did
  > not carry.** Full ruling: [`gaia/DECISIONS.md`](gaia/DECISIONS.md) §Data tables.
  >
  > ⚠ **The ballot's phrase "a dense-column archetype" is a category error, and it is the thing the
  > owner's remark caught.** A dense id is signature-excluded, so `get_or_create_archetype(&[DenseRow])`
  > returns **the empty archetype** (measured: `dense_arch == empty_arch → true`) — under the ballot's
  > own wording 4000 item rows would land in the world's component-less bucket beside every bare
  > entity in the game. **(i) Storage is `StorageKind::Table`**, which is what *"data that doesn't
  > have to be dense"* denotes in this engine's vocabulary: a table whose rows carry the row component
  > and nothing else is **one archetype with exactly one `ComponentPool`, already contiguous**, while
  > dense would add `s2e`, an `e2s` sized to the maximum entity id ever inserted, a live bitmap and a
  > free list for a contiguity `Table` already has. The mechanical rule for which kind a table gets:
  > *is the row component carried by entities outside the table?* **(ii) Rows materialize EAGERLY at
  > load**, never lazily — under lazy materialization `save_world` records which rows *happened to
  > have been touched*, so the same game state saves to different files and a round trip is no longer
  > one; and every insert-path mechanism F4 enumerates would fire at an arbitrary later frame.
  > **(iii) A table is loaded by its own explicit load and PINNED**, so `unload_cell` never enumerates
  > it; the runtime handle is an **`Entity` captured at load**, never a row index, because
  > `Archetype::swap_remove` moves rows and loads append into dedup'd archetypes.
  >
  > **The price, measured rather than asserted.** 4000 rows of a 40-byte row type as table entities
  > cost **522 752 B ≈ 511 KiB** marginal against a running world (pool data 262 144 + two tick
  > regions 65 536 each + `entity_ids` 65 536 + inland 64 000), and **0.006%** of the inland store's
  > slot ceiling; the per-frame cost of those idle entities is **one extra archetype mask test per
  > query**, not 4000 row visits. **Rejected (b), and its price:** it saves **129 536 B (25%)**, or
  > 62% *only if a new tick-free VM primitive is also written*, because `VmColumn<T>::new` **panics**
  > unless `size_of::<T>()` divides the commit granule and 40 does not (nor 48, 56, 72). Against
  > 0.13 MB it costs a format version bump, **≥ 607 production + ≥ 444 test lines** by the dense
  > region's own precedent — for a region whose payload shape, unlike dense's, has **no existing
  > analogue** — and **the entire per-`ResourceId` serialize seam from scratch**:
  > `grep -rniw "resource" crates/boyko_serialize/src/` returns **0 occurrences of the word anywhere
  > in the crate, comments included**. It also forfeits queryability, which is the shape Principle 0
  > exists to refuse.
  >
  > ⚠ **Two corpus corrections made in passing:** `load_archetype`/`load_dense_store` are **not** in
  > `boyko_serialize` (they are `pub fn` in `boyko_ecs::…::serialize::load_writer`; the crate's own
  > halves are `load_one_archetype`/`load_dense_region`), so the ballot's "second load path" straddles
  > two crates; and two engine docs contradict each other on the W4 anchor — one says loads go into
  > freshly created archetypes, the other says `create_archetype` **dedups** and the anchor is
  > relaxed. Neither is Gaia's to fix.
- **F3 — the shape of a table file.** Single-file-per-asset only (one document = one asset), or
  ALSO a table file that bakes N rows into one dense column. Price of the second form: the grammar
  grows a row-repetition shape and identity has to name a row INSIDE a file (which is F8's
  territory) — against the authoring win of one document holding 400 items instead of 400
  documents. The ballot is not "one form or two": it is whether both forms bake through ONE schema,
  which is the recommendation — the table file is a spelling, not a second type system. Rejected:
  a table dialect, which the one-language-three-profiles ruling already refuses in the large.
  Blocks the grammar (G5's authoring surface).

  > ✅ **RESOLVED 2026-08-30 BY THE OWNER — both shapes, over ONE schema.** The table file is a
  > **spelling**, not a second type system. Rejected: a table dialect. Added under the standing
  > rule rather than escalated: the DEFAULT shape is chosen mechanically by the LOADING UNIT — an
  > own file when the record is independently referenced and independently loadable/unloadable, a
  > table row when the records are a catalogue always resident together. The shape question is
  > settled by code rather than taste: `create_archetype` DEDUPS by id set and `load_archetype`
  > APPENDS at the archetype's current row head, so N one-row documents and one N-row document
  > land in the SAME archetype, contiguously — file shape is resident-memory-, query-cost- and
  > layout-neutral. ⚠ Two riders, both G2 deliverables and neither a ballot: a locally-stable
  > canonical printer (red fixture — edit row 200 of 400, re-print, assert the diff touches one
  > span), and the deletion of the "dense column" wording everywhere it appears in this corpus.
  > Ground: `feat/threadpool-ke16` `gaia/DECISIONS.md` §Data tables.
  >
  > *— and the same ruling as `feat/threadpool-ke16` recorded it on the day, kept by the merge `merge/ke16-into-render`, 2026-09-10. Both are kept because each carries ground the other does not; where the two differ in status, the paragraph above is the later one.*
  >
  > ✅ **RESOLVED 2026-08-30 BY THE OWNER — both forms, over ONE schema.** The table file is a
  > **spelling**, not a second type system. Rejected: a table dialect. The owner asked for a
  > recommendation on the details, which is answered separately and is not this line. Ground:
  > [`gaia/DECISIONS.md`](gaia/DECISIONS.md) §Data tables. ⚠ Its identity half — how a row is named
  > *inside* a file — is **F8**'s and stays open; F2's ruling constrains only the *runtime* handle
  > (an `Entity` captured at load) and says nothing about the authored identity.
- **F5 — streaming scope, and the alternative nobody wrote down.** The recommendation on the table
  is (a) *format-ready-loader-later*: the cell catalog, its attribution, and the persistent id map
  (GK-1) land NOW as one design unit; `load_cell`, unload, and cross-cell references land at G8.
  **The alternative was never named, which is exactly why it is on this list.** It is (b) — the
  streaming half INSIDE G6: `load_cell`/`unload_cell`, GK-1's cross-load map with a declared
  lifetime, and cross-cell reference resolution, shipped with the scene profile rather than after
  it. Price of (a): a catalog with no loader is a datum nothing consumes until G8 — this
  repository's own recurring defect class — mitigated only by G6's reconstruct-and-compare gate,
  which does read the catalog, so it is a datum with a consumer before the loader exists. Price of
  (b): G6 absorbs the hardest half of the design, because a per-load `LoadEntityMap` cannot express
  references BETWEEN chunks (measured), and the scene profile cannot land until that is solved.
  Blocks G6, through what its catalog is required to contain.

  > ✅ **RESOLVED 2026-08-30 BY THE OWNER — option (b), the alternative nobody had written down.**
  > His words: *"Все сразу грамотно по списку с самого начала."* `load_cell`/`unload_cell`, GK-1's
  > cross-load map with a declared lifetime, and cross-cell reference resolution ship **with** the
  > scene profile. ⚠ **THE RECOMMENDATION PRINTED ABOVE — (a) — IS THE OPTION HE REJECTED**, and
  > this register carried it as the recommendation until 2026-09-03. Ground for (b) over (a): a
  > per-load `LoadEntityMap` cannot express references BETWEEN chunks (measured), and (a)'s
  > catalog-with-no-loader is this repository's own recurring dead-datum class. ⚠ Follow-on
  > finding, NOT a ballot: **GK-1 as specified violates Principle 0** — its requirement 4 makes
  > `LoadEntityMap` a world-owned `Resource` keeping its `Vec`, turning the transient-scratch
  > exception into a durable parallel data system; the in-tree cure is `PathIndex`'s `VmColumn`
  > shape. Ground: `feat/threadpool-ke16` `gaia/DECISIONS.md` §Streaming scope.
  >
  > *— and the same ruling as `feat/threadpool-ke16` recorded it on the day, kept by the merge `merge/ke16-into-render`, 2026-09-10. Both are kept because each carries ground the other does not; where the two differ in status, the paragraph above is the later one.*
  >
  > *"Все сразу грамотно по списку с самого начала."* `load_cell` / `unload_cell`, GK-1's cross-load
  > map **with a declared lifetime**, and cross-cell reference resolution ship **with** the scene
  > profile at **G6**, not after it at G8. **The rejected option's price is the one the body already
  > states** (a catalog with no loader is a datum nothing consumes until G8); **the ruling's price is
  > the larger one and is accepted knowingly** — G6 absorbs the hardest half.
  >
  > ⚠ **This is the single largest change to the campaign's ground, and it lands directly on F4.**
  > Six GK-1 requirements now belong to G6, each from a measured blocker; the two that must not be
  > lost are (i) **insert-after-`finalize` must become legal**, because today a second cell's inserts
  > land in an unsorted tail that `binary_search` **silently misses** in release, and (ii) **the
  > fresh-world contract must be lifted** — `load_dense_store`'s guard is a `debug_assert!` whose own
  > message reads *"merge load is unsupported"*, and it **vanishes in the shipping build**, so
  > `load_cell` would corrupt the dense store silently in release and loudly only in dev. ⚠ **GK-1 and
  > GK-3 must land together** or the cross-cell half is silently half-present: a `ChildOf` across a
  > cell boundary only enters the reverse index when F4's sub-pass 4 fires. Ground:
  > [`gaia/DECISIONS.md`](gaia/DECISIONS.md) §Streaming scope; the rung rewrite is in
  > [`gaia/CAMPAIGN.md`](gaia/CAMPAIGN.md) rows **G6** and **G8**.
- **F6 — the fate of `.ui` once Gaia absorbs it.** (a) migrate the existing `.ui` documents to the
  Gaia `ui` profile and DELETE the old format in the same campaign; (b) freeze `.ui` where it is
  and decide after an owner-eval of a real Gaia HUD. Price of (a): the migration is done blind —
  G7's prerequisites (a windowed UI pass, a real `UiPlugin`) do not exist, so nothing renders a
  Gaia HUD to judge before the old format is gone. Price of (b): two authoring formats for one
  subsystem, which is the diverged-pair cost this repository has already measured on `docs/ru/` —
  a reader cannot tell which is current and finds out by acting on the stale one. Recommendation
  (a). Blocks G7.

  > ✅ **RESOLVED 2026-08-30 BY THE OWNER — (a): migrate to the Gaia `ui` profile and DELETE the
  > old format in the same campaign.** The price is accepted and recorded at G7 rather than
  > discovered later: the migration is done **blind**, because G7's own prerequisites do not
  > exist. What survives the deletion, verbatim: reconcile-by-name+ordinal hot reload (the
  > engine's only apply-to-a-live-world primitive, and the editor's future live preview), the
  > typed leaf parsers, the bind path, bar quantization and the action seam. ⚠ Measured while
  > pricing it, and it strengthens the ruling: `.ui` **cannot author a background colour at all**
  > — `UiBackground` is carried by the Panel/Button/Bar bundles and appears nowhere under
  > `crates/boyko_ui/src/text/`, so the writer silently drops every panel and button fill while
  > the round-trip gate stays green, because the gate's universe is the writer's own emit roster.
  > Ground: `feat/threadpool-ke16` `gaia/DECISIONS.md` §Language shape.
  >
  > *— and the same ruling as `feat/threadpool-ke16` recorded it on the day, kept by the merge `merge/ke16-into-render`, 2026-09-10. Both are kept because each carries ground the other does not; where the two differ in status, the paragraph above is the later one.*
  >
  > ✅ **RESOLVED 2026-08-30 BY THE OWNER — (a): migrate to the Gaia `ui` profile and DELETE the old
  > format in the same campaign.** Rejected: freeze until an owner-eval of a real Gaia HUD, on the
  > diverged-pair cost already measured in this repository. **The ruling's own price is recorded at
  > G7 rather than left to be discovered:** the migration is done **blind**, because G7's
  > prerequisites — a windowed UI pass and a real `UiPlugin` — do not exist. Ground:
  > [`gaia/DECISIONS.md`](gaia/DECISIONS.md) §Language shape.
- **F7 — mods, and the executable the ratified refusal does not mention.** ⚠ **This borders a
  ratified refusal and has to be framed against it, not asked fresh.** What is ratified: *own text
  → build-time bake → binary; reflection only at bake, behind a default-off feature; the shipped
  load path has zero reflection* ([`gaia/DECISIONS.md`](gaia/DECISIONS.md) §Inherited pipeline) —
  a constraint on the GAME BINARY. A text-mod pipeline does not violate it as written: it ships the
  bake tool to players as a SEPARATE executable, and the game still loads only bytes. So the
  question is not "may reflection ship" — that is answered, no — but **whether the refusal meant
  "no reflection in the game binary" or "no bake tooling in a player's hands at all"**. The two
  readings differ only where mods exist, which is why the corpus carried both without noticing.
  Options: (a) ratify "mods are out of v1" explicitly AND record the constraint in its narrow form,
  so a later mod campaign is not blocked by a sentence that never meant to block it; (b) accept the
  separate-executable pipeline now and design the format's stability guarantees for it from the
  start. Price of silence: it is a DEFAULT DECISION — the wide reading calcifies by never being
  contradicted. Blocks nothing today; it decides what a later campaign is allowed to propose.

  > ✅ **RESOLVED 2026-08-30 BY THE OWNER — (a): mods are NOT supported for now, ratified
  > explicitly**, so that silence stops being a default decision; and the reflection constraint is
  > recorded in its NARROW form (no reflection in the GAME BINARY), so a later mod campaign is not
  > blocked by a sentence that never meant to block it. Blocks nothing on the ladder. ⚠ Recorded
  > because it is the proximity-settlement shape this list exists to catch: the ratified
  > §Refusals line *"no external-mod pipeline in v1"* was already in
  > [`gaia/DECISIONS.md`](gaia/DECISIONS.md) while this ballot was OPEN — a ratified line
  > answering half of an open ballot, on a page whose own campaign file forbids exactly that. The
  > ruling makes the line correct retroactively; it did not make it legitimate at the time, and
  > the line now carries this date so it reads as sourced.
  >
  > *— and the same ruling as `feat/threadpool-ke16` recorded it on the day, kept by the merge `merge/ke16-into-render`, 2026-09-10. Both are kept because each carries ground the other does not; where the two differ in status, the paragraph above is the later one.*
  >
  > explicitly.** The ballot existed because silence was itself a decision; it is no longer silent.
  > **What the explicit ratification buys is the NARROW form of the constraint**: what is ratified is
  > *no reflection in the GAME BINARY*, not *no bake tooling in a player's hands at all* — so a later
  > mod campaign is not blocked by a sentence that never meant to block it. **Rejected (b) — accept
  > the separate-executable pipeline now and design the format's stability guarantees for it.**
  > Price: v1's format gains a compatibility surface for a consumer that does not exist, which is the
  > dead-datum class. Price of the ruling, accepted knowingly: if a mod campaign is ever taken, those
  > guarantees are retrofitted rather than designed in — and the *"for now"* in the owner's answer is
  > what makes that a schedule rather than a prohibition. Ground:
  > [`gaia/DECISIONS.md`](gaia/DECISIONS.md) §Refusals.

---

## 2026-08-28 — The `machine` "one transition per frame" claim is FALSE: two same-frame events run BOTH exit/action/enter chains — FIXED 2026-08-30 (rung R2)

Found while designing per-entity machines, confirmed by two independent reads of the emitter and
the schedule. `run_state_transitions` executes ONCE, before the executor loop, so `State<S>` is
constant for the whole frame — while the emitter gives every route its OWN system gated only by
`.run_if(in_state(leaf))`, with no ordering edges and no latch between siblings. Two events of
different types arriving for one leaf in one frame therefore run BOTH exit/action/enter chains, and
`NextState` is decided by the last write. The emitter's own comment (`expand.rs`, the transition-fn
doc) calls this "§5.1 one transition per machine per frame", and the registration-order note claims
declaration order makes two same-frame transitions deterministic — it determines WHICH wins, not
HOW MANY run.

**Scheduled fix (owner-approved direction):** the `state_chart!` move into `boyko_macros` (campaign
rung R2, [`aether-v2/CAMPAIGN.md`](aether-v2/CAMPAIGN.md)) merges a leaf's routes into one dispatch,
which makes a second same-frame chain structurally impossible AND lands the arbitration alignment
(first-declared-wins) the owner chose. **The red test comes first**: two events, one leaf, one
frame → today it must FAIL by observing two chains; after R2 it pins exactly one.

Nothing else is blocked; the global `machine` misbehaves only under same-frame multi-event load,
which the shipped tests deliberately avoid (distinct event types per edge).

**RESOLVED 2026-08-30, rung R2.** The red test is
[`aether_tests/tests/r2_chart_arbitration.rs`](../crates/aether_tests/tests/r2_chart_arbitration.rs),
and it was watched failing before anything moved. MEASURED pre-fix, all five counters at once on
one leaf with `on Alpha => ToAlpha` declared before `on Beta => ToBeta`:

| exit | alpha action | beta action | enter ToAlpha | enter ToBeta | settled state |
|---|---|---|---|---|---|
| **2** | 1 | 1 | 1 | 1 | **ToBeta** |

Two exits, two actions, two enters — and the surviving state was the LAST-declared route's, which
is the arbitration artifact M6 called out. Post-fix the same run reads `1 / 1 / 0 / 1 / 0 /
ToAlpha`.

The fix is the per-leaf **route merge** in `boyko_macros::state_chart!`
(`state_chart::emit::leaf_fn`): one system per leaf drains every lane, a single `__sc_route`
selection takes the first-declared accepting route, and one `match` arm runs the only chain. A
second same-frame chain is now structurally impossible — there is one selection point, so the
question "how many chains ran" has no way to answer anything but one.

Two things a reader should not have to rediscover:

* the emitter comment that made this claim is gone with the emitter — the flattening now lives in
  `boyko_macros`, and Aether lowers to it;
* every lane is still drained even when its route loses, which preserves the pre-merge per-lane
  drain exactly. The kernel's `EventIter` advances the cursor only past what it yielded, so a
  merge that skipped the loser's lane would have left this frame's events to re-fire on the next.

---

## 2026-08-28 — RECORD: **the no-syntax edge class is a LIST, not a closure — a FOURTH member, a residue guarantee WITHDRAWN, and the anti-rot annotation's own prose measured false at 21 sites**

**Status: RECORDED — no new question, and the SCOPE call below is unchanged.** Found by the
**eleventh** adversarial pass, classified *(c) — four positive assertions still false*, and closed by
a code landing again confined to `crates/boyko_ui/tests/ui_a1_source_census.rs` (**3913 → 4134
lines**; no tracked file touched) plus this record. Full detail and every measurement:
[`docs/UI-PLAN-ANIMATION-A1.md`'s A1 landing note, part 11](UI-PLAN-ANIMATION-A1.md#part-11--the-eleventh-pass-classified-c-four-positive-assertions-still-false).

### 1 — the class is a LIST, not a closure, and the FOURTH member is DESUGARING

`for x in it` is a walked node; the `Iterator`/`IntoIterator` impl behind it is **not**. The same
shape reaches `From` through `?`, `Display` through macro interpolation, and `Deref` through `*`. The
probe — an `impl Iterator` carrying a fourth termination condition, driven from the opacity arm —
left the crate at **EXIT=0, 53 targets, 354 passed, identical to the clean tree**, *including after
its one `SITES` row was paid*. Widened rather than filed as residue, because it measured free:
`DESUGARED_IMPLS` is a **sibling table** over 8 traits, served by the same scanner, so
`OPERATOR_SITES` stays **43** and `OPERATOR_IMPLS` stays **3 over 28** and no external citation of
those numbers was invalidated. Population, measured with the table empty: **ONE**.

⚠️ **The transferable half.** Four members across four passes — drop glue (8th), hook registration
(8th), operator dispatch (10th), desugaring (11th) — **and every one was found by a PROBE, never by
construction.** Nothing here derives the class from the language, so a green run means *"none of the
four known no-syntax edges is unaccounted for"* and never *"there is no no-syntax edge"*. Costs:
drop glue and operator dispatch **zero rows** each, desugaring **zero rows for the body**.

### 2 — residue item 14's guarantee is WITHDRAWN: the cross-crate population is NOT zero

The item that exists to state the cross-crate blind spot asserted the population was **zero**,
"because the region's operands are `f32`/`u32`/`u8`/`[f32;2]`, all primitives". Measured: **at least
five**, all reached on every armed frame, and **two of them carry no operator syntax at the site at
all** — so the withdrawn sentence named the wrong *mechanism*, not merely the wrong count. It is
reached through a **deref**: `Mut`'s, `Res`'s and `ResMut`'s. Details in item 14 below.

### 3 — ⚠️ the anti-rot annotation's own PROSE is the thing that rots, at 21 sites and five descriptions

The tenth pass ruled that the annotation stops asserting currency and becomes a dated transcript. **It
applied the ruling to the pointer and not to the sentence that says where the pointer used to land** —
and the eleventh pass measured every one of those sentences false. The three live coordinates
(`:2857`, `:3105`, `:1696`) all still resolve; **the descriptions of the discarded values do not.**

* **The ripple that the tenth pass measured for the live pointer hit the description in the same
  sentence.** Its own warning — *"a document cannot record a coordinate into itself without moving
  it"* — was written about `:2506 → :2706`, a **+200** drift in four steps inside one landing. Part
  10's description of the *stale* value sat two lines above that warning and moved by the same 200:
  read at `:2530` on the tree part 11 inherited, the D7 prose part 10 quoted for it stood **200 lines
  below**. *(Quoted as content with a tree attached, never as a line: writing this record moved the
  lines again, which is the finding restated as a fact about this very paragraph.)*
* **Two of the three descriptions are *"is a blank line"*, and a blank line cannot be re-resolved by
  content, because it has none.** The repair this campaign prescribes for a rotted anchor — open the
  target and read it — is **structurally inapplicable** to exactly the descriptions that rotted
  hardest. For those, a dated transcript is not the better form; it is the only available one.
* **The population was undercounted by every pass, and the reason is mechanical.** Part 10 wrote
  *three*; the eleventh pass's refuter counted *thirteen*; enumerated by grep over the five
  documents, the answer is **21 carriers of five false descriptions**. **3 counts the coordinates,
  13 counts the documents each bullet names, and 21 counts the sentences** — and only the last can be
  false. A grep for the *coordinate* misses [`UI-PLAN-SPRITES-DECISIONS.md` S-D20](UI-PLAN-SPRITES-DECISIONS.md#s-d20--the-s6-pre-build-audit-the-cursor-hole-closes-with-a-hook-and-six-of-the-rungs-own-sentences-did-not-survive-the-tree) — pre-split `UI-PLAN-SPRITES.md:1647` — entirely, because that site <!-- doc-anchor-ignore -->
  names no stale coordinate at all; a grep for the *document* misses the second copy each
  `OPEN-QUESTIONS` twin carries.
* **Two of the five were in nobody's list.** `UI-PLAN-SPRITES.md` said `UI-PLAN-SPRITES.md:663` is *"today a sentence <!-- doc-anchor-ignore -->
  about `UI_FALLBACK_MAX_DELTA`"* — it is not, and that carried the word "today", the strongest
  currency claim in the cluster. And part 10 wrote that `UI-PLAN-SPRITES.md:1656` *"is today the F2 ordering-axis <!-- doc-anchor-ignore -->
  red-ledger row"* **in the same sentence in which it disclaimed the quote it was correcting** — the
  wrong line and the wrong ledger row.

**The generalisation.** Every pass treated *the anchor* as the thing that rots and re-aimed it. A
re-aim writes **three** claims, not one: the new coordinate, the old coordinate's content, and often
a passing remark about a third line. **Only the first is ever re-read, and only the first has never
been found false. The rot is not in the pointers this campaign maintains — it is in the prose it
writes about them, which nothing has re-read even once**, and the descriptions outnumber the
coordinates **21 to 3**. This reinforces the SCOPE call already filed below rather than raising a new
one: nothing here is mechanical until the **594** bare-basename citations become link form.

### 4 — ⚠️ the hazard that is not about this rung: A KILLED AGENT LEAVES ITS PROBE APPLIED

Three instances in one day across two lanes. One was a one-token swap the suite is green over *by
construction*; one was a loud panic nothing ran to see; and one sat **between an `#[allow]` attribute
and the function it was written for, silently re-attaching the attribute to the probe struct**.
`git status` showed nothing (the probe was inside an already-modified file), the untracked list
showed nothing, and a name-based grep misses it unless you guess the name right. **The check that
caught all three was `git diff --numstat` against a known baseline.** The corollary: every figure in
the reports written before the removal had been measured against a tree nobody re-measured, so the
eleventh pass re-read all of them from the instrument rather than carrying one forward.

---

## 2026-08-28 — RECORD: **the A1 residue is FIFTEEN, not thirteen — a third execution edge with no syntax at the call site, and the campaign's own anti-rot annotation measured FALSE**

**Status: RECORDED — three items, none of them a new question.** The SCOPE call the item below files
is unchanged and is reinforced by item 3. Found by the **tenth** adversarial pass over
[`docs/UI-PLAN-ANIMATION-A1.md`'s A1 rung](UI-PLAN-ANIMATION-A1.md#a1--the-sink-the-four-channels-the-fused-tick--size-l--no-cross-plan-dependency) and closed by the **tenth** landing. Full detail and every
measurement: that document's A1 landing note, part 10.

### 1 — the class, extended: the third execution edge is an OPERATOR, and it costs NOTHING to hide there

The eighth pass found two edges with no syntax at the call site — drop glue and a kernel-registered
hook. **The tenth found a third, and it is cheaper than both.** `Walk::visit_expr` matched
`Expr::Binary` only for `And`/`Or`; `Index`, `Unary`, `Assign` and `AssignOp` matched no arm at all.
An overloaded operator therefore calls a body that is **not a branch node, not a call node, not a
callee, not a call site**, and carries no `Drop`. A `Mul` impl holding a fourth termination
condition, invoked from the opacity arm, left every headline count **bit-identical** and the whole
crate at EXIT=0; an `Index` impl — a structurally different node — did the same. **The recorded
cross-module escape costs two rows and the factory escape costs two rows; this one costs zero.**

Dispositioned by two scans that are blind to different things, both shipped: `OPERATOR_SITES` pins
all **43** operator expressions of the walked region, and `OPERATOR_IMPLS` pins every user operator
`impl` in `boyko_ui/src` over **28** traits. ⚠️ **"Closed" was the wrong word and the eleventh pass
proved it: a FOURTH edge — DESUGARING — walked straight past both scans**, and the class now carries
a THIRD scan (`DESUGARED_IMPLS`, 8 traits, measured population **one**). **The class is a LIST, not a
closure** — four members across four passes, every one found by a probe rather than by construction,
and nothing derives it from the language. See the eleventh-pass record above. ⚠️ **The pass that
prescribed the second scan asserted
the crate holds zero such impls. It holds three** — `PartialEq for UiTextBuffer`, `PartialEq for
UiVisual`, `PartialOrd for UiName` — and the middle one is AD11's bitwise sink equality, five `&&`
and four `Index` operations, reached by `set_if_neq` from the walked region's **last line**. It is a
disposition rather than a hole (`the_sinks_equality_is_idempotent_under_nan` gates it), but *a
predicted-zero population that measures three is the same defect as a coverage table nobody planted
a `panic!` into.*

Two further things the ninth landing asserted in the positive were measured **false** and are fixed
rather than caveated: a nested item was recorded as one node and **not walked**, under a comment
claiming the definition table compensated — the compensation is call-gated and there is no call; and
a witness could be **laundered through the allowlist**, one row buying back "reachable" without
buying back execution. The sentence *"a witness that exists and is never called … reds exactly as
loudly as one that was deleted"* is **deleted at all three sites where it appeared**, not narrowed.

### 2 — the residue is FIFTEEN, and item 9's stated REASON was refuted

Source of truth: `crates/boyko_ui/tests/ui_a1_source_census.rs:168-363` (the `# THE RESIDUE`
section; `:168` is its heading, `:363` its last line — *read at its target 2026-08-28, part 11. This
record said `:148-300`, correct on the 3913-line tree it was written against and stale within hours
of part 11's landing; on the 4134-line tree both ends land on plausible content*). Items 1–13 are
unchanged in substance. **Item
9 kept its disposition and lost its reason**: it excused straight-line code as introducing *"no path
and no callee"*, which an operator overload refutes — it introduces both while looking like
straight-line code at the site. It now reads *"straight-line code that is neither a call nor an
operator"*, and the line is drawn by construct.

* **14 — a no-syntax `impl` that is in ANOTHER CRATE.** `lerp1(from: f32, …)` becoming
  `lerp1(from: Px, …)` leaves `from + (to - from) * t` printed identically while `+` starts calling
  `Px`'s `Add`. This is item 1's class reached through an operator instead of a call. ⚠️ **This
  item's guarantee was WITHDRAWN by the eleventh pass, and it is the item that exists to state it.**
  It said *"measured population: zero cross-crate, because the region's operands are `f32`, `u32`,
  `u8` and `[f32; 2]` — all primitives, which is exactly what makes a first non-primitive one red the
  expression pin."* **The operand list is wrong and the population is at least FIVE**, every one
  reached on every armed frame: three auto-derefs (`*sink` on `Mut<'w, UiVisual>`; `clock.dt_real()`
  on `Res<'_, UiClock>`; `done.done.push(…)` on `ResMut<'_, UiTweenScratch>`) and two cross-crate
  `Iterator` bodies driven by `for` (`q.iter_entities_mut()`, and `&done` via `impl IntoIterator for
  &Vec<T>`). **Two of the three derefs carry no operator syntax at the site at all**, so no widening
  of `OPERATOR_SITES` could have listed them — the withdrawn sentence named the wrong *mechanism*,
  not merely the wrong count. What holds, as a property and not a promise: *`OPERATOR_SITES` reds
  when the printed TOKENS of an operator expression of the walked region change — that and nothing
  more.*
* **15 — the observer scan's SCOPE is `CARGO_MANIFEST_DIR/src`.** A registration from a sibling
  crate leaves *"0 observer registrations"* true and irrelevant. **Measured population: zero, so it
  is latent and not live** — no crate outside `boyko_ecs` calls any `OBSERVER_VERB`, and every
  cross-crate `#[component(on_*)]` in the tree sits in `boyko_ecs` fixtures or `aether_lang`
  expander tests, none naming a `boyko_ui` path. The hook scan shares the scope but covers all four
  hook kinds within it.

**Items 1 and 6 remain the two a next pass should attack**, unchanged.

### 3 — ⚠️ the campaign's own anti-rot annotation, *"re-measured by CONTENT"*, is UNCHECKED PROSE and it measured FALSE

This is the part that generalizes past this rung. Three sweeps adopted the annotation as the
discipline that replaces arithmetic offsets, and it is a **positive assertion that nothing checks**.
Measured over the whole corpus rather than sampled: **41 annotation sites across seven documents**,
and the false population is **larger than the four the pass reported**.

* **Six sites in [`UI-PLAN-ANIMATION-A1.md`](UI-PLAN-ANIMATION-A1.md#a1--the-sink-the-four-channels-the-fused-tick--size-l--no-cross-plan-dependency) (rows E1, H1, H1-arm, H4, H8, and two prose paragraphs)**
  carry part-7 coordinates into `crates/boyko_ui/tests/ui_a1_zero_alloc.rs` that part 9's own
  landing invalidated. **E1 alone was stale at thirteen coordinates, eight of them on plausible
  content** — `:297` is `const VIRTUAL_SPEED: f32 = 0.5;`, *a different `const`*; `:509` is a
  `.collect::<Vec<_>>()`; and `:839`, cited as the per-index comparison loop, is `fn armed_once(` —
  **the right shape for a neighbouring row of the same list.**
* ⚠️ **The row whose whole subject is stale anchors had itself FORKED.** H4 exists to record where
  the per-index `min` really is. Part 7 moved it to `:640` and updated the **E1** row but not this
  one, so for two parts the document carried two different answers — `:531` here, `:640` there — and
  part 9 invalidated both. True today: `:980`. **A document that repairs one citation of a fact and
  not its twin has forked the fact, and a sweep that walks ROWS cannot see that; only one that walks
  FACTS can.**
* **Three cross-document coordinates, two of them cited four times each.** The dead
  `benches/ui_animation.rs` path — true [`UI-PLAN-ANIMATION-A2-A8.md` A8](UI-PLAN-ANIMATION-A2-A8.md#a8--the-measurement-rung-what-the-tick-actually-costs--size-s--depends-on-a1a5) (pre-split `:2857`); the D7 owner self-citation — <!-- doc-anchor-ignore -->
  true [`UI-PLAN-ANIMATION.md` §8](UI-PLAN-ANIMATION.md#8--dependencies-stated-explicitly) (pre-split `:3105`); and that document's sibling self-citation, rotted a **fourth** time and this time OUT <!-- doc-anchor-ignore -->
  of content — true [`UI-PLAN-ANIMATION-A1.md` A1](UI-PLAN-ANIMATION-A1.md#a1--the-sink-the-four-channels-the-fused-tick--size-l--no-cross-plan-dependency) (pre-split `:1696`). ⚠️ **Part 11 re-read all three at their targets 2026-08-28: the three <!-- doc-anchor-ignore -->
  LIVE values still hold. What had gone false is the description of each STALE value** — part 10
  wrote that `:2282` and `:1673` *"are blank lines"* and `:2530` *"is prose about an unrelated <!-- doc-anchor-ignore -->
  axis"*, and none of those three sentences is true on today's tree. **They are anchors into the same
  moving document, and the ruling below was applied to the pointer and not to them.** Measured: part
  10's D7 quote stood exactly **200 lines** below the value it describes — the same ripple
  the warning two paragraphs down measures for the live pointer, arriving at the description that sat
  in the same sentence. Full accounting in [`UI-PLAN-ANIMATION-A1.md`'s Cluster B](UI-PLAN-ANIMATION-A1.md#u4--the-anti-rot-annotation-is-unchecked-prose-and-it-was-false-at-more-sites-than-the-pass-found).
* **Eight census coordinates went stale inside the tenth landing itself**, within hours, because it
  took `ui_a1_source_census.rs` from 3181 to 3913 lines. Every one of part 9's values was correct on
  the tree part 9 read; every one is plausible content now. **The `ADVANCE_PIN` / termination-test
  pair has now been re-aimed by parts 8, 9 and 10 and landed on plausible content each time.**

**The ruling, and it is not a widening.** The annotation **stops asserting currency**. Every site now
reads as *what was read, when, and by which part* — a dated transcript, under the same rule this
campaign already applies to red ledgers — and the rows most exposed additionally say in words that
nothing gates them. It is deliberately **not** made mechanical here, and the reason is measured
rather than asserted: the existing `internal_docs_anchors` gate widened to these five documents
produces **405 misbindings out of 541 flags**, and its sensitivity control moved the stale count
**92 → 92** when a real-rot site was repointed 999 458 lines past EOF (item 1 of the SCOPE record
below). The prerequisite is the **594** bare-basename citations that must become link form first —
**that is the owner-facing budget, and it is unchanged.** Until it lands, a dated transcript is the
honest form.

⚠️ **One more thing, measured on this landing rather than argued.** Writing this record moved the two
cross-document coordinates **four times**: `:2506`/`:2754` → `:2686`/`:2934` when the landing note <!-- doc-anchor-ignore -->
was inserted, → `:2696`/`:2944` when the warning about the first move was added, and → <!-- doc-anchor-ignore -->
**`:2706`/`:2954`** when a seventh finding and this sentence were added. **A document cannot record a <!-- doc-anchor-ignore -->
coordinate into itself without moving it, so the re-read has to be the LAST action of a landing,
never a step inside it** — and the fix for the ripple is to make the last edit
**line-count-neutral**, which is what finally converged this one.

---

## 2026-08-28 — RECORD: **rung A1's source census now claims only what it MEASURES, and the difference is a THIRTEEN-item residue** — plus, the repository's anchor gate is green over a file set DISJOINT from this landing

> ⚠️ **Superseded in two numbers by the record above, 2026-08-28 (tenth pass): the residue is
> FIFTEEN, and the census section it points at is now `:148-300`, not `:126-221` — `:126` is today a
> module-doc line.** Everything else in this record stands as measured.

**Status: RECORDED — three items, none of them a new question.** Item 3 restates the SCOPE
recommendation already open in the item below, with its price re-measured, so it can be weighed
without opening a test file. Found by the **eighth** adversarial pass over [`docs/UI-PLAN-ANIMATION-A1.md`'s
A1 rung](UI-PLAN-ANIMATION-A1.md#a1--the-sink-the-four-channels-the-fused-tick--size-l--no-cross-plan-dependency) and closed by the **ninth** landing. Full detail and every measurement: that document's A1
landing note, parts 8 and 9.

### 1 — the class, because it is reusable: an execution edge can have NO SYNTAX AT THE CALL SITE

A1's defence is a **source census** — it parses `crates/boyko_ui/src/animation.rs` and asserts that
the set of control-flow nodes, callees and call sites is exactly a written-down table, so a new
member fails the build by EXISTING rather than by being noticed. Seven passes hardened it against
things *written* in the file, the last of them replacing a regex scanner with an AST walk. The eighth
pass found the edges that are written nowhere:

* **`impl Drop`.** A fourth termination cap — same threshold as a macro form the census DOES catch —
  placed in a guard object dropped at the end of the opacity arm. Drop glue is called by no syntax at
  all, so every census was blind: **EXIT=0, 53 targets, 349 passed.**
* **A kernel-registered hook.** `ui_visual_sink_on_add` runs on every `Tween*` insert; its
  registration is an attribute one module over, written INSIDE a `macro_rules!` body. It appeared in
  no census — no walked entry, no site row, no call row — while already carrying an unenumerated live
  branch. A second branch plus a heap allocation planted in it: **EXIT=0, 349 passed.**
* **And an "EXECUTABLE witness" was satisfied by MENTIONING its name.** One token —
  `witness_…(&mut world, …)` → `let _ = witness_…;` — left five covered paths executed **zero** times
  at EXIT=0, because the reachability scan was a *mention set*, not a call graph.

All three are closed by measurement: three new tests, and the whole-crate comparator that read
EXIT=0 / 349 passed now reads **EXIT=101, 53 targets, 351 passed, 1 failed**. **The transferable part
is the question, not the fix** — *what calls this code where I cannot see a call?* Drop glue, hooks,
observers, trait dispatch selected by type, macro expansion. Each is an edge no syntactic gate sees.

### 2 — the residue: THIRTEEN things the instrument does not measure, written down at the instrument

**This is the rung's honest close, and it is the part worth carrying to other gates.** The census's
claim used to be *"every executable branch of the A1/A0 systems and their intra-file callees"* — a
claim about a **runtime reachability set**, checked by a **syntactic walk of nine bodies in one
file**. Two of those are not the same thing, and eight passes' worth of defects lived in the gap. The
claim now describes the walk, and the gap is an enumerated list instead of a sentence asserting
closure. *(The previous spelling of that section ended "this is now the WHOLE residue" and was wrong
three separate ways.)*

Source of truth, with each item's full reasoning:
`crates/boyko_ui/tests/ui_a1_source_census.rs:148-300` (the `# THE RESIDUE` section; `:148` is its
heading, `:300` its last line — *read at its target 2026-08-28 by the tenth pass; this record was
written with `:126-221`, and `:126` is today a module-doc line*). Digest, **for THIRTEEN items; the
list is fifteen — see the record above**:

1. **Branches inside a callee in another module or crate.** Enumerating a callee makes its branches
   *enumerable*, not *visible*. MEASURED: a `pub(crate) fn zz_cap` in `components.rs` called from the
   opacity arm reds two census tests — and **adding the two rows those failures ask for gives EXIT=0,
   352 passed, with the cap still live**, because neither the `local` flag nor the `note` prose is
   checked against anything. *The answer to a new cross-module callee is a place to write a sentence.*
2. **Trait impls selected by type.** Nothing here resolves a receiver's type or a blanket impl; the
   drop scan is over-approximate in the fail-closed direction precisely because it cannot resolve.
3. **Macro EXPANSIONS.** The invocation is a node and the walk descends into a parseable body, but
   what a `macro_rules!` body expands to is not. An unparseable body is a RED, not a silent skip —
   fail-closed, not sighted.
4. **`#[cfg]`-dead code is COUNTED.** MEASURED: a branch under a feature that exists in no profile
   reds the census and demands a coverage row. Right direction, **at the price of a row whose
   justification cannot be falsified.**
5. **A `Drop` impl the scan cannot reach** — in another crate, inside a `macro_rules!` body, or on a
   type obtained from `make_guard()`. The scan sees struct literals and `T::assoc(…)`, nothing else.
6. **A coverage row pins an OBSERVABLE, not an EXECUTION COUNT.** MEASURED: a one-token edit
   (`let mut shift = 0;` → `= 32;`) drives a loop body to zero iterations while its node key does not
   move, and **both the census and the allocation gate stay green**. One unrelated test elsewhere
   catches that particular neuter, so it is a coverage-column defect, not an escape. **The exposure
   is concentrated and now printed rather than folklore: one witness is the sole defence of
   SEVENTEEN of the 50 paths and observes four liveness counts.**
7. **The weaker coverage variant** asserts the named `#[test]` *exists*; it does not assert that gate
   would fail if the path stopped executing. Both rows using it carry a hand-taken measurement of
   exactly that.
8. **The PATH decomposition of a node** is authored, not derived. What is mechanical is that no node
   may exist without a row and no row without a node.
9. **Straight-line code that calls nothing** — `*elapsed += dt;` introduces no path and no callee.
10. **Row ORDER.** Two structurally identical nodes are told apart only by position; reordering reds,
    but the message says "edited", not "reordered".
11. **String CONTENT.** Literals are emptied before comparison, so a message that has become FALSE
    does not red. This has already bitten once.
12. **`macro_rules!` bodies as trees.** Pinned as printed TOKENS — whitespace-, comment- and
    line-ending-immune, which raw bytes would not be — never as a tree.
13. **Anchor currency outside this crate** — item 3 below.

**Items 1 and 6 are the two a next pass should attack.** Closing 6 means witnesses that assert *this
path executed N times*, which needs per-branch instrumentation the fixture does not have; closing 1
means checking a callee's prose against its body. Neither is cheap, and neither is pretended closed.

### 3 — the anchor gate's green is about OTHER FILES (P6), and what gating the UI plans costs

Re-measured for this record on this tree, each number from the command beside it.

* `cargo test -p boyko-engine --test internal_docs_anchors -- --nocapture` ⇒ **EXIT=0, 5 passed**;
  `ARCHITECTURE.md` 6 + `FEATURE_MAP.md` 222 + `MESHLET-VIRTUAL-GEOMETRY-PLAN.md` 177 +
  `SYSTEMS.md` 330 = **735 anchor(s) checked, 0 stale**.
* **Line-numbered citations from those four documents into any of this landing's TEN moved source
  files** — `gather.rs`, `ui_a1_sink_reaches_discovery.rs`, `animation.rs`, `layout.rs`, `sprite.rs`,
  `text/measure.rs`, `miri_a1_tween.rs`, `ui_a1_tween.rs`, `ui_a1_zero_alloc.rs`,
  `ui_a1_source_census.rs`: **0, 0, 0, 0, 0, 0, 0, 0, 0, 0** (`grep -oE "<basename>:[0-9]+"` over the
  four). The member-table `(N)` form cannot reach them either — no `**File:**` header in the four
  names any of the ten. The only contact of any kind is **two link-form path mentions** of
  `crates/boyko_ui/src/layout.rs` (`FEATURE_MAP.md:137`, `SYSTEMS.md:2162`), checked for EXISTENCE,
  carrying no line number.
* **The gate is NOT vacuous, measured here rather than transported.** `SYSTEMS.md:597`/`:599`'s two
  live anchors swapped (`enable_store.rs:206` ↔ `:299`) ⇒ **EXIT=101**, *"docs/SYSTEMS.md: 330
  anchor(s) checked, 2 stale"*, each message naming the symbol it failed to find. Restored by inverse
  `Edit`, `cmp` EXIT=0, SHA-256 identical, `git status` does not list the file, gate back to EXIT=0.

**So "735 anchors, 0 stale" is true and says nothing whatever about the sixteen files this landing
changed.** And part 9 supplied the counter-example in the same session: the only file this round
moved is `ui_a1_source_census.rs` (2267 → 3181 lines), and **eight coordinates citing it inside
`UI-PLAN-ANIMATION.md` went stale — every one onto plausible content — including three test names the
rewrite had renamed out of existence** (`grep -c` = 0 for each). A renamed test is the rot a
coordinate sweep cannot repair by re-reading a line: the citing *text* is wrong, not just its number.
None of it is visible to a green anchor gate, because the gate does not read that document.

**⚠️ And a correction to the sweep doctrine this campaign adopted three passes ago: driven by the
diff is NECESSARY and NOT SUFFICIENT.** Part 7 replaced "sweep the anchors you TOUCHED" with "sweep
the anchors your landing MOVED" — computable, bounded, mechanical, and right. Part 9 found the class
it leaves out. Two coordinates into `crates/boyko_ui/tests/ui_a1_zero_alloc.rs` — **a file this round
did not move at all** — were stale and both had landed on plausible content: `:203`, cited as a named
constant, reads `const WARM_FRAMES: usize = 8;`, a different constant eighteen lines above it (true:
`:221`); `:685`, cited as a pointer, reads a doc line of an unrelated test (true: `:46`). **A
coordinate that goes stale in landing N and is missed there survives every later sweep**, because
each later sweep is scoped to its own diff and that file is not in it. The shift-driven scope filters
*new* rot and has no term for *inherited* rot — and inherited rot is the half that accumulates. The
eighth pass's `docs/**` sweep (197 spans, 2 out of range) could not see these either: **both are in
range.** The honest scope is the union — *what this landing moved* plus *a rotating re-read of the
rest* — and only the first half is mechanical.

**The recommendation, with its price.** Unchanged from the item below — **(1a): convert the five UI
documents' bare-basename citations to link form FIRST, then widen `GATED_DOCS`.** Widening without
that step is measured below to produce 405 misbindings out of 541 flags, i.e. noise, not rot. The
conversion is the whole cost, and it is now sized: over the five documents there are **825**
line-numbered citations, of which **594 are bare basenames** with no path component and **231**
already carry one *(measured 2026-08-28 on this tree, after part 9's landing:
`grep -oE '[A-Za-z0-9_./-]+\.(rs|toml|md|hlsl|spv):[0-9]+'` per document, split on whether the match
contains `/`)*. **594 citations is the document-surgery budget the owner is being asked to approve**,
and it buys the weaker in-file property (item 2 below explains why a `cargo test` cannot check the
real one) rather than the class this campaign keeps hitting.

---

## 2026-08-28 — SCOPE: **widening `GATED_DOCS` to the UI plans does NOT close the anchor-rot class — measured — and the class may not be closable by a test at all**

**Status: OPEN — four items. Item 1 supersedes the 2026-08-27 item's option list below and needs an
owner call; items 2–4 are recorded, not asked.** Found by the eighth adversarial pass, whose whole
job was to close the anchor class rather than its instances.

### 1 — the measurement that refutes the options already on the table

The 2026-08-27 item below offers "add the five and repair the 96" or "add them and waive the
backlog". Both assume the 96 are ROT. **Re-measured on today's tree, they are not.**

`GATED_DOCS` widened to the five UI documents, run unpiped, then the test file restored by `cp` and
proved byte-identical by `cmp` + SHA-256:

| | anchors | flagged |
|---|---|---|
| the four already gated | **735** | **0** |
| the five UI documents | **618** | **541** |

The 618 is not the 143 the older table records: this landing added ~756 lines to
`UI-PLAN-ANIMATION.md`, which took that document alone from 4 bound anchors to 463. The
decomposition is what matters:

| class | count | what it is |
|---|---|---|
| **MISBOUND** | **405 (74.9 %)** | the gate checked a file whose basename **does not appear on the citing line at all** |
| shape-only | 132 | "is not a definition" — the gate models an anchor as pointing at a DEFINITION; these plans cite EVIDENCE lines |
| out of range | 4 | **and all four are misbindings too** |

The misbinding is structural, not incidental. This gate binds an anchor to the nearest resolvable
**path** mention to its left; the UI plans cite **bare basenames** in prose (`` `gather.rs:272` ``,
`` `boyko_render/Cargo.toml:80-82` ``), which are not path mentions, so anchors attach to whatever
was last linked. Dozens of `ui_a1_zero_alloc.rs` citations were checked against
`crates/boyko_render/Cargo.toml`. The four "past end of file" findings are the same defect: the gate
reported `components.rs:1117` and `ui_a1_tween.rs:1116` past EOF *"(996 lines)"* — that is
`animation.rs`'s length; `components.rs` is 1310 lines and `ui_a1_tween.rs` is 1301, so **both
coordinates are in range and the verdict was about the wrong file.** Same for
`UI-PLAN-ANIMATION.md`'s self-citation — `:1673` when the gate ran — judged against `gather.rs`'s <!-- doc-anchor-ignore -->
541 lines in a document that was **2564** lines at the time. *(Both halves are transcript figures and
both have since moved: part 11 re-read `gather.rs` at **541**, unchanged, and the plan at **3111**
lines, and the self-citation is now `:1696`. Dated rather than updated, because the sentence is about <!-- doc-anchor-ignore -->
what the gate reported, not about the tree today.)* The gate's own module doc records this exact
failure from the meshlet plan's
first attempt — *"83 'stale' of 146, dominated by misbindings"* — and names the prerequisite that
fixed it: converting the citations to link form.

**And the sensitivity control says widening would not have caught the rot this pass actually
found.** `gather.rs:272` — cited four times in `UI-PLAN-SPRITES.md` for `text_uv: None`, whose true
line is `:389` — was repointed to `gather.rs:999999` with the gate widened. Anchor count <!-- doc-anchor-ignore -->
**130 → 130**, stale count **92 → 92**, and no report mentions it. A coordinate 999 458 lines past
the end of a 541-line file, at one of the four real-rot sites, is **completely invisible**.

So the options are: **(1a)** convert the five documents' bare-basename citations to link form
*first* — the meshlet plan's own prerequisite, a document-surgery rung with its own budget — and
only then widen; **(1b)** widen anyway and blanket-`~` the 541, which buys the in-file bounds check
and nothing else, on documents where the bounds check is already proven blind to the real rot;
**(1c)** leave the five ungated and record why. **This pass recommends (1a) and did not weaken the
check to make the corpus quiet.**

### 2 — the class may be structurally ungateable by a `cargo test`

Rot here is *"the coordinate no longer holds the content it was written against"*. That is a
property of a **diff** — it needs the tree the citation was written against and the tree today. A
test sees one tree, so it can only check the weaker property *"the coordinate holds something
definition-shaped that matches a symbol on the citing line"*. Every sweep in this campaign has been
manual for that reason, not for lack of diligence. If the class is to be closed mechanically it
belongs in a **pre-commit / CI diff check** — "for every file this change moved, re-verify every
`docs/**` citation into it" — which is exactly the sweep this pass ran by hand, and which took a
line-shift table plus a basename-exact matcher.

### 3 — the scope rule that produced three consecutive misses, stated so it is not re-derived

Three sweeps in a row scoped themselves to **documents the landing edited**. Rot is a property of
the **(target file, coordinate)** pair, so the correct scope is **every document that cites a file
the landing MOVED, wherever it lives**. Measured this pass over all of `docs/**` (224 markdown
files): **218 citation spans / 220 coordinates** into the sixteen files this landing changed, of
which **36 were inherited coordinates the landing invalidated** — spread across `UI-PLAN-SPRITES.md`,
`UI-PLAN-INTERACTION.md`, `UI-PLAN-AETHER.md`, `UI-ADVANCED-RESEARCH-ANIMATION.md`,
`AUDIT-2026-07-PLAN.md` and three `docs/archive/` plans, **none of which any previous sweep opened**.

### 4 — two findings in passing, neither asked

* **[`UI-PLAN-SPRITES-S5.md`](UI-PLAN-SPRITES-S5.md#s5--sprite-sheets-and-the-flipbook--size-m) §S5's probe census rests on a stale list length.** It says *"the list holds
  six pack inputs today"* and derives `ui_pack_inputs!(count) + 1` = **7.00 probes/node/frame**.
  Measured: `__ui_pack_inputs_list!` (`gather.rs:126-138`) holds **seven** members — S5's
  `UiSpriteSheet` landed after the S4 build that paragraph measured. The `7.00` is left as measured
  and the derived increment is owed a re-measurement before that rung reports leg 10.8(c).
* **A "correct" verdict expires.** The seventh pass's M9 measured this repository's two census
  citations and reported them CORRECT — and they were, on the tree it read. The same landing that
  drifted M9's own prose coordinates rewrote `ui_a1_source_census.rs` from 1379 to 2267 lines, and
  `:705` / `:733` became `Path` fields inside `SITES` — **plausible content**. Both re-aimed this <!-- doc-anchor-ignore -->
  pass to `:1150` / `:1188`. A verification result is only valid against the tree it was taken on, <!-- doc-anchor-ignore -->
  and the tree moved between the review and the landing.

---

## 2026-08-27 — The event participant context is a DEAD DATUM: computed, leaked, stored, never read — RESOLVED 2026-08-28

Found while designing the Aether sugar for `event`, when the owner asked whether a participant could
carry query-style filters (`victim: entity(with Health, without Invulnerable)`). Before answering I
followed what today's `entity(Health)` actually reaches, and it reaches nothing.

The chain is complete and every link is real:

```text
Aether  entity(Health)
  -> #[participant(components = "Health")]
  -> ParticipantInfo { name, required_components: &[Health::component_id()] }
  -> EventTypeInfo::participant_info                    (event_registry.rs:126, :175)
  -> pub fn get_event_participants(event_id)            (event_registry.rs:272)
  -> (nothing)
```

**Grep over the whole workspace** (`crates/`, `--include=*.rs`): `required_components` occurs
**once** — its own field declaration at
[`participants.rs:28`](../crates/boyko_ecs/src/ecs/core/events/participants/participants.rs). `get_event_participants` has
**zero callers**. Not in the dispatcher, not in `EventReader`/`EventWriter`, not in a `debug_assert`,
not in a test, not in a bench.

So every event registration pays for it — one `<Comp as Component>::component_id()` per context
component, a `&'static` leak per participant list behind a `OnceLock` — and no code path consults the
result. This is the class already recorded as recurring in this repo (five prior instances); this is
a sixth, and it is on the PUBLIC event surface, which is why it is worth a decision rather than a
silent deletion.

Scope of the check: `get_event_participants` is `pub`, so an out-of-tree consumer is possible in
principle. Inside the engine, its tests and its benches there is none.

**What is worth deciding — three readings, and they are not equally cheap.**

1. **A debug-time assertion.** On `send`, `debug_assert` that the participant entity actually carries
   the declared components. This is what the field looks designed for, it vanishes in release, and it
   turns a whole class of "the event fired but the reader's `get_mut` returned `None`" into a loud
   failure at the send site. Cost: one archetype lookup per participant per send, debug only.
2. **A read-side filter.** Events are double-buffered and cross a frame boundary, so by read time the
   participant may be dead, may have lost `Health`, or may have gained `Invulnerable`. Today every
   reader re-checks this by hand (`let Ok(h) = q.get_mut(d.participants.victim) else { continue }`).
   If the context were live, `EventReader` could skip such events itself and the check would leave
   every reader. This is the reading that would make `without` mean something, and it is the most
   valuable — and the most expensive, because filtering must not cost the readers that do not need it.
3. **Documentation, and say so.** Keep it descriptive, and write in the doc comment that it is
   never consulted — so the next person does not spend the same half hour looking for the consumer.

**What it blocks.** The Aether sugar `entity(with A, without B)` is on hold until this is answered.
Extending a list nobody reads would double the dead datum, and it would ship a syntax that *looks*
like a guarantee while giving none — which is worse than not having it. Reading 2 additionally needs
engine work (`#[participant]` grows a second channel, `ParticipantInfo` a second slice) that should
not be started before the first slice has a consumer.

Nothing in the engine is blocked: events dispatch correctly today, because none of this is on the
dispatch path. What is blocked is the language surface above it.

**RESOLVED 2026-08-28 — reading 1, scoped to the machine event router (owner delegated the call).**
The per-entity machine design gives the datum its first consumer: the generated router that
deposits an event into the victim row `debug_assert`s, in debug builds only, that the victim
actually carries the declared context components. A stun sent to a crate becomes loud at the send
site; release cost is zero; a miss stays the safe silent `None`. The `without`-filter extension
remains unbuilt (reading 2 stays unfunded until the checked half proves itself). Recorded in
[`aether-v2/DECISIONS.md`](aether-v2/DECISIONS.md) §M4.

---

## RESOLVED 2026-08-27 (owner: option **(a)**) — ⚠️ `add_tag` / `remove_tag` PANICKED IN RELEASE on any entity whose archetype has ever hosted a dense component. Raised 2026-08-26 while landing ECS EG1; FIXED in the kernel as its own change.

**RESOLVED 2026-08-27 — the owner chose (a): fixed in `boyko_ecs`, as its own change, before EG2 and
outside its diff.** The diagnosis below stands as written. What it got wrong is the SIZE — a sweep run
before the fix measured this as a **class, not two instances**:

* **Five** `.expect` sites walk a retained id list into a per-archetype pool lookup: the two named
  below, plus `migrate_entity_remove` Step 1, `migrate_entity_insert` Step 1, and the required-ctor
  pass. All five are `.expect`, so all five are release-present.
* **Two archetype RESOLVERS** — `merged_archetype_id_dyn` and `without_ids_archetype_id` — seed their
  union / difference from the same unfiltered list, so every archetype newly minted through `add_tag`
  / `remove_tag` RETAINED the non-signature id. The kernel was manufacturing the next victim. Their
  generic twins (`merged_archetype_id`, `without_component_archetype_id`) already filtered, which is
  what makes this a restoration rather than an invention.
* **The widest victim class is wider than this entry's.** In `migrate_entity_remove` the entity, its
  source archetype **and the removed component** are all pure table; only the remove TARGET retains,
  and the walk over the target's list is unconditional.
* It is not a *dense* defect but a **non-signature-storage** one — the bitset variant panics at the
  identical site. Hence the shared predicate rather than a `matches!(.., Dense)`.

**Landed:** six guards in `crates/boyko_ecs/src/ecs/core/commands/migration_helpers.rs`, each one
`if !component_registry::is_signature_id(cid) { continue; }` — four walks plus both resolvers. Pure
additions; not one existing line was deleted or changed. `is_signature_id` is the per-id companion of
`is_signature_storage`, i.e. the predicate `clone/materialize.rs` and the two generic twins already
route through.

**Gated by** `crates/boyko_ecs/tests/retained_id_walk_pool_skip.rs` — 8 positive tests, watched RED
against the unfixed kernel in BOTH profiles first (identical NAMES, identical panic sites, exit 101),
green in both afterwards, and Miri-TB clean. Two of them observe the RESOLVER half directly, because
a filtered walk hides an unfiltered resolver by construction: reverting only
`without_ids_archetype_id`'s guard reds exactly that one test and nothing else — observed, not
assumed.

**What was deliberately NOT fixed, and why.** Each is a different premise with a VALUES call inside
it, so none belongs in a change that had to be the filter restoration and nothing else:

* **The required-ctor pass** (`migrate_entity_insert`, `for_each_required_id_excluding`). Its ids
  come from `B::component_ids()`, not from a retained list. `#[require(SomeDenseComponent)]` panics
  because the kernel has no route for CONSTRUCTING a required dense component — filtering there would
  turn a loud panic into a silently missing component. A missing feature, not a missing filter.
* **A bundle carrying a BITSET component** (`insert_command.rs`, `bundle_column_cache.rs`). Both
  guard with a `Dense`-only `matches!`, so a bitset id falls through to a pool lookup and panics in
  release; reachable from safe user code, because `#[derive(Bundle)]` accepts any `Component` field.
  Widening the guard is not enough on its own — a bitset id must not be routed to a `DenseStore`
  either, and what *inserting an enable tag through a bundle* should MEAN is your call, not mine.
* **The observer-flag walks** (`archetype_master.rs`: `create_archetype`'s OBS-SEED,
  `remove_observer`'s sibling recompute, and `debug_assert_observer_flags_consistent`). Three
  writers, two of which read the unfiltered list while `add_observer` uses the signature mask — so
  registering an observer for a dense component AFTER a retaining archetype exists trips the debug
  assertion. Reconciling them means deciding which writer is canonical. Nothing dispatches wrongly
  today: every fire loop already filters through `is_signature_id`.

---

Landing `components_of_into` needed a fixture entity carrying three components. The obvious route —
`spawn_two(table, dense)` then `add_tag` — panicked, and the panic is in the kernel, not in the glue.

**What it is.** `migrate_entity_attach_ids` copies the retained columns by walking the **source
archetype's `component_ids()`** and calling `component_pools().get_pool(cid)` for every id in it:

```
crates/boyko_ecs/src/ecs/core/commands/migration_helpers.rs:1831
    .expect("invariant: source hosts its own component id");
```

`component_ids()` is the raw id list the archetype was minted from, and it **retains** `Bitset` and
`Dense` ids by the kernel's own documented design — only the signature *mask* is filtered. A `Dense`
id has no per-archetype pool by construction, so the walk dies. `migrate_entity_detach_ids` has the
identical defect at `migration_helpers.rs:2116`. Both are `.expect`, not `debug_assert!` — this is
**release-present**, and `add_tag` / `remove_tag` are `pub` on `EcsMaster`.

**The victim class is wider than it looks, and this is the part worth your attention.** It is not
"an entity that carries a dense component". Archetype dedup keys on the FILTERED mask while the
retained list belongs to whichever call first minted that archetype, so a **table-only** entity that
dedups into an archetype first minted by a table+dense spawn panics on `add_tag` as well — it never
carried the dense component at all. Spawn order decides whether the program works.

**Measured**, in `D:/wt/reflect` on `feat/reflection`, in a throwaway probe with no reflection code
on the stack (probe deleted, tree left clean):

* `spawn_two(table, dense)` → `add_tag` → panic at `migration_helpers.rs:1831`.
* the same entity built with the tag in its signature → `remove_tag` → panic at `:2116`.
* `spawn_one(table)` → `add_tag` → fine, **when** no dense-carrying entity minted that archetype
  first.

In the production tree the one dense component is `GpuTransform3D`, so any entity that is
GPU-interpolated cannot be tagged, and neither can anything sharing its archetype.

**Why I did not fix it.** It changes the behaviour of a shipping API with existing callers, for
reasons that belong to a reflection campaign — the inversion the directional rule exists to prevent,
and the same disposition already recorded for `has_component`'s missing `Bitset` branch (F6) and
`set_component_raw`'s tick asymmetry (F14). EG1 routed around it: gate 4 builds its three-component
subject with `create_entity`, and gate 6 spawns its table-only sibling *before* the dense-carrying
subject so the shared archetype retains no dense id. Both sites say so, with this as the reason.

**What it blocks, and the call.** Not EG1 — the glue is read-only. But **EG2 and EG6 are built on
these two functions**: `S1` is specified as *"a bytes-carrying sibling of
`migrate_entity_attach_ids`"* and `S2` as the already-existing detach helper made `pub`. So
`add_default` / `remove` inherit the panic unless the retained-id walk is filtered through
`is_signature_storage` first.

* **(a)** Fix it in `boyko_ecs` **before EG2**, as its own change, on the kernel's schedule. The fix
  is one `if !is_signature_storage(storage_kind(cid)) { continue; }` in each of the two walks — the
  same predicate every other kernel consumer of `component_ids()` already routes through.
* **(b)** Leave it, and have EG2's new `add_component_by_id` / `remove_component_by_id` carry the
  filter themselves while `add_tag` / `remove_tag` stay broken. This ships a seam that is correct
  where its sibling is not, which is worse than either.
* **(c)** Fold the fix into EG2. It is the same code path, but it puts a shipping-behaviour change
  inside a reflection rung.

I did not choose. **(a)** is what the directional rule points at, and it is the reason this is here
rather than in a report.

## 2026-08-27 — SCOPE + VALUES: **the UI animation sink has no production reader to be ordered after, and the linter gate that would have said so was RED at HEAD**

**Status: OPEN — five items. Item 1 BLOCKS rung A4; items 2 and 3 are VALUES calls that block
nothing; items 4 and 5 are recorded, not asked.** Found at the [`UI-PLAN-ANIMATION-A1.md`](UI-PLAN-ANIMATION-A1.md#a1--the-sink-the-four-channels-the-fused-tick--size-l--no-cross-plan-dependency) A1 red-ledger
and adversarial passes (2026-08-27) and their remediation. Full detail and every measurement:
[`docs/UI-PLAN-ANIMATION-A1.md`, the A1 landing note](UI-PLAN-ANIMATION-A1.md#a1--the-sink-the-four-channels-the-fused-tick--size-l--no-cross-plan-dependency).

### 1 — SCOPE, and it BLOCKS A4: nothing in production registers `ui_render_discovery`

Rung A1 MEASURED that a system filtering on `Changed<UiVisual>` and ordered BEFORE `ui_visual_tick`
does not read the sink one frame late — **it never reads it**. Over 20 animating frames the reader
after the tick hit 20 times and the reader before hit **1**, and that one was an out-of-schedule
insert stamp. *(Of the 20, **19** are attributable to the sink: frame 1 is true through BOTH `Or`
arms, because the spawn stamps the pack-input component in that frame too. The gate asserts the 19
frame-by-frame and asserts frame 1 separately, crediting it to neither arm — corrected 2026-08-27,
it used to assert the total and silently credit the sink with a hit its control could not exclude.)*
The mechanism is that `Changed`'s window `(last_run, this_run]` is HALF-OPEN — `schedule.rs:288`
(`let this_run = world.bump_change_tick();`) and `:342`
(`sys_box.system.set_change_ticks(prev_this_run, this_run)`), with the comparison itself in
`change_detection/tick.rs:169-171` (`ticks_since_system > ticks_since_insert`), consumed at
`query/filter.rs:1205`, `:1225`, `:1493` and `:1503`. So a write stamped in frame N is above frame N's
reader and at-or-below the exclusive lower bound of frame N+1's — it falls in neither. **A golden
blessed under the wrong order pins a picture that never updates, not one that lags.**

> ⚠️ **This entry cited `schedule.rs:152` as the mechanism until 2026-08-27, and that is a doc
> comment about gated-system dispatch which merely contains the `(last_run, this_run]` notation.**
> The half-open semantics is real and is measured by the gate; only the pointer was to prose. The
> wrong pointer had been replicated into `animation.rs`, `sprite.rs` and both copies of this file —
> **a citation that reads plausibly is copied without being opened.**

The natural fix is `.after_set(UiAnimationSet)` on `ui_render_discovery`. It cannot be written today:

* **Verified 2026-08-27** — all **12** `add_system(ui_render_discovery)` sites in the tree are in
  `crates/boyko_render/tests/`. `src/` has **zero**. There is no production registration for an edge
  to be attached to.
* `boyko_ui` could not declare it in any case: `crates/boyko_render/Cargo.toml:94` depends on
  `boyko-ui`, and `boyko_ui` names no render crate. The edge, if it existed, would belong to whoever
  registers the system.

So A1 shipped the edge as a **contract stated at four sites plus a gate over the contract**
(`crates/boyko_render/tests/ui_a1_sink_reaches_discovery.rs`). **Rung A4 is where that becomes a real
defect** — A4 puts `UiVisual` into `ui_pack_inputs!`, which is what makes the discovery filter watch
the sink at all. A4 has been given a blocking precondition in the plan.

**The call.** Creating a render-side UI plugin that registers `ui_render_discovery` is a new design
decision, not a remediation: which schedule, which set, how it interacts with `AaPlugin` /
`Render3dPlugin`, and whether `.after_set` on a set no plugin registered is even legal on this
builder. **Options:** (a) build that plugin before A4; (b) land A4 with the contract still prose and
say so in A4's landing note; (c) something else the owner has in mind for how UI gets wired into a
real app schedule. A remediation rung must not invent (a) on its own, which is why it is here.

### 2 — VALUES: closing the last degenerate-duration route costs +3.8 % on every animating row

A1 found that `duration_ms` of `+inf`, `-inf`, `-0.0` or **any negative finite value** produced a row
that never completes — the reap can never reach it. The guard for it was only a `debug_assert!`,
which compiles out. Measured, release, 50 frames: `±inf` freeze the node silently; a
**negative** duration is worse than the code's own doc claimed — the sink diverged to `-49.999977`
from a `0.0 → 1.0` tween and bumped `set_if_neq` on **49 of 49** frames, which disarms the whole UI's
repaint skip rather than spoiling one node.

> ⚠️ **Corrected 2026-08-27: `NaN` is NOT one of the immortal shapes**, and this entry (with three
> source doc blocks) said it was. Under the shipped `advance`, `t` is NaN and the `t < 1.0` spelling
> puts a NaN `t` on the COMPLETING side — the row completes on frame 1, assigns its endpoint and is
> reaped. **Four of the five refused shapes are immortal; `NaN` is refused as an authoring mistake,
> not as an immortal row.** The two defences OVERLAP on that member, which is exactly what let the
> gate's NaN arm pass with the guard disabled. `-0.0` is the shape that was never enumerated at all
> and is genuinely immortal (`1000.0 / -0.0` is `-inf`).

That is now closed at the entry point: `start_tween_*` **refuses** such a duration in release (no row
is created), and `advance`'s completion test was inverted to put NaN on the completing side — the
latter MEASURED **free** (14.002 vs 14.008 ns/row).

> ⚠️ **And the refusal's first spelling was itself a regression, fixed 2026-08-27.** It read
> `is_finite() && duration_ms > 0.0`, and `0.0f32 > 0.0` is **false**, so a `+0.0` duration created
> no row and no sink at all — a node authored with a zero duration silently never animated, where
> before the guard it SNAPPED to its endpoint. Reachable from the safe `pub` API (a `duration *
> speed_multiplier` with a zero multiplier, a config value, a computed 0), and the denormals were
> unaffected, which is what hid it. The shipped predicate is `is_finite() && is_sign_positive()`:
> `+0.0` snaps, `-0.0` stays refused, and the sign bit is the only thing separating them.

**What is left open** is the hand-inserted-`TweenXBundle` route, where an author writes the row's
`pub` fields directly. Closing that in code needs a per-row `inv_duration > 0.0` term inside
`advance`, and it is NOT free: **14.544 ns/row against a 14.008 floor — +0.536 ns/row, +3.8 %**,
forever, on every animating row (4096 nodes × 4 channels, release, floor across five process
invocations). It was declined because the same struct's `from`, `to` and `elapsed` are equally
undefended on that route, so the tax buys one field out of four. **The call is whether the owner wants
belt-and-braces at that price**; the edit is one clause.

### 3 — VALUES, pre-existing: `UiTweenScratch.done` is a `std::Vec` inside a `Resource`, and the exception is nowhere written down

Principle 0 says durable subsystem data lives in the kernel's own storage. This buffer is
`Resource`-owned per-frame scratch on the established `UiBarScratch` shape, filled and drained inside
one frame — so it is arguably a legitimate exception. But it is **also** the exact shape the O11-SP4
colored-solve data race came from, and unlike the `unsafe` blocks and the `disallowed_types` allows,
this class of exception has no written rationale anywhere and no way to enumerate its instances.

Not A1's to fix, and A1 did not create it. **The call is whether "`Resource`-owned per-frame scratch"
is a named, greppable exception to principle 0** — the way `#[allow(clippy::disallowed_types)] +
rationale` is — or whether these should migrate to kernel storage.

### 4 — Recorded, not asked: the mandated clippy gate was RED at `e7a16fd9` and masked FOUR crates

`cargo clippy --workspace --all-targets --keep-going -- -D warnings` **exited 101** at commit
`e7a16fd9`. Cache-matched A/B (`cargo clean -p boyko-ui` before both runs, one edit apart), measured
twice by two independent passes with identical results:

| | red (at HEAD) | green (after the fix) |
|---|---|---|
| EXIT | **101** | 0 |
| crates reaching a `Checking` line | `boyko-input`, `boyko-threadpool`, `boyko-ui` | **+ `boyko-render`, `boyko-app`, `boyko_rhi_vulkan`, `aether-tests`** |

So **four** crates were never linted at all, and `boyko-ui` is not one of them — it is the crate that
errored. **Every clippy claim made on this branch between `e7a16fd9` and 2026-08-27 was vacuum-green
for those four.** With the fix applied the four are clean (0 warnings, 0 errors), so nothing was
hiding — but the reporting window was blind, and the owner should know its width. *(This is the same
shape as a red `const` assert masking a whole crate's compile, measured separately in the same pass.)*

### 5 — Recorded, not asked: three gates that do not cover what a reader would assume

* **The UI plans are not anchor-gated.** `tests/internal_docs_anchors.rs:349` (`GATED_DOCS`; fifteen at the A7 merge, still no UI plan) gated exactly four at `2a10f3a4`, the
  documents — `FEATURE_MAP.md`, `SYSTEMS.md`, `ARCHITECTURE.md`,
  `MESHLET-VIRTUAL-GEOMETRY-PLAN.md`. `UI-PLAN-ANIMATION.md`, `UI-PLAN-SPRITES.md`,
  `UI-PLAN-INTERACTION.md` and `UI-PLAN-AETHER.md` are **not** among them, so none of their
  `file:line` citations is mechanically checked. A sibling lane measured **96 of 143** UI-plan
  anchors already stale, with nothing gating them. Adding the UI plans to `GATED_DOCS` is a SCOPE
  call — it would red immediately and loudly, which is the point and also the cost.
* **`tests/ignore_reasons_census.rs` does not exist on `feat/ui-advanced`.** It lives on
  `feat/multi-paradigm-render` and `fix/inherited-red-gates`. It is NOT RUN on this branch by
  construction and must not be reported green in any landing note here.
* **The new `ui_pack_inputs!(assert_table)` const-assert cannot fire for the regression it reads as
  guarding** *(added 2026-08-27)*. It turns "a dense member of the pack-input list is invisible to
  `ui_render_discovery`" into a compile error, and for a **NEW** dense member it does exactly that
  (`error[E0080]` naming the member, reproduced). But for an **EXISTING** member flipped to dense —
  the realistic regression — the `E0277 … : Bundle` errors from that member's direct-insert sites in
  `boyko_ui` arrive first, `boyko-render` never compiles, and the const never evaluates. Measured on
  `StackIndex` (nothing else catches it) and on `UiVisual` (caught loudly, but by `UiVisual`'s OWN
  const-assert at `crates/boyko_ui/src/components.rs:1117`, a different guard in a different crate).
  It also guards **one** of the five production `Or<..>` lists in the workspace, not all five —
  `light_system.rs:989`, `layout.rs:88`, `layout.rs:118` and `text/measure.rs:61` carry no
  equivalent. Both limits are now written at the macro; **the point for the owner is that a guard
  reads as covering a class and covers one direction of it.**

### What it blocks

Item 1 blocks [**rung A4**](UI-PLAN-ANIMATION-A2-A8.md#a4--the-pack-fold--size-m--depends-on-a1-depends-on-the-sprites-plans-seam-rung-d31-gather--d6-gate) of `UI-PLAN-ANIMATION-A2-A8.md` (or forces A4 to land with the ordering contract
still unwired and to say so). Items 2–5 block nothing; they are calls and records.

---

## 2026-08-26 — A worker panic under the windowed host FREEZES the window instead of ending the process

MEASURED while wiring `PhysicsPlugin` into `examples/playground.rs`. The plugin's
`.colored_solve()` registered `physics_solve_colored` — a stage that reads
`ResMut<ColoredSoftStepSolver>` **by name**, not through the pipeline's generic `S` — while the
resource inserted was `SoftStepSolver`. The param resolve did exactly what it should:

```text
thread 'boyko-worker-1' panicked at crates/boyko_ecs/src/ecs/core/system/params/diagnostics.rs:22:5:
Resource `boyko_physics::solver::colored::ColoredSoftStepSolver` not registered.
```

**What the operator saw was none of that.** The window went "not responding" and stayed there: no
crash, no exit code, no CPU (6.5 s of CPU over 9 minutes of wall clock — the process was blocked,
not spinning), and the panic text scrolled past in stderr where a windowed run does not look. It was
reported to me as "boyko playground is not responding", and I had to bisect the frame loop with
`eprintln!` probes to find a *panic*.

**The plugin bug is fixed in this commit** — `insert_physics_resources` now inserts
`ColoredSoftStepSolver` whenever `colored_solve` is set, so the stage's parameter always resolves.
That closes THIS instance and none of the class.

**The class, and the question.** `Schedule::run` re-raises the first worker panic (the event-lane
campaign's fix, `schedule.rs:223`), and `CommandQueue::apply` calls `resume_unwind` — so a panic is
*supposed* to leave the frame loop. Under `boyko_app`'s windowed runner it did not: the process
stayed alive with a dead schedule and a live window. Whether the panic is swallowed on the way out
of the Fixed schedule specifically, or the re-raise lands somewhere the runner never observes, I did
not chase — it needs a deliberate look at the propagation path rather than an incidental one.

Two things that are worth deciding rather than assuming:

1. **Should a worker panic take the window down?** A frozen window is the worst of the three
   outcomes (crash / degraded frame / freeze): it hides the diagnosis the engine already produced,
   and an operator cannot tell it apart from a GPU hang or a deadlock. My inclination is that the
   host should catch the propagated panic and exit with the message on the console, but that is a
   VALUES call about how a shipped game should die.
2. **Should a registered system with an unresolvable resource fail at BUILD time?** Every parameter
   a schedule needs is known when `ScheduleBuilder::build` runs, and the world is right there. A
   build-time check would have turned this into a startup error naming the missing type, rather than
   a first-substep panic on a worker thread. The cost is that it forbids "insert the resource later,
   before the first run", which some legitimate boot orders may rely on.

Nothing is blocked on either — the scene runs. What is blocked is anyone else meeting this class and
spending the same hour on it.
---

## 2026-08-21 (second pass) — `boyko_reflect`: the four owner calls are FIVE, two of them were the same call, and the largest was on no list

A critique pass over the four plan documents (`REFLECTION-PLAN-CORE/ECS/BOUNDARY/GATES.md`) found
five blockers. Four were fixed inside the plans without needing you. The fifth is a scope call and it
is **new**, so the entry below supersedes the previous one **as a list** — the four items it names are
unchanged and still stand.

**The single list is now `docs/REFLECTION-ANALYSIS.md` **B.13**.** Five rows, each with what the plan
does *while it waits*, so nothing stalls and nothing is decided behind your back:

| # | Decision | Blocks | Plan's position while it waits |
|---|---|---|---|
| **1** | **NEW — may engine crates carry a `reflect` feature?** | rung 0 | proceeds on **yes** |
| **2** | **MERGED — the by-id `boyko_ecs` seam: FOUR items, ONE call** | ECS EG2 | **ANSWERED 2026-08-27: approved, all four** — see the entry at the tail |
| **3** | `BindAccessor`: one table or two | CORE C3/C8 | proceeds on Horn 2 |
| **4** | Aether components reflectable by default or opt-in | the Aether seam | proceeds on opt-in |
| **5** | Build order within (B): arrays → nested → enums → `String` last | what ships first | taken |

**Row 1 is new and it is the largest, because it decides whether v1 can inspect the engine's own
components at all.** The design's rule was *"the consumer writes the opt-in"* — but every dogfood
target the campaign names (`Transform`, `Name`, `Visibility`, `GpuTransform3D`, `EmitterActive`) is
defined in a **shared engine crate**, so there is no consumer to write it. The opt-in's
`#[cfg(feature = "reflect")]` is evaluated in the *defining* crate, which means `boyko_scene` and
`boyko_render` must declare a `reflect` feature — and `boyko_scene` has five workspace dependents, so
the gate rule as first written (*"only a leaf may declare it"*) reds mechanically the moment
`Transform` opts in. Nor can the declaration be dodged: the workspace's `unexpected_cfgs` lint turns a
`cfg(feature = "reflect")` in a crate with no such feature into a warning, and `-D warnings` makes it
red.

**Yes** ⇒ `boyko_scene` / `boyko_render` gain a **non-default** `reflect` feature plus an optional
`boyko-reflect` edge, contained by six mechanical clauses instead of the leaf rule (never in a
`default`; optional edges only; `dep:` spelling; **no dependency edge may enable it**; ship targets
declare and forward nothing; **every enabling command line is a named CI leg**). This is not novel:
`hwrt` is already a non-default feature forwarded across three shared shipping crates
(`boyko_rhi_vulkan` → `boyko_render` → `boyko_app`) and reaches no ship build, because nothing enables
it. The one thing wrong with `hwrt` is that **no CI leg compiles it at all** — which the sixth clause
exists to prevent here.

**No** ⇒ v1's dogfood becomes fixture-local *copies* of the engine's shapes, the phrase "real engine
types" leaves four gates, and the campaign loses the one claim that makes reflection worth building.
One commit either way; nothing else in the design moves.

**Row 2 was two questions.** The ECS plan routed a three-item structural seam to you and *rejected*
`EnableTagId::try_from_component_id` (landing a compile-fail fixture asserting it must not exist);
the BOUNDARY plan called that same constructor *"a required `boyko_ecs` seam"* and filed it as its own
question. You were about to receive one decision twice with opposite recommendations. It is one call
over four items — and the ECS plan's substitute turned out to be **wrong**, not merely inferior:
`register_enable_tag(name)` is idempotent only within the dynamic-tag name table, which *derived*
`storage = "bitset"` components never enter, so "toggle a bitset component off" would have minted a
brand-new tag, cleared **its** bit, left the real one set, and reported success.

**Fixed without you, recorded for visibility:** the mandatory Miri obligation was written as
`-p boyko-reflect --features reflect` in four documents — a command that **fails before compiling
anything**, because that crate has no such feature (the `cfg` is consumer-side); it is now two rows
with different shapes, and the row that reaches derive-generated `unsafe` is the fixture's. The ship
census's third leg — the one that decides whether the census means anything — was specified with a
fixture that made it a duplicate of its own control, so it would have recorded a **false conclusion
about the instrument, produced by the instrument**. And the first commit of the campaign was
specified by two documents that disagreed about its contents, with a completion condition that
required a rung standing behind itself.

---

## 2026-08-21 — `boyko_reflect`: the scope fork is TAKEN, and re-grounding it opened four owner calls

`docs/REFLECTION-ANALYSIS.md` was a 2026-06-15 snapshot of branch `ecs`. Re-grounding it against
today's tree (`feat/reflection`) confirmed its central finding untouched — "reflection only in a
Debug build" is not a compiler property; the mechanism is an optional crate behind a Cargo feature
plus a CI absence gate — but falsified several of its supporting claims and surfaced work the
document had no way to scope. **§6's scope fork is recorded as TAKEN: option (B)** (POD + `String` +
nested + `#[repr(Int)]` enum in v1; collections deferred to v2), with its reason, and marked
reversible by construction. What follows is what could **not** be decided without the owner.

**1. `BindAccessor`: one table or two? — the largest one, and it is about SHIPPING API surface.**

`#[derive(Bindable)]` / `BIND_ACCESSORS` shipped 2026-06-21 (`8a11f31b`), six days after the
snapshot, and is ~80 % of the field model the reflection design proposes to invent: a per-
`ComponentId` `[OnceLock<T>; MAX_COMPONENTS]` table of flat `fn` pointers, by-`u8`-index access, a
`field_id(name)` resolver, a documented cold-path discipline. It is read-only, flat, numeric-only,
requires named fields, and it **ships** (in `boyko_ecs`, read by `boyko_ui`, in the release binary).

* **Horn 1 — merge into `TypeInfo`.** One source of truth about a component's fields; `boyko_ui`
  gets kinds and nesting for free. **Cost:** a *shipping* crate then consumes reflection metadata,
  which strains the directional rule that keeps "dev-only" honest.
* **Horn 2 — two parallel tables.** Nothing that ships changes. **Cost:** two descriptions of the
  same fields, which drift silently on a rename; needs a feature-on consistency test to stay honest.

Neither is obviously right. Horn 2 is the lower-risk default. **Blocks Wave 1.** *(Nuance that makes
the call cheaper than it looks: there is no production `#[derive(Bindable)]` type yet — all five
registration sites are tests registering one fixture — so this is a decision about API direction,
not a migration.)*

**2. A public by-id structural seam on `EcsMaster` — a dev-only feature widening a shipping crate.**

`add_default` / `remove` were specified as "routes through the existing structural insert/remove, so
they inherit hooks/observers for free." A genuine by-id path now exists and is already driven by the
public `add_tag` / `remove_tag` — but all five helpers (`merged_archetype_id_dyn`,
`without_ids_archetype_id`, `migrate_entity_attach_ids`, `migrate_entity_detach_ids`,
`retag_in_place`) are **`pub(crate)`**, so an external `boyko_reflect` cannot call any of them. The
seam must therefore be made public, and it lands in `boyko_ecs` — **permanent shipping API surface
added for a dev-only feature.** It does not breach the directional rule (the signature is pure
`Entity` + `ComponentId`, as `add_tag` proves), and it is arguably useful on its own merits to scene
loading and the editor. But it should be a decision, not a Wave-3 discovery. **Blocks Wave 3.**

**3. Are Aether components reflectable by default, or opt-in?**

The design says "the *consumer* writes `#[cfg_attr(feature = "reflect", derive(Reflect))]`". For a
component declared in an `aether! { component Foo { … } }` block **the consumer never writes the
struct** — `aether_lang`'s expander emits it, and `ComponentDef` has no attribute-passthrough
grammar, so there is no syntactic slot. The macro must add it. The change is small and costs
`aether_lang` no new dependency (the emitted `#[component(reflect)]` is a token resolved downstream —
the tokens-not-deps rule its own manifest already states). The **question** is the default: opt-in
matches the rest of the design; default-on matches what a gameplay-authoring DSL usually wants.

Second-order and worth knowing: once Aether emits the opt-in, the reflection design's rule that
*"the consumer's reflect-enabling feature MUST be named `reflect`"* becomes a requirement Aether
silently imposes on **every crate containing an `aether!` block**. That belongs in the Aether
language docs, not in a reflection appendix. **Blocks Wave 2.**

**4. Build order within (B): arrays before `String`.**

(B)'s justification was that v1 could then dogfood `Name(String)`, `Transform`, and
`#[repr(u8)] enum State`. Two of those three are wrong about today's tree: `Name` carries a `u32`
(`Name(NameId)`, layout-pinned to 4 bytes; the string lives in a leak-backed setup-only interner),
and `State<S>` is a **generic Resource**, not an enum component. A whole-tree walk found **zero**
`#[derive(Component)]` structs with a `String`, `Box<str>`, or `&str` field — the engine's idiom for
component text is a fixed-capacity inline byte array (`UiName`, `UiTextBuffer`).

So the `String` half of v1 — the half this document spent its **headline CRITICAL** on (the raw
`drop_in_place` + `ptr::write` dance and its Miri-TB gate) — has no consumer, while
**fixed-size arrays `[T; N]` are pervasive and appear in neither the v1 taxonomy nor the v2
exclusion list.** They fall through to `Opaque`, which is a hard error, so the flagship dense
component (`GpuTransform3D { prev, curr }` over `TrsPacked { [f32;4] × 3 }`) is **un-derivable
today**. Arrays are strictly easier than `String` (offset + stride + count, all `const`, zero alloc,
no drop, no TB exposure).

**(B) stays taken.** The recommendation is only that Wave 2 build nested + enums + arrays first and
`String` last. This changes what the first inspector can show, so it is a scope call. *(The enum
half, by contrast, has eleven in-tree consumers — `Visibility`, `Interaction`, `FocusPolicy` are
themselves components, and eight more `#[repr(u8)]` enums are component fields — so it is
load-bearing, not speculative.)* **Blocks Wave 2.**

**Not asked, decided with evidence, recorded for visibility:** enumeration must take three sources
rather than the archetype signature alone (both `Bitset` and `Dense` are excluded from every
signature, so the design as written would refuse to *show* `GpuTransform3D` — the one component it is
fully able to *read*); the refusal matrix needs all four storage citizens plus GPU residency and
runtime-minted dynamic tags, not just "bitset + ZST"; and three CI-shaped items that would otherwise
have produced gates that cannot fail — the symbol-absence gate is inert without fat LTO (measured
in-tree, `--gc-sections` does nothing), CI's Miri is a hand-listed **allowlist** that would not cover
a new `boyko_reflect` package, and a bare root `cargo build` now selects every crate, so feature
unification needs no `--workspace` flag to reach the ship crate.

**One incidental `boyko_ecs` finding**, surfaced because reflection is the code that would hit it:
`EcsMaster::has_component(e, id)` **silently returns `false` for every bitset enable tag.** It
branches on `Dense` but has no `Bitset` branch, so a bitset id falls through to the archetype column
lookup, finds a null column (a bitset tag has no column by construction), and reports absent for a
tag the entity demonstrably has enabled. Wrong answer rather than a refusal. Not fixed here — it is
a kernel question, not a reflection one.


---

## 2026-08-26 — VALUES: **the UI hitch clamp is 100 ms, it was never measured, and it SHIPPED before the question was ever asked**

**Status: OPEN — a VALUES call. Blocks nothing, because the number is already load-bearing in
shipped, gated code.** Found at the [`UI-PLAN-ANIMATION-A0.md`](UI-PLAN-ANIMATION-A0.md#a0--the-ui-clock-and-the-one-consumer-that-already-exists--size-s--m--no-cross-plan-dependency) A0 pre-build audit.

### The situation

[`UI-PLAN-ANIMATION.md` §7](UI-PLAN-ANIMATION.md#7--open-questions-for-the-owner-values--scope--also-to-be-filed-in-docsopen-questionsmd) has carried this as its question 1 since 2026-08-21, and that section's
own heading promises its questions are *"also to be filed in `docs/OPEN-QUESTIONS.md`"*. **This one
never was.** In the meantime the sprites ladder's S5 rung landed the value:

```rust
pub const UI_FALLBACK_MAX_DELTA: f32 = 0.1;   // crates/boyko_ui/src/sprite.rs:320
```

It is `pub` inside `pub mod sprite` (`boyko_ui/src/lib.rs:64`), i.e. **public API**. It was applied INLINE inside
`ui_sprite_flipbook` until animation rung **A0b** (landed 2026-08-26) moved that system onto
`Res<UiClock>` and DELETED the inline `min` *(no line anchor for it: a coordinate into deleted
state resolves to whatever live line now occupies it)*; since then the clamp is taken once per
frame by `ui_clock_tick`
(`crates/boyko_ui/src/animation.rs`) and this const has exactly one reader, `UiClock::default()`.
One leg of a shipped gate asserts its effect
(`g5_2_the_clock_fallback_is_clamped_scaled_and_pause_aware` (a), `ui_s5_sprite_sheet.rs:534`: a
two-second alt-tab stall advances the flipbook ONE frame, not twenty). So a VALUES question was
answered by landing it — the shape this file exists to prevent.

### What the number decides

It is a **UI-local** clamp applied on top of the kernel's own `Time::max_delta` (250 ms,
`time/time.rs:23`), and it is what a user sees after a stall: below it, a transition that was running
when the game hitched resumes mid-flight; above it, the transition jumps to its end, which reads as a
glitch rather than as an animation. 100 ms is the plan's proposal *"because it is below the shortest
hitch a user perceives as a stall and above any frame time a shipping build targets"* — the plan says
outright that this is not measured, and no instrument in the tree measures perception.

### The options

1. **Accept 100 ms.** The value ships today; nothing changes.
2. **Name another number.** It is **one** line: [`UI-PLAN-ANIMATION-DECISIONS.md` AD9](UI-PLAN-ANIMATION-DECISIONS.md#ad9--which-field-a-consumer-reads-is-decided-by-whether-it-carries-d15s-flags-bit-and-the-clamp-has-exactly-one-definition) (3) makes
   `UiClock::default()` *reference* this const rather than restate `0.1`, so the flipbook and the
   tweens cannot come to disagree about what a hitch is.
3. **Make it per-app rather than a constant.** A0 **landed** a validated
   `UiClock::set_max_delta(f32)` (mirroring `Time::set_max_delta`, `time/time.rs:155-160`; panics on
   non-finite or non-positive input, gated by four unit legs in `boyko_ui::animation::tests`), so a
   host can already override it TODAY; the question is only what the DEFAULT is.
   *(2026-08-26 — **"already TODAY" was itself ungated until the A0 verification.** The setter
   worked, but `UiAnimationPlugin` inserts `UiClock` only if the world has none, and nothing tested
   that: MEASURED, replacing that guard with an unconditional `insert_resource` left the rung's
   whole gate at 7/7 while silently restoring the 0.1 default over any host value. A host that set
   its clamp BEFORE `add_plugin` — the exact shape this option describes — would have lost it.
   `a_host_configured_clock_survives_the_plugin` (`crates/boyko_ui/tests/ui_a0_clock.rs`) now runs
   that shape and reads the host's clamp back out of a truncated 2 s hitch, so the override is
   behavioural, not merely a field that retained a number.)*

### What it blocks

Nothing. Recorded because a VALUES call that was promised to the owner, never delivered, and then
settled by an implementation is worse than an open question — the owner cannot weigh in on a decision
he was never shown.

## 2026-08-26 — SCOPE: **the doc-anchor gate covers four documents; the five UI campaign plans are not among them, and 96 of their 143 anchors are STALE**

**Status: OPEN — a SCOPE call. Blocks nothing; it is what makes every other measurement in those
five documents unverifiable.** Found at the `UI-PLAN-SPRITES-DECISIONS.md` S6 pre-build audit ([S-D20](UI-PLAN-SPRITES-DECISIONS.md#s-d20--the-s6-pre-build-audit-the-cursor-hole-closes-with-a-hook-and-six-of-the-rungs-own-sentences-did-not-survive-the-tree) (11))
while trying to CERTIFY the amendments rather than trust them.

### The measurement

`tests/internal_docs_anchors.rs` is the only thing in the tree that checks a `file.rs:N` citation.
Its scope is a hand list of four: `GATED_DOCS` (`:231`) = `FEATURE_MAP.md`, `SYSTEMS.md`,
`ARCHITECTURE.md`, `MESHLET-VIRTUAL-GEOMETRY-PLAN.md`. **PROVEN vacuous for the UI corpus:** an
anchor deliberately repointed to `insert_command.rs:999999` inside `docs/UI-PLAN-SPRITES.md` left the <!-- doc-anchor-ignore -->
gate at `5 passed`, exit 0.

Widening `GATED_DOCS` to the five UI documents for one run (then restoring the test file
byte-identically, `cmp`):

| Document | anchors checked | STALE |
|---|---|---|
| the four already gated | **735** | **0** |
| `UI-PLAN-SPRITES.md` | 118 | **80** |
| `UI-ADVANCED-ARCHITECTURE.md` | 7 | **7** |
| `UI-PLAN-AETHER.md` | 6 | **4** |
| `UI-PLAN-ANIMATION.md` | 4 | **3** |
| `UI-PLAN-INTERACTION.md` | 8 | **2** |
| **the five UI documents** | **143** | **96 (67%)** |

⚠️ **SUPERSEDED 2026-08-28 — see the item above.** These numbers were measured on the tree of the
day and are kept as the record. Re-measured after this landing: **618 anchors, 541 flagged**, of
which **405 are misbindings** (the gate checked a file whose basename is not on the citing line),
132 are the definition-shape model against evidence-line citations, and 4 are misbound *and* out of
range. **Zero are rot**, and a sensitivity probe proved the widened gate blind to the rot this
campaign did find. The three options below rest on the 96 being real; they are not.

Three dead PATHS as well: `crates/boyko_ui/benches/ui_animation.rs` ([`UI-PLAN-ANIMATION-A2-A8.md` A8](UI-PLAN-ANIMATION-A2-A8.md#a8--the-measurement-rung-what-the-tick-actually-costs--size-s--depends-on-a1a5), pre-split `:2857`, read at its target 2026-08-28 by the tenth pass and re-read by the eleventh — *"**Lands.** `crates/boyko_ui/benches/ui_animation.rs` + its `[[bench]]` entry in `Cargo.toml` —"*; it read `:663`, then `:2282`, **read as a BLANK line on the part-10 tree** and carried through two re-aims of this sentence without being re-opened. ⚠️ **Part 11 re-read `:2282` and that description had itself rotted**: on the tree part 11 inherited it was *"defence out of the fixture and into a **source census** — and the seventh showed that a"*, not a blank line. A description of a stale value is an anchor too) and <!-- doc-anchor-ignore -->
`crates/boyko_render/shaders/ui_rect` twice ([`UI-PLAN-SPRITES-S5.md` S5](UI-PLAN-SPRITES-S5.md#s5--sprite-sheets-and-the-flipbook--size-m) and [`UI-PLAN-SPRITES-S6-S7.md` S6 · LANDED, "the landed set, file by file"](UI-PLAN-SPRITES-S6-S7.md#the-landed-set-file-by-file) — pre-split `UI-PLAN-SPRITES.md:3067`, `:3470`). <!-- doc-anchor-ignore -->

**0 of 735 against 96 of 143 is the gate, not the authors.** The gate's own module doc already
records the same shape from the other side: 75% of `FEATURE_MAP`/`SYSTEMS`/`ARCHITECTURE`'s anchors
were wrong before it existed, and *"a wrong anchor is worse than no anchor — it sends a reader, human
or agent, to a plausible-looking but unrelated line"*.

### The options

1. **Add the five to `GATED_DOCS` and repair the 96** — a repair rung with its own protocol and
   budget. The gate's `~` waiver and `<!-- doc-anchor-ignore -->` marker exist for the anchors that
   should not be shape-checked, so the repair has an escape hatch and does not have to be perfect.
2. **Add them and waive the backlog**, arming the gate for NEW anchors only. Cheaper, and it stops
   the bleeding at the cost of leaving 96 wrong pointers in place.
3. **Leave the five ungated** and stop writing `file.rs:N` in them — the anchors are the value, so
   this is really "accept that the plans' evidence is unverifiable".

**What it blocks:** nothing mechanically. It decides whether "MEASURED at `file.rs:NN`" in a plan
means anything a week later.

---

## 2026-08-26 — SCOPE: **D7, the `.ui` registration table, has no owning document** — three plans name three different owners and none of them builds it

**Status: OPEN — a SCOPE call. Blocks nothing today** because [`UI-PLAN-SPRITES-S6-S7.md`'s S6](UI-PLAN-SPRITES-S6-S7.md#s6--the-ui-authoring-landing-for-the-sprite-vocabulary--size-s) carries a
hand-written fallback, but the fallback is now the path rather than the contingency, and it is
roughly twice the size the plans say.

### The sweep

| Document | What it says about D7 |
|---|---|
| [`UI-PLAN-SPRITES.md` §0](UI-PLAN-SPRITES.md#0--what-this-plan-owns-and-what-it-does-not), [§6](UI-PLAN-SPRITES.md#6--what-this-plan-exposes-to-its-siblings) | owner is **`UI-PLAN-AETHER.md`**; §0 explicitly **Rejects** sprites owning it |
| `UI-PLAN-AETHER.md:73` | files D7 in its own **INBOUND** dependency table, *"soft"*, *"**D7 does not gate any rung here**"*; no rung U0–U8 lands a registration table |
| [`UI-PLAN-ANIMATION.md` §8](UI-PLAN-ANIMATION.md#8--dependencies-stated-explicitly) (pre-split `:3105`; read at its target 2026-08-28 by the tenth pass and re-read by the eleventh — the D7 row whose owner cell is verbatim the quote below; it read `:846`, then `:2530`, **read on the part-10 tree as the prose *"than deleted, because an implementer who builds the axis measures a flat line and reports it as"*** — plausible content carried through one re-aim of this row. ⚠️ **Part 11 re-read `:2530` and that description had itself rotted**: on the tree part 11 inherited it was *"*N+1* renders a strictly-moved value. The failure this catches is one frame of nothing at the head"*, with the prose part 10 quoted **exactly 200 lines below** — the same ripple this landing measured for the live pointer, arriving at the description in the same sentence) | owner is *"`docs/UI-PLAN-SPRITES.md` (rung 1) or wherever it is sequenced"* — the option SPRITES §0 rejected. ⚠️ That plan's own D7 row counts FOUR naming sites and one of them, its self-citation `UI-PLAN-ANIMATION.md:846`, no longer resolves; measured, there are three | <!-- doc-anchor-ignore -->
| `UI-PLAN-INTERACTION.md:512-521` | names no owner; *"This plan does not block on D7"* |
| `UI-ADVANCED-ARCHITECTURE.md:371`, `:1773` | D7 is §11 **sequencing item 1** — an architecture ladder no plan file claims |

`grep -rn UiVocab docs/` finds the derive named only in the architecture and the research corpus,
never in a rung. **Exactly one rung in the whole campaign is behind D7** — SPRITES' S6 — and
`grep -rn "\bS6\b"` across the siblings, the architecture and the book returns one passing citation,
so nobody outside `UI-PLAN-SPRITES.md` knows that rung exists either.

### What it costs to leave it

Two numbers moved when the landing count was traced site-by-site ([`UI-PLAN-SPRITES-DECISIONS.md` S-D20](UI-PLAN-SPRITES-DECISIONS.md#s-d20--the-s6-pre-build-audit-the-cursor-hole-closes-with-a-hook-and-six-of-the-rungs-own-sentences-did-not-survive-the-tree) (6)):
a hand-written component is **nine** landings, not five, so S6's fallback is ~30 rather than 15 and
D7's own justification figure is ~108 rather than 60. The argument for doing D7 gets STRONGER; the
schedule for doing it still does not exist.

Also: `UI-ADVANCED-ARCHITECTURE.md` §11 item 1 pins *"§10.9 must be green — all 19 existing
components — before **rung 4** adds the twentieth"*, and rung 4 is D1, which adds no vocabulary
member. The rung that adds the twentieth `.ui` name is S6, which §11 does not list. Landing S6 on the
fallback spends that pin and makes the literal "19" stale at 22 in three places.

### The options

1. **Give D7 to a document and schedule it.** `UI-PLAN-AETHER.md` is the natural home only if it
   wants it — today it explicitly does not, and its reason is sound (the construct emits Rust, never
   `.ui` text). A fifth plan file, or the architecture's own ladder, are the alternatives.
2. **Declare D7 out of scope for this campaign** and delete the dependency rows. S6 then lands
   hand-written by decision rather than by default, the "cost of not doing it" arithmetic becomes
   historical, and §11's pin is retired explicitly instead of being quietly spent.
3. **Leave it.** What happens today. The cost is that four documents keep pointing at each other and
   the one rung behind it lands on a fallback nobody chose.

**What it blocks:** nothing today. It decides whether ~30 landings are written once for sprites and
then again for animation and interaction.

---

## 2026-08-26 — SCOPE: **ten of the nineteen `.ui` components already round-trip and hot-reload SILENTLY WRONG**, and D7's pin would reproduce it

**Status: OPEN — a SCOPE call. Pre-existing; found at the [`UI-PLAN-SPRITES-DECISIONS.md` S6 pre-build audit](UI-PLAN-SPRITES-DECISIONS.md#s-d20--the-s6-pre-build-audit-the-cursor-hole-closes-with-a-hook-and-six-of-the-rungs-own-sentences-did-not-survive-the-tree)
(S-D20 (4)) and MEASURED, not inferred.**

### The measurement

A `.ui` source spelling `UiImage { texture: 7, uv_min: [0, 0], uv_max: [1, 1], tint: 4294967295 }`
parses and inserts — `UiImage present after parse = true` — and `serialize_ui` then emits the node's
`UiLayout` line **and nothing else**: `round-trip contains UiImage = false`. (Probe test, written,
run and deleted; the worktree was restored byte-identically.)

`parse_and_insert` has **19** component arms. `serialize_ui` writes **8** of them plus `UiName` from
the `#name` sigil, because `write_node` reads only `LiveNode`'s seven component fields
(`reload/tree_view.rs:49-56`); `ComputedRect` is deliberately excluded (Decision 14, documented in
place). `patch_node` reconciles the same 8. The remaining **ten** — `UiText`, `Button`, `Bar`,
`BarFill`, `UiImage`, `UiGrid`, `UiAnchor`, `OnClick`, `OnHover`, `OnSubmit` — have neither a
serializer nor a reconcile arm.

**Why no gate sees it.** The round-trip corpus asserts a FIXED POINT —
`assert_serialize_fixed_point` compares `s1` to `s2` (`p3_round_trip.rs:66-78`) — and a component the
serializer drops is dropped from both sides. A fixed point cannot see a missing `serialize.rs` arm.

### Why it matters now

This is precisely the failure `UI-ADVANCED-ARCHITECTURE.md`'s D7 cites as its own justification
(*"a silent failure mode — hot reload drops the component; the round trip loses it"*), and **D7c
pins "same round-trip bytes" for all 19**, which would reproduce the loss rather than remove it. It
also lands on sprites directly: a realistic sprite node carries `UiImage` (the sheet only substitutes
its slot and UV), so S6's three components would be landed MORE completely than the component they
modify, and S6's own round-trip gate has to use a `UiImage`-free fixture to be achievable at all.

### The options

1. **Land the missing halves for the ten** — ~9 landings each on today's hand-written path, ~90.
2. **Land only `UiImage`'s** (~9), because it is the one an S6 sprite node actually needs, and record
   the other nine as known.
3. **Declare the ten write-only by decision** — they are authorable but never serialized, and the
   round-trip contract covers the 8 + `UiName` only. Cheap and honest, and it makes D7c's pin
   expressible without reproducing a bug; it also means a `.ui` file is not a faithful save format.

**What it blocks:** nothing today; S6 works around it with a `UiImage`-free round-trip fixture. It
decides what `.ui` round-trip MEANS.

---

## 2026-08-26 — KERNEL DEFECT: `#[require(C)]` where `C` is a DENSE component PANICS at insert, and the panic names an expansion that never happened

**Status: OPEN — a real kernel bug, found while BUILDING UI-ADVANCED S5 and reproduced on every
insert. Not blocking S5**, which routes around it with a `Bundle`. Filed because the next subsystem
to pair a table component with a dense one will reach for exactly this attribute, and because the
panic message points away from the cause.

### The measurement

`UiSpriteAnim` (table) with `#[require(UiSpriteCursor)]` where `UiSpriteCursor` is
`#[component(storage = "dense")]`. Every spawn that inserts the animation panics:

```
crates\boyko_ecs\src\ecs\core\commands\migration_helpers.rs:728:22:
invariant: target hosts every required id (expanded archetype)
```

Three S5 gates failed this way before the attribute was removed; nothing about the message suggests
the storage kind.

### The cause, and why it is structural rather than a slip

The require pass resolves each required id's `ComponentPool` **in the target ARCHETYPE**
(`tgt!().component_pools_mut().get_pool_mut(req_id)`), because it materialises the required value
into that pool's next row. A dense id **has no per-archetype pool**: dense plan D0 makes it a
NON-SIGNATURE storage kind — "excluded from every archetype signature, owns NO per-archetype
`ComponentPool`; its global `DenseStore` is owned by the per-world `DenseRegistry`"
(`dense_d0_spawn_rejection.rs`'s own module doc). So the archetype expansion the `.expect` names
cannot have included the id, and the `.expect` is the first thing to notice.

Note that the SPAWN path already handles the mix correctly — dense plan D2 PARTITIONS a component
list, routing the table subset into the archetype and the dense subset to its `DenseStore`. The
require pass is the one structural path that did not learn the partition.

### The three options, and what each costs

1. **Route required dense ids to the `DenseStore`**, the way D2 already routes a spawn list. The
   honest fix, and it makes `#[require]` mean the same thing for both storage kinds.
2. **Reject it at COMPILE time** — the derive knows the target's `STORAGE_IS_DENSE` and could refuse
   with a message naming the storage kind. Cheaper than (1) and strictly better than today, but it
   leaves the capability missing rather than fixed.
3. **Leave it, and document it.** What S5 did, because the rung is not the place to change kernel
   structural ops: `AnimatedSpriteBundle` carries the animation, the sheet AND the cursor in one
   spawn, so the pairing is structural at the AUTHORING site instead of at the component. Gate
   G5-12 pins both halves — the bundle animates, and a hand-spawned animation without a cursor is
   frozen, silently.

**What it blocks:** nothing today. It costs every future dense/table pairing the same discovery,
and today that discovery is a panic message about archetypes on a line that never touches storage
kind. Option (2) alone would turn a runtime panic into a compile error for one afternoon's work;
whether (1) is worth doing is a SCOPE call.

**Addendum 2026-08-26 (S6 pre-build audit — [`UI-PLAN-SPRITES-DECISIONS.md` S-D20](UI-PLAN-SPRITES-DECISIONS.md#s-d20--the-s6-pre-build-audit-the-cursor-hole-closes-with-a-hook-and-six-of-the-rungs-own-sentences-did-not-survive-the-tree) (1)): there is a FOURTH
option, and it is buildable today.** A TABLE component's `#[component(on_add = …)]` hook can
deferred-insert the dense component through a one-field `#[derive(Bundle)]` wrapper. MEASURED in a
probe: after the apply, `has_component` reports the dense `UiSpriteCursor` present with its correct
`Default`. It works because `InsertCommand` **partitions** the bundle's ids and routes the dense
subset off the table path (`commands/insert_command.rs:128-137`) — the same partition (dense plan
D2) the require pass never learned. Note the trap: the BARE type does not work —
`insert(UiSpriteCursor::default())` is `error[E0277]: UiSpriteCursor: Bundle is not satisfied`,
because dense storage suppresses the single-component `Bundle` impl
(`boyko_macros/src/component.rs:413`). This does not close the defect — `#[require]` still panics,
and the hook is more code and DEFERRED rather than synchronous — but it means **no rung has to wait
for this entry to resolve**, and "the capability is missing" is now only true of the attribute.

---

## 2026-08-21 — KERNEL DEFECT: a dense `Changed<C>` / `Added<C>` inside `Or<..>` can NEVER be true, and it fails SILENTLY

**Status: OPEN — a real kernel bug, found at the UI-ADVANCED S5 pre-build audit and MEASURED. Not
blocking S5**, which routes around it ([`UI-PLAN-SPRITES-DECISIONS.md` **S-D16**](UI-PLAN-SPRITES-DECISIONS.md#s-d16--the-flipbook-writes-uispritesheetindex-and-it-must-a-dense-changedc-inside-or-is-measurably-dead): the flipbook's per-frame write
lands on a TABLE column and the dense cursor is never a discovery term). Filed because the next
subsystem to reach for it will not know, and because there is no diagnostic — the query compiles,
runs, and quietly matches nothing.

### The measurement

A three-frame schedule on a `boyko-ecs` world with one entity carrying a table `TSheet` and a dense
`DCursor` (rustc 1.97.1, this tree at `b2318ac5`; the probe was deleted after reading):

| frame | `Query<(), Changed<DCursor>>` | `Query<(), Or<(Changed<TSheet>, Changed<DCursor>)>>` |
|---|---|---|
| 1 — insert | 1 | 1 |
| 2 — idle | 0 | 0 |
| 3 — **dense write through `Mut`** | **1** | **0** |

Frame 1's `1` in the right-hand column comes from the TABLE arm. The dense arm is never true.

### The cause

`Changed<C>` and `Added<C>` support dense storage completely — `HAS_DENSE`, `HAS_DENSE_INCLUDE`,
`resolve_dense`, `dense_include_candidates` and a per-slot tick read
(`crates/boyko_ecs/src/ecs/core/iters/query/filter.rs:1000-1060`, `:1321-1390`). The `Or<(..)>`
`QueryFilter` impl **overrides none of them** (`filter.rs:1834-2030` sets `IS_ARCHETYPAL`,
`NEEDS_CHANGE_DETECTION`, `CONTAINS_ENABLE_TERM`, `CONTAINS_CHANGE_DETECTION` and nothing else), so
they all take the trait defaults: `HAS_DENSE = false`, `HAS_DENSE_INCLUDE = false`, `resolve_dense`
an empty body. The cursor therefore never resolves the inner term's `DenseStore`, its
`ChangedFetch.dense` stays the `init_fetch` NULL, and `filter_fetch`'s first line is
`if fetch.dense.is_null() { return false; }` (`filter.rs:1483-1484`). The tuple-as-AND impl should be
checked for the same omission.

Note that `matches_component_set` for a dense `Changed<C>` returns `true` unconditionally
("signature-excluded; the exact per-row gate is `filter_fetch`"), so the `Or`'s per-arm `matches`
flag is set and the dead arm IS evaluated — it just always answers `false`. Nothing anywhere reports
it.

### Why it matters beyond the UI

Two in-flight plans were writing against the property this refutes.
`UI-ADVANCED-ARCHITECTURE.md`'s tier table listed the **dense** `UiSpriteCursor` as "a term of
`ui_render_discovery`'s `Or<…>`", and `UI-PLAN-ANIMATION-DECISIONS.md` plans `UiVisual` as a dense include
(`AM2`, ~~`:112`~~ — the claim is the *cost model* paragraph, not the bench-axis line the anchor
pointed at) while `ui_render_discovery`'s filter is a flat `Or` — so `Changed<UiVisual>` would
have been dead too, and the symptom in both cases is a frozen picture with no error, no panic and no
failing assertion. ~~Both documents are amended.~~

⚠️ **"Both documents are amended" was FALSE for the `UiVisual` half, and stayed false for six days.**
Verified 2026-08-27 at the A1 pre-build audit: [`UI-PLAN-ANIMATION-DECISIONS.md`'s AM2](UI-PLAN-ANIMATION-DECISIONS.md#am2--d9bs-a-uivisual-row-with-no-live-channel-is-skipped-is-false-for-an-all-dense-anyof) still asserted
*"`Mut<UiVisual>` is a dense include"* and derived its whole per-frame cost model, A8's bystander
bench axis and AD7's `dense_registry().store(UiVisual::component_id())` guard from it; A1's landing
list named the storage kind of the four `Tween*` and **not** of the sink; and
`UI-ADVANCED-ARCHITECTURE.md`'s D9 still spelled `#[component(storage = "dense")] pub struct
UiVisual`. Only the `UiSpriteCursor` half had been applied — to the tier table's row 1, which names
`UiVisual` in the same cell. The shipped tree, meanwhile, had ruled and said so:
*"Animation adds `UiVisual` HERE (a table component — **the animation plan's own text is corrected to
say so**)"* (`crates/boyko_render/src/ui/gather.rs:121-122`) — a claim about a document, made in source,
and untrue at the moment it was written. **Both documents are amended NOW** (2026-08-27):
`UI-PLAN-ANIMATION-DECISIONS.md` [**AM8**](UI-PLAN-ANIMATION-DECISIONS.md#am8--d9-declares-uivisual-dense-and-d10-makes-it-a-term-of-ui_render_discoverys-or-those-two-cannot-both-be-true) and [**AD10**](UI-PLAN-ANIMATION-DECISIONS.md#ad10--uivisual-is-a-table-component-the-four-tween-are-dense) rule the sink TABLE and the four channels dense, with the
measurement; `UI-ADVANCED-ARCHITECTURE.md`'s D9 attribute and D9b's query are struck and corrected in
the same change. *(The lesson is the entry's own: a cross-document amendment is not landed when it is
decided, and "amended" written in a shared record is a claim that needs the same verification as any
other. This corpus is outside the anchors census's `GATED_DOCS`, so nothing reddened.)*

### The options, for the owner

1. **Fix the `Or` impl** — OR-fold `HAS_DENSE` / `HAS_DENSE_INCLUDE` over the members, forward
   `resolve_dense` and `dense_include_candidates` to each. It is the same paired-ident macro that
   already forwards `set_table_*`, so the shape exists; the gate is a runtime test per the existing
   `dense_d4_change_detection.rs` shape, since a type-level test cannot see it.
2. **Forbid it at compile time** — a sealed `OrComposable`-style bound that a dense `Changed`/`Added`
   does not satisfy, turning a silent no-op into `error[E0277]`. Cheaper, and it is the `M1`
   precedent this same impl already uses to keep `Enabled<T>` out of an `Or`.
3. **Document it and move on** — the S5 route: keep repaint-driving data in table columns. This is
   the status quo plus a written warning, and it leaves the trap armed for the next reader.

**Recommendation: (2) now, (1) when a subsystem actually needs it.** A silent always-false filter term
is the campaign's own headline defect class — a gate that cannot fire — and (2) removes the
possibility rather than handling it, which is the discipline `ui_node_sub_codes` was rewritten under.
This is a VALUES/SCOPE call because (1) touches the kernel's query core and its cost is a real design
review, not a patch.

---

## 2026-08-21 — SCOPE: `host_upload_frame` + `pack_sort_upload` are public API with no caller in the workspace — delete them, or wire the host that `APP-HOST-PLAN.md` already specifies?

**Status: OPEN — SCOPE, owner's call. Not blocking S4**, which routes around them
([`UI-PLAN-SPRITES-DECISIONS.md` **S-D13 (2)**](UI-PLAN-SPRITES-DECISIONS.md#2-s4-does-not-expand-the-legacy-loop--it-has-no-caller-in-this-workspace): the nine-slice expansion lands in `gather_into_staging` only).

### The measurement

Four greps, all re-runnable, all over `crates/` + `src/` + `tests/`:

* **`pack_sort_upload`'s only non-doc caller is `host_upload_frame`** (`upload.rs:526`).
* **`host_upload_frame`'s only non-doc occurrence in the entire tree is its own definition**
  (`upload.rs:509`). Every other hit is a doc comment, and two of those (`ui/upload.rs:39`,
  `boyko_ecs/src/ecs/core/system/dispatcher_token.rs:531`) describe the already-DELETED
  `host_upload_frame_from_world`. **The tests do not reach it either** — which is a change since the
  research corpus's finding 3 (`UI-ADVANCED-RESEARCH-SPRITES.md:210`), written when it was
  test-driven.
* **Neither name is re-exported** from `crates/boyko_render/src/lib.rs` or `src/ui/mod.rs`. They are
  public only as inherent methods on the re-exported `UiUploadSystem`, so they ARE public API — with
  zero in-workspace callers.
* The only mention of `UiUploadSystem` outside `boyko_render` is a doc comment
  (`dispatcher_token.rs:531`).

This is the surviving half of the path **S0 already replaced** with the two-phase seam. Its
world-facing sibling `host_upload_frame_from_world` was deleted at S0 for having no possible caller
(the entry below); this half was left standing.

### Why it is not simply deletable — the counter-evidence, stated so the call is made on both halves

**`docs/APP-HOST-PLAN.md` still prescribes it.** Op 5 of the per-frame sequence is
`if has_ui { pack_sort_upload(&token) }` (`APP-HOST-PLAN.md:316`), and R0b's token-discipline churn list names
`pack_sort_upload` among the borrow-taking writers to migrate (`APP-HOST-PLAN.md:136`), with the `has_ui` boot gate
at `APP-HOST-PLAN.md:230` and the token rule at `APP-HOST-PLAN.md:343`. That prescription is **unimplemented** — `boyko_app` does not
drive the UI pass at all — but it is a live plan, not dead prose. So the two functions are either
*dead code awaiting deletion* or *a landed API awaiting its caller*, and which one they are is a
scope decision about the host, not about the UI.

### The options

1. **Delete both.** Removes ~70 lines of unexercised public API and one of the two pack encodings,
   which is the source of the "two loops, two append encodings" hazard that has now cost this
   campaign three separate gate re-pointings (S3's reconciliations, S-D12's G4-1, S-D13 (2)).
   Requires striking `APP-HOST-PLAN.md` op 5 and re-specifying the host's UI upload on the two-phase
   seam.
2. **Keep and wire.** Implement `APP-HOST-PLAN.md` op 5 so the legacy loop has a caller and the
   golden/host path is real. Then S4's expansion would owe that loop a second implementation after
   all — but with a gate that can be run, which is what S-D13 (2) actually objected to.
3. **Keep, unwired, documented as such.** The status quo, with a doc note at the definition. Cheapest
   now; it is also how this pair reached its current state.

**What it blocks:** nothing today. What it costs if left: every UI rung from here on must re-derive
the reachability before it can decide whether "in both loops" is an instruction or a trap. S4 is the
third rung to pay that cost.

---

## 2026-08-21 — UI-ADVANCED S4 stopped BEFORE the first line: the nine slices are painted, then covered by the image they slice

**RESOLVED 2026-08-21 (architect's ruling, recorded as [`UI-PLAN-SPRITES-DECISIONS.md` **S-D12**](UI-PLAN-SPRITES-DECISIONS.md#s-d12--a-nine-sliced-nodes-slices-are-its-image-sub-10-is-suppressed-the-source-split-is-an-authored-border_uv-and-a-slice-with-no-texture-is-a-structural-skip); still no S4
code written). All four questions ruled; ~~S4 is buildable as amended.~~ **RE-OPENED THE SAME DAY and
resolved a second time by [`UI-PLAN-SPRITES-DECISIONS.md` **S-D13**](UI-PLAN-SPRITES-DECISIONS.md#s-d13--the-ruling-that-had-no-red-the-loop-that-has-no-caller-and-ten-sentences-that-could-not-be-written-as-spelled) — the implementer refused the S-D12-amended
rung as well, and was right a second time.** The findings below stand as
raised — every premise was re-verified at source before being ruled on, and two of them turned out
stronger than reported. The resolution is at the end of this item.

> **✅ THIRD BLOCK CLEARED — S4 IS BUILT AND LANDED, 2026-08-21.** The third implementer built
> the S-D13-amended rung. Ten further corrections were needed and are ruled as
> [`UI-PLAN-SPRITES-DECISIONS.md` **S-D14**](UI-PLAN-SPRITES-DECISIONS.md#s-d14--the-ten-corrections-landing-found-and-the-two-reds-that-could-not-fire-as-ruled); **two of them are this campaign's own headline class, and neither
> was visible to reading — both were found by applying the mutation and watching the wrong thing
> happen:**
> **(a)** **M4-c2's ruled sub pair fires for the wrong reason.** Sub 0 is pushed for EVERY node, so
> swapping the decode arms for sub 0 and sub 9 sends every node's background record — including
> nodes with no `UiNineSlice`, one of which G4-2's scene contains by construction — into an arm that
> resolves `nine_slice` and PANICS before any order assertion runs. The pair is **sub 1 (TL) and
> sub 9 (BR)**: total on the sliced node, count unchanged, and it reds G4-2 on the per-slice `min_px`
> the row already reads.
> **(b)** **M4-d's ruled bound is a red that cannot fire.** `assert!(scratch.pack.capacity() <
> 2 * emitted)` was applied against a setup-time reserve and came back GREEN — `sort_by_stack` ends
> in `core::mem::swap(&mut self.pack, gather)`, so the two buffers rotate every frame and the reserve
> was parked in the caller's `gather` (measured: `pack 4 096 / gather 22 528`). The bound belongs on
> the PAIR.
> The other eight: `fill_center` had no ruled `Default` and `bool::default()` falsifies the picture
> ruled one field earlier; `UI_NINE_SLICE_MODE_COUNT` was named by a ruling and minted by no Lands
> item; the gate table's preamble over-claimed (three of eight rows drive no pack loop, not one);
> Lands item 2's emitter shape is not writable as spelled; G4-3 stated no TINT and `UiImage`'s
> default tint is alpha 0, which disarms two reds; G4-3 could pass without comparing anything
> (`BOYKO_UI_GOLDEN_REQUIRE_DEVICE=1` now makes a skip fail); G4-8's "either build profile" had no
> release invocation; and the measurement paragraph's noun was off by the probe that is not a pack
> input.
> **All eight gates green with exit codes seen unpiped, all eight reds applied and OBSERVED, both
> mutated sources restored byte-identically (`cmp` + SHA-256), the five existing image pins
> unmoved, and the new pin blessed with all ten of its colours counted.** Full record:
> [`UI-PLAN-SPRITES-DECISIONS.md` **S-D14**](UI-PLAN-SPRITES-DECISIONS.md#s-d14--the-ten-corrections-landing-found-and-the-two-reds-that-could-not-fire-as-ruled) and the [**S4 · LANDED**](UI-PLAN-SPRITES-S4.md#s4--landed-2026-08-21--the-landed-set-the-red-ledger-the-golden-and-what-the-build-found) section.

> **⚠️ Second block, 2026-08-21, AFTER the resolution below was written.** S-D12 ruled these four
> questions correctly and introduced two blocking defects of its own, both confirmed by an
> adversarial pass:
> **(a)** its truth table made the image record and the BR sub-quad **mutually exclusive**, which
> rendered **M4-c unapplicable** — leaving **G4-2**, the row carrying S-D12 (1)'s own headline claim,
> with **no red at all** (G4-5 and G4-7 turned out to be named by none either). Split into M4-c1
> (emit sub 10 as well) + M4-c2 (swap the decode's arms for sub 0 and sub 9 — **not** the key push,
> which the sort normalizes away).
> **(b)** `pack_sort_upload`, the loop that both Lands item 2 and G4-1 required the expansion to land
> in, **has no caller anywhere in this workspace** — the surviving half of the path S0 replaced. S4
> now expands `gather_into_staging` only. Its deletion is a public-API SCOPE question and is filed as
> its own item above.
> Twelve amend-level findings came with them, including one this entry must own: **S-D12 changed
> G4-3's border to `[16,24,16,24]` in one row and left M4-b's margin — and two of its own
> sentences — computing from `[16,16,16,16]`.** The doc-rot-repair class, committed by the repair.
> Full record: [`UI-PLAN-SPRITES-DECISIONS.md` **S-D13**](UI-PLAN-SPRITES-DECISIONS.md#s-d13--the-ruling-that-had-no-red-the-loop-that-has-no-caller-and-ten-sentences-that-could-not-be-written-as-spelled) and ledger rows **20-33**.

~~**Status: BLOCKING. Two contradictions and one wrong number, in the rung as AMENDED by the
2026-08-21 pre-build audit. No S4 code was written; the protocol's stop condition ("a gate that
cannot fail, a red that cannot fire, a contradiction") is met three times over.**~~

⚠️ **Two coordinates in this entry are DEAD, and they were dead BEFORE the 2026-08-28 plan split —
so they are MARKED, not repaired.** `UI-PLAN-SPRITES.md:917` (cited twice below, in §1 and §2) and <!-- doc-anchor-ignore -->
`UI-PLAN-SPRITES.md:1019` (cited once, in §1) do not hold what the citing sentences attribute to <!-- doc-anchor-ignore -->
them, and re-reading the pre-split blob `2a10f3a4` — the last tree on which that file existed as one
document — shows they did not hold it there either:

* `:917` sits mid-sentence inside **S0's discovery-test note**; its readable remainder on that tree <!-- doc-anchor-ignore -->
  is *"with a message naming the three places to add it"* — `ui_pack_inputs!` and `PackInput::ALL`,
  nothing to do with `UiNineSlice`'s field list.
* `:1019` reads *"frames — the sheet bleed S-D7 designed a guard around, reproduced by the mechanism <!-- doc-anchor-ignore -->
  that retired"* — the S-D11 tiling argument, not the G4-4 self-gating quote attributed to it.

**Neither repair the neighbouring sites use is available here.** Stamping them "pre-split `:917`", <!-- doc-anchor-ignore -->
the form every re-aimed coordinate in this document carries, would assert the coordinate was good on
the pre-split tree — a NEW falsehood, written by the repair. Silently re-aiming them at whatever now
holds the quoted text is what this document's own ruling forbids: a dangling coordinate and a
re-aimed one are different facts, and only the first says the referent was never at that address.
They stand as written, marked dead, for the owner to re-source. ⚠️ **No gate can catch either**:
neither this document nor the UI plans are in the anchors census's `GATED_DOCS`, and that census
reads `file.rs:N` anchors, not `file.md:N`.

### 1 — the emission contract occludes itself (BLOCKING)

**S-D11 (3)** fixed the record count as ADD and stated the consequence "so no gate has to guess"
([`UI-PLAN-SPRITES-DECISIONS.md` S-D11](UI-PLAN-SPRITES-DECISIONS.md#s-d11--tiling-is-frac-inside-the-sub-rect-it-belongs-to-s5-and-the-nine-sub-quads-are-added-to-the-background-rect-not-substituted-for-it), pre-split `:389-392`): sub **0** background, subs **1..=9** the nine-slice regions,
sub **10** the image — *"A nine-sliced node emits **10** records …, **11** with an image."*

Four facts in the tree turn that arithmetic into a self-cancelling picture:

* **The only texture a sub-quad can sample is the node's own `UiImage`.** `UiNineSlice` as ruled
  (`UI-PLAN-SPRITES.md:917` — ⚠️ dead, and dead pre-split; see the note at the head of this entry) <!-- doc-anchor-ignore -->
  carries `border_px`, `mode`, `fill_center`, `_pad` — **no slot and no UV**. So a *visible*
  nine-slice REQUIRES the node to carry `UiImage`.
* **The image record is the WHOLE node rect at the WHOLE authored sub-rect.**
  `pack_ui_image_instance` covers "the SAME `ComputedRect` as the node's background"
  (`ui/pack.rs:289-292`) and packs `input.rect` verbatim with `uv: image.uv` (`ui/pack.rs:332-342`).
* **It paints LAST.** D4's order is *background → nine-slice sub-quads → image*
  (`UI-ADVANCED-ARCHITECTURE.md:250`), and both pack loops emit in ascending append order
  (`ui/upload.rs:428-436`, `ui/upload.rs:554-565`), so sub 10 lands after subs 1..=9.
* **Premultiplied-alpha blend means an opaque source REPLACES the destination**
  (`ui/resources.rs:439`, `BlendState::PREMULTIPLIED_ALPHA`).

**Therefore, on G4-3's own scene** (96×96 node, 3×3 opaque source, opaque tint, `border_px = 16`):
the nine slices cover 9 216 px and the sub-10 image covers the same 9 216 px on top of them. The
pinned image IS a plain stretched sprite, and:

* **G4-3 cannot fail as a pin** — it would bless the picture that proves nine-slice does NOT work;
* **M4-b cannot fire** (`UI-PLAN-SPRITES-S4.md:304`) — moving a corner from 16×16 to 32×32 moves only occluded geometry;
* **M4-e cannot fire** (`UI-PLAN-SPRITES-S4.md:397`) — permuting two sub-quads' source UVs moves only occluded geometry.

*(The `border_px = 16` and the 16×16 corner above are the scene **as it stood when this was raised**.
The resolution's own ruling (2) changed the border to `[16, 24, 16, 24]` in the same day's edit, which
makes a correct corner 16 × 24 = 384 px and M4-b's delta 2 560 of 9 216 — recorded here because
leaving the old arithmetic to be discovered downstream is exactly the doc-rot the second block below
had to correct in three other places. The finding is unaffected: an occluded delta of any size is
still zero.)*

Note the audit ledger's own row 3 caught the mirror-image of this for glyphs and the focus ring
("subject unconstructible") but did not ask whether the two terms it KEPT can coexist.

**The escape hatch is closed too.** G4-3 could avoid the occlusion by hand-packing nine
`UiInstance`s directly (the shape `ui_sprite_gpu_golden.rs:130-149` already uses), never
constructing an ECS node and so never emitting sub 10. But that is the *self-gating* defect this
same audit flagged one row later for G4-4 — "its `build_frame` calls `pack_ui_instance` directly, so
extending it would re-implement the expansion policy inside the test and gate the test against
itself" (`UI-PLAN-SPRITES.md:1019` — ⚠️ dead, and dead pre-split; see the note at the head of this <!-- doc-anchor-ignore -->
entry). So G4-3 either drives the production emitter and is occluded,
or hand-rolls the expansion and tests itself. **Both branches are defective; there is no third.**

**The decision needed (architect's, not the implementer's — S-D11 (3) is where the record count was
ruled).** Does `UiNineSlice` **suppress** the sub-10 image record — the slices ARE the image, sliced
— leaving a nine-sliced node at 10 records (9 without the centre) and `UI_RECORDS_PER_NODE = 11` as
the *stride*? That is the only reading found in which the rung draws what it exists to draw, it
leaves D4's ORDER intact (the image term is simply absent when slicing is on, the way a rect-only
node has no image term), and it keeps S-D8's default-OFF row byte-for-byte. But it contradicts
S-D11's stated arithmetic, and S-D11's ADD reasoning ("the background is the surface the frame sits
on") argues only about the BACKGROUND — it never contemplated the image record surviving beside the
slices.

### 2 — the SOURCE-side UV split is stated nowhere (BLOCKING)

Nine destination rects come from `border_px`. Nine **source** sub-rects come from nothing:

* `UiNineSlice` carries no source inset (`UI-PLAN-SPRITES.md:917` — ⚠️ dead, and dead pre-split; see <!-- doc-anchor-ignore -->
  the note at the head of this entry);
* `UiImageInput` is `{ slot, uv, tint }` — **no texture dimensions** (`ui/pack.rs:28-40`), and
  `border_px` is a *destination* quantity, so px → UV is not convertible without them;
* the dimensions cannot be fetched at S4 even in principle: the bindless table lives behind
  `RhiContext`, which is **Phase 2's `!Send` projection** (`ui/upload.rs:774`) and is structurally
  unreachable from the Phase-1 pack — and reading them in the shader is a shader change, which S4
  is defined not to make (S-D11 (2)).

Unity and Bevy both specify the border in **source texels** and both have the texture size; this
pack has neither. A grep across the plan, the architecture and the research corpus returns **no
statement of the rule** (`UiSheet.inset_uv` at `UI-PLAN-SPRITES-S5.md:19` is S5's half-texel bleed guard, a different
thing). Meanwhile **M4-e presupposes it exists** ("swap the TL and TR sub-quads' *source UV
sub-rects*"), and G4-3 pins a 3×3 source without saying why 3×3.

Only one rule is implementable at S4 from data the node carries: **split the node's `uv` rect into
equal thirds**. It is exact for G4-3's 3×3 source and it makes M4-b and M4-e fire. It is also
genuinely restrictive (a 32×32 chrome with an 8 px border wants 1/4, not 1/3). The degenerate
alternative — source fractions = destination fractions — is excluded by measurement-free reasoning:
it makes slicing a no-op and M4-b unable to fire. **This is a decision by elimination, and the
campaign's own rule is that an undetermined datum gets ruled, not guessed** — it is the same class
as the record count the audit itself escalated (ledger row 4).

### 3 — M4-f's threshold is a category error (recorded, non-blocking)

`:1074` — *"the box overflows at 187 nine-sliced imaged nodes"*. At stride 11 into a 4 096-row box,
**187 nodes emit 2 057 records and do not overflow**; the first overflowing node is **373**
(11 × 372 = 4 092 ≤ 4 096 < 4 103). `187 = ceil(2048 / 11)` — the NODE budget divided by the stride
instead of the ROW budget. The red still fires because G4-6 drives `UI_MAX_NODES = 2048` nodes
(22 528 records), so only the explanatory number is wrong; but it is a number asserted rather than
computed, in a rung whose own ledger opens with that class.

### What is NOT in dispute

Lands items 6 (`ui_pack_inputs!` gains `UiNineSlice`), 7 (the decode becomes a match, the key push
becomes a loop) and 8 (`UI_STAGING_ROWS = UI_MAX_NODES * UI_RECORDS_PER_NODE` = 22 528 rows =
1.72 MiB) are all verified correct against the tree and are buildable the moment (1) and (2) are
ruled. G4-6's scene fits the derived box exactly (2 048 × 11 = 22 528).

### RESOLUTION — 2026-08-21, [`UI-PLAN-SPRITES-DECISIONS.md` **S-D12**](UI-PLAN-SPRITES-DECISIONS.md#s-d12--a-nine-sliced-nodes-slices-are-its-image-sub-10-is-suppressed-the-source-split-is-an-authored-border_uv-and-a-slice-with-no-texture-is-a-structural-skip)

**(1) SUPPRESS.** `UiNineSlice` + `UiImage` ⇒ the image is drawn **sliced**: subs 1..=9 are the whole
of its rendering and **sub 10 is not emitted**. 10 records (9 without the centre);
`UI_RECORDS_PER_NODE = 11` stays, now explicitly as the **stride**, with a hole in the sub space that
costs nothing. *Reason:* nine-slicing is a rendering **mode of an image**, not a layer above one —
Unity's `type = Sliced`, Godot's `NinePatchRect` and Bevy's `NodeImageMode::Sliced` all slice the
image *instead of* drawing it, and all three keep the node's own background beneath, which is the half
S-D11 ruled correctly. A node wanting a sliced frame **and** an unsliced picture is **two nodes** — a
nine-sliced parent with an imaged child, exactly as in all three engines — which the DFS gather over
`Children` already supports with no new datum, and which is what "capability is component presence"
requires: `UiNineSlice`'s presence *is* "draw my image sliced".

**(2) An authored `border_uv: [f32; 4]`** — the source inset per side, as a **fraction of the current
UV sub-rect**, `[l, t, r, b]` (matching `PackInput::border_width`, not `corner_radius`). Component
**20 B → 36 B** (measured under rustc 1.97.1, both spellings; the two trailing bytes are implicit tail
padding again, so `_pad` stays), **zero GPU bytes** — the split resolves at pack into each sub-quad's
`uv`. Equal thirds is its `Default`, so option (a) survives as the zero-configuration case and G4-3
authors no new field, but is not the rule — a 32×32 chrome with an 8 px border wants 1/4, and a rule
right only for third-sized cells is one an author discovers wrong, not a gate. **Fractions of the
sub-rect rather than absolute UVs** is the load-bearing half: at S5 the sub-rect becomes a flipbook
frame that changes every tick, and a fraction is frame-invariant — the same "wrap and inset both
belong to the sub-rect" property S-D11 (1) found for `frac`. *One premise came back stronger than
raised:* the texel size is not merely unreachable behind the `!Send` projection — **the engine never
records it**. `BindlessTextureTable::register` takes a bare `VkImageView` (`boyko_render/src/bindless.rs:287`) and the
table holds no dimension map (`:217-221`), so Unity's and Godot's texel-border shape is unavailable
rather than deferred. Validity is ruled too (it was unstated): each side in `[0,1)`, `l + r < 1` and
`t + b < 1`, `debug_assert!` in dev and a proportional shrink in release, with the same shrink for the
destination twin `border_px[0] + border_px[2] > rect.w` — a 96×96 chrome tweened to 8×8 is an ordinary
animation and without it the corners overlap and the edges invert.

**(3) A nine-sliced node with no `UiImage` emits its background and nothing else** — `UiNineSlice`
alone is a **no-op**, not nine invisible quads. It is the structural-skip rule S3 already spells at
`pack.rs:205`. The rule that discharges Lands item 7 properly: **the key push is the sole authority on
which subs exist**, so every arm of the decode's `match` has its precondition established at the push
and **no `.expect` is reachable for any of the four component combinations**. New gate **G4-8** (all
four rows of the truth table in one world, derived count, no panic) and new red **M4-g** — because
item 7 fixed a release panic and no gate constructed the node that panics.

**(4) 410**, not 187 and not 373. `187 = ceil(2048/11)` was the NODE budget over the stride;
correcting only that gives 373; ruling (1) makes it 10 records/node, so **410**
(`10 × 409 = 4 090 ≤ 4 096 < 4 100`). Computed. The red is unaffected — G4-6 drives 20 480 records
into a 4 096-row box — and item 8's derivation stays on the **stride** (22 528 rows, 1.72 MiB): the
160 KiB of slack over the true worst case buys a constant that cannot go stale when a later rung adds
a sub code.

**Two further findings folded in.** **G4-1** was wrong on one loop and unobservable on the other —
`pack_sort_upload`'s `append` is the running record index (`ui/upload.rs:549-553`), not the `(node, sub)`
code, and `UiUploadSystem.keys` is private (`ui/upload.rs:206`) — so it now asserts the **consequence** in
`staged()`, which is strictly stronger: reading the key lane before the sort would go green on exactly
M4-a's duplication-and-loss. No accessor was added. **G4-3's `border_px = [16,16,16,16]`** repeated
the amendment's own symmetry blindness one axis over (`[l,t,r,b]` and `[t,l,b,r]` hash identically,
and no site stated the order at all) and becomes `[16, 24, 16, 24]`. **G4-7's instrument** was already
repaired in the tree at `50a724ac` and the row now names the `PackInput` enum so the property is not
re-lost.

---

## 2026-08-21 — UI-ADVANCED S0 stopped at a plan defect: `host_upload_frame_from_world` has no POSSIBLE caller, and S0's observer + two gates are specified against it

**RESOLVED 2026-08-21 (architect's WorldView ruling; landed the same day).** The finding, in
substance:

* **Option (a) (`EcsMaster::world_view`) is SOUND — and REFUSED as a dead datum at birth, not as
  unsound.** `WorldView` is ptr + `PhantomData` + a debug `ThreadId` — no tick, no epochs, no
  command queue — so a `&mut EcsMaster` mint discharges its invariants at least as strongly as the
  token's (`DispatcherToken::new`'s own Safety block, `dispatcher_token.rs:87-92`, blesses
  "`&mut EcsMaster` ⇒ `running == 0` at the language level"). But it does NOT unblock:
  `RhiContext` is a NonSend resource inside the SAME `EcsMaster`, so `world_view()` +
  `nonsend_resource_mut()` conflict on the master exactly as `world()` + `nonsend_resource_mut()`
  conflict on the token — the same E0502 one level up. The conflict is SEMANTIC, not syntactic:
  both operands of the fused signature are projections of one object.
  `run_closure_once` already serves host-time reads.
* **Option (c) (unsafe smuggling) confirmed rejected** against `dispatcher_token.rs:13-18` — the
  previously hand-asserted form of this property was UB on two axes (C1 worker reachability, M1
  aliasing).
* **The fix: sequence, never fuse, inside one `run_dispatcher`** (mirroring the shipped
  `GpuSystem` ordering). Phase 1 (shared borrow): the generation gate + `gather_into_staging`
  against the token's read-only view, the view dropped at the phase's closing brace, only the
  packed COUNT crossing. Phase 2 (exclusive borrow): `nonsend_resource_mut::<RhiContext>()` +
  `upload_staging` — no world type in the signature. `host_upload_frame_from_world` DELETED, not
  re-signed — its parameter list WAS the defect.

**Landed:** the two-phase `run_dispatcher` + `gather_into_staging`/`upload_staging` split + the
preallocated staging `Box` in `boyko_render/src/ui/upload.rs`; the fused fn deleted; the plan's
ten edit sites re-pointed ([`docs/UI-PLAN-SPRITES-S0-S2.md` S0](UI-PLAN-SPRITES-S0-S2.md#s0--the-seam-the-gate-the-observer--size-l)); the observer + G0-2 + G0-3 re-pointed at
Phase 1 (device-free, bare `EcsMaster` — `tests/ui_s0_seam.rs`, green); G0-5 as the SEAM GATE
(signature pins + the trybuild fixture `tests/ui_s0_seam_fusion/refused_refusion.rs`, E0502,
blessed and green); measurement legs §10.8(d)/§10.3 run headless (static dispatch median
0.2–0.4 µs, probes = 0 asserted; changed-frame full cost 231.7 µs @ 256 / 2250.3 µs @ 2048,
unreduced by the gate — reported honestly). S2 is unblocked as written.

**One claim of the ruling REFUTED by the compiler, recorded rather than landed.** The re-specified
M0-a — "hoist Phase 1's braces (delete the view drop) ⇒ E0502 AT COMPILE TIME" — is FALSE under
NLL: the view's borrow ends at its last use, so the hoisted-brace form COMPILES (probed in-tree
2026-08-21, `cargo check -p boyko-render --lib` exit 0, no diagnostic). The compile-time tripwire
exists on the shape that HOLDS the view across Phase 2 — M0-b (probed: E0502, exit 101) and the
G0-5 fixture (blessed E0502) both red as ruled. The brace is landed as scope hygiene with a
comment saying exactly this; the plan's M0-a row carries the refutation. The architect may want to
re-rule M0-a (e.g. as a `#[deny]`-able lint shape or drop it in favour of M0-b + G0-5, which
already cover the property).

*(Original entry follows, unedited — the record of why the call was made outlives the call.)*

**The situation.** [`UI-PLAN-SPRITES-S0-S2.md` rung S0](UI-PLAN-SPRITES-S0-S2.md#s0--the-seam-the-gate-the-observer--size-l) item 7 wires the observer through
`UiUploadSystem::host_upload_frame_from_world`, item 5 hoists the D6a per-slot generation gate to
the top of that same function, and gates G0-2/G0-3 (with reds M0-a/M0-b) drive it across frames.
The plan's own fact table records the seam has "zero callers outside its own doc comments" — as a
symptom. The cause turns out to be structural: **no caller can exist**. The signature demands a
`WorldView<'_>` and a `&mut RhiContext` alive at the same call site, and every route dies in the
compiler (all three probed in-tree on 2026-08-21, errors captured verbatim):

1. **In-schedule shape** (`RhiContext` as the NonSend resource it already is,
   `boyko_app/src/runner.rs:239`): inside `System::run_dispatcher`, `token.world()` borrows the
   token shared and `token.nonsend_resource_mut::<RhiContext>()` needs it mutable —
   **E0502** (`cannot borrow token as mutable because it is also borrowed as immutable`). This is
   M1 working as designed (`dispatcher_token.rs` — "a `WorldView` cannot coexist with
   `nonsend_resource_mut`").
2. **Host shape, owning adapter** (`RhiContext`/`Renderer` as host locals, minted into a
   `run_system_once` adapter): `System: Send + Sync + 'static` (`system/system.rs:57`) vs `RhiContext`'s
   `*mut c_void` / `OnceCell` / `RefCell` — **E0277** (not `Send`, not `Sync`).
3. **Host shape, borrowing adapter**: the same `'static` bound — **E0521/E0505** ("argument
   requires that … is borrowed for `'static`").

`WorldView` has private fields and exactly one constructor (`DispatcherToken::world`);
`DispatcherToken::new` is `pub(crate)` with two mint sites (scheduler dispatch, `run_system_once`)
— both put the caller inside a `Send + Sync + 'static` system. The set of shapes is exhaustive.

Note the endgame recorded in `upload.rs`'s own doc ("until an ECS-resident swapchain handle
exists") does not rescue the signature: shape 1 IS that endgame, and it is the E0502 case. The
`WorldView`-taking form is unsalvageable even after the Renderer becomes a resource — an
in-schedule body must gather, END the view borrow, then project the context, i.e. it can only ever
call the split form (`host_upload_frame` on the gathered nodes).

**What S0 landed anyway (defect-free half, gated, all green):** the `boyko-ui` Cargo promotion
(item 1); `ui_pack_inputs!` with the one-spelling list (item 2); `gather_ui_nodes` + host-owned
`UiGatherScratch` (item 3); `ui_render_discovery` (item 4); the per-slot
`last_seen_generation: [u64; FRAMES_IN_FLIGHT]` + hoisted compare as specified (item 5 — compiles,
but see below); both diagnostic counters (item 6). Gates G0-1 and G0-4 run green
(`boyko_render/tests/ui_s0_discovery.rs`); M0-c redded as specified (E0308 "expected a tuple with
3 elements" AT the gather); M0-d redded on G0-1's settle assert (its declared gate G0-2 cannot
run). **Blocked by the defect:** item 7 (observer), G0-2, G0-3, G0-5, M0-a, M0-b, and measurements
§10.8(a,d)/§10.3. Item 5's hoisted compare is therefore LANDED BUT UNGATED — a gate inside a
function nothing can call is exactly the "gate that could not fail" class this plan warns about,
which is why this entry exists instead of a quiet green report.

**The options (architecture fork — but it edits the KERNEL's capability surface and re-specifies a
plan rung, so it is recorded before anyone lands it):**

- **(a) One kernel API:** `EcsMaster::world_view(&mut self) -> WorldView<'_>` — sound by the same
  argument `run_system_once` already makes (`&mut self` ⇒ `running == 0` at the language level);
  the host then holds the view + its own locals, and item 7 lands as written. One function in
  `boyko_ecs`, no unsafe surface for callers.
- **(b) Re-specify the seam:** drop the `WorldView` parameter; the gather half takes `&EcsMaster`
  (whose `&self` read surface is exactly what `WorldView` forwards to), or the plan's item 7 is
  re-worded to the callable decomposition (gate → `run_system_once(gather)` → `host_upload_frame`).
- **(c) Not an option:** an adapter smuggling `*mut RhiContext` behind `unsafe impl Send/Sync` —
  production wiring routing around M1/M2 through the exact hole they exist to close.

Blocks: the rest of S0 (observer, G0-2/G0-3/G0-5, M0-a/M0-b, §10.3/§10.8), and therefore the S2
image-hash protocol's "the observer exists before the gate" sequencing argument (SR1).

## 2026-08-20 — Gate #17: two findings about the INSTRUMENT, one fixed in this commit and one still open

Measuring the particle passes (213 legs over four sessions) produced two facts about the measuring
apparatus itself. They are recorded here because both outlive the measurement: anyone who reads a
`ZONE_PARTICLE_DRAW` number, or who wires a harness around `particle_lab`, walks into them.

> ⚠️ **A THIRD fact about the instrument was found on 2026-09-18, and it is about every number in
> this section: the 21-frame window these legs report over is 20 warm-up frames plus one timed
> one.** `runner.rs` declared `VB_BENCH_WARMUP = 20` and budgeted the window as `warm-up + frames`,
> but fed **every** retired frame to `WindowReducer::observe_frame` — the discard the budget
> assumed was never written, and landed only with that date's repair. So `frames_checked = 21`
> below is the retired count, and each median is dominated by shader-compile and clock-ramp
> frames. **No figure here is adjusted or withdrawn; the correct repair is a re-measurement**, and
> the same invocation now produces `n = 1` (pass `BOYKO_VB_BENCH_FRAMES=21` for 21 timed frames).

**1. `ZONE_PARTICLE_DRAW` was readable only WITHIN one scene. — RESOLVED (architect's ruling,
2026-08-20); the fix lands in this same commit.**

MEASURED: putting an `SdfPrimitive` slab in the scene moves `ZONE_PARTICLE_DRAW` by **+74 752 ns** at
65 536 alive and by **+369 664 ns** at 102 400 — for work the particle draw does not do. The three
`BOTTOM_OF_PIPE`-stamped compute rows (kickoff/emit/sim) read **exactly 0 ns** for the same change at
all eight density cells, so this is not scene noise: it is the `TOP_OF_PIPE` drain absorption
`gpu_zone.rs` already warns about (*"the `TOP` rows … may each include a share of the drain ahead of
them"*), now measured on the particle family. The consequence while it stood: **any cross-scene DRAW
comparison was invalid**, and nothing said so at the point of reading.

The premise id 51 topped on was *"the whole lit producer runs before the particle draw, so its
isolation is unconditional"*. That is true — and it is a statement about **BRACKETING**, which the
measurement shows is not the same claim as isolation from **DRAIN**. **The id is restamped to
`BOTTOM_OF_PIPE`** (the DP6-0b precedent for ids 10/11, and cheaper here: unlike `ZONE_VB_SHADE`, no
published number is defined against id 51's `TOP` stamp, so there is no compatibility pin to break).
**The fix is measured, not assumed.** Re-taking the same null at 65 536 alive, 3 legs per arm, after
the restamp: base DRAW **93 184 ns** (was 106 496 — the absorbed drain leaving the bracket) and
**ctrl − base = +3 072 ns / +3.2 %**, against **+76 800 ns / +72 %** before. **96 % of the cross-scene
absorption is gone**; the resolvable 3-step residual is stated rather than rounded away. The three
compute rows are unmoved (SIM 73 728 on both arms), which is the control, and all six legs report
`measured = 315`, `lost = torn = not_bracketed = 0`, `frames_checked = 21`, `violations = 0`. The five
image goldens are byte-identical — a begin stage moves no pixel, and that is proven rather than
argued. Gate #17's own DRAW column is void as a baseline across the restamp by construction — the
same treatment DP6-0's four cells got, and for the same reason.

**2. Every gate-#17 run is a RED TEST BY CONSTRUCTION, and a harness cannot tell it from a real
failure. — STILL OPEN.**

An armed-zone `particle_lab` run writes its profiling artifact and *then* fails the process. The
cause is two **mutually-exclusive** exits from the same frame loop in `crates/boyko_app/src/runner.rs`:

* the **zone-budget exit** — `vb_zone_seen >= VB_BENCH_WARMUP(20) + vb_zone_frames`, which writes the
  artifact and `return`s from `App::run`. *(That `VB_BENCH_WARMUP` term budgets for a discard the
  reducer did not perform until 2026-09-18; since then the window is still `20 + frames` retired
  frames, of which only the last `frames` are folded.)*;
* the **capture-driver exit** — the conjunction over the five settle/drain drivers (host dump, census,
  HZB, VB probe, cull readback, particle readback), which fires at presented frame 30 (+3 drain).

With `BOYKO_VB_BENCH_FRAMES=1` the budget is 21 retired frames, so the first exit always fires first
and the frame-30 dump is never written — after which the fixture panics reading a file that does not
exist. Every number in gate #17 came out of a process that exited non-zero. **What is open: a harness
that keys on exit codes cannot distinguish "the measurement completed and the dump was skipped" from
"the run genuinely broke".** I am not fixing it here — the fix is a scope call (either the two exits
learn about each other, or the measurement legs stop asking for a dump they cannot reach), and both
touch a loop that five other capture drivers share.

**Also recorded, as an environmental shape rather than a defect: 1 windowing flake in 213
artifact-producing runs (0.5 %).** One leg reported `frames=400` (the `BOYKO_WINDOW_FRAMES` cap) with
no artifact: the window's client area was 0×0, so the runner's minimized path `continue`s before the
fence, the uploads and the render, and no zone frame ever retired. Re-run in isolation it gave
`frames=23` with a clean census. It is written down because **a silent zero-artifact run is
indistinguishable from a disarmed instrument to anything that only checks "did the file appear"**.

---

## 2026-08-20 — Particles P2 item 1: a dead fixture knob (repaired), and one pin I will not re-point without you

Landing the Deferred `-D DEPTH_LINEAR` arm turned up two things that are yours rather than mine.

**1. `BOYKO_PARTICLE_RATE` was a DEAD KNOB, and gate #17's density numbers depend on it.** MEASURED
while trying to drive a denser fan for the occlusion control: a `BOYKO_PARTICLE_RATE=8` run produced
a dump **byte-identical** to the rate-1 one (`sha256 60f39a3c…`). Cause: `particle_scene::setup`
seeds `ParticleEmitter::burst` from the env value, but `lab_arm_burst` — ordered BEFORE the fold,
including on frame 0 — overwrote it with a hardcoded `1` before anything consumed it. The knob's own
carrier advertised it as live in two places (the env table and `spawn_per_frame`'s doc, which names
gate #17 as its consumer), so the next person to run a density measurement would have measured a
1-per-frame scene and reported it as 8.

**Repaired here** (the re-arm reads `spawn_per_frame()`, the one fn both sites now read), because
the alternative — documenting it dead — leaves gate #17 undrivable. Verified in both directions:
rate 8 now renders **1737** saturated particle pixels against **265** at rate 1, and the default is
unchanged, so `particle_additive` and the other four image goldens re-proved byte-identical.

**What is yours: the P0 measurement rows this invalidates.** Gate #17 asks for kickoff/emit/sim/draw
µs at 10k/100k/1M. Any number produced through this knob before today came from a 1-per-frame scene
whatever the env said. I do not know which of P0's reported measurements, if any, were taken that
way — the ones I can see in the plan are stated as budgets, not as captures — but if a density
number was ever quoted from this fixture, it needs re-taking.

**2. `particle_additive` is still pinned on VisibilityBuffer, and re-pointing it is a VALUES call.**
The pin's comment said "re-point this pin at Deferred when that arm lands"; the arm has landed and I
did **not** re-point it. Doing so replaces a blessed digest with one nobody has looked at, and the
two paths shade the scene differently (only the particle pixels are identical between them — the
full BMPs are not). The comment now records the landing and leaves the decision open. **Say the
word and it is one `-Bless` run; the reason to want it is that the pin would then cover the engine's
DEFAULT path instead of the one it fell back to.**

---

## 2026-08-12 — L8a: two SCOPE calls I made narrowly, and one I did not make at all

Rung L8a migrated `boyko_render` / `boyko_image` / `boyko_serialize` / `boyko_physics` (16 sites,
twelve codes). Three things sat on the boundary between "decide it yourself with numbers" and "this
is the owner's". I decided two and am recording them for review; the third I am not deciding.

**1. `resolve_candidate` conflates an ABSENT optional texture with an UNREADABLE one, and I left it
conflated.** `crates/boyko_render/src/texture.rs`'s `load_slot` has two arms: a file that decodes
wrong is now `boyko-W2206` (a `Warn`), and a file that does not resolve is an `info!` with no code,
because all five material-folder slots are documented as optional and warning on absence would put
a `Warn` in the log of every material that ships four maps instead of five.

The problem is that the second arm also covers a file that *exists and cannot be read* — a
permissions fault, a locked file, a bad mount — because `resolve_candidate` discards the
`io::Error` with `.ok()`. So a real fault is reported at `info` level, indistinguishable from a map
somebody simply chose not to author.

**The cost of splitting it**, measured: `resolve_candidate` returns
`Option<(PathBuf, Vec<u8>)>` and would have to return the reason, which means re-blessing the three
tests that pin its `Option` signature (`resolve_candidate_prefers_the_first_existing_candidate_in_order`,
`..._falls_back_to_a_later_alias_when_the_first_is_absent`, `..._returns_none_when_no_candidate_resolves`).
That is a signature change to a helper in the asset load path, inside a rung whose scope is
"replace `eprintln!` with a coded emitter". I kept the rung's scope and recorded the hole rather
than widening quietly. **If you want it split, it is a small, self-contained change and I will do
it in its own commit.**

**2. `RatePolicy` is declared on every registry row and applied by nothing.** Measured by reading
the expansion, not the design: `warn!`/`error!` gate on the three ceilings and call `emit_impl`;
neither reaches `rate::admit`, which still has zero production callers. Every `Once` in this
registry works because a human placed an `OnceSite` at the emitter.

That is not itself a defect — `Once` and `Every` need no machinery. What it means is that a row
declaring `EveryN` or `MinIntervalMs` would be a **promise with nothing behind it**, and no check
would notice. I added `no_live_row_declares_a_policy_the_emission_path_cannot_honour` to `codes.rs`,
which reds on exactly that. It cannot prove a declared `Once` has an `OnceSite`; its failure text
says so.

**The question is what happens to `rate::admit`.** It has been carried for several rungs with no
caller. Either L11a/L14 wire it into the emission path — which puts a rate check on the enabled
path of every `Warn`/`Error` — or it is deleted and the registry column narrows to the three
policies the engine can actually honour. I have no measurement that favours either, and it is a
scope call.

**3. `E2203` floods, and I did not damp it.** `GpuSystem::run_unsafe` has no `Result` channel, so a
device fault that recurs reaches an operator only as a record per frame. I declared `Every`,
matching the `eprintln!` it replaced, on the reasoning that a `Once` would report the first bad
frame of a session and let an hour of broken frames look identical to a good one. The flood is
bounded by the ring, which drops and counts. If you would rather see one line per second than one
per frame, that needs item 2 resolved first — there is no mechanism today that could deliver it.


## 2026-08-11 — ⚠️ MEASURED: validation DOES run on this box, and the 2026-08-06 entry below is narrower than it reads

Opening logging rung L7 (migrate `boyko_rhi_vulkan`) started by re-deriving the site list, because
the rung's row cites line numbers that have drifted. It also had to establish what `E2101` — "add
an `error!` when validation is requested but the node was not chained" — can actually observe here.
Two measurements, both against HEAD, **before any L7 code was written**:

1. **A validation-ON boot works.** With `BOYKO_DISABLE_VALIDATION` **unset** and
   `enable_validation: true`, `cargo test -p boyko_rhi_vulkan --test compute` boots and passes
   **4 of 4**. The standing note that the SDK's MSVC-built `VkLayer_khronos_validation.dll` crashes
   this MinGW process on load is not true of the **headless compute path**. Whatever it describes —
   most likely the windowed/golden path — it is narrower than "validation cannot run here", and I
   have been treating it as the wider claim.
2. **The chained validation-features node is built, not unbuildable.** `create_instance` enables
   `VK_EXT_validation_features` when present and chains `VkValidationFeaturesEXT` with
   synchronization validation as the head of the instance `p_next`. Disposition **F2** ("a chained
   validation-features node is unbuildable here") is refuted by the tree.

**What that costs the plan.** G7's first clause — "`E2101` fires on a validation-**on** run" — holds
only if the node can never be chained. It can, so on a correct box a validation-on run must be
**silent**, and the gate as specified would be red against a working engine. I have re-cut `E2101`
to mean *validation was requested and this process is not getting it* (the escape hatch took it, or
the extension is absent), which makes G7 two-sided and runnable here: positive = escape hatch set,
negative = unset. The full argument is in the corpus (`logging/ladder`, the L7 block).

**This is an architecture call and I made it** rather than waiting — it is a gate's polarity, not a
value. It is here because it **contradicts a disposition the owner may have relied on**, and because
of what it does NOT change: `M25` stands. `compute.rs`'s own `negative_chained_barrier_hazard`
documents in the tree that sync-validation is enabled and still does **not** flag a compute→compute
RAW hazard on this path. The layer being *present* and the layer being *sensitive* are two
questions; L7 can gate the first and nothing gates the second.

> ⚠️ **A question I asked here and then measured, and it should not have been asked.** The first
> version of this entry offered the owner a choice: every golden runs under
> `BOYKO_DISABLE_VALIDATION=1`, so after L7 each one emits a `boyko-E2101` line — *"should it be
> suppressed for the golden legs?"* **Both halves of the premise are false**, and the question's
> shape was worse than either: it invited weakening a diagnostic to protect a channel, when
> **saying that a golden run's validation was disabled is the entire reason the code exists.**
> Suppressing it there would deliberately rebuild the defect the 2026-08-06 entry below describes.
>
> 1. **No collision is possible.** `scripts/golden.ps1:253` scans with the literal pattern
>    `\[vk-validation\]`. `boyko-E2101` cannot match it.
> 2. **In a golden run the line does not exist at all.** Measured: **no host calls
>    `boyko_log::lifecycle::boot` or `enable`** — the only callers anywhere are `boyko_log`'s own
>    tests and `boyko_ecs/tests/log_seam.rs`, and `crates/boyko_ecs/src/ecs/core/log/plugin.rs:40`
>    says so in its own doc comment. So the `error!` goes into a `.bss` lane ring nothing drains,
>    and not one byte is printed.

### And (2) is the finding that matters more than the question it answers

**The logger is in exactly the state the profiler was in at `e0160555`: complete, gated, and
unreachable from every host.** L5 landed the ECS seam, L6 landed the engine's emitters, and nothing
turns it on — so every record L6 just wired up is written into a ring with no consumer. Twelve
`Live` rows, five new codes, ten doc pages, and in a shipped run the whole apparatus is silent for a
reason no gate reports.

It is not a defect *of* L5 or L6 — `boot`/`enable` belong to `boyko_app`, which is **L8b's** row, and
`plugin.rs` was written knowing it. What is worth the owner's attention is that this is the **same
shape, in the same campaign, two rungs after it was found the first time**: every gate builds its own
world, enables logging itself, and asks whether the record arrived — so none of them can see that no
host ever does. I am recording it now rather than at L8b because the last time this shape appeared,
fifteen green rungs had passed over it.

**Nothing is blocked and no decision is needed**; L8b closes it by construction. If the owner wants
it closed *earlier* — a host that boots the logger before L7's migration lands, so L7's own sites are
observable in a real run rather than only in tests — that is a scope call and the only one here.

---

## 2026-08-11 — L6 found three mechanisms that exist and are unreachable, and left them that way on purpose

Logging rung L6 migrated `boyko_ecs` and `boyko_threadpool`. Three things it touched are **built,
correct, and consumed by nobody**. None of them blocks the rung; each is recorded here because
"reached for it and decided not to" is the only thing that distinguishes a deliberate gap from an
oversight, and because two of them are the same shape as the defect L6 opened with.

**1. `TargetControl`'s sync-route bit has no reader.** `target.rs` packs `bit [7] sync route —
format on the caller, write synchronously`, with a constructor, an accessor, a CAS that preserves
it and its own unit tests. `grep sync_route` over `crates/boyko_log/src` returns **`target.rs` and
nothing else**: `emit_impl` never consults it. A target with the bit set behaves exactly like one
without. Its intended writer is `apply_control_spec` (L14, the `net=debug/6!` form), so the bit is
early rather than wrong — but a *control* nobody reads is exactly what `site.decode` was, and that
one went three rungs unnoticed. **Not implemented at L6** because honouring it means a second
emission path (render on the caller, take `OUT_LOCK`) which is L14's row and needs L14's gates.

**2. `rate::admit` has zero production callers.** The rate limiter is complete and unit-tested —
`EveryN`, `MinIntervalMs`, the 512 cache-line slots, the suppressed counter. Every engine registry
row declares `Every` or `Once`, and both are answered by a **site-local latch** by design (F11), so
`RATE` is never touched. L6 considered `MinIntervalMs(1024)` for `W0701` — an event lane that
refuses every frame is exactly what a per-second cap is for — and **refused**: it drags a clock read
onto a cold ECS path and puts the rate decision *ahead* of the macro's own runtime gate, so a
disabled target would pay for a policy on a record it will not emit. The honest statement is that
`RATE`'s 32 KiB of `.bss` is reserved for a policy no engine row currently declares.

**3. `E0201`'s stderr fallback owes a `print_allowlist.txt` row at L8c.** `abort_on_task_panic`
prints for itself **iff** `flush()` answered `NoConsumer` — see the L6 decision block for why the
ledger's `error!` + `flush()` alone would have made the abort decision invisible in a
diagnostics-off process. L8c's `print_census.rs` bans `eprintln!` in production; this site needs a
row naming that reason. Flagged now because L8c is four rungs away and an unexplained allowlist
entry written then would read as laundering.

**What the owner may want to decide**: nothing is blocked. If (1) or (2) should be *deleted* rather
than left for L14/L11a — a bit and an array that cost `.bss` and reader attention — that is a
values call, and it is the opposite of the call this campaign has been making (absent rather than
stubbed). My own reading is that both are fine to keep, because both have a named consumer at a
named rung, which `site.decode` never did.

---

## 2026-08-11 — ⚠️ A profiling test's HAND-PICKED zone id is a bet against every schedule the rest of the crate runs. `ZONE = 7` still is one.

Found by rung 12's `G18`, which is the first profiling gate to assert an **exact session total** rather
than a single cell. In a full-workspace sweep it counted **20 013 of 20 000** samples: thirteen it
never pushed.

**The mechanism, and why the module lock cannot help.** `profiling/tests.rs` serialises every test
that arms, on one `test_serial()` lock — which is correct and insufficient. `ARM_MASK` is
**process-global**, so while any profiling test holds the profiler armed, **every other test in the
`boyko-ecs` binary that runs a schedule emits `SystemSpan` samples** (`profiling/zones.rs:193`) —
on its own thread, into its own lane, which the fold drains along with everything else. Those samples
carry per-system zone ids minted at `try_build` out of the same monotone `ENGINE_ID_NEXT` the static
zones use.

A hand-picked `const ZONE: u16 = 7` is therefore a bet that no system anywhere in the crate's test
suite lands on id 7 — and the bet is **re-rolled by every change to test execution order**, which is
why it had never fired before.

**Fixed for rung 12's own zone:** the tier tests now use a `declare_zone!`-minted handle. The counter
is monotone and shared, so an id that handle owns is one no `SystemMeta` can ever be given. Not a
mitigation — the collision becomes unrepresentable.

**NOT fixed, and this is what the owner may want to decide.** `const ZONE: u16 = 7` at
`profiling/tests.rs:41` is still a raw number, used by roughly thirty assertions across the rung-2
and rung-3 suites. They are far less sensitive — they read one `(frame, zone)` cell after a drain
they control, rather than a session total — so a stray sample would have to land in the same frame
*and* the same cell to be seen at all. But the hazard is the same one, and it is the kind that
surfaces as an inexplicable off-by-N years later.

Two ways to close it, neither urgent:

1. **Mint it too** (`declare_zone!` + a `fn zone()`), mechanical but touches ~30 assertions in tests
   that currently pass — a diff whose risk is entirely in the churn.
2. **Leave it and rely on the insensitivity**, with the hazard recorded here, which is the state
   today.

⚠️ **The general form is worth more than the instance:** *any* test that arms the process-global
profiler is measuring a channel the rest of the test binary is also writing to. A profiling gate that
asserts a total — as every rung-12-and-later gate does — must own an id nothing else can be given.

---

## 2026-08-11 — ⚠️ The `trybuild` corpus is blessed for a DIFFERENT rustc than the toolchain the project mandates. 23 fixtures were red at `3163078f`.

Found while sweeping rung 11, and **proved not to be rung 11's** with `git stash`: with the whole
working tree stashed, `cargo test --no-fail-fast` over the eight `compile_fail` suites at
`3163078f` reds **seven of them, 23 fixtures**, under
`RUSTUP_TOOLCHAIN=stable-x86_64-pc-windows-gnu` (rustc **1.97.1**, installed 2026-08-04).

**The diff has TWO causes, and my first diagnosis of it was WRONG — recorded that way because the
wrong one is the plausible one.** I wrote "compiler rendering drift between rustc 1.95 and 1.97.1"
and then tested it. Both claims below are probe results, not inference.

**Cause 1 — RUNG 11'S OWN, and it is not drift at all: `impl_self_bundle!` crossed a rustc
rendering THRESHOLD.** `compile_fail_relations/relationship_hook_collision.stderr` changed from a
span-based `help:` block into an inline `= help:` list of bare names. That looks exactly like a
compiler-version change and is not one. MEASURED with a five-line probe compiled by both binaries:
**rustc 1.95.0 and 1.97.1 render the identical trait-bound error identically**, and **rustc switches
from spans to the compact list at FIVE candidate impls**:

```text
3 impls -> help: … --> probe.rs:2:12  |  2 | struct T1; impl Marker for T1 {}      (spans)
4 impls -> spans
5 impls ->   = help: the following other types implement trait `Marker`:  T1 T2 T3  (list)
```

The `Component` list held **three** entries (`ChildOf`, `Children`, `LikedBy`) and rung 11 added
**three more** (`ProfilingScope`, `ProfilingScopeEnabled`, `ProfiledZone`) — six, past the threshold,
so the whole block re-rendered. `compile_fail_relations` was **green on the clean tree and red with
the change**: that suite is entirely mine. The same two `impl_self_bundle!` lines also appear inside
several other fixtures' `Bundle` lists without flipping their format.

⚠️ **The general lesson, which is worth more than this instance: adding ONE trait impl anywhere in
the engine can re-render every pinned diagnostic that lists that trait's implementors — and past the
fifth impl it changes the FORMAT, not just the contents.** A `.stderr` corpus is coupled to the
engine's impl *count*, invisibly.

**Cause 2 — INHERITED, 23 fixtures across 7 suites, and its origin is UNDETERMINED.** Proved not to
be mine by `git stash`, above. The signatures are additions and substitutions the blessed files do
not carry — e.g. `compile_fail_chunk/mut_data_rejected.stderr` gains an entire
`note: required by a bound in Query::<'w, 's, D, F>::for_each_chunk` block; elsewhere
`\| $crate::panicking::panic_fmt(…)` collapses to `= note: the failure occurred here`, and `AtomicIN`
becomes `Atomic<iN>`.

**I could not determine what produced them, and I am not guessing in this file.** The hypothesis I
had — that rung 10's *"543 targets ok, 0 failed"* was measured under the chocolatey `cargo`/`rustc`
1.95.0 that shadows `~/.cargo/bin` on `PATH` (real, found in this session, and the cause of a phantom
`E0133` on `__cpuid` plus a wall of MSVC `link.exe` failures) — is **not supported** by the probe:
the two compilers agree on the renderings I could test. The remaining candidates are a stale bless
predating an unrelated signature change in `query.rs`, or a rustc I no longer have. **What is
measured and certain: 23 fixtures were red at `3163078f` under the mandated toolchain, and rung 10's
green certification did not cover them.**

Both causes are re-blessed here in one pass, under 1.97.1, and are listed separately above so the
diff is reviewable rather than a wall.

**RESOLVED 2026-08-11 (owner: "реши сам"). Both decided; shipped as
`tests/trybuild_corpus_compiler_witness.rs`.**

**1. NO `rust-toolchain.toml`. A COMPILER WITNESS instead — and the reason is that a
`rust-toolchain.toml` would not have caught this.** The shadowing binary is a **standalone**
`cargo.exe`/`rustc.exe` from chocolatey, not a rustup proxy; a standalone cargo ignores
`rust-toolchain.toml` outright, so the file would have looked like protection while providing none.
Worse, the only form of it that would fix the *other* half — pinning the host triple, `channel =
"stable-x86_64-pc-windows-gnu"` — breaks every non-Windows build of a workspace whose stated targets
are *"Windows / Linux (x86_64)"*.

What ships reads the compiler's own version string and compares it to a `BLESSED_RUSTC` const
updated **in the same commit as any re-bless**. It catches a toolchain update *and* the shadow, on
any host, and it fails with both versions named plus what to do about it. **Its two REDs were run:**
naming a compiler that is not running prints `blessed: 1.98.0 … running: 1.97.1 …`; raising the
fixture floor prints `claims to speak for at least 9999 … and found 90`.

**The precedent decides the shape.** This repository already freezes a compiler for a byte-exact
corpus: every committed `.spv` is gated against a **frozen `dxc` recipe in the shader's own header**,
so a compiler change cannot silently redefine the artifact. A `.stderr` corpus is that shape with a
different compiler and had no freeze. Now it does.

**MEASURED while writing it, and it corrected the entry above:** the corpus is **90 `.stderr`
files**, not the 24 this section first said. 24 was the number of files rung 11's *diff* touched. **A
count taken from a diff is a count of what changed, not of what exists**, and the two are equal only
by accident.

**2. `trybuild` STAYS.** A compile-fail fixture proves a property no runtime test can reach — that
the type system *rejects* a shape — and this rung leaned on exactly that for `G12` clause 3 and rung
10 for `G22b`. The price is real and is now **visible instead of silent**: when rustc changes, one
named gate fires and says so, rather than 23 fixtures mismatching under a green-looking sweep.

⚠️ **One coupling the witness does NOT remove, recorded because it is the surprising one.** A
`.stderr` corpus is coupled to the engine's **impl count**, not only to the compiler: past five
implementors rustc switches the *"other types implement trait …"* block from spans to an inline
list. Adding one trait impl anywhere can therefore re-render a pinned diagnostic in a crate that has
nothing to do with it. No gate can prevent that; the witness at least stops it being confused with
compiler drift, which is exactly the confusion it caused here.

---

## 2026-08-10 — RESOLVED (owner: "реши сам"): both rung-10 gate questions, decided and closed

The owner delegated these two rather than deciding them. Both are decided below, with the reason
each way was taken, and both entries in `05-LADDER-GATES.md` are updated to match.

### `G17` keeps the A/B ratio. No release-profile absolute-nanosecond gate.

**Decision: keep what ships.** The question was whether to build a release bench harness and pin a
per-box nanosecond floor so the corpus's literal thresholds (`static-armed ≤ 12 ns`,
`dyn-armed ≤ 14 ns`, …) could run.

An absolute-ns threshold is a claim about the **machine**, not about the code. The property the row
is actually protecting is *"the handle carries its arm bit, so the emission path never dereferences
`REGISTRY`"* — a structural property of the implementation, and one that holds or fails identically
on a 2 GHz laptop and a 5 GHz desktop. The A/B measures exactly that: both variants, interleaved,
one thread, one sitting, and a machine that is slow today is slow for both legs. A 2 ns budget over
12 would additionally have to be re-blessed on every box the repository is ever built on, and would
red-light on a busy CI runner for a reason having nothing to do with this code — which is the
failure mode a gate exists to *avoid*, not to demonstrate.

What is kept from the corpus's intent: the absolute figures **are printed**, with the build profile
named beside them, so a human can compare them to the row's numbers whenever they want to. What is
refused is *asserting* on them.

The one caveat that survives is stated at the file: the ratio was measured in `debug` (2.02×).
`cargo test --release` runs the same test and prints the release pair; nothing in the assertion
depends on which profile it ran under, so there is no separate release leg to build.

### `G22b` clause 2 is REWRITTEN against the real symbol, not deleted

**Decision: rewrite, narrowed to the property that can actually fail.**

Deleting it was the tempting option — the clause as written describes a failure the type system
makes unwriteable (`SyncCells<T, N>` takes its extent as a const generic, so no run-time
`ProfilerConfig` value can size one), so the claim is vacuously true. But deleting leaves the corpus
with **no clause naming `assert_zero_init_eligible` at all**, and rung 10 measured that this is
precisely the property that breaks: `ZoneDesc` carries a `&'static str`, cannot be `ZeroInit`, and
`DYN_DESCS` needed a `MaybeUninit` wrapper that reads like ceremony and is easy to delete.

So the clause now reads: *a `.bss` arena declared over a type whose all-zero bit pattern is not a
valid value must fail at compile time; delete `DYN_DESCS`'s `MaybeUninit` wrapper ⇒ `E0277` ⇒ red* —
which is the `trybuild` case already shipped at rung 10 and already blessed. The old sentence's
claim is kept as a **recorded impossibility** with the const-generic argument beside it, so a future
revision does not re-add a gate that cannot fail.

`G22b` clause 1 is untouched and remains BLOCKED on `rustup component add llvm-tools`.

---

## 2026-08-10 — RESOLVED: retire the whole S1.5 harness, phase driver included. Rung 7's mechanical gate is CLOSED.

Owner's answer to both scope questions was the same: retire. Shipped. `rg
'TimestampCollector|VbTimedPass|Sv0TimedPass' crates/` now returns **zero code matches** — the rung's
gate, open since the campaign began, is closed.

Two things worth knowing about what the deletion cost, neither of them a decision to make:

**The S1.5 A/B is gone as an experiment, not only as a printer.** `sv0_bench_lighting_flags` drove
`SHADOWS|AO` off on two frames in four; every frame now pushes what every non-bench frame always
pushed. The transcribed numbers stay in `sv0_deferred_term_bench.rs` with its device-free arithmetic
gates, and this repository's own rule applies to them: a result established on a retired instrument
bounds nothing about the current one. Any rung needing a CURRENT Deferred-marcher figure takes a new
measurement on the zone artifact.

**Rung 7 ends with no A/B gate on any GPU family.** `gbuffer_zone_port_gate.rs` went with its leg A,
as `vb_zone_ab_witness_gate.rs` did. That is correct — there is nothing left to compare — and it
means the stage tables under `zone_begin_stage` are pinned by `const` blocks rather than measured
against an independent copy. Named at the function, not left to be discovered.

---

## 2026-08-10 — Rung 7 step 6c attempted and REVERTED: the SV0 bench is not a printer, it DRIVES the A/B it reports.

You answered the previous item with "retire the harness", and that part is settled — the
`window_present_gbuffer.rs` timing leg goes. The deletion still did not land, and the obstacle is a
new one that only surfaced by attempting it. **The tree is back at the last green commit; nothing is
half-finished in it.**

Rung 7 step 2 deleted the VB *printed measurement channel* as pure output. The corpus carries that
same framing forward to the SV0 half, and it is wrong there. `runner.rs`'s S1.5 harness computes
`sv0_bench_lighting_flags` from an ABBA phase counter and threads it into the scene every frame — it
does not merely REPORT the interleaved A/B, it DRIVES it, by changing what the marcher shades.
Deleting the timing channel therefore deletes a **render-path input**, which is a different act from
deleting a printer.

So there is a second question, and it is yours for the same reason the first was:

* **Retire the whole S1.5 harness** — the phase driver with the printer. Its transcribed numbers stay
  in the plan; nothing in the tree reproduces them afterwards.
* **Keep the phase driver, delete only the timing** — the A/B still runs and still changes the
  frame, but nothing measures it. That is a scene input with no consumer, which this campaign has a
  name for: a value nothing can make move.

My recommendation is the first. The second leaves a mechanism whose only purpose was to be measured.

**This blocks the last two collectors and the rung's mechanical gate.** Everything upstream of it is
shipped and green.

---

## 2026-08-10 — Rung 7 step 6 is blocked on a SCOPE call: does `engine_grand_showcase_512_gpu_pass_cost` get ported, or retired?

The last two GPU collectors (`TimestampCollector`, `Sv0TimestampCollector`) cannot be deleted while
something reads their durations, and one thing does: the `#[ignore]`d offline printer
`engine_grand_showcase_512_gpu_pass_cost` in `crates/boyko_rhi_vulkan/tests/window_present_gbuffer.rs`.
Its sibling, `software_ray_baseline_cost.rs`, migrated in ten minutes — it turned out to be using the
collector as a plain array of query pools. This one genuinely reads per-pass timings.

**Why it is not simply a port.** `run_showcase_body_ddgi` builds ONE `GBufferScene` literal (~230
lines) and holds it across the whole timing loop. The zone leg needs `open_frame` (`&mut`) → a
shared borrow parked in `scene.gpu_zone` → present → `retire` (`&mut`), every frame. The shared
borrow's lifetime is in `GBufferScene<'a>`'s type, so setting the field to `None` between frames does
not release it and the `&mut`/`&` cannot alternate. `boyko_app`'s runner never hits this because it
rebuilds the scene every frame; this fixture would have to move a 230-line literal into the loop.

**Three ways out.**

1. **Rebuild the literal per frame.** Mechanical, contained, and makes a 230-line construction run
   200+ times where it now runs once. Nothing measures that construction, so the cost is unknown
   rather than negligible.
2. **Give `GpuZoneRecorder` interior mutability** so `open_frame`/`retire` take `&self`. `FrameSlot`
   already holds two atomics and an `UnsafeCell`, so this is the same change rung 5c made to
   `CommandWitness`, one level down. It is the most reusable answer and the one that widens the
   kernel's surface: anyone holding a `&GpuZoneRecorder` could then claim a ring slot.
3. **Retire the harness.** It is an `#[ignore]`d printer; the zone artifact carries the same four
   brackets; its numbers are already transcribed into the HW-RT plan. This is the option the corpus's
   own precedent points at — `vb_bench_totality_gate.rs` and `vb_zone_ab_witness_gate.rs` were both
   deleted rather than migrated once their subject moved.

**This is a SCOPE call, so it is yours.** My recommendation is **3**, and the reason is that 2 buys a
kernel capability for one caller that a per-frame scene rebuild already gives that caller for free —
but the harness is a published measurement channel for HW-RT R0, and deleting a channel is not mine
to decide. **Until it is answered, rung 7 step 6 stops** and the mechanical gate stays unsatisfiable.

---

## 2026-08-10 — Rung 7's mechanical gate would be satisfied by deleting the record of what it gated. Corrected in the corpus; disclosed here because it is a SPEC change.

The corpus gates rung 7 on `rg 'TimestampCollector|VbTimedPass|Sv0TimedPass' crates/` returning zero
matches. After the VB family's half of the deletion the tree has **zero code references and roughly
a dozen prose ones** — `gpu_zone.rs` explaining what its ten `ZONE_VB_*` constants are what remains
of, `command_witness.rs` reconstructing the rung-7c stage defect from the collector that carried it,
`vg_occ_split_timing.rs` naming the channel its table used to read.

`rg` cannot tell a surviving CONSUMER from a comment that records what was deleted and why.
Satisfying the gate literally means erasing exactly the measured history the campaign exists to
keep. **I scoped it to CODE** and wrote the reasoning at the gate — the same scoping the second
mechanical gate already has (`crates/*/src`). Recorded here rather than decided silently because it
narrows a specified gate, and a narrowed gate is the owner's to widen back.

## 2026-08-10 — Deleting the old VB collector deleted the only gate on the stage table. Stated, not repaired.

`zone_begin_stage` says which pipeline stage each VB zone opens at. It had a real gate while
`VbTimedPass::begin_stage` existed as an independently-written second copy: `G10`'s stage clause
compared the two stamp for stamp — 26 frames, 520 timestamps, all identical. That clause is what
caught rung 7c's silently-changed stages after five green commits.

Rung 7 step 5 deletes leg A, so the comparison has no second side. What replaces it is a `const`
block pinning each of the ten ids to the stage both tables agreed on. **It catches a row edited by
hand and cannot catch a bracket moved to a site where the other stage is the right one** — that
question is a measurement, and after this rung it belongs to rung 8. No action is requested; the
loss is named at the function it guards so nobody re-derives the table believing it is checked.

---

## 2026-08-09 — Rung 3d shipped two zones where the corpus specified a 90.8 KiB `RoundRecord` column. Reversible; the owner should know what it costs.

**A SCOPE call, disclosed rather than asked**, because the fork itself was a perf/architecture one
and those are mine to decide with numbers. What lands here is the one thing the numbers do not
settle: a specified public surface (`Profiler::rounds(back) -> &[RoundRecord]`) does not exist, and
that is the owner's to reverse if the lost quantity matters.

**What the corpus asked for.** `RoundRecord { frame, round, dispatched, begin, end }`, 24 B × 121
frames × `MAX_ROUNDS_PER_FRAME = 32` = **90.8 KiB** of the reservation, keeping *"dispatch shape
only: rounds per frame, wave width, round span"*.

**What shipped.** Two zone sites on the dispatcher — `__round` (Span) and `__round_width`
(Counter) — from which rounds per frame is `__round`'s `count`, round span is its
`total`/`min`/`max`, and wave width is `__round_width`'s. All three named quantities, per frame,
with distributions rather than a single row each.

**Why, in the order the reasons actually weigh.**

1. **The write path.** This is the decisive one and it is not an optimisation argument. The
   dispatcher does **not** hold `&mut EcsMaster` while a round is in flight — the `UnsafeEcsCell` it
   minted is shared with the workers. Writing a column from there needs either a second published
   pointer into the reservation, written by a thread the fold's `&mut` does not cover, or a
   per-schedule scratch buffer flushed after the run — profiling state owned by the scheduler. A
   lane push has neither problem and is the mechanism rung 3c already blessed for `SystemSpan`.
2. **No truncation.** `MAX_ROUNDS_PER_FRAME = 32` would have counted the 33rd round of a
   deep-dependency schedule as *dropped* rather than measured, with its own drop class. Two zones
   truncate at nothing.
3. **90.8 KiB and one drop class not spent.**

**What is lost, and it is a real thing.** The **correlation** between one round's width and that
same round's span — "was the widest round also the longest?" Per-frame aggregates cannot answer it.
Nothing in the profiling corpus asks that question today, which is why the call went this way.

**To reverse:** restore `RoundRecord` with a scratch-and-flush path in `ExecutorScratch`, accept
the 32-round truncation and its counter, and keep the two zones or drop them. Say the word.

*(Two smaller departures ride along and need no decision: `Interval.sys` is `Interval.zone`, because
`sys_of` is gone and zone → system resolves at report time; and `G8` has no SKIP clause, because
`ThreadPoolBuilder::num_threads(2)` never consults the machine, so fewer than two workers would be a
threadpool defect rather than an environment to excuse. Both are argued in
`docs/diagnostics/profiling/05-LADDER-GATES.md`.)*

---

## 2026-08-09 — The dev residency budget has 1.3 MiB of headroom, not the 9 MiB the corpus's table implies. `J1`'s `Z` contradiction, now with a number.

**Recorded, not asked** — it is `J1`'s to settle and was already logged at rung 3a. What is new is a
measurement, and the measurement changes how urgent it looks.

`profiling_residency` now prints its configuration. On this box, armed, analysis ON:
**total 14 667 776 B (reservation 14 614 528, statics 53 248)** against a 16 MiB dev budget.

The corpus's own dev rows are **6.67 MiB** (analysis off) and **7.05 MiB** (on) — roughly half. The
gap is *not* the new interval ring, which is 262 144 B of it. It is `D8`'s `Z = 1024` against the
shipped `ENGINE_ZONE_SLOTS = 4096`: the five columns come to 21 B × 4096 × 121 = 10 407 936 B where
the table budgets 21 B × 1024 × 121 = 2 601 984 B.

So the sizing table and the shipped constant have disagreed by a factor of four since rung 2, and
the consequence is that the dev budget is at **87 % utilisation** rather than the ~44 % the table
suggests. `J1` owns the fix — either `ENGINE_ZONE_SLOTS` comes down to 1024, or every sizing row
in `profiling/01-EMISSION-STORAGE.md` is recomputed at 4096 and the retail budget re-derived with
it. Nothing is blocked today; the headroom is just much thinner than it reads.

---

## 2026-08-08 — The whole `92xx` code block described the wrong eighteen conditions. Repaired; recorded because of HOW it hid.

**Recorded, not asked.** The repair direction was not a judgement call and it is already committed.
What belongs here is the mechanism, because it is a hole in this project's own gate design and it
will recur.

**What was wrong.** L2 reserved eighteen `92xx` rows for the profiler — correctly, and for a good
reason — and then wrote eighteen plausible summaries composed from the code *numbers*. Sixteen of
them name conditions the profiling corpus does not have. `W9207` is the sharpest case: the corpus
pins it as **invariant TSC absent** in five documents, and logging's own `W0101` was **struck in its
favour**, so the invented summary ("a GPU query pool returned fewer results than were issued") left
the engine's only invariant-TSC code naming something else while the condition it was struck for had
no code at all. `9213` is `E9213` in the corpus (six mentions, four files) and was seeded `W9213`.

**Why nothing caught it, and this is the part worth keeping.** The registry has seven checks and all
seven were green. A `Pending` row owes **no doc page** (check 2 is `Live`-only) and **no emitter**
(check 3a is `Live`-only) — both narrowings are correct on their own terms, and L2 argued for them
in writing: *"otherwise L2 would owe eighteen pages for codes with no emitters, which is doc-rot
manufactured by a gate."* That reasoning is still right. But the two narrowings together mean a
`Pending` row's summary is compared against **nothing**, by construction — and the registry's own
check-4 message already names this defect class: *"inventing a summary here is how three rows of
this registry came to disagree with the messages the engine prints."* The registry documented the
failure mode, then shipped it at six times the scale, in the one status where no check could look.

**The generalisation, which is not repaired and is the reason this is written down.** A `Pending`
row is a **promise with no gate on its content**. Today the only thing that will ever check a `92xx`
summary is the rung that flips it `Live`, i.e. between one and fourteen rungs from now. Rung 2 flips
seven of them and reads the other eleven; the remaining eleven are still un-compared, and if a later
rung flips one without re-reading the corpus, the invented sentence ships.

**Two ways to close it, neither taken here** (both are bigger than rung 2 and one is a VALUES call):

* **(a) A check that pins every row's summary against the corpus.** Mechanically: for each `92xx`
  row, require its condition text to appear in `docs/diagnostics/profiling/05-LADDER-GATES.md`'s
  §Integration list. Cheap, and it would have caught all sixteen. Cost: it couples the registry's
  wording to a document's wording, which is a second statement of one fact — the exact shape this
  corpus deletes elsewhere.
* **(b) Do not seed summaries at all** — a `Pending` row carries the rung and an empty summary, and
  the summary is written when the emitter is. Structurally correct, and it is what
  `FORWARD_DECLARED` already does for the seventeen logging codes. Cost: `explain()` returns nothing
  useful for a `Pending` code, which is a real loss for anyone reading a corpus document today.

**Blocks nothing.** Rung 2 landed with the summaries repaired and seven rows flipped `Live` with
their pages. The other eleven are correct as of this commit and un-gated after it.

---

## 2026-08-08 — L5 shipped, and it weakened a specified latency bound. No decision needed unless you want it back.

**Recorded, not asked.** This is an architecture fork I decided with the tree in front of me; it is
here because it makes a *specified* number worse, and a number that quietly got worse is the thing
this file exists to prevent.

**The situation.** The logging corpus says `log_drain_system` runs "in `Last`", and states the
in-frame latency bound as **one frame** under `Scheduled` / "sink park + one frame" under `Thread`.
This engine has no `Last`. `CoreSchedule` is a **closed set of two** (`Main`, `Fixed`) and its own
doc gives the intended answer — *"finer-grained structure WITHIN a schedule is what Phase-15 sets
are for."*

**What shipped.** The drain runs in `Main`, `in_set(LogSet)`, and `LogPlugin::build` interns the set
so a host's `.before(LogSet)` resolves regardless of plugin add-order.

**What it costs.** With no ordering edge the scheduler may place the drain anywhere in the frame, so
a record emitted *after* it appears in the next frame's ring. **Each specified bound gains one
frame** for a host that does not add the edge. The drain has no data conflict with anything, so
nothing forces it late on its own.

**Three ways to get the frame back**, none taken, because all three are bigger than L5:

* **(a) Do nothing** — document the edge and let each host add `.before(LogSet)`. What shipped.
  Costs one line per host; costs a frame if a host forgets, silently.
* **(b) Add a third `CoreSchedule` variant.** Honest, matches the corpus verbatim, and touches the
  frame driver, the routing methods and every `add_systems_in` call site. An engine change to serve
  one subsystem.
* **(c) Give the engine a standing `EngineSet::Last`** that the app plugins order everything before.
  Cheaper than (b) and useful beyond logging, but it is a scheduling convention the engine does not
  have yet, so introducing it from the logging seam is the tail wagging the dog.

**Blocks nothing.** L16's `G15` is where the bound is actually measured, and it must be measured
against whichever of these is true then.

---

## RESOLVED 2026-08-08 — the owner chose (b): raise the budget. Q1 stands, no code changes.

**Decision:** accept the footprint and raise the gate's shipping budget from **1024 KiB to
1280 KiB**. `LANE_COUNT` stays 80 in every profile; `REGION_CAPACITY` stays 128; D15 keeps
committing the sample slab at first `arm`. **No source change of any kind.**

**Why it is the right call, in the owner's own framing.** The alarming number measured a
*reservation*, not what a machine holds. The row is now printed in three columns instead of one
that conflated them:

| Column | Shipping profiler | Meaning |
|---|---|---|
| declared / reserved | **1 208.2 KiB** | address space; free in any practical sense on 64-bit |
| committed at `arm` | **≈ 1 142 KiB** | the reservation, taken when diagnostics are turned **on** |
| resident, flag off | **≈ 0** | nothing armed, no lane claimed, no `.bss` page touched |

The owner asked whether this costs runtime or RAM. It costs neither in the shipped default: with
the flags off a site pays **one `.bss` byte load and one predicted branch**, and above the compile
ceiling it pays nothing at all — the site and its argument expressions are deleted. That per-site
floor is the only thing a runtime flag cannot remove, and no budget choice touches it.

**Consequences applied:** `G23a` and `G23b` are **unblocked** — they assert against 1280 KiB and
have a reachable green state again. Options (d) `REGION_CAPACITY = 64` and (a) per-lane lazy commit
are **retained below as levers**, not as work: pull one only if a measurement later says the
*committed* figure is too high.

*(Original entry follows, unedited — the record of why the call was made outlives the call.)*

---

## 2026-08-08 — ⚠️ Q1 raised the shipping diagnostics footprint by 1.07 MiB, and the profiler's "≤ 1 MiB retail" headline is now FALSE

**This supersedes the ≈ 2.08 MiB figure in the round-3 entry below.** The correct joint figure is
**≈ 3.15 MiB**.

**What happened.** At rung D0 I resolved architect blocker **Q1** by deleting `LANE_COUNT`'s build
profile axis — it was 32 in the shipping profiles while the quantity it indexes,
`boyko_threadpool::MAX_WORKERS = 64`, is unconditional, so 32 was unsound and below the topology's
own floor of 66. The resolution (80 in every profile, `455c074`) was correct and I stand behind it.
**What I did not do at the time was propagate its cost**, and four cells across the two plans were
sized by that constant:

| Cell | Was (32 lanes) | Is (80 lanes) | Kind |
|---|---|---|---|
| profiling `LANES` (`.bss`) | 8 KiB | 20 KiB | reserved |
| profiling **sample slab** | 192 KiB | **480 KiB** | **committed at first `arm`** |
| logging `LOG_LANES` (`.bss`) | 512 KiB | 1.25 MiB | reserved |
| logging `SAMPLE_CTR` (`.bss`) | 16 KiB | 40 KiB | reserved |

Profiler half **908.2 → 1 208.2 KiB**; logger half **1 220.26 → 2 012.26 KiB**; joint
**2.08 → 3.15 MiB**. The `dev` figures did not move at all, because `dev` was already at 80 —
**which is exactly why nothing caught this: every check that looked at one row looked at the row
that was still right.**

**Two things follow that are not "the number got bigger".**

1. **The profiler is now over its OWN budget**, not just the joint one: 1.18 MiB against a stated
   ≤ 1 MiB. Gates **G23a and G23b assert that bound**, so both now fail at the baseline — they have
   no reachable green state until this is answered.
2. **Only 288 KiB of the +1.07 MiB is committed memory.** The rest is `.bss` reserved extent whose
   resident cost is per *touched* lane, which is the property that made 80-everywhere affordable in
   the first place. The committed part is entirely the profiler's sample slab, which **D15** commits
   for all `LANE_COUNT` lanes at first arm.

### The call

> **RECOMMENDATION, final: (b) — raise the budget and restate the row honestly.** The owner asked
> the right question: *is 3.15 MiB actually a lot?* It is not, and more importantly **the figure
> measures the wrong thing.** It is a **reservation**. What a machine actually holds is:
>
> | Configuration | Resident |
> |---|---|
> | shipped title, diagnostics flag OFF (the default) | **~0** — every table is demand-zero `.bss` that nothing touches, nothing is armed, nothing is committed |
> | shipped title, diagnostics ON | the profiler's sample slab (480 KiB, committed at `arm`) plus the logger's *touched* lanes and staging — order of **1 MiB**, not 3.15 |
>
> Address space on 64-bit is free in any practical sense, and 1 MiB resident against a single
> 2048² RGBA8 texture at 16 MiB is not a trade worth buying with code. So the cheapest fix of all
> is the one that changes no code and no constant: **raise the gate's shipping budget** (1024 →
> 1280 KiB covers 1208.2 with headroom) and print the row in three columns — reserved,
> committed-when-armed, resident-when-off — instead of one number that conflates them.
>
> That also unblocks G23a/G23b immediately, which (d) and (a) do only after a code change.
> **(d) and (a) below are kept as the levers to pull if a measurement later says the committed
> figure is too high**, not as things to do now.

- **(d) — kept as the cheap lever, no longer the recommendation.** Set `REGION_CAPACITY = 64` in the shipping
  profiles instead of 128. The slab is `LANE_COUNT × 2 regions × REGION_CAPACITY × 24 B`, so
  `80 × 2 × 64 × 24` = **240 KiB** instead of 480, and the row lands at
  `66 + 240 + 636 + 6.8 + 11.4 + 8` = **968.2 KiB — under the 1024 KiB budget**, with Q1 intact.

  **Cost:** a region holds 64 samples instead of 128 before the fold must drain it, so a shipping
  build drops samples earlier under a burst. In shipping the tier is `Always` only, so the sample
  stream there is already an order of magnitude thinner than in `dev`.

  **What it does not cost:** not one line of code, not one branch on the hot path, not one gate
  re-specified. `REGION_CAPACITY` is *already* a per-profile constant — unlike `LANE_COUNT`, whose
  axis Q1 deleted as unsound — so this is the knob doing the job it exists for.

  *I proposed (a) first because I was looking at where the bytes are rather than at what is
  cheapest to remove. The order is the other way round: the constant that exists for this, then
  code.*

- **(a) — now the fallback, if measurement later shows 64 samples per region is too shallow.**
  Commit sample regions **per lane on first use** instead of all 80 at
  arm.

  **Performance is not the problem with (a); the gate is.** The hot path already loads
  `buf: AtomicPtr<Sample>` on every sample, so a null test is one `test`+`jz`, predicted
  not-taken after a lane's first sample — call it zero. The commit itself is one syscall per lane
  on a `#[cold]` path, ten of them over a process. Two real costs, though: the syscall lands
  **inside a frame** (a worker's first zone, during the first frames, where frame times are
  already noisy — mitigable by committing at `arm()` for lanes that already exist, since the pool
  is built before `arm`), and it adds unsafe surface to the profiler's hottest path. The one that
  matters most: **G23a/G23b stop being able to assert a single armed total.** The figure becomes
  warm-up-dependent, and a crisp gate becomes a "after N frames" gate. A shipping title on an 8-core box claims roughly `workers + dispatcher + host ≈ 10` lanes, so
  ≈ 60 KiB of slab instead of 480 — the profiler is back under 1 MiB **with Q1 intact and no
  constant changed**. It edits D15 ("committed once at first arm, never freed"), which is a
  shipped-behaviour decision, which is why it is yours and not mine.
- **(b)** Accept 3.15 MiB reserved / ~1.2 MiB committed and restate the headline.
- **(c)** Cut a table instead: logging's `LOG_LANES` (1.25 MiB), `SINK_OUT` (256 KiB), or the
  profiler's dynamic-zone arenas (96 KiB per the profiling plan, 40 KiB per `SEAM.md` — that
  divergence is still open and is the profiling plan's to close).

**Blocks:** profiling rungs 2 and 10 (G23a/G23b). Does **not** block logging L0 or anything on the
substrate ladder.

**The lesson, recorded because it is the fourth time this shape has appeared in this campaign:** a
total that is a perfect sum of its printed operands proves nothing about whether the *operands* are
current. 2.08 MiB was correct arithmetic over two halves the substrate had already invalidated. The
check that would have caught it is not "does the total add up" but "has anything this total depends
on been decided since it was written".

---

## 2026-08-06 — ⚠️ MEASURED: synchronization validation is not live, so the `-ValidationOn` leg proves nothing about barriers

**A genuine missing barrier changed no pixel and emitted no message.** Executed while resolving
piece 2's first step, which existed precisely to find this out.

The probe: delete the ONLY declared read of a resource with exactly one reader — the HZB pyramid's
mip `d-1` read — while the dispatch that reads it stays. Pass 0 writes mip 5, pass 1 reads mip 5, no
derived dependency.

| | messages | `SYNC-HAZARD-*` | golden |
|---|---|---|---|
| baseline (×2, same build) | 19 | — | byte-identical |
| **real missing barrier** | **19** | **none** | **byte-identical** |

The feature bit IS requested in `boyko_rhi_vulkan/src/device.rs`, but the instance chain degrades
**silently** when `VK_EXT_validation_features` is absent, and the whole 19-message baseline is
`vkCreate*`-time — nothing in it was ever produced by a recorded frame.

**Why this is here rather than merely recorded.** It is not a piece-2 fact. It says that the
engine's validation leg — the instrument this campaign has been leaning on since the P1-2
`-ValidationOn` repair — covers object, descriptor and format legality and **nothing about
synchronization**. Every "validation clean" claim in the campaign's commit messages is true and
narrower than it reads.

**Options.** (a) Leave it, and gate barrier correctness structurally (pin the derived barrier stream
by FIELDS, which is what piece 2's G4 now does). (b) Find out whether `VK_EXT_validation_features` is
genuinely absent on this device or merely not reaching the layer, and fix it if it is the latter —
this is a ~1-hour investigation and would restore a general-purpose instrument. (c) Both.

**My recommendation is (c), with (a) first**, because (a) is already specified and blocks nothing,
while (b) is worth doing before piece 3 — that piece adds the first pyramid READER, and a
read-after-write across two passes is exactly the hazard class the layer would catch and the golden
cannot.

⚠️ **A methodological note worth as much as the finding.** The FIRST probe was inconclusive by
construction: it deleted one of SIX declared readers of the same image, so siblings still carried
both the transition and the dependency and nothing was tested. Its negative result would have been
recorded as "the extension is absent on this device" — a true statement reached by an invalid route.
When probing for a missing dependency, count the OTHER declared accesses to that resource first.

---

## 2026-08-05 — CI's release leg is red, and two of the classes are STRICTNESS calls

Found while preparing the P1-5a baseline, which needs a leg that passes. Running CI's own command —
`cargo test --workspace --all-targets --release --exclude boyko_demo --exclude bench-bevy-vs-boyko`
([ci.yml:62](../.github/workflows/ci.yml), `:103`) — **fails on six targets**. Two more appear on a
second run, which is itself the diagnosis: those are flaky, not release-specific.

Six were mechanical and are **fixed**: five `#[should_panic]` tests over `debug_assert!` guards
missing `#[cfg(debug_assertions)]` (boyko_math, boyko_ecs, boyko_render ×3, plus two in
boyko_rhi_vulkan), and one missing `VB_PINS` entry that was mine — `vb_mesh_hzb` from VG R3 P1-2.

Two classes are left, and both are decisions about how strict a gate should be rather than
architecture forks, so they are here rather than taken.

### 1. `boyko_shaderdsl --test eval_byte_identity` — three failures, on the NaN SIGN BIT alone

`NaN (0xffc00000)` vs `NaN (0x7fc00000)`. Both quiet NaNs; the values are identical (a NaN is not
even equal to itself); only the sign differs.

This is the same family as what gate G3 measured on the depth pyramid the same day: **the sign of a
zero and the sign of a NaN are exactly the two bits no `<` in a program can observe**, which is why
hardware and optimisers are free to move them — G3 caught a driver fusing a compare-and-select into
a hardware min whose ±0 tie-break differs. Expecting either bit to be stable between `-O0` and `-O3`
is not well founded.

**Options.** (a) Compare NaN as "both are NaN" rather than by bits. (b) Canonicalise the sign before
comparing. (c) Leave it, and accept that the eDSL's release leg is not a gate.

**My recommendation is (a).** The contract the eDSL exists to enforce is about VALUES, and the sign
of a NaN is not a value. It costs nothing on the finite domain, which is the whole domain that
matters, and it stops a real gate from being permanently red — which is worse than a slightly
narrower one, because a red gate nobody can fix is a gate nobody reads.

### 2. Two global-state tests that are flaky under parallel execution

`boyko-scene bundles_s6::interner_is_off_the_per_frame_path` reads the process-global
`identity::interner_len()`. `boyko-ui zero_alloc::unchanged_frame_layout_pair_allocates_zero_over_baseline`
reads a global allocation counter — and reported a delta of **minus one**, an improvement its
`assert_eq!` cannot express while its own message says "no more than".

Both pass alone, pass under `--test-threads=1`, and pass in debug. They fail only in release with
default parallelism.

**Options.** (a) A serial guard in each test file. (b) `--test-threads=1` for those binaries in CI.
(c) Make the UI assertion `<=`, matching its own message, and serialise only the scene one.

**My recommendation is (a) plus the `<=` repair**, because the harness flag would slow every test in
those crates to fix two, and because an equality assertion that fails on an improvement will fail
again the next time somebody improves it.

---

## 2026-08-05 — SCOPE: the pyramid needs a core framegraph change (per-subresource sync state)

**Decided and under way, not blocking — recorded because it grows piece 1 beyond what its plan
scoped.** Step P1-5 (declare the HZB build passes) cannot be written against the framegraph as it
stands, and the framegraph says so itself.

**The wall.** `framegraph/graph.rs:360-445` carries `INVARIANT HZB-SUBRESOURCE-UNIFORM`: every
access to one `ResId` must declare the same `(base_mip, mip_count, base_layer, layer_count)`,
because `FrameGraph::state` is a `Vec<ResSync>` indexed by `ResId` alone and `transition` never
receives the span. The HZB build needs, on ONE image in ONE pass, a read of mip `6p-1` and a write
of mips `[6p, 6p+n)`.

That comment was written in advance, names this exact pass, and prescribes the answer:

> "PER-SUBRESOURCE TRACKING IS THE CORRECT LONG-TERM ANSWER, and this assert is its TRIGGER … the
> HZB build writes mip k while reading mip k-1. When that pass is authored, it trips this assert.
> That is the INTENDED way to discover the work … The response is to build per-subresource
> tracking, never to relax the condition until it goes quiet."

**It is not merely a debug assert.** In release the assert is compiled out and the derivation is
genuinely wrong, traced at 512×512: pass 0 first-touches mips [0,6) so only those leave `UNDEFINED`;
pass 1 then writes mips [6,10) with the state claiming GENERAL for the whole ResId, so the derived
barrier has `old_layout == new_layout` and mips 6..9 are **never transitioned** while the dispatch
writes them through storage descriptors declared `GENERAL`. Every extent with
`prev_pow2(max(W,H)) >= 64` reaches it.

**The workaround I rejected.** Three ResIds aliasing one `VkImage` over disjoint mip spans is
uniform by construction and needs no framegraph change. I turned it down for two reasons. It is
literally what the invariant's own text forbids ("not by making the declarations agree by hand"),
and it is a dead end one piece later: piece 3's cull selects a pyramid LEVEL per instance, and a
per-pass ResId cannot be named by a dynamic level. Taking it would be the interim design deferred to
later that this project has ruled out.

**What it costs.** A new step P1-5a ahead of P1-5, touching the core state machine every render path
compiles through. The byte-identity argument is strong — `SubRange::color_mips` is called by nothing
today and every existing `image_access` site passes `base_mip: 0, mip_count: 1`, so a per-mip
machine should fold to today's behaviour barrier-for-barrier — but "should" is why it gets its own
gate rather than a golden pin, which cannot see a redundant or a missing barrier.

**Nothing is blocked on an answer.** Architecture forks are mine to decide; this is here because the
SCOPE grew, and the owner may prefer piece 1 to stop at "allocated and compiled" and hand the
framegraph work to its own campaign. Say so and I will split it.

---

## 2026-08-04 — ⚠️ `golden.ps1 -ValidationOn` never enabled the validation layer on ANY `boyko-app` pin

**Found while gating VG R3 P1-2, and it is the vacuum-green shape again.** The switch is the
engine's validation-audit instrument; the campaign records a "Validation-ON audit — COMPLETE"
milestone that ran through it. On the 22 of 25 pins that boot through `boyko_app`, it could not
fail.

**Mechanism.** The backend gates the layer on a conjunction —
`config.enable_validation && BOYKO_DISABLE_VALIDATION unset` (`boyko_rhi_vulkan/src/device.rs:2362`)
— and `boyko_app`'s runner hardcoded `enable_validation: false`. `-ValidationOn` only ever
*stripped the env var*, i.e. satisfied the second conjunct while the first stayed false. The layer
was never requested, no messenger existed, and the scan for `[vk-validation]` lines therefore
reported **"clean (0 messages)"** unconditionally.

The runner's own doc said so, in a passage read as a design note rather than as a gate defect:
"The shipped runner does NOT request the validation layer… a debug validation knob arrives with a
later rung."

**Measured, not inferred.** I built a `512×512` image with `mip_levels: 12` (the legal max is 10).
`vkCreateImage` returned SUCCESS and the audit reported clean. With the fix, the same corruption
draws the exact message: `vkCreateImage(): pCreateInfo->mipLevels (12) must be less than or equal
to 10`.

**Fixed here**, because P1-2's load-bearing gate depends on it: `BOYKO_ENABLE_VALIDATION` opts the
runner in (absent ⇒ boot byte-identical to before), and `-ValidationOn` now sets it alongside the
strip.

**⚠️ WHAT IT REVEALED, AND THE OPEN QUESTION.** With the layer actually live, the `vb_mesh` pin
emits **19 validation messages** — a baseline nobody has seen:

| count | message |
|---|---|
| 9 | `vkCreateComputePipelines()`: compute shader uses descriptor `[Set 1, …]` |
| 6 | `vkCreateGraphicsPipelines()`: vertex attribute at location 1/2 not consumed by vertex shader |
| 1 | **`vkDestroyDevice(): VkDevice has 13 leaked objects that have not been destroyed`** |
| 1 | `vkCreateShaderModule()`: SPIR-V capability `Geometry` declared without the feature |
| 1 | `vkCreateShaderModule()`: SPIR-V capability `DemoteToHelperInvocation` declared without the feature |
| 1 | duplicate-limit warning |

The pyramid is not implicated: armed and unarmed logs are **byte-identical** after handle
normalization, so P1-2 contributes zero. But the two shader-capability messages and the 13 leaked
objects are real, they are on the flagship VB pin, and one of them is a resource leak.

**The question is SCOPE, not method.** Options: (a) I audit and fix the 19 now, before continuing
the pyramid — it is a leak and two feature-declaration bugs on the main path; (b) I finish piece 1
and take the validation baseline as its own campaign afterwards; (c) I fix only the leak now and
defer the rest. I lean (b): the 19 predate this work by a long way, the pyramid is proven clean
against them, and interleaving an unbounded audit into a decomposition that was created
*specifically* to keep scope local would undo the decomposition. But it is your call — this is
scope, and the leak is the kind of thing that gets worse while it waits.

---

## RESOLVED 2026-08-03 — the HZB feature design does not converge in one piece: decomposed

**Situation, measured over three review rounds rather than felt.**

| round | prior items closed | new blockers | new majors |
|---|---|---|---|
| 1 | — | 8 | — |
| 2 | 10 YES / 6 PARTIAL / 0 NO | 3 | ~12 |
| 3 | 31 YES / 12 PARTIAL / 2 NO | 6 | 11 |

Each round genuinely resolves most of what the last one raised, and each raises about as much
again. After round 3 **every substantive step carries a blocker** (S4, S6a, S6b, S7, S9); the four
clean steps are gates and records that depend on the blocked ones. So unlike the foundation case
there is no independent subset to land.

The new blockers have also changed CHARACTER, which is the useful signal. They are no longer "the
algorithm is wrong" — they are collisions with shipped invariants: a fourth route by which the
design disarms rung R2d-6 (doubling the survivor list breaks the very const-assert added in R2d-4
to prevent an out-of-bounds device read); an `+INFINITY` fixture vertex reaching a second, unfenced
host consumer on the shipped VB path; a capability that is a per-frame ECS fact gating objects
minted at boot with no seam named between them.

**What I read from that.** The feature is simply larger than one design pass can hold. The
foundation converged in a single round each because S1/S2/S3 were small, independent and
self-contained — not because the process was better there.

**Proposal.** Decompose the feature the way the foundation already was, and give each piece its own
design + review round:

1. **The pyramid alone** — allocate, build, gate against the S3 host oracle. No cull integration of
   any kind. It is self-contained, its oracle already exists, and its own blockers are local.
2. **The capability and the raster split alone**, inert — the second scope drawing nothing, proven
   byte-identical on the pins.
3. **The cull integration**, once 1 and 2 are shipped and the collisions above are concrete rather
   than predicted.
4. **The arming**, with the drawn-set gate.

**The cost, stated.** Four design rounds instead of one, and the feature lands later. **The
alternative cost**, also stated: a fourth whole-feature round that on this evidence resolves ~30
items and raises ~6 more.

**OWNER'S DECISION: decompose.** The four pieces above each get their own design + review round
and land independently, starting with the pyramid alone. Recorded so a later reader sees the four
rounds were a deliberate structure, not a design that kept failing.

**Blocks.** Nothing.

---

## RESOLVED 2026-08-02 — the depth-complexity fixture is Khronos Sponza (delegated to me)

**Situation.** Decision (b) above commits to a separate fixture with real occlusion. What it should
contain was not decided, and it is an asset question I raised rather than settled on my own — the owner then delegated it back to me.

The VG corpus is seven fetched Khronos glTF sample assets arranged on a 5x3x3 grid — chosen for
triangle density, and it has almost no occlusion by construction. A depth-complexity fixture wants
the opposite: large occluders with substantial geometry behind them. The classic choices are Sponza
(an interior with a colonnade that occludes heavily) or Bistro; neither is in the manifest today.

**OWNER'S DECISION: delegated to me.** Chosen: **Sponza, from the same Khronos glTF-Sample-Assets
family the density corpus already draws on**, fetched into `assets/vg_occlusion/` under its own
manifest, gitignored and content-pinned exactly as `assets/vg_corpus/` is.

**Why Sponza and not Bistro or Intel Sponza.**

- *Same source family as the existing corpus*, so the fetch script's shape, the licence posture and
  the gitignore precedent all transfer. No new infrastructure and no new licensing question — the
  three things that would otherwise make this a multi-session detour.
- *Same loader path*: glTF/`.glb` through `GlbMeshLoader`, already exercised by every corpus asset.
- *It has the right OCCLUSION STRUCTURE*, which is the whole point: a colonnade plus an upper
  gallery, so a camera at floor level down the nave has its far half hidden behind columns. That is
  exactly what the density corpus lacks by construction.
- *Size*. This session already hit zero free space at a 73 GB `target/`. Bistro is ~2.4 GB, and its
  glTF conversions vary in provenance — which matters more here than usual, because this repo pins
  by content hash and a pin on an artifact nobody can re-derive is not a pin.
- *Comparability*: Sponza is the published occlusion/GI benchmark, so a number measured on it means
  something to a reader outside this repository.

**The risk I am taking, stated rather than discovered later.** ONE scene is the same
vacuous-selection exposure the corpus notes warn about — a single framing can be chosen to flatter.
Mitigation is the corpus's own: several committed camera paths spanning degrees of occlusion (down
the nave = heavy; from the gallery = moderate; outside looking in = little), with the WEAKEST
binding, exactly as `orbit_mid` binds the density corpus. A win claimed off the heavy framing alone
would be the defect, not the fixture.

**Still to do when it is built.** The `source_url` / `archive_sha256` / per-file `glb_sha256` pins
are filled from the first verified fetch, the way `CORPUS.toml`'s were — not written from a guess.

**Blocks.** Any occlusion perf claim. Blocks no implementation work, and is not on the HZB critical
path.

---

## RESOLVED 2026-08-02 — Occlusion perf claim: option (b), a separate depth-complexity fixture

**Situation.** The VG corpus is a triangle-density instrument, deliberately recomposed at rung R0b′
to measure density rather than occlusion. Measured ceiling on it: **1 of 44 drawn instances at
`orbit_mid`** (the binding framing) and 11 of 31 at `approach_close`. A min-reduced HZB can only
reject instances that win zero pixels, so those are hard upper bounds — and they bound more than
occlusion, since an instance also wins zero pixels when it is sub-pixel.

So the HZB and two-pass occlusion work now in flight can be built correctly and gated for
correctness, but **no occlusion speed-up can be demonstrated on any content in this tree**.

**Options.** (a) Ship it correctness-gated with no perf claim, as rung R2d shipped structural —
honest, and leaves the claim unmade until content exists. (b) Add a scene with real depth complexity
(an interior, a street) as a *separate* perf fixture, kept out of the density corpus so the two
instruments cannot contaminate each other. (c) Accept the claim will be made by whatever project is
built on the engine, not by this repository.

**OWNER'S DECISION: (b).** Build a scene with real depth complexity — an interior or a street — as
a SEPARATE perf fixture, deliberately kept out of the density corpus so the two instruments cannot
contaminate each other. Until it exists, the HZB rung ships correctness-gated with no speed claim.

**Follow-on, and it needs an asset decision** — recorded as its own item below rather than assumed.

---

## RESOLVED 2026-08-02 — K2: option (c), stays deferred

**Situation.** The virtual-geometry campaign's kill criterion K2 requires a Nanite reference table.
It has never been produced (UE is not installed; I cannot install it — the flow requires accepting an
EULA and creating an account, which I must not do). K2's own text says an unproducible baseline
*forces a scope restatement*: an absolute target instead of a relative one.

**Options.** (a) Owner installs UE and produces the table. (b) Restate the goal against an absolute
target (frame time at a stated triangle count and error bound) and record K2 as taken by its own
escape hatch. (c) Leave deferred and keep the goal formally unfalsifiable.

**OWNER'S DECISION: (c).** Stays deferred; the goal remains formally unfalsifiable, knowingly. No
rung is blocked by it. Recorded rather than quietly dropped, so a future reader does not mistake the
campaign's silence on K2 for K2 having been satisfied.

---

## RESOLVED 2026-08-02 — Cross-frame occlusion soundness

**The worry was mine and it was misframed.** A previous-frame pyramid is indeed not conservative —
but only for a ONE-pass cull. In two-pass it is never the last word: soundness lives entirely in the
late pass, which tests against a pyramid built from THIS frame's depth. The early pass is an
*unverified heuristic* whose only job is to fill the depth buffer with a good occluder set; its
mistakes cost late-pass work and never cost geometry. The theorem quantifies over every possible
early-pass output, so nothing about the early pass has to be proven at all.

Confirmed against practice rather than assumed: UE5 Nanite, Assassin's Creed Unity (SIGGRAPH 2015),
Granite, Bevy 0.16 and Unity 6's GPU Resident Drawer all have this same structure.

Full statement and proof: [VG-R3-HZB-PLAN.md](VG-R3-HZB-PLAN.md) §1. **No owner decision needed.**

---

## RESOLVED 2026-08-02 — HZB: option (a), then REVISED TO (b) — foundation first

**Situation.** With soundness settled, the implementation design was reviewed and returned REJECTED
by both reviewers. The blockers are real, not stylistic — among them: the design revives
frustum-culled instances and thereby deletes rung R2d-6's arming; unknown mesh bounds produce a
PERMANENT false reject for any streaming-in mesh, surviving both passes; the one gate that can see a
false reject cannot be built as specified, because `vb_depth` carries no `TRANSFER_SRC` and the
readback path it depends on is listed UNVERIFIED while being load-bearing; and the pyramid build,
being compute, must split the VB raster's single dynamic-rendering scope in two, which the plan does
not address — a naive second scope would `LOAD_OP_CLEAR` away the early pass.

Full list: [VG-R3-HZB-PLAN.md](VG-R3-HZB-PLAN.md) §5.

**Options.** (a) One more design revision round against the 8 blockers, then implement — the same
loop that took rung R2d from 8 blockers to shipped. (b) Implement the uncontroversial foundation
first (S1 the RHI `TextureView`, S2 the framegraph guard, S3 the host oracle) while the cull design
is revised — these three are independently useful and none depends on the disputed parts.
(c) Park the rung.

**OWNER'S DECISION: (a).** One full revision round against all 8 blockers, then implement in step
order. My own recommendation had been (b) — land the uncontroversial foundation first — and it was
not taken; (a) is the same loop that carried rung R2d from 8 blockers to shipped, and it keeps the
step order intact rather than interleaving foundation work with a design still in motion.

**REVISED TO (b) the same day, by the owner, after the revision round returned.** The round closed
every prior blocker (10 YES / 6 PARTIAL / **0 NO**) and produced 3 NEW blockers plus a dozen majors
— and every one of them lands in the FEATURE: candidate routing, a capability predicate missing its
`mesh_leg` conjunct, a boot clear needing a `TRANSFER_DST` the image is not created with, an
un-ringed per-frame UBO, an unobservable `prev_view_proj`, an unexecutable anti-vacuity clause.

**Not one lands against the foundation** — the RHI `TextureView`, the framegraph subresource guard,
or the host oracle. Three design rounds, zero blockers there. That is evidence rather than
preference, and it inverts my original reason for recommending (b): it was a hunch then, it is a
measurement now.

Some blockers exist BECAUSE the foundation does not: one says outright that a step's acceptance
cannot be executed at that step because the instrument does not exist yet. Building the foundation
first removes a class of objections rather than postponing it.

**So: implement S1 (RHI `TextureView`), S2 (framegraph subresource guard) and S3 (host oracle) now.**
Each is independently correct, needed regardless of whether occlusion culling is ever armed, and
none depends on a disputed part. The feature's design continues to settle against its remaining
blockers, on a foundation that by then exists.

**Blocks.** The occlusion feature only. The foundation proceeds.

---

## 2026-08-07 — TWO THINGS PIECE 3 CANNOT DECIDE FOR ITSELF

VG R3 piece 3 is COMPLETE and pushed (`b6337dd`..`6a9a7f9`). Two items are blocked on the owner,
and neither is a defect.

### 1. Four new pins are UNBLESSED, and blessing is not mine to give — **RESOLVED: blessed @e160434**

**The owner reviewed the BMPs and signed off; all four legs now record
`85b7d378…4d2913d9` and re-verify green.** The review raised one real question — the corner
spheres look stretched — which was investigated before blessing, not waved through: the
silhouettes are ellipses with RADIAL major axes at `1/cos θ ≈ 1.18` (FOV_Y = 52°), and a
pixel-exact Bevy 0.14 replica of the two corner spheres reproduced the same ellipses to 0.2 px.
Rectilinear perspective, not a defect. The original record follows.

`goldens/PINS.toml` gained `vb_occ_mixed_off`, `vb_occ_mixed_keep`, `vb_occ_mixed` and
`vb_occ_mixed_late`, every `sha256_*` seeded with the literal `PENDING`. That is the path the file's
own header prescribes for adding a leg by hand; `golden.ps1` reports "NO PIN recorded" and exits 2
on all four rather than passing. **Verified, all four.**

All four render the SAME image:

    actual = 85b7d3788130a8bb65f0b5b92ba86c71499bd7a4babe7d6900a711944d2913d9

That identity across four regimes — disarmed, FORCE_KEEP, armed (defers 4), FORCE_LATE (defers 6,
re-admits 2) — is the piece's central claim: the cull rejects geometry and the picture does not move.

**What is needed:** a visual sign-off on the freshly-dumped BMPs. Then bless `vb_occ_mixed_off`
first (both legs) and verify the other three reproduce the same literal;
`the_pins_declared_byte_identical_actually_agree` keeps them from drifting afterwards. Until then
those four gates claim nothing — `PENDING == PENDING` is vacuous, and the guard's own doc now says so.

### 2. Piece 4 has no plan, and its scope is a VALUES call — **RESOLVED: planned @799db99, shipped P4-1…P4-7**

**Both halves are answered.** `docs/VG-R3-P4-CONFIG-AND-INSTRUMENT-PLAN.md` went through four
architect × four critic rounds to APPROVED (@799db99) and landed as seven rungs, each committing
alone and green: `49e5630` · `28c3772` · `85b3313` · `c7465bf` · `58687d3` · `cf2d367` · this one.

- **The config field exists.** `boyko_render::OcclusionConfig { mode: OcclusionMode }` — two
  variants, `Off` (default) and `TwoPhase` — a Resource on `HzbConfig`'s surface, read live per
  frame. `BOYKO_VG_OCC_FORCE` and its boot panic left shipping code; the verdict overrides are now
  `boyko_app::OcclusionForce`, a test instrument. **`FORCE_KEEP` is no longer the disarm route;
  `OcclusionMode::Off` is**, and unlike `FORCE_KEEP` it suppresses the split predicate, the late
  passes and the extra descriptor-set bindings.
- **The number exists, and it says NOT RESOLVED.** Ten timestamp brackets in the shipping recorder,
  the piece-3 protocol re-run on that channel: `NetRun +10 240 ns` against a band of `49 152 ns`.
  Every contrast `NOT RESOLVED` — and that is a RESULT, not a failure: the instrument resolves
  per-pass costs, these fixtures do not separate the arms. **The default stays `Off` as a SCOPE
  statement**, not as an inconclusive measurement: default-ON would need `NetRun < −band` across ≥3
  sittings on ≥2 fixtures of differing occlusion density *and* a second consumer for the pyramid, so
  that `HzbBuild` is not charged to this feature alone. P4-6 is one campaign, two fixtures, one
  machine, no second consumer.
- **What is left for the owner is one VALUES call**, recorded in the next item: flipping the default.

The original record follows.

There is no `VG-R3-P4-*.md`. Piece 3's own text assigns piece 4 the **owner-facing config field** —
occlusion culling as a setting rather than an env var — and until then the supported disarm is
`BOYKO_VG_OCC_FORCE=keep`. That is a product-surface decision, not a perf/architecture fork, so it
is not mine to settle.

Piece 4's other inherited job is a number. The three-number measurement returned **NOT RESOLVED on
every contrast**, and the reason is structural rather than statistical: nothing brackets
`vb_batch_cull`, `vb_cull_late` or the late raster scope with timestamps, and `swapchain.rs` sets
`VK_PRESENT_MODE_FIFO_KHR` unconditionally, so wall clock is bounded below by the display refresh
(measured 6.893 ms/frame = 145.1 Hz). The zero control came in at 0.47 % against a resolution band
of 287.91 %. Adding the bracket touches the shipping recorder, which piece 3's boundary excludes —
so it is piece 4's first job if piece 4 wants a number.

---

## 2026-08-07 — VG R3 piece 4 is COMPLETE: two VALUES calls, and the dispositions that close the piece

Rungs P4-1…P4-7 all landed. **Nothing here blocks anything** — the piece ships with the default that
was designed for it. Two items are the owner's to decide, and the rest is recorded so no disposition
is left implied.

### VALUES 1 — should `OcclusionMode` default to `TwoPhase`?

It is one attribute, and a real behaviour change: with piece 4's host disjunct, any world carrying an
`OcclusionCulling` marker would then build a depth pyramid by default.

**My position: no, and it is not an inconclusive measurement.** The decision's failure mode is
DELETED GEOMETRY while its upside is bounded by the early raster's share of a frame — the same
asymmetry that makes the marker itself opt-in. On this corpus the benefit is provably zero (a
converged static scene's late scope correctly draws nothing) and the cost is not. The bar for
flipping it is written down and unmet: `NetRun < −band` across ≥3 sittings on ≥2 fixtures of
differing occlusion density, **and** a second consumer for the pyramid so `HzbBuild` is not charged
to occlusion alone. P4-6 is one campaign, two fixtures, one machine, no second consumer.

### VALUES 2 — present mode is a product surface, and nothing owns it

`present/swapchain.rs` creates the swapchain with `VK_PRESENT_MODE_FIFO_KHR` **unconditionally**, so
every wall-clock measurement in this repository is bounded below by the display refresh, and there is
no owner-facing way to ask for anything else.

Piece 4 deliberately did **not** fix it (disposition (c1)). The reason is not cost: the channel it
would have improved — host wall clock — was superseded by the timestamp brackets and is now labelled
`KNOWN-BLIND`, deciding exactly one thing (*did arming the instrument wreck the frame?*). Present
mode is vsync, tearing and power: an owner-facing `PresentConfig`, and a product decision, not an
occlusion piece's business. **Recorded here as a VALUES item rather than silently carried as a
perf TODO.**

### The dispositions, so none is implied

| item | disposition |
|---|---|
| **(c1)** unconditional FIFO present mode | **OUT** — superseded channel + product surface. VALUES 2 above |
| **(c2)** D8: `vb_indirect_late`'s provenance is covered by nothing | **OUT, BOOKED to framegraph core.** Piece 4 declared no new access, so it neither improved nor worsened it; P4-5 additionally asserts the shipping late chain is field-identical with and without the readback probe. The fix is P2-7's `is_write \|\| res_written \|\| res_seeded` change plus a 14-site audit, whose only gate is a replica this campaign has MEASURED blind to the class it would catch — so that rung needs an instrument before it needs code |
| **(c3)** PROBE-ON barrier-stream rows | **DONE at P4-5**, as a derived delta. Two findings: the plan's re-sourcing prediction is refuted by the tree, and the probe's perturbation is larger than any doc said — nine declared accesses over two passes on seven buffers, eight derived barriers, five pinned barriers moving |
| **(c4)** the intra-pass `TRANSFER → COMPUTE` edge on `VbCullUniform` | **DONE at P4-3, the record-order half only** — and it is a COMPILE-time red in both profiles, where the plan's own shape would have stayed green on the very defect it existed to catch. The DECLARATION half stays open (OQ 9): `FrameGraph::pass_access_count` is private and there is no per-pass accessor |
| **(c5)** the stale future-tense header in `vb_occ_split_gate.rs` | **DONE at P4-7.** It sat in FOUR places, not the two the plan named |
| **(c6)** `goldens/PINS.toml`'s UTF-8 BOM, which strict TOML rejects | **KNOWINGLY LEFT, and the reason is measured.** `golden.ps1 -Bless` writes the file back with `Set-Content -Encoding UTF8`, and the only PowerShell on this box is 5.1, whose `-Encoding UTF8` is BOM-**ful** — verified by round-tripping a BOM-less file through that exact call and getting `EF BB BF` back. Stripping the BOM alone would be silently undone by the next bless; the fix belongs at the WRITER and lands with a bless run. No impact today: `golden.ps1` parses with line regexes, and every strict-TOML check in this campaign strips the BOM explicitly |

### Two gaps piece 4 opened and could not close, recorded rather than absorbed

- **The pin-binary split gate runs PROBE-ON while the pins run PROBE-OFF.** The gap is small and
  named: `vb_probe_dump` is a host-side counter sink that records no commands and cannot enter the
  split predicate. It is still a gap.
- **The dual-read equality invariant is dev-profile only.** A release bench run does not execute it.
  If a release-only divergence between the two query readers is ever suspected, the check has to be
  re-run in the dev profile on the same scene; nothing in the ladder can detect it in release.

---

## 2026-08-07 — RESOLVED — the cull verdict divided, and a division cannot agree with a host oracle

**ANSWERED and implemented in the same session. Kept here in full because the reasoning is the
transferable part, and because the FIRST reading recorded below was wrong in a way worth preserving:
it named a direction from a sample of one.**

**Resolution.** The verdict no longer divides. `for all i: cz_i < occ * cw_i` replaced
`max_i(cz_i/cw_i) < occ` in the shader and in `boyko_render::hzb`'s oracle, `depth_near` moved under
`#ifdef VB_CULL_DEBUG_PROBE` so the shipping module no longer computes the quantity that used to
decide, and the boundary corpus was re-derived to plant against the new predicate. Measured after:

    DepthNearCensus { compared: 72, identical: 72, gpu_below: 0, gpu_above: 0, max_ulps: 0 }
    verdict disagreements: 0 of 72
    24 EXACT-TIE KEEP probes, 24 strict KEEP probes, 24 strict REJECT probes

The tie arm is what proves `<` is strict, and it is now reachable by construction rather than by
luck: the plant uses `z = near·2^k` and `occ = 2^-k`, both dyadic, so the tie is exact on both sides.

**One thing the fix cost, and it is the part worth remembering.** Re-pinning the artifact census
showed `op_ford_less_than` going DOWN by two at the exact step that ADDED a per-corner comparison —
because `!(cz < bound)` lowers to `OpFUnordGreaterThanEqual`, which the census had no field for. A
census that counts only the ordered compare would have read a verdict's *deletion* as a small
decrease and pinned it without comment. The field was added
(`op_funord_greater_than_equal: 4`, two of them the verdict, one per inlined copy).

---

### The finding as originally recorded

**Not a blocker. Recorded because it is a MEASURED correctness finding, and because the fork it
opened was mine to decide — the owner should be able to overrule it before it ships.**

VG R3 piece 3 step P3-4 (the occlusion leaf) is in the working tree, uncommitted. Its new gate
`crates/boyko_app/tests/hzb_verdict_oracle_gate.rs` runs four corpora. Three pass, including the
131,072-pair random corpus and the sentinel corpus. The fourth — exact tangency — fails on its first
probe:

    [64x48 boundary probe 0 (equal)] batch 0: the record's instanceCount is 0 but the oracle
    keeps 1 of 1 instances early. (deferred: gpu 1 / oracle 0)

The GPU **rejects** where the oracle **keeps**. The shader's comparison
(`vb_batch_cull.comp.hlsl:872`) is `return depth_near < occ;` — strict, and correct: equality must
keep. So the operator is right and the VALUE differs — the shader's `depth_near` lands below the
host's. The shader's own comment at `:766-767` named this in advance as *the geometry-deleting
direction*, and the fixture's comment at `:1309-1313` predicted the exact signature: a 1-ULP
disagreement "would show up as a failure on the exactly-equal arm and nowhere else."

**Both predictions were written before the run, and both came true.** The gate is working. This is
the campaign's eighth instance of the pattern — and the first where the instrument caught the defect
instead of being vacuous over it.

### The measurement, and what it overturned

A `-D VB_CULL_DEBUG_PROBE=1` variant now exports the shader's own `depth_near`, level and taps, so
the divergence is OBSERVED rather than inferred. The shipping module is untouched: the
macro-undefined source preprocesses character-identically and `vb_batch_cull_spv_byte_identical`
stays green, so the numbers describe the module that actually ships. Over 72 boundary probes:

    DepthNearCensus { compared: 72, identical: 66, gpu_below: 3, gpu_above: 3, max_ulps: 1, incomparable: 0 }
    verdict disagreements: 2 of 72   (one host=Early gpu=Late, one host=Late gpu=Early)

**This overturns the first reading above.** The divergence is NOT in the geometry-deleting
direction — `gpu_below` and `gpu_above` are 3 and 3, and the two verdict disagreements point
opposite ways. It is symmetric rounding at 1 ULP, not a bias. The first reading came from a single
probe, which is exactly the sample size at which a direction claim is worth nothing.

`level` and all four `taps` are IDENTICAL on every one of the 72 probes, so the window rect and the
level selection already agree exactly and only the depth differs.

### The cause, and why it closes option (A)

Under the corpus matrix, row2 = `[0,0,0,near]` and row3 = `[0,0,1,0]`, so `cz = near` and `cw = z`
are exact and bit-identical on both sides — corroborated by the identical taps. The only inexact
step left is the reciprocal. Vulkan's precision appendix specifies `OpFAdd`/`OpFSub`/`OpFMul` as
correctly rounded but allows **`OpFDiv` 2.5 ULP** at 32-bit; Rust's divide is the IEEE 0.5-ULP one.
`precise` emits `NoContraction`, which constrains contraction and reassociation and says NOTHING
about a division's ULP allowance.

So **(A) as originally posed is dead** — not expensive, *impossible*: no amount of tightening the
existing fold reaches bit-exactness, because the gap is a spec allowance, not a code shape. The
shader comment at `:909-910`, which claims `precise` "forbids substituting a reciprocal-estimate",
is a false claim and is being corrected.

**(B) is also wrong now** that the direction is known: rounding UP would trade one arbitrary
direction for another, and the 1-ULP bound it would lean on is measured on 72 probes on ONE device
while the spec permits 2.5.

### What is being done instead: remove the division from the DECISION

For `cw_i > 0` — already guarded by the behind-eye early-out —

    max_i (cz_i / cw_i) < occ    <=>    for all i:  cz_i  <  occ * cw_i

The right-hand form is one correctly-rounded multiply under `NoContraction`, so the shader and the
oracle agree **by construction** rather than within a tolerance, and it is *cheaper* than the divide.
That is an exact reformulation, not a relaxation and not a bias — which is why it is preferred over
every option originally listed. The window rect keeps its divide, which is measured to agree exactly.

**(C) — declare tangency untestable and relax the arm — stays rejected**, and is now unnecessary.

---

## 2026-08-07 — Profiling + logging, review round 3: both REJECTED, and the SEAM is incompatible

The two plans reached revision 3 and were reviewed a third time — separately, and for the first
time **against each other**. Verdicts: profiling `REJECTED (6 blockers)`, logging
`REJECTED (10 blockers)`, seam `INCOMPATIBLE AS WRITTEN (6 blockers)`. Revision 4 is in flight;
these three items are not the reviewers' to decide.

### The seam was never designed, and that is the round's main finding

Two prior rounds read one plan each. The first reader of the seam found the two documents
asserting **contradictory facts**: profiling justifies moving its ABI into `boyko_utils` because
that crate has zero dependencies; logging states flatly that `boyko_utils` depends on `boyko_log`.
Both cannot hold. Below that, each plan independently invented the same four primitives — a
per-thread lane index, an `rdtsc` calibration, a never-freeing lane allocator, and a loss
accounting — with incompatible semantics: one worker would be lane 5 to the profiler and lane 37
to the logger, and only one of the two clocks would know about a suspend/resume. That is precisely
the failure Principle 0 names: a capability two subsystems need, built twice as per-crate adapters
instead of once as a kernel feature.

**Decided by me, not the owner** (architecture, per the standing agreement): a new zero-dependency
bottom crate `boyko_diag` owns the clock, the lane registry, the loss vocabulary and the
never-freed storage policy; it is *diagnostically mute* (it emits no `boyko-####` code and prints
nothing, which is what keeps the graph acyclic); `profiling_abi` is hosted there rather than in
`boyko_utils`, which keeps its empty `[dependencies]`. Full design:
`docs/DIAGNOSTICS-SUBSTRATE-PLAN.md`.

### VALUES 1 — how much does a SHIPPED title pay for diagnostics?

Nobody had computed the joint number. Measured from the two plans' own tables:

| | profiling alone | logging alone | **jointly** |
|---|---|---|---|
| dev, `.bss` + reserved | 6.65 MiB | 3.46 MiB | **9.33 MiB** (10.11 naive; the shared crate saves 0.78) |
| **shipping** | 0.85 MiB | 1.16 MiB | **1.95 MiB** — **WRONG; corrected immediately below** |
| hot-path cache lines | 3-4 | ≤ 4 | **7-8** |

> **CORRECTED 2026-08-08 — the shipping figure above was already wrong on the day it was first put
> to you, and it is corrected in the open here rather than quietly re-based.**
>
> **1.95 MiB has never equalled the sum of its own operands, in any revision.**
>
> - As put to you above (rev 3): `0.85 + 1.16 = **2.01**`, printed as **1.95**.
> - At the corpus's first carved revision the operands moved and the total did not:
>   `0.89 | 1.15 | naive 1.95`, and `0.89 + 1.15 = **2.04**`.
> - **Then a second, independent error surfaced underneath the first.** The logger re-derived its
>   own `shipping` column term by term (`docs/diagnostics/logging/01-EMISSION-RING.md:130`:
>   512 + 32 + 16 + 16 + 4.25 + 0.008 + 256 + 64 + 320 = **1 220.26 KiB ≈ 1.19 MiB**) and showed
>   that **no subset of its rows sums to the 1 180 KiB** the seam was quoting — so 1.15 was not a
>   different configuration, it was wrong too.
>
> There is no third quantity 1.95 could have been. Both revisions state that the shared substrate
> saves **ZERO bytes in shipping** — the 0.78 MiB saving is dev-only — and with a zero saving the
> joint figure simply **is** the naive sum. **The corrected shipping figure is ≈ 2.08 MiB**
> (908 + 1 220.26 = 2 128.26 KiB), against ≈ 2.01 MiB on the numbers as they were handed to you.
> The error ran against you every time: the ask was understated by 0.06 MiB then and by 0.13 MiB
> now.
>
> 🔑 **The lesson that outlives the number, because it defeated a repair pass whose stated job was
> to catch exactly this.** After the first correction the seam table was *internally consistent* —
> `0.89 + 1.15` really is `2.04` — and that is precisely why the stale **operand** survived. **A
> total that checks out against its printed operands proves nothing about those operands.** The
> durable rule: with a zero shipping saving the joint figure is the sum of the two columns, and
> any edit to it must re-read the source rows it quotes rather than re-adding the numbers already
> printed beside it.
>
> **What the figure MEANS also changed, and that narrows what is being asked.** S13 —
> *free when not enabled*, folded in after this entry was written — moved every syscall, thread,
> hook and first write off the boot path and onto the enable path, so a shipped process that never
> enables diagnostics **never touches these tables at all**. An untouched all-zero `.bss` table is
> emitted by the linker with a virtual size and no raw data, so ≈ 2.08 MiB is **declared address
> space, not resident RAM** (`docs/diagnostics/SEAM.md` §S13, MEMORY row). Two limits on that,
> both stated by the corpus itself rather than smoothed away: the property holds **only if boot
> touches nothing** — one write to one lane buffer commits that page and it is lost for that page
> — and the corpus **explicitly refuses to claim** that the loader leaves an untouched page
> uncommitted (`substrate/section-report` proves the bytes are absent from the *image*, and no
> more; `docs/diagnostics/substrate/05-LADDER-GATES.md`, gate DG12).
>
> **So the question is narrower than this section's heading suggests.** Not *"what does a player's
> machine spend on diagnostics"*, but: **is ≈ 2.08 MiB of declared address space — resident only
> in the sessions where diagnostics are actually switched on — an acceptable price for a shipped
> title?** Still a VALUES call, and still not mine.

So the profiling plan's headline **"≤ 1 MiB retail" is false in the configuration that will
actually ship**, and the shared substrate saves **nothing** in shipping — its 0.78 MiB saving is
dev-only. It is bought for correctness (one lane number, one clock epoch, a loss report that
cannot itself be dropped), not for footprint, and neither plan may claim otherwise.

Cutting **≈ 2.08 MiB** means cutting one of: logging's 32 × 16 KiB lanes (512 KiB), `SINK_OUT`
(256 KiB), or the profiler's dynamic-zone arenas (96 KiB — *the current revision states this third
candidate as **40 KiB** in `shipping`, `docs/diagnostics/SEAM.md` §Open — needs the OWNER, item 1;
the divergence is recorded here, not resolved, because resolving it belongs to the profiling
plan*). **This is a VALUES call about what a player's machine spends on diagnostics, and it is not
mine.**

### SCOPE 1 — what does `shipping-min` actually mean?

Logging's `shipping-min` exists for a title that wants **no resident diagnostics thread**. But
profiling's `Always` tier still writes a telemetry window synchronously on the dispatcher, so such
a title pays a periodic `write_all` anyway. Either `shipping-min` also disables telemetry, or the
profile does not mean what its name says. **SCOPE call.**

### SCOPE 2 — the plans are growing faster than they are converging

Three review rounds, and the blocker count has not come down: **35 findings → 17 → 22**. The two
documents are now 3370 lines for two subsystems that do not exist as a single line of code, and
more than half of round 3's new blockers were introduced by what round 2 added — the game-facing
half — while the seam only became visible because both documents grew into full architectures.

That is a signal about **how much is being designed at once**, not about the reviewers. The
alternative is a narrowed first tranche — `boyko_diag` + CPU zones + log levels — built and
measured, with telemetry, retention and the game-facing API returning afterwards on a working
foundation. Stated here as an option; **the owner decides the scope, not me.** Work continues on
the full revision 4 unless told otherwise.

### SCOPE 3 — the workspace's `--cfg loom` leg has not compiled, and nothing said so

Found at rung D1, 2026-08-08, while checking that the new `boyko_threadpool -> boyko_diag` edge
did not disturb the loom build. It did not. The loom build was **already broken**:

```
RUSTFLAGS=--cfg loom cargo check -p boyko-threadpool --lib
error[E0599]: no method named `get_mut` found for struct `loom::sync::atomic::AtomicPtr<T>`
  --> crates/boyko_threadpool/src/scope.rs:185:39
```

loom 0.7.2 offers `with_mut`, not `get_mut`. `-p boyko-ecs` fails on the **same** error because it
reaches the same lib, so **both** crates that carry a `[target.'cfg(loom)'.dependencies]` block are
dead, not one. `rg loom .github/workflows` returns nothing — no CI leg passes `--cfg loom`, which
is why this has been invisible. Confirmed not to be a D1 regression: `git stash`-ing the D1 diff
reproduces the identical error at `93dbcf8`.

Why it needs a decision rather than a quiet fix:

1. The substrate plan cites these two crates as the working precedent `boyko_diag`'s `claim_lane`
   loom model will copy. The **manifest** shape is a valid precedent; the claim that a model *runs*
   beside it is not. The plan text is now qualified — the citation is not silently left standing.
2. The fix at `scope.rs:185` is one line inside an `unsafe` `Drop` on the scope-teardown path, and
   a green `cargo check` under `--cfg loom` is **not** a run model. Making the leg mean something
   means running the models, and this machine crashes loom **release** binaries at startup
   (recorded previously, unrelated).
3. So the real question is scope: (a) repair `scope.rs:185`, run the existing models in debug, and
   add a CI leg that keeps them compiling; (b) repair it and add no CI leg, accepting the same
   silent rot later; or (c) leave it, and land `boyko_diag`'s concurrency evidence as the
   proptest + Miri legs only, stating in the plan that no loom model backs `claim_lane`.

**Not repaired at D1** — it is outside D1's subject and the choice above is not mine. The lane
claim path's Miri and property legs are unaffected and still planned.

---

## KNOWN FRICTIONS — no decision needed, recorded so they are not rediscovered

- **`target/` grows without bound and silently breaks builds.** It reached 73 GB and hit zero free
  space mid-build this session. The failure presents as a *mingw linker error*; the real cause is on
  the last line (`no space on device`). `cargo clean` recovered 73.6 GiB.
- **The trybuild test fails under a concurrent full-suite run** (`compile_fail_frame_write_token`,
  3/3 fixtures) and passes standalone. trybuild spawns its own cargo into the same `target`. Not a
  flake — reproducible contention.
- **`cargo test` STOPS at the first failing binary, and the suite count silently shrinks.** A run
  that trips the flake above reports ~51 suites instead of ~445 — so "I ran the full suite" can mean
  "I ran a ninth of it" with nothing in the output saying so. Always pass `--no-fail-fast` when the
  claim being made is about coverage, and read the suite COUNT, not just the failure count.
- **graphify has been off-target for the render/VB path** for this entire session; every query
  returned `boyko_demo` internals. Grep/Read is the working path there.
- **The `graphify` binary is not installed in this environment at all** (not on `PATH`), while the
  `PreToolUse` hooks demand `graphify query` before every Read and Grep. `graphify-out/` exists but
  its newest subdirectory is from July, so the graph is also stale. The hooks only remind, never
  block — but they fire on every file access. Fix is either an install or a hook that checks the
  binary exists before demanding it.
- **The ECS's global query-type registry can exhaust under the full lib suite.**
  `MAX_QUERY_TYPES = 1024` is a process-global cap minted lazily, and `boyko-ecs --lib` runs 864
  tests in parallel. When scheduling happens to mint the 1025th distinct query shape, whichever test
  is unlucky dies with a TERMINAL panic naming the cap. Observed once, then 3 consecutive clean runs
  of the same binary. It is order-dependent, not a regression signal — check by re-running before
  bisecting anything.
- **`boyko-engine --test internal_docs_anchors` is RED on `master`'s content, and has been for a
  while.** 13 stale line anchors (10 in `docs/MESHLET-VIRTUAL-GEOMETRY-PLAN.md`, 3 in
  `docs/SYSTEMS.md`) plus 8 over-waivers against a cap of 6 in the meshlet plan. **Proved
  pre-existing at L5** by stashing the whole rung and re-running: byte-identical failure. It makes
  `cargo test --workspace` red for everyone, so the next person to run it will spend the same
  fifteen minutes proving it is not theirs — which is the cost of leaving a standing red in place.
  Re-deriving the anchors is a self-contained chore and belongs in its own commit.
- **A line inserted into a widely-cited source silently invalidates every doc anchor below it.**
  L5 added `VmColumn::as_mut_slice` at ~line 355 and shifted `vm_column.rs:437-449` to `462-474` in
  two corpus files. The anchor test does not cover `docs/diagnostics/**`, so nothing would have
  said so. Grep `<file>.rs:[0-9]` across `docs/` after any insertion into a kernel primitive.
- **A gate can be RED for five commits because nobody runs it, and no gate can close that.**
  `G2a`'s file census (`tests/gpu_blocking_reader_census.rs`) went red the instant rung 5a landed
  `present/gpu_zone.rs`, whose module doc names `vkGetQueryPoolResults` while explaining what the
  BLOCK cost. It stayed red across `ee9196b6`, `cb54752d`, `8ca4e05b`, `cf8ffd20`, `7ae9162a` —
  three of which reported "workspace green". Found at 5c only because a full `--workspace` run
  finally happened. The mechanism worked perfectly; the *asking* was the gap. Practical rule:
  **a rung that adds a file must run the census gates, not only its own.**
- **The disk fills to zero and it looks like a mingw linker bug.** Second occurrence (see the
  2026-07-23 audit note). `target/` reached 72 GB with 8 KB free on `D:`, and the symptom was a wall
  of `linking with x86_64-w64-mingw32-gcc failed` across a dozen unrelated targets. Cure:
  `rm -rf target/debug/incremental` (12 GB here). Check `df -h /d` BEFORE reading a linker error.
- **G10's A/B runs as two processes and the corpus says one; the reason is in the docs but it is a
  deviation an auditor should see.** Leg A's `read_vb_bench_ns` waits with `VK_QUERY_RESULT_WAIT_BIT`,
  so a single process alternating legs would reach it on a frame leg B recorded — the hang class
  P4-1 removed. If a future rung wants the corpus's literal shape, it must first make leg A's
  readback non-blocking, which is rung 7's deletion anyway.
- **RESOLVED (owner: "whichever is more performant") — profiling rung 6's G10 fork went to the host
  arming knob.** `BOYKO_GBUF_BENCH` costs one boot-time `Option` that is `None` in every shipped run
  and expires with rung 7; the alternative would have forced `GpuZoneRecorder::open_frame` and
  `retire` to `&self`, deleting clause (c) of `FrameSlot`'s `Sync` argument permanently and pushing
  `set_mark` toward a locked RMW in a hot recorder. **The leg is armed and never read** — the witness
  clause needs no timings, and `read_query_pool_ns` would hang on a frame that skipped a pass.
  ⚠️ **Rung 8's reader must consult the witness masks before it waits on anything**: three of the
  four software-ray passes are bracketed inside their own `if let` arms, and neither old gbuffer
  collector has a totality epilogue.
- **RESOLVED (owner: "decide yourself what is optimal") — profiling rung 7's artifact format is
  decided and its writer/reader/gate have shipped.** See `profiling/artifact.rs`'s module doc for
  each decision and what it costs to get wrong. ⚠️ **One of the six answers turned out to be wrong in
  its REASONING and was corrected by insisting on a RED**: the fear that a wider file collapses
  `vg_occ_split_timing.rs:916`'s GCD is false — that consumer's own `(v * 10.0).round()` absorbs the
  extra digits, measured across seven values. One decimal is still right, for smaller reasons
  (direct comparability with the printed lines, which is what makes the next step's A/B possible).
  The six values, as decided:
  1. **Numeric precision in the artifact.** `vg_occ_split_timing.rs:916` reconstructs the GPU tick
     lattice by GCD over **tenths**, because that is the precision the summary prints. Full-precision
     `f64` collapses the GCD and sub-floors every band; the file's own doc measures the error at
     **32×** and says such a choice *"would satisfy every assertion here while under-stating the
     instrument's resolution by the whole lattice factor"*. **A silent false-win, not a red test.**
  2. **File path, per-run uniqueness, truncation.** `append_artifact(p, path)` takes a caller path
     and nothing else — no default, no rotation, no env knob. `vg_decidability_floor.rs` spawns 42
     sequential children; a fixed path is a stale-read generator.
  3. **One file = one sitting, one process, or many appended runs.** `G24`'s reverse RED is *defined*
     on staleness and cannot be written until this is chosen.
  4. **Who aggregates the 21 per-session artifacts into the PROFILING-FLOOR document** (a rung-7b
     deliverable that does not exist yet — it is named here, not linked, so the anchors gate does not
     read a plan as a path) — rung 7b depends on it and no line assigns it.
  5. **Whether `WorkloadTag` is an artifact field.** `resolve` checks it, 7b publishes it into
     markdown, nothing says the session file carries it.
  6. **What the artifact records when the device declines timestamps** — today an `eprintln!` that
     three consumers key their third outcome on.
  ⚠️ **`G24`'s reverse RED named two fields that cannot do the job, and one of them does not exist.**
  `crates/boyko_diag/` has **no `build.rs`** and `BUILD_HASH` appears nowhere in the workspace — a
  planned rung-0 artifact that never landed. `SessionId` exists but is minted INSIDE the child, so a
  parent cannot predict it. The discriminator is therefore a **parent-supplied run token**, the only
  field that can catch staleness within one run.
  Rung 7's remaining halves, in order: the reducer that fills the artifact, the producer wiring
  (verified by A/B against the still-printing channel while BOTH are live), 7b's floor
  re-measurement, then the deletions — **713** lines from `gpu_timing.rs`, **1381** from `runner.rs`
  (31 % of the file), **465** from `gpu_scene/mod.rs`, plus five consumer migrations.
- **RESOLVED (owner: the STRICT option of three) — the workload tag is two halves, and an
  undeclared one is not a floor.** Measured while opening rung 7's consumer migrations. The tag is
  `format!("{path:?}_{legs:?}")` over `ResolvedRenderPath` — `deferred_both`, `visibilitybuffer_mesh`
  and so on. `vg_decidability_floor.rs` runs its NULL experiment twice per repetition, once with
  `BOYKO_VB_FROXEL_FORCE_OFF` and once without, and **neither `path` nor `legs` changes between
  them**: `froxel_light_cull` is a separate field of the same struct and the tag does not read it.
  `BOYKO_VB_BENCH_LIGHTS` (`N_ps`) is invisible to the engine entirely.
  The migration itself is not blocked — the floor test writes each leg to its own file path and
  never reads the tag to separate them. What the hole costs is **downstream**: `resolve` refuses a
  `Floor` whose `workload` differs, and that refusal is the ONLY mechanism keeping a floor measured
  on one configuration from bounding a delta measured on another. With this tag, a flat-leg floor
  silently bounds a froxel-leg claim — which `vg_decidability_floor.rs`'s own "What this does NOT
  decide" forbids in words (*"It is one CONFIGURATION"*, *"a rung that measures at a different scale
  must re-measure its own floor rather than cite this one"*).
  ⚠️ **And a correction to this entry's own first sentence about the mechanism.** It said `resolve`
  refuses a mismatched `Floor` as if that were shipped code. MEASURED: `Floor`, `resolve`,
  `FloorWorkloadMismatch` and `NotResolved` exist **only in the corpus documents** — `rg` over
  `crates/` returns nothing. They are rung 8's content and are unwritten. So nothing was silently
  wrong; the tag is the INPUT to a comparator that does not exist yet, which is why what it names
  had to be settled before 7b publishes a floor that later rungs cite.
  **Shipped as decided:**
  * **Derived** — [`config_tag`] over the WHOLE `ResolvedRenderPath` (readable `path_legs` prefix +
    8 hex of FNV-1a over every field), not a hand-picked subset: the bug was not that the wrong
    field was chosen, it was that fields were chosen. A field added to that struct invalidates prior
    floors deliberately — floors on this box drift faster than that anyway.
  * **Declared** — `content_tag`, from `BOYKO_PROFILE_WORKLOAD`, set by the measuring test in its
    own spawner code where the value already lives, not in an operator's shell.
  * **The refusal is enforced NOW, not promised to rung 8**: `Artifact::floor_source` returns
    `UndeclaredContent` on an empty or blank content tag, because a clause whose subject does not
    exist yet is a promise rather than a gate. A missing KEY is a malformed header, kept distinct
    from a present-but-empty one — the whole of the refusal is that distinction.
  RED run: revert the derivation to `path × legs` ⇒ *"the flat and froxel legs produced the SAME
  workload tag ... flat: deferred_both, froxel: deferred_both"*. Measured on a live run:
  `workload_tag = "deferred_both#99f4482e"`.
  ⚠️ **Narrowed 2026-08-10, and the obvious cause is ruled OUT.** The module already serialises
  itself: `armed()` takes `test_serial()` and hands the `MutexGuard` back to the caller, so every
  test that arms a store holds the module's one lock, and a second explicit site covers the
  plugin test. So the contention is **not** profiling test against profiling test. What remains is
  state global to the PROCESS rather than to the module — `boyko_diag::profiling_abi`'s zone
  `REGISTRY`/`NEXT_SLOT` (slots are minted lazily by `declare_zone!` and never returned) and the
  store's `bind_world` — which any of the other ~880 tests in the same binary can move. Both flakes
  pass 915/915 in isolation and in repeated full-lib runs; they fail only under the full workspace
  sweep. **Next step is to identify which non-profiling test touches the registry**, not to widen
  the module's lock, which is already as wide as the module.
- **`boyko-ecs --lib`'s profiling tests are ORDER-DEPENDENT FLAKES — now TWO of them.**
  `every_dispatching_round_records_one_span_and_one_width` joined
  `one_zone_taking_a_hundred_thousand_samples_keeps_count_exact` on 2026-08-10: observed failing
  once in a full `--workspace --all-targets` run, then passing in isolation and in two consecutive
  full lib runs (915/915 each). **A second name in the same class raises the priority**: this is
  no longer one unlucky test but a property of the module, and a real regression here would be
  indistinguishable from the flake. Original entry follows. Observed failing once in a full `--workspace --all-targets` run, then
  passing in isolation and in two consecutive full lib runs (915/915). Same class as the
  `MAX_QUERY_TYPES` note above: process-global profiling state (lanes, zone slots) against 915
  tests in parallel. Re-run before bisecting.
- **⚠️ A KNOWN-RED TARGET IS A SHADOW — the workspace gate must run `--no-fail-fast`.**
  MEASURED 2026-08-10. `cargo test --workspace --all-targets` **stops at the first failing target**,
  so `internal_docs_anchors` — red for everyone, long known — had been hiding every target ordered
  behind it. Every "workspace suite green except the pre-existing anchors failure" reported during
  the diagnostics campaign was a claim about **what cargo reached**, not about the workspace. Run
  with the flag, there were **three** red targets, both of the others older than the session that
  found them:
  * `boyko_rhi_vulkan --test compile_fail_frame_write_token` — `26b385eb` (2026-08-06) added one
    line to `token_use_after_submit_rejected.rs` and never re-blessed its `.stderr`; the entire diff
    was `41:5` vs `42:5`. **Red for the 87 commits since.** FIXED by hand-editing the two line
    numbers rather than `TRYBUILD=overwrite`, because that fixture's own comment warns that blessing
    can turn a right-for-the-wrong-reason red into a wrong green — the error kind, the moved value
    and the move site were all verified unchanged first.
  * `boyko_rhi_vulkan --test cluster_bound_arraylength` — VG R3's `vb_batch_cull` (+ its `-D DEBUG`
    sibling) gained an `OpArrayLength` and `BOUND_BY_ARRAYLENGTH` was not updated in that commit,
    which is what the gate's own message instructs. FIXED, and the pin **widened to carry each
    entry's SUBJECT**: the set no longer has one, since those two bound `VbLateCount`'s reserved
    tail slot rather than a froxel light walk, and the old failure text would have given a reader
    advice about a `use_clusters` guard those shaders do not have.
  * `boyko-ui --test p3_watch_zero_alloc` — `watch_nochange_path_is_tiny_and_far_below_reload`
    reported `nochange 1, reload 1`: the "reconciling" tick was reporting the no-change path's cost
    because it WAS the no-change path. The watcher's signature is `(mtime, size)` and the fixture
    rewrote `Px(77)` over `Px(40)` — **the same byte length** — so detection rested entirely on the
    filesystem clock, and this box's mtime granularity swallows the fixture's 3 ms sleep. `4956420c`
    had already diagnosed this class once ("the hot-reload flake was the FILESYSTEM CLOCK, not
    shared state") and answered it with longer sleeps, which buys margin against a granularity
    nobody measured. FIXED by making the rewrite change the SIZE, which removes the dependence
    instead of widening it: `nochange=1 reload=53` after.
  ⚠️ **And my own reporting of this finding was truncated the first time.** The first sweep's
  failure list was piped through `head -10` and I read the truncation as the total — "three red
  targets" when the honest count was six. The same mistake one level up from the one being
  reported. The full picture, measured:
  * **Genuinely red, now FIXED**: `compile_fail_frame_write_token`, `cluster_bound_arraylength`,
    `p3_watch_zero_alloc`.
  * **Genuinely red, NOT fixed**: `internal_docs_anchors` — 25 stale anchors across three internal
    docs plus an over-waiver count above its cap. Pre-existing, unrelated to any campaign here, and
    large enough to be its own unit of work.
  * **Not red at all, but FAILING UNDER THE FULL PARALLEL SWEEP**: `boyko-log --lib` (84/85 in the
    sweep, **85/85** in isolation), `boyko_rhi_vulkan --test sdf_gbuffer_hybrid` (**43/43** in
    isolation, 54 s), and the two `boyko-ecs --lib` profiling tests recorded above. Four targets in
    one class. **The workspace suite is therefore not deterministic**, and a real regression in any
    of them would be indistinguishable from the noise. Every one touches process-global or
    device-global state — profiling lanes and zone slots, logging sinks, the GPU device — which is
    what a `--test-threads` bound or a per-target serial marker would address.
  `CLAUDE.md`'s build-command block now carries the flag and the reason.
- **`.claude/settings.local.json` is dirty** from earlier sessions and is deliberately never staged.

## Rung 9 — `resolve`'s session check refuses every real leg pair, and reports it as `EpochBreak`

**Found while wiring rung 9's correlation, which gives `clock_epoch` a real meaning in this tree for
the first time. Not introduced by rung 9 — surfaced by it.**

`crates/boyko_app/src/profiling/contrast.rs`'s `LegSummary` carried a field named `clock_epoch`
holding `(header.session_lo, header.session_hi)` — a `SessionId`, not
`boyko_diag::clock::clock_epoch()`. Rung 9 renamed it to `session`, because the artifact now carries
a real `cpu_gpu_epoch` and two different things would otherwise have shared one name in one module.
The rename is done. **Two things about the check it feeds are not, and both are the owner's call:**

1. **The refusal reports `NotResolvedReason::EpochBreak` for a SESSION difference.** That is
   defensible in spirit — `clock_epoch()` is a per-process counter, so "both at epoch 0" from two
   processes compares two numbers that mean nothing to each other — but the reason word names
   something the check does not test. `G13`'s sibling clause pins the word `EpochBreak`, so
   renaming it is corpus surface, not a local edit.

2. ⚠️ **On real inputs the check refuses unconditionally.** MEASURED: `resolve` and
   `LegSummary::from_artifact` have **no caller outside `contrast.rs`'s own tests** — rung 8 shipped
   the comparator to *license* later verdicts, not to serve one — and every leg pair a real harness
   would build today comes from two spawned child processes (`vg_decidability_floor.rs`'s protocol
   is seven processes per condition). Two processes have two session ids by construction, so the
   first production consumer of `resolve` will find that it returns `NotResolved { EpochBreak }` for
   every pair it is ever given. The existing tests do not catch this because both legs are hand-set
   to the same value.

**What a fix would have to decide** (not decided here): whether cross-process legs are comparable at
all. If they are — and the whole floor protocol assumes so, since it pools seven sessions — then the
check is wrong as written and the real guard is something else (same `workload_tag`, same box, same
`clock_epoch` *within* each artifact). If they are not, then the floor protocol and this check
contradict each other and one of them is the error.

## Rung 9 — the per-frame ring is now TWO deferrals pointing at one mechanism

The correlation is published once per window with a measured 173 ppm drift across it
(`Correlated::deviation_at_ns` interpolates). Rung 8's per-zone `vkCmd*` counters reach the printed
census but not the artifact, for a structural reason of the same shape (the witness resets each
frame while `retire` yields a frame recorded ~4 frames earlier). Both want a per-frame channel that
does not reduce to medians — which is also what the owner's original ask needs ("break a frame down
by system and pass, catch per-frame spikes"). Recorded so it is built once rather than twice.

## The `hwrt` feature leg does not COMPILE — pre-existing, found by rung 9's clippy sweep

**MEASURED, and proved not to be this campaign's:** `cargo clippy -p boyko-app --lib --features
boyko_rhi_vulkan/hwrt` fails with

```
error[E0063]: missing fields `atrous_layout_denoise_hwrt`, `motion_cam_ubo_ring`, `mv_bind_group`
and 21 other fields in initializer of `boyko_rhi_vulkan::present::GBufferScene<'_>`
    --> crates\boyko_app\src\gpu_scene\mod.rs:6298:25
```

Twenty-four missing fields. Confirmed pre-existing by `git stash`ing every rung-9 source change and
re-running: **identical error on the untouched tree at `71085737`.**

**Why nothing caught it.** `hwrt` is `default = false`, and every gate in this tree —
`cargo check --workspace --all-targets`, the clippy gate, the test sweep — runs the DEFAULT feature
set. The leg is never built, so it can rot without turning anything red. This is the same shape as
the two findings already recorded above (a target ordered behind a known-red one; a virtual manifest
type-checking a subset): **a configuration nothing builds is a configuration nothing gates.** Rung 8
recorded that lesson for `profiling-alloc` and fixed it by making both configurations buildable;
`hwrt` is the larger instance of it and has been un-built for long enough that the drift is
twenty-four fields wide.

**Not fixed here.** It is unrelated to rung 9, it needs `GBufferScene`'s twenty-four `hwrt` fields
understood one at a time, and guessing at them would be worse than the current honest break. **Owner
call:** repair the leg and add it to the gate set, or state that `hwrt` is dormant and stop
implying otherwise.

## Rung 10 — two corpus gates could not run as written, for two different reasons — **RESOLVED 2026-08-10, see the top of this file**

Both are recorded in `docs/diagnostics/profiling/05-LADDER-GATES.md`'s rung-10 record with their
arithmetic. Surfaced here because each is a **decision the owner may want to take differently**, not
merely a note.

**1. `G17`'s absolute nanosecond thresholds were replaced by an A/B ratio.** The row asks for five ns
budgets in one sitting, the tightest pair being "static-armed ≤ 12 ns" against "dyn-armed ≤ 14 ns" —
two nanoseconds of headroom. This campaign measured its own artifact-channel floor at **6.5 %**,
with repetitions spanning 4.7–14.3 %, on GPU passes costing microseconds. A 2 ns budget is inside
that noise by an order of magnitude. What ships instead implements **both** variants — the shipped
gate and the `REGISTRY[id]`-dereferencing one the row names as its RED — and interleaves them in one
process, asserting the shipped one is not slower. MEASURED (debug): 10.89 vs 22.01 ns/iter, 2.02×.
**If the owner wants the absolute thresholds, they need a release-profile bench harness and a
recorded per-box floor for the ns scale** — neither exists, and inventing a threshold without one is
how a gate comes to fail for a background process.

**2. `G22b` clause 2 names a symbol that does not exist and a failure that cannot be written.** The
clause says *"a `#[test]` declaring a `.bss` array sized from a `ProfilerConfig` value must fail
`assert_bss_eligible` at compile time; remove the const-assert ⇒ it compiles ⇒ red."* MEASURED:
`assert_bss_eligible` has **zero hits** in `crates/` (the symbol is `assert_zero_init_eligible`), and
the failure it describes is impossible here — `SyncCells<T, N>` takes its extent as a **const
generic**, so a run-time `ProfilerConfig` value cannot size one whatever any assertion says. There
is no const-assert to remove because nothing needs one. A `trybuild` case gating the property rung 10
DID introduce ships instead (deleting `DYN_DESCS`'s `MaybeUninit` wrapper must be `E0277`). **Owner
call: rewrite the clause against the real symbol, or delete it as satisfied by the type system.**

**And `G22b` clause 1 remains BLOCKED on the same missing tool as `G22a`** — no
`llvm-readobj`/`objdump`/`nm` under the active `stable-x86_64-pc-windows-gnu` toolchain. Rung 10
added two more symbols (`DYN_DESCS`, `DYN_NAMES`) to the set that probe must cover when
`rustup component add llvm-tools` lands, so the D0 line item now unblocks four names rather than two.

## Rung 10 — `G23b`'s literal RED is not producible, and no setting of the constant makes it so

The row's RED: raise `MAX_USER_BUDGET` in the **shipping** profile from 512 to 3072 ⇒ +20 KiB
`REGISTRY` and +120 KiB `DYN_DESCS` ⇒ 1 348.2 KiB crosses the 1 280 KiB budget.

There is no shipping profile: the `BOYKO_PROFILE` axis is rung 14, and MEASURED, **no `build.rs`
exists anywhere in this workspace**. So the residency gate runs at the dev row against a **16 MiB**
budget. And the constant has a hard ceiling that is not a policy choice: zone ids are `u16`, so
`ENGINE_ZONE_SLOTS + MAX_USER_BUDGET` must stay under `u16::MAX`, capping `MAX_USER_BUDGET` at
~61 439. At 8 B/id in `REGISTRY` plus 24 B/id in `DYN_DESCS` that is **~1.9 MiB of growth against a
16 MiB budget** — it does not cross, and nothing a caller can set makes it cross.

An upper bound nothing can push past is a gate that cannot fail. What ships is a **composition
identity** — domain 3 must equal the four `.bss` terms, computed independently — whose RED *is*
producible: dropping `dyn_descs_bytes()` from the sum gives `left: 143360, right: 217088`. That is
the claim the row's own title makes (*"this row is what puts their bytes inside the budget sum"*),
and it is gated. **The budget clause itself stays honest but toothless until rung 14 gives it a
shipping row to be tight against.**

---

## Rung 13 — a second CRC-32 table, and the graph edge that would remove it

**Recorded rather than decided, because both options cost something real and neither is urgent.**

`boyko_diag::telemetry` computes the block CRC with its own 256-entry const table (1 KiB of
`.rodata`). `boyko_image::png` already has one — same polynomial, IEEE 802.3, private, and shaped
for PNG chunks (`crc32_chunk` takes the chunk kind and prepends it).

The duplication cannot be removed by using `boyko_image`'s: `boyko_diag` must keep an **empty
`[dependencies]`**, which is the property that makes it the bottom of the graph. The only direction
that works is the other one — hoist a shared CRC-32 *into* `boyko_diag` and have `boyko_image`
depend on it. That is refused here for now, on the crate's own rule: a checksum is not a diagnostics
primitive, and §4's growth checklist admits a module only when **both** subsystems write it and a
disagreement between two copies would be observable in a joined artifact. Two CRCs over two
different byte streams cannot disagree with each other about anything.

**Cost of leaving it:** 1 KiB of `.rodata` and six lines, twice.
**Cost of hoisting it:** the bottom crate gains a general-purpose utility, `boyko_image` gains an
edge into the diagnostics substrate, and the growth rule loses the property that makes it hard to
satisfy.

Owner's call if the second one is ever preferred. Nothing is blocked either way.

## Rung 13 — `G26`'s budget is a RELEASE claim, and the gate says so instead of pretending

`G26`'s figures (`__telemetry_reduce` p95 ≤ 150 µs, `__telemetry_write` ≤ 200 µs, sum ≤ 350 µs) are
asserted only under `not(debug_assertions)`. MEASURED, this box, 64 quantile zones, p95 over 32
runs:

| | debug | release |
|---|---|---|
| `__telemetry_reduce` | 5 820.8 µs | **128.0 µs** |
| `__telemetry_write` | 163.1 µs | **11.2 µs** |
| **sum** | 5 983.9 µs | **139.2 µs** |

A debug build is **43× over** the total budget. Asserting the budget there would red on every
developer's machine and prove nothing about the shipped one, so what the gate asserts in *every*
profile is the property the budget encodes and a profile cannot change: the reduce dominates, and it
is the term that scales with the quantile count.

~~**The open half:** the release leg is not in CI today. `scripts/` has no release test step, and the
five-profile CI matrix is rung 14's content. Until rung 14 lands, **the budget clause runs only when
somebody runs `cargo test --release`**, and this note is the record that it is not automatic. It is
the same shape as rung 10's `G17` and is expected to be resolved by the same rung.~~

**CORRECTED at rung 14, 2026-08-11: THAT PARAGRAPH WAS FALSE WHEN IT WAS WRITTEN.**
`.github/workflows/ci.yml`'s `test` job is a `matrix: profile: [debug, release]`, and its release
arm has run `cargo test --workspace --all-targets --release` since long before rung 13. A second
job, `force-alloc-panic`, runs the release suite again under `--cfg force_alloc_panic`. Neither
excludes `boyko_app`, so `G26`'s budget clause has been running in CI on every push the whole time.

The defect is not the conclusion, it is **where I looked**: I checked `scripts/` for a release step,
found none, and reasoned from that to a claim about CI — without opening the CI file. That is the
root cause this corpus has already written down in as many words: *verification is an ACTION, not an
understanding*, and errors land exactly where checking something would have meant doing something.
`scripts/` and `.github/workflows/` are two different places and only one of them was read.

The note is struck through rather than deleted because the false claim is the useful half. Rung 14's
`profile-legs` matrix is still net new and still worth having; what it does **not** do is close a gap
that was never open.

## Rung 13 — `W9214` has an emitter and a doc page, but no test observes it

`W9214` (telemetry path unwritable) is `Live` in the registry, is raised by `TelemetryStream::open`
and has a `docs/diagnostics/W9214.md` page. Checks 2, 3a and 3b all pass. What does **not** exist is
a test that observes it being emitted, which is the obligation every `Live` row owes.

The reason is that producing it needs an unwritable path, and the ways to get one are all
platform-specific and flaky in CI: a directory that does not exist works on both platforms but is
the least interesting case; a read-only file needs `chmod`/`icacls`; an open-without-sharing needs a
second handle and is Windows-only.

`W9215` and `W9218` **are** observed (`crates/boyko_app/tests/profiling_telemetry_stream.rs`), so
this is one row rather than three. Recorded rather than papered over with a
directory-does-not-exist test that would pass on a typo.

## Rung 14 — `profiling-analysis` is now OPT-IN, and that changes what a plain `cargo build` gives you

**This one is a behaviour change and the owner should know about it before it surprises him.**

`boyko_ecs` used to declare `default = ["profiling-analysis"]`, so every build carried the interval
ring and `ConcurrencyReport` — the "did this schedule actually run in parallel?" answer. It is now
`default = []`, and that answer requires `--features boyko-ecs/profiling-analysis`.

**Why it had to move**, measured rather than argued:

- An environment variable cannot set a cargo feature. Cargo resolves features before any build
  script runs, and `cargo::rustc-cfg` reaches only the crate that emitted it. So `BOYKO_PROFILE`
  could never have switched it, whatever the corpus's table says.
- While it was default-on, **no command line could turn it off**. `cargo tree --workspace -e
  features --no-default-features` still reported it enabled: **nine** sibling manifests depend on
  `boyko-ecs` and not one says `default-features = false`, so unification restored it. Moving the
  request onto a dependency edge would not have helped either — an explicit `features = [...]`
  survives `--no-default-features` by design.
- So the axis's `shipping` row would have been a claim nothing could honour, and `G14(c)` would have
  been a gate with no reachable state.

**What replaces it:** the axis emits `ANALYSIS_ADMITTED`, and `boyko_ecs` refuses at compile time to
be built with the feature on under a profile that does not admit it. The refusal is one-way on
purpose — analysis missing from a `dev` build is a developer who passed fewer flags than they meant
to; analysis present in a `shipping` build is the profile being a lie.

**A coverage consequence, found by asking rather than by a gate.** Six tests in `boyko_ecs`'s
profiling suite are `#[cfg(feature = "profiling-analysis")]`. Opt-in means a bare
`cargo test --workspace --all-targets` no longer compiles or runs them — and a sweep that silently
stops running six tests looks exactly like a sweep that passes. Every CI leg that used to get the
feature from the default list now names it explicitly (`check`, both `test` arms, `clippy`,
`force-alloc-panic`), and the feature-OFF side is covered by the four `profile-legs` builds that
refuse it outright. **A local sweep must pass `--features boyko-ecs/profiling-analysis` too, or those
six do not run.**

**The owner's call, if he wants one:** whether a plain local `cargo build` should carry it. The only
way to have both is to accept that no flag can remove it, which is the state we just left.

## Rung 14 — the symbol census needs `lto = "fat"`, and that raises a separate question about the shipped profile

`G14(a)` is only decidable under LTO. MEASURED on this box, `deep_zone` (one `Deep` zone site,
`boyko_diag` its only dependency):

| link configuration | `dev` | `shipping` | can the gate fail? |
|---|---|---|---|
| default release | `mint_cold` = 1 | 1 | **no** |
| `-C link-arg=-Wl,--gc-sections` | 1 | 1 | **no** — no effect at all |
| `lto = "fat"`, `codegen-units = 1` | 1 | **0** | yes |

The default-release image contains `drop_glue::<boyko_diag::telemetry::Block>` in a binary whose
source never mentions telemetry: the whole rlib rides in and nothing collects it, so a whole-image
census answers "was this codegen'd on the way here?" rather than "can this program reach it?".

The gate passes LTO through `--config` for its own two builds only, so nothing else in the
repository changed. **The open question is a different one:** `[profile.release]` here sets
`codegen-units = 1` for benches and no LTO anywhere. A shipped title almost certainly wants
`lto = "fat"` — it is the difference between `deep_zone` at 1433 symbols and at 4647. That is a
build-configuration decision with compile-time cost, and it belongs to the owner rather than to a
diagnostics rung.

⚠ **ANSWERED since this was written; corrected 2026-09-10.** "No LTO anywhere" is stale:
`Cargo.toml:111-112` now sets `[profile.release] lto = "fat"` for the whole workspace (with
`codegen-units` deliberately left alone there, and `[profile.bench]` pinning `codegen-units = 1`
explicitly so it does not inherit the pair). The owner took the decision; the gate's own
`--config profile.release.lto="fat"` is now redundant and is kept only so the gate does not depend
on a profile decision that is the owner's to reverse. One consequence is recorded at
`crates/profile_fixture/tests/profile_axis_census.rs`: the census's documented "drop the flag ⇒
every cell reads 1" RED no longer reproduces, and the mutation is now
`--config profile.release.lto=false`.

## Rung 14 — `BOYKO_PROFILE=off` does not turn the profiler off, and the thing that would does not exist

`SEAM.md` §S9's table gives the `off` row the tier-column entry *"feature `profiling` off"*.
MEASURED at this rung: **there is no `profiling` cargo feature anywhere in this workspace.**
`boyko_diag` declares `section-gate`; `boyko_ecs` declares `profiling-analysis`, `big_query_table`
and `bench-alloc`; no crate gates `zone!` or `declare_zone!` on a feature at all. And `ZoneTier`'s
three values are `Always`, `Dev` and `Deep` — there is no position below `Always`, so the lowest
compile ceiling the profiler has still admits every `Always` site.

`off` therefore ships as a **logging** off switch: `LOG_CEILING = 0`, `LANE_ARRAY_LEN` becomes
zero-length, which is `G2`'s subject and works. The profiler stays at its floor.

Two ways to close it, both out of this rung's scope: land the FEATURE axis (`G1`) so a `profiling`
feature exists and `#[cfg]` can delete the macro definitions before name resolution; or accept that
"off" means the runtime axis (`ARM_MASK`, `GJ1`) and rename the row. Nothing is blocked either way —
the row is honest as built, and it is written down here because "off" is a word a reader will trust.

## Rung 14 — J1's logging half is owed, and it is owed to rungs that have not landed

`J1` is one rung by construction (S9: one compile axis cannot be split across two rungs), and the
axis is now whole. `L17`'s **other** content is not:

- `LogRuntimePreset` — the five-preset runtime axis.
- The three header facts: `build_profile`, `runtime_preset` and `ceiling` printed as three
  independent values, plus a fixture proving the first two can differ in one binary.
- `G16(d)`, which is the gate over those three fields.
- A **dynamic** logging site in the census fixture. `dyn_debug!` is `L10`'s and does not exist, so
  `G16(a)/(b)` covers the static path only.

All four need a sink header to print into. MEASURED: `boyko_log` has `census`, `codes`,
`drain_owner`, `lane`, `level`, `lifecycle`, `macros`, `rate`, `record`, `site`, `sync_out`,
`target` and `sink/{ecs,file,mod}` — and **no** `sample.rs`, `sink/binary.rs`, `sink/request.rs`,
`sink/crash.rs` or `bin/logdec.rs`. The logging ladder stands at roughly L5 of 17.

This is not a defect in the axis and not a shortcut taken: the axis is the indivisible part and it
landed indivisibly. It is a scheduling fact, recorded so nobody reads "J1 shipped" as "the logging
plan reached L17".

## Rung 16 (J2) — REFUSED ON THE MEASUREMENT: the "both-present" configuration does not exist

J2 is the joint baseline sitting: re-take `zone_cost`, `fold_cost`, `P1` and `P2` **with the profiler
and the logger both present**, in one sitting, and run `GJ1` (the measured off-cost) there. Attempted
after rung 15 and **refused**, because the configuration it is supposed to baseline is not one this
workspace can currently be in. MEASURED:

| | measurement |
|---|---|
| `boyko_log::{error,warn,info,debug,trace}!` across every crate's `src/` | **2 hits, neither an emission site** — a *comment* in `boyko_log/src/lib.rs:86` and rung 15's own `profile_fixture_log` |
| callers of `boyko_log::enable` / `boot` | **none** — no sink thread, no consumer, no panic hook |
| manifests depending on `boyko-log` | **two** (`boyko_ecs`, `profile_fixture_log`); absent from `boyko_app`, `boyko_render`, everything that runs a frame |
| non-test callers of `Profiler::arm` | **none** — rung 11 measured this and it is unchanged |

So `GJ1`'s leg **A** — *"profiler armed, logger enabled, at the shipping ceiling"* — cannot be built,
and legs B and C are defined relative to it.

**Why this is a refusal and not a deferral.** The tempting move is to take the sitting anyway and
stamp the files `both-present`. That is strictly worse than having no baseline: every later
regression gate compares against these files, `config_tag` is what tells a reader the comparison is
legitimate, and a tag that says `both-present` on a run with neither present makes every one of those
gates confidently wrong. The corpus's own rule — *"whichever subsystem landed second must not be
measured against a baseline taken without it"* — is exactly the rule being obeyed here.

**Consequence, stated so it is not read as progress:** the `UNPROVEN` state REMAINS IN FORCE. The
+25 % gate, the revert clauses and `GJ1` still record `UNPROVEN` and still may not fail a rung.

**Preconditions, so the rung can be re-entered rather than re-argued:**

1. Logging **L3** — the sink thread, `enable`, the drain. Without a consumer there is nothing to
   measure the cost of.
2. Logging **L6–L8** — the migration that gives the engine emission sites at all. Today it has none,
   so "logger on" and "logger off" are the same binary doing the same work.
3. ~~A **non-test arm path** for the profiler, so "profiler armed" is a state a shipped frame
   reaches.~~ **DONE, and the note above UNDERSTATED the problem.** It said `Profiler::arm` had no
   non-test caller. Measured immediately afterwards: **`ProfilerPlugin` was added nowhere outside
   tests either** — so the store was not merely unarmed, it was never *inserted*, and fifteen rungs
   of profiler were unreachable from any host. `App::update_with_delta` had been calling
   `fold_frame` all along; it found no `Profiler` and returned. `EnginePlugins` now adds the plugin
   unconditionally — safe by the store's own design, `Profiler::new` *"reserves nothing, commits
   nothing, calibrates nothing"* — and `BOYKO_PROFILE_ON` arms it, which is `SEAM.md`'s route (a).
   Gated by `crates/boyko_app/tests/profiling_host_reachable.rs` (installed + disarmed) and
   `profiling_host_arm_flag.rs` (the flag arms it), both REDs shown.

   **The shape is worth keeping separately from the fix.** Every one of those fifteen rungs was
   green, and none of them could see this: each gate builds its own world and inserts its own store,
   so "does a HOST have one?" was a question no test in the campaign was asking. A subsystem can be
   fully gated and entirely unreachable at the same time.

   **And a second constraint fell out of writing the gate:** `EnginePlugins` **cannot be built twice
   in one process** — the second build panics in
   `register_component_hooks::<boyko_render::light::DirectionalLight>`, because component hooks are
   process-global and the derive's installation is not idempotent. That is why the two legs are two
   test *binaries*. It is pre-existing, it belongs to the render plugins rather than the profiler,
   and it is invisible until something builds the host twice.

**And one defect to fix before the rung, not during it: `config_tag` is already taken.**
`boyko_app::profiling::artifact::config_tag` exists and returns a `String` FNV-1a hash of
`boyko_render::ResolvedRenderPath`'s `Debug` — it identifies the **render path** (Deferred/Forward/VB
× Both/Mesh/Sdf), and `ArtifactHeader::workload_tag` is built from it. S10 asks for a field of the
same name meaning `{profiler, logger}`, in the same crate. Landing it under that name would put two
facts under one identifier, and the failure mode is specific: a reader compares a VB baseline against
a Deferred one, the tag matches, and the difference is reported as a regression. The J2 field needs a
different name (`diag_tag`, say) or the render one does.

---

## L8b — VALUES: L6/L7/L8a silenced 31 diagnostics that used to print unconditionally, and no document says so

**This is not a bug report.** The behaviour is specified and it is gated. It is an owner call about
what a default run of this engine tells its operator, and it is raised here because the migration
rungs took the decision as a side effect of a cost argument rather than as a decision.

**Measured, in this order:**

1. `boyko_app::plugins::boot_and_enable_logging_from_env` calls `boot()` unconditionally and then
   **returns before `enable()`** when `BOYKO_LOG` is unset. `CONTROL` stays `.bss`-zero, so every
   target's runtime ceiling is `Off`.
2. So a migrated `warn!`/`error!` in a default run produces **nothing** — not a dropped record, not
   a counted loss. The macro's third gate is false and the site is one predicted branch.
3. `git show` on the three migration commits: **31 `println!`/`eprintln!` lines were removed from
   production sources** — 3 at L6 (`49cf2230`), 12 at L7b (`b30fa810`), 16 at L8a (`1a76e4a9`).
   Spot-checked against the parent commit, `boyko_render`'s `W2201` site was an **unconditional**
   `eprintln!` behind a one-shot latch, not an env-gated or `debug_assertions`-gated one.
4. The silence is **deliberate and pinned**: `logging/sink-lifecycle` Decision 25 states *"a
   flag-off run of any other preset configures nothing either, because `enable()` never ran and no
   sink slot was ever opened"*, and `crates/boyko_app/tests/log_host_reachable.rs` asserts
   `flush() == NoConsumer` after a full `EnginePlugins` build, calling it *"the half that makes the
   cost claim true rather than merely stated"*.

**What no document in the corpus says** is what (4) does to (3). The plan gated the *cost* — one
sink thread, a 20 ms clock calibration in `enable()`, a panic hook — and in doing so gated the
*diagnostics*, and the migration rungs then converted 31 unconditional prints into records behind
that gate without the trade being written down anywhere.

**The question, stated as a fork:**

* **(A) Diagnostics stay opt-in** (today's behaviour). A shipped run is silent until an operator
  sets `BOYKO_LOG`. Cost: nothing. Consequence: a `Warn` nobody sees is a `Warn` that does not
  exist, and the engine's 24 Live `W`/`E` codes are documentation rather than diagnostics.
* **(B) The host enables at a `Warn` floor unconditionally**, and `BOYKO_LOG` raises it. Cost:
  ~20 ms of clock calibration and one sleeping thread per process — including every child process
  the test suite spawns. Consequence: the pre-migration behaviour is restored and the codes become
  reachable without foreknowledge.
* **(C) Wire the synchronous route.** `TargetControl::SYNC_BIT` is declared, `sync_out` exists, and
  `emit_unlaned_line` already renders and writes through it — but the bit **has no reader**:
  `lane.rs`'s only synchronous path is the no-lane fallback, not a route the control byte can
  select. Wiring it would let `error!` reach `stderr` with no thread and no calibration. This is
  the architecturally right answer and it is L12-shaped work, not L8b's.

**L8b did not wait on this.** The three terminal-exit codes (`E3002`/`E3003`/`E3004`) fall back to
`eprintln!` when `flush()` answers `NoConsumer`, on `boyko_threadpool::worker`'s already-blessed
precedent — so the host cannot exit silently under any of the three answers. The degrade codes and
the thirty `info!` sites follow (A) as specified. **The 31 already-silenced sites from L6/L7b/L8a
are untouched and remain silent**, which is what this entry is about.

---

## L8b — `boyko_app` never calls `flush()` or `shutdown()`, so SEAM S5's teardown half does not exist

Measured: `boyko_app` names `boyko_log::lifecycle` in exactly one place, `plugins.rs`, and calls
`boot` and `enable`. There is no `flush()` and no `shutdown()` anywhere in the crate.
`SEAM.md`'s S5 gives `boyko_app` *"the boot and teardown order, `flush_gpu` ahead of `flush`"*; the
boot half landed at L7 and the teardown half never did.

The consequence is not theoretical. `lifecycle::enable` spawns the sink thread and **drops its
`JoinHandle`** (`.spawn(sink_loop).map_or_else(…, drop)`), so nothing joins it. A record emitted
just before `return AppExit(true)` races the drain and loses more often than not — and the sites
L8b migrates are exactly the print-then-exit ones.

L8b's three terminal reporters call `flush()` themselves, so those records leave. **Every other
record emitted late in a run is still exposed**, including the `VB-ZONE summary` and artifact lines
that a measurement run ends with. The fix is a `flush()` on the normal teardown path and a
`shutdown()` after it, and it wants doing with the rung that owns the lifecycle rather than bolted
onto this one.

---

## L8b — deleting `boyko_demo`'s `log` facade left two channels with no replacement

The ledger specifies the deletion of `log = "0.4"`, `env_logger` and `console_log` from
`boyko_demo`, and L8b did it. Two things went with them:

* **Native**: `env_logger` was the only subscriber for the `log` facade in that binary, and
  `eframe`/`egui`/`wgpu`/`naga` all emit through it. **wgpu adapter selection and validation
  messages now go nowhere.** The replacement is a `log`-facade bridge feeding `boyko_log`, which no
  rung owns.
* **wasm**: `console_log` was the only channel reaching the browser console, and `boyko_log`'s
  console sink writes to `stderr`, which is a no-op on `wasm32-unknown-unknown`. So `E3001` — the
  record whose entire purpose is to explain a blank canvas — is emitted and unreachable there.
  It costs nothing **today**, because the wasm build is blocked upstream in `boyko_ecs` (the layout
  asserts fail 32-bit const-eval) and its CI leg is explicitly non-fatal. Whoever unblocks wasm
  owes the console sink a `web_sys::console` arm, or the failure goes back to being silent.

Both are recorded in `crates/boyko_demo/Cargo.toml` beside the dependency, so the next reader of
that manifest finds them without finding this file.

---

## L8c — four `Pending` code rows name profiling rungs that have SHIPPED, and all four conditions exist and are silent

Check 3c (`Pending == 0`) is L8c's, and it cannot arm while these four rows stand. Measured against
HEAD, each condition **exists in the tree and reports nothing**:

| row | condition, located | state |
|---|---|---|
| `W9202` `Pending("profiling 5")` | `boyko_rhi_vulkan::present::gpu_zone::alloc_pair` returns `None` once `used_pairs >= MAX_GPU_PAIRS` (128) | the bracket is simply unrecorded; nothing reports it |
| `W9217` `Pending("profiling 5")` | `runner.rs` calls `flush_vb_zone` **only** inside `vb_zone_seen >= WARMUP + frames` | a run that ends earlier — window closed, or `E3003`'s terminal `return` — leaves slots in flight, unflushed, unreported |
| `W9205` `Pending("profiling 8")` | `reduce.rs` increments `census.lost`; `contrast.rs` reads it as `window_complete` | counted, carried into the artifact, never warned about |
| `W9206` `Pending("profiling 8")` | `contrast.rs` has `NotResolved` + `NotResolvedReason` fully built | a refusal is returned; nothing warns |

**`GpuZoneRecorder::flush` itself is correct** and labels every in-flight slot `Flushed` — its own
doc says it exists so *"the last `GPU_RING_DEPTH` slots would [not] be dropped silently, which is
the loss a profiler exists to report rather than to commit"*. `W9217`'s hole is not in `flush`; it
is in the one path that never calls it.

**Also measured, and it is the shape of the thing:** `boyko_app::profiling` contains **zero**
`warn!`/`error!` calls across fifteen shipped rungs. The `92xx` emitters all live in
`boyko_ecs::…::profiling::diag`, which its own header names as the **sole** emitter of the block —
*"which is what keeps a profiler drop reported as a counter read rather than as a log record that
can itself be dropped under exactly the load that produced the drop"*. So these four do not become
`warn!` at the condition site: they route through `boyko_diag::loss::raise(DiagFlag::…)` sticky
bits that `diag.rs` reads. `flag_code`'s `match` is deliberately not `_`-terminated, so a new
`DiagFlag` variant **fails to compile** until it is paired with a code — the mechanism is already
built and simply has four unused inputs.

**The question for the owner is not how, it is whether these belong to L8c at all.** They are
profiling conditions, in profiling crates, whose rungs are marked shipped. L8c inherits them only
because `Pending == 0` is its gate. Either the profiling ladder reopens rungs 5 and 8 to land the
emitters it reserved codes for, or the four rows are re-dispositioned. Recorded rather than decided
because it moves work between two ladders.

---

## ~~L13b's revert clause has fired~~ — RESOLVED 2026-08-17: keep L13b, the 5× was an estimate

**OWNER RULING: keep L13b. The `5×` was an estimate, not a requirement.** Recorded below as it was
asked, because the measurement is the reason the threshold moved and a resolved question that
deletes its own evidence teaches nothing.

**What changed in the tree**: `02-SINK-LIFECYCLE.md`'s clause is re-cut from an acceptance
threshold into a **regression guard** at `≥ 4.0×` and `≥ 3 M rec·s⁻¹`, set from the four readings
and deliberately below the observed minimum rather than pinned to it — a bound at today's number
reds on ordinary variance, and a gate that cries wolf gets ignored. The bench prints `PASS` /
`REGRESSION` instead of `PASS` / `FAIL (revert clause)`, and its RED was shown by raising the guard
to `6×`.

**The lesson the corpus keeps**: the `5×` was written before anything was measured and nothing was
ever measured against it until the bench existed. A number invented in advance is a guess about the
answer; this corpus does not get to hold a guess and a measurement in one sentence and call the
guess the requirement.

`02-SINK-LIFECYCLE.md` states the clause without hedging: *"the entire justification is throughput.
If `sink_sustained_rate_binary` does not measure ≥ 5× `sink_sustained_rate` in the same sitting,
**L13b is reverted**. A format whose only reason to exist is speed must show the speed."*

The bench now exists (`crates/boyko_log/benches/sink_sustained_rate.rs`) and it was built to answer
exactly this. Four sittings on this box:

| sitting | text ns/rec | binary ns/rec | ratio | A-vs-A' twin gap |
|---|---|---|---|---|
| 1 | 41.02 | 9.54 | **4.30×** | 0.020 ns |
| 2 | — | — | **4.63×** | 0.085 ns |
| 3 | — | — | **4.68×** | 0.120 ns |
| 4 | — | — | **4.54×** | 0.080 ns |

**The instrument is sound and the result is not marginal noise.** The A-vs-A' twin — the same leg
measured twice around the other — drifts by 0.02–0.12 ns while the legs differ by ~31 ns, so the
sitting is not drifting. The separation is ~31 ns against a combined spread floor of ~1 ns, so it
resolves. Four independent sittings land in a 0.38× band, none of them touching 5×.

**The absolute half of the clause passes by a wide margin**: 104.8 M rec·s⁻¹ against a floor of
3 M. It is only the *ratio* that misses.

**And the measured scope is the one most favourable to L13b.** The bench times only where the two
paths differ — `render_payload` against `encode_record` — because everything upstream of the drain
and downstream of the sink's `write` is shared. An end-to-end sink rate would add a constant both
formats pay, which can only push the ratio *down*. So 4.5× is an upper bound on the end-to-end
figure, and the clause still misses.

**The three dispositions, and why this is not mine to pick:**

1. **Revert L13b as written.** The clause is unambiguous and the number is reproducible. Costs:
   `binary.rs`, its dictionary, `W0116`, the format tests and the offline decoder plan all go.
2. **Keep it and amend the threshold.** 4.5× at 105 M rec·s⁻¹ is a real improvement; a 5× line
   drawn before anything was measured is not obviously the right line. This requires the owner to
   say the threshold was the estimate, not the requirement.
3. **Keep it and make it faster.** The text leg's 41 ns is dominated by `core::fmt`; the binary
   leg's 9.5 ns is already close to a `memcpy` of 39 bytes. The ratio is more likely to move by
   *slowing nothing and speeding the text leg less* than by optimising the binary one — i.e. this
   route probably does not reach 5× without changing what the text sink does.

Each of these trades shipped, tested, documented code against a number, which is a values call, not
a performance fork. Recorded and surfaced rather than decided.

### Correction to the readings above, made after the instrument was fixed

The four sittings quoted in the table were taken with a spread floor that was **2 % of the reading
and nothing else** — the IQR was exactly zero, so `se.max(med * 0.02)` reported the subject's size,
not the clock's resolution. Fixing that (`benches/instrument.rs`) and re-measuring gives a fuller
picture:

* **Eleven sittings on an idle box: 4.29× – 4.68×.** The original four sit inside that band, so the
  ruling rests on the same evidence it always did, and the `4.0×` guard keeps its margin.
* **One sitting read 5.94×**, taken immediately after a build with the machine still busy. It is
  not a contradiction: the TEXT leg is the load-sensitive one, so load inflates the ratio.
* **The A-vs-A' twin test was too lax and has been re-cut.** It compared the twin gap against the
  ~32 ns *separation*, which admitted a sitting that drifted 3.4 ns on a 41 ns leg — 8 %, enough to
  move the reported ratio by ~0.35, wider than the entire band. It is now proportional: the twin
  must agree to within 2 % of the leg.
* **That change caught a false red.** A drifted sitting produced `2.61×`, which under the old test
  would have been reported as `REGRESSION` — an accusation against a format that had lost nothing.
  It is now correctly `NOT MEASURABLE (instrument)`.

The direction of the ruling is unaffected. What changed is that "reproducible" is now a claim about
an idle box with a drift-rejecting twin, rather than a claim resting on byte-identical numbers that
were byte-identical because the floor was fictional.

---

## `Once` is now the ONLY policy honoured by hand, and 39 sites do not obviously honour it

**Status:** OPEN — measured 2026-08-19, not fixed. Raised because the fix is a rung, not a footnote.

`rate::admit` is wired (`__log_rate_admits!`, the fourth gate). `Every`, `EveryN` and
`MinIntervalMs` are now applied mechanically by the emission macros. `Once` and `OnceCounted` are
NOT, deliberately: the latch stays a named `OnceSite` the site declares, because a `static` inside
a macro expansion cannot be named and `OnceSite::reset` exists precisely so an observer can reset
the latch it is about to test.

That leaves `Once` as the last policy whose declaration is kept by human diligence — which is this
campaign's signature defect shape. **Measured across `crates/**/*.rs` and `src/**/*.rs`, excluding
`codes.rs`, `tests/` and `benches/`: 45 `Live` rows declare `Once`/`OnceCounted`, and 39
(identifier-use, file) pairs have NO `OnceSite` anywhere in the file.** Two were read by hand and
are real:

* **`W0111` (`crates/boyko_log/src/census.rs:122`, `report_unsunk`)** — `#[cold]`, no latch, called
  from inside `census::rows()`, which is a **public iterator** any host may walk per frame. Its own
  doc comment says `Once`, "because the condition is a CONFIGURATION and not an event". A per-frame
  census overlay would emit it once per unsunk target per frame.
* **`E0109` (`crates/boyko_log/src/sink/crash.rs:82`, `report_unopenable`)** — `#[cold]`, no latch.
  It fires once today only because `arm()` is called once on the enable path; the row's `Once` is
  honoured by the CALL STRUCTURE, not by anything at the site.

The crude scan cannot tell an emitter from a mention (a `use`, a doc link, a test assertion), so 39
is an upper bound and the real count needs an emitter-aware walk — the shape `code_registry.rs`'s
existing checks already have.

**Two dispositions, and the second is the one that needs a ruling:**

1. **Audit the 39 and place the missing latches.** Mechanical, site by site, and each site's
   correct granularity is a judgement (`W2205` deliberately keeps its latches in a `Resource`, not
   a `static`, so one world's first divergence cannot silence another's).
2. **Place the latch in the macro after all, and make it resettable.** The objection above is
   testability, and it is answerable: register every macro-placed latch against its `&LogSite` in a
   walkable table and give `test-probe` a `reset_all_once_sites()`. Then all 45 rows are honoured
   mechanically and an observer resets everything before driving its site. This changes behaviour
   at 45 rows and costs `.bss` plus a registration on first emission, so it is a scope call.

Recorded rather than decided.

**Addendum (same session): the corpus's own accounting for `Once` is not built either.**
`00-GOAL-TARGETS.md:37`, `01-EMISSION-RING.md:273` and `05-LADDER-GATES.md:918` all specify an
`ONCE_SITES` intrusive list and one `LOG-ONCE` census row per fired site
(`code=W2102 site=device.rs:3118 fired=1 suppressed=UNCOUNTED(by policy)`). **Neither exists.** Two
doc comments in the tree named it as though it did — `crates/boyko_log/src/rate.rs` called it "the
`ONCE_SITES` walk's answer", and `crates/boyko_rhi_vulkan/src/present/passes/gbuffer.rs` claimed a
site "enrols itself in `ONCE_SITES` so the `LOG-ONCE` census can report that it fired at all". Both
corrected in place with the wiring commit.

This is the same finding as the 39 latch-less sites, from the other end: **nothing enumerates
`Once` sites, so nothing could notice.** Building `ONCE_SITES` + the `LOG-ONCE` rows is the natural
pair to disposition 1 above — the audit needs the enumeration, and the enumeration makes the audit
mechanical instead of a grep.

---

## UPDATE (same day): the enumeration is BUILT, two of the sites are fixed, and the audit is now a number

`crates/boyko_log/src/once_sites.rs` is the register the corpus specified. The drain notes every
emission from a site whose `LogSite::rate` is `Once`/`OnceCounted` — off the emitting thread, from
cold `'static` data — and `census::print` emits one row per fired site:

```
LOG-ONCE code=W2102 site=crates/boyko_rhi_vulkan/src/device.rs:3118 fired=1 suppressed=UNCOUNTED(by policy)
```

**`fired > 1` is the defect, stated as a number.** The row even says so:
`  <-- DECLARES Once AND HAS NO LATCH`. The 39-pair grep was an upper bound with no way to tighten
it — it cannot tell an emitter from a `use` or a doc link — and this replaces it with a per-site
run-time count.

**Two engine sites are fixed with it, and the first was found BY it:**

* **`W0111`** — `report_unsunk` was called from inside `census::rows()`, a **public iterator** a
  host may walk every frame. Reverting the fix and walking `rows()` ten times makes the register
  read `fired: 10` for `census.rs`. The report moved to `census::print()` (flush and shutdown)
  behind a named `UNSUNK_REPORTED` latch. **A query must not have a diagnostic as a side effect**,
  which is the general form of this defect.
* **`E0109`** — `report_unopenable` had no latch; its `Once` was honoured by the call structure
  (`arm` runs once on the enable path), so a process that disabled and re-enabled would report
  again. It has a named `UNOPENABLE_REPORTED` latch now.

**What is still open.** The remaining ~37 pairs are not audited: the register only reports sites
that FIRE, and a site that never fires in a test run leaves no row. Reading it properly means
running the engine and reading the census, which is the next step rather than a grep. The
macro-auto-latch question (disposition 2 above) is untouched and still needs a scope call.

---

## The certification recipe never runs 311 tests, and 177 of them do not say why

**Status:** OPEN — measured 2026-08-19. Surfaced rather than half-fixed, because the fix is a
policy call.

The two-half recipe in `CLAUDE.md` reports green without running a single `#[ignore]`d test:

```
cargo test --workspace --exclude boyko_rhi_vulkan --all-targets --no-fail-fast
cargo test -p boyko_rhi_vulkan --all-targets --no-fail-fast -- --test-threads=1
```

Measured across `crates/`, `src/` and `tests/`: **311 `#[ignore]` sites in 90 files.**

**Do not read that as 311 defects.** Of the 134 that state a reason, 122 name a GPU, device,
window or swapchain requirement, 3 name process-wide state and 2 name a dump or golden — all
legitimate reasons for a test to be driven by hand. The number worth acting on is the other one:

**177 sites carry `#[ignore]` with NO reason at all.** They sit in 78 files, mostly windowed/GPU
suites whose module docs do explain the requirement — so 177 is an upper bound on "silenced with
no record", not a defect count. But a bare `#[ignore]` cannot be told apart from a test that went
red once and was quieted, and that is precisely the distinction this campaign exists to make
mechanical.

**One instance is verified and load-bearing right now.**
`crates/boyko_log/tests/l14_sink_policy.rs`'s `an_armed_target_no_sink_accepts_is_unsunk_and_says_so`
is the **only** observer of the `W0111` latch this rung introduced, and the recipe does not run it.
It was run by hand for this commit (`-- --ignored --test-threads=1`, green) and that is a process
step, not a gate.

I did not merge it into its sibling: the second test re-`boot`s and re-`enable`s, and `enable()`
on an already-enabled process is a different code path — a merge could produce a test that passes
for a new and wrong reason, which is worse than one that does not run.

**Two questions for the owner:**

1. Should the recipe gain a third leg, `-- --ignored --test-threads=1` per crate — accepting that
   the GPU/windowed suites will then need a device present?
2. Should a bare `#[ignore]` become a tidy-check failure, so that switching a gate off always
   leaves a written reason? That is the same rule as the mandatory `// SAFETY:` comment and the
   `#[allow(clippy::disallowed_types)]` rationale, applied to the third way to make a check
   disappear.

---

## RESOLVED: the 39 was an upper bound, and the sharpened count is 19 latched of 20

**Status:** CLOSED 2026-08-19 — the residue is named below and is small.

The entry above reported "39 (identifier, file) pairs carry no `OnceSite`" and said in the same
breath that it was an upper bound with no way to tighten it, because a grep for identifier USES
cannot tell an emitter from a `use`, a doc link or a test assertion. It has now been tightened, and
the answer is different in kind:

**Emission-aware, production-code-only: 20 files emit a `Once`/`OnceCounted` code. 19 hold a latch.
The one flagged was `boyko_log/src/macros.rs`, whose `warn!` DOC COMMENT explains the class/number
pairing using `W2102` as its example** — not an emission at all, and gone once the scan reads the
production stream instead of raw text.

**A middle draft was wrong in the other direction and is worth recording.** Requiring the latch
inside the emitting FUNCTION returned 21 — of which **19 were correct code**:
`boyko_ecs::…::profiling::diag` holds one `OnceSite` per live code in a `LATCHES` array behind a
`claim(number)` helper, so no emitter there names `.claim()` itself. A gate written that way would
have accused nineteen sites that latch properly. Measuring before writing the check is what caught
it.

**Two real defects came out of this and are fixed**, both found by the run-time register rather than
by either grep: `W0111` emitting from inside `census::rows()`, a public iterator a host may walk
every frame; and `E0109`, whose `Once` was honoured by the call structure rather than by anything at
the site.

**The gate is `check_8_every_file_emitting_a_once_code_has_a_latch`** in
`crates/boyko_log/tests/code_registry.rs`, with an anti-vacuity floor (a scan finding fewer than ten
emitting files fails rather than passes) and two REDs shown.

**What it still cannot prove**, stated in the check's own doc: that the latch guards THAT emission
rather than another in the same file. That residue is `boyko_log::once_sites`' at run time, where a
`Once` row reading `fired > 1` is the defect stated as a number.

**Disposition 2 above — placing the latch in the macro — is therefore NOT needed** and is withdrawn
as a question. The human link is now checked mechanically at build time and observably at run time,
which is what auto-latching was going to buy, without making all 45 rows untestable in isolation.

---

## 2026-08-20: the census reaches only the console — a `shipping` log never carries its own loss summary

**Context.** The windowed runner now ends the diagnostics session (`lifecycle::shutdown()` at the
end of `run_windowed`), so `close_out` runs in every real host: the final drain delivers the lane
tail and `census::print()` fires. Measured on live 16-frame runs of the `clear` example.

**The question.** `census::print` writes through `sync_out::write_oracle_line` — the synchronous
console channel — and through nothing else. Under `dev`/`editor` (console on) the census reaches
the operator. Under `shipping` (binary file, console off) and `shipping-min` (text file, console
off) the rows are refused at the console gate and reach **no destination at all**: the uploaded
log a released title produces does not say whether it lost anything, which is the one question a
reader of that log asks first.

**Two dispositions, neither taken without you:**

1. **Route the census through the ring** (ordinary records, `Diag` target) so it lands in
   whatever sinks the preset opened. Cost: the census becomes subject to the same admission
   control it reports on — a storm that drops records could drop the census rows that say so.
   The current synchronous channel exists precisely to be outside that machinery.
2. **Render the census into the file sinks directly under the drain token at `close_out`**,
   beside the console write. Cost: a second delivery path for one report, and the binary sink
   would carry text lines outside the record format (or needs a frame kind for them).

**RESOLVED same day, by the owner's standing directive to decide without asking (2026-08-20 chat).
Disposition 1 was taken, narrowed to shutdown-time.** Disposition 2 fell to a fact discovered on
inspection: the `.blog` is a framed format, so "write the rows into the sink directly" means
constructing record frames anyway — which IS disposition 1 with extra steps. `census::print` now
emits every row as an ordinary ring record (`LOG-CENSUS …` / `LOG-ONCE …` under the `log` target),
`shutdown` orders itself "emit → deliver → close" in both arms, and the sinks — the binary one
included, which previously had no shutdown-time close at all — close only after the final pass.
The stated cost stands and is accepted: the census obeys the admission control it reports on,
which at shutdown means a quiet ring and an immediate delivery pass. Gate:
`log_host_shipping_min.rs` asserts `LOG-CENSUS` rows in the preset's own file after `shutdown`.

---

## 2026-08-20: SV0 returns as the dedicated pass — decision taken, not asked

Diagnostics closed; rendering resumed per the standing directive. The first decision was SV0's
disposition, OPEN since Rev 9: **taken — `sdf_mesh_shadow.comp`, RENDER-PARITY-PLAN §3.2's
critic-agreed Option B.** The numbers that decided it: the inline carried a MEASURED ~+75%
dark-path tax on every VB frame (reverted at `13f1c9a3`), the armed ratio 2.34× was the VB tail
being a worse HOST for the march (not the march's own cost), and the "dedicated pass rejected by
measurement" premise was retracted as false — the pass never existed to be measured. Ladder and
gates: `docs/VB-SV0-SDF-SHADOW-PLAN.md` Rev 10 (DP1–DP5), including the two gates the inline never
had — a per-producer byte budget and a dark-dispatch ABBA A/B with a pre-registered grid-step
budget.

---

## 2026-08-20: two Aether A7 candidates, recorded here so the book's "recorded" claim is true

Both were found by review during A6 and are documented at their parse sites, but no planning
document listed them until this entry. Neither blocks anything; both are DX defects of the shipped
surface.

1. **`at BARE_PATH { … }` swallows the node body as a struct literal.** `camera at MY_POSE
   { aspect: 1.5 }` parses `MY_POSE { aspect: 1.5 }` as one expression, and the diagnostic that
   results — ``the `camera` node needs an `aspect:` key`` — contradicts what the user wrote. The
   workaround (parenthesize the pose) is documented at `parse_at` and at the `sdf` arm, but the
   error message carries no hint. The fix is a hint on the required-key diagnostic when the `at`
   expression was a brace-suffixed path. Same hazard exists for `sdf EXPR`.
2. **`clippy::too_many_arguments` on a generated system fn lands on the whole `aether!` block.**
   A `system` with 8+ demand-driven params emits a fn clippy flags, and the span is the macro
   token — the user cannot act on it. An `#[allow]` inside `system_fn` would re-bless every A2
   token pin, so the fix belongs to a deliberate A7 pass, not a drive-by.

Also recorded: `engine_packages_census` does not classify the three `aether*` crates — it reds on
every full-workspace run for a reason unrelated to whatever is being tested. The census's
classification table needs three rows (they are engine crates: the language front-end, the shim,
and the integration-test crate).

> **RESOLVED — verified 2026-08-29. The three rows exist, and the disposition is the OPPOSITE of the
> one this paragraph recommended.** `tests/engine_packages_census.rs` now carries `"aether-lang"`,
> `"aether"` and `"aether-tests"` in `USER_PACKAGES` — the list documented as *"workspace members
> that are deliberately NOT engine packages"* — each with its own rationale comment: the transpiler
> half and its two-line proc-macro shim execute only inside rustc's process, so no runtime zone can
> ever exist in them and `Engine` would claim a runtime membership that is false by construction;
> `aether-tests` boots real `App`s the way a game does, on the `bench-bevy-vs-boyko` precedent. So
> the classification hole is closed at source, but "they are engine crates" above is **not** what
> landed, and the sentence is left standing rather than rewritten so the record shows the call that
> was actually made. (What is verified here is the three rows and their reasons, read at the file;
> the census was not re-run for this note, so no claim is made about the run's colour.)

---

## 2026-08-20: DP6a is BLOCKED on arithmetic, not on questions — recorded for the resume reader

DP6-0b's re-taken cells landed §R4.3.7 on branch 2 (MIXED): `E_split_host = 11 264 ns` of the
split tail's hosting surcharge is real (the other 87.8% of DP6-0's finding was instrument skew,
now repaired — the split shade fell 112 640 → 35 328 ns on the restamped instrument). Per the
design's own pre-registered rule, DP6a does not land until Decision 3's fused row is re-derived
with that term. The re-derivation is in flight; this entry exists so a resume reader does not
start DP6a from the ladder order alone.

**RESOLVED, same day.** The re-derivation landed as Decision 3's *"Trade-off, RE-DERIVED at
DP6-0b"* sub-block (design Rev 4.4): `Δ NET_fused ∈ [−5 404, +14 848] ns`, point estimate
`+1 034`, against a `+5 120` bar. §R4.3.7's block is discharged and **DP6a has landed** (design
Rev 4.5 carries its review dispositions). The headline of that revision is a WITHDRAWAL — §Goal's
"fused boots: cost-neutral **by construction**" was a construction claim refuted by measurement,
and neutrality is now claimed at DP6d or not at all.

---

## 2026-08-20: `vb_occ_dense` cites a release-live assert that does not exist — and one citation is inside a panic message

Found while enumerating the fixtures VB-SV0 DP6a turns; **pre-existing, unrelated to DP6a, and
left unfixed deliberately** (out of that rung's scope — filed rather than drive-by repaired).

`crates/boyko_app/tests/vb_occ_dense/mod.rs` twice attributes a guard to
`boyko_app/src/runner.rs:1101-1106`:

* its doc: *"`boyko_app::runner`'s bench arming carries a release-live `assert!(!mesh_geo_shade_split, …)`
  whose message is about **VB-P1d's published break-even**"*;
* the body of `assert_no_split_producer`'s own **panic message**, which tells a tripped author that
  arming a pre-light consumer *"makes `boyko_app::runner`'s bench arming panic at runner.rs:1101-1106
  with a message about VB-P1d's break-even"*.

**No such assertion exists anywhere in `runner.rs`.** A repo-wide search for a release-live
`assert!(!… mesh_geo_shade_split …)` returns nothing outside `boyko_render`'s own unit tests, and
`runner.rs:1100` is the DP6-0b `vb_zone_chain` / `vb_zone_derived` selector — an `if
… mesh_geo_shade_split { VB_CHAIN_SPLIT } else { VB_CHAIN_FUSED }`, which explains nothing about
break-evens and panics on nothing.

Why it is worth an entry rather than a silent fix: **the second citation is inside a panic
message**, i.e. it is read by exactly the person who has just tripped the fixture and is looking
for the cause. Sending them to a line that is a chain selector costs them the one lead they were
given. The class is the campaign's recorded one — a datum nobody re-derives, which the first
"fix" re-blesses. Repair should either name the real guard (if the intent survives somewhere) or
delete both citations and state the fixture's own reason without borrowing another rung's.

---

## 2026-08-20: DP6b's `-P` gate cannot be "character-identical" as the design spells it — and the measured reason

VB-SV0 DP6-DESIGN §Metrics (P1-5) specifies the new two-sided preprocessor gate as: *"`dxc -P
vb_geo.comp.hlsl` (no defines) is **character-identical** to the pre-DP6b file's `-P`"*. **As
written that is unachievable for any additive edit, and the shipped gate
(`crates/boyko_rhi_vulkan/tests/vb_geo_preprocess_sync.rs`) states a normalized form instead.**

Two independent reasons, both measured on the pinned VulkanSDK 1.4.350.0 `dxc`:

1. **`#line` directives carry the path and the line number.** The pre-DP6b side has to be
   materialised somewhere (`git show` writes it to a temp file), so the paths differ; and DP6b's
   edit is additive, so every line after the first insertion is renumbered. A gate comparing the
   raw text would red on a pure comment addition.
2. **`dxc -P` does not preserve blank lines across an elided region.** With the `#ifdef
   VB_SV0_TERM` block present, the single blank line between `} pc;` and `[numthreads(64, 1, 1)]`
   is swallowed by the `#line` jump that replaces the guarded span. **With `#line` stripped the two
   sides differ by exactly one empty line** — on a source every added line of which is inside a
   guard. No source formatting removes it: the jump is emitted whenever the elided run is long.

**Shipped form:** strip `#line` directives and empty lines, compare the remainder
character-for-character. Measured: **571 identical lines** pre-DP6b vs HEAD with the flag OFF;
**916 lines** with `-D VB_SV0_TERM=1`. The gate ships with its own two-directional sensitivity
control — an edit OUTSIDE every guard must move the base text, an edit INSIDE the guard must not
(and must move the `-D` text) — so the normalization is proved not to have removed the teeth
rather than argued to have kept them.

**What the normalization gives up, stated:** a mutation that changes ONLY whitespace outside a
guard. That cannot change a compile, and the `.spv` byte gate
(`vb_raster_geo_classify_spv_sync.rs`) covers the compile independently.

**Not filed as a defect in the design.** The spelling was written before anyone ran `dxc -P` on
this file — the same shape as the design's own P1-5 finding that no `.rs`/`.ps1` in-tree invoked
`-P` at all, only plan prose. It is recorded here so the next reader of that sentence does not try
to "fix" the test back to a literal comparison it can never pass.

## 2026-08-20: DP6b landed its SHADER half only — the `vb_geo_aux_layout` widening is atomic with a file that was checked out dirty

DP6b's ladder entry has two halves. The shader/variant/gate half is landed (guarded span,
`vb_geo_sv0.comp.spv`, `embed_spirv!` + accessor, `spv_sync` row, the new `-P` gate,
`sdf_field_edsl_sync` re-pointed, manifest row). **Decision 5's `vb_geo_aux_layout` 3 -> 5 widening
and its two boot descriptor writes are NOT landed**, because
`crates/boyko_rhi_vulkan/src/present/targets.rs` carried unrelated uncommitted work from a
concurrent lane at the time.

**They cannot be split, and the reason is mechanical rather than stylistic** — recorded so nobody
lands "just the layout" as a smaller step:

* `rhi_impl/device.rs::create_bind_group` sizes the descriptor POOL from the `entries` histogram
  and then allocates a set from the LAYOUT. A 5-binding layout fed 3 entries allocates against a
  pool missing one `STORAGE_IMAGE` and one `STORAGE_BUFFER` -> `VK_ERROR_OUT_OF_POOL_MEMORY` on
  every split boot.
* The same function carries `debug_assert!(count == desc.layout.entry_count)`, so a debug build
  panics before it gets there.

So the widening (`boyko_app/src/gpu_scene/mod.rs`), the `!rg8_ok` placeholder bind
(`targets.rs::vb_geo_aux_set`) and the `vb_geo_sv0` boot pipeline land together or not at all. The
DP6b gate is otherwise met: all `*_edsl_sync`/`*_spv_sync` green, the five named goldens
byte-identical, `gpu_command_census` and `vb_bench_query_validation` green.

## 2026-08-20: the SV0 tuning-block VALUE pin covered two of four shipped hosts — CLOSED at DP6b, with one adjacent claim left open

**Found by the DP6b review, and the finding is partly about the report that preceded it:** the DP6b
implementation report said this gap was "filed". It was not — it was named in a report and filed
nowhere, while the twin defect of the same class (`vb_occ_dense`'s citation of a non-existent
release-live assert, the entry above) was filed properly the same day. *One twin filed, one
claimed.* Recorded because the failure mode is the campaign's own: a datum that exists only in
prose nobody re-derives.

**The defect.** `sdf_shadow_leaf_oracle.rs`'s `SHADOW_CONST_SOURCES` read
`["deferred_pbr.hlsl", "sdf_gbuffer_composite.hlsl"]` above a doc calling them *"the two sources
that carry the SHADOW march tuning block"* — while **six** shipped shaders declared `SHADOW_K`. The
two SV0 marcher hosts, `sdf_mesh_shadow.comp.hlsl` (DP1) and `vb_geo.comp.hlsl` (DP6b), were never
added. An edit to `SHADOW_K` in either one alone stayed green in that oracle, green in
`sdf_field_edsl_sync.rs` (which pins the consts' PRESENCE and their ORDER against the include, never
their VALUES), and green in every golden — the only red would have been the `.spv` byte gate, whose
documented repair for a red is *"re-run the header recipe and commit the result"*, i.e. the exact
motion that blesses the divergence.

Two independent citations pointed at a test that does not exist — `sv0_consts_match_deferred_and_marcher`
— in `sdf_field_edsl_sync.rs`'s panic message and in `docs/VB-SV0-SDF-SHADOW-PLAN.md`. The real
test had been RENAMED to `sdf_shadow_and_ao_consts_match_deferred_and_marcher`, and the rename is
what carried the reader past the coverage hole: looking up the cited name returns nothing, so
nobody reached the list to notice what was missing from it.

**CLOSED, not filed.** `SHADOW_CONST_SOURCES` is now all four shipped hosts; the selection-size
assertion derives from `SHADOW_CONST_SOURCES.len()` instead of a hand-written `21`, so adding or
removing a source can no longer silently shrink the gate; the doc states why the two SV0 hosts were
missing; the `sdf_field_edsl_sync.rs` panic message cites the real test name. **All four hosts fold
identically on the first run** — the blocks were verbatim mirrors, so no divergence had accumulated
in the window; the gate is red-capable, demonstrated by perturbing `SHADOW_K` in `vb_geo.comp.hlsl`
alone (`folds to 9 (0x41100000), but the host mirror is 8 (0x41000000)`).

**Still open, filed rather than fixed (pre-existing, unrelated to DP6b).**
`vb_geom_fetch.hlsli:581-583` claims of `vb_sv0_face_normal`'s cost argument that *"that is verified
on the artifact, not assumed: the committed `.spv` are disassembled and the `Cross`/`InverseSqrt`
chain must appear INSIDE the gate's conditional region, never hoisted into the entry block by
`-O3`"*. **No test in the workspace disassembles any `.spv` for a `Cross`/`InverseSqrt` hoist
check.** The claim's own words are what make it worth an entry — "verified on the artifact, not
assumed" is precisely the sentence a reader trusts instead of re-checking. Repair is either the
census the sentence describes (a `spirv-dis` block-membership assertion, the
`vb_raster_geo_classify_spv_sync.rs` builtin-census idiom) or striking the claim; DP6b widened the
`VB_SV0` definer set to two, so the sentence now also covers a module nobody has looked at.

## RESOLVED 2026-08-20: particles P1's Lipschitz skip — the ARCHITECT ruled the PLAN was the stale artifact, not the code

**Disposition (architect, 2026-08-20): the shipped form stands; `docs/PARTICLES-PLAN.md` §D9 is
amended and carries an ERRATUM.** The original line applied the reported→euclidean transform to an
operand that was already euclidean; the corrected block, the tunneling class it authorized for any
`k > 0` smooth edit, and the `radius·(L−1)` conservative band the shipped form pays instead are all
recorded there. See **§D9 + ERRATUM**. The entry below is kept verbatim as the escalation that
produced the ruling.

**What remains open is only the second half:** no `k > 0` fixture exists, so the two forms are
still discriminated by DERIVATION and not by measurement. Building one is the follow-up, and it is
a scene, not a code change.

**Not a question about what to do — the conservative form shipped. Recorded because the plan's own
text now disagrees with the code on one line, and the reader who checks D9 against
`particle_sim.comp.hlsl` deserves to find the reason here rather than derive it.**

`docs/PARTICLES-PLAN.md` D9 writes rung P1's per-substep skip as

```
if (cached_d - speed*timestep/FIELD_LIPSCHITZ_L > radius) { cached_d -= speed*timestep/L; skip }
```

— the Lipschitz constant DIVIDING the travel. The shipped block multiplies by it instead
(`travel_l = length(vel) * pc.timestep * FIELD_LIPSCHITZ_L`, compared against `radius * L`).

**The defect is a UNIT MISMATCH, and naming it is the point of this entry.** The three quantities
in that line do not live in the same space. `cached_d` is a value the FIELD reported; `radius` and
`speed*timestep` are EUCLIDEAN world lengths. `sdf_field.hlsli` states the conversion between them
in its own words — *"`d / L` is a conservative lower bound on the Euclidean clearance"* — so the
comparison is only meaningful once one side is converted. D9 converts neither: it divides the
euclidean travel by `L` (a quantity that is already euclidean) and compares it against a reported
value left unconverted.

Done properly, in euclidean units throughout: the true clearance `c` satisfies `c >= cached_d / L`,
a move of `s` leaves `c' >= cached_d/L - s`, and the substep is safe to skip exactly when
`cached_d/L - s > radius`. Multiplying through by `L` — which is what the shipped code does, so
that every per-substep operation stays a multiply — gives `cached_d - L*s > L*radius`. **`L`
multiplies the travel and the radius; it divides nothing.**

**What the shipped form costs, stated so it is not mistaken for the hazard.** Against a field with
`L == 1` the shipped test re-evaluates earlier by `(L-1)*(s + radius)` of reported distance. The
travel term `(L-1)*s` is the *necessary* correction for a super-Lipschitz field. The remaining
`radius*(L-1)` is a **conservative band**: the shell is a euclidean length carried through the same
`1/L` clearance bound as the distance, so it is over-stated in reported units. Its price is extra
field evaluations near a surface — never a missed contact. D9's form errs in the opposite
direction, and that direction is unbounded: `d > radius + s/L` passes wherever the safe
`d > L*(radius + s)` does and in a band above it, so it skips substeps in which contact happened.
Every skipped substep is a substep in which the field is not evaluated at all, so this is a
TUNNELING class — the one failure this rung's gate exists to bound.

**Scope of the disagreement, stated so nobody re-opens it as a bug.** The two forms are IDENTICAL
at `L == 1`, which is every hard-CSG (`smoothness == 0`) scene, i.e. every fixture in the tree today
including P1's own live fire. They diverge only where a smooth edit makes the field
super-Lipschitz — precisely the regime the constant exists for.

**What was left open, and how it closed:** whether D9's line should be corrected in the plan — an
architect edit, since the plan is APPROVED Rev 4 and an implementer amending its normative
pseudocode is not the same thing as recording a deviation. **Ruled 2026-08-20: corrected, with an
ERRATUM.** The second half — whether a fixture with a `k > 0` edit should exist to exercise the
`L > 1` regime — stays open: today no shipped scene has one, so the divergence remains unmeasured in
both directions and the shipped form's soundness rests on the derivation.

---

## The owner channel exists twice, and the copies have diverged (2026-08-21) — RESOLVED 2026-09-21 (rung A0: both commits OBSOLETE, neither merge)

**Measured, not suspected.** `git merge-tree --write-tree feat/multi-paradigm-render
claude/trusting-ramanujan-0f8927` reports five conflicts, and one of them is an **add/add on this
very file**. Both branches created `docs/OPEN-QUESTIONS.md` independently as "the standing owner
channel per CLAUDE.md", neither knowing the other had:

* `claude/trusting-ramanujan-0f8927` seeded its copy at `867dd734` with the worktree-clippy
  decision and the two-dispatcher-lanes observation;
* this branch grew the copy you are reading now, to 2853 lines.

Neither is a subset of the other. **A reader cannot tell which is current, and finds out only by
acting on the stale one** — the same failure mode the `docs/ru/` rule exists to prevent, where a
diverged pair is worse than a missing one.

**Why this is filed rather than fixed.** The merge that would reconcile them is not mechanical.
Its other four conflicts are `docs/FEATURE_MAP.md`, `docs/SYSTEMS.md`, and — load-bearing —
`crates/boyko_ecs/src/ecs/core/ecs_master/event_api.rs` and
`crates/boyko_ecs/src/ecs/core/schedule/schedule.rs`. Both sides edited the same executor: the
event-lane branch moved completion publication under unwind protection, and this branch moved
elsewhere in the same functions. Resolving that requires understanding both changes, not choosing
a side, and it touches the ECS kernel's panic path — the part of the scheduler whose failure mode
was a **silent hang**, which is the worst-shaped bug this kernel has produced.

**What the owner needs to decide:** whether the event-lane fix reaches this branch by merging
`claude/trusting-ramanujan-0f8927` directly, or by landing it on `master` first and merging that.
The kernel conflict is the same either way; the difference is which history carries it.

**Not started because** the main checkout currently holds ~484 lines of live uncommitted work in
`crates/boyko_rhi_vulkan/src/present/targets.rs` and `device.rs`. A kernel merge wants a clean
tree and its own worktree.

**Ruled 2026-09-21 (rung A0 of the unified plan; the code-reviewer's review, accepted by the
orchestrator): neither route.** Both `claude/trusting-ramanujan-0f8927` commits are OBSOLETE —
`6a9871bb`'s panic half is a strict subset of A2, its lane gate contradicts ruling E4, and
`867dd734`'s seed is exactly the union hazard this entry names; the full ruling is the 2026-09-21
A0 entry at the top of this file. What the seed held, appended here so the branch's prune loses
nothing:

* **Entry 1 (clippy config leaks across nested worktrees) — resolved by history, not by a merge.**
  Its option 1, "merge the hot-path ban to master", happened when `master` fast-forwarded on
  2026-09-07 (`7e908c87` carries `clippy.toml` from `3b3f9ee6`; the seed's base `e65a5673` did
  not), so every branch cut from `master` now carries the same config, and the plan's lanes live in
  `D:/wt/*`, outside the ancestor walk from `D:/claude/BoykoEngine`.
* **Mechanism (recorded nowhere else on this line):** a worktree nested under `.claude/worktrees/`
  inherits the parent checkout's `clippy.toml` through clippy's ancestor-directory search;
  `CLIPPY_CONF_DIR` does NOT stop the walk; a local empty `clippy.toml` does (nearest wins).
  Measured as 73 false `disallowed_types` errors across `boyko-macros`, `boyko-threadpool` and
  `boyko_shaderdsl`, whose aborted lib lints then masked their dependents.
* **Residual exposure: nested worktrees only** — a worktree under the checkout on a branch whose
  `clippy.toml` differs from the parent's. Entry 2 (two distinct dispatcher-owned lanes) is carried
  in the A0 entry, where it belongs with ruling E4 and D-E20.

---

## No `.gitattributes`, and the worktree rule just armed what that costs (2026-08-21)

**Measured while fixing `the_thresholds_file_is_the_one_r0a_froze`.** Git for Windows ships
`core.autocrlf = true` in its system gitconfig, so every checkout made under it writes text files
with CRLF, and every checkout made before it kept LF. This repository has **no `.gitattributes` at
all**, so on-disk bytes are a property of *when and where the checkout was made*.

The main checkout has LF. All three worktrees (`D:/wt/gates`, `D:/wt/reflect`, `D:/wt/ui`) have
CRLF. Any gate that hashes a file's **raw** bytes therefore reports where it ran.

**Why this surfaced now and not a year ago.** The one-worktree-per-system rule created the first
new checkouts this repository has seen in a while. The defect it exposed was latent from birth —
`assert_thresholds_frozen` has never normalised, across every commit since the module was created
at `21edc80f` — and was invisible for exactly as long as only the LF checkout ran it. Adopting the
rule did not cause the bug; it armed it, and it will keep arming this class.

**Fixed at the one site that could red.** `assert_thresholds_frozen` now hashes LF-normalised bytes,
matching the definition its two sibling sites (`vg_thresholds_freeze.rs`, `vg_r0_reference_rig.rs`)
already used and documented. The pinned literal is unchanged.

**One residual, which cannot red and is therefore worse in a quiet way.**
`crates/boyko_app/tests/vg_r0d_census.rs:332` hashes `assets/vg_corpus/CORPUS.toml` over raw bytes
and *writes the digest into* `docs/VG-R0-DENSITY-CENSUS.md`. Nothing compares it against a pin, so
no gate will ever complain — but the number recorded in that document depends on which checkout
produced it. A future reader comparing two runs would be comparing checkout configurations. It sits
behind a GPU `#[ignore]`, so it is not urgent.

**The decision that is yours, not mine.** A `.gitattributes` — `* -text`, or a narrower rule for the
frozen/hashed files — would make on-disk bytes deterministic everywhere and retire this whole class.
The cost is that it rewrites line endings across working trees on the next checkout, and two lanes
(`feat/reflection`, `feat/ui-advanced`) are mid-flight in worktrees right now. That is a
disruption I should not schedule for you. The thresholds gate is immune either way.

## Fixing defect B reopens axis A — by the register's own return row (2026-09-07)

`KE16-RESULTS.md` §B shipped the sentence "Axis B — B0 BY CONSTRUCTION", and the `compile_error!`
that justified it said the worker joiner "needs a REGISTERED destination deque, which only the A1
arms give it". **That reason was false.** Every arm builds `worker_count` deques and registers their
`Stealer`s unconditionally (`thread_pool.rs:718-737` — the only `cfg` there picks LIFO vs FIFO), and
each worker is handed one by move (`worker.rs:33`). What the A1 arms uniquely publish is the deque's
ADDRESS into the thread's TLS (`worker.rs:60-61`); `tls::worker_lane_for` already has a working
non-A1 branch (`tls.rs:238-248`) carrying `allow(dead_code)` only because nothing calls it. The
refusal itself stays — `b1`/`b3` call `lane.deque()` — but it is a REACHABILITY gap, not a
structural impossibility, and the message now says so.

The defect is live under `a3` by direct measurement, not by occupancy inference: the shipped
`the_worker_joiner_does_not_run_a_residue_before_re_checking_its_scope` under `--features ke16-a3`,
five runs, `running 1 test` each, reads a **median 192.1 ms** against a 60 ms budget and a 132 ms
residue floor, where the B arms read 4 and 5 us.

**The decision that is yours.** A remedy exists and its code is already written (`join_on_worker`,
five review rounds old); publishing the TLS deposit under a new `ke16-b4` costs one store per worker
per process and changes **no steal granularity at any site**, so it does not trip the letter of
`a1f`'s return condition. But `KE16-REJECTED.md:359-362` and `:368-369` give `b1`/`b3` a SECOND
return trigger — *"or if defect B is fixed by some other route that gives the worker joiner a
registered destination deque"* — and that route is exactly this one. So `b1`/`b3` return the moment
the remedy lands, they build only over `a1`/`a1f`, and the comparison becomes **`a3+b4` vs
`a1f+b1`**, not vs `a1f+b0`.

The monotone shortcut does not save it: `a3+b4 <= a3+b0 < a1f+b0` is arithmetically true and bounds
nothing about `a1f+b1`. `a1f+b1` needs a 27.9 % gain on `worker/body_10us_tasks_4W` to tie, that
cell is the worker route, and `a1`'s worker route was measured **with defect B live**
(`top_lane = 13/24/21`), where 33 x 10 us = 330 us sits at the same order as the 159 us cell.

**So the remedy's true price is a re-run of the axis-A head-to-head on a new substrate**, interleaved
pass-by-pass in one session, on the physics primary and the deciding cell — a quiet machine. The
alternative is legitimate and already shipped: leave defect B live, as `a3+b0` does today while
clearing both acceptance clauses (the first by 2.4x, the second at 93.5 % of budget). Not scheduled
without you. See `docs/threadpool/KE16-DESIGN-B4.md`.

## graphify is not installed on this machine at all (2026-09-07)

`CLAUDE.md` and two `PreToolUse` hooks require `graphify query` before reading or grepping source,
and the `post-commit` hook rebuilds the graph. **The tool is absent.** All three of the hook's
interpreter probes fail: the pinned `C:\Python314\python.exe` exists but `import graphify` raises
`ModuleNotFoundError`; `graphify-out/.graphify_python` records that same interpreter (so the file is
not wrong about the path, it is wrong about the module); and no `graphify` launcher is on PATH from
either shell. `C:\Python312` is an empty directory with no `python.exe` — the interpreter that held
it was removed by the upgrade to 3.14, and site-packages went with it. Neither `uv` nor `pipx` is
present.

Consequences: the hook has failed on **every** commit, so `graphify-out/graph.json` is dated
**2026-08-03** while the branch tips are September; and the graphify-first instruction has been
unsatisfiable for a month while its reminder still prints on every grep.

**Yours to decide** because it installs third-party code on your machine: the PyPI package is
`graphifyy` (not `graphify`) — `uv tool install --upgrade graphifyy` or `pip install graphifyy`,
after which `graphify-out/.graphify_python` must be repointed or the hook will fail the same way.

## `master` is 603 commits behind, and the branches are a chain rather than a fan (2026-09-07)

`master` is at `e65a5673`, dated **2026-07-09**. Every active lane is 559-603 commits ahead of it and
**zero** behind. Two months of engine, particles, Aether, render and UI work lives on unmerged
branches.

The encouraging half, measured pairwise rather than assumed: `fix/inherited-red-gates` and
`feat/aether-v2` are **entirely contained** in `feat/threadpool-ke16` (their-only = 0);
`feat/multi-paradigm-render` differs by 2 commits; `feat/reflection` and `feat/ui-advanced` by 20 and
16. So integration is close to linear, not a merge fan — `master` could fast-forward to the
threadpool tip and pick up 603 commits including the gates and Aether lanes.

This is your release line and the call is yours; it is recorded here because a two-month-old `master`
silently changes what "the shipped code" means in every other document.

## The protector gate's `overlaps >= 1` is a PROCESS-GLOBAL threshold read by two parallel tests (2026-09-07)

Found while reviewing a proposed change to the Miri probe, and it is a finding about the tree rather
than about that change, so it is recorded rather than fixed.

`assert_probe_armed` (`tests/miri_scope_completion_protector.rs:290-303`) computes `overlaps` as a
delta over a window in which the sibling test is also running: the default Miri build of that binary
runs two non-feature-gated `#[test]`s in parallel (`:350-351`, `:793-794`), and the file itself
records that libtest runs them in parallel with process-global counters (`:263-266`).

That file justifies the non-strict comparison for `firings` on the grounds that a concurrent test
"can only ADD" — correct there, because for `firings` the ADD direction is harmless. **For `overlaps`
the ADD direction is the FALSE-GREEN direction**: context X's `overlaps >= 1` can be satisfied
entirely by context Y's genuine overlap, so a per-context claim is not per-context. Today both tests
exercise the same arm with the same shape, so the consequence is mild — but the gate guards a shipped
UB fix, and this is the same shape as the family this repository keeps cataloguing.

Cures: per-context counters, or pinning the Miri gate to `--test-threads=1`. The second additionally
bounds window-slot occupancy but changes the recipe the measured table at `:88-98` was taken under,
invalidating the 4/4 and 3/4 baselines every negative control compares against — so it needs its own
re-measurement pass and is not a free tightening.

A separate, cheaper question is still open and needs no edit to answer: whether the gate's
address-keyed window slots are **already** being satisfied by address reuse. Miri's default same-thread
heap reuse rate is 0.5, every `Box<ScopeShared>` is allocated and freed on one thread, the window key
is address equality alone (`scope.rs:272-275`), and the recipe pins no reuse rate (zero occurrences
of `address-reuse` anywhere in the tree). Sixteen Miri processes over
`-Zmiri-address-reuse-rate` x `-Zmiri-address-reuse-cross-thread-rate` in {0,1} decide it: identical
printed counts mean the defect is latent, any increase at rate 1 means `overlaps=3/4` is partly an
artefact today.

## RESOLVED 2026-09-07 — all four of the day's questions, ruled by the owner

1. **Defect B — DEFERRED to the quiet window, not dropped.** B4-1 lands in the SAME session that has
   a quiet machine for axes W and C, so the re-run it forces — `a3+b4` against `a1f+b1` — happens
   interleaved there, in one session, and the axis-A verdict never sits unproven. Until then `a3+b0`
   ships as it does today, clearing both acceptance clauses.
2. **graphify — INSTALLED.** `graphifyy` into the interpreter the hook already pins; the hook's first
   probe now passes and `graphify-out/.graphify_python` needed no change. Verified by the next commit,
   which launched a background graph rebuild instead of printing the error it had printed on every
   commit for a month.
3. **`master` — FAST-FORWARDED** from `e65a5673` (2026-07-09) to `45dd0dbd`, 611 commits, a true
   fast-forward: `master` held nothing this branch lacks and no worktree had it checked out.
   `feat/reflection` (+20) and `feat/ui-advanced` (+16) are merged separately, later.
4. **The gate's `overlaps >= 1` threshold — PER-CONTEXT COUNTERS.** `--test-threads=1` was rejected
   because it changes the recipe the measured 4/4 and 3/4 table was taken under, and every negative
   control compares against that table. Not yet implemented; it is the next small item after Stage 3b.

## Axis W is decided against `wg`; what remains is a GATE, not a number (2026-09-08)

Three interleaved passes at CS-4 eliminated `wg` (W-b) and `wgc`: zero improvements, thirteen and
fourteen regressions, and — the reading that decides it — an **occupancy receipt**. Under `wg` the
ECS protocol pass reports `max_in_flight` of **5–6 of 16 lanes** with the speedup pinned at
**2.00×** in every pass, against the reference's 16 and 13.9–15.7×. W-b does not add overhead; it
leaves ten lanes parked, because on a wave pushed to one destination only the first two pushes see
`pre_len` of 0 and 1. `KE16-DESIGN-W.md` §2.4 had named this outcome as its own risk clause, on
exactly the cells it fired on.

`wc` (W-d′) is a tie on all 38 cells and preserves 16/16 occupancy. **Step W rule 2 makes its keep
conditional on two gates rather than on any number**: a real-park loom M1c, and the route-(b)
many-seeds Miri gate. Neither has been run at CS-4. So **W\* is undecided between `wc` and `w0`,
and no measurement can decide it** — a red on either gate drops the feature outright.

⇒ **Nothing here needs you.** The two gates are structural work and will be run when the machine is
free; they are not a values call. This entry exists so that a reader who sees "axis W measured" does
not conclude that W\* is fixed.

### ⚠ What IS worth your ruling: the physics ranking, and whether it is worth a session

**No physics verdict was filed, and it cannot be from this data.** The three passes drift
monotonically — the reference's `bench_thread_install` reads 36.24 / 13.36 / 8.12 ms across passes
1, 2, 3, and the PRIMARY row `in_scheduled_system` carries an **85 % band**. Against a band that
wide every arm is a tie by construction, which is a statement about the band and not about the arms.
The machine was still settling from the build when pass 1 ran; the load receipts show 14.8 % CPU at
pass 1's open and 1.4 / 0 / 0.1 % at pass 3's.

**More passes taken the same way would not fix it** — the drift is monotone, so extra passes extend
the trend rather than average it out. A physics ranking needs a session that opens ALREADY SETTLED:
the box idle before the first build, not after it.

**My recommendation is that this is NOT worth a session, and the reason is that it is not on the
critical path.** W\* is a choice between `wc` and `w0`, and rule 2 decides it on the two gates. A
physics ranking would matter only if `wc` were a candidate to keep on performance grounds — it is
not; it is a measured tie. And Step App re-takes every consumer number on the unconditional shipped
code anyway, which is where the acceptance line is formally taken. **Unless you want the record to
carry a CS-4 physics row for its own sake, the next quiet window is better spent on the axis-A
re-judgement that fixing defect B forces** (`a3+b4` vs `a1f+b1`, ruling 1 of 2026-09-07).

### ⚠ Raw data this pass destroyed, recorded because it cannot be recovered

`--save-baseline` is keyed on the variant string, which does not name the code state, and criterion
overwrites in place. Re-taking `a3+b0+w0+c0` at CS-4 overwrote the **CS-2 reference's raw samples for
r1, r2 and r3**; r4 and r5 survive only because the pass stopped at three. The medians, bands and
per-run values are preserved as prose in `KE16-RESULTS.md` §W, so the findings are intact — the
per-sample data behind three of five runs is gone, and criterion baselines are not rebuildable from
anything in the tree. The naming scheme now carries the short HEAD hash (`e7910d5f`), so two code
states can no longer address one directory. **No action is asked of you; it is here because a
document that only records its successes is the thing this campaign keeps finding to be wrong.**

## A measurement RULE was rewritten after it ruled, and that is yours to accept or reject (2026-09-08)

`KE16-DESIGN-MEASUREMENT.md` §7's Step-A rules 2 and 3 are amended, and the amendment is flagged
here rather than merely applied because of when it happened: **rule 3 decided against `a1f+b1`, and
`a1f+b1` was then measured better on ten of thirty-five cells with both consumers tied.** A decision
rule rewritten after it has ruled is the shape that most deserves an owner's eye, so the original
text is preserved verbatim directly beneath the amendment and the change is reversible by deleting
one block.

**What was wrong with the rule, as a fact rather than a preference.** Rule 3's tiebreak gave the
decisive vote to the 64W column. The engine cannot produce that column: `BatchingStrategy::default()`
sets `batches_per_thread = 1`, so an archetype splits into exactly W chunks — one per worker — and
reaching 64W needs `batches_per_thread = 64`, which **no caller in the tree sets**;
`MIN_ARCHETYPE_FOR_PARALLEL = 1024` floors a chunk at 1024 rows, so the 1 µs body the tiebreak fires
on needs a row costing under a nanosecond, against a real ECS chunk of ~82 ms; and the physics
solver does not use that path at all. So one cell at an unreachable width and an unreachable body
size outvoted six mid-body cells at 2–4 × and a tie on both consumers.

**What the amendment says.** Rule 2 becomes CONSUMER AGREEMENT rather than physics primacy — a
candidate leads only if it leads or ties on every consumer, and a disagreement between consumers is
reported as a tie rather than broken by choosing one. Rule 3's veto surface becomes the widths the
engine PRODUCES: `tasks = W` decides, 4W and 64W are recorded and cannot decide, and no single cell
may again override the rest of the grid and both consumers.

**Why it is put to you.** Perf and architecture forks are the orchestrator's to settle, but
rewriting the yardstick mid-campaign is closer to scope. Three things are worth your ruling:

1. **Accept, reject, or narrow the amendment.** Rejecting it does not restore `a3`: §A-RE's reversal
   rests on the measurement, not on the rule — ten cells improved, two regressed, both consumers
   tied, gate ladder green. Rejecting the amendment means the campaign carries a rule that its own
   measurement contradicts, which is a coherent choice only if the rule is then fixed some other way.
2. **`4W` is reachable but unused.** `BatchingStrategy` is public and `par_iter().batching_strategy()`
   accepts it; nothing in the engine calls it. If you expect callers to start tuning that, 4W should
   join the deciding surface and the amendment should say so.
3. **The consumer set is thin.** The pool has four production dispatchers — the scheduler, ECS
   `par_iter`, the physics colour solver, and the MSDF bake — and the campaign measures TWO of them.
   A rule that requires consumer agreement is only as good as the consumers it agrees across. Adding
   the scheduler and the bake as harnesses is a real cost and a real coverage gain; it is your call
   whether the campaign pays it now or records the gap.

## The `Vec` side-store did not come back — Stage 4 of the 2026-07 remediation was never run (2026-09-08)

Asked why physics holds bulk data in `std::Vec` again after the O11-SP4 incident "when we checked
everything and there were no problems". It never came back. It was never removed, and three
structural reasons kept that invisible.

**IT WAS FOUND, NAMED, AND STAGED.** `docs/ARCH-AUDIT-ECS-DATA-REMEDIATION.md` lists
`ConstraintGraph ×8` under **S3** — *"per-step SoA/CSR scratch, ~20 resources / ~90 Vec fields …
only the backing medium (std::Vec → ComponentPool) changes"* — and it is **not** in that document's
"Legit-keep" list. Its remedy is **Stage 4**, *"mechanically swap the S3 per-step `Vec` fields →
`ScratchColumn`"*.

**STAGES 0 AND 1' RAN; STAGE 4 DID NOT.** `ScratchColumn` exists (`scratch_ids.rs`), the three body
mirrors are converted, and `solver/colored.rs:305` documents the address-stable backing. So what was
fixed is **the race**, not **the class** — which is exactly why the memory of it is "we checked and
it was clean": SP4 is closed and its gate is green. Counted 2026-09-08 across
`crates/boyko_physics/src`: **50 fields on `ScratchColumn`, 106 still on `Vec`.** Part of S3 did get
converted (`ra_x`/`ra_y` at `colored.rs:806` are the `ContactColumns` SIMD lanes); `ConstraintGraph`'s
eight did not. The remediation is **partially executed**.

**WHY NOTHING SURFACED IT — three reasons, all structural:**

1. **There is no gate on `Vec`.** `clippy.toml` mechanically bans `HashMap`, `HashSet`, `Mutex`,
   `RwLock`, `Rc`, `RefCell` and their `parking_lot`/`hashbrown` forwards. `std::vec::Vec` is
   absent. Principle 0's "no parallel data system" clause rests on prose alone, while the repo's
   other two make-a-check-disappear mechanisms — `unsafe` and `#[allow]` — each carry a mandatory
   written justification and a census.
2. **Two different "we checked everything" statements read as one.** `clippy.toml`'s header says a
   full classification across 14 crates *"found ZERO violations"* — true, and scoped to the types it
   lists. The architecture audit says *"~90 Vec fields"* in physics — also true, and about a class
   the lint never covered. The first sentence lives in the gate file, sounds absolute, and carries
   the same 2026-07 date as the second.
3. **Neither plan document tracks stage status.** There is no "Stage 1' — done" and no "Stage 4 —
   open" anywhere in `ARCH-AUDIT-ECS-DATA-REMEDIATION.md` or `DENSE-COMPONENTS-PLAN.md`. A
   half-executed plan is textually identical to an unexecuted one.

**WHAT IS PUT TO YOU:**

1. **Does Stage 4 get scheduled, or is S3 accepted as a standing exception?** Either is defensible —
   the pattern (CSR/SoA, cleared-and-refilled, zero-alloc) is already correct and the audit says so;
   what `ScratchColumn` buys is address-stability on grow, which matters only where a raw pointer
   outlives a resize. If it is accepted, it belongs in "Legit-keep" with that reason, not left
   looking like open work.
2. **Should `Vec`-as-a-durable-side-store get a mechanical gate?** It cannot be a blanket
   `disallowed-types` entry — `Vec` is legitimate almost everywhere — so it would have to be a
   census over `Resource`-held structs, in the shape `ignore_reasons_census.rs` and
   `print_census.rs` already use. That is real work and it is the only thing that would stop this
   recurring.
3. **Should the plan documents carry per-stage status?** The cheapest of the three, and it is what
   turned a known backlog into a surprise.

## OPEN 2026-09-09 — three questions the ECS-native physics lane raised and cannot rule itself

Lane: `feat/ecs-native-storage` (worktree `D:/wt/ecsnative`), branched from `master` `7e908c87`.
Nine commits, pushed. Stage 4 stands at **21 of ~60 columns** (`ConstraintGraph` 8,
`BroadphaseGrid` 13). Physics 38 targets / 369 tests / 0 failed; workspace clippy `-D warnings`
exit 0.

### 1. ⚠ Step 7 is blocked by LANE OWNERSHIP, not by difficulty — and it is the whole remaining gap

The Jolt-parity measurement (`benches/jolt_parity_pyramid.rs`, Jolt v5.3.0 built on the same
machine, its `PyramidScene.h` transcribed index for index) leaves the deficit in exactly one place:

| W | Jolt ms | Jolt x | boyko ms | boyko x |
|---|---:|---:|---:|---:|
| 1 | 17.59 | 1.00 | 18.27 | 1.00 |
| 8 | 4.21 | 4.18 | 20.45 | 0.89 |
| 16 | 3.95 | 4.45 | 25.07 | 0.73 |

⚠ **Measured on the pre-A7 contact set** (noted 2026-09-18). Every boyko number here predates A7a,
which gives each clipped face-contact point its own feature id. That changes the pile's contact set
and warm-start keys: on A7-R1's height-15 pile at step 600, 12817 points before it and 14605 after.
A re-run on a tree with A7a measures a different workload, so a difference from these numbers is
not a solver or pool change until the same run on a tree without A7a (`9f712204`, for one) shows it.
A7b (S5, the face-versus-edge rule, the commit after `08fe7b9f`) changes the contact set again and
by more: 22 975 points at the same step, since a resting support now carries its clipped patch
instead of one edge point. The same rule applies to it, with `08fe7b9f` as the tree without A7b.

⚠ **CORRECTED 2026-09-09 evening — the "within 4 %" this paragraph used to claim was an artifact of
comparing across measurement sessions.** The boyko column above (18.27) was measured after the
`BroadphaseGrid` migration; the Jolt column (17.59) was carried over from the earlier head-to-head.
Re-measured back to back in one quiet window, on the current HEAD:

| W | Jolt ms | Jolt × | boyko ms | boyko × | boyko / Jolt |
|---|---:|---:|---:|---:|---:|
| 1 | 16.362 | 1.00 | 18.551 | 1.00 | **1.13×** |
| 2 | 8.955 | 1.83 | 19.348 | 0.96 | 2.16× |
| 4 | 5.530 | 2.96 | 19.397 | 0.96 | 3.51× |
| 8 | 4.109 | 3.98 | 21.218 | 0.87 | 5.16× |
| 16 | 3.901 | 4.19 | 26.091 | 0.71 | 6.69× |

So single-threaded the gap is **13 %, not 4 %** — and the difference between those two figures is
almost entirely JOLT's own number moving (17.59 → 16.36 ms, −7 %, same binary, same scene, same
machine), not boyko regressing (18.27 → 18.55, +1.5 %). **A ratio whose halves come from different
sessions is not a measurement.** Both sides now have to be taken in one window, and the 7 % swing on
an unchanged binary is the resolution any single-thread claim about these two engines has to respect:
quote it as "≈1.1×", never as a tight figure.

⚠ The absolute single-thread ratio is still NOT a solver-quality verdict, for the reason already on
file: the iteration budgets differ (Jolt 10 velocity + 2 position; here 4 substeps × (1 + 2 relax)).
Only T(1)/T(N) compares like with like — and that is where the verdict is unambiguous.

**The entire deficit is dispatch, not solver quality**: Jolt reaches 4.19× while boyko is
NEGATIVE — 0.71× at 16, its best result on ONE worker. The chunk-allocation half is fixed here (`MIN_SLOTS_PER_CHUNK`, −23.5 % at W = 16). What
remains is the per-wave ramp: 4 substeps × (1 + 2 relax) × ~6 colours = **72 waves per step**, each
waking and parking W workers — 1152 events per step at W = 16.

The fix is one `pool.scope` per STEP instead of 72, which needs an in-scope BARRIER on
`boyko_threadpool::Scope` (it has `spawn` and `spawn_batch` and nothing else). The wait logic
already exists inside `Scope::drop` and is extractable.

**But `boyko_threadpool` is the KE16 campaign's system, open in `D:/wt/threadpool`, and the scope
completion protocol is precisely what axes W and C measured.** Editing it from this lane would
collide with that worktree on the same files — the thing the worktree-per-system rule exists to
prevent — and would invalidate KE16's baselines, which are pinned to the current behaviour.

**Question for the owner: sequence the in-scope barrier INTO the KE16 lane (and if so, before or
after Step App?), or close KE16 first and take it afterwards?** This lane cannot answer it, and it
is the difference between 0.89× and something that scales.

### 2. A single global chunking constant is MEASURED not to serve two legitimate scenes

`MIN_SLOTS_PER_CHUNK` was swept on the parity pyramid, W = 8 / 16:

| floor | W8 ms | W16 ms |
|---:|---:|---:|
| 32 | 22.87 | 29.67 |
| 64 | 21.60 | 26.99 |
| 128 | 20.93 | 25.13 |
| 256 | 20.14 | 20.98 |

Monotone and still descending at 256 — so on that scene the answer is "raise it". Raising it to 256
turns `many_disjoint_pairs_are_one_wide_color_and_must_dispatch` RED: that scene is ONE colour of
500 slots, `500 / 256 == 1` chunk, and the single-chunk short-circuit sends it inline, undoing the
1.95× the P2 gate-metric fix had just been measured to deliver.

The two scenes want opposite things and both are ordinary: a pyramid is many NARROW colours whose
cost is per-wave dispatch (wants coarse chunks); a debris pile is ONE WIDE colour whose cost is idle
lanes (wants fine ones). **64 currently ships as the largest value that keeps both gates green — the
optimum of neither.**

This is the concrete, measured case for the cut policy being a per-call-site OBJECT rather than a
global integer. Note that "add more integers" has already been tried and refused by the code base:
`BatchingStrategy` has carried `batches_per_thread`, `min_batch_size` and `max_batch_size` since it
was written and **no production caller passes a non-default** — the only non-default in the tree is
in a test. The distinguishing quantity between call sites is not a number but a FUNCTION: ECS rows
cut by a closed form, colours by CSR slot runs ("data-dependent … not available in closed form",
`colored.rs`), broadphase by a survivor prefix sum, and the SIMD path by `COHORT = 8`, which its own
doc calls "a fixed SIMD-width constant, NOT a perf knob".

**Question: fund a `Cuts` policy object (`Uniform { per_worker, min, max }` | `Explicit(&[u32])`)
plus one occupancy instrument every dispatch reports through — or accept per-scene tuning and say so
in the constants?**

### 3. `par_iter` compile-rejects dense storage, which blocks "one worker pool, all systems"

`const { assert!(!D::HAS_DENSE && !F::HAS_DENSE, …) }` at `par_iter.rs:305`, with the reason given
as plumbing: "the parallel path does not resolve the dense store into each worker chunk's `Fetch`
(the chunk runner has no world cell)".

The exact boundary: **field `RigidBody`, stage `physics_integrate`, site `systems.rs:149`,
contract `par_iter.rs:307`.** The day `RigidBody` becomes `#[component(storage="dense")]` — which is
Stage P's own headline in `DENSE-COMPONENTS-PLAN.md` — that `query.par_iter_mut()` stops compiling.

Until it is fixed, "every system's work is balanced by the same workers through the same seam"
is unreachable by construction, because the storage class Principle 0 prescribes for bulk
per-entity data is the one the parallel driver refuses.

**Question: is resolving the dense store per worker chunk in scope for the kernel now, or does
Stage P's dense migration wait behind it?** It is a kernel change, not a physics one, and it is not
in any current plan.

## RESOLVED 2026-09-09 — the three ECS-native-lane questions, ruled by the owner

1. **The in-scope barrier waits for KE16 to CLOSE.** Not interleaved into that campaign and not
   taken from this lane. Step App and the rule-3 rewrite finish first, KE16 closes, and the barrier
   is then taken in `D:/wt/threadpool` against a settled protocol. The cost is accepted with its
   number: the Jolt deficit stays at 0.89x against 4.18x until then, and it is the whole remaining
   gap. (⚠ That ruling cited "single-threaded the two engines are within 4 %"; re-measured in one
   window the figure is ≈13 %, and the correction is above. It does not change the ruling — the
   scaling ratio, not the single-thread ratio, is what the ruling rests on.)
2. **The `Cuts` policy object is DEFERRED until after the barrier**, on the reasoning that the
   barrier changes the per-dispatch cost the policy is being tuned against — so tuning first would
   calibrate against a cost that is about to move. `MIN_SLOTS_PER_CHUNK = 64` therefore ships as-is,
   documented as the largest value keeping both gates green rather than the optimum of either.
3. **`par_iter`'s dense rejection waits for Stage 4 to finish.** The remaining ~34 columns this lane
   owns are all T2 (`ScratchColumn`), which the const assert does not touch, so Stage 4 is not
   blocked by it. The kernel fix is raised when Stage P's dense migration actually needs it.

Resulting order of work: **finish Stage 4 in this lane → close KE16 → in-scope barrier → re-open
questions 2 and 3 with the barrier's numbers in hand.**

---

## `Scope::spawn_batch` after App-4 lost: the design says do not ship it, and four production sites call it (2026-09-09)

**THE RULE, VERBATIM.** `KE16-DESIGN.md`'s App-4 row states its own kill condition: *"no cell better
than per-task spawn beyond 2× the band — then the API is not shipped (smaller surface wins a tie)"*.
The tournament shipped `c0` — per-task spawn. App-4 lost. By that sentence the API comes out.

**WHAT ACTUALLY SHIPPED.** The BEHAVIOUR is gone and the SURFACE is not. `Scope::spawn_batch`
(`crates/boyko_threadpool/src/scope.rs:1060-1074`) is now:

```rust
let mut k = 0usize;
for f in bodies { self.spawn(f); k += 1; }
debug_assert!(k <= n, "invariant: `spawn_batch` bodies yielded more than the `n` it was promised");
```

Its own doc says so — *"One `spawn` per body: every body is registered and pushed on its own, and
every push takes its own wake decision"*. There is no `fetch_add(n)`, no once-per-wave wake, nothing
App-4 proposed. What remains is a loop plus a debug-asserted upper bound.

**THE STATED REASON TO KEEP IT IS NOT EXERCISED BY ANY CALLER.** The doc justifies the surface as
*"the API exists so that a caller whose chunk count is only an upper bound has one call to make"*.
All four production call sites pass an EXACT count, and all four have the identical shape
`spawn_batch(N, (0..N).map(..))`:

| site | argument |
|---|---|
| `crates/boyko_ecs/src/ecs/core/iters/query/par_iter.rs:406` | `spawn_batch(n_chunks, (0..n_chunks).map(..))` |
| `crates/boyko_ecs/src/ecs/core/iters/query/par_chunk.rs:263` | `spawn_batch(n_chunks, (0..n_chunks).map(..))` |
| `crates/boyko_physics/src/resources.rs:1781` | `spawn_batch(n_waves, (0..n_waves).map(..))` |
| `crates/boyko_physics/src/soft/colored.rs:1042` | `spawn_batch(n_waves, (0..n_waves).map(..))` |

So the `n` is never an upper bound in production; it is the length of the range immediately beside
it, and the `debug_assert!` it feeds cannot fire at any of the four.

**WHAT REMOVAL WOULD COST.** Four production sites become `for chunk in 0..n { scope.spawn(..) }`.
Twelve `#[test]` functions in `scope.rs` go with the API (eleven `spawn_batch_*` plus
`scope_multi_drain_frees_once`), as do the call sites in
`crates/boyko_threadpool/tests/block_allocation_receipts.rs:746,876` and
`crates/boyko_threadpool/tests/miri_scope.rs:634,689` — the latter two are receipts about the
allocator and the release window, not about batching, so they would be rewritten onto `spawn`
rather than deleted.

**WHY THIS IS NOT DECIDED HERE.** Mechanical evaluation of the removed `ke16-c-batch` feature keeps
whatever was outside a `cfg`, and `spawn_batch` was outside one. Removing a public method, rewriting
four production sites and retiring twelve tests is a scope decision, not a cfg evaluation, and the
rule that mandates it was written before the API acquired production callers.

**WHAT IS PUT TO YOU:**

1. **Does `Scope::spawn_batch` come out, as its own design rule says?** The honest reading is yes:
   it is a loop with a promise no caller needs, and "smaller surface wins a tie" was written for
   exactly this outcome.
2. **Or does the rule get amended in place?** Defensible too — the four sites read better with it,
   and the `debug_assert!` is a real invariant for a FUTURE caller whose count is a bound. If so the
   App-4 row must say that the API survives its own rejection and why, because as written the
   document and the tree disagree.
3. **Either way the doc changes.** Today `KE16-DESIGN.md` says the API is not shipped and the API is
   shipped. That is a documented rule contradicted by the tree, which is the shape this campaign has
   spent its whole length removing.

---

## The `a1f` placement reads the defect-A serial floor at 1 µs, the record said 1.36×, and the cause is NOT DETERMINED (2026-09-10)

**WHAT WAS MEASURED (all on one box, one night, bench profile, load receipts, arm witness per run;
`KE16-RESULTS.md` §A-RE replay and §App).** `worker/body_1us_tasks_64W`, 1,024 tasks × 1 µs from
inside a worker: `a3+b0` 99.9 / 100.6 µs; `a1f+b0` 1.184 / 1.148 ms; `a1f+b1` 1.170 ms / 444 µs
(bimodal); unconditional HEAD 1.146–1.200 ms over seven runs. The record (`5863b041`, CS-4) had
80.5 / 110.3 / 109.5 µs — ALL arms parallel — and the same commit with the same features reads the
floor tonight. Excluded on measurement: the Step App removal, the box's compute speed
(`ecs seq/65536` to 0.014 %), timer resolution (probe 756–15,006 µs while the cell held ±4 %).
Excluded on code (Fable pass, refuted and re-verified): H1 joiner-outruns-thieves (10 µs bodies
reach 8–15 lanes in the same process), H2 the FIFO owner/thief CAS storm (`b0` never pops its own
deque, and it is stably serial), H3 a sweep that misses the spawner's stealer, H4 a dropped wake.
Two facts point outside the pool: dispatcher-route 1 µs cells under `a3` vs `a1f` run near-identical
code and read 100 vs 540 µs; and EVERY wake-bound cell is elevated arm-independently tonight
(`a3`'s 1 µs × W 2.4–3.8× its §C values; `empty_schedule_control` 6.3–7.6× its five recorded
sessions) while compute-bound cells reproduce within 1 %.

**THE HYPOTHESIS THAT FITS EVERYTHING, UNTESTED.** A box scheduling-latency state — on a laptop
part (Ryzen 9 5900HS) most plausibly core parking / C-state exit latency — that is (a) present in
every arm, (b) gated by body duration (only wake-bound cells suffer), (c) an order of magnitude
worse for `a1f` than for `a3` (a woken `a1f` thief must sweep 15 stealers to find the one deque;
an `a3` thief batch-steals 32 from the global injector at stage 2, so far fewer wakes must land
in time), and (d) BETTER on a busy box — which is what the CS-4 record's own timeline suggests: the
driver keys the table depends on were committed 09:43:31 and the table written at 10:15, ≤ 31.5 min
for ~35 min of passes, with 214 doc lines committed at 09:43 and a 17.7 KB source file written at
09:54 in the same window (provenance lens, refuted-and-confirmed). A box kept warm by an agent
editing and compiling would have SHORTER wake latencies than a quiet one, which is the direction
the record differs from the replay. This is a hypothesis; it is the only one that explains all of
(a)–(d), and it has not been tested.

**THE DISCRIMINATING TESTS, in cost order (machine must be free; none was run — owner said no
more timings 2026-09-10 00:30):**

1. **Warm-box A/B.** Run `worker/body_1us_tasks_64W` on the shipped binary twice: once quiet, once
   with a one-core low-priority spinner (or the `High performance` power plan / core parking
   disabled) held for the run. If the cell drops from ~1.15 ms toward ~100–400 µs under the warmer,
   the state is the box's wake latency and the CS-4 record was taken on a warm box. ~2 minutes.
2. **`empty_schedule_control` alone.** One criterion run; 1.4–1.8 µs means the 6–7× was session
   state; 9–11 µs on a quiet box with (1) positive confirms the same phenomenon.
3. **Instrumented wave (mechanism lens's test).** `ke16_nested_scope_occupancy.rs` with `BODY = 1 µs`;
   read `pool.parked_mask()` right after the spawn loop and count `Steal::Retry` at
   `worker.rs::drain_one`. `lanes_used == 1` with the 15 sibling bits CLEAR and Retry ≈ 0 ⇒ the
   siblings were woken and did not run in time (box); bits still SET ⇒ a real dropped wake (code);
   `lanes_used > 1` at the floor ⇒ the bodies themselves are serialised.
4. **Cold dispatcher cell.** `dispatcher/body_1us_tasks_64W` alone on the `a1f` binary: ~100 µs
   means the 540 µs was carried-over process state; 540 µs means the binaries differ in a way the
   source does not show.

**TEST 1 RUN 2026-09-10 ~01:30 (owner: "do the A/B") — REFUTED.** `worker/body_1us_tasks_64W` on
the shipped HEAD binary, three runs per state: quiet 1.123 / 1.161 / 1.108 ms; sixteen IDLE-priority
spinners holding every core out of C-states (CPU 100 %) 1.054 / 1.137 / 1.150 ms; one
BELOW-NORMAL spinner 1.067 / 1.155 / 1.137 ms. `empty_schedule_control` 10.18 / 9.38 / 8.93 µs.
The cell does not move with core warmth; the box-wake-latency hypothesis above is dead, and the
"warm CS-4 box" reading of the timeline explains nothing.

**THE ONE VARIABLE STILL UNCONTROLLED IS THE COMPILER.** `~/.rustup/toolchains/stable-x86_64-pc-windows-gnu`
was rewritten at **2026-09-09 02:52:35** (`rustup update` to 1.98.1; `20fc8a66` records the move
from 1.97.1). The CS-4 §A-RE table was produced **2026-09-08 10:15** — every CS-4 binary was built by
rustc **1.97.1**; every binary measured on 09-09/10, including the "same commit, same features"
replay at `5863b041`, was built by **1.98.1**. A source-level audit cannot see a codegen change,
and the tree already met one 1.98 effect at the TLS boundary (`missing_const_for_thread_local`,
63 sites): `worker_lane_for` — the a1f placement predicate — is a `thread_local!` read on every
push and in every join step, and `a3` never takes it. **Test 0 (cheapest decisive, ~5 min +
a ~200 MB download): `rustup toolchain install 1.97.1-x86_64-pc-windows-gnu`, rebuild the pool bench
at `5863b041` with `--features ke16-a1-fifo,ke16-b1,ke16-w-count` under `+1.97.1-…`, run the cell.**
~110 µs ⇒ the record was right for ITS compiler and 1.98.1 regressed the a1f path (a shipped
codegen regression, and a `BLESSED_RUSTC`-class receipt is owed beside every grid);
~1.15 ms ⇒ the compiler is excluded too and only tests 3–4 remain.

**TEST 0 RUN 2026-09-10 ~02:00 (owner: "install it and run") — THE CAUSE IS THE COMPILER.** Same
commit `5863b041`, same features, same box, arm witness printed, taken on a box the owner had just
called NOT quiet (park probes 7.8–15.5 ms; a noisy box inflates a reading, it does not deflate one):

| build | `worker/body_1us_tasks_64W` | `worker/body_10us_tasks_4W` |
|---|---:|---:|
| `a1f+b1+wc+c0`, **rustc 1.97.1** | **99.3 / 105.0 / 132.7 µs** | 65.8 / 138.4 / 76.3 µs |
| `a1f+b1+wc+c0`, rustc 1.98.1 (this session, 9 runs) | 1,170–1,215 µs | 72.5–74.7 µs |
| `a3+b0+wc+c0`, rustc 1.97.1 | 165.8 / 128.1 µs | 182.8 / 157.9 µs |
| `a3+b0+wc+c0`, rustc 1.98.1 (this session) | 99.9 / 100.6 µs | 175.7 / 178.9 µs |

The record's 109.5 µs reproduces under the compiler that produced it. **rustc 1.98.1 slows the
`a1f` placement path ~11× at 1 µs bodies and leaves `a3` alone**; the deciding cell is unaffected
(0.4–0.5 either way, so the axis-A verdict stands under both compilers). `~/.rustup/toolchains/stable-…-gnu`
was rewritten 2026-09-09 02:52; every shipped binary since is 1.98.1. **This is a shipped codegen
regression on the pool's fine-granularity path, and it is the answer to the 10× question.** Both
earlier causal stories in this entry (wake latency, warm box) were wrong and are struck above.

**MECHANISM FOUND 2026-09-10 ~03:00, IN THE ASSEMBLY — `docs/threadpool/RUSTC-198-WINDOWS-GNU-TLS.md`.**
Under rustc ≥ 1.98.0 on `x86_64-pc-windows-gnu` every `thread_local!` read calls
`std::sys::thread_local::guard::windows::enable()` = `lock inc [ACTIVE_ENABLE_CALLS]` + `lock or [rsp],0`
+ `kernel32!FlsSetValue` + `lock dec` — two contended RMWs on one process-global line plus a Win32 call
per access (rust-lang/rust #148799 + #157483, both milestone 1.98.0; `target_thread_local` is unset on
windows-gnu, so the per-thread memoisation is compiled out and `const {}` does not help). The `a1f`
path pays it twice per spawn (`worker_lane_for`) and twice per victim in every thief sweep (crossbeam-epoch
pin before the `len` check); `a3`'s paths carry zero TLS reads and its `Injector` steal never pins.
Everything else in the two binaries is relocation-identical. The bracket below is answered by the
sources (1.98.0); what remains owed is the falsification run in that file's §5 and the two local fixes
in its §4(b), neither before the owner frees the box.

~~**NEXT (no runs until the owner says the box is free):** (i) bracket it — 1.98.0 is installed~~ (superseded) (i) bracket it — 1.98.0 is installed
(`~/.rustup/toolchains/1.98.0-…`, 2026-09-02): one build + one run says whether the regression is
1.98.0 or the 1.98.1 point release; (ii) name the function — `cargo asm` / `objdump -d` diff of
`push_on_lane_no_wake`, `worker_lane_for` (a `thread_local!` read on every push; 1.98 already
misfires `missing_const_for_thread_local` on this tree's 63 `const {}` sites, so TLS codegen on
windows-gnu is the first suspect), `try_steal_random` and crossbeam's `steal_batch_and_pop` between
the two builds; (iii) a `BLESSED_RUSTC`-class receipt beside every published grid — the compiler
that produced the bytes, not the channel name — because a load receipt cannot see this either.

**TWO GAPS THIS EXPOSES, EITHER WAY THE TESTS FALL:**

* **`a1` (LIFO owner end) + `b1` was never measured.** `a1` was eliminated at CS-1 on physics in
  the one session whose own machine witness FAILED (`KE16-REJECTED.md`: `O5` moved 16.84 % between
  its two runs, `bench_thread_install` 45 %), before `b1` existed; its return condition is *"a
  session whose `O5` spread is ≤ 1 %"*. LIFO is the end discipline `KE16-DESIGN-A.md` §1.4 names as
  the one whose ends "meet only on the last element". If test 3 shows thieves ARRIVING and LOSING,
  `a1+b1` is the arm to measure next; if it shows them not arriving, LIFO does not help and the
  answer is in wake policy (axis W was closed on `wc` = every push wakes one).
* **The physics consumer runs at the collapsed shape.** 96 = 6W chunks per colour, 1–10 µs bodies,
  from a worker (`KE16-DESIGN-MEASUREMENT.md` §7 correction). Its per-arm numbers were never
  re-taken after CS-4, and tonight's HEAD physics is 1.44× the record with no meter to normalise by.

**WHAT IS PUT TO YOU:**

1. **Run test 1 when the box is free?** Two minutes, no code, and it decides whether the 11× is a
   property of the shipped pool or of a quiet laptop. If the box property is real, the shipped
   configuration is the one most punished by it, and that is a design fact about `a1f`, not noise.
2. **If the box is the cause: does the record keep numbers taken on a warm box?** The CS-4 table
   would then be neither wrong nor reproducible — it would be a measurement of a state the protocol
   did not control. The protocol's load receipt records processes and CPU %, not wake latency; a
   `park_timeout` probe was added for App-12 and swings 20× on its own. A wake-latency receipt (the
   `empty_schedule_control` row, or `dispatcher/body_1us_tasks_W`, both arm-independent) beside
   every published grid would make the next such divergence visible in the table instead of a year
   later.
3. **Does `a1+b1` get measured?** It is the only untried combination of a shipped joiner with the
   end discipline the design itself prefers at fine granularity.

---

## RETRACTED the same day — the Miri protector gate's arming is NOT seed-dependent; the six-seed receipt was taken under a reduced recipe (2026-09-10)

While gating the TLS fix, `crates/boyko_threadpool/tests/miri_scope_completion_protector.rs` went red
at `:454` ("the probe fired 2 times but NOT ONE of 2 frees landed inside a release window") under the
default seed. A control run on HEAD passed with `overlaps=1/2` — one overlap of margin. Six seeds under
the project's `-Zmiri-tree-borrows` (an environment `MIRIFLAGS` REPLACES the config's; the first sweep
was run without the flag by mistake and produced Stacked-Borrows noise in crossbeam-epoch, identical on
HEAD): **fix tree 5 of 6 green; HEAD 0 of 6 green** (`overlaps=0/2` on four seeds, other contexts red
on the rest). The fix does not cause the red; the gate's arming was already a coin flip whose default
seed happened to land. The test's own message names the remedy — re-tune `MIRI_RELEASE_PROBE_YIELDS`
against the observation — and that is owed as its own change, with the seed sweep as the receipt
(`scratchpad/miri_seeds_tb.log`, reproduced by the commands in this entry).

~~**WHAT IS PUT TO YOU:** a Miri gate whose armed-ness flips with the seed is not a gate; retune it, or
make the arming assert print all six seeds and require a majority.~~

**RETRACTED 2026-09-10, nothing is put to you.** The receipt above is invalid as evidence about the
constant: `miri_seeds_tb.log` was taken with `MIRIFLAGS="-Zmiri-tree-borrows -Zmiri-seed=N"`, which
drops four flags the test header declares mandatory (`-Zmiri-disable-isolation`
`-Zmiri-permissive-provenance -Zmiri-ignore-leaks -Zmiri-preemption-rate=0`) — an environment
`MIRIFLAGS` REPLACES the config's, and this entry knew that and still spelled only two. Without
`-Zmiri-preemption-rate=0` the run measures the preemption RNG, and the gate's own monitor said so:
the reduced runs print `block_overlaps < overlaps`, the inversion `assert_probe_armed` documents as the
signature of that missing flag. Under the documented recipe, measured on this branch: **every value
16..96 of `MIRI_RELEASE_PROBE_YIELDS` arms on all six seeds** (36 runs), 16 additionally confirmed on
seeds 0, 7, 8, 9, 10 — 11 seeds, 0 disarmed — and the gate still reds 3/3 by mutation (`complete_task`
receiver back to `&Self`), on the protector property and not on its own arming check. The constant
stays **16**; the sweep table, the recipe A/B and the mutation receipt are recorded on the constant's
doc in `crates/boyko_threadpool/src/scope.rs`, and the test header now carries the full-recipe seed
loop. The "fix tree 5 of 6, HEAD 0 of 6" numbers say nothing about the TLS merge; the merge was gated
separately (`tls_lane_merge.rs`, Miri Tree Borrows under the full recipe). The mechanism to keep: a
Miri receipt whose flag set is not spelled in full is a receipt about one binary's RNG stream.

## MEASURED 2026-09-10 — Jolt head-to-head with the KE16 winner: boyko's scaling is POSITIVE now (was negative), the gap at 8 workers fell from 5.2× to ≈2.5×, and the residual is as consistent with ~45 % serial work as with dispatch

**Why this entry exists.** Every boyko column in the 2026-09-09 head-to-head was taken on a tree that did
NOT contain KE16 — `feat/ecs-native-storage` carried the parity harness but not the pool (checked:
`67563d3b`, `e6115223`, `be1bbbfd` were not ancestors; `tls.rs` still held two slots). This merge put the
harness and the shipped pool in one tree, and BOTH engines were then measured in ONE window.

**Protocol.** Jolt v5.3.0 `PerformanceTest.exe` (built here, MinGW, LTO) and
`crates/boyko_physics/benches/jolt_parity_pyramid.rs` — the same 1240-body pyramid transcribed index for
index — interleaved BY W (Jolt W=1, boyko W=1, Jolt W=2, …), three passes, every bench binary built before
the window, a load receipt before and after every timed region, `rustc 1.98.1 (48a229cea)`,
`available_parallelism = 16` (SMT on 8 physical cores — W=16 is not 16 cores, for either engine). Pass 1
opened on three consecutive clean 20 s polls and is the cleanest; its W=16 pair carried a build process in
one receipt and is marked DIRTY.

| W | Jolt ms | Jolt T1/TW | boyko ms | boyko T1/TW | boyko/Jolt |
|---|---:|---:|---:|---:|---:|
| 1 | 19.799 | 1.00 | 20.311 | 1.00 | **1.01** |
| 2 | 13.273 | 1.60 | 15.290 | 1.33 | 1.15 |
| 4 | 7.406 | 2.86 | 12.361 | 1.65 | 1.73 |
| 8 | 4.966 | 3.99 | 11.931 | **1.71** | **2.47** |
| 16 | 5.037 | 3.93 | 13.237 | 1.54 | 2.38 |

Medians of three passes. Pass 1 (cleanest) boyko scaling: 1.00 / 1.23 / 1.73 / **1.98** / 1.89.

⚠ **Measured on the pre-A7 contact set** (noted 2026-09-18). Every boyko number here predates A7a,
which gives each clipped face-contact point its own feature id. That changes the pile's contact set
and warm-start keys: on A7-R1's height-15 pile at step 600, 12817 points before it and 14605 after.
A re-run on a tree with A7a measures a different workload, so a difference from these numbers is
not a solver or pool change until the same run on a tree without A7a (`9f712204`, for one) shows it.
A7b (S5, the face-versus-edge rule, the commit after `08fe7b9f`) changes the contact set again and
by more: 22 975 points at the same step, since a resting support now carries its clipped patch
instead of one edge point. The same rule applies to it, with `08fe7b9f` as the tree without A7b.

⚠ **Four corrections, noted 2026-09-19** (the physics perf campaign, `docs/physics/perf-campaign/`:
the tree report's C1–C3 and the plan's H1). Every number in this entry is labelled **"pre-A7, likely
gnu"** and is never compared with a row from the fixed-window runner (`MEASUREMENT-QUEUE.md` §10).
Line citations in C1 and H1 are to files under `crates/boyko_physics/` at `1c31aeac`, the tree the
campaign was cut from and the one that still holds the Criterion bench; the campaign replaced the
bench with the runner and shifted `systems.rs`. The bench statements cited are unchanged since this
entry's own `ca582e72`, where they sit 16 lines higher: the one commit between the two, `d11962a9`,
inserted the opt-in mimalloc arm above them.

- **C1: the broadphase reason given below is wrong; the conclusion stands.** The bench never sets
  `broadphase`, so it ran the default `AllPairs` (`1c31aeac:resources.rs:451`), an O(n²) serial double
  loop at every W (`1c31aeac:systems.rs:304-322`). This file's 2026-09-09 Stage 4 entry already says
  so. The bench's `parallel_broadphase = workers > 1` (`1c31aeac:jolt_parity_pyramid.rs:216`) is read
  only on the `Grid` arm (`1c31aeac:systems.rs:331`), so it did nothing. `MIN_PARALLEL_BODIES = 4096`
  (`1c31aeac:resources.rs:664`, tested at `1c31aeac:resources.rs:1734`) gates the Grid path's parallel emit, which was
  never reached. The same goes for note (3) below: the W>1 arm turned on `parallel_solve` and nothing
  else. The all-pairs pass is about 0.9 ms at 1240 bodies (the 2026-09-09 entry, derived from the
  `broadphase` bench, not timed in the step), so about 4.4 % of T(1) and 7.5 % of T(8) here.
- **C2: gnu, not msvc. boyko against Jolt at W>1 has never been measured on msvc.** The entry records
  `rustc 1.98.1 (48a229cea)` without a host. The same day's allocator entry below, same harness,
  records `x86_64-pc-windows-gnu`. The commit that moved the tree's recipes to the msvc host,
  `97bcf826`, is not an ancestor of this entry's commit `ca582e72`. Two pool commits whose effect
  depends on the host are ancestors: `e6115223` (one `thread_local` read per spawn instead of two, a
  gnu cost) and `51631371` (the empty-victim steal gate, a gnu-only win and an msvc-only 2.2× loss,
  since then a `cfg!` constant). So the timed pool ran gnu's side of that gate, and no number here
  describes the msvc build at W>1.
- **C3: the current ratio is unmeasured.** On top of the pre-A7 note above: on this bench's scene, S5
  (A7b) alone cost +10.3 % at W=1 and +7.1 % at W=4 (`MEASUREMENT-QUEUE.md` §9, `full_step/1` and
  `full_step/4`, msvc, taken on the window H1 describes). A boyko/Jolt ratio on a tree with A7 is a new
  row, not an update of this one.
- **H1: the timed window is broken.** For each W the bench builds a fresh world, runs 20 warm steps
  (`1c31aeac:jolt_parity_pyramid.rs:247`), then lets Criterion sample by time on that world without
  resetting it (`:250`). Which stretch of the collapse gets timed therefore depends on how fast the
  steps run: on W, on the pass, on the box's load. The allocator entry below saw arms run 420 and 630
  iterations and sample different windows. So T(1)/T(W) compares different stretches of the
  simulation, while Jolt times steps 0..500 from t=0 (v5.3.0 `PerformanceTest/PerformanceTest.cpp:90`,
  timed loop at `:368`). The fixed-window runner replaces it: 500 steps from spawn, one `Instant` pair
  per step, no warm-up.

The per-stage measurement this entry says is owed (below) is queued as `MEASUREMENT-QUEUE.md` §10. It
uses the kernel's own profiler rather than one `Instant` per system (the plan's Decision 1).

**What changed, drift-free.** The ratio boyko/Jolt is taken back to back within seconds and does not depend
on the box's state: **2.16 → 1.15** (W=2), **3.51 → 1.73** (W=4), **5.16 → 2.47** (W=8), **6.69 → 2.38**
(W=16) against 2026-09-09, reproducing in all three passes. boyko's own scaling went from
0.96 / 0.96 / 0.87 / 0.71 (more workers = slower at every W) to 1.33 / 1.65 / 1.71 / 1.54. Single-threaded
the two engines are indistinguishable (1.01 / 1.03 / 0.96 across passes — quote as ≈1.0×, never tighter).

⚠ **What must NOT be read from it.** (1) The absolute milliseconds are not comparable across sessions: the
UNCHANGED Jolt binary is 1.21–1.48× slower today than on 2026-09-09 at every W, so the box moved. (2) A
pre-KE16 build of the SAME harness was not run today; "KE16's contribution" is attribution by content — the
only W-varying machinery in this harness is the coloured solve's `pool.scope` waves, and KE16 is what changed
their cost — not an A/B in one window. If the number is to be quoted as KE16's, that A/B is owed.
(3) T(1) runs boyko's NON-parallel path (`parallel_solve = parallel_broadphase = (workers > 1)`), so
T(1)/T(W) is serial-vs-parallel, which is the honest speed-up and also the reason the 1→2 step includes the
cost of turning the machinery on.

**The recorded mechanism was an upper bound, not a count.** "72 waves per step" assumed every colour
dispatches. From the source: 12 `solve_all_colors` passes per step (4 substeps × (1+2) relax,
`crates/boyko_physics/src/resources.rs:497-498`,
`crates/boyko_physics/src/solver/colored.rs:3559/3582/3616`), and a colour opens a `pool.scope`
only if `color_slots >= MIN_PARALLEL_SLOTS_PER_COLOR = 256` AND `n_chunks >= 2`
(`crates/boyko_physics/src/solver/colored.rs:2880-2899`, `:2928-2945`; these five anchors were
re-derived from the current source at the A5.1 merge on 2026-09-21 — the widened anchors gate
bound the bare range to the bench file cited above and reported it past that file's end). Both
gates depend on SLOTS, not on W — so the **wave COUNT is W-independent**; only the
wake/park EVENTS per wave scale with W, which is exactly the term KE16 cheapened. Waves were not counted on
the device because physics has no env-var diagnostic and the counters are private `fn` — counting them means
editing the crate under test.

**The next fix is not yet earned, and the adjudicator is right.** "Dispatch remains the deficit" is
inferred from boyko/Jolt growing with W — but Amdahl predicts that growth too. The curve saturates
(1.65 → 1.71 from W=4 to W=8), and solving Amdahl at W=8 gives a **serial fraction of 0.43–0.53** of the
serial step if the parallel part were perfect. The serial candidates are real and large: `physics_narrowphase`
(`systems.rs:369`) is a plain serial system building box-box manifolds for every pair; the broadphase is
serial at 1240 bodies (`MIN_PARALLEL_BODIES = 4096`); and this file's own 2026-09-09 entry records the pyramid
as "many NARROW colours" — colours under 256 slots solve INLINE, so part of the solve itself is serial. And
`MIN_SLOTS_PER_CHUNK = 64` caps chunks at slots/64: a ~500-slot colour yields 9 chunks, so 8 lanes get a
straggler round and 16 lanes have 7 idle.

**WHAT IS OWED before any fix — one measurement, no wave histogram needed:** per-stage wall time at W=1 vs
W=8 (narrowphase / broadphase / solve / inline-colour share), one `Instant` per system. If narrowphase +
broadphase + inline colours ≈ 40–50 % of T(1), the fix is parallelising the narrowphase (Jolt does) and
lowering the per-colour gate / chunk quantisation — not further pool work, and not the in-scope barrier
alone. If the solve by itself still scales < 2× at W=8, then it is per-wave cost and the barrier is the tool.
Secondary: `[profile.bench]` is `lto = false`, so none of these absolutes is the shipped binary.

## MEASURED 2026-09-10 — the system heap against mimalloc on the Jolt pyramid: no effect at any worker count, and the 2671-per-step figure that motivated it came from a pool without Stage 3b

**Why this was measured.** A per-frame allocation census of the 1240-body pyramid, taken on
`feat/ecs-native-storage` @ `ad0ebea4`, counted 2671 heap acquisitions per parallel step — one cell per
spawned task — and 0 on the physics data path. Two mechanisms could make that cost time at W=8: (a) the
malloc/free instructions themselves, or (b) cross-thread frees, because the spawner allocates and the
stealer frees. mimalloc is built for (b), so swapping the global allocator bounds both.

**Protocol.** Two bench executables built before the window and invoked by path, so no cargo ran inside
it: `jolt_parity_pyramid` without and with the `bench-alloc` feature (this commit). Disassembly proves
the arms: in the first `__rust_alloc` jumps to `__rdl_alloc` (the Windows process heap); in the second
to `mi_malloc_aligned`, and `__rust_dealloc` to `mi_free`. Load receipt before and after every region,
arms interleaved per W, four passes, then the adjudicator's own W=8 cell with the arm order reversed.
`rustc 1.98.1 (48a229cea)`, `x86_64-pc-windows-gnu`, Ryzen 9 5900HS, 8 cores / 16 threads.

| W | system ms | mimalloc ms | mimalloc / system | band (pass to pass) |
|---|---:|---:|---:|---:|
| 1 | 19.958 | 19.933 | **0.998** | ±0.3 % |
| 8 | 11.152 | 11.407 | **1.023** | ±5 % |
| 16 | 12.347 | 12.335 | **0.992** | ±2.6 % |

Medians of criterion's slope over the four clean passes. The reversed-order W=8 cell gave 1.005 on the
slope and 0.974 on the median of samples, so the 2.3 % at W=8 changes sign with the arm order: it is
noise inside the band. Scaling is the same in both arms (T1/T8 = 1.79 and 1.75, T1/T16 = 1.62 in both).

**Verdict.** Swapping the global allocator changes nothing measurable at any W. Mechanism (b) is not
supported; mechanism (a), estimated at a few tenths of a percent, is below the resolution.

⚠ **Why the A/B could not have seen (b), found by the adjudicator.** The census and the A/B ran on
different thread pools. `d51b4ced` ("stage 3b lands the per-scope block") is an ancestor of this tree
and NOT of `ad0ebea4`: on the census tree `task.rs` still allocated one cell per spawn and `block.rs`
was wired to nothing; on this tree scoped task cells are emplaced in the per-scope `ScopeBlock`. A
probe linked against the timed binaries' own rlibs, running the bench's scene, measured per step:

| W | allocations (max) | bytes | cross-thread frees | reallocs |
|---|---:|---:|---:|---:|
| 1 | 2.1 (3) | 4.5 KB | 0.1 | 0 |
| 8 | 331.6 (339) | 1.27 MB | 0.1 | 0 |
| 16 | 331.6 (339) | 1.27 MB | 0.1 | 0 |

So the 2671 is a property of the pre-3b pool, not of the shipped one, and the stealer-freed cells that
(b) needed did not exist in the binaries that were timed. Bytes went UP (856 KB to 1.27 MB) because each
scope takes at least one 4 KiB chunk, allocated and freed on its own thread and not reused across scopes.
The same class of error as the 2026-09-10 Jolt entry above — a measurement taken on a branch that did not
contain the code the conclusion was about — and it is recorded as such, not smoothed over.

**What this does and does not establish.**
- It bounds the shipped pool's allocator cost at W=8 by the band (±5 %), with an estimate of 0.3–0.6 %.
- It says nothing about a Linux glibc arm, which was not measured.
- It is NOT the pre-3b vs post-3b comparison. That A/B, owed since the Jolt entry above as "KE16's
  contribution", is still owed.
- A confounder both arms share: the pyramid world is never reset and warm-up is time-based, so arms that
  ran 420 against 630 iterations sampled different simulated-time windows. Where the counts matched
  (two passes at W=16) the ratios were 0.987 and 0.991, the same answer.
- W=16 is slower than W=8 in both arms, so running 16 workers on 8 physical cores costs the same whatever
  the allocator. That regression remains its own open question.

**Consequences.** The Jolt next-fix ordering in the entry above is unchanged: the per-stage wall time at
W=1 against W=8 (narrowphase, broadphase, solve, the inline-colour share) is still owed before any fix.
Removing the remaining ~330 dispatch allocations per step goes ahead under the owner's 2026-09-10 ruling
(no allocator but the engine's own; every runtime structure onto the engine's storage, as close to the
ECS paradigm as possible), and it is justified by that ruling, not as a speed fix.

The `bench-alloc` feature is kept, off by default and in the same form as `boyko_ecs`'s, so this A/B
can be re-run (for the glibc arm, or on the pre-3b tree). The raw criterion data and receipts live
outside the repository and are not reproduced here.

---

## The msvc lane's rotted citations are repaired BY CONTENT - 166 of them, 69 refused, and the count was never 219 (2026-09-10; pass 2 the same day: 208 written, 19 refused, and pass 1's own clean-audit was wrong)

`97bcf826` left one finding open rather than half-repairing it: citations elsewhere in the corpus
that now point at lines the lane moved. That call was right, and this entry closes the part of it
that can be closed without guessing.

**The number in that commit message is wrong.** It says *219 citations*. The tester's own saved run
ends `229 rotted citation(s)`, and a re-baselined re-run of the same script reproduces **229**
byte-for-byte. A transcription slip, not a change of state: the contributor table is right
(`device.rs` 91, `PINS.toml` 64, `golden.ps1` 32, `loom_pool.rs` 18, frozen `docs/ru/` 5) and the
"and a tail" is what dropped `.cargo/config.toml` 10.

**229 is also not 229 defects.** 24 of them are the scanner's own false positives - its regex is an
unanchored basename, so `rhi_impl/device.rs:1619-1621` and `crates/boyko_rhi/src/device.rs:891-903` <!-- doc-anchor-ignore -->
are both read as `device.rs:NNN` and attributed to the lane's `boyko_rhi_vulkan/src/device.rs`. The
tree holds **four** `device.rs` (43 / 1223 / 2584 / 4212 lines); the 2584-line one is not lane-
modified and its citations cannot have rotted. Conversely the same regex UNDER-counted by ~80: it
never matched a bare `` `:NNN` `` continuation nor a comma member past the first, so
`` `golden.ps1:201,226,232` `` counted as one rot and the `` `:226` `` beside it as none - precisely <!-- doc-anchor-ignore -->
the forms the lane named as the reason a regex rewrite would corrupt things silently.

**What was written: 166 citation tokens / 238 line numbers, across 26 documents.** Repaired by
CONTENT, never by arithmetic on line numbers. For each cited `F:N`, the old file's line N is taken
verbatim and must occur in the new file **exactly once** (99 tokens); failing that, the five-line
window `old[N-2..N+2]` must occur exactly once as a contiguous block (67 tokens - still pure content
matching, but the pin is the neighbourhood rather than the line, so it is counted apart). 38
citations already pointed at the right content and were left alone.

The result is checked, not asserted. After the write, every one of the 238 numbers satisfies
`old_F[N] == new_F[M]` as an exact string comparison. `difflib`'s independent opcode map disagrees
with the content method on **zero** of them. ~~The diff is +113/-113~~ **[corrected 2026-09-10, pass 2: `git diff --numstat` said +216/-113 at that point. +113/-113 is the ~~25~~ **26** citation files *[pass 3: 26, not 25 - this file is one of them; its lines 1954, 2754, 4189, 4209 and 4989 are pass-1 rewrites, 5 of the 113]*; the other +103/-0 is THIS ENTRY, so `docs/OPEN-QUESTIONS.md` DID change its line count. It moved nothing: no citation in the corpus points at `OPEN-QUESTIONS.md` past line 5166, measured.]** The 113 rewritten lines have **zero lines differing in
anything but digits**; CRLF is preserved in all 26 files and 25 of them kept their line count (~~not one changed~~), so no
other citation in the corpus moved as a side effect. The two files that are both citation targets
and citing files (`scripts/golden.ps1`, `docs/threadpool/KE16-RESULTS.md`) were checked for the
obvious tail-eating case: of the five lines edited in them, **none** is cited from anywhere.

**Left untouched - 69 tokens - for the lane's own reason.**

- **`ambiguous-path` 50.** *[pass 2: 8 resolved by a sibling, 23 shown rot-free either way, 19 remain.]* Bare `device.rs:NNN` against four candidates. A verbatim-overlap probe
  decides only 6, and 3 of those name `rhi_impl/device.rs`, which the lane never touched. 44 are
  undecided and stay that way.
- **`non-unique-in-new` 18.** *[pass 2: all 18 resolved - the line TEXT repeats, the difflib EQUAL BLOCK holding it does not.]* `PINS.toml` section headers and `loom_pool.rs` idioms that repeat 3-65
  times with no unique window. This is exactly where a guess corrupts silently.
- **`line-edited-or-deleted` 1** - `VG-R3-P3-CULL-INTEGRATION-PLAN.md:988`. *[pass 2: resolved - the range's END line is one the lane ~~deleted; clipped to the last surviving line~~.]* **[pass 3: REPLACED, not deleted - same key, `gnu` -> `msvc`; the end now names the replacement line.]** <!-- doc-anchor-ignore -->
- **frozen `docs/ru/` 5** - reported, excluded by the 2026-09-07 freeze, not repaired. The freeze was
  taken with divergence as its accepted cost; this is 5 units of that cost, now measured.

**The inventory script CANNOT show this repair, and re-running it makes the number go UP.** After
the write it prints **237**, and the decomposition is the whole explanation: **153 findings gone**
(the old numbers), **161 findings new** - the SAME sites, now carrying the corrected numbers - and a
net **+8** that is *this entry*, which quotes stale citations as evidence. The script is not failing.
It reads a cited number N as an OLD line number and asks `old_to_new[N] != N`; a repaired citation
holds a NEW number, and a new number sits in the moved region exactly as the wrong one did, so the
check fires just as loudly when the citation is RIGHT as when it was wrong. It is a sound rot
DETECTOR and a useless rot METER: re-running it is the correct reproduction step and the wrong
verification step, and anyone who reports "the count fell" from it is reporting an artefact. The five
lines above that quote citations as examples now carry the corpus's own
`<!-- doc-anchor-ignore -->` marker, and the detector honours it under `--honor-ignore`, which is
off by default so the lane's original run still reproduces byte-for-byte. With the flag the two runs
are **229 before and 229 after - exactly invariant**: 153 findings leave and 153 return at the same
sites carrying the corrected numbers. That the number cannot move under a repair is now measured,
not argued.

The measurable property is the content one, over the same citation population: **citations whose
cited line does not hold the intended content fell 268 -> 32**, 4 of the 32 being the frozen
`docs/ru/` side. That meter is `w2_verify.py`, written because "re-run the scanner" would otherwise
read as a gate that cannot pass - the failure shape this register has catalogued a dozen times.

**One defect of my own, found by reading my own output.** The repair models four citation forms -
`F:N`, `F:a-b`, `F:a,b,c` and the backticked continuation `` `:NNN` ``. A **fifth** exists:
a slash-separated list, `` `PINS.toml:411/474/499/528/556` ``. The regex matched its first member <!-- doc-anchor-ignore -->
only, so the first pass rewrote `411`->`429` and left the other four at their pre-lane values - a
list mixing new and old numbering with nothing marking which half is which, strictly worse than
leaving it alone. Found by reading the applied diff line by line, then confirmed by a census of the
character following every citation token. Repaired to `429/492/517/546/574`, cross-checked against
the same five numbers resolved independently at line 201 of the same document, where they appear in
the comma form; the two derivations agree. **The `/` is not generally a separator here** - of its
three occurrences, one divides two different files (`device.rs:1883/ffi.rs:2133`) and one precedes a <!-- doc-anchor-ignore -->
`:`-prefixed continuation (`device.rs:2114/:2132`) - so no blanket `/` rule was adopted; it would be <!-- doc-anchor-ignore -->
wrong on two of three. ~~An audit for any remaining partial-state citation returns clean.~~ **[RETRACTED 2026-09-10, pass 2.** The audit looked at the character AFTER a token and at unrewritten tokens on rewritten lines. It could not see a REFUSED token beside a rewritten one for the same target on the same line, nor a `` `:NNN` `` on the NEXT line inheriting a rewritten citation's file, nor a comma-SPACE list, and it returned clean over **15 mixed clusters**. Pass 2 below.]

**The anchors gate covers none of this.** `tests/internal_docs_anchors.rs` has
`GATED_DOCS = ["FEATURE_MAP.md", "SYSTEMS.md", "ARCHITECTURE.md", "MESHLET-VIRTUAL-GEOMETRY-PLAN.md"]`,
and not one of the 26 files edited here - nor of the 29 that still carry an unresolved citation - is
among them. *[pass 7: population-limited. `MESHLET-VIRTUAL-GEOMETRY-PLAN.md` is among them, and it carries two lane-shifted citations that no pass's scanner could read: `:2517`'s link-form `PINS.toml` 372 and 417. The gate reads both and passes both, because both carry the `~` waiver, which checks only that the line is inside the file - Pass 7, (2)]* Everything above was invisible to `cargo test` before the repair and is invisible after <!-- doc-anchor-ignore -->
it. Nothing in this entry is gated by anything.

**Scripts, all stdlib, all re-runnable, in the session scratchpad**
(`.../dad9a19e-5512-4bbd-9976-cb44400eeb64/scratchpad/`): `w2_scan_committed.py` (the detector,
re-baselined off `e6115223..HEAD` because the lane is now committed and the original
`w2_scan.py` dies on a clean worktree), `w2_mapper.py` (proposal + evidence), `w2_apply.py` (the
write, CRLF-safe), `w2_fix_slash.py`, `w2_forms2.py` (the form census), `w2_partial_audit.py`,
`w2_selfref.py`, `w2_verify.py` (the content meter).

**WHAT IS PUT TO YOU:** the ~~44~~ **19** still-undecided ambiguous citations *[pass 2 cut 44 to 19; the rest is below]* (18 of the 19 cite a bare
`device.rs`; the one that does not cites `05-LADDER-GATES.md`, which exists three times under
`docs/diagnostics/`) are undecidable from the text alone -
a bare basename with four candidate files is not a citation, it is a hint. Either they get a path
prefix as they are next edited (cheap, incremental, never verified), or `GATED_DOCS` widens to the
plan corpus and the gate starts failing on them (expensive once, honest thereafter). The second is
the only one that makes the next lane's rot visible on the day it happens.

### Pass 2 (2026-09-10, same day): the 42 citations pass 1 left half-done, and the three claims above that it got wrong

The verifier's verdict on pass 1 was **NOT SOUND** - and not one written line was wrong. 234/234
numbers hold `old_F[N] == new_F[M]`, zero unlisted rewrites, CRLF clean, docs/ru untouched. What was
wrong is that the pass rewrote one token of a citation, refused the token beside it for the SAME
target, and then certified the corpus clean: a line reading half new numbering and half old, with
nothing marking which half is which - strictly worse than untouched, and exactly the state the
slash-list paragraph above says it had found and fixed once. Three classes, counted before and after:

| class | before | after |
|---|---|---|
| **D1** same-line MIXED - a rewritten token beside a refused one, same target, same line | 8 clusters | **0** *[pass 3: on the old rule's population. The widened rule finds 1 more on the same state, and 2 more half-new lines are visible only by content; all 3 repaired - pass 3]* |
| **D2** cross-line MIXED - a `` `:NNN` `` on the NEXT line inheriting a rewritten citation's file | 7 clusters | **0** |
| **D3** same-file inconsistency - one site resolved at one line, refused as ambiguous-path at another | 8 tokens | **0** |
| all-old clusters (every member still pre-lane; honest, listed) | 36 | 16 (19 tokens) *[pass 3: population-limited - the rule refused every bare token whose antecedent was an unnumbered file mention. The widened rule reads 24 all-old + 1 MIXED on the same post-pass-2 state; after pass 3, 16 ambiguous + 2 named residuals, 0 stale]* |

D1+D2 = **15 mixed clusters** by `w2_pass2_check.py`, which finds any line or line-pair citing one
lane target with numbers on both sides of the map and exits with their count. It prints **0** now.

**Written: 42 tokens / 61 line numbers on 35 lines of 17 files**, four of them new to the diff
(`KE16-DESIGN-{A,APP,SPACE,W}.md`, which held all-old `loom_pool.rs` clusters). Nothing by
arithmetic; the evidence tiers, reported apart from pass 1's L1/L2:

- **L3 - 19 tokens**, the whole refused `non-unique-in-new` class. The line TEXT repeats (a TOML
  section header 32 times, a closing brace 3 times), but the number sits inside a difflib EQUAL
  block of 25-1730 lines with one uniform offset (+18 for `PINS.toml`, +26 for `loom_pool.rs`) -
  the block pins the line, and it is the same map the pass-1 verifier validated 234/234 against.
  Every pass-1 sibling on the same line had moved by that block's offset.
- **L4 - 2 ranges**, `:271-307` and `:335-340` in `PINS.toml`: both END on a <!-- doc-anchor-ignore -->
  `RUSTUP_TOOLCHAIN = "stable-x86_64-pc-windows-gnu"` line the lane ~~DELETED (that deletion is what~~
  ~~the lane is). The end is clipped to the last surviving old line inside the range (306 -> 324,~~
  ~~339 -> 357).~~ **[pass 3: false premise. The lane REPLACED those two lines, it did not delete them: hunks `-307 +325` and `-340 +358`, the same key with `gnu` -> `msvc`. `[vb_mesh]` is new 289-325 and `[vb_mesh_hzb.env]` new 353-358, so each end extends to its replacement line, `:289-325` and `:353-358`; pass 3 wrote both.]** These are the only 2 non-frozen numbers the content meter still counts as not <!-- doc-anchor-ignore -->
  holding the intended content: ~~the intended line no longer exists anywhere~~. **[pass 3: it exists, as its replacement; the meter still counts both, now as REPL - the line is there and its value changed.]**
- **X - 13 tokens on 9 line-pairs**, the cross-line class: the 7 the verifier named, plus
  `VG-R3-P2-CAPABILITY-SPLIT-PLAN.md:1012` (two lines below its antecedent, through a continuation <!-- doc-anchor-ignore -->
  line), `KE16-DESIGN-B4.md:457`, and `KE16-RESULTS.md:1116`'s `, 330` - a comma-SPACE list, a <!-- doc-anchor-ignore -->
  SIXTH form, which pass 1 had left as `234-262, 330`, half new on one line. Two candidates are <!-- doc-anchor-ignore -->
  correctly NOT ours: `VG-R3-P3-CULL-INTEGRATION-PLAN.md:2852`'s `:59`/`:238` follow an unnumbered <!-- doc-anchor-ignore -->
  `vg_density_census.rs` on their own line (the rule refuses any bare token with an unnumbered file
  mention before it) *[pass 3: right for this file, which is not a lane target - and wrong as a rule: the same refusal hid 9 clusters whose unnumbered mention IS a lane target]*, and `KE16-DESIGN-W.md:504` inherits `miri_scope.rs` at lines that did not move. <!-- doc-anchor-ignore -->
- **P3 - 8 tokens**, ambiguous-path resolved by a sibling: the same FILE had already resolved the
  same old number (`VB-P1E-HIERARCHICAL-CULL-PLAN.md` x4, `VG-R3-P2-CAPABILITY-SPLIT-PLAN.md:1538`, <!-- doc-anchor-ignore -->
  `00-GOAL-TARGETS.md:191`), or the same LINE's other numbers fit only one candidate <!-- doc-anchor-ignore -->
  (`VG-R3-P3-CULL-INTEGRATION-PLAN.md:117`'s 1160 beside 3279; `05-LADDER-GATES.md:231`'s 2110 <!-- doc-anchor-ignore -->
  beside its quoted list `3100,3158,3189` - a SEVENTH form, `F:N` / `a,b,c`). **[pass 5: the line is a QUOTATION of L7's row - it says those numbers had drifted - so it was not a citation to resolve; all four numbers are restored, pass 5, (2)]** <!-- doc-anchor-ignore -->
- **RF - 23 tokens**, ambiguous-path but rot-free either way: every cited number lies in the
  offset-0 head of `device.rs` (lines 1-791), so the citation is right whichever `device.rs` it
  means. Reclassified, not edited.

Reverted to pre-lane numbering: **0** - every cluster resolved, so no cluster had to go back whole.

Checked: `old_F[N] == new_F[M]` on all 61 (for L4, N is the surviving line *[pass 3: superseded - for those two ends N is the replaced line and M its replacement, same key, so they hold as REPL, not as ==]*); the content meter over
the population, now including the cross-line, comma-space and slash forms: **297 -> 6** *[pass 3: population-limited by the same refusal - 361 numbers. The widened walk reads 367 -> 76 on the same post-pass-2 state (438 numbers) and 373 -> 28 after pass 3 (444)]* (4 frozen
`docs/ru` + the 2 L4 ends *[pass 3: replaced-line ends, not deleted-line ends]*); CRLF == LF == CR in all 30 files; no file's line count changed except
this one's tail. The pass-1 partial-state audit and forms census were re-run and are NOT quiet, which
is the point: the census shows the same 3 slash sites as before plus this entry's own 3 quotations, and
the audit - which knows only pass 1's log - flags 13 tokens on lines both passes touched: the pass-2
rewrites, five offset-0 or non-lane numbers (`PINS.toml:15`, `KE16-RESULTS.md:12`, `loom_pool.rs:12-16`, <!-- doc-anchor-ignore -->
`rhi_impl/device.rs:1013`, `boyko_rhi/src/device.rs:22`), and an unbackticked `(:61-69)` into <!-- doc-anchor-ignore -->
`miri_scope.rs` whose lines did not move. That audit is superseded by the checker, not re-certified.

**The content meter's label was too generous, and the split is now printed.** "Repaired" meant
`old_F[N] == new_F[M]` - a correct MOVE. Whether the sentence around the citation CLAIMS what that
line holds is a different question, and the verifier now asks it for all 143 rewritten citing lines:
an identifier named on the citing line must occur within three lines of the cited old line.
**45 confirmed, 53 NOT confirmed, 45 uncheckable** (no identifier to test). ~~The 53 are pre-lane rot~~
~~carried over verbatim - correct as a move, wrong as a citation.~~ **[pass 4, 2026-09-11: not all 53. The five `.cargo/config.toml` rows among them were RIGHT at `e6115223` (`:14-15`, `[env]` and `MIRIFLAGS`); the lane commit rewrote them wrong (`:31-32`) and pass 1 took that as old numbering and moved it again (`:53-54`) - lane rot moved twice, neither pre-lane rot nor a correct move. Pass 4 wrote `:36-37`. The other 48 were not re-derived by pass 4.]** With the evidence: <!-- doc-anchor-ignore -->
`VB-P1E-HIERARCHICAL-CULL-PLAN.md:3452` cites `device.rs:2099`, which is `cmd_blit_image`, while <!-- doc-anchor-ignore -->
`SYNCHRONIZATION_VALIDATION_EXT` is at 2282; the 2026-08-06 entry above cites `device.rs:2362`, the <!-- doc-anchor-ignore -->
string `"vkEnumerateInstanceLayerProperties(count)"`, while the `enable_validation &&
BOYKO_DISABLE_VALIDATION` conjunction it quotes is at ~~2480~~ **[pass 4: 2486 - old 2468, offset +18; 2480 is a doc-comment line, old 2468 plus the +12 of a different region of the file]**; ~~four documents cite~~ **[pass 4: three documents, on five lines, cite]**
`.cargo/config.toml:53-54`, two comment lines, while `MIRIFLAGS` is at ~~15~~ **[pass 4: 37 - 15 is its `e6115223` number, while this sentence's other numbers (2099, 2282, 2362) are `97bcf826` numbering]**; <!-- doc-anchor-ignore -->
`VB-PERFORMANCE-TRACK.md:235` cites `PINS.toml:723`, a blank line, for `[vb_mesh_ssao]`, which is at <!-- doc-anchor-ignore -->
1012. **Not repaired here** - it is a different debt and a regex would guess. The list, with the
cited old line and the nearest old line holding each named identifier, is `w2_pass2_prerot.json`.
The heuristic under-reports where a sentence claims without naming and over-reports where a name
sits near the line by chance (`KE16-DESIGN-B4.md:457` is flagged on `pop_any`, which belongs to the <!-- doc-anchor-ignore -->
`worker.rs` citation beside it); each row prints enough to judge.

Scripts, stdlib, in the same scratchpad: `w2_lib.py` (old/new from git, difflib equal blocks),
`w2_pass2_census.py` (forms + cross-line census), `w2_pass2_apply.py` (the write; `--dry` first,
and the dry run caught its own double-move on the slash list's head before anything was written),
`w2_pass2_check.py` (the mixed-cluster gate, exit code = count), `w2_pass2_verify.py` (invariant,
metric, label split), `w2_pass2_note.py` (this addendum).

**Still open, all-old and listed - 19 tokens in 16 clusters.** *[pass 3: the 16 ambiguous clusters stand as listed; the population was limited by the refusal rule, which hid 9 more - see pass 3 below.]* 18 bare `device.rs`:
`RENDER-PARITY-PLAN.md` x7, `OPTIMIZATION-PLAN-RENDER.md` x2, `ARCHITECTURE-FRAME-GRAPH-PLAN.md` x2,
`crates/boyko_app/src/runner.rs:148`, `PARTICLES-PLAN.md:183`, `RENDER-R0-INSTRUMENT-PLAN.md:90`, <!-- doc-anchor-ignore -->
`TAA-PLAN.md:138`, `VG-R3-P2-CAPABILITY-SPLIT-PLAN.md:920-921`, <!-- doc-anchor-ignore -->
`VG-R3-P3-CULL-INTEGRATION-PLAN.md:2842`, `diagnostics/profiling/02-GPU.md:615`; and the <!-- doc-anchor-ignore -->
`05-LADDER-GATES.md:918` at line 4187 of this file, which by its two siblings means the `logging/` <!-- doc-anchor-ignore -->
copy - not lane-modified, but `ONCE_SITES` is at its lines ~~54, 85, 375 and 405~~ **[pass 4: 54, 85, 375, 376 and 405]**, never 918:
pre-existing rot, not lane rot. Two of the 18 have a cross-FILE sibling with the identical old
number and identical quoted context (`runner.rs:148` quotes the same conjunction as the 2026-08-06 <!-- doc-anchor-ignore -->
entry's 2350; `VG-R3-P2-CAPABILITY-SPLIT-PLAN.md:920`'s 2107-2111 holds the 2110 that <!-- doc-anchor-ignore -->
`00-GOAL-TARGETS.md:150` resolved). That rule was not adopted - this pass disambiguated by <!-- doc-anchor-ignore -->
same-file siblings only - and they are put to you with the rest.


### Pass 3 (2026-09-10, same day): the refusal rule hid 9 clusters, the L4 premise was false, and a tenth half-new line nobody named

The pass-2 verifier returned four defects. This pass repairs them; every figure below is measured by
the scripts named at the end, none is carried over.

**Who wrote what.** 25 of this pass's 26 token edits (58 numbers) were written to the tree at 22:52
by an earlier invocation of this pass, which stopped before verifying or recording them; its log is
`w2_pass3_applied.json`. This invocation re-derived every one of those numbers from `git show` and
re-checked it, and wrote one token more (2 numbers). Nothing inherited was taken on trust.

**The widened rule.** A bare `:N`, `:N-M` or comma(-space) list whose NEAREST file mention - on its own
line, else the last one on the line above - is an UNNUMBERED mention of a lane target (full path or
basename) cites that target. In every other case pass 2's rule stands verbatim, and an unnumbered
NON-lane mention still refuses the token: `VG-R3-P3-CULL-INTEGRATION-PLAN.md:2852`'s <!-- doc-anchor-ignore -->
`vg_density_census.rs` is still skipped. One rule in one file: `w2_pass3_tok.py` is imported by both
the checker and the meter, because pass 2 carried two copies of the old rule and they shared its
blind spot. A comma-SPACE list inside backticks is now a bare token as well - pass 2's bare-token
pattern could not match `VG-R3-P3-CULL-INTEGRATION-PLAN.md:40`'s 26-number row at all. <!-- doc-anchor-ignore -->

**It is a widening, and it fires - both shown, not argued.**

- Census over the whole tree, the old rule against the new on every line: 2074 tokens identical, 62
  added, 2 re-assigned to a nearer lane mention, **0 dropped**.
- Injected probe. A scratch COPY of `VG-R3-P2-CAPABILITY-SPLIT-PLAN.md` as pass 2 left it, with three
  synthetic stale tokens appended past its last line - same-line `scripts/golden.ps1` (`:259`), <!-- doc-anchor-ignore -->
  cross-line `goldens/PINS.toml` then (`:339`) on the next line, comma-space `goldens/PINS.toml` at <!-- doc-anchor-ignore -->
  `:25, 46` - and one control, `vg_density_census.rs` (`:187`). The widened checker's output on the <!-- doc-anchor-ignore -->
  copy differs from its output on the un-injected copy by exactly three STALE clusters (four
  numbers) and one skipped control; pass 2's checker prints byte-identical output on both copies.
  The tree was never written for this (`p3proof/`).
- The "before" state is the tree with this pass's logged edits reversed in memory - no checkout. It
  is validated, not assumed: pass 2's checker, run unchanged over it, reproduces pass 2's final log
  byte for byte, and the meter's old-rule population reproduces pass 2's 361 numbers and 297 -> 6.

**What the widened rule finds on the post-pass-2 state: 9 clusters, 63 stale numbers** - the
verifier's count, cluster for cluster (the checker exits 7 there: 1 MIXED + 6 STALE; the other two
are the named residuals below).

| cluster | numbers | target by text | target by content | disposition |
|---|---|---|---|---|
| `VG-R3-P2-CAPABILITY-SPLIT-PLAN.md:1032-1033` | 3 | `PINS.toml` | `golden.ps1` | rewritten, `golden.ps1` map (+3) <!-- doc-anchor-ignore --> |
| `VG-R3-P2-CAPABILITY-SPLIT-PLAN.md:1386-1388` | 4 | `golden.ps1` | `golden.ps1` | rewritten (+3) <!-- doc-anchor-ignore --> |
| `VG-R3-P2-CAPABILITY-SPLIT-PLAN.md:1548` (MIXED) | 4 | `PINS.toml` | `golden.ps1` | rewritten (+3); defect 3 <!-- doc-anchor-ignore --> |
| `VG-R3-P3-CULL-INTEGRATION-PLAN.md:40` | 28 | `PINS.toml` | `PINS.toml` | rewritten (+18; one REPL) <!-- doc-anchor-ignore --> |
| `VG-R3-P3-CULL-INTEGRATION-PLAN.md:1862` | 4 | `PINS.toml` | `PINS.toml` | rewritten (+18) <!-- doc-anchor-ignore --> |
| `archive/LIGHTING-L0-L1-PLAN.md:801` | 4 | `device.rs` | `device.rs` | rewritten (+12) <!-- doc-anchor-ignore --> |
| `threadpool/KE16-DESIGN-W.md:153` | 3 | `loom_pool.rs` | `loom_pool.rs` | rewritten (+26) <!-- doc-anchor-ignore --> |
| `VG-R3-P2-CAPABILITY-SPLIT-PLAN.md:877-878` | 5 | `PINS.toml` | `vg_density_census.rs`, not a lane file | **not** rewritten - content-refuted <!-- doc-anchor-ignore --> |
| `logging/00-GOAL-TARGETS.md:182` | 8 (16 occurrences) | `golden.ps1` | `golden.ps1` as of the carve | **not** rewritten - historical <!-- doc-anchor-ignore --> |

Each offset above is a difflib EQUAL block's, and each number was checked inside its block
(`old_F[N] == new_F[M]`), never applied as arithmetic.

**The widening is a better detector, not an oracle: three of its nine attributions are wrong by
content.** `:1032-1033` and `:1548` follow an unnumbered `PINS.toml`, but they describe what <!-- doc-anchor-ignore -->
`golden.ps1` does - the throw under `-Bless`, the `Set-Content`, the check path's `exit 2` - and old
`golden.ps1` 240, 258, 259, 266 and 270 hold exactly those lines. They were rewritten by the
`golden.ps1` map, and the checker now prints every written token whose recorded target differs from
the rule's reading (15 across the three passes). `:877-878` follow `goldens/PINS.toml` too, but cite <!-- doc-anchor-ignore -->
the test named on line 876, `vg_density_census.rs:187`: old `PINS.toml:196` is `width = 512`, while <!-- doc-anchor-ignore -->
the test holds `.filter(|s| s.starts_with("vb"))`. The verifier's "PINS +18" for that cluster was the
rule's reading taken as a fact; applied, it would have moved five numbers of one file into another.

**Defect 1 - repaired.** `VG-R3-P3-CULL-INTEGRATION-PLAN.md:988` `:353-357` -> `:353-358`, and <!-- doc-anchor-ignore -->
`VG-R3-P2-CAPABILITY-SPLIT-PLAN.md:1011` `:289-324` -> `:289-325`. Re-derived from `git show <!-- doc-anchor-ignore -->
97bcf826:goldens/PINS.toml`: new 325 and 358 are `RUSTUP_TOOLCHAIN = "stable-x86_64-pc-windows-msvc"`,
the last lines of `[vb_mesh.env]` and `[vb_mesh_hzb.env]`; old 307 and 340 are the same key with
`gnu`. The pass-2 text asserting a deletion is struck above, in place.

**Defect 3 - repaired, and its class searched rather than its instance.** `:1548`'s four <!-- doc-anchor-ignore -->
`golden.ps1` numbers are in the table. `VG-R3-P2-CAPABILITY-SPLIT-PLAN.md:1538`'s `:2110-2122` -> <!-- doc-anchor-ignore -->
`:2122-2134` and `:2187-2193` -> `:2199-2205`: the last-numbered-antecedent rule gives them <!-- doc-anchor-ignore -->
`ffi.rs:1665`, but `ffi.rs` 2187-2193 is `VkDescriptorSetVariableDescriptorCountAllocateInfo`; by <!-- doc-anchor-ignore -->
content they are `device.rs`. The rule mis-assigned them; the text was never ambiguous to a reader.
`w2_pass3_suspects.py` then lists every bare token the rule gives to a non-lane numbered mention
while a lane mention stands before it: 19 distinct tokens, each read. 16 are not lane citations
under any reading, or are offset-0 under the lane's map whichever file they mean (the plan's own
`:369`/`:371`/`:105`/`:353`, `ci.yml`'s `:64`, `check_hotpath_exceptions.py`, `vm_column.rs`, <!-- doc-anchor-ignore -->
`vg_occ_split_timing.rs`, `host.rs`, `ffi.rs`'s own `:1670`, ...), 2 are `:1538`'s above, and <!-- doc-anchor-ignore -->
**one is a tenth half-new line that neither the verifier nor the widened rule saw**:
`VG-R3-P2-CAPABILITY-SPLIT-PLAN.md:1500`, where pass 1 had moved `device.rs:2164`, `:2165-2172` and <!-- doc-anchor-ignore -->
`:2199-2205` and left `:2110-2111` behind `runner.rs:213`. By content it is `device.rs` - the <!-- doc-anchor-ignore -->
`sync_validation_available` conjunction that `00-GOAL-TARGETS.md:150` cites as the same `:2110-2111` <!-- doc-anchor-ignore -->
and pass 1 moved to `:2122-2123`. Rewritten to `:2122-2123` by the map. The mirror census - a token <!-- doc-anchor-ignore -->
static under its rule target but moved under another lane target named before it - returns 0.

**Numbers written this pass: 60** - 57 by the map (`old_F[N] == new_F[M]`, and difflib's
equal-block map agrees) and 3 by REPL (a 1:1 `replace` line with the same key): the two range ends
of defect 1, and `VG-R3-P3-CULL-INTEGRATION-PLAN.md:40`'s 738 -> 756, a lone number in the row that <!-- doc-anchor-ignore -->
is also a replaced `RUSTUP_TOOLCHAIN` line. That third one extends defect 1's rule beyond a range
end; it is stated here rather than left implicit. `w2_pass3_verify.py` prints all 60, each with its
old and new line text.

**Pre-lane rot, moved correctly and still wrong as a citation - listed, not repaired (pass 2's
policy).** `VG-R3-P3-CULL-INTEGRATION-PLAN.md:40` says "26 top-level tables at ...", but at <!-- doc-anchor-ignore -->
`e6115223` only 12 of its 26 numbers are top-level headers - of the other 14, six are comments,
four are key lines (`BOYKO_VG_SCENE`, `sha256_software`, `BOYKO_SHADOW_DENOISE`,
`RUSTUP_TOOLCHAIN`), two are `crate`/`test_binary` lines and two (778, 912) are `.env` sub-tables -
and `BOYKO_VG_OCC` is not at 394. The `device.rs` numbers at `:1500` and `:1538` were already wrong at <!-- doc-anchor-ignore -->
`e6115223`: 2110 is `})`, while `sync_validation_available` is at 2225 and the `p_next` choice at
2305. A move preserves exactly what a citation pointed at; that it pointed at the wrong thing before
the lane is a separate debt, the same class as pass 2's 53 NOT-CONFIRMED *[pass 4: less its five `.cargo/config.toml` rows, which were lane rot moved twice - see the correction in pass 2 above]*.

**The historical-narrative decision: `00-GOAL-TARGETS.md:182` is left as written, intentionally <!-- doc-anchor-ignore -->
old.** The paragraph is the record of a withdrawal at the carve. It quotes what v4 and the withdrawn
row SAID and what `golden.ps1` held when that was measured; rewriting its numbers to today's would
make it report that v4 said a number v4 never said, and would break the argument it exists to make.
Its knock-on is recorded rather than repaired: its present-tense "The file has not moved since
`66148ae`" became false at `97bcf826`, whose one `golden.ps1` hunk (`-21,2 +21,5`) moved every line
below it by +3; and the rows it points at (that file's lines 111 and 191) carry today's
numbering since pass 1. The paragraph and its neighbours now disagree by exactly the lane's +3.

**Final state, measured.**

- Widened checker over the whole tree: 216 clusters, **0 MIXED, 0 STALE** *[pass 4: population-limited - true of this checker's population, not of the tree. The pass-3 verifier's scanner, which also reads a bare number two or more lines below its mention and one behind a nearer non-lane mention, reads 25 more unmoved numbers on 10 lines (4 stale, 21 already wrong at `e6115223`), and 10 wrong numbers - the `.cargo/config.toml` double move - that no checker taking the HEAD token as the old number can see; see pass 4]*; 18 all-old = pass 2's 16
  ambiguous clusters + the 2 named residuals. Exit 0 (it exits 7 on the post-pass-2 state).
- Meter, three runs over one walk: old rule / post-pass-2, 361 numbers: 297 -> 6 (pass 2's figure,
  reproduced); widened / post-pass-2, 438: 367 -> 76; widened / tree, 444: 373 -> **28** = 4 frozen
  `docs/ru` + 3 REPL + 16 occurrences in the historical paragraph + 5 content-refuted. **0 unclassed.** *[pass 4: population-limited in the same way - 0 unclassed among the 444 numbers this walk reaches; its 10 `.cargo/config.toml` numbers counted as holding, because the meter took the logged HEAD token as the old line (`old[31]` and `new[53]` are both `#`)]*
- No stray edit: every one of the 30 modified files, with the logged edits of passes 3, 2 and 1
  reversed, equals `git show HEAD:` (LF-folded; this file on its first 5165 lines). CRLF == LF == CR
  in all 30; `docs/ru/` untouched.

**Residuals, by name.** (1) Pass 2's 16 ambiguous clusters, unchanged. (2)
`VG-R3-P2-CAPABILITY-SPLIT-PLAN.md:877-878` - content-refuted, not a lane citation; its numbers are <!-- doc-anchor-ignore -->
pre-existing rot against `vg_density_census.rs`, whose test function is at 220 today, not 187. (3)
`00-GOAL-TARGETS.md:182` - historical, above. (4) The 4 frozen `docs/ru/` citations. (5) The 3 REPL <!-- doc-anchor-ignore -->
numbers: they name the right line, whose value the lane changed, so no exact-text meter can ever
call them "holding the intended content". Plus the pre-lane rot above, listed with pass 2's.

Scripts, stdlib, same scratchpad: `w2_pass3_tok.py` (the one token walk), `w2_pass3_state.py` (the
logs, and the reversed post-pass-2 view), `w2_pass3_check.py` (exit = MIXED + STALE; `--state pre3`,
`--only`, `--sub REPO=COPY`, `--census`), `w2_pass3_oldrun.py` (pass 2's checker, unchanged, over a
chosen state or copy), `w2_pass3_verify.py` (invariant, meter, label split, stray-edit reversal,
CRLF), `w2_pass3_suspects.py` and `w2_pass3_suspects2.py` (defect 3's class), `w2_pass3_apply.py` with
`w2_pass3_applied.json` (the 22:52 write), `w2_pass3b_apply.py` with `w2_pass3b_applied.json` (this
invocation's one token), `w2_pass3_note.py` (this addendum), and `p3proof/` (the probe copies and
both checkers' output on them).

**WHAT IS PUT TO YOU:** whether `00-GOAL-TARGETS.md:182` gets a dated note naming today's numbers <!-- doc-anchor-ignore -->
beside the carve-time ones; and whether the plan corpus keeps writing bare `:N` at all. Every
cluster this pass repaired is a bare number whose file had to be inferred, and the best text rule
measured here infers three of nine wrong.

### Pass 4 (2026-09-11): a double move no checker could see, two wrong-file rewrites, and 21 of the verifier's 25 continuation numbers refused by content

The pass-3 verifier returned **NOT SOUND** with a finite list; this pass works that list and nothing
else. Every number it writes is justified by the text of the cited line, printed beside the old line
by `w2_pass4_apply.py --dry` before anything was written, never by an offset.

**Written: 16 line numbers - 11 token edits on 9 lines of 4 documents** - and, in this entry, 8
register lines corrected in place above (struck through, nothing deleted).

- **The double move, 10 numbers on 5 lines** - `KE16-DESIGN-MEASUREMENT.md:37` and `:530`, <!-- doc-anchor-ignore -->
  `KE16-DESIGN.md:373`, `KE16-TASKREP-ALLOCATION.md:67` and `:772`: `.cargo/config.toml:53-54` -> <!-- doc-anchor-ignore -->
  `:36-37`. Old line 14 `[env]` is new 36; old line 15 `MIRIFLAGS = "-Zmiri-tree-borrows"` is new 37. <!-- doc-anchor-ignore -->
- **The two wrong-file rewrites, 2 numbers** - `VG-R3-P2-CAPABILITY-SPLIT-PLAN.md:1504` `:375` -> <!-- doc-anchor-ignore -->
  `:357` and `:1506` `:368` -> `:350`, i.e. back to what they said before pass 1. <!-- doc-anchor-ignore -->
- **One cluster of the verifier's continuation table, 4 numbers** - <!-- doc-anchor-ignore -->
  `VG-R3-P2-CAPABILITY-SPLIT-PLAN.md:1013-1014`: `:327`, `:328`, `:335`, `:339` -> `:345`, `:346`, <!-- doc-anchor-ignore -->
  `:353`, `:357`. Old and new lines are identical and are what the sentence names: `test_binary = <!-- doc-anchor-ignore -->
  "vb_mesh"`, `test_name = "vb_mesh_screenshot_dump"`, `[vb_mesh_hzb.env]`, `BOYKO_VG_HZB = "1"`.
  Line 1012 already gave that pin as `:327-359`, so until now `:327` named two lines in one sentence. <!-- doc-anchor-ignore -->

**How the double move happened, and why three checkers passed it.** At `e6115223` the five lines
said `:14-15`, which was right. The lane commit itself rewrote them to `:31-32`, which was already <!-- doc-anchor-ignore -->
wrong: at `97bcf826` those are two lines of the toolchain comment. Pass 1 then took the HEAD token
`31-32` as OLD numbering and moved it through the lane's map to `53-54`. Old 31 is `#` and new 53 is
`#`, so `old_F[N] == new_F[M]` held - as it holds for any number inside an equal block. Pass 1's
meter, pass 2's checker and meter, and pass 3's checker and meter all read the number under test
from the HEAD tree (or from a pass log's `old`, which is the HEAD token), so all of them certified
`53-54`. The verifier read the citing file at `e6115223` instead. Asking what a line said BEFORE
the lane is the only question that can see a citation the lane itself rewrote.

The class is closed, not sampled. Of the 144 lines passes 1-3 wrote, exactly 5 differ between
`e6115223` and `97bcf826` in their digits, and they are these five (`w2_pass4_dmcensus.py`). Over
all 44 files the lane changed, its own digit-only line rewrites are the same config citations and
nothing else (`w2_pass4_lanerw.py`; the fifth, `:67`, sits in an unequal replace block). <!-- doc-anchor-ignore -->

The checker's blind spot is now measured from the other side. Run over the repaired tree,
`w2_pass3_check.py --state tree` exits **7**, where it exited 0 before this pass. It reports the
correct `:36-37` as 5 STALE clusters, because it reads 36 as an old number that should have moved <!-- doc-anchor-ignore -->
+22. It reports `:1504` and `:1506` as 2 MIXED, because it gives the plan's own numbers to <!-- doc-anchor-ignore -->
`PINS.toml`. All 7 are right by content. That checker is superseded here, not re-certified.

**The two wrong-file rewrites.** Pass 1 moved `:357` and `:350` by the `PINS.toml` map because <!-- doc-anchor-ignore -->
`PINS.toml` was the nearest file mention. By content, they are the plan citing its OWN round-1 text:
- at `9e80cd4e` (the round-1 critique commit), the plan's line 357 reads `vb_scopes == 2,
  vb_late_draws == <batch count>`, the clause `:1504` quotes; <!-- doc-anchor-ignore -->
- its line 350 carries "one that cannot be blessed wrong", the phrase `:1506` says to strike; <!-- doc-anchor-ignore -->
- its line 353 carries the `base_instance` control that `:1504` also names. <!-- doc-anchor-ignore -->

Neither phrase has ever been in `goldens/PINS.toml`: 0 occurrences at `e6115223` and at `97bcf826`,
and `git log -S` over every ref finds none. The plan is not a lane file, so those numbers never
moved. The verifier's scanner still gives both to `PINS.toml`, and now flags them as STALE and as
same-line MIXED. That is its rule, not the text; they are listed below.

The same reading found one more of this class, on a NUMBERED token, which the verifier's bare-token
reading did not cover. `VG-R3-P2-CAPABILITY-SPLIT-PLAN.md:882` says the critique <!-- doc-anchor-ignore -->
cited that note as `PINS.toml:57-64`, and `9e80cd4e:540` says exactly `PINS.toml:57-64`. The point <!-- doc-anchor-ignore -->
of the sentence is that the numbers are right and the file is wrong. Pass 1 rewrote the quote to
`PINS.toml:75-82`, so it now reports a citation the critique never made. The verifier's list does <!-- doc-anchor-ignore -->
not include it, so it is **not written**; the restore is put to you. **[pass 5: restored, with eight more members of its class - pass 5, (2)]** The parenthetical on line 883
("`PINS.toml:75-82` is the `[grand_showcase_2mat]` block") does hold at HEAD. <!-- doc-anchor-ignore -->

**The continuation-line class: 25 numbers on 10 lines, 4 written, 21 refused.** The verifier's
scanner reads what pass 3's rule cannot: a bare number two or more lines below its mention
(`V3_WALK=6`), and a number behind a nearer non-lane mention (`v3_para.py`). Its table gives each of
the 25 a map value, and every map value holds as a MOVE. Pass 4 then asked whether the sentence is
true of the line the value names. For 21 of the 25 it is not, and it was already not true at
`e6115223`:
- `VB-SV0-SDF-SHADOW-PLAN.md:1627-1628` (8 numbers) *[pass 8: dated records, kept as written. Both lines sit under the section's lead-in, `:1542`, "Every line below was opened while writing this revision", and `git blame` gives the lead-in and both lines to `62731d91`, where all 8 numbers hold: `PINS.toml` `[vb_both]` 313-343, empty edit list 322, pre-fill note 326-328, `[vb_sdf_only]` 345-377, empty 355. Not moving them was right; counting them as rot was not - Pass 8, (B)]*: `[vb_both]` is at 583 at `e6115223` and 601 <!-- doc-anchor-ignore -->
  at HEAD, not 313, and `[vb_sdf_only]` is at 706 and 724, not 345. Old 313-377 and new 331-395 are
  comments and keys inside `[vb_mesh_hzb]` and `[vb_occ_split]`. Its siblings, which pass 1 moved on
  lines 1625 and 1521, are the same rot, moved.
- `VG-R3-P2-CAPABILITY-SPLIT-PLAN.md:916` (4): `VkValidationFeaturesExt` is packed at old 2271 / new <!-- doc-anchor-ignore -->
  2283, and the `p_next` chain head is chosen at new 2317-2318. The cited 2153-2193 / 2165-2205 are
  `#[inline(never)]`, a message string, an instance-extension check and the `VkApplicationInfo`
  literal. The same holds for line 914's `device.rs:2164`, <!-- doc-anchor-ignore -->
  which pass 1 moved from 2152: at HEAD, line 2164 is `#[cold]`.
- `archive/LIGHTING-L0-L1-PLAN.md:183` (2): `query_device_caps` is at old 3211 / new 3229, and <!-- doc-anchor-ignore -->
  `DeviceCaps` at old 200. The cited 1819-1842 is a function-pointer table (`instance,` ... `gipa,`).
  Its siblings on lines 181 and 801, moved by passes 1 and 3, are the same rot.
- `threadpool/KE16-DESIGN-A.md:497` (2) and `KE16-DESIGN-W.md:156`, `:160`, `:163` (5): see the <!-- doc-anchor-ignore -->
  `loom_pool.rs` finding below.

Moving any of these 21 would have made them agree with the map and stay false. The rule says a
number that does not hold is not written, so none was.

**A finding of the same kind, measured over every `loom_pool.rs` number any pass wrote: 35 numbers on
15 lines in 7 files, and none was ever `e6115223` numbering.** The file was 387 lines at `778739f0`,
where the KE16 design corpus was written. It was 1640 lines at `7fdd738e` and `611999cb`, where
`KE16-DESIGN-B4.md` and `KE16-RESULTS.md` last cite it. Then `67563d3b` (2026-09-09) cut it to
644, and that is what the lane started from.

All 35 were read at the numbering they were written against, and all 35 hold there. At `778739f0`:
- 143-166 is the M2 transport-ordering fidelity note;
- 217 is `fence(Ordering::SeqCst); // injector push transport fence`;
- 140 and 343 are the two stale `worker.rs` references that `KE16-DESIGN-SPACE.md:554` lists. <!-- doc-anchor-ignore -->
At `7fdd738e`, 387 is the `#[should_panic(expected = "M2: lost wake")]` attribute and 340 the
steal-transport fence.

At `e6115223` and at HEAD the same numbers hold other text: a closing brace, a blank line, a CAS
helper's doc comment. So every map move on these lines, by passes 1, 2 and 3, moved a pointer that
already pointed at the wrong line, and pass 3's table row for `KE16-DESIGN-W.md:153` ("rewritten <!-- doc-anchor-ignore -->
(+26)") is a correct move of a wrong number. Pass 4 wrote none of them and refused the five of this
kind in the verifier's table. Whether this corpus gets dated notes giving today's numbers (the
`00-GOAL-TARGETS.md:182` question again) or a new derivation is put to you. <!-- doc-anchor-ignore -->

**The 15 written ~~numbers~~ *[pass 5: tokens, carrying 18 numbers - the 7 that hold are one number each, the 8 that do not carry 11]* whose recorded target differs from the rule's reading, re-read by content.**
- 7 hold: `VG-R3-P2-CAPABILITY-SPLIT-PLAN.md:1033`'s `:261`/`:269`/`:273` and `:1548`'s <!-- doc-anchor-ignore -->
  ~~`:262`/`:243`/`:261`/`:269`~~ *[pass 7: since pass 6, `:1548` reads `:259`/`:240`/`:258`/`:266`, its as-of-2026-08-06 numbering (`:1536`). At `9e80cd4e` those `golden.ps1` lines are `Set-Content`, `if ($Bless) {`, the throw and the `PENDING` check, the same four texts. Seven still hold: `:1033`'s three at HEAD, `:1548`'s four as a record - Pass 7, (4)]*. New lines 243, 261, 262, 269 and 273 of `golden.ps1` are `if ($Bless) {`, <!-- doc-anchor-ignore -->
  the throw, `Set-Content`, the `PENDING` check and `exit 2`. The verifier's 7 WRONG there are false
  alarms from its rule, which gives them to `PINS.toml`.
- 8 do not hold, and all 8 are `device.rs` numbers:
  - `VB-P1E-HIERARCHICAL-CULL-PLAN.md:2571`'s 2099 is `cmd_blit_image`; <!-- doc-anchor-ignore -->
  - `VG-R3-P2-CAPABILITY-SPLIT-PLAN.md:1500`'s 2122-2123 is `})` / `}`; <!-- doc-anchor-ignore -->
  - `:1538`'s 2164, 2199-2205 and 2122-2134 are `#[cold]`, instance-extension checks and a <!-- doc-anchor-ignore -->
    doc-comment tail;
  - `VG-R3-P3-CULL-INTEGRATION-PLAN.md:117`'s 1172 is a comment and its 3279 is `)`; <!-- doc-anchor-ignore -->
  - `logging/00-GOAL-TARGETS.md:191` and `logging/05-LADDER-GATES.md:231`'s 2122 is `})`. <!-- doc-anchor-ignore -->

  On every one of the 8 the citing line reads the same at `e6115223` and at HEAD. So none is a double
  move: all are pre-lane rot, moved by the map~~, as the pass-2 and pass-3 lists already say~~ **[pass 5: 6 of the 8 were already listed - `w2_pass2_prerot.json` holds `:2571`'s 2099, `:1538`'s 2164 and `00-GOAL-TARGETS.md:191`'s 2122, and pass 3's pre-lane paragraph covers the `device.rs` numbers of `:1500` and `:1538` (2122-2123; 2199-2205, 2122-2134). The other 2 - `VG-R3-P3-CULL-INTEGRATION-PLAN.md:117`'s 1172 and `05-LADDER-GATES.md:231`'s 2122 - were listed for the first time here, and the second is a quotation, restored by pass 5]**. <!-- doc-anchor-ignore -->

**Gate - the pass-3 verifier's own scanner, run unchanged from copies in `p4/`, before and after
this pass.** Before, `v3_scan.py` read:
- walk 1: STALE 21, WRONG 17, paragraph-MIXED 1;
- walk 6: STALE 44, WRONG 17, paragraph-MIXED 7;
- same-line MIXED 0 on both walks.

After:
- walk 1: STALE 23, WRONG 7, same-line MIXED 2, paragraph-MIXED 3;
- walk 6: STALE 42, WRONG 7, same-line MIXED 2, paragraph-MIXED 8.

Every flagged number is labelled, 0 unlabelled (`p4/gate.py`):
- STALE: the 21 named residuals (`:877-878`, `00-GOAL-TARGETS.md:182`), the 17 refused above that it <!-- doc-anchor-ignore -->
  reaches, `:881`'s two (the plan's "that file's own `:57-64`", `vg_density_census.rs` by lines <!-- doc-anchor-ignore -->
  883-884), and the 2 restored plan self-references;
- WRONG: the 7 `golden.ps1` false alarms;
- MIXED: the 2 same-line MIXED are the restored self-references. Every paragraph-MIXED is a
  paragraph where a moved number sits beside one of the refused, residual or restored numbers above.

The gate as worded - 0 stale and 0 mixed outside the named residuals - is therefore **not met, and
cannot be met by writing**. Every number that would clear it is false by content at HEAD.

`v3_para.py` goes from 158 candidates to 156:
- 42 are lane-nearest, each one labelled above;
- 114 sit behind a nearer non-lane file named on their own line or the line above. ~~109~~ **[pass 5: 110]** of them cite
  that file (spot-read, not every one read). The other ~~5~~ **[pass 5: 4 - lines 160 and 163 carry four numbers, `:143-166` and `:147-152`; the fifth refused `loom_pool.rs` number, `KE16-DESIGN-W.md:156`'s `:237`, is one of the 42 lane-nearest above, and 114 = 110 + 4]** (`KE16-DESIGN-W.md:160`/`:163`) are the <!-- doc-anchor-ignore -->
  refused `loom_pool.rs` numbers.

**Residuals, by name.**
1. Pass 2's 16 ambiguous `device.rs` clusters.
2. `VG-R3-P2-CAPABILITY-SPLIT-PLAN.md:877-878`, content-refuted. `:881` joins it: it is the same <!-- doc-anchor-ignore -->
   file's own lines.
3. `00-GOAL-TARGETS.md:182`, historical. <!-- doc-anchor-ignore -->
4. The 4 frozen `docs/ru/` citations.
5. The 3 REPL numbers.
6. **New:** the 21 refused continuation numbers above. *[pass 8: 13 of them. `VB-SV0-SDF-SHADOW-PLAN.md:1627`-`1628`'s 8 are dated records kept as written - Pass 8, (B)]* <!-- doc-anchor-ignore -->
7. **New:** the 35 pass-written `loom_pool.rs` numbers, which were never `e6115223` numbering.
8. **New:** `:882`'s rewritten quote. **[pass 5: restored - it is one of nine; the class is listed in pass 5, (2)]** <!-- doc-anchor-ignore -->
9. The pre-lane rot pass 2 and pass 3 already list, now including the `device.rs` 8 above.

Also, for information, the verifier's two items outside the `:N` form: the `~L` references in
`RENDER-HWRT-OPTIONAL-ANALYSIS.md` and `RENDER-P5-HYBRID-PLAN.md`, and `KE16-TASKREP-ALLOCATION.md:827`'s <!-- doc-anchor-ignore -->
`:1534+`, which is the same `loom_pool.rs` story: M1c's header is there at `7fdd738e` (1640 lines). <!-- doc-anchor-ignore -->

**Hygiene.** CRLF == LF == CR in every written file. `git status` shows the same 30 ` M` files as
at the start and nothing else, and `docs/ru/` is untouched. Every doc line pass 4 changed differs
from HEAD only in digits. Register corrections are insertions only: each old line survives as a
character subsequence of its new text. No cargo; no checkout, stash, restore, reset or commit.

Scripts, stdlib, same scratchpad:
- `w2_pass4_apply.py` (the doc edits; `--dry` prints every number's old and new line), with its log
  `w2_pass4_applied.json`;
- `w2_pass4_note.py` (this entry and the in-place corrections);
- `w2_pass4_dmcensus.py` and `w2_pass4_lanerw.py` (the double-move class);
- `p4/` (the verifier's scanner copied unchanged, `gate.py`, `para_class.py`, before/after logs).

**WHAT IS PUT TO YOU:**
- whether `:882`'s quote goes back to `PINS.toml:57-64`; **[pass 5: it went back]** <!-- doc-anchor-ignore -->
- whether the 35 `loom_pool.rs` numbers, and the 21 refused ones, get dated notes or a fresh
  derivation against today's files;
- whether any future pass may certify a number without reading the citing line at the base commit.
  Three checkers built to catch exactly this passed a number that was wrong twice.

### Pass 5 (2026-09-11): closing scope - lane shifts repaired, quotations restored, pre-lane rot named as debt

The orchestrator closed the lane with a fixed scope. Four passes showed that a map move cannot make a
citation right if it was already wrong before the lane, and every new verifier counted that rot as a
failure of the lane again. The scope:

1. **Lane-caused shifts.** A citation whose target line moved in the lane commit (`e6115223` ->
   `97bcf826`) and was TRUE at `e6115223` must be TRUE at HEAD.
2. **Rewritten quotations** go back to their pre-pass text. They report what a document said at a
   named commit, not what a file holds now.
3. **Pre-lane rot** is not repaired and not moved further. That is every number already false at
   `e6115223`. It is listed below by name, as separate debt. A number that passes 1-3 moved by the
   map and that was false both before and after is recorded as *moved, still false - the move
   neither helped nor harmed*, and it is not moved back, because that would churn files without
   making anything true.
4. **This register:** the pass-4 verifier's three false sentences are corrected in place, and the
   quotation class is listed in full.

**(1) Lane-caused shifts: none of the written numbers is still wrong; ~~one citation that no pass wrote was, and it is now written.~~ [pass 6: two were. `PARTICLES-PLAN.md:183` is written; `VG-R3-P3-CULL-INTEGRATION-PLAN.md:2924` (`[vb_mesh_hzb]:335-341`) escaped every regex by its form, and pass 6 leaves it as written because it is a dated record - Pass 6, F1]** <!-- doc-anchor-ignore -->

- The pass-4 verifier's `v4/rows_j.json` covers every number passes 1-4 wrote: 357 numbers on 146
  lines. Each was re-read in its target file at both commits.
  - 344 hold `old_F[N] == new_F[M]` exactly.
  - 3 are REPL: the lane's own `gnu` -> `msvc` line, same key.
  - The 10 `.cargo/config.toml` numbers hold from the citing text as it was at `e6115223`:
    `:14-15` there is `[env]` / `MIRIFLAGS`, and those are new 36-37. <!-- doc-anchor-ignore -->
  A content-preserving move cannot change whether a citation is true. So ~~the 152 numbers the~~
  ~~verifier judged TRUE were true at `e6115223` and are true at HEAD~~ *[pass 6: ~~147 of the 152 were.~~ 5 were judged TRUE and were false at `e6115223`: `KE16-DESIGN-B4.md:378`'s 531, `:385`'s 1158 and 1163, and `KE16-RESULTS.md:2257`'s 44 and 48. [pass 7: and seven `device.rs:3158`, which at `e6115223` is the DDGI reporter's `W2102,`, not the shadow-denoise site: 12 in all, so 140 of the 152 were true at `e6115223` - Pass 7, (1)] This pass checked that each target line kept its text across the lane, which is true of ~~all 152~~ [pass 7: 150. `VG-R3-P2-CAPABILITY-SPLIT-PLAN.md:1011`'s 307 and `VG-R3-P3-CULL-INTEGRATION-PLAN.md:988`'s 340 point at REPL lines, `RUSTUP_TOOLCHAIN` with `gnu` -> `msvc` - Pass 7, (5)], and not whether the verifier's TRUE was right - Pass 6, F4]*, and nothing it judged false was <!-- doc-anchor-ignore -->
  true at `e6115223`.
- The class also holds citations that no pass wrote.
  - Pass 2's 16 ambiguous clusters were read at `e6115223`. 15 cite a bare `device.rs`, and the lane
    changed only one of the four. 14 of those 15 do not hold in it there, so they cannot be lane
    shifts, whichever file they mean. The 16th cluster cites a bare `05-LADDER-GATES.md:918` (at <!-- doc-anchor-ignore -->
    `OPEN-QUESTIONS.md:4187`). No copy holds `ONCE_SITES` at 918. The `logging/` copy, which its two <!-- doc-anchor-ignore -->
    siblings mean, has it at 54, 85, 375, 376 and 405, and the lane did not touch that copy.
  - The one that does hold is `PARTICLES-PLAN.md:183`: `device.rs:578`, `:2029`, for "`vkCmdDispatchIndirect` <!-- doc-anchor-ignore -->
    is **loaded**". At `e6115223`, line 2029 is
    `cmd_dispatch_indirect: load_device_command(gdpa, device, c"vkCmdDispatchIndirect")?,` and line
    578 is `pub cmd_dispatch_indirect: PfnVkCmdDispatchIndirect,`. That pair fits one file only. The
    lane moved 2029 to 2041 and left 578 in its offset-0 head.
  - **Written: `:2029` -> `:2041`.** This is the one number pass 5 wrote under (1). It was TRUE at <!-- doc-anchor-ignore -->
    `e6115223`, the lane made it false, and no pass had touched it. `docs/PARTICLES-PLAN.md` is the
    31st modified file.
- The 114 `v3_para.py` candidates behind a nearer non-lane file:
  - 1 lies in an unmoved region (`codes.rs:900`). <!-- doc-anchor-ignore -->
  - 109 were read against the lane target at `e6115223`. None holds its sentence there; each cites
    the nearer file (`system_meta.rs`, `rhi_impl/device.rs`, `runner.rs`, `vb.rs`, `vb_probe_dump.rs`,
    ...).
  - 4 are the refused `loom_pool.rs` numbers.

**(2) Quotations: nine lines restored, 16 numbers.** The orchestrator named seven. The other two
turned up when the same question was put to every line the passes rewrote: does this sentence report
what a document said at a named commit? Each restore was checked in three places:
- the pass log, for the pre-pass text;
- the source commit, for the text that makes it a quotation;
- the tree, for the line after the restore.

`w2_pass5_apply.py --dry` printed all three before the write. Every restored line differs from
`97bcf826` only in digits, and ~~7 of the 9~~ *[pass 7: 8 of the 9. Pass 6 restored `:1508`'s `PINS.toml:327-359` to `309-341`, so that line is byte-identical to `97bcf826` now - Pass 7, (4)]* are byte-identical to it. ~~The other two are 1010 and 1508,~~ *[pass 7: The other one is 1010,]* <!-- doc-anchor-ignore -->
whose neighbouring present-tense numbers stay moved.

| line | pass wrote | restored to | what makes it a quotation |
|---|---|---|---|
| `VG-R3-P2-CAPABILITY-SPLIT-PLAN.md:882` | p1: `PINS.toml:75-82` | `PINS.toml:57-64` | "the critique cited that note as"; `9e80cd4e`'s plan, line 540 (round 1, M5): "`PINS.toml:57-64` records that this test already caught exactly this omission" <!-- doc-anchor-ignore --> |
| `VG-R3-P2-CAPABILITY-SPLIT-PLAN.md:1010` | p1: `:299-302` | `:281-284` | "round 1's anchor"; `9e80cd4e`'s plan, line 536 (M3): `PINS.toml:281-284` <!-- doc-anchor-ignore --> |
| `VG-R3-P2-CAPABILITY-SPLIT-PLAN.md:1508` *(found)* | p1: `PINS.toml:75-82` | `PINS.toml:57-64` | this line IS round 1's M5, carried in the plan, the text `:882` quotes (`9e80cd4e`, line 540) <!-- doc-anchor-ignore --> |
| `VG-R3-P4-CONFIG-AND-INSTRUMENT-PLAN.md:260` *(found)* | p1: `PINS.toml:499` | `PINS.toml:481` | "`PINS.toml:481` said otherwise; P4-7 repaired that comment": `799db999`'s plan, line 258, "`PINS.toml:481` says otherwise", and `799db999`'s `PINS.toml` line 481 is "...the NAMED DISARM ROUTE until piece 4..." - the same token as `:797` <!-- doc-anchor-ignore --> |
| `VG-R3-P4-CONFIG-AND-INSTRUMENT-PLAN.md:796` | p1: `395-396` | `PINS.toml:377-378` | the "what this plan said" column; `799db999`, line 776 <!-- doc-anchor-ignore --> |
| `VG-R3-P4-CONFIG-AND-INSTRUMENT-PLAN.md:797` | p1: `499` | `PINS.toml:481` | the same column; `799db999`, line 776 <!-- doc-anchor-ignore --> |
| `VG-R3-P4-CONFIG-AND-INSTRUMENT-PLAN.md:798` | p1: `428` | `PINS.toml:410` | the same column; `799db999`, line 779: "`PINS.toml:410` says "TWO of the 26 pins"" <!-- doc-anchor-ignore --> |
| `threadpool/KE16-RESULTS.md:1826` | p1: `497:20` | `miri_scope.rs:492:20` | a verbatim Miri trace, first written at `611999cb`. There, `miri_scope.rs:492` column 20 is the `Arc::new` of `outer_wid` in `nested_scope_from_worker_is_stolen_by_sibling`, the test the trace names <!-- doc-anchor-ignore --> |
| `logging/05-LADDER-GATES.md:231` | p2: `2122` / `3118,3176,3207` | `device.rs:2110` / `3100,3158,3189` | "L7's row cites line numbers that have drifted": the row at `b1725b32^`, `03-CODES-REGISTRY.md:313` `device.rs:2110` and `:314` `device.rs:3100,3158,3189` <!-- doc-anchor-ignore --> |

Why the two found ones are in the class:
- **`:1508`.** Restoring `:882` alone would leave the plan saying the critique cited <!-- doc-anchor-ignore -->
  `PINS.toml:57-64` while showing the critique citing `PINS.toml:75-82`. It is also the wrong-file <!-- doc-anchor-ignore -->
  class pass 4 restored at `:1504`/`:1506`. The number means `vg_density_census.rs:57-64` (lines <!-- doc-anchor-ignore -->
  883-884 say so), and pass 1 moved it by the `PINS.toml` map. As a `PINS.toml` citation it is false
  in either numbering. Restored, it is right for the file it means.
- **`:260`.** The sentence is in the past tense, about a comment P4-7 rewrote. No HEAD number can be <!-- doc-anchor-ignore -->
  true of it, and 499 is `sha256_hwrt = "PENDING"`.

What the restores do not change:
- `:883`'s `PINS.toml:75-82` stays moved: it is present tense and holds at HEAD. <!-- doc-anchor-ignore -->
- `:1010`'s `:318`/`:319` and `:1011` stay moved. <!-- doc-anchor-ignore -->
- `:1504`'s `PINS.toml:299-302` stays moved. *[pass 6: restored to 281-284 - the line is under the critique's "Every line number below is as-of-the-critique" (`:1426`); Pass 6]* It is round 1's own text, but its citation was TRUE at <!-- doc-anchor-ignore -->
  `e6115223` - class (1) - so it stays true, and `:1010`'s "round 1's anchor `:281-284` had MOVED" <!-- doc-anchor-ignore -->
  is true beside it.
- `05-LADDER-GATES.md:231`'s `3158` and `3189` were two of the verifier's 152 TRUE numbers. The <!-- doc-anchor-ignore -->
  restore takes them out of the citation class on purpose: the sentence quotes them as drifted, it
  does not cite them.

Two more groups could qualify for this class. They are named here and **not restored.** *[pass 6: restored, by the orchestrator's ruling on dated records - Pass 6]* They record
what a source file held at a named commit, not what a document said. Both are false as present-tense
citations whichever way they are numbered, and true only as records of their commit. They fall
inside the orchestrator's (3), so restoring them is a scope call, and it is put to you:
- `logging/05-LADDER-GATES.md:236`, `:237`, `:245`. The paragraph is "Measured at HEAD, before a line <!-- doc-anchor-ignore -->
  of L7 was written" (`b1725b32`). There, `device.rs:3788` is the within-file test's SKIP `eprintln!` <!-- doc-anchor-ignore -->
  and 3732 is ~~`mod tests {`~~ *[pass 7: `#[cfg(test)]`, the first line of the `#[cfg(test)] mod tests` the ladder says opens at `:3732`; `mod tests {` is 3733 - Pass 7, (5)]*. Pass 1 moved these to 3806 / 3750. <!-- doc-anchor-ignore -->
- `threadpool/KE16-DESIGN.md:359`, `:360` and `KE16-DESIGN-SPACE.md:554`. These are tables of the <!-- doc-anchor-ignore -->
  stale references inside `loom_pool.rs` at `778739f0`: its line 140 said `worker.rs:78` and its <!-- doc-anchor-ignore -->
  line 343 said `worker.rs:86`. Both references have since been removed from the file. <!-- doc-anchor-ignore -->

**(3) The debt: ~~212 numbers on 94 lines~~ *[pass 6: 216 numbers on 96 lines - F3 adds `KE16-DESIGN-B4.md:378` and `:385`, 4 numbers; 149 on 71 after pass 6's dated-record restores]* *[pass 7: 170 on 80 - (1) adds five `device.rs` numbers, the link form two, and six lines pass 6 restored are pre-lane rot, 14 numbers; Pass 7]* *[pass 8: 153 on 73 - the five lines of `KE16-DESIGN-B4.md`'s round-2 critique, 9 numbers, are restored as dated records, and `VB-SV0-SDF-SHADOW-PLAN.md:1627`-`1628`, 8 numbers, are dated records kept as written; Pass 8]*, every one by name.** Each of them was already false at <!-- doc-anchor-ignore -->
`e6115223`. The lane did not make it false, and the lane's line map cannot make it true, so none of
it belongs to this lane. This is the by-name list that pass 3's pre-lane paragraph, pass 4's
residuals 6, 7 and 9, and the pass-4 verifier's "62 on 34 lines no register entry names" each covered
only in part. Pass 2's 53 NOT-CONFIRMED lines map onto it as follows:
- ~~43~~ *[pass 6: 44 - `KE16-DESIGN-B4.md:378` is the 44th]* are in the table; <!-- doc-anchor-ignore -->
- 5 are the `.cargo/config.toml` lines, which were lane rot, repaired by pass 4;
- ~~3~~ *[pass 6: 2]* hold at HEAD and were false alarms of pass 2's heuristic: `VG-R3-P2-CAPABILITY-SPLIT-PLAN.md:517` <!-- doc-anchor-ignore -->
  and `:883`~~, and `KE16-DESIGN-B4.md:378`~~ *[pass 6: not this one - at HEAD its `:575` reads "**FIRES.** The winner is `a3`" and its unmoved `:12` reads "| B | CLOSED ON `b1`"; it is in the table now - Pass 6, F3]*; <!-- doc-anchor-ignore -->
- 2 are quotations restored in (2): `:1010` and `VG-R3-P4-CONFIG-AND-INSTRUMENT-PLAN.md:798`. <!-- doc-anchor-ignore -->

The table covers:
- 150 numbers the verifier judged never true at `e6115223`: `device.rs` 65 (the other 15 of its 80
  are 13 TRUE and the 2 quoted at `:231`), `PINS.toml` 83 and `miri_scope.rs` 2; <!-- doc-anchor-ignore -->
- the 35 `loom_pool.rs` numbers written against `778739f0` / `7fdd738e`;
- the 6 near misses;
- the 21 continuation numbers pass 4 refused.

It leaves out the three numbers restored above as quotations. Every row whose "moved by" names a pass
is *moved, still false - the move neither helped nor harmed*. The columns:
- **last written** is `git blame` of the citing line at `e6115223`.
- **HEAD by content** takes the line the number held in that commit and follows it to `97bcf826`
  through difflib equal blocks, never by arithmetic. It is where the author's line is now: a
  candidate for a future derivation, not a verified repair. It inherits any mistake the author made.
  A `·` marks a number that held only a brace or a blank line. A `?` marks a row whose line was last
  written when its number was already stale, so the follow has nothing to start from. `gone` means
  the text no longer exists. The notes give the target by name where it was read.

Also named, and not repeated in the table:
- pass 2's 15 remaining ambiguous clusters (the 16th is `PARTICLES-PLAN.md:183`, now written); <!-- doc-anchor-ignore -->
- `VG-R3-P2-CAPABILITY-SPLIT-PLAN.md:877-878` and `:881`, which cite `vg_density_census.rs`, a <!-- doc-anchor-ignore -->
  non-lane file;
- `logging/00-GOAL-TARGETS.md:182`, the historical record; <!-- doc-anchor-ignore -->
- the 4 frozen `docs/ru/` citations.

| citing line | numbers now | moved by | last written | HEAD by content | note |
|---|---|---|---|---|---|
| `crates/boyko_log/src/census.rs:261` | 3118 | p1 | `eadd2ff6` *(stale already)* | 3118→? | quotes the corpus's worked example `site=device.rs:N`, and carries the same number as that example (`LOGGING-SYSTEM-PLAN.md:454`) - consistent with it, false with it <!-- doc-anchor-ignore --> |
| `LOGGING-SYSTEM-PLAN.md:75` | 3118 | p1 | `303a7a92` | 3118→gone | the DDGI-storage `eprintln!` when written; since L7b it is `report_ddgi_storage_unsupported`, 3168-3180 (`W2102` at 3176) <!-- doc-anchor-ignore --> |
| `LOGGING-SYSTEM-PLAN.md:424` | 3118, 3176 | p1 | `053f6c9f` | 3118→gone, 3176→gone | the DDGI-storage `eprintln!` when written; since L7b it is `report_ddgi_storage_unsupported`, 3168-3180 (`W2102` at 3176); the trio at HEAD is the three `report_*` functions, 3171 / 3192 / 3208 **[pass 7: 3176 added. It was the shadow-denoise site when written; at `e6115223` 3158 is the DDGI reporter's `W2102,`, and at HEAD the shadow-denoise reporter is at 3192 (`W2102.number()` 3197) - Pass 7, (1)]** <!-- doc-anchor-ignore --> |
| `LOGGING-SYSTEM-PLAN.md:454` | 3118 | p1 | `303a7a92` | 3118→gone | the DDGI-storage `eprintln!` when written; since L7b it is `report_ddgi_storage_unsupported`, 3168-3180 (`W2102` at 3176); a worked example of an output line <!-- doc-anchor-ignore --> |
| `LOGGING-SYSTEM-PLAN.md:1949` | 2122 | p1 | `5ebcd585` | 2122→2237 | `sync_validation_available = ... is_instance_extension_present(global, VK_EXT_VALIDATION_FEATURES_EXTENSION_NAME)` <!-- doc-anchor-ignore --> |
| `LOGGING-SYSTEM-PLAN.md:2012` | 2122 | p1 | `5ebcd585` | 2122→2237 | the `E2101` site: condition 2237-2239, reports 2144 / 2166 <!-- doc-anchor-ignore --> |
| `LOGGING-SYSTEM-PLAN.md:2013` | 3118, 3176 | p1 | `053f6c9f` | 3118→gone, 3176→gone | the DDGI-storage `eprintln!` when written; since L7b it is `report_ddgi_storage_unsupported`, 3168-3180 (`W2102` at 3176); the trio at HEAD is the three `report_*` functions, 3171 / 3192 / 3208 **[pass 7: 3176 added. It was the shadow-denoise site when written; at `e6115223` 3158 is the DDGI reporter's `W2102,`, and at HEAD the shadow-denoise reporter is at 3192 (`W2102.number()` 3197) - Pass 7, (1)]** <!-- doc-anchor-ignore --> |
| `MULTI-PARADIGM-RENDER-PLAN.md:46` | 2658, 2680 | p1 | `60963e50` | 2658→3056, 2680→3078· | the RT feature probe's `bda_feat` literal; the hwrt-only enable is `_rt_bda` at 3730-3736 <!-- doc-anchor-ignore --> |
| `OPEN-QUESTIONS.md:2754` | 2362 | p1 | `66148ae1` | 2362→2486 | the `enable_validation && BOYKO_DISABLE_VALIDATION` conjunction (register pass 4: 2486) <!-- doc-anchor-ignore --> |
| `OPEN-QUESTIONS.md:4189` | 3118 | p1 | `555ed0c4` *(stale already)* | 3118→? | quotes the corpus's worked example, with its number; the example's site is the DDGI one, 3168-3180 <!-- doc-anchor-ignore --> |
| `OPEN-QUESTIONS.md:4209` | 3118 | p1 | `eadd2ff6` *(stale already)* | 3118→? | the corpus's worked example, with its number; the DDGI site is 3168-3180 <!-- doc-anchor-ignore --> |
| `PBR-MATERIALS-PLAN.md:32` | 1809, 1854 | p1 | `8888bf04` | 1809→3229, 1854→3584· | `query_device_caps` is 3229 (signature unchanged, outside difflib's blocks) to 3584 <!-- doc-anchor-ignore --> |
| `PBR-MATERIALS-PLAN.md:433` | 1809, 1854 | p1 | `8888bf04` | 1809→3229, 1854→3584· | `query_device_caps` 3229-3584 <!-- doc-anchor-ignore --> |
| `VB-P1E-HIERARCHICAL-CULL-PLAN.md:229` | 2099 | p1 | `9a6593e8` | 2099→2282 | the `SYNCHRONIZATION_VALIDATION` array, 2282 <!-- doc-anchor-ignore --> |
| `VB-P1E-HIERARCHICAL-CULL-PLAN.md:370` | 2602 | p2 | `9a6593e8` | 2602→2785 | `subgroup_size_control: VK_FALSE`, 2785 <!-- doc-anchor-ignore --> |
| `VB-P1E-HIERARCHICAL-CULL-PLAN.md:675` | 2602 | p1 | `dc0684ef` | 2602→2785 | the same field <!-- doc-anchor-ignore --> |
| `VB-P1E-HIERARCHICAL-CULL-PLAN.md:2248` | 2099 | p1 | `9a6593e8` | 2099→2282 | the same array <!-- doc-anchor-ignore --> |
| `VB-P1E-HIERARCHICAL-CULL-PLAN.md:2571` | 2099 | p2 | `9a6593e8` | 2099→2282 | the same array (register pass 2: 2282) <!-- doc-anchor-ignore --> |
| `VB-P1E-HIERARCHICAL-CULL-PLAN.md:3112` | 2602 | p2 | `9a6593e8` | 2602→2785 | the same field <!-- doc-anchor-ignore --> |
| `VB-P1E-HIERARCHICAL-CULL-PLAN.md:3184` | 2602 | p2 | `dc0684ef` | 2602→2785 | the same field <!-- doc-anchor-ignore --> |
| `VB-P1E-HIERARCHICAL-CULL-PLAN.md:3452` | ~~2099, 2100, 2107~~ 2087, 2088, 2095 | p1 | `9a6593e8` | 2099→2282, 2100→2283, 2107→2290· | array 2282, `VkValidationFeaturesExt` 2283-2290 **[pass 6: restored as a dated record - no longer debt]** <!-- doc-anchor-ignore --> |
| `VB-P1E-HIERARCHICAL-CULL-PLAN.md:3453` | ~~2602~~ 2584 | p1 | `9a6593e8` | 2602→2785 | the same field **[pass 6: restored as a dated record - no longer debt]** <!-- doc-anchor-ignore --> |
| `VB-PERFORMANCE-TRACK.md:235` | 723 | p2 | `15199271` | 723→1012 | `[vb_mesh_ssao]` (register pass 2: 1012) <!-- doc-anchor-ignore --> |
| `VB-PERFORMANCE-TRACK.md:266` | ~~281, 310~~ 263, 292 | p1 | `15199271` | 281→289, 310→318 | `[vb_mesh]` / its `sha256_software` **[pass 6: restored as a dated record - no longer debt]** <!-- doc-anchor-ignore --> |
| `VB-PERFORMANCE-TRACK.md:271` | 746, 776, 785, 818 | p1 | `15199271` | 746→1035, 776→1065, 785→1074, 818→1107 | `[vb_mesh_froxel]` / sha, `[vb_mesh_tex_froxel]` / sha <!-- doc-anchor-ignore --> |
| `VB-PERFORMANCE-TRACK.md:273` | 723, 737 | p1/p2 | `15199271` | 723→1012, 737→1026 | `[vb_mesh_ssao]` / its sha <!-- doc-anchor-ignore --> |
| `VB-SV0-SDF-SHADOW-PLAN.md:243` | 301, 303 | p1 | `3da55688` | 301→309, 303→311 | `[vb_mesh]`'s "real bless output" note, 309-311 <!-- doc-anchor-ignore --> |
| `VB-SV0-SDF-SHADOW-PLAN.md:357` | 306, 309 | p1 | `62731d91` | 306→304, 309→307 | NEAR MISS: exact when written, 2 lines off at `e6115223`, 2 lines off at HEAD; the note is 304-307 <!-- doc-anchor-ignore --> |
| `VB-SV0-SDF-SHADOW-PLAN.md:952` | 301, 303 | p1 | `3da55688` | 301→309, 303→311 | the same note <!-- doc-anchor-ignore --> |
| `VB-SV0-SDF-SHADOW-PLAN.md:1055` | 340 | p1 | `62731d91` | 340→610 | `[vb_both]`'s "boot-seeded EMPTY" <!-- doc-anchor-ignore --> |
| `VB-SV0-SDF-SHADOW-PLAN.md:1056` | 373 | p2 | `62731d91` | 373→734 | `[vb_sdf_only]`'s "boot-seeded EMPTY" <!-- doc-anchor-ignore --> |
| `VB-SV0-SDF-SHADOW-PLAN.md:1521` | 340, 373 | p1 | `62731d91` | 340→610, 373→734 | the two notes above <!-- doc-anchor-ignore --> |
| `VB-SV0-SDF-SHADOW-PLAN.md:1625` | ~~281, 329, 301, 303~~ 263, 311, 283, 285 | p1 | `3da55688` | 281→289, 329→611, 301→309, 303→311 | `[vb_mesh]` 289-325 and its bless note 309-311; the range end 311 held a line that is in `[vb_both]` at HEAD **[pass 6: restored as a dated record - no longer debt]** <!-- doc-anchor-ignore --> |
| `VG-R3-P2-CAPABILITY-SPLIT-PLAN.md:518` | 777 | p1 | `87cd3f61` | 777→1018 | "The pin run itself exercises every new declare/record parity" <!-- doc-anchor-ignore --> |
| `VG-R3-P2-CAPABILITY-SPLIT-PLAN.md:914` | 2164 | p1 | `87cd3f61` | 2164→2282 | the `SYNCHRONIZATION_VALIDATION` array, 2282 <!-- doc-anchor-ignore --> |
| `VG-R3-P2-CAPABILITY-SPLIT-PLAN.md:1192` | 777 | p1 | `87cd3f61` | 777→1018 | the same pin comment <!-- doc-anchor-ignore --> |
| `VG-R3-P2-CAPABILITY-SPLIT-PLAN.md:1371` | 371, 376, 495, 501 | p1 | `87cd3f61` | 371→612, 376→617, 495→736, 501→742 | the pre-fill-and-verify comments of `[vb_both]` and `[vb_sdf_only]` <!-- doc-anchor-ignore --> |
| `VG-R3-P2-CAPABILITY-SPLIT-PLAN.md:1500` | ~~2164, 2165, 2172, 2199, 2205, 2122, 2123~~ 2152, 2153, 2160, 2187, 2193, 2110, 2111 | p1/p3b | `9e80cd4e` | 2164→2282, 2165→2283, 2172→2290·, 2199→2317, 2205→2323·, 2122→2237, 2123→2238 | round-1 critique text (`9e80cd4e`); array 2282, struct 2283-2290, `p_next` head 2317-2323, condition 2237-2238 **[pass 6: restored as a dated record - no longer debt]** <!-- doc-anchor-ignore --> |
| `VG-R3-P2-CAPABILITY-SPLIT-PLAN.md:1538` | ~~2164, 2199, 2205, 2122, 2134~~ 2152, 2187, 2193, 2110, 2122 | p2/p3 | `9e80cd4e` | 2164→2282, 2199→2317, 2205→2323·, 2122→2237, 2134→2252· | round-1 critique text (`9e80cd4e`); same targets as `:1500` **[pass 6: restored as a dated record - no longer debt]** <!-- doc-anchor-ignore --> |
| `VG-R3-P2-CAPABILITY-SPLIT-PLAN.md:1548` | ~~373, 376, 497, 501~~ 355, 358, 479, 483 | p1 | `9e80cd4e` | 373→614, 376→617, 497→738, 501→742 | round-1 critique; the same two comments **[pass 6: restored as a dated record - no longer debt]** <!-- doc-anchor-ignore --> |
| `VG-R3-P2-CAPABILITY-SPLIT-PLAN.md:1556` | ~~777~~ 759 | p1 | `9e80cd4e` | 777→1018 | round-1 critique; the same pin comment **[pass 6: restored as a dated record - no longer debt]** <!-- doc-anchor-ignore --> |
| `VG-R3-P3-CULL-INTEGRATION-PLAN.md:40` | 415, 447, 491, 538, 616, 668, 720, 756, 796, 826, 849, 888, 930, 963, 412 | p3 | `678f60f7` | 415→601, 447→633, 491→677, 538→724, 616→802, 668→854, 720→906, 756→942, 796→982, 826→1012, 849→1035, 888→1074, 930→1116, 963→1149, 412→429 | "26 top-level tables" - all 14 flagged numbers were headers when written; `BOYKO_VG_OCC` 429 <!-- doc-anchor-ignore --> |
| `VG-R3-P3-CULL-INTEGRATION-PLAN.md:117` | ~~1172, 3279~~ 1160, 3261 | p1/p2 | `8c9c1328` | 1172→1229, 3279→3586 | `get_device_queue` 1229; "one queue from `queue_family_index`" 3586 (`queue_count: 1` at 3662) **[pass 6: restored as a dated record - no longer debt]** <!-- doc-anchor-ignore --> |
| `VG-R3-P3-CULL-INTEGRATION-PLAN.md:1020` | 408, 413, 412 | p1 | `678f60f7` | 408→425, 413→442, 412→429 | `[vb_occ_split.env]` 425-442; its end 413 was the `gnu` `RUSTUP_TOOLCHAIN` line, now the `msvc` 442 (REPL) <!-- doc-anchor-ignore --> |
| `VG-R3-P3-CULL-INTEGRATION-PLAN.md:1862` | 408, 413, 412 | p3 | `678f60f7` | 408→425, 413→442, 412→429 | as `:1020` <!-- doc-anchor-ignore --> |
| `VG-R3-P3-CULL-INTEGRATION-PLAN.md:1998` | 408, 413 | p1 | `678f60f7` | 408→425, 413→442 | as `:1020` <!-- doc-anchor-ignore --> |
| `VG-R3-P4-CONFIG-AND-INSTRUMENT-PLAN.md:50` | ~~467, 469~~ 449, 451 | p1 | `799db999` | 467→481, 469→483 | plan D12's fixed point, 481-483 **[pass 6: restored as a dated record - no longer debt]** <!-- doc-anchor-ignore --> |
| `VG-R3-P4-CONFIG-AND-INSTRUMENT-PLAN.md:51` | ~~416, 602~~ 398, 584 | p1 | `799db999` | 416→422, 602→624 | NEAR MISS at `e6115223`; exact when written: the two `sha256_software` lines **[pass 6: restored as a dated record - no longer debt]** <!-- doc-anchor-ignore --> |
| `VG-R3-P4-CONFIG-AND-INSTRUMENT-PLAN.md:52` | ~~411, 412, 479, 480, 504, 505, 533, 534, 561, 562~~ 393, 394, 461, 462, 486, 487, 515, 516, 543, 544 | p1/p2 | `799db999` | 411→417, 412→418, 479→493, 480→494, 504→526, 505→527, 533→555, 534→556, 561→583, 562→584 | the five occlusion pins' `test_binary` / `test_name` **[pass 6: restored as a dated record - no longer debt]** <!-- doc-anchor-ignore --> |
| `VG-R3-P4-CONFIG-AND-INSTRUMENT-PLAN.md:53` | ~~447, 450~~ 429, 432 | p1 | `799db999` | 447→461, 450→464 | the MACHINE-CHECKED note of the `[vb_occ_mixed*]` family **[pass 6: restored as a dated record - no longer debt]** <!-- doc-anchor-ignore --> |
| `VG-R3-P4-CONFIG-AND-INSTRUMENT-PLAN.md:133` | 452, 457 | p1 | `799db999` | 452→466, 457→471 | THE VARIABLE LADDER, 466-471 <!-- doc-anchor-ignore --> |
| `VG-R3-P4-CONFIG-AND-INSTRUMENT-PLAN.md:201` | 429, 492, 517, 546, 574 | p1 | `799db999` | 429→441, 492→506, 517→539, 546→568, 574→596 | the five `BOYKO_VG_HZB = "1"` <!-- doc-anchor-ignore --> |
| `VG-R3-P4-CONFIG-AND-INSTRUMENT-PLAN.md:242` | 519, 576 | p1 | `799db999` | 519→541, 576→598 | the two `BOYKO_VG_OCC_FORCE` <!-- doc-anchor-ignore --> |
| `VG-R3-P4-CONFIG-AND-INSTRUMENT-PLAN.md:717` | 402, 406 | p1 | `799db999` | 402→408, 406→412 | NEAR MISS at `e6115223`; exact when written: sync validation not live, 408-412 <!-- doc-anchor-ignore --> |
| `VG-R3-P4-CONFIG-AND-INSTRUMENT-PLAN.md:767` | 429, 492, 517, 546, 574 | p1 | `799db999` | 429→441, 492→506, 517→539, 546→568, 574→596 | the same five <!-- doc-anchor-ignore --> |
| `archive/LIGHTING-L0-L1-PLAN.md:181` | 1850, 1853 | p1 | `c204ab82` | 1850→3510, 1853→3569· | the `DeviceCaps { .. }` literal; the struct itself is 200 (register pass 4) <!-- doc-anchor-ignore --> |
| `archive/LIGHTING-L0-L1-PLAN.md:801` | 1850, 1853, 1831, 1854 | p3 | `c204ab82` | 1850→3510, 1853→3569·, 1831→3265, 1854→3584· | `DeviceCaps` literal and `query_device_caps` (3229-3584) <!-- doc-anchor-ignore --> |
| `archive/LIGHTING-L0-L1-PLAN.md:1047` | 1850, 1853 | p1 | `c204ab82` | 1850→3510, 1853→3569· | as `:181` <!-- doc-anchor-ignore --> |
| `archive/LIGHTING-L0-L1-PLAN.md:1048` | 1831, 1854 | p2 | `c204ab82` | 1831→3265, 1854→3584· | `query_device_caps`, 3229-3584 <!-- doc-anchor-ignore --> |
| `diagnostics/logging/00-GOAL-TARGETS.md:37` | 3118 | p1 | `d02f74a3` | 3118→gone | the DDGI-storage `eprintln!` when written; since L7b it is `report_ddgi_storage_unsupported`, 3168-3180 (`W2102` at 3176) <!-- doc-anchor-ignore --> |
| `diagnostics/logging/00-GOAL-TARGETS.md:150` | ~~2122, 2122, 2123~~ 2110, 2110, 2111 | p1 | `d02f74a3` | 2122→2237, 2122→2237, 2123→2238 | same conjunction, 2237-2238 **[pass 6: restored as a dated record - no longer debt]** <!-- doc-anchor-ignore --> |
| `diagnostics/logging/00-GOAL-TARGETS.md:191` | ~~2122~~ 2110 | p2 | `d02f74a3` | 2122→2237 | a record of a verification; the conjunction is 2237-2238 **[pass 6: restored as a dated record - no longer debt]** <!-- doc-anchor-ignore --> |
| `diagnostics/logging/01-EMISSION-RING.md:253` | ~~3118~~ 3100 | p1 | `d02f74a3` | 3118→gone | the DDGI-storage `eprintln!` when written; since L7b it is `report_ddgi_storage_unsupported`, 3168-3180 (`W2102` at 3176); the trio at HEAD is the three `report_*` functions, 3171 / 3192 / 3208 **[pass 6: restored as a dated record - no longer debt]** <!-- doc-anchor-ignore --> |
| `diagnostics/logging/01-EMISSION-RING.md:297` | 3118 | p1 | `d02f74a3` | 3118→gone | the DDGI-storage `eprintln!` when written; since L7b it is `report_ddgi_storage_unsupported`, 3168-3180 (`W2102` at 3176); a worked example of an output line <!-- doc-anchor-ignore --> |
| `diagnostics/logging/03-CODES-REGISTRY.md:359` | 2122 | p1 | `b30fa810` *(stale already)* | 2122→? | the `E2101` condition is 2237-2239 and its two reports 2144 / 2166 <!-- doc-anchor-ignore --> |
| `diagnostics/logging/03-CODES-REGISTRY.md:360` | 3118, 3176 | p1 | `b30fa810` *(stale already)* | 3118→?, 3176→? | the trio at HEAD is 3171 / 3192 / 3208 (`report_*`); its parenthetical `eprintln!` is gone since L7b **[pass 7: 3176 added. Stale when this line was written: at `b30fa810` 3158 is already the DDGI reporter's line. Its "at HEAD" is read in Pass 7, (7)]** <!-- doc-anchor-ignore --> |
| `diagnostics/logging/05-LADDER-GATES.md:236` | ~~3806~~ 3788 | p1 | `b1725b32` | 3806→3919 | DATED RECORD ("Measured at HEAD", `b1725b32`, where 3788 is the SKIP `eprintln!`) - true there; see (2) **[pass 6: restored as a dated record - no longer debt]** <!-- doc-anchor-ignore --> |
| `diagnostics/logging/05-LADDER-GATES.md:237` | ~~3750~~ 3732 | p2 | `b1725b32` | 3750→3862 | DATED RECORD, `b1725b32`: ~~`mod tests {` at 3732~~ [pass 7: `#[cfg(test)]` at 3732, opening the `mod tests {` at 3733] - true there; see (2) **[pass 6: restored as a dated record - no longer debt]** <!-- doc-anchor-ignore --> |
| `diagnostics/logging/05-LADDER-GATES.md:245` | ~~3806~~ 3788 | p1 | `b1725b32` | 3806→3919 | DATED RECORD, as `:236` **[pass 6: restored as a dated record - no longer debt]** <!-- doc-anchor-ignore --> |
| `threadpool/KE16-DESIGN-A.md:494` | 169, 192, 234, 243 | p2 | `778739f0` | 169→217, 192→gone, 234→gone, 243→gone | fidelity note header 217; the push-transport fence (217 when written) is now `publish_fence()` at 328; this line moved, `:497` did not <!-- doc-anchor-ignore --> |
| `threadpool/KE16-DESIGN-APP.md:275` | ~~169, 192, 234, 243~~ 143, 166, 208, 217 | p2 | `778739f0` | 169→217, 192→gone, 234→gone, 243→gone | as `KE16-DESIGN-A.md:494` ~~**[pass 6: restored as a dated record - no longer debt]**~~ **[pass 7: pre-lane rot returned to its original text. No dating clause is written in its sentence, paragraph, heading or lead-in, so it is debt again - Pass 7, (7)]** <!-- doc-anchor-ignore --> |
| `threadpool/KE16-DESIGN-B4.md:72` | 234, 262, 356 | p1 | `d647d930` | 234→217, 262→244, 356→338 | written against the 1640-line file: fidelity note 217-244, steal-path fence comment 338 <!-- doc-anchor-ignore --> |
| `threadpool/KE16-DESIGN-B4.md:378` | ~~12, 575~~ 12, 531 | p1 (`:575`); `:12` never moved | `7fdd738e` | 12→gone, 531→582 | **[added by pass 6, F3]** round-2 critique text, ~~which declares no as-of for~~ ~~its line numbers~~ ~~[pass 7: these numbers. Its one as-of, at `:408`, scopes two `ke16_feature_scheme_census.rs` anchors on its own line - Pass 7, (6)]~~ [pass 8: the critique dates every site below its lead-in, `KE16-DESIGN-B4.md:312`, "Reviewed 2026-09-07 against `d647d930`. Every site below was re-verified by hand." And `:408` is the as-of sentence; the two anchors it scopes are on `:407` - Pass 8, (A), (E)]; follow from the numbers as written. At `7fdd738e` `:12` was "B0 BY CONSTRUCTION" and `:531` "axis A closes on `a3`"; at `e6115223` `:12` reads "CLOSED ON `b1`" and the quote is 538; at HEAD `:12` still reads "CLOSED ON `b1`", 575 is "**FIRES.** The winner is `a3`", the quote is 582 **[pass 8: restored to `:531` as a dated record of the round-2 critique. At `d647d930` `KE16-RESULTS.md:12` is "B0 BY CONSTRUCTION, NOT BY MEASUREMENT" and `:531` is "axis A closes on `a3` and axis B stops" - no longer debt]** <!-- doc-anchor-ignore --> |
| `threadpool/KE16-DESIGN-B4.md:385` | ~~1202, 1207~~ 1158, 1163 | p1 | `7fdd738e` | 1158→2071, 1163→2076 | **[added by pass 6, F3]** the `a1` worker-route block (`top_lane=13/24/21`, HEAD 2073); follow from the numbers as written. At `e6115223` 1158-1163 are rows of the `wg`/`w0` table and the block is 2027-2032; HEAD 1202-1207 is that same table **[pass 8: restored to 1158-1163 as a dated record of the round-2 critique (`:312`). At `d647d930` it is the `a1` worker-route block, `top_lane=13/24/21` at 1160 - no longer debt]** <!-- doc-anchor-ignore --> |
| `threadpool/KE16-DESIGN-B4.md:421` | ~~572, 577~~ 567, 572 | p1 | `7fdd738e` | 572→gone, 577→gone | the `#[cfg_attr(not(any(a1, a1-fifo)), ignore)]` it names no longer exists: `miri_scope.rs` has no `cfg_attr` at HEAD **[pass 8: restored to 567-572 as a dated record of the round-2 critique (`:312`). At `d647d930` 567-572 is that `#[cfg_attr(` ... `)]`, on `nested_scope_inline_body_spawns_through_tls_deque_under_live_join` - no longer debt]** <!-- doc-anchor-ignore --> |
| `threadpool/KE16-DESIGN-B4.md:456` | ~~366~~ 340 | p1 | `7fdd738e` | 366→348 | the steal-transport fence, 348 **[pass 8: restored to 340 as a dated record of the round-2 critique (`:312`). At `d647d930` 340 is `fence(Ordering::SeqCst); // injector steal transport fence` - no longer debt]** <!-- doc-anchor-ignore --> |
| `threadpool/KE16-DESIGN-B4.md:457` | ~~259, 266~~ 233, 240 | p2 | `7fdd738e` | 259→241, 266→248 | the consumer-fence note, 241-248 **[pass 8: restored to 233-240 as a dated record of the round-2 critique (`:312`). At `d647d930` 233-240 is the note "The CONSUMER's fence is still the transport's, and is modelled" - no longer debt]** <!-- doc-anchor-ignore --> |
| `threadpool/KE16-DESIGN-SPACE.md:554` | ~~166, 369~~ 140, 343 | p2 | `778739f0` | 166→gone, 369→gone | ~~DATED RECORD~~ of stale references INSIDE `loom_pool.rs` at `778739f0` (`worker.rs:78`, `:86`); both were removed from the file ~~**[pass 6: restored as a dated record - no longer debt]**~~ **[pass 7: pre-lane rot returned to its original text. No dating clause is written in its sentence, paragraph, heading or lead-in, so it is debt again - Pass 7, (7)]** <!-- doc-anchor-ignore --> |
| `threadpool/KE16-DESIGN-W.md:138` | 169, 192 | p2 | `778739f0` | 169→217, 192→gone | fidelity note header 217 <!-- doc-anchor-ignore --> |
| `threadpool/KE16-DESIGN-W.md:153` | 226, 291, 243 | p3 | `778739f0` | 226→311, 291→440·, 243→gone | the M2 test is `loom_m2_idle_race_c_no_lost_wakeup`, 311-; its producer fence is now the `publish_fence()` call at 328; this line moved, 156/160/163 did not <!-- doc-anchor-ignore --> |
| `threadpool/KE16-DESIGN.md:356` | ~~129, 140~~ 103, 114 | p1 | `778739f0` | 129→173, 140→gone | the join-wait note 173; the transport sentence (114 when written) is gone ~~**[pass 6: restored as a dated record - no longer debt]**~~ **[pass 7: pre-lane rot returned to its original text. No dating clause is written in its sentence, paragraph, heading or lead-in, so it is debt again - Pass 7, (7)]** <!-- doc-anchor-ignore --> |
| `threadpool/KE16-DESIGN.md:357` | ~~169, 192, 234, 243~~ 143, 166, 208, 217 | p1/p2 | `778739f0` | 169→217, 192→gone, 234→gone, 243→gone | as `KE16-DESIGN-A.md:494` ~~**[pass 6: restored as a dated record - no longer debt]**~~ **[pass 7: pre-lane rot returned to its original text. No dating clause is written in its sentence, paragraph, heading or lead-in, so it is debt again - Pass 7, (7)]** <!-- doc-anchor-ignore --> |
| `threadpool/KE16-DESIGN.md:359` | ~~166~~ 140 | p1 | `778739f0` | 166→gone | ~~DATED RECORD,~~ as `KE16-DESIGN-SPACE.md:554` ~~**[pass 6: restored as a dated record - no longer debt]**~~ **[pass 7: pre-lane rot returned to its original text. No dating clause is written in its sentence, paragraph, heading or lead-in, so it is debt again - Pass 7, (7)]** <!-- doc-anchor-ignore --> |
| `threadpool/KE16-DESIGN.md:360` | ~~369~~ 343 | p2 | `778739f0` | 369→gone | ~~DATED RECORD,~~ as `KE16-DESIGN-SPACE.md:554` ~~**[pass 6: restored as a dated record - no longer debt]**~~ **[pass 7: pre-lane rot returned to its original text. No dating clause is written in its sentence, paragraph, heading or lead-in, so it is debt again - Pass 7, (7)]** <!-- doc-anchor-ignore --> |
| `threadpool/KE16-DESIGN.md:383` | 226, 291 | p1 | `778739f0` | 226→311, 291→440· | as `KE16-DESIGN-W.md:153` <!-- doc-anchor-ignore --> |
| `threadpool/KE16-RESULTS.md:178` | 413 | p1 | `611999cb` | 413→395 | `#[should_panic(expected = "M2: lost wake")]`, 395 <!-- doc-anchor-ignore --> |
| `threadpool/KE16-RESULTS.md:1116` | 234, 262, 356 | p1/p2 | `7f294afe` | 234→217, 262→244, 356→338 | as `KE16-DESIGN-B4.md:72` <!-- doc-anchor-ignore --> |
| `scripts/golden.ps1:163` | 2362 | p1 | `66148ae1` | 2362→2486 | the same conjunction, 2486 <!-- doc-anchor-ignore --> |
| `VB-SV0-SDF-SHADOW-PLAN.md:1627` | 313, 343, 322, 326, 328 | refused p4 | `62731d91` | 313→601, 343→631, 322→610, 326→614, 328→616 | refused by pass 4; `[vb_both]` 601-631, empty-list note 610, pre-fill note 614-616; 343 was the `gnu` toolchain line, now 631 (REPL) **[pass 8: a dated record kept as written, under the section's lead-in `:1542`, "Every line below was opened while writing this revision"; `62731d91` wrote both, and at `62731d91` `[vb_both]` is 313-343, the empty edit list 322 and the pre-fill note 326-328 - no longer debt]** <!-- doc-anchor-ignore --> |
| `VB-SV0-SDF-SHADOW-PLAN.md:1628` | 345, 377, 355 | refused p4 | `62731d91` | 345→724, 377→756, 355→734 | refused by pass 4; `[vb_sdf_only]` 724-756, note 734; 377 was the `gnu` toolchain line, now 756 (REPL) **[pass 8: a dated record kept as written, as `:1627`; at `62731d91` `[vb_sdf_only]` is 345-377 and its empty-list note 355 - no longer debt]** <!-- doc-anchor-ignore --> |
| `VG-R3-P2-CAPABILITY-SPLIT-PLAN.md:916` | 2153, 2160, 2187, 2193 | refused p4 | `87cd3f61` | 2153→2283, 2160→2290·, 2187→2317, 2193→2323· | refused by pass 4; struct 2283-2290, `p_next` head 2317-2323 <!-- doc-anchor-ignore --> |
| `archive/LIGHTING-L0-L1-PLAN.md:183` | 1819, 1842 | refused p4 | `c204ab82` | 1819→3265, 1842→3584· | refused by pass 4; `query_device_caps` 3229-3584 <!-- doc-anchor-ignore --> |
| `threadpool/KE16-DESIGN-A.md:497` | 147, 152 | refused p4 | `778739f0` | 147→221, 152→226 | refused by pass 4; the lost-wake window 221-226 <!-- doc-anchor-ignore --> |
| `threadpool/KE16-DESIGN-W.md:156` | 237 | refused p4 | `778739f0` | 237→348 | refused by pass 4; the steal-transport fence, 348 <!-- doc-anchor-ignore --> |
| `threadpool/KE16-DESIGN-W.md:160` | 143, 166 | refused p4 | `778739f0` | 143→217, 166→gone | refused by pass 4; note header 217 <!-- doc-anchor-ignore --> |
| `threadpool/KE16-DESIGN-W.md:163` | 147, 152 | refused p4 | `778739f0` | 147→221, 152→226 | refused by pass 4; 221-226 <!-- doc-anchor-ignore --> |
| `LOGGING-SYSTEM-PLAN.md:455` | 3176 | p1 | `303a7a92` | 3176→gone | **[added by pass 7, (1)]** a worked example of the second site's output line, `site=device.rs:3158` when written (the shadow-denoise `eprintln!(`); at `e6115223` 3158 is the DDGI reporter's `W2102,`, and at HEAD the shadow-denoise reporter is at 3192 <!-- doc-anchor-ignore --> |
| `diagnostics/logging/01-EMISSION-RING.md:298` | 3176 | p1 | `d02f74a3` | 3176→gone | **[added by pass 7, (1)]** as `LOGGING-SYSTEM-PLAN.md:455` <!-- doc-anchor-ignore --> |
| `MESHLET-VIRTUAL-GEOMETRY-PLAN.md:2517` | 372, 417 | never moved: no pass read the link form | `6d1af3a9` | 372→664, 417→709 | **[added by pass 7, (2)]** link form `](../goldens/PINS.toml):N~` in a gated doc, passed by the `~` waiver. When written, `test_name` of `[vb_both_sdf]` and `crate` of `[vb_both_sdf_tex]`; at `e6115223` both are comments (the P3-8 non-vacuity note, the P4-4 `hzb_plan_for` note), which the lane moved to 390 and 435. The two unblessed hwrt legs are `[vb_both_sdf]` 633 (`sha256_hwrt = "PENDING"` 669) and `[vb_both_sdf_tex]` 677 (716) <!-- doc-anchor-ignore --> |

**(4) This register.** The three false sentences the pass-4 verifier found are corrected in place, by
insertion and strike-through:
- the "15 written numbers" are 15 tokens carrying 18 numbers;
- of the "8 `device.rs` tokens ... as the pass-2 and pass-3 lists already say", 6 were listed and 2
  were added by pass 4;
- "109 ... the other 5" is 110 and 4, because `KE16-DESIGN-W.md:160`/`:163` carry four numbers and <!-- doc-anchor-ignore -->
  `:156` belongs to the 42 lane-nearest. <!-- doc-anchor-ignore -->

The quotation class is listed in (2). Annotations now point there from:
- pass 4's paragraph on `:882`, its residual 8 and its WHAT IS PUT TO YOU; <!-- doc-anchor-ignore -->
- pass 2's `05-LADDER-GATES.md:231` bullet, which called the list "quoted" and then rewrote it. <!-- doc-anchor-ignore -->

The marker `<!-- doc-anchor-ignore -->` was added to 15 lines of this entry. 14 of them already carried a citation-shaped token without it (lines 5211, 5304, 5306, 5309, 5311, 5312, 5314, 5346, 5357, 5358, 5359, 5363, 5364, 5365), and line 5632 carries one since its correction above. The whole entry now passes the same check.

**The durable fix is not a sixth pass. It is the recommendation already on record** (pass 1's WHAT IS
PUT TO YOU): widen `GATED_DOCS` in `tests/internal_docs_anchors.rs` to the plan corpus, in the
doc-gates lane, so that rot reds on the day it happens. The "last written" column shows the mechanism.
In almost every row the number was right in the commit that last wrote its line. It went false
later, when the cited file changed and the citing line did not. A gate that read the plan corpus
would have failed that later commit, while its author was still in context. What the widening has to handle first is exactly
this pass's class (2). The gate's own scope note keeps "dated records" out because rewriting them
falsifies the record. The nine restored quotations and the two dated-record groups above are those
lines, and each needs the gate's existing opt-out, `<!-- doc-anchor-ignore -->`, before a widened
gate runs over it.

**Hygiene.**
- CRLF == LF == CR in every file pass 5 wrote.
- `git status` shows 31 ` M` files: the 30 of passes 1-4 plus `docs/PARTICLES-PLAN.md`. `docs/ru/`
  is untouched.
- Every doc line pass 5 changed differs from `97bcf826` only in digits.
- Register edits are insertions only: each old line survives as a subsequence of its new text.
- No cargo; no checkout, stash, restore, reset or commit.

Scripts, stdlib, same scratchpad:
- `w2_pass5_apply.py` (the ten doc edits; `--dry` log `w2_pass5_apply_dry.log`, record `w2_pass5_applied.json`);
- `w2_pass5_debt.py` (the table's data, `p5_debt.json`);
- `p5_blamemap.py` and `p5_nonlane.py` (the content-follow and the 114 re-read);
- `w2_pass5_note.py` (this entry and the in-place corrections).

**WHAT IS PUT TO YOU:**
- whether the two dated-record groups in (2) are restored as well; *[answered: restored - Pass 6]*
- whether the two quotations pass 5 found (`:1508`, `:260`) stand - reverting them is two token edits; <!-- doc-anchor-ignore -->
- the `GATED_DOCS` widening, which is the only item here that stops the next lane from needing a
  register entry like this one.

### Pass 6 (2026-09-11): dated records restored, and the pass-5 verifier's four findings

The pass-5 verifier left a finite work list, F1-F4. The orchestrator added one ruling, and it decides
the class pass 5 had put to you. This section records both.

**The rule, recorded once.** The orchestrator decided it. A sentence that records what was verified or
measured at a named moment is a quotation of a past state. It is restored to its pre-pass text, even
where rule (1) would have moved it, because a dated record that silently changes its numbers stops
being a record. The ruling names these forms:
- "Re-verified at the carve";
- "Verified unchanged, exactly as written";
- "Verified this session at";
- "Measured at HEAD" at a named commit;
- a table of references as they stood at a named commit.

A restored number is true of the moment its text names. As a present-tense citation it is usually
false at HEAD, and that is intended.

How pass 6 decided what else is in the class:
- **The dating must be written.** It can be in the sentence or its verification parenthetical. It can
  also be in a lead-in, heading or intro that scopes a block of anchors: "Every line number below is
  as-of-the-critique", "Facts, verified this session", "Every line number below was opened and
  checked while writing Rev 4".
- **A document-wide status header does not qualify.** The KE16 design set says the opposite of a
  record. `threadpool/KE16-DESIGN.md:19-22`: its code line numbers "will move once the pass edits the <!-- doc-anchor-ignore -->
  files" and are given "so the developer can find the site, not as a coordinate to be preserved". And
  if a header dated every citation of its file, rule (1) would have nothing left to govern.
- **A critique that makes a present-tense claim, and declares no as-of for its numbers, is not a
  record.** ~~Example: `KE16-DESIGN-B4.md`'s round-2 critique (F3).~~ *[pass 8: not an example. That critique dates every site below its lead-in, `KE16-DESIGN-B4.md:312` - Pass 8, (A)]* <!-- doc-anchor-ignore -->

**Restored: 35 lines, 119 numbers.** *[pass 7: 29 of the 35 lines, 105 numbers, are dated records under the orchestrator's rule. The other 6 lines, 14 numbers, have no written dating clause: `KE16-DESIGN.md:356`, `:357`, `:359`, `:360`, `KE16-DESIGN-SPACE.md:554` and `KE16-DESIGN-APP.md:275`. They are pre-lane rot returned to their original text, and they are in the debt - Pass 7, (7)]* <!-- doc-anchor-ignore -->
- 11 lines are the ones the orchestrator named, with F2.
- 24 more were found by the sweep. The sweep put the ruling's question to every one of the 140 lines
  the passes had rewritten, and read each with its paragraph and its nearest heading
  (`v6/changed.py`, `v6/ctx2_out.txt`).
- Each restored line is now identical to its `97bcf826` line. Before the restore it differed from
  that line only in digits.
- Three files are back to HEAD: `editor/EDITOR-BOUNDARY.md`, `threadpool/KE16-DESIGN-SPACE.md` and
  `threadpool/KE16-DESIGN-APP.md`.

The dated clause of each line is quoted from the tree, with its markdown emphasis dropped.
`w2_pass6_apply.py --dry` asserted three things
before the write: the clause is present, the line lies inside the clause's scope, and the pre-pass
text differs from the pass-written text only in digits.

| line | why | pass wrote | restored to | the dated clause |
|---|---|---|---|---|
| `logging/00-GOAL-TARGETS.md:111` | named | `golden.ps1:199-205`, `:199-201`, `:204`, `:205`, `:229`, `:235`, `:230`, `:236` | `golden.ps1:196-202`, `:196-198`, `:201`, `:202`, `:226`, `:232`, `:227`, `:233` | same line: "Re-verified at the carve" <!-- doc-anchor-ignore --> |
| `logging/00-GOAL-TARGETS.md:191` | named | `golden.ps1:199-205`, `:229`, `:235`, `device.rs:2122` | `golden.ps1:196-202`, `:226`, `:232`, `device.rs:2110` | same line: "Verified unchanged, exactly as written:" <!-- doc-anchor-ignore --> |
| `LOGGING-SYSTEM-PLAN.md:491` | named | `scripts/golden.ps1:204,229,235` | `scripts/golden.ps1:201,226,232` | same line: "Verified this session at" <!-- doc-anchor-ignore --> |
| `logging/02-SINK-LIFECYCLE.md:53` | named | `scripts/golden.ps1:204`, `:229`, `:235` | `scripts/golden.ps1:201`, `:226`, `:232` | same line: "Verified this session at" <!-- doc-anchor-ignore --> |
| `logging/05-LADDER-GATES.md:236` | named | `device.rs:3806` | `device.rs:3788` | `:230`: "Measured at HEAD, before a line of L7 was written" <!-- doc-anchor-ignore --> |
| `logging/05-LADDER-GATES.md:237` | named | `:3750` | `:3732` | `:230`: "Measured at HEAD, before a line of L7 was written" <!-- doc-anchor-ignore --> |
| `logging/05-LADDER-GATES.md:245` | named | `device.rs:3806` | `device.rs:3788` | `:230`: "Measured at HEAD, before a line of L7 was written" <!-- doc-anchor-ignore --> |
| `threadpool/KE16-DESIGN.md:359` | named | `tests/loom_pool.rs:166` | `tests/loom_pool.rs:140` | `:340`: "## 7. Doc comments the pass corrects" **[pass 7: not a dating clause; pre-lane rot returned to its original text - Pass 7, (7)]** <!-- doc-anchor-ignore --> |
| `threadpool/KE16-DESIGN.md:360` | named | `tests/loom_pool.rs:369` | `tests/loom_pool.rs:343` | `:340`: "## 7. Doc comments the pass corrects" **[pass 7: not a dating clause; pre-lane rot returned to its original text - Pass 7, (7)]** <!-- doc-anchor-ignore --> |
| `threadpool/KE16-DESIGN-SPACE.md:554` | named | `tests/loom_pool.rs:22,24,166,369` | `tests/loom_pool.rs:22,24,140,343` | `:550`: "Doc corrections owed by whichever variant lands (from A.2)" **[pass 7: not a dating clause; pre-lane rot returned to its original text - Pass 7, (7)]** <!-- doc-anchor-ignore --> |
| `threadpool/KE16-RESULTS.md:2257` | F2 | `MEASUREMENT-QUEUE.md:56-60` | `MEASUREMENT-QUEUE.md:44-48` | `:2258`: "False at this HEAD," <!-- doc-anchor-ignore --> |
| `logging/00-GOAL-TARGETS.md:150` | found | `crates/boyko_rhi_vulkan/src/device.rs:2122`, `:2122-2123` | `crates/boyko_rhi_vulkan/src/device.rs:2110`, `:2110-2111` | same line: "Verified this session:" <!-- doc-anchor-ignore --> |
| `LOGGING-SYSTEM-PLAN.md:119` | found | `golden.ps1:199-205`, `:229`, `:235` | `golden.ps1:196-202`, `:226`, `:232` | `:118`: "Measured this session:" <!-- doc-anchor-ignore --> |
| `logging/01-EMISSION-RING.md:253` | found | `crates/boyko_rhi_vulkan/src/device.rs:3118`, `:3176`, `:3207` | `crates/boyko_rhi_vulkan/src/device.rs:3100`, `:3158`, `:3189` | `:255`: "All three device sites re-verified this session at exactly `:3100`" <!-- doc-anchor-ignore --> |
| `threadpool/KE16-DESIGN.md:356` | found | `:129-140` | `:103-114` | `:340`: "## 7. Doc comments the pass corrects" **[pass 7: not a dating clause; pre-lane rot returned to its original text - Pass 7, (7)]** <!-- doc-anchor-ignore --> |
| `threadpool/KE16-DESIGN.md:357` | found | `tests/loom_pool.rs:169-192`, `:234-243` | `tests/loom_pool.rs:143-166`, `:208-217` | `:340`: "## 7. Doc comments the pass corrects" **[pass 7: not a dating clause; pre-lane rot returned to its original text - Pass 7, (7)]** <!-- doc-anchor-ignore --> |
| `threadpool/KE16-DESIGN-APP.md:275` | found | `tests/loom_pool.rs:169-192, 234-243` | `tests/loom_pool.rs:143-166, 208-217` | `:273`: "Rows new in revision 3:" **[pass 7: not a dating clause; pre-lane rot returned to its original text - Pass 7, (7)]** <!-- doc-anchor-ignore --> |
| `VB-PERFORMANCE-TRACK.md:266` | found | `goldens/PINS.toml:281`, `:310` | `goldens/PINS.toml:263`, `:292` | `:264`: "Anchor check (re-verified this revision, not assumed)" <!-- doc-anchor-ignore --> |
| `VB-P1E-HIERARCHICAL-CULL-PLAN.md:3452` | found | `crates/boyko_rhi_vulkan/src/device.rs:2099`, `:2100-2107` | `crates/boyko_rhi_vulkan/src/device.rs:2087`, `:2088-2095` | `:3429`: "Every line number below was opened and checked while writing Rev 4" <!-- doc-anchor-ignore --> |
| `VB-P1E-HIERARCHICAL-CULL-PLAN.md:3453` | found | `crates/boyko_rhi_vulkan/src/device.rs:2602` | `crates/boyko_rhi_vulkan/src/device.rs:2584` | `:3429`: "Every line number below was opened and checked while writing Rev 4" <!-- doc-anchor-ignore --> |
| `VB-SV0-SDF-SHADOW-PLAN.md:1625` | found | `:281-329`, `:301-303` | `:263-311`, `:283-285` | `:1542`: "Every line below was opened while writing this revision" <!-- doc-anchor-ignore --> |
| `VG-R3-P2-CAPABILITY-SPLIT-PLAN.md:1500` | found | `crates/boyko_rhi_vulkan/src/device.rs:2164`, `:2165-2172`, `:2199-2205`, `golden.ps1:170`, `:2122-2123` | `crates/boyko_rhi_vulkan/src/device.rs:2152`, `:2153-2160`, `:2187-2193`, `golden.ps1:167`, `:2110-2111` | `:1426`: "⚠️ Every line number below is as-of-the-critique. Re-anchor at use." <!-- doc-anchor-ignore --> |
| `VG-R3-P2-CAPABILITY-SPLIT-PLAN.md:1504` | found | `PINS.toml:299-302` | `PINS.toml:281-284` | `:1426`: "⚠️ Every line number below is as-of-the-critique. Re-anchor at use." <!-- doc-anchor-ignore --> |
| `VG-R3-P2-CAPABILITY-SPLIT-PLAN.md:1506` | found | `golden.ps1:249-262`, `:286`, `PINS.toml:333-336` | `golden.ps1:246-259`, `:283`, `PINS.toml:315-318` | `:1426`: "⚠️ Every line number below is as-of-the-critique. Re-anchor at use." <!-- doc-anchor-ignore --> |
| `VG-R3-P2-CAPABILITY-SPLIT-PLAN.md:1508` | found | `PINS.toml:327-359` | `PINS.toml:309-341` | `:1426`: "⚠️ Every line number below is as-of-the-critique. Re-anchor at use." <!-- doc-anchor-ignore --> |
| `VG-R3-P2-CAPABILITY-SPLIT-PLAN.md:1538` | found | `device.rs:2164`, `:2199-2205`, `:2122-2134`, `golden.ps1:170` | `device.rs:2152`, `:2187-2193`, `:2110-2122`, `golden.ps1:167` | `:1536`: "⚠️ Every line number below is as-of-2026-08-06 and must be re-anchored at the moment of…" <!-- doc-anchor-ignore --> |
| `VG-R3-P2-CAPABILITY-SPLIT-PLAN.md:1548` | found | `:262`, `:243`, `:261`, `:269`, `PINS.toml:373-376`, `:497-501` | `:259`, `:240`, `:258`, `:266`, `PINS.toml:355-358`, `:479-483` | `:1536`: "⚠️ Every line number below is as-of-2026-08-06 and must be re-anchored at the moment of…" <!-- doc-anchor-ignore --> |
| `VG-R3-P2-CAPABILITY-SPLIT-PLAN.md:1556` | found | `golden.ps1:183`, `:196`, `PINS.toml:777` | `golden.ps1:180`, `:193`, `PINS.toml:759` | `:1536`: "⚠️ Every line number below is as-of-2026-08-06 and must be re-anchored at the moment of…" <!-- doc-anchor-ignore --> |
| `VG-R3-P3-CULL-INTEGRATION-PLAN.md:117` | found | `device.rs:1172`, `:3279` | `device.rs:1160`, `:3261` | `:109`: "### Hard local constraints, each re-verified in this tree" <!-- doc-anchor-ignore --> |
| `VG-R3-P3-CULL-INTEGRATION-PLAN.md:2851` | found | `PINS.toml:353-359` | `PINS.toml:335-341` | `:2712`: "⚠️ Every line number below is as-of-the-critique. Re-anchor at use." <!-- doc-anchor-ignore --> |
| `VG-R3-P4-CONFIG-AND-INSTRUMENT-PLAN.md:50` | found | `PINS.toml:467-469` | `PINS.toml:449-451` | `:32`: "### Facts, verified this session" <!-- doc-anchor-ignore --> |
| `VG-R3-P4-CONFIG-AND-INSTRUMENT-PLAN.md:51` | found | `PINS.toml:318`, `:350`, `:416`, `:602` | `PINS.toml:300`, `:332`, `:398`, `:584` | `:32`: "### Facts, verified this session" <!-- doc-anchor-ignore --> |
| `VG-R3-P4-CONFIG-AND-INSTRUMENT-PLAN.md:52` | found | `PINS.toml:411-412`, `:479-480`, `:504-505`, `:533-534`, `:561-562` | `PINS.toml:393-394`, `:461-462`, `:486-487`, `:515-516`, `:543-544` | `:32`: "### Facts, verified this session" <!-- doc-anchor-ignore --> |
| `VG-R3-P4-CONFIG-AND-INSTRUMENT-PLAN.md:53` | found | `PINS.toml:379-382`, `:447-450` | `PINS.toml:361-364`, `:429-432` | `:32`: "### Facts, verified this session" <!-- doc-anchor-ignore --> |
| `editor/EDITOR-BOUNDARY.md:41` | found | `crates/boyko_rhi_vulkan/src/device.rs:767-915` | `crates/boyko_rhi_vulkan/src/device.rs:767-903` | `:29`: "Not taste. Three code-level facts, each read at this checkout." <!-- doc-anchor-ignore --> |

Why the found lines are in the class:
- **`logging/00-GOAL-TARGETS.md:150`.** Its own parenthetical reads "*(Verified this session: <!-- doc-anchor-ignore -->
  `let sync_validation_available = ...` spans `:2110-2111`.)*". It was the same form as `:191`. <!-- doc-anchor-ignore -->
- **`LOGGING-SYSTEM-PLAN.md:119`.** It is the first bullet under "Measured this session:" (`:118`), <!-- doc-anchor-ignore -->
  and it is the monolith text that `00-GOAL-TARGETS.md:111` carries forward "re-verified at the <!-- doc-anchor-ignore -->
  carve". The one other rewritten bullet under it, `:123`, is a present-tense rule and stays moved: <!-- doc-anchor-ignore -->
  "it is the reason `golden.ps1:229`'s line-start match ... keeps working". <!-- doc-anchor-ignore -->
- **`logging/01-EMISSION-RING.md:253`.** The paragraph after it reads "*(All three device sites <!-- doc-anchor-ignore -->
  re-verified this session at exactly `:3100` ..., `:3158` ... and `:3189` ...)*". No pass had <!-- doc-anchor-ignore -->
  touched `:255`, because it carries no file name. So the pair contradicted itself: 3118/3176/3207 <!-- doc-anchor-ignore -->
  beside "re-verified at exactly 3100/3158/3189".
- **`threadpool/KE16-DESIGN.md:356`-`357`.** *[pass 7: in the class by association only. The §7 heading dates nothing, so under the orchestrator's rule these two are pre-lane rot, and so are the named `:359`-`:360` - Pass 7, (7)]* They are rows of the §7 table whose `:359`-`:360` <!-- doc-anchor-ignore -->
  the orchestrator named. Every quoted comment in those four rows stood in `loom_pool.rs` at
  `778739f0`, and none stands at `e6115223`:
  - "loom (issue #246) does NOT persist" at `:105-106`; <!-- doc-anchor-ignore -->
  - "In PRODUCTION that fence is supplied by the crossbeam **injector** transport" at `:156`; <!-- doc-anchor-ignore -->
  - "post-`mark_idle` re-poll (`worker.rs:78`)" at `:140`; <!-- doc-anchor-ignore -->
  - "`worker.rs:86`'s" at `:343`. <!-- doc-anchor-ignore -->
- **`threadpool/KE16-DESIGN-APP.md:275`.** *[pass 7: "Rows new in revision 3" says when a row was added, not that anything was verified. Pre-lane rot - Pass 7, (7)]* It lists "Rows new in revision 3" of that same checklist, <!-- doc-anchor-ignore -->
  and one of them is the `:143-166, 208-217` fidelity note. It is the same kind of list as the named <!-- doc-anchor-ignore -->
  `KE16-DESIGN-SPACE.md:554` ("Doc corrections owed"). <!-- doc-anchor-ignore -->
- **`VB-PERFORMANCE-TRACK.md:266`.** The sentence reads "*Anchor check (re-verified this revision, not <!-- doc-anchor-ignore -->
  assumed)*". At `15199271`, which wrote it, `PINS.toml:263` is `[vb_mesh]` and `:292` is its <!-- doc-anchor-ignore -->
  `sha256_software`.
- **`VB-P1E-HIERARCHICAL-CULL-PLAN.md:3452`-`3453`** and **`VB-SV0-SDF-SHADOW-PLAN.md:1625`.** Both are <!-- doc-anchor-ignore -->
  appendices whose intros date every line below them.
- **`VG-R3-P2-CAPABILITY-SPLIT-PLAN.md:1500`-`1508`** and **`:1538`-`1556`.** Round 1's critique opens <!-- doc-anchor-ignore -->
  with "Every line number below is as-of-the-critique" (`:1426`), and its §5 repeats the rule "as-of-2026-08-06" <!-- doc-anchor-ignore -->
  (`:1536`). This reverses pass 5's "`:1504` stays moved": pass 5 kept it as class (1), and the ruling <!-- doc-anchor-ignore -->
  overrides class (1). `:1010`'s "round 1's anchor `:281-284` had **MOVED**" now quotes `:1504` <!-- doc-anchor-ignore -->
  exactly.
- **`VG-R3-P3-CULL-INTEGRATION-PLAN.md:117`, `:2851`, `VG-R3-P4-CONFIG-AND-INSTRUMENT-PLAN.md:50`-`53`** <!-- doc-anchor-ignore -->
  **and `editor/EDITOR-BOUNDARY.md:41`.** Their dating comes from the heading or intro above them: <!-- doc-anchor-ignore -->
  - "Hard local constraints, each re-verified in this tree";
  - round 1's "Every line number below is as-of-the-critique" (`:2712`); <!-- doc-anchor-ignore -->
  - "Facts, verified this session";
  - "Three code-level facts, each read at this checkout".

**F1 - not written. The line F1 compared it with is restored instead.** `VG-R3-P3-CULL-INTEGRATION-PLAN.md:2924` <!-- doc-anchor-ignore -->
reads `[vb_mesh_hzb]:335-341`. As a present-tense citation it is a lane shift, exactly as F1 says: <!-- doc-anchor-ignore -->
- at `e6115223`, `PINS.toml:335` is `[vb_mesh_hzb.env]`, `:339` is `BOYKO_VG_HZB             = "1"` and <!-- doc-anchor-ignore -->
  `:341` is the blank before `[vb_occ_split]` at 342; <!-- doc-anchor-ignore -->
- at HEAD those lines are 353, 357, 359 and 360.

But the line is in round 2's critique, and that critique opens with "⚠️ Line numbers are
as-of-this-critique. Re-anchor at use." (`:2890`). The finding it belongs to opens with "Re-anchored <!-- doc-anchor-ignore -->
this session." (`:2922`). Under the ruling, which overrides rule (1) in its own words, the line keeps <!-- doc-anchor-ignore -->
335-341.

F1 cited `:2851` as already reading 353-359. That line is in round 1's critique, under "Every line <!-- doc-anchor-ignore -->
number below is as-of-the-critique." (`:2712`). Pass 1 had moved it, and pass 6 restored it to <!-- doc-anchor-ignore -->
335-341. The two lines now agree, each as a record of its own critique.

**This departs from F1's instruction.** It is put to you below. *[pass 7: decided by the orchestrator. Both lines keep 335-341, as dated records of their critiques - Pass 7, (3)]*

The scan F1 asked for:
- `git grep -E '\[[A-Za-z0-9_.\-]+\]`?:[0-9]+'` over every tracked file finds `:2924` and nothing <!-- doc-anchor-ignore -->
  else, `docs/ru/` included.
- A wider form, any `]` followed by `:N`, adds one hit: `archive/PHASE4-CORE-SEAMS-PLAN.md:377`'s <!-- doc-anchor-ignore -->
  `#[repr(C)]:72`. It cites `system_meta.rs`, which is not a lane file. <!-- doc-anchor-ignore -->
- A section name followed by a bare `(:N)` finds `VG-R3-P2-CAPABILITY-SPLIT-PLAN.md:1014`, which passes <!-- doc-anchor-ignore -->
  moved as class (1), and `VG-R3-P3-CULL-INTEGRATION-PLAN.md:1862`, which is in the table. <!-- doc-anchor-ignore -->
- Pass 5's marker check did not match `[section]:N` either. Pass 6's does.

**F2 - `threadpool/KE16-RESULTS.md:2257` restored to `MEASUREMENT-QUEUE.md:44-48`.** The paragraph <!-- doc-anchor-ignore -->
dates itself: "**False at this HEAD**, ... the profile changed 2026-09-04 in HEAD `4a363678` itself".
- At `4a363678`, and at `611999cb`, which wrote the line, `MEASUREMENT-QUEUE.md:45` holds the quoted <!-- doc-anchor-ignore -->
  claim: "`codegen-units = 1` and **no `[profile.release]` section at all**".
- At `e6115223`, `:44-45` already say the opposite: "neither one is simply "better"" and "`Cargo.toml` <!-- doc-anchor-ignore -->
  carries `[profile.release] lto = "fat"`".

So 44 and 48 were never true there as present-tense citations. Pass 1's 56-60 is that corrected
paragraph at HEAD, which is not what the sentence quotes.

**F3 - `threadpool/KE16-DESIGN-B4.md:378` and `:385` ~~are debt, and~~ the table now holds them.** *[pass 8: they are dated records, restored, and so are `:421`, `:456` and `:457`; their rows are marked no longer debt - Pass 8, (A)]* <!-- doc-anchor-ignore -->

Both lines were written at `7fdd738e`, where their numbers were right:
- `KE16-RESULTS.md:12` read "| B | **B0 BY CONSTRUCTION, NOT BY MEASUREMENT** ..."; <!-- doc-anchor-ignore -->
- `:531` read "> axis A closes on `a3` and axis B stops.**"; <!-- doc-anchor-ignore -->
- `:1158-1163` was the `a1` worker-route block, with "top_lane=13/24/21" at 1160. <!-- doc-anchor-ignore -->

At `e6115223` none of them holds:
- `:12` reads "| B | CLOSED ON `b1` 2026-09-08 ..."; <!-- doc-anchor-ignore -->
- `:531` reads "**FIRES.** The winner is `a3` ...", and the quote is at 538; <!-- doc-anchor-ignore -->
- `:1158-1163` are rows of the `wg`/`w0` table, and the block is at 2027-2032. <!-- doc-anchor-ignore -->

So the lane did not make them false. Pass 1 moved 531 to 575 and 1158-1163 to 1202-1207. At HEAD:
- 575 is "**FIRES.** ...", and the quote is at 582;
- 1202-1207 is still the `wg`/`w0` table, and the block is at 2071-2076.

~~Neither line is a dated record.~~ *[pass 8: both are. The round-2 critique opens with the lead-in `KE16-DESIGN-B4.md:312`, "Reviewed 2026-09-07 against `d647d930`. Every site below was re-verified by hand." Every number on both lines holds at `d647d930` - Pass 8, (A)]* Both are present-tense claims ("is un-published", "records") of <!-- doc-anchor-ignore -->
`B4`'s round-2 critique. That critique, unlike those of `VG-R3-P2` and `VG-R3-P3`, declares no as-of
~~for its line numbers.~~ *[pass 7: ~~for these two lines' numbers. It does declare one,~~ [pass 8: false. It declares one for every site below its lead-in, `:312`, and a second] at `KE16-DESIGN-B4.md:408`: "Those two anchors are AS OF this analysis". That sentence scopes the two `tests/ke16_feature_scheme_census.rs` anchors ~~on its own line~~ [pass 8: on `:407`, the line above it], neither of them a lane citation, and it sits under `## Non-blocking, recorded`, while `:378` and `:385` sit under `## BLOCKING 3`. ~~F3's debt decision stands~~ [pass 8: it does not stand - Pass 8, (A)] - Pass 7, (6)]* The file header dates the document ("Written 2026-09-07 against <!-- doc-anchor-ignore -->
`feat/threadpool-ke16` @ `7f294afe`"), but by the rule above a header does not make a citation a
record. ~~Both lines are therefore *moved, still false*,~~ *[pass 8: restored as dated records - Pass 8, (A)]* and the table gains two rows with 4 numbers
(12 and 575; 1202 and 1207). The rows sit after `KE16-DESIGN-B4.md:72`, at lines 5908-5909. <!-- doc-anchor-ignore -->

**F4 - pass 5's four false sentences, corrected in place.** The corrections use insertion and
strike-through only.
- **(a) (3)'s mapping.** It said "3 hold at HEAD ..., and `KE16-DESIGN-B4.md:378`". Only 2 hold: <!-- doc-anchor-ignore -->
  - `:517`: HEAD's `golden.ps1:183`/`:196` carry no `--release`; <!-- doc-anchor-ignore -->
  - `:883`: `PINS.toml:75-82` lies inside `[grand_showcase_2mat]`, 64-84. <!-- doc-anchor-ignore -->
  So pass 5's table, with F3's rows, holds 44 of pass 2's 53 NOT-CONFIRMED lines.
- **(b) (1)'s "the 152 numbers the verifier judged TRUE were true at `e6115223` and are true at
  HEAD".** This is false for ~~5~~ *[pass 7: 12 - these 5 and seven `device.rs:3158`, Pass 7, (1)]* of them: `:378`'s 531, `:385`'s 1158 and 1163, and <!-- doc-anchor-ignore -->
  `KE16-RESULTS.md:2257`'s 44 and 48. Where the 152 stand now: <!-- doc-anchor-ignore -->
  - ~~95~~ *[pass 7: 90. The five present-tense `device.rs:3158` were never true at `e6115223`, and they are in the table]* are moved and true at HEAD; <!-- doc-anchor-ignore -->
  - ~~50 were true at `e6115223`~~ *[pass 7: 49 were. `01-EMISSION-RING.md:253`'s 3158 was not; it holds at `d02f74a3`, the moment its record names]* and pass 6 restored them as dated records; <!-- doc-anchor-ignore -->
  - 2 were restored by pass 5 (`05-LADDER-GATES.md:231`); <!-- doc-anchor-ignore -->
  - ~~3~~ *[pass 7: 8]* are false and in the table (F3 *[pass 7: 3, and the five `device.rs:3158` - Pass 7, (1)]*) *[pass 8: 5 in the table now, the five `device.rs:3158`. F3's 3 hold at `d647d930`, the moment the critique's lead-in names, and are restored as dated records - Pass 8, (A)]*; <!-- doc-anchor-ignore -->
  - 2 are false and were restored as a dated record (F2).
- **(c) (3)'s headline, "212 numbers on 94 lines, every one by name".** F3 makes it incomplete. At the
  close of pass 5 the debt was 216 numbers on 96 lines.
- **(d) (1)'s heading, "one citation that no pass wrote was, and it is now written".** `:2924` is a <!-- doc-anchor-ignore -->
  second one, and it is not written (F1).

Three pointers were also inserted:
- at pass 5's "`:1504` stays moved"; <!-- doc-anchor-ignore -->
- at its "not restored" paragraph;
- at its first WHAT IS PUT TO YOU item.

**The debt after pass 6: 149 numbers on 71 lines.** *[pass 7: 170 on 80 - Pass 7]* *[pass 8: 153 on 73 - Pass 8]* That is 216 on 96, less the 25 table rows (67
numbers) that pass 6 restored as dated records. Each of those rows is marked in place, and its
"numbers now" cell shows the restored value. By class:

| class | pass 5 | after pass 6 |
|---|---|---|
| never true at `e6115223`, `device.rs` | 65 | 39 |
| never true at `e6115223`, `PINS.toml` | 83 | 58 |
| never true at `e6115223`, `miri_scope.rs` | 2 | 2 |
| `loom_pool.rs` | 35 | 21 |
| near misses | 6 | 4 |
| refused continuations | 21 | 21 |
| F3, `KE16-RESULTS.md` | - | 4 |
| **total** | **212** | **149** |

**Read in the sweep and not restored.** These looked dated and are not:
- `PARTICLES-PLAN.md:183`, pass 5's one lane write: its heading, "Verified in-tree facts", names no <!-- doc-anchor-ignore -->
  moment. It stays at 2041.
- `PBR-MATERIALS-PLAN.md:32`: "all integration points verified against the code" names no moment. <!-- doc-anchor-ignore -->
- `VB-P1E-HIERARCHICAL-CULL-PLAN.md:3184`: "Concrete precondition, verified" names no moment. <!-- doc-anchor-ignore -->
- `logging/03-CODES-REGISTRY.md:360`: ~~"at HEAD" names no commit~~ *[pass 7: right result, wrong reason. "At HEAD" is the same kind of moment as "this session" or "in this tree": the commit that wrote the clause, here `b30fa810`. Its 3100/3158/3189 do not hold there, nor at its parent, so the line records no moment - Pass 7, (7)]*. <!-- doc-anchor-ignore -->
- `OPEN-QUESTIONS.md:1954`: "scans with the literal pattern" is present tense. It sits inside a dated <!-- doc-anchor-ignore -->
  entry, but it records no verification.
- `VB-SV0-SDF-SHADOW-PLAN.md:243` and `:952`: changelog and discharge text ("now states"). <!-- doc-anchor-ignore -->
- `KE16-DESIGN-B4.md:456`-`457` *[pass 8: restored after all. They sit under the round-2 critique's lead-in `:312`, which this sweep did not read - Pass 8, (A)]*: the section's first bullet names `7f294afe` for §0's corrections <!-- doc-anchor-ignore -->
  alone; their own sentence claims the present.
- Design prose that cites the same `loom_pool.rs` sites as §7, not as a list of corrections:
  `KE16-DESIGN-A.md:494`, `KE16-DESIGN-W.md:138`, `:153`, `KE16-DESIGN.md:383`, `KE16-DESIGN-B4.md:72` <!-- doc-anchor-ignore -->
  and `KE16-RESULTS.md:1116`. <!-- doc-anchor-ignore -->
- `KE16-RESULTS.md:178`: a calibration-run record, but its section names a configuration <!-- doc-anchor-ignore -->
  (`a0+b0+w0+c0`), not a moment, and the citation is present tense ("its attribute at ... is"). This one is
  borderline.
- The present-tense rules that cite `golden.ps1:229`/`:235`: `LOGGING-SYSTEM-PLAN.md:123`, `:495`, <!-- doc-anchor-ignore -->
  `:2031`, `:2175`, `SEAM.md:290`, `02-SINK-LIFECYCLE.md:57`, `03-CODES-REGISTRY.md:393` and <!-- doc-anchor-ignore -->
  `05-LADDER-GATES.md:955`-`956`. These are class (1), true at HEAD. <!-- doc-anchor-ignore -->

**Hygiene.**
- CRLF == LF == CR in the 16 docs pass 6 wrote and in this register.
- `git status` shows 28 ` M` files. The three that dropped out are back to their `97bcf826` content.
  `docs/ru/` is untouched.
- Register edits are insertions and strike-through only: each old line survives as a subsequence of
  its new text.
- The marker `<!-- doc-anchor-ignore -->` is on every citation-shaped line of the lane's entry,
  `[section]:N` form included. No pre-existing line lacked it.
- No cargo; no checkout, stash, restore, reset or commit.

Scripts, stdlib, same scratchpad:
- `w2_pass6_apply.py` (the 35 restores; `--dry` log `w2_pass6_apply_dry.log`, record `w2_pass6_applied.json`);
- `w2_pass6_note.py` (this section, the F4 corrections and the table update);
- `v6/changed.py` and `v6/ctx2.py` (the sweep's inputs).

**WHAT IS PUT TO YOU:**
- **F1's departure.** *[answered by the orchestrator: both keep 335-341 - Pass 7, (3)]* `VG-R3-P3-CULL-INTEGRATION-PLAN.md:2851` and `:2924` either keep their <!-- doc-anchor-ignore -->
  as-of-the-critique 335-341 (pass 6), or read HEAD's 353-359 (F1's instruction). Either way it is
  two token edits.
- **The sweep's reach.** 24 lines were restored beyond the 11 that were named, all by the
  written-dating test above. *[pass 7: under the orchestrator's rule, 21 of the 24 are dated records. The other 3 - `KE16-DESIGN.md:356`, `:357` and `KE16-DESIGN-APP.md:275` - carry no written dating clause. They are pre-lane rot returned to their original text, the bytes "do not move pre-lane rot" leaves. Of the 11 named, `KE16-DESIGN.md:359`, `:360` and `KE16-DESIGN-SPACE.md:554` fall the same way - Pass 7, (7)]* The table lists every one of them. Any row reverts by setting its line <!-- doc-anchor-ignore -->
  back to the "pass wrote" text. The borderline lines left alone are `KE16-RESULTS.md:178` and <!-- doc-anchor-ignore -->
  `KE16-DESIGN-B4.md:456`-`457`. *[pass 8: both restored as dated records - Pass 8, (A)]* <!-- doc-anchor-ignore -->
- **The `GATED_DOCS` widening**, unchanged from pass 5. Every restored line needs
  `<!-- doc-anchor-ignore -->` before a widened gate reads it.

### Pass 7 (2026-09-11): closing - the pass-6 verifier's seven items, and the final accounting

The pass-6 verifier returned seven items. The orchestrator decided two of them outright, F1 and the
dating rule, and fixed the scope: this pass changes register prose and the debt table only. No
citation in any other document is edited. The verifier confirmed each one right as it stands, and
every item below is about how this register counts and classifies them. Every figure is recomputed by
the scripts named at the end; none is carried over.

**(1) The `device.rs` W2102 trio: five numbers counted as lane shifts were never true at `e6115223`.**

Five present-tense citations name `device.rs:3158` as the second of the three `W2102` sites, the RT <!-- doc-anchor-ignore -->
shadow-denoise one:
- `LOGGING-SYSTEM-PLAN.md:424`, `:455` and `:2013`; <!-- doc-anchor-ignore -->
- `logging/01-EMISSION-RING.md:298`; <!-- doc-anchor-ignore -->
- `logging/03-CODES-REGISTRY.md:360`. <!-- doc-anchor-ignore -->

Pass 1 moved each 3158 to 3176 by the lane's map. The number was right when four of the lines were
written: at `053f6c9f`, `303a7a92` and `d02f74a3`, line 3158 is the shadow-denoise `eprintln!(`. The
fifth, `:360`, was written at `b30fa810`, where it was already stale. By `e6115223`, rung L7b had <!-- doc-anchor-ignore -->
turned the three `eprintln!`s into three reporter functions:
- line 3158 is `W2102,` inside `report_ddgi_storage_unsupported`, the FIRST site;
- the shadow-denoise reporter is `report_shadow_denoise_storage_unsupported`, at 3174, with its
  `W2102.number()` at 3179.

At HEAD, 3176 is again the DDGI reporter's `W2102,`, and the shadow-denoise reporter is at 3192
(3197). So the lane did not make these five false, and the move neither helped nor harmed. The
pass-4 verifier had judged all five TRUE, and pass 6 counted them among its "95 moved and true at
HEAD".

They are in the table now, by name. Three rows gain a second number (`:424`, `:2013`, `:360`), and two <!-- doc-anchor-ignore -->
rows are new, appended at the table's end (`LOGGING-SYSTEM-PLAN.md:455`, `01-EMISSION-RING.md:298`). <!-- doc-anchor-ignore -->
The same 3158 also stands on two restored lines, where it stays:
- `01-EMISSION-RING.md:253` is a dated record of `d02f74a3`, where 3158 holds; <!-- doc-anchor-ignore -->
- `05-LADDER-GATES.md:231` quotes it as a number that had drifted. <!-- doc-anchor-ignore -->

The counts are corrected in place, in pass 6's F4 (b) and in pass 5's (1). Of the 152 numbers the
pass-4 verifier judged TRUE, **140** were true at `e6115223` and 12 were not: F3's 3, F2's 2 and seven
`3158`. Where the 152 stand now:

| where | numbers |
|---|---|
| moved, and true at HEAD | 90 |
| on lines pass 6 restored as dated records: 49 true at `e6115223`, and `01-EMISSION-RING.md:253`'s 3158, which holds only at `d02f74a3`, the moment its record names | 50 <!-- doc-anchor-ignore --> |
| restored by pass 5 as a quotation (`05-LADDER-GATES.md:231`) | 2 <!-- doc-anchor-ignore --> |
| false at `e6115223`, in the table: ~~F3's 3 and~~ the five above *[pass 8: F3's 3 are restored as dated records]* | ~~8~~ 5 |
| false at `e6115223`, restored as a dated record (F2) | 2 |
| **total** | **152** |

The 90 were re-checked mechanically (`p7/lane90.py`). For each one, the tree's citing line still
carries the number passes 1-4 wrote, and the target's line at `e6115223` equals the line that number
names at `97bcf826`:
- 78 exactly;
- 2 as REPL lines, the same key with `gnu` -> `msvc`;
- 10 are the `.cargo/config.toml` numbers, read from their `e6115223` citing text, `:14-15`. <!-- doc-anchor-ignore -->

Whether each sentence is true of its line is the pass-4 verifier's judgement, less the 12 above; this
pass did not re-judge the other 140. One is noted and not reopened: the third site, `:3189`, is at <!-- doc-anchor-ignore -->
`e6115223` the `#[inline(never)]` one line above `fn report_ssao_denoise_storage_unsupported`, and the
pass-4 verifier counted it TRUE.

After (1) alone, the debt is 154 numbers on 73 lines.

**(2) A fourth citation form escaped the repair's scanners: the markdown link, `[text](path):N`.** <!-- doc-anchor-ignore -->

`MESHLET-VIRTUAL-GEOMETRY-PLAN.md:2517` reads `` [`PINS.toml`](../goldens/PINS.toml):372~ `` and the <!-- doc-anchor-ignore -->
same link with `:417~`. No pass's scanner could read it. Each takes a file name, or a backticked <!-- doc-anchor-ignore -->
`` `:N` ``, immediately before the number, and here an unbackticked `)` stands there.

The scan (`p7/linkscan.py`) covered every tracked text file in the tree, `docs/ru/` included:
- 235 citations are of this form;
- 6 of them name a file the lane changed, all in `MESHLET-VIRTUAL-GEOMETRY-PLAN.md`;
- two neighbouring forms were checked as well: `](path#Lnn)` occurs 237 times and names no lane <!-- doc-anchor-ignore -->
  file, and `](path:N)` does not occur. <!-- doc-anchor-ignore -->

The six:
- **`:2517`, `PINS.toml` 372 and 417: into the table, not moved.** They were written at `6d1af3a9` <!-- doc-anchor-ignore -->
  (2026-07-28). There, 372 is `test_name` of `[vb_both_sdf]` and 417 is `crate` of
  `[vb_both_sdf_tex]`, inside the two unblessed hwrt legs the sentence names. At `e6115223` both are
  comments (the P3-8 non-vacuity note and the P4-4 `hzb_plan_for` note), so they were false before
  the lane. The lane moved those comments to 390 and 435, and nobody wrote the citation. At HEAD
  `[vb_both_sdf]` is at 633, its `sha256_hwrt = "PENDING"` at 669, and `[vb_both_sdf_tex]` at 677
  (716).
- **`:2516`'s `PINS.toml:15`, and `:992`, `:1119` and `:1137`'s `sv0_deferred_term_bench.rs` 297, 53 <!-- doc-anchor-ignore -->
  and 805: not in the table.** The lane's map is the identity at all four lines and at the three
  range ends, so the lane cannot have made any of them true or false. Its one hunk in
  `sv0_deferred_term_bench.rs` is a one-line replace at 261, and `PINS.toml:15` lies in the file's <!-- doc-anchor-ignore -->
  offset-0 head. They are the same class as pass 2's 23 rot-free tokens.

**The anchors gate did read the two, and it passes them.** `MESHLET-VIRTUAL-GEOMETRY-PLAN.md` is the
fourth entry in `GATED_DOCS`, and the gate's scanner does resolve a link target followed by `:N`. But
both anchors carry the `~` waiver, and a waived anchor is checked only for lying inside the file
(`tests/internal_docs_anchors.rs:967`). Both do. So pass 1's "the anchors gate covers none of this" <!-- doc-anchor-ignore -->
was population-limited: pass 1's population could not contain the link form. It is annotated in place.

The four citation forms that a scanner in use missed and a later reading found:

| # | form | example | missed by | found by |
|---|---|---|---|---|
| 1 | slash-separated list | `PINS.toml:411/474/499/528/556` | pass 1 (first member only) | pass 1, reading its own diff <!-- doc-anchor-ignore --> |
| 2 | comma-space list | `234-262, 330` | pass 1 | pass 2 <!-- doc-anchor-ignore --> |
| 3 | section-name prefix | `[vb_mesh_hzb]:335-341` | every repair pass through 5 | the pass-5 verifier (F1) <!-- doc-anchor-ignore --> |
| 4 | markdown link | `` [`PINS.toml`](../goldens/PINS.toml):372~ `` | every repair pass through 6 | the pass-6 verifier <!-- doc-anchor-ignore --> |

Two things are not counted. Pass 2's "seventh form" (`F:N` / `a,b,c`) was named by pass 2 while it
read and rewrote it. And a continuation two or more lines below its file mention is a failure to bind
a number to its file, not a token form. For the fourth form, the pass-5 verifier's own scan did list
both lines, as an alternative binding (`v5/su.txt`); no report named them until the pass-6 verifier's.

**(3) F1, decided by the orchestrator.** `VG-R3-P3-CULL-INTEGRATION-PLAN.md:2924` keeps <!-- doc-anchor-ignore -->
`[vb_mesh_hzb]:335-341`, and `:2851` keeps its restored 335-341. Each sits under a critique that <!-- doc-anchor-ignore -->
dates its own line numbers: round 2's "Line numbers are as-of-this-critique" (`:2890`) and round 1's <!-- doc-anchor-ignore -->
"Every line number below is as-of-the-critique" (`:2712`). Both lines are dated records. This answers <!-- doc-anchor-ignore -->
pass 6's WHAT IS PUT TO YOU; a pointer is inserted there and at pass 6's "This departs from F1's
instruction".

**(4) Two stale sentences, corrected in place by strike-through.**
- Pass 5 (2): "7 of the 9 are byte-identical ... The other two are 1010 and 1508". It is 8 of the 9
  now. Pass 6 restored `:1508`'s `PINS.toml:327-359` to `309-341`, so that line is byte-identical to <!-- doc-anchor-ignore -->
  `97bcf826`, and only 1010 is not. Checked over all nine.
- Pass 4: "7 hold: ... `:1548`'s `:262`/`:243`/`:261`/`:269`". Since pass 6, `:1548` reads <!-- doc-anchor-ignore -->
  `:259`/`:240`/`:258`/`:266`, its as-of-2026-08-06 numbering. At `9e80cd4e`, `golden.ps1` lines 259, <!-- doc-anchor-ignore -->
  240, 258 and 266 are `Set-Content`, `if ($Bless) {`, the throw and the `PENDING` check - the same
  four texts. Seven still hold: three at HEAD and four as a record.

**(5) Two inaccurate sentences, corrected in place.**
- Pass 6's "which is true of all 152", about each target line keeping its text across the lane. It is
  true of 150. `VG-R3-P2-CAPABILITY-SPLIT-PLAN.md:1011`'s 307 and <!-- doc-anchor-ignore -->
  `VG-R3-P3-CULL-INTEGRATION-PLAN.md:988`'s 340 point at REPL lines: `RUSTUP_TOOLCHAIN`, with `gnu` -> <!-- doc-anchor-ignore -->
  `msvc`.
- Pass 5's "3732 is `mod tests {`", in (2) and in the table row for `05-LADDER-GATES.md:237`. At <!-- doc-anchor-ignore -->
  `b1725b32`, line 3732 is `#[cfg(test)]`, and `mod tests {` is 3733. The ladder's own words,
  "`#[cfg(test)] mod tests` opening at `:3732`", are right, so the restore stands. Only the register's <!-- doc-anchor-ignore -->
  gloss was wrong.

**(6) F3's reason, reworded in place, in pass 6's F3 and in the `KE16-DESIGN-B4.md:378` row.** Pass 6 <!-- doc-anchor-ignore -->
wrote that `KE16-DESIGN-B4.md`'s round-2 critique "declares no as-of for its line numbers". As
written, that is false. `KE16-DESIGN-B4.md:408`, in the same critique, reads "Those two anchors are AS <!-- doc-anchor-ignore -->
OF this analysis". That sentence scopes the two `tests/ke16_feature_scheme_census.rs` anchors ~~on its~~
~~own line,~~ *[pass 8: on `:407`, the line above it; the sentence is on `:408` - Pass 8, (E)]* `:55-67` and `:79-90`, neither of them a lane citation. It sits under <!-- doc-anchor-ignore -->
`## Non-blocking, recorded` (`:399`), while `:378` and `:385` sit under `## BLOCKING 3` (`:362`). It <!-- doc-anchor-ignore -->
~~does not reach them, so F3's debt decision stands.~~ *[pass 8: it does not reach them, and it did not need to. The critique's lead-in, `KE16-DESIGN-B4.md:312`, "Reviewed 2026-09-07 against `d647d930`. Every site below was re-verified by hand.", dates every site below it. F3's debt decision does not stand: `:378`, `:385`, `:421`, `:456` and `:457` are restored - Pass 8, (A)]* <!-- doc-anchor-ignore -->

**(7) The dating rule, decided by the orchestrator, and how every case falls.**

The rule: a dating clause counts only when it is written in the sentence, in its paragraph, or in a
heading or lead-in that scopes the block of anchors. A document-wide header does not count.

This pass applies two more conditions with it. Both were already inside pass 6's ruling, "a sentence
that records what was verified or measured at a named moment":
- the clause dates a verification or a measurement, not a present-tense rule that happens to sit
  under a dated lead-in;
- the restored numbers hold at the moment the clause names. Otherwise there is no record to preserve.

"At HEAD", "in this tree", "this session", "this checkout" and "this revision" are the same kind of
moment: the commit that wrote the clause, which `git blame` recovers. They count alike. Pass 6 had
rejected one "at HEAD" for naming no commit, while accepting "re-verified in this tree", which names
none either.

How the 35 lines pass 6 restored fall:
- **29 lines, 105 numbers: dated records.** Every clause is quoted in pass 6's table, and each line's
  numbers hold at the commit that wrote it (`v7/dated2.txt`, read at `git blame`).
- **6 lines, 14 numbers: pre-lane rot, returned to its original text.** They are
  `KE16-DESIGN.md:356` and `:357`, which the orchestrator named; `:359`, `:360` and <!-- doc-anchor-ignore -->
  `KE16-DESIGN-SPACE.md:554`, which pass 6 had named as dated; and `KE16-DESIGN-APP.md:275`. <!-- doc-anchor-ignore -->
  - `KE16-DESIGN.md`'s §7 heading, "Doc comments the pass corrects", dates nothing.
    `KE16-DESIGN-SPACE.md:550`'s "Doc corrections owed by whichever variant lands" dates nothing <!-- doc-anchor-ignore -->
    either. `KE16-DESIGN-APP.md:273`'s "Rows new in revision 3" says when a row was added, not that <!-- doc-anchor-ignore -->
    anything was verified.
  - All 14 numbers hold at `778739f0`, which wrote them, and none holds at `e6115223`:
    `loom_pool.rs` went from 1640 lines to 644 at `67563d3b`, before the lane.
  - Each line is byte-identical to `97bcf826`. That is exactly what "do not move pre-lane rot" would
    have left, so no document changes.
  - They are back in the table: the pass-6 "no longer debt" marker on each of the six rows is struck.
    So a row is debt unless its un-struck text says "no longer debt". A struck first number in
    "numbers now" records only that a pass once rewrote it, not that the row left the debt.
  - Pass 6 named `:359`, `:360` and `:554` itself, as "a table of references as they stood at a named <!-- doc-anchor-ignore -->
    commit". No commit is written there, so the rule reaches them as it reaches `:356`-`:357`. This <!-- doc-anchor-ignore -->
    goes beyond the two lines the orchestrator named, and it is put to you below.

The cases pass 6 decided by a different test, re-read under this rule:

| line | the clause, and where it is written | falls as |
|---|---|---|
| `VG-R3-P3-CULL-INTEGRATION-PLAN.md:117` | "each re-verified in this tree", heading `:109` | dated record: the heading scopes the table, and 1160 and 3261 hold at `8c9c1328` <!-- doc-anchor-ignore --> |
| `logging/03-CODES-REGISTRY.md:360` | "at HEAD", in the sentence | pre-lane rot, stays in the table. The clause was written at `b30fa810` (`git log -S`), and 3100/3158/3189 are not the three sites there, nor at its parent. Pass 6's reason, "names no commit", is replaced in place <!-- doc-anchor-ignore --> |
| `logging/05-LADDER-GATES.md:236`, `:237`, `:245` | "Measured at HEAD, before a line of L7 was written", lead-in `:230` | dated records: 3788 and 3732 hold at `b1725b32` <!-- doc-anchor-ignore --> |
| `threadpool/KE16-RESULTS.md:2257` (F2) | "False at this HEAD", in the paragraph, which names `4a363678` | dated record: 44 and 48 hold at `611999cb` <!-- doc-anchor-ignore --> |
| `logging/01-EMISSION-RING.md:253` | "re-verified this session at exactly `:3100` ..., `:3158` ... and `:3189`", in the parenthetical paragraph directly AFTER it (`:255`) | dated record, and the one borderline case: the clause follows its anchors instead of leading them. It is counted as the line's own paragraph because it restates and dates those exact three numbers, which hold at `d02f74a3`. Read strictly, `:253` is not a record: 3100 and 3158 become pre-lane rot, and 3189, TRUE at `e6115223`, becomes a lane shift owed 3207 - one token <!-- doc-anchor-ignore --> |
| `LOGGING-SYSTEM-PLAN.md:123` | "Measured this session:", lead-in `:118` | class (1), stays moved, true at HEAD. It is a present-tense rule, written a revision later (`303a7a92`) than the lead-in (`053f6c9f`) <!-- doc-anchor-ignore --> |
| `PARTICLES-PLAN.md:183`, `PBR-MATERIALS-PLAN.md:32`, `VB-P1E-HIERARCHICAL-CULL-PLAN.md:3184` | "Verified in-tree facts", "verified against the code", "Concrete precondition, verified" | not records: a verification with no moment <!-- doc-anchor-ignore --> |
| `threadpool/KE16-DESIGN-A.md:494` | "read at this checkout", `:480` | not a record: a different paragraph <!-- doc-anchor-ignore --> |
| `threadpool/KE16-DESIGN-B4.md:456`-`457` | "hold at `7f294afe`", `:445` | ~~not records: that bullet dates its own claim only~~ **[pass 8: dated records. They sit under the round-2 critique's lead-in `:312`, "Reviewed 2026-09-07 against `d647d930`. Every site below was re-verified by hand.", and 340 and 233-240 hold at `d647d930` - restored, Pass 8, (A)]** <!-- doc-anchor-ignore --> |

The six lines bring the debt to 170 numbers on 80 lines. *[pass 8: 153 on 73 - Pass 8, (C)]*

Corrected in place in pass 6:
- the restore count, "35 lines, 119 numbers";
- the six lines' rows in the restore table;
- the "Why the found lines are in the class" bullets for `:356`-`:357` and `:275`; <!-- doc-anchor-ignore -->
- "at HEAD names no commit";
- "all by the written-dating test above", which now says which of the 24 found lines are dated
  records (21) and which are pre-lane rot returned to original text (3: `KE16-DESIGN.md:356`, `:357`, <!-- doc-anchor-ignore -->
  `KE16-DESIGN-APP.md:275`). <!-- doc-anchor-ignore -->

#### Final accounting

| class | lines | numbers | state |
|---|---|---|---|
| lane-caused shifts | 46 | 91 | all true at HEAD: 90 written by passes 1-4 (78 exact, 2 REPL, 10 `.cargo/config.toml`) and 1 by pass 5 (`PARTICLES-PLAN.md:183`'s `:2041`) <!-- doc-anchor-ignore --> |
| quotations restored (pass 5) | 9 | 16 | true of the document and commit each one quotes |
| dated records restored (pass 6, under the rule in (7)) | 29 | 105 | true of the moment each one names |
| dated record kept as written (F1, `VG-R3-P3-CULL-INTEGRATION-PLAN.md:2924`) | 1 | 2 | as-of round 2's critique <!-- doc-anchor-ignore --> |
| pre-lane rot returned to its original text (pass 6) | 6 | 14 | counted in the debt below |
| **debt** | **80** | **170** | false at `e6115223` and false at HEAD *[pass 8: 73 lines, 153 numbers - Pass 8's final accounting]* |

`VG-R3-P2-CAPABILITY-SPLIT-PLAN.md:1508` is in both restored rows, so the quotations and dated records <!-- doc-anchor-ignore -->
are 37 distinct lines and 121 numbers. A lane-shift line can also carry a debt number: `:424` carries <!-- doc-anchor-ignore -->
3189 (a shift) beside 3118 and 3176 (debt).

The debt by target file:

| target | lines | numbers |
|---|---|---|
| `crates/boyko_rhi_vulkan/src/device.rs` | 34 | 50 |
| `goldens/PINS.toml` | 24 | 72 |
| `crates/boyko_threadpool/tests/loom_pool.rs` | 19 | 42 |
| `crates/boyko_threadpool/tests/miri_scope.rs` | 1 | 2 |
| `docs/threadpool/KE16-RESULTS.md` | 2 | 4 |
| **total** | **80** *[pass 8: 73]* | **170** *[pass 8: 153]* |

The debt by class, continuing pass 6's table:

| class | pass 5 | after pass 6 | after pass 7 |
|---|---|---|---|
| never true at `e6115223`, `device.rs` | 65 | 39 | 44 |
| never true at `e6115223`, `PINS.toml` | 83 | 58 | 60 |
| never true at `e6115223`, `miri_scope.rs` | 2 | 2 | 2 |
| `loom_pool.rs` | 35 | 21 | 35 |
| near misses | 6 | 4 | 4 |
| refused continuations | 21 | 21 | 21 |
| F3, `KE16-RESULTS.md` | - | 4 | 4 |
| **total** | **212** | **149** | **170** *[pass 8: 153 - Pass 8, (C)]* |

The four citation forms that escaped the scanners are listed in (2): the slash list, the comma-space
list, the section-name prefix and the markdown link.

**The durable fix on record** is the one pass 1 put to you and passes 5 and 6 repeated. Widen
`GATED_DOCS` in `tests/internal_docs_anchors.rs` to the plan corpus, in the doc-gates lane, so that rot
reds on the day it happens. Before a widened gate runs, the ~~37~~ *[pass 8: 45]* restored lines - the 9 quotations and
the 29 dated records *[pass 8: and 8 more dated records - F1's `VG-R3-P3-CULL-INTEGRATION-PLAN.md:2924`, `VB-SV0-SDF-SHADOW-PLAN.md:1627`-`1628` and the five `KE16-DESIGN-B4.md` lines; listed in Pass 8, (D)]* - need the gate's existing opt-out, `<!-- doc-anchor-ignore -->`, because
rewriting them falsifies them. Pass 7 adds one lesson. The `~` waiver is not evidence: it is how
`:2517`'s two stale numbers pass the gate that reads them today, and a widened gate that honours `~` <!-- doc-anchor-ignore -->
the same way would pass them too. The ~~80~~ *[pass 8: 73]* debt lines are what a widened gate should red on; they are
listed by name so that each can be decided before that day.

**Hygiene.**
- CRLF == LF == CR in this register. No other file was written.
- `git status` shows the same 28 ` M` files as at the start. `docs/ru/` is untouched.
- Register edits are insertions and strike-through only: each old line survives as a subsequence of
  its new text. The three new rows sit after the table's last row, so no line number that this entry
  cites has moved.
- Pass 7 changed nothing in the first 5165 lines of this file. They differ from `97bcf826` only on the
  five lines pass 1 rewrote: 1954, 2754, 4189, 4209 and 4989.
- Every citation-shaped line of the lane's entry carries `<!-- doc-anchor-ignore -->`. The check's
  pattern now matches the link form `](path):N` as well. <!-- doc-anchor-ignore -->
- No cargo; no checkout, stash, restore, reset or commit.

Scripts, stdlib, in the scratchpad's `p7/`:
- `lane90.py` - the 90 lane shifts, checked mechanically;
- `linkscan.py`, with `linkscan.json` - the link-form scan;
- `rest.py` - the verifier's judgements on the 35 restored lines;
- `w2_pass7_note.py` - this section, the in-place corrections and the table rows; `--dry` first; `w2_pass7_note2.py` - the table's reading rule, added after.

The pass-6 verifier's `v7/` is the source of items (1)-(7): `c1show.txt`, `dated2.txt`, `rem.txt`
and `cls1.py`.

**WHAT IS PUT TO YOU:**
- *[pass 8: the orchestrator's pass-8 accounting, 153 on 73, counts them as pre-lane rot in the debt]* Whether `KE16-DESIGN.md:359`, `:360` and `KE16-DESIGN-SPACE.md:554` count as pre-lane rot, as the <!-- doc-anchor-ignore -->
  rule gives them, or stay dated records by pass 6's naming. Either way no document byte changes; only
  this register's debt moves, by 4 numbers on 3 lines.
- `01-EMISSION-RING.md:253`, the one clause that follows its anchors. Read strictly, its 3189 is owed <!-- doc-anchor-ignore -->
  3207.
- The `GATED_DOCS` widening, and whether a widened gate keeps honouring `~` as a bounds-only check.

### ~~Pass 8 (owed, not applied): the pass-7 verifier's findings, stopped at the owner's checkpoint (2026-09-11 08:38)~~ *[done: see "Pass 8 (2026-09-11): the last dating-rule application" below]*

The owner asked to stop at a checkpoint and record everything. ~~Pass 8 was launched and stopped before it wrote anything, so the tree is exactly the pass-7 state.~~ *[done: pass 8 is applied - see "Pass 8 (2026-09-11): the last dating-rule application" below]* What pass 8 is to do, from the pass-7 verifier (full report: `docs/unification/checkpoint-2026-09-11/msvc-citations-pass8-work-order.md` on `feat/multi-paradigm-render`):

- ~~**A.** `KE16-DESIGN-B4.md:312` ("Reviewed 2026-09-07 against d647d930. Every site below was re-verified by hand.", under `# ROUND-2 CRITIQUE` at `:310`) is a dating lead-in under the Pass 6/7 rule, so `B4:378`, `:385`, `:421`, `:456` and `:457` are dated records: restore 575->531, 1202-1207->1158-1163, 572-577->567-572, 366->340, 259-266->233-240, strike them from the debt table, and correct the sentences that call `:408` the critique's only as-of.~~ *[done - Pass 8, (A)]* <!-- doc-anchor-ignore -->
- ~~**B.** `VB-SV0-SDF-SHADOW-PLAN.md:1627` and `:1628` (8 numbers) are dated records kept as written under the section lead-in `:1542`; move them out of the debt and refused counts. No document edit.~~ *[done - Pass 8, (B)]* <!-- doc-anchor-ignore -->
- ~~**C.** The debt becomes 153 numbers on 73 lines (device.rs 34/50, PINS.toml 22/64, loom_pool.rs 17/39), not 170 on 80.~~ *[done - Pass 8, (C)]*
- ~~**D.** The opt-out list for the doc-gates lane grows from 37 to 45 lines (adds F1's `VG-R3-P3:2924`, the two SV0 lines and the five B4 lines).~~ *[done - Pass 8, (D)]* <!-- doc-anchor-ignore -->
- ~~**E.** Minor: the as-of sentence is on `B4:408` but its anchors are on `:407`; strike the two superseded figures ("50 were true at e6115223", "147 of the 152 were").~~ *[done - Pass 8, (E)]* <!-- doc-anchor-ignore -->

~~The check for pass 8 is mechanical: re-run the pass-7 verifier's scripts and expect 153 on 73 with clean hygiene. The durable fix for the named debt stays the doc-gates lane's: widen `GATED_DOCS` to the plan corpus after the opt-out markers are added.~~ *[done: the check was run - see "Pass 8 (2026-09-11): the last dating-rule application" below]*

### Pass 8 (2026-09-11): the last dating-rule application

The pass-7 verifier found no arithmetic error. It found two lead-ins that the dating rule covers and
that no pass had read, so seven debt lines were classified wrong. The orchestrator decided (A)-(E)
below and fixed the scope: one document edit, five lines of `threadpool/KE16-DESIGN-B4.md`, digits
only, and this register. Every figure here is recomputed by the scripts named at the end.

**(A) `KE16-DESIGN-B4.md`'s round-2 critique is dated by its lead-in, and its five debt lines are restored.**

`:310` is the heading `# ⚠ ROUND-2 CRITIQUE — three blocking findings. B4-1 IS NOT FREE.` `:312`, directly <!-- doc-anchor-ignore -->
under it, reads "Reviewed 2026-09-07 against `d647d930`. Every site below was re-verified by hand."
No other top-level heading follows, so the critique runs from `:310` to the end of the file (`:463`). <!-- doc-anchor-ignore -->
`git blame` gives `:310`-`:312` and all five lines to `7fdd738e`, and the document-wide header is `:3`, <!-- doc-anchor-ignore -->
a different line. This is the same shape of lead-in as `VG-R3-P2-CAPABILITY-SPLIT-PLAN.md:1426` and <!-- doc-anchor-ignore -->
`VG-R3-P3-CULL-INTEGRATION-PLAN.md:2712`/`:2890`, which were already counted as dating clauses. <!-- doc-anchor-ignore -->
`d647d930..7fdd738e` changes only `KE16-DESIGN-B4.md`, so every number below also holds at the commit
that wrote the lines.

| line | before (pass 1's text) | after (`97bcf826` and `e6115223` text) | what holds at `d647d930` |
|---|---|---|---|
| `:378` | `KE16-RESULTS.md:12` ... `:575` | `KE16-RESULTS.md:12` ... `:531` | `:12` "\| B \| **B0 BY CONSTRUCTION, NOT BY MEASUREMENT** ..."; `:531` "> axis A closes on `a3` and axis B stops.**" <!-- doc-anchor-ignore --> |
| `:385` | `KE16-RESULTS.md:1202-1207` | `KE16-RESULTS.md:1158-1163` | the `a1` worker-route block: 1158 "**`a1`, worker route:** W=4 three reps ...", `top_lane=13/24/21` at 1160, 1163 "`scratch` runs on a registered worker." <!-- doc-anchor-ignore --> |
| `:421` | `tests/miri_scope.rs:572-577` | `tests/miri_scope.rs:567-572` | 567 `#[cfg_attr(`, 568 `not(any(feature = "ke16-a1", feature = "ke16-a1-fifo")),`, 572 `)]`, on `nested_scope_inline_body_spawns_through_tls_deque_under_live_join` at 573 <!-- doc-anchor-ignore --> |
| `:456` | `loom_pool.rs:366` | `loom_pool.rs:340` | 340 `fence(Ordering::SeqCst); // injector steal transport fence` <!-- doc-anchor-ignore --> |
| `:457` | `:259-266` | `:233-240` | 233 "// The CONSUMER's fence is still the transport's, and is modelled: ...", through 240 "// claim CAS) remain REAL production code." <!-- doc-anchor-ignore --> |

`w2_pass8.py --dry` asserted three things before the write. Each restored line equals its `97bcf826`
and its `e6115223` line. It differed from the tree line only in digits. And the `d647d930` text in the
last column is where the table says it is. `KE16-DESIGN-B4.md` still differs from `97bcf826` on one
line, `:72`, which is above the critique and stays in the debt. <!-- doc-anchor-ignore -->

The five rows of the debt table are marked "no longer debt", and their "numbers now" cells show the
restored values. These register sentences were false, and each is corrected in place by
strike-through and insertion:
- the `KE16-DESIGN-B4.md:378` row: "which declares no as-of for", and "Its one as-of, at `:408`, scopes <!-- doc-anchor-ignore -->
  two ... anchors on its own line";
- pass 6's F3: its heading's "are debt", "Neither line is a dated record", "Both lines are therefore
  *moved, still false*", and in its pass-7 bracket "It does declare one", "on its own line" and "F3's
  debt decision stands";
- pass 6's rule bullet, whose example was this critique;
- pass 6's F4 (b), "F3's 3 ... in the table";
- pass 6's sweep list and its WHAT IS PUT TO YOU, which left `:456`-`:457` alone; <!-- doc-anchor-ignore -->
- pass 7's (6): "own line" and "so F3's debt decision stands";
- pass 7's (7) row for `:456`-`:457`, "that bullet dates its own claim only"; <!-- doc-anchor-ignore -->
- pass 7's 152 table, "F3's 3 and the five above | 8", now 5.

Of the pass-4 verifier's 152 TRUE numbers, the three F3 numbers (`:378`'s 531, `:385`'s 1158 and 1163) <!-- doc-anchor-ignore -->
were false at `e6115223` and are now dated records:

| where the 152 stand | numbers |
|---|---|
| moved, and true at HEAD | 90 |
| on lines pass 6 restored as dated records | 50 |
| restored by pass 5 as a quotation (`05-LADDER-GATES.md:231`) | 2 <!-- doc-anchor-ignore --> |
| false at `e6115223`, restored as dated records: F2's 2 (pass 6) and F3's 3 (pass 8) | 5 |
| false at `e6115223`, in the debt: the five `device.rs:3158` | 5 <!-- doc-anchor-ignore --> |
| **total** | **152** |

**(B) `VB-SV0-SDF-SHADOW-PLAN.md:1627` and `:1628` are dated records kept as written.** They carry 8 <!-- doc-anchor-ignore -->
numbers, which passes 5-7 counted as refused continuations. They sit in the same paragraph as `:1625`, <!-- doc-anchor-ignore -->
which pass 6 restored, under the same §9 lead-in, `:1542`: "Every line below was opened while writing <!-- doc-anchor-ignore -->
**this revision**". `git blame` gives `:1542`, `:1627` and `:1628` to `62731d91` (its lines 767, 832 <!-- doc-anchor-ignore -->
and 833, identical to the tree). At `62731d91`, `goldens/PINS.toml` has:
- `[vb_both]` at 313, its block ending at 343 (`RUSTUP_TOOLCHAIN`, then a blank);
- the "boot-seeded EMPTY" edit-list note at 322;
- the note that treats the pre-filled hashes as live at 326-328;
- `[vb_sdf_only]` at 345, its block ending at 377;
- its "boot-seeded EMPTY" note at 355.

All 8 numbers hold. Both lines are byte-identical at `62731d91`, at `e6115223`, at `97bcf826` and in the
tree, so no document changes. Pass 4's refusal to move them stands. They are the same class as F1's
`VG-R3-P3-CULL-INTEGRATION-PLAN.md:2924`. The two table rows are marked "no longer debt", and pass 4's <!-- doc-anchor-ignore -->
bullet and its residual 6 carry a pointer.

**(C) The debt, recomputed: 153 numbers on 73 lines.** `debt8.py` parses the table with the
register's own reading rule. On the pre-pass register it reproduces pass 7's 170 on 80 and every
per-target and per-class figure. On the written register it gives:

| target | lines (pass 7) | numbers (pass 7) | lines | numbers |
|---|---|---|---|---|
| `crates/boyko_rhi_vulkan/src/device.rs` | 34 | 50 | 34 | 50 |
| `goldens/PINS.toml` | 24 | 72 | 22 | 64 |
| `crates/boyko_threadpool/tests/loom_pool.rs` | 19 | 42 | 17 | 39 |
| `crates/boyko_threadpool/tests/miri_scope.rs` | 1 | 2 | 0 | 0 |
| `docs/threadpool/KE16-RESULTS.md` | 2 | 4 | 0 | 0 |
| **total** | **80** | **170** | **73** | **153** |

| class | pass 5 | after pass 6 | after pass 7 | after pass 8 |
|---|---|---|---|---|
| never true at `e6115223`, `device.rs` | 65 | 39 | 44 | 44 |
| never true at `e6115223`, `PINS.toml` | 83 | 58 | 60 | 60 |
| never true at `e6115223`, `miri_scope.rs` | 2 | 2 | 2 | 0 |
| `loom_pool.rs` | 35 | 21 | 35 | 32 |
| near misses | 6 | 4 | 4 | 4 |
| refused continuations | 21 | 21 | 21 | 13 |
| F3, `KE16-RESULTS.md` | - | 4 | 4 | 0 |
| **total** | **212** | **149** | **170** | **153** |

It matches the orchestrator's expected figure. It keeps `KE16-DESIGN.md:359`, `:360` <!-- doc-anchor-ignore -->
and `KE16-DESIGN-SPACE.md:554` as pre-lane rot, which pass 7 put to you; counted as dated records they <!-- doc-anchor-ignore -->
would make it 149 on 70.

**(D) The durable-fix opt-out list: 45 lines.** Every restored or kept-as-written dated record and
every quotation needs `<!-- doc-anchor-ignore -->` before `GATED_DOCS` widens, because a widened gate
would otherwise red on it and invite rewriting it. Pass 7 listed 37: the 9 quotations and the 29 dated
records, with `VG-R3-P2-CAPABILITY-SPLIT-PLAN.md:1508` in both. It left out 8: F1's `:2924`, the two <!-- doc-anchor-ignore -->
SV0 lines, and the five B4 lines. None of the 45 carries the marker today, and this pass adds none;
the list is for the doc-gates lane. The count is corrected in place in pass 7's durable-fix paragraph.

| document | lines | count |
|---|---|---|
| `LOGGING-SYSTEM-PLAN.md` | `:119`, `:491` | 2 <!-- doc-anchor-ignore --> |
| `VB-P1E-HIERARCHICAL-CULL-PLAN.md` | `:3452`, `:3453` | 2 <!-- doc-anchor-ignore --> |
| `VB-PERFORMANCE-TRACK.md` | `:266` | 1 <!-- doc-anchor-ignore --> |
| `VB-SV0-SDF-SHADOW-PLAN.md` | `:1625`, `:1627`, `:1628` | 3 <!-- doc-anchor-ignore --> |
| `VG-R3-P2-CAPABILITY-SPLIT-PLAN.md` | `:882`, `:1010`, `:1500`, `:1504`, `:1506`, `:1508`, `:1538`, `:1548`, `:1556` | 9 <!-- doc-anchor-ignore --> |
| `VG-R3-P3-CULL-INTEGRATION-PLAN.md` | `:117`, `:2851`, `:2924` | 3 <!-- doc-anchor-ignore --> |
| `VG-R3-P4-CONFIG-AND-INSTRUMENT-PLAN.md` | `:50`, `:51`, `:52`, `:53`, `:260`, `:796`, `:797`, `:798` | 8 <!-- doc-anchor-ignore --> |
| `diagnostics/logging/00-GOAL-TARGETS.md` | `:111`, `:150`, `:191` | 3 <!-- doc-anchor-ignore --> |
| `diagnostics/logging/01-EMISSION-RING.md` | `:253` | 1 <!-- doc-anchor-ignore --> |
| `diagnostics/logging/02-SINK-LIFECYCLE.md` | `:53` | 1 <!-- doc-anchor-ignore --> |
| `diagnostics/logging/05-LADDER-GATES.md` | `:231`, `:236`, `:237`, `:245` | 4 <!-- doc-anchor-ignore --> |
| `editor/EDITOR-BOUNDARY.md` | `:41` | 1 <!-- doc-anchor-ignore --> |
| `threadpool/KE16-DESIGN-B4.md` | `:378`, `:385`, `:421`, `:456`, `:457` | 5 <!-- doc-anchor-ignore --> |
| `threadpool/KE16-RESULTS.md` | `:1826`, `:2257` | 2 <!-- doc-anchor-ignore --> |
| **total** | | **45** |

**(E) Minor corrections, in place.**
- Pass 6's F3 bracket, the `KE16-DESIGN-B4.md:378` row and pass 7's (6) said the `:408` as-of scopes <!-- doc-anchor-ignore -->
  the census anchors "on its own line". The anchors are on `:407`; the as-of sentence is on `:408`. <!-- doc-anchor-ignore -->
- The two superseded figures are now struck, not only bracketed: pass 6's "50 were true at `e6115223`"
  and "147 of the 152 were".

#### Final accounting

| class | lines | numbers | state |
|---|---|---|---|
| lane-caused shifts | 46 | 91 | all true at HEAD, unchanged since pass 7: 90 written by passes 1-4 and `PARTICLES-PLAN.md:183`'s `:2041` <!-- doc-anchor-ignore --> |
| quotations restored (pass 5) | 9 | 16 | true of the document and commit each one quotes |
| dated records restored (pass 6) | 29 | 105 | true of the moment each one names |
| dated records restored (pass 8, `KE16-DESIGN-B4.md`'s round-2 critique) | 5 | 9 | true at `d647d930` |
| dated records kept as written (F1's `VG-R3-P3-CULL-INTEGRATION-PLAN.md:2924`; `VB-SV0-SDF-SHADOW-PLAN.md:1627`-`1628`) | 3 | 10 | true at their critique and at `62731d91` <!-- doc-anchor-ignore --> |
| pre-lane rot returned to its original text (pass 6) | 6 | 14 | counted in the debt |
| **debt** | **73** | **153** | false at `e6115223` and false at HEAD |

The quotations and dated records are 45 distinct lines, because `:1508` is in two rows, and 140 <!-- doc-anchor-ignore -->
numbers: pass 7's 121, plus 2, 9 and 8. Those 45 lines are the opt-out list in (D).

**The lane is closed.** Every lane-caused shift is true at HEAD. Every quotation and dated record
is back to the text of its moment. Every remaining false number is named debt, by line, in the table
of pass 5 (3), and it belongs to the doc-gates lane. That lane has three tasks. First, add the 45
markers. Then widen `GATED_DOCS` to the plan corpus. Then decide the 73 debt lines, and whether a
widened gate keeps honouring `~` as a bounds-only check. Pass 7's other open item,
`01-EMISSION-RING.md:253`, stays as pass 7 counted it: a dated record. <!-- doc-anchor-ignore -->

**Hygiene.**
- CRLF == LF == CR in both files pass 8 wrote.
- `git status` shows two ` M` files, `docs/OPEN-QUESTIONS.md` and `docs/threadpool/KE16-DESIGN-B4.md`.
  `docs/ru/` is untouched.
- Register edits are insertions and strike-through only: each old line survives as a subsequence of its
  new text. No line was added or removed above this section, so no line number this entry cites has
  moved. The first 5165 lines are unchanged.
- Every citation-shaped line of the lane's entry carries `<!-- doc-anchor-ignore -->`.
- The "Pass 8 (owed, not applied)" section is kept and marked done in place.
- No cargo; no checkout, stash, restore, reset or commit.

Scripts, stdlib, in the scratchpad's `p8/`:
- `w2_pass8.py`: the five restores, the in-place corrections and this section. `--dry` first, with its
  log in `dry.log`.
- `debt8.py`: the table's reading rule, with targets and classes.

The pass-7 verifier's `v8/` is the source of (A)-(E): `debt.py`, `leadin.py`, `ins.py`, `link.py` and
`lane90_repl.py`.

---

## 2026-08-21: UI-ADVANCED S3 — `NonUniformResourceIndex` is UNGATED on this box, and the reason is the same one that made §10.1 flat

**Status: recorded, not blocking. Owner-facing because it is a gate that CANNOT fail here, which is
the class this project keeps finding late.**

S3's red mutation **M3-b** — drop `NonUniformResourceIndex` from the `ui_rect.fs` sprite branch,
re-emit, re-DXC — was run on this box (RTX 3060 Laptop, validation on). It **did not red**. Not the
new 64-slot gate (`ui_sprite_divergence.rs`: 256 dense 4×4-px quads over 64 distinct bindless slots,
every quad asserted against its own texture), not either sprite golden, not the validation
messenger.

**The cause is structural, and it is the same fact behind §10.1's flat measurement.** The descriptor
index is `nointerpolation`, i.e. per INSTANCE. This rasterizer does not pack one warp from two
primitives, so a per-instance index is wave-uniform *by construction* here — there is no divergence
to punish, which is also why 1 / 8 / 64 distinct slots timed identically. Two observations, one
cause; each makes the other believable.

**Why the qualifier stays anyway.** A non-uniform descriptor index without it is **undefined
behaviour by the Vulkan spec**, not "usually fine". Another driver, another vendor, or a future
rasterizer that packs warps differently is free to resolve one lane's descriptor for the whole wave.
Removing it because this box tolerates it would be trading a spec guarantee for one machine's
observation.

**What IS live on it:** the byte gate. `ui_rect_spv_sync` sees the qualifier's removal (the `.spv`
moves 8760 → 8680 B) even though no pixel does, and `ui_rect_edsl_sync` sees a hand-edit of the
span. So the qualifier is pinned to the generator — which catches an accidental removal, but is not
the same thing as a test that catches its CONSEQUENCE.

**The open half, for the owner:** whether this campaign should acquire a hardware leg that can
actually make a divergent-descriptor read wrong (a vendor whose warps span primitives, or a
synthetic shader that forces one wave across two indices), or whether "spec-required, byte-gated,
consequence-unobservable-here" is the honest resting place. Nothing in S3–S5 depends on the answer;
S7's Model-A disposition already has its number from §10.1.

---

## 2026-08-26: UI-ADVANCED S6 — a component's `on_remove` hook CANNOT enqueue a removal, because despawn fires it too and the entity is dead by the drain

**Status: KERNEL DEFECT, worked around in S6. Owner-facing because the workaround leaves a capability
missing, and because the shape generalises to every `on_remove` hook anyone writes.**

S6 closes the sprite-cursor hole with `#[component(on_add = …)]` on `UiSpriteAnim`: the hook
deferred-inserts the dense `UiSpriteCursor` through a one-field `Bundle` wrapper. That half works on
every construction path (MEASURED: a fresh `Commands::spawn`, and `cmds.entity(e).insert(anim)` onto
a live entity, both leave `has_component(e, UiSpriteCursor::component_id()) == true` with
`dir: 1` after the apply).

**The symmetric `on_remove` hook does not, and the failure is a panic rather than a no-op.**

```
thread '…' panicked at crates/boyko_ecs/src/ecs/core/commands/remove_command.rs:75:13:
RemoveCommand::apply: stale entity Entity { id: EntityId(0), generation: 0 }
```

MEASURED, in this order:

1. `on_remove` fires on an ordinary `cmds.entity(e).remove::<Anim>()` and the enqueued
   `remove::<Cursor>()` applies correctly. That is the case the hook is for.
2. `on_remove` ALSO fires on the per-component pass of a DESPAWN. At hook time the entity is still
   live — `w.is_alive(ctx.entity)` reads **`true`** — so the obvious guard does not help. By the time
   the outermost drain runs the enqueued `RemoveCommand`, the entity is gone, and
   `RemoveCommand::apply` panics on the stale handle rather than treating it as a no-op.
3. A despawn already reclaims the dense row on its own (`has_component` after despawn = `false`), so
   the hook would buy nothing at despawn even if it could run.

**So the hook is safe only for components nobody ever despawns**, which is not a property a component
author can check. Today the only hazard is the one S6 declined to take, but any future
`on_remove` hook that touches `commands()` inherits it silently.

**Three options, and the SCOPE call is the owner's:**

1. **Make `RemoveCommand::apply` (and `InsertCommand::apply`) tolerate a stale entity** — a dead
   entity's component removal is already a no-op semantically. This is the smallest change and the
   one that makes `on_remove` hooks generally usable. It weakens a deliberate liveness assertion,
   which is presumably there to catch a different class of bug, so it is not free.
2. **Give the hook a way to know it is firing inside a despawn** — an `on_despawn`-in-progress flag
   on `HookContext`, or simply documenting that `on_remove` must not enqueue anything and enforcing
   it. This keeps the assertion and makes the restriction visible instead of latent.
3. **Leave it, and document `on_remove` + `commands()` as unsupported.** That is effectively the
   status quo, and it is what S6 assumed once the measurement came back.

**What it blocks today:** nothing. S6 lands `on_add` alone. An animation removed from a SURVIVING
node (a `.ui` deletion that keeps the node) leaves an 8 B dense cursor row behind; it is inert (the
flipbook queries all three components) and self-healing (a re-added animation gets a fresh `Default`
cursor — MEASURED). Recorded in `docs/UI-PLAN-SPRITES-DECISIONS.md` [S-D20](UI-PLAN-SPRITES-DECISIONS.md#s-d20--the-s6-pre-build-audit-the-cursor-hole-closes-with-a-hook-and-six-of-the-rungs-own-sentences-did-not-survive-the-tree) (1) and [S-D21](UI-PLAN-SPRITES-DECISIONS.md#s-d21--the-six-corrections-building-s6-found-and-the-pre-existing-bug-the-rung-could-not-build-around) (1).

---

## 2026-08-26: UI-ADVANCED S6 — `.ui` bracketed values had NEVER parsed, and the test that should have caught it was green by coincidence

**Status: FIXED in S6, recorded because the FALSE DOC and the coincidental green are the interesting
half, not the one-line fix.**

`crates/boyko_ui/src/text/split.rs`'s `split_top_level` tracked paren depth and quotes but not
`[`/`]`. Its own doc said the P3 field list is *"provably free of `{`/`[`/quoted-comma values …
locked by a rejection test"*. **Both halves were false**: GUI P6a added `UiImage`'s
`uv_min`/`uv_max`, which are `[u, v]`, and `grep` finds no such rejection test anywhere in the tree.

MEASURED consequence: `UiImage { texture: 7, uv_min: [0, 0], uv_max: [1, 1], tint: … }` split into
`uv_min: [0` / `0]` / `uv_max: [1` / `1]`; `parse_f32_pair` rejected both UV fields; both kept their
`Default`s; four recoverable errors went into the LOWERING report.

**`p6a_equivalence::image_widget_three_ways_equivalent` was green over it for two independent
reasons, and closing either one alone would not have been enough:**

* `p3_common::spawn_dot_ui` asserts the PARSE report and hands the lowering an
  `owned.report.clone()` that is dropped, so the four errors were unobservable through the harness;
* the authored UVs happen to EQUAL `UiImage::default()`'s (`[0,0]` / `[1,1]`), so the mis-parse
  landed back on the right values.

A test that meant to prove "the `.ui` path carries these UVs" proved that the defaults are
`[0,0]`/`[1,1]`.

**Fixed** by making `(` and `[` open the same depth counter (and `)`/`]` close it). The
`boyko_input` copy of the function is untouched — `.keys` has no bracketed values — so the file's
"COPIED VERBATIM" header now reads "copied, then DIVERGED, and here is why".

**The open half, for the owner:** the *harness* defect is still there. `spawn_dot_ui` drops the
lowering report, so any `.ui` corpus test can be green over a per-field parse error. S6's own gates
route around it (`ui_s6_authoring` captures the lowering report itself), but the shared harness is
what most `.ui` tests use. Making `spawn_dot_ui` assert the lowering report would red today on
anything else already mis-parsing, which is a repair rung with its own budget rather than a line in
S6.

## 2026-08-27: UI-ADVANCED A1 — a tween duration above ~6 days is accepted and is IMMORTAL, and the bound is a VALUES call

**Status: OPEN — a VALUES call, deliberately NOT decided by the rung.** The behaviour is documented
at the site and gated; what is not decided is whether to refuse it.

`crates/boyko_ui/src/animation.rs`'s `invalid_tween_duration` guard closes **degenerate
reciprocals** — `duration_ms` non-finite or not positively signed. It does **not** close the
"immortal row" class, and A1 part 4 retracts any reading that it does.

**The mechanism, MEASURED by exact `f32` simulation 2026-08-27.** `elapsed` is an `f32` accumulating
`+= dt`, so absorption gives it a HARD CEILING — past it, `elapsed += dt` is a no-op and the value
never grows again:

| `dt` | `elapsed` ceiling | in days | frames to reach |
|---|---|---|---|
| 1/60 s | **524288 s** (`2^19`) | **6.068** | 24,986,955 |
| 16 ms | 524288 s (`2^19`) | 6.068 | 25,150,895 |
| 100 ms — the `UiClock` clamp ceiling | **2097152 s** (`2^21`) | **24.273** | 18,073,720 |

A row completes only when `elapsed` reaches `duration_ms / 1000`. **Above the ceiling it never can,
no matter how long the process runs.** At 60 Hz that boundary is `duration_ms > 5.24288e8`
(≈ 6.07 days) — ⚠️ **STRICTLY above: the operator is `>`, not `>=`, because the boundary value
itself COMPLETES.** Re-measured at THIS site 2026-08-27 (`rustc -O`, exact `f32`), because the
number was first measured elsewhere and carried here: at `duration_ms = 5.24288e8` (bits
`0x4dfa0000`, exactly `524288000`) `inv_duration` is bits `0x36000000` — exactly `2^-19` — so `t`
at the ceiling is `524288.0 * 2^-19` = exactly `1.0` (bits `0x3f800000`), and `advance`'s `t < 1.0`
spelling puts exactly `1.0` on the COMPLETING side. The first genuinely never-completing duration is
ONE ULP ABOVE it: `5.24288032e8`, bits **`0x4dfa0001`**, whose `t` at the ceiling is `0.99999994`.
The 100 ms clamp behaves identically — `2.097152e9` (bits `0x4efa0000`) reaches exactly `1.0` at
`2^21` and completes; `0x4efa0001` is the first that does not. `1e10` ("115 days"), `1e30` and
`f32::MAX` are **all** above it — so all three are
immortal in exactly the sense the guard claims to have closed.

**Why this is worse than the shape that IS refused.** An accepted over-ceiling row bumps
`set_if_neq` on **every** frame. The REFUSED `+inf` bumps **zero** times after the first (its `t` is
a constant). So `f32::MAX` — accepted — is strictly worse for the A4 repaint skip than `+inf` —
refused — while the opacity it renders never leaves the neighbourhood of its START endpoint, which
is visually the same picture as the refusal.

⚠️ **The opacity figure, RE-MEASURED at THIS site 2026-08-28.** This sentence used to name
`3.673e-36`, and that number belonged to **no frame of any fixture** — it is corrected rather than
dropped. Read back FROM THE ENGINE through the gate's own `f32::MAX` arm
(`start_tween_opacity(.., 0.0, 1.0, f32::MAX, LINEAR, 0)` at `FRAME = 100 ms`) and, independently,
by exact `f32` simulation of `advance` — the two agreeing bit-for-bit on every frame and every arm:

| frame | opacity | bits |
|---|---|---|
| 1 | **2.938736e-37** | `0x02c80001` |
| 5 | **1.469368e-36** | `0x03fa0001` |

⚠️ **`2.938736e-37` shares all seven mantissa digits with the `2.938736e-36` in the paragraph
below**, and that is not a coincidence: `1000.0 / f32::MAX` is bits `0x047a0001`, the same bit
pattern named there as the smallest duration with a finite reciprocal. A rendered opacity and a
stored reciprocal are different quantities one decade apart — tell them apart by BITS, never by the
printed digits. This adjacency is the most likely origin of the wrong figure above.

**A second, benign boundary sits far below**, and is recorded only so it is not confused with the
first: `1000.0 / duration_ms` overflows to `+inf` only for `duration_ms` below bits **`0x047a0001`**
(`2.9387360564219222e-36`) — that is the SMALLEST duration with a finite reciprocal. ⚠️ **`250.0 ×
f32::MIN_POSITIVE` is NOT that value.** Re-measured at THIS site 2026-08-27: the product is bits
`0x047a0000` (`2.9387358770557188e-36`), ONE ULP BELOW, and it is the LARGEST duration that still
overflows — the closed form named the value on the wrong side of the boundary it defines. ⚠️
**State it by BITS** — the two
floats bracketing that boundary both print `2.938736e-36` at 7 significant figures (`{:.6e}`), so
**no decimal AT THAT WIDTH** can name it. ⚠️ The unqualified form of that sentence — *"so a decimal
cannot name it"* — is FALSE, and the same paragraph refutes it: the two 17-digit decimals printed
above round-trip **exactly** to their stated bits (re-verified 2026-08-28), and even the
shortest-round-trip form distinguishes them (`2.9387359e-36` vs `2.938736e-36`). The limit is the
WIDTH, not decimal. That regime is where `+0.0` and the denormals live, and it is benign: those SNAP
to the endpoint.

**The concrete option, if the owner wants the class closed.** Add a second conjunct
`duration_ms < UI_MAX_TWEEN_MS` to the existing `if` in `tween_helpers!`
(`crates/boyko_ui/src/animation.rs:885` — `if !(duration_ms.is_finite() &&
duration_ms.is_sign_positive())`; the macro begins at `:828`, and the call it guards,
`invalid_tween_duration(duration_ms);`, is at `:886`. All re-read by CONTENT 2026-08-28 for the
third time: they were `:839` / `:782`, then `:857` / `:800` after this entry's own opacity
correction added 18 doc lines above them, and the animation census landing later that day added a
further **+28**. ⚠️ Two successive re-readings by content, one day apart, and the second was stale
within hours — an anchor into a file under active edit is a measurement with a shelf life, not a
fact. The class this entry asks the owner about is now ALSO pinned mechanically, by
`the_termination_condition_is_pinned_to_the_disclosure` in
`crates/boyko_ui/tests/ui_a1_source_census.rs`, which reds if this guard is weakened, clamped,
or bounded — so a stale coordinate here no longer means the question is unguarded).

> **Price note, because this campaign's two cautionary prices do NOT transfer.** That guard is on
> the `start_*` path — **once per tween start**, not per row per frame. The two prices on record
> (+15 ns / +5 %, and +0.536 ns / +3.8 %) were both **per-row** guards on the TICK path. If the
> answer is yes, price it anyway with the FLOOR statistic, interleaved A/B/A/B across process
> invocations (a one-shot read on this box spreads 14.0–22.9 ns for identical code), and add an arm
> to `a_degenerate_duration_creates_no_row`.

**What is NOT open:** the boundary is `dt`-dependent (6.07 d at 60 Hz vs 24.27 d at the clamp
ceiling), so any compile-time constant is a conservative policy choice rather than a derived one —
which is exactly why the rung did not pick one silently. The disclosure is gated meanwhile by
`crates/boyko_ui/tests/ui_a1_tween.rs`'s
`an_over_ceiling_duration_is_accepted_and_never_completes`, so if a later rung adds an upper bound
that test reds and the documentation must be rewritten in the same edit.

## 2026-08-27: the crate's PRE-EXISTING `zero_alloc` suite is FLAKY in release — A1 neither introduced nor fixed it

**Filed as a SEPARATE defect and deliberately NOT fixed inside the A1 landing.** It is recorded
because A1's release certification is intermittently red and the red is not A1's; a reader who
re-runs the suite has to be able to tell the two apart without re-deriving this.

**MEASURED at THIS site 2026-08-27** (`rustc 1.97.1`, release, the pre-existing binary
`target/release/deps/zero_alloc-*.exe` run standalone, `--test-threads=1`, each run a separate
process): **5 red in 60 runs**. Sites: `crates/boyko_ui/tests/zero_alloc.rs:238` alone 4 runs,
`:296` alone 1 run, both together 0. Verbatim:

```
thread 'unchanged_frame_layout_pair_allocates_zero_over_baseline' panicked at crates\boyko_ui\tests\zero_alloc.rs:238:5:
steady-state: layout pair must allocate no more than the scheduler baseline (baseline 5, pair 6; the layout pair's own per-frame allocs = 1)
```

`:296` is the sibling `resize_frame_layout_pair_allocates_zero_over_baseline`. What first surfaced
it was three whole-package release runs (`cargo test -p boyko-ui --all-targets --release`) reading
`0, 101, 0`; the 60-run tally above is the re-measurement that pins the rate and both sites.
[`docs/UI-PLAN-ANIMATION-A1.md`'s part-4 landing note](UI-PLAN-ANIMATION-A1.md#a1--the-sink-the-four-channels-the-fused-tick--size-l--no-cross-plan-dependency) already recorded the `boyko_ui/tests/zero_alloc.rs:296` half independently at
**6 red / 100 standalone runs**; this entry is the owner-facing filing of the same defect, widened to
`boyko_ui/tests/zero_alloc.rs:238` and given the per-site mechanism. **The rate is load-dependent and the site mix moves with it**
— 6 % at `boyko_ui/tests/zero_alloc.rs:296` there, 6.7 % at `:238` and 1.7 % at `:296` here — so treat the rate as a range, not a
constant.

**A1 neither introduced nor fixed it, and that is PROVEN rather than asserted.**
`crates/boyko_ui/tests/zero_alloc.rs` is untouched by the landing (`git status` on that path is
empty, so the file is at `e7a16fd9`), and `crates/boyko_ui/src/layout.rs` — the code these two
assertions measure — has **zero non-comment changed lines** in the landing.

**The mechanism — and ⚠️ the two sites do NOT share a statistic, so they are stated separately
rather than one measurement being carried to the other.** Both assertions are `assert!(pair <= base)`
with ZERO slack, and A1's D1/E1 measured, on ITS OWN fixture, that the parallel executor contributes
a sporadic **+1** allocation per frame (idle rate 0.20–0.34 per frame per index). What differs is how
each side is sampled:

* **`boyko_ui/tests/zero_alloc.rs:238`** takes each side from `warmed_idle_allocs` (`boyko_ui/tests/zero_alloc.rs:189-194`; call sites `:216`
  and `:220`), which reduces four samples with **`.max()`** — the statistic A1's own D1/E1 was
  convened to replace:

  ```rust
  (0..4).map(|_| count_allocs(|| sched.run(world))).max().unwrap_or(0)
  ```

  so ONE noisy sample anywhere in the pair's four promotes `pair` to `base + 1` for the whole test.
* **`:296`** does not use `warmed_idle_allocs` at all: `base` and `pair` are each a **single**
  `count_allocs` of ONE frame (`:286` and `:294`). One sample against one sample — strictly weaker
  than max-against-max, and it reds whenever that one pair frame is the noisy one.

**The file's own comment at `:231-237` already saw half of this**: it relaxed
`assert_eq!` to `<=` in 2026-08-11 after catching `baseline 6, pair 5`, and states that
"`warmed_idle_allocs` takes the MAX of four samples on each side independently". But it reasons only
about the BASELINE's max drifting above the pair's, which `<=` absorbs. **The direction that
actually reds is the opposite one, and `<=` does not absorb it.**

**The remedy is on the shelf, not invented here.** `ui_a1_zero_alloc.rs` replaced exactly this
statistic with a floor over independent REPETITIONS of the same frame index (`REPS = 8`), which
removes per-frame-random noise while keeping per-site resolution — `.max()` does neither. Applying
it here is a mechanical change to `warmed_idle_allocs` and its call sites.

**VALUES call for the owner.** Does the `zero_alloc` suite get re-based on the repetition floor now,
or stay flaky until the rung that owns it lands? It was left alone here on purpose: re-basing
another rung's gate from inside the A1 landing is the "the repair for the previous pass carried the
next defect" pattern this campaign has already paid for twice, and a suite that reds ~8 % of release
runs is a known quantity where a silently re-based one is not.

## ANSWERED 2026-08-27 — B.13 #2 (the four by-id kernel items): **APPROVED, all four**

The owner approved the seam in full: **S1** `add_component_by_id`, **S2**
`remove_component_by_id`, **S3** `mark_component_changed`, **S4′**
`EnableTagId::try_from_component_id`. Not a subset — all four, in one call, as asked.

**EG2 is unblocked**, and with it **EG3, EG5, EG6** and **BOUNDARY B4**.

### What was measured before the answer, so the record shows what it rested on

The owner's question was whether this costs the *shipping* build anything. Measured, controlled,
on this machine:

| | release binary | probe symbols in the image |
|---|---|---|
| kernel as committed | 2 656 768 B | — |
| kernel + four uncalled `pub fn` of EG2's shape | 2 656 768 B | **0** |

**Delta: 0 bytes.** The linker discards an uncalled `pub fn` entirely, and it does so *without*
LTO — this workspace has no `[profile.release]` section at all, so release runs on Cargo's
defaults with LTO off.

⚠️ **The first attempt at this measurement lied**, reporting +69 120 B. One of the two builds had
not recompiled the kernel. Caught by rebuilding three times from identical source and getting a
byte-identical binary each time — the build is deterministic, so the spread was the instrument's,
not the code's. The number above is from the controlled repeat.

### The hot path, which was decided before any code

**D9** already refuses the shape that would have cost something: widening
`migrate_entity_attach_ids` with an `Option<&[&[u8]]>` parameter would put a branch in **every tag
attach**, and would blur the ZST `debug_assert!` — which is not decoration but the statement that
makes the byte-write-free fast path sound — into *"…unless bytes were supplied"*, which is no
longer checkable. S1 is a **bytes-carrying sibling**, so the existing path keeps zero new branches.

### The cost that IS real, stated at approval time

Four `pub fn` become part of `boyko_ecs`'s permanent contract. Removing them later is a breaking
change. That is the price of the answer, and it was named before it was given.

### Not measured

The delta was taken on a small fixture binary, not on the full engine. The mechanism (an uncalled
symbol is discarded) does not depend on binary size, but the number was not re-taken there. And
**S3 is the one of the four that adds a NEW mechanism** rather than opening an existing one —
there is no by-id change-tick write in the kernel today.

---

## ANSWERED 2026-08-27 — the retained-id migration panic: **(a), fix the kernel first**

The owner chose **(a)**: the retained-id filter is restored in `boyko_ecs` as **its own change,
before EG2 and outside its diff**, exactly as `REFLECTION-PLAN-ECS.md` prescribes for this answer.
**LANDED as `f7c46c76`, and the sweep made it SIX sites, not two** — four panic-site functions,
guarding six `.expect` calls between them, plus two archetype resolvers that seed the unfiltered
list into newly-minted archetypes: six guards in total, each independently gated by
`crates/boyko_ecs/tests/retained_id_walk_pool_skip.rs` (7 tests, watched RED in both profiles
first). *(This line read "five panic sites plus two archetype resolvers" — 5 + 2 = 7, presented
as SIX — until the EG2 adversarial pass counted the diff. The number was echoed from
`f7c46c76`'s own commit message, which is pushed history and is left as written.)*

| guard | function | kind | `.expect`s guarded |
|---|---|---|---|
| `migration_helpers.rs:546~` | `migrate_entity_insert` | panic site | 2 |
| `:1070~` | `migrate_entity_remove` | panic site | 1 |
| `:1291~` | `merged_archetype_id_dyn` | **resolver** | — |
| `:1373~` | `without_ids_archetype_id` | **resolver** | — |
| `:1527~` | `migrate_entity_attach_ids` | panic site | 2 |
| `:1818~` | `migrate_entity_detach_ids` | panic site | 1 |

*Measured, not echoed:* `git show f7c46c76 -- …/migration_helpers.rs | grep -c "^+.*is_signature_id"`
→ **10** added lines, of which **6** are guard `if`s and 4 are comment lines; each guard's enclosing
`fn` and each protected `.expect` was then read at HEAD. 4 + 2 = 6.

**The ballot was incomplete when the earlier approval was given, and that is recorded here rather
than quietly fixed.** The plan claimed option (b) *"is precisely what a verbatim copy silently
produces"*. False: (b) requires deliberately ADDING a filter, and a verbatim copy adds nothing. The
default is an unenumerated **(d) — both broken**, which was never on the list. The owner was shown
(d) before answering.

### Re-measured at HEAD, because the original entry's probe had been deleted

Three probes, written from the kernel source rather than from either document, run and deleted:

| probe | profile | result |
|---|---|---|
| table+dense entity → `add_tag` | debug / release | **exit 101** at `migration_helpers.rs:1831:18` |
| **table-only sibling** → `add_tag` | debug / release | **exit 101**, same site |
| `remove_tag` on a dense-retaining archetype | release | **exit 101** at `:2116:18` |

The table-only probe printed `same=true` for the two archetype ids and asserted `!dense_contains`
before panicking — **the wider victim class is real**: dedup keys on the filtered mask while the
retained list belongs to whichever spawn minted the archetype first. Release-present confirmed by
execution, not by reading an `.expect`.

### Two things that made (a) the answer

**S2 needs no copy at all.** It is the existing detach helper made `pub`, so the approved seam as
written would ship that helper's release panic under a new public name — and the §4 table the
approval entry points at says of it *"exists and is correct"*.

**All eleven of EG2's gates are blind to the filter's presence** — traced one by one: the dense
gates route around migration, the table gates use table-only fixtures in fresh worlds, and the
rejection gate refuses before the merge.

### The template already in the tree

`clone/materialize.rs:121` walks the same list and skips pool-less ids through
`is_signature_storage`, whose own doc calls itself *"the single shared predicate every
signature-exclude / pool-skip site routes through."* The two migration walks are the documented
outlier.

---

> **A note on placement, so it is not a silent choice.** The four entries below are appended at the
> tail rather than at the head, against this file's stated *"Newest first"* convention. Reason,
> measured: seven line anchors in two ungated documents (`docs/VB-SV0-DP6-DESIGN.md:504` and
> `docs/VG-R3-P2-CAPABILITY-SPLIT-PLAN.md` ×6) cite lines 196–527 of this file, and
> `tests/internal_docs_anchors.rs` does **not** gate `OPEN-QUESTIONS.md` — so a head insertion would
> silently misbind all seven with nothing to catch it. The two preceding `ANSWERED 2026-08-27`
> entries are already at the tail for the same reason, and the RU twin mirrors that layout. The
> convention deviation is real and is recorded here rather than fixed, because fixing it means
> re-deriving seven anchors in two documents that this rung does not otherwise touch.

---

## 2026-08-27 — `DenseStore::arch_presence` is not re-seeded when a TABLE migration moves an entity that already carries a dense component

**This is a pre-existing kernel defect. EG2 did not create it — EG2 added two more routes to it, and
is the first rung to write it down.**

**Mechanism, read rather than predicted.** `DenseStore::mark_arch_present` has **eight call lines
across seven files** — `clone/materialize.rs:895`, `commands/insert_command.rs:267`,
`commands/migration_helpers.rs:799` (inside `migrate_entity_insert`),
`commands/spawn_at_command.rs:275`, `commands/spawn_batch_command.rs:595`,
`ecs_master/component_api.rs:132`, and `serialize/load_writer.rs:716` and `:855`. Every one of them
fires only for a dense id that is **in the bundle / insert set being applied**. Not one fires for an
entity's *pre-existing* dense memberships when that entity changes archetype. Query candidate
archetypes are seeded from `arch_presence` (`iters/query/data/read.rs:271`,
`data/mut_.rs:411`, `data/write.rs:267`, `filter.rs:564`, `:1084`, `:1387`).

**What survives and what is lost.** `dense_contains` (`ecs_master/component_api.rs:49`) reads the
store's membership and still answers `true` — the entity really does still carry the component. What
is lost is the **candidate seed**: the new archetype is never marked present, so a `Query<&Dense>`
never visits it and the row is invisible. Membership is intact; visibility is not.

**Reachable routes.** `add_tag` and the typed `Commands::insert` were measured by the EG2 refuter.
S1 / S2's table arms are the *same* code path: `migrate_entity_attach_ids_with_bytes` and
`migrate_entity_detach_ids` call `mark_arch_present` **zero** times (measured: `grep -c` over both
function bodies returns 0). So the new by-id seam adds two more routes to a defect it did not create
— and unlike the typed routes, these are reachable from a scene loader.

**Gated, RED by design.** `crates/boyko_ecs/tests/seam_by_id.rs:1314`,
`g12c_a_dense_carrying_victim_stays_visible_to_a_dense_query_after_a_table_attach`, `#[ignore]`d with
a `deferred:` reason. Watched red in **both** profiles: `seen == 1` against a baseline of `2`, exit
**101** in debug and in release. Lift the `#[ignore]` when the kernel fix lands and it turns green on
its own.

**Not fixed here, by the owner's own condition** that EG2's kernel diff be exactly the four approved
items plus the D9 sibling. **The question for the owner is only *when*, not *whether*:** the fix is a
separate kernel change (re-seed `arch_presence` for retained dense ids on every migration that mints
or changes an archetype), and it is the kind of silent-wrong-answer defect this repo normally treats
as blocking.

⚠️ **A correction to a premise, measured.** `tests/ignore_reasons_census.rs` — which `CLAUDE.md`
describes as failing the build on a bare `#[ignore]` — **does not exist on this branch**: `tests/`
holds eleven entries (ten `.rs` files plus the `reflect_scan_support/` directory) and it is not among
them, and `git cat-file -e master:tests/ignore_reasons_census.rs` reports it ABSENT on `master` too
(`master` is an ancestor of HEAD, verified with `git merge-base --is-ancestor`). The `deferred:`
prefix above is written because it is forward-compatible with the documented vocabulary — but
**nothing enforces it here**, and nobody should certify that ignore as "mechanically classified".

⚠️ **Three numbers in this paragraph were inherited from the EG2 adversarial pass and all three were
wrong by the time they were re-measured here.** (i) The site count was **327**, not 326 — the
`g12c` ignore this rung just wrote is itself the 327th, so the figure moved *because of* the change
it was describing, and it moved again for the same reason: EG2-R round 3's `g17` takes it to
**328** (`grep -rn '#\[ignore' --include=*.rs crates/ | wc -l` → 328, re-measured at that round).
It is **404** on the tree today too (401 at `2431c570`; the A7 merge adds three) <!-- measure: tree-lines crates rs #[ignore = 404 -->, and that
half is no longer prose: the round stamp keeps the historical sentence true, and the marker keeps
the live one checkable — a paragraph whose whole subject is that a count rots should not carry one
that nothing re-takes.
(ii) `tests/` holds eleven *entries*, not eleven *files*. (iii) Most importantly:
the pass claimed *"exactly one carries a class-shaped reason today (`miri_fixed_loop.rs:72`)"* — and
**that site is not class-shaped at all**. Its reason reads
`"windows-gnu Miri executor wall-time (bugfix_56 class); driver logic covered by M-P20-1"`: free
prose containing the English word "class", not a `<class>:` prefix from the closed vocabulary.
A `grep` for `#[ignore = "<lowercase-prefix>:` over `crates/` returns **three** sites, and none of
them is that one: `crates/boyko_ecs/tests/seam_by_id.rs:1308` (`deferred:` — written by this rung),
`crates/boyko_ecs/tests/seam_by_id.rs:2217` (`deferred:` — `g17`, written by EG2-R round 3) and
`crates/reflect_fixture/tests/reflect_absence_census.rs:983` (`calibration:`, a word the documented
vocabulary does not contain). **So the true count of vocabulary-conforming reasons in the tree before
this rung was ZERO**, which strengthens the point rather than weakening it.

## 2026-08-27 — VALUES: what does `#[require(C)]` MEAN when `C` is a dense component? A second public API now reaches the gap

`merged_archetype_id`'s required-expansion loop (`commands/migration_helpers.rs:426-437`) pushes
required ids **unfiltered**, while dense-filtering `bundle_ids` immediately above it (`:385-390`).
`merged_archetype_id_dyn` behaves identically — it filters only its *source seed* (`:1291`), never
its `extra` list. So a dense id named in `#[require(..)]` lands in the archetype **signature** and
mints a per-archetype pool it should not have: a phantom dense column.

`f7c46c76` filed this deliberately rather than deciding it — its own commit message calls it *"the
required-constructor pass, where filtering would turn a loud panic into a silently missing component
and the real gap is a missing feature."* EG2's FORK A preserves the behaviour **exactly**, which is
the correct call for a rung whose job is parity. But the by-id seam makes the gap reachable from a
**scene loader** rather than only from a typed `insert`, and `add_component_by_id`'s
`debug_assert!(added.iter().all(|&cid| target.has_component_id(cid)))`
(`migration_helpers.rs:2499-2502`, the *filtered* signature mask) means a dense required id now trips
a **loud debug failure** sitting exactly on the filed gap.

**That loudness is deliberate and is the honest behaviour** — it is the sweep's own stated preference
over *"a silently missing component"*. It is recorded here so it is not mistaken for a regression when
someone first writes `#[require(SomeDenseComponent)]`.

**This is a semantics question, not an implementer's question**, which is why it is here and not
decided. The options as they stand: **(a)** dense ids are legal in `#[require]` and the expansion
must filter them out of the signature while still constructing the dense datum; **(b)** dense ids are
illegal in `#[require]` and the derive rejects them at compile time; **(c)** status quo — the phantom
column stays, documented. Nothing in this rung depends on the answer.

## 2026-08-27 — SCOPE: EG2's approved diff GREW past its own inverted invariant, and the owner should hear it here rather than discover it at review

`docs/REFLECTION-PLAN-ECS.md` states as an **approval condition** that
`commands/migration_helpers.rs` receives *"exactly ONE additive hunk,
`pub(crate) fn migrate_entity_attach_ids_with_bytes`"*. That sentence is a scope description, not a
correctness one, and it no longer holds byte-for-byte:

1. **One parameter type.** Restoring `#[require]` parity through the by-id seam (BLOCKING — without
   it, `add_component_by_id` silently drops required components that the typed path constructs)
   needed the new fn's `bytes: &[&[u8]]` to become `srcs: &[AttachSrc<'_>]`, with
   `pub(crate) enum AttachSrc<'a> { Bytes(&'a [u8]), Ctor(RequiredCtor) }` declared beside it
   (`migration_helpers.rs:2078`, param at `:2425`). It mirrors `CloneColumnSrc` in
   `clone/materialize.rs` verbatim — this codebase's established shape for "bytes vs reconstruct".
   The hunk is still **one** hunk and still additive; `merged_archetype_id_dyn` is touched **zero**
   times, which was a design goal, not a coincidence.
2. **One test file the invariant does not mention at all** — `crates/boyko_ecs/tests/miri_phase22.rs`
   joins the diff for a one-line repair (see the next entry).
3. **A SIXTH kernel item, added by EG2-R round 3 and FLAGGED rather than absorbed.** FORK A needed
   `dense_insert_and_fire` split into `dense_insert_only` (`ecs_master/component_api.rs:122`) and
   `dense_fire_add_insert` (`:142`), so `component_api.rs` — a file the approved diff names nowhere —
   joins it. It is a **pure extract-method**: the fused helper stays, both pre-existing callers keep
   calling it and observe nothing. It is also not optional, and that was MEASURED rather than argued:
   exact parity with the typed path needs the dense STORE WRITE and the dense FIRES on **opposite
   sides** of the table migration, and both mirror-image orderings of the fused helper were run and
   both red gate 13b — attach-first fails VISIBILITY (`left: 0  right: 1`), dense-first fails ORDER
   (`left: false  right: true`). The zero-scope-delta alternative was to inline those eight lines
   into `seam_by_id.rs`: it needs no new item and leaves a **second copy** of the store-write plus
   `NonNull::from(&mut *self)` mint discipline — the diverging-duplicate shape this campaign has
   already been burned by twice (the doubled owner channel, the diverged EN/RU twins). The sixth item
   was taken; the delta is raised here.

**Measured diff growth:** 17 → **18** tracked files. Insertions: **979** at the approved landing →
**1049** after the code remediation → **1456** once these documentation acts are folded in
(deletions 170 → 174). That last step is documentation only — `docs/` accounts for **567** of the
final insertions across four files, of which this entry is part.

⚠️ **A number that describes the diff it lives inside is self-referential, and it drifted once
already** — it read `1065` between the code pass and the doc pass, and was corrected rather than
left. Re-take it with `git diff HEAD --stat` rather than quoting it; it is recorded here as measured
at the END of the documentation pass.

The two untracked landing files also grew — `seam_by_id.rs` src 367 → **533** lines, its test
1277 → **1742** — which `git diff` cannot show at all, because untracked files carry no baseline.

**EG2-R round 3 moved every number above again, and the shape of the growth changed.** Measured with
`git diff HEAD --stat | tail -1` at the end of that round's documentation pass:
**23 files changed, 1939 insertions(+), 229 deletions(-)**; `docs/` alone is
**8 files changed, 976 insertions(+), 86 deletions(-)**. The untracked pair was, at the end of
round 3, src **869** and test **2283**. ⚠️ **Read "was", not "is": at round 5 the pair WAS src
**903** and test **2572** (`wc -l`, and round 5 stopped editing both before taking it).** ⚠️ **That
sentence wrote "is" while teaching "was", and four rounds later the test half is out by **502**
lines** — at the state this landing ships the pair is src **903** and test **3283**, and
the second half moved three times more after round 5 (a gate at round 8, a record pass at round 9,
gate 19 at round 12). It is written here as a ROUND-STAMPED reading, and the live pair is
re-derived from the tree by a named test in the census file — the sentence you are reading is not
what that test reads, and the paragraph below (5) says which copies are and are not. The diff
totals are deliberately NOT restated here: a `git diff HEAD --stat | tail -1` figure describes the
edit set the sentence is part of, so writing it changes it — round 5 measured it, then made four
more edits, and the number it had just written was already eight insertions short. **Run the
command; a self-referential total cannot be maintained, only re-taken.** What is worth reading off that, rather than the totals: **four documents joined the
diff purely to repair anchors** — `docs/FEATURE_MAP.md`, `docs/REFLECTION-ANALYSIS.md`,
`docs/SYSTEMS.md` and `docs/REFLECTION-PLAN-CORE.md` — none of which any part of this rung meant to
touch. They are collateral of item 3 above, and they are the concrete price of splitting a file five
documents cite by line. The fourth is the sharpest: `REFLECTION-PLAN-CORE.md` was dragged in by a
*doc-to-doc* anchor into `REFLECTION-PLAN-ECS.md` that this round's own edits pushed onto a blank
line — see hole (4) below — and its repair was deliberately written **line-count-neutral** (10
insertions, 10 deletions, 4026 lines before and after) because that file carries **eleven**
self-anchor citations, **nine** of which one added line near its top would have moved — and the
prose version of this repair did exactly that, of which **eight would have rotted in silence**
(measured: only the one that happened to land on a blank line redded).

⚠️ **That `eleven` was true at `f7c46c76`, was `12` when this entry was written, and the count has
rotted AGAIN since — twice inside the edit set it describes.** Measured, unpiped:
`grep -c "REFLECTION-PLAN-CORE\.md:[0-9]" docs/REFLECTION-PLAN-CORE.md` → **11** against
`f7c46c76`, **14** now <!-- measure: lines-in-digit docs/REFLECTION-PLAN-CORE.md REFLECTION-PLAN-CORE.md: = 14 -->. The twelfth was the round's own repair — a second referent
given its own anchor rather than the first being repointed, which was the right call and which
nobody counted afterwards; the thirteenth and fourteenth arrived the same way, one round later.
**A count of the edit set it lives inside drifts inside that edit set, and it does so EVERY round.**
The figure against `f7c46c76` is stamped and stays true. The live one is no longer trusted to prose:
it carries a `measure:` marker (form specified above `MEASURE_MARKER` in
`tests/internal_docs_anchors.rs`; **do not spell one out in prose — an illustration parses as a
real marker**, measured), and `documents_that_count_the_tree_are_re_measured` re-derives it on
every run, so the next round that moves it is told rather than asked to remember.

⚠️ **The anchor bill, stated so it is not paid twice by accident: 37 citations.**
`tests/internal_docs_anchors.rs` reported **31** stale anchors after the split, all into
`component_api.rs`, across five documents (`FEATURE_MAP.md` 4, `REFLECTION-ANALYSIS.md` 5,
`REFLECTION-PLAN-BOUNDARY.md` 4, `REFLECTION-PLAN-ECS.md` 14, `SYSTEMS.md` 4) — plus **three more
anchors in `REFLECTION-PLAN-ECS.md`, cited at six places** (pre-repair `component_api.rs:208~`, `component_api.rs:392~` and
`component_api.rs:765-768~`, each twice), which are `~`-waived and therefore rotted in **silence**: the recorded "a waiver licenses
silent rot" hazard firing on schedule. The transform was one constant (every anchor at or past the
old line 121 gained 30; lines 49, 76 and 94 did not move; the old line 105,
`store.mark_arch_present`, moved into `dense_insert_only`), all **37** were repaired in the same
round, each verified against its target line's CONTENT and at BOTH ends of every range, and the
census is green.

⚠️ **NAME THE POPULATION, because there are two and they were read as refuting each other.** Every
figure in the paragraph above counts **each `:N` token**, bare continuations included — a line that
names the file once and then writes two bare continuations after it contributes **three**, not one.
*(No example is spelled out here on purpose: a specimen citation written into prose is
indistinguishable from a real one to a line scanner, and this document is the one no census reads.)*
`seam_by_id.rs`'s module
header counts a **different** population: a PREFIXED `component_api.rs:N` token plus the `(N)`
table form, which is why it says **40** citations at **19** distinct anchors where this paragraph
says 37 repaired. Round 4 struck *"31"* and *"37"* from the header as having *"matched no countable
population"*. **That strike was wrong and is withdrawn.** Re-measured at round 5 against
`f7c46c76`, counting all `:N` tokens and keeping only those the FORK A transform moved: four of the
five per-document figures reproduce **exactly** — `FEATURE_MAP.md` 4 of 4, `SYSTEMS.md` 4 of 4,
`REFLECTION-ANALYSIS.md` **5** of 9, `REFLECTION-PLAN-BOUNDARY.md` **4** of 5. The fifth,
`REFLECTION-PLAN-ECS.md` 14, is what the census **reported** rather than what moved (22 of that
file's 29 tokens moved, six of them `~`-waived and therefore silent), and it was NOT re-derived at
round 5 — so the totals 31 and 37 rest on it and are recorded, not re-proved. What IS settled is
that these figures count a real population and that the header counts a different one.

⚠️ **The *"cited at six places … each twice"* clause is exact, and the header's *"four, at three
anchors"* is exact, and they are the same three anchors under the two populations.** Measured:

```
git show f7c46c76:docs/REFLECTION-PLAN-ECS.md | grep -n -e '208~' -e '392~' -e '683-686~'
```

prints **four** lines — 1850, 1852, 1897 and 2288 — and the last of them cites all three anchors on
one line, two of the three in bare-continuation form. Six citations, each anchor exactly twice, of
which four are prefixed. The round 4 correction that wrote *"measured: four, at three anchors …
NOT six"* into `REFLECTION-PLAN-ECS.md` had counted only the prefixed form and missed line 2288;
that *"NOT six"* is struck at its own site.

⚠️ **FOUR holes in that census, and the third is the one that matters most.** (1) It validates a
range by its **START** and never by its END — which is how `ci.yml:566-573` survived with a line 313
that does not exist. (2) `is_file_shaped` (`tests/internal_docs_anchors.rs:834`) accepts any path
that merely has an extension, so the class reaches **workflow files**, where the anchor names a
*block* rather than an item and a one-line drift is invisible. (3) **`GATED_DOCS` is a hard-coded
list of NINE documents (`tests/internal_docs_anchors.rs:349`) and `docs/OPEN-QUESTIONS.md` is not one
of them** — nor is `docs/ru/OPEN-QUESTIONS.md`. Both carry `.rs:N` anchors, so the owner-facing
document this campaign files everything into has **zero** anchor coverage. Measured consequence in
this very round: **FIVE** anchors into `crates/boyko_ecs/tests/seam_by_id.rs` (`:895`, `:942`,
`:1308`, `:1314`, `:2217`) are cited **TWELVE** times across the two twins, and **no gate can see
any of them** — they are checked by hand. ⚠️ **This read "four anchors … eight citations" until
EG2-R round 4 recounted it: a number written about one's own edit is not exempt from measurement.**
Whether `GATED_DOCS` grows is the anchor census's owner's call; it is recorded here so the decision
gets made rather than defaulted.

⚠️ **Hole (3) is not hypothetical, and round 5 paid it in this very section.** FOUR of the anchors
in these two paragraphs pointed into `tests/internal_docs_anchors.rs` and had all gone stale
unseen: `is_file_shaped` (written `:790`, actually `:798`),
`doc_to_doc_anchors_carry_the_text_they_quote` (`:2724` → `:2890`),
`doc_to_doc_anchors_without_a_quotation_are_pinned` (`:2819` → `:3001`) and `KNOWN_STALE`
(`:2757` → `:2922`). ⚠️ **Both sides of those four pairs are ROUND-5 coordinates, kept as history:
the instrument has grown twice since, and all four have moved AGAIN — to `:834`, `:3131`, `:3247`
and `:3170` respectively, measured 2026-08-28, which is what the live citations above now carry.
A "→ where it really is" record rots exactly like the anchor it records.**
The first was wrong the day it was written; the other three were made wrong by
the landing that wrote them, in the same file that landing was extending. They are repaired above,
by hand, because no gate reads this document. **The contrast is the argument.** Two citations of the
SAME class were live in gated documents — `REFLECTION-PLAN-ECS.md` and `REFLECTION-PLAN-BOUNDARY.md`
each cite `PLANNED_EXACT` for its own row — and both had been repaired onto a line that holds a test
body rather than the constant. The census named them by file and line on the next run, with the
target line quoted back. Inside its reach the defect is a build failure; outside it, four instances
sat in the owner's own document across two rounds.

(4) ~~**A doc-to-doc `.md:N` anchor is checked only for
NON-BLANKNESS** — `looks_like_definition` returns `true` for every non-`.rs` extension after the
empty-line test — so such an anchor can be wrong from birth and stay green forever.~~ **CLOSED at
EG2-R round 4:** `doc_to_doc_anchors_carry_the_text_they_quote`
(`tests/internal_docs_anchors.rs:3250`) now requires the words a citation quotes to BEGIN inside the
lines it names, and `doc_to_doc_anchors_without_a_quotation_are_pinned` (`:3257`) pins the
population it cannot reach. It found **five** further stale doc-to-doc anchors on its first run; all
five are now repaired and `KNOWN_STALE` (`:3180`) is empty. **How the hole was found is the part
worth keeping.** `REFLECTION-PLAN-CORE.md` cited `REFLECTION-PLAN-ECS.md:1567` three times for an
EG3 item, and that citation went red **only** because a round's edits pushed 1514 onto a blank line
— bounds fired, not content. ⚠️ **The repair then landed on `REFLECTION-PLAN-ECS.md:1851`, a plausible sibling row of the
same table, and stayed green. The words are at `REFLECTION-PLAN-ECS.md:1853`, where all three now point.** The sentence
that used to close this paragraph — *"all three were corrected, verified by content"* — was untrue
when it was written, and that is exactly why the content check now exists.

⚠️ **The content check reaches a MINORITY of the population, and until round 5 the denominator was
published nowhere.** `UNQUOTED_MAX` pins only the *un*-checkable count — at the live value with zero
headroom for every document, which is real discipline — and a ceiling of `30` reads as a small
residue when it is in fact the larger part. Round 5 made the gate print the split itself
(`DocScan::doc_to_doc`), so the table below is the gate's own stdout, re-read 2026-08-28:

```
cargo test -p boyko-engine --test internal_docs_anchors -- --nocapture doc_to_doc
```

| document | doc-to-doc anchors | content-checked | bounds-only |
|---|---|---|---|
| `MESHLET-VIRTUAL-GEOMETRY-PLAN.md` | 2 | 0 | 2 |
| `REFLECTION-ANALYSIS.md` | 1 | 0 | 1 |
| `REFLECTION-PLAN-BOUNDARY.md` | 5 | 4 | 1 |
| `REFLECTION-PLAN-CORE.md` | 46 | 16 | 30 |
| `REFLECTION-PLAN-ECS.md` | 12 | 3 | 9 |
| **total** | **66** | **23** | **43** |

The other four gated documents write no doc-to-doc anchor at all. **23 of 66 — about a third — and
even that overstates the reach**, because a range pins where the material BEGINS and never where it
ends, so a wide window constrains almost nothing. The widest doc-to-doc window in the gated corpus
is `REFLECTION-PLAN-GATES.md:1196-1451` — **106 lines** — cited three times from
`REFLECTION-PLAN-CORE.md` (its lines 889, 3639 and 3949); the next widest names **10** lines.
**Publish the denominator beside the cap, or the cap reads as coverage.**

**The work was not scaled down to preserve the sentence.** Flagging it is the point: an approval
condition that quietly stops being true is worth more as a raised hand than as a clean-looking diff.

## 2026-08-27 — CI's Miri sweep runs NINE packages with `--all-targets` and NO `--no-fail-fast`, which is how one missing struct field silenced eleven tests

`.github/workflows/ci.yml:566-572` is a single `run:` block:
`cargo +nightly miri test --all-targets` over `boyko-ecs`, `boyko-utils`, `boyko-threadpool`,
`boyko-serialize`, `boyko-math`, `boyko_sdf_math`, `boyko_image`, `boyko-reflect` and
`reflect-fixture`. **`--no-fail-fast` appears three times in `ci.yml` — at `:211`, `:214` and
`:246` — and never on this row.** A build failure in **one** target therefore means **no** target in
the invocation runs.

That is exactly the mechanism EG2 hit. `crates/boyko_ecs/tests/miri_phase22.rs` is `#![cfg(miri)]`,
so only Miri builds it; a `ComponentHooks` literal there was missing `on_despawn`, giving
`error[E0063]`, and the row died before printing a `running N` line at all. **Eleven tests had not
executed in a long time.** One inserted line (`on_despawn: None`) revives them and they pass
**11/11 under Tree Borrows** in 14.91 s.

**R5 fixed the symptom; the flag fixes the class**, and the next missing field will silence the next
eleven identically. Adding `--no-fail-fast` changes CI behaviour for all nine packages in that row,
so it belongs to whoever owns the workflow, not to this rung. **Recorded, not acted on.**

⚠️ **And a second, independent reason that row deserves a look.** The implementer measured that
`cargo miri test -p boyko-ecs --test seam_by_id` (the whole target) **does not complete** — it stalls
at `g7_mark_component_changed_*` (`crates/boyko_ecs/tests/seam_by_id.rs:895`, `:942`), both of which
build a real 2-thread work-stealing `ThreadPool` and run a `Schedule` (mechanism verified here by
reading; the stall itself is the implementer's measurement and was **not** re-run in this pass,
because reproducing it means waiting on a hang). g7 is an EG2-landing gate, not one of this
remediation's. The deferred `g12c` uses the same shape but is `#[ignore]`d, so it adds no Miri load.
This matters because the sweep above runs `--all-targets -p boyko-ecs` — the same row.

⚠️ **And the range in that first sentence was ungated by construction, which is how it rotted.** It
read `ci.yml:306-313` until EG2-R round 3; the block ends at `:572` and line 313 does not exist.
`tests/internal_docs_anchors.rs` accepts any path that merely has an extension
(`is_file_shaped`), and it validates a range **by its START** and never by its END — so `:344`
resolving to a real line was enough for both halves to pass. The class reaches **workflow files**,
where a one-line drift is invisible and where the anchor names a *block* rather than an item. The
correction to `306-312` is documentary: no gate confirms it. Filed with the rest of the anchor-census
holes in the SCOPE entry above.

## 2026-08-27 — Entity-targeted observers: the by-id ADD half now fires them, the REMOVE half does NOT, and the remove half is a PRE-EXISTING kernel gap shared with `add_tag` / `remove_tag`

**Measured, EG2-R round 3.** `observe_entity` on two victims for the same component id, then attach
it — typed `Commands::insert` on one, `add_component_by_id` on the other. Counters: `typed_add = 1`,
`by_id_add = 0`. The remove direction has the same shape: `typed_remove = 1`, `by_id_remove = 0`, and
the by-id call still returns `true`. The typed path fires them at
`commands/migration_helpers.rs:945-955` (Add / Insert) and `:1483-1484` (Replace / Remove), and
re-raises the DESTINATION archetype's sticky `HAS_ENTITY_OBSERVER` bit at `:1127` and `:1542`
**before** the flags are read for that entity's fires — raising it afterwards skips exactly the
migration that made it necessary.

**Only the ADD half was repaired, and the reason is the caller table, not the difficulty.** Each
repair is **two SITES and nine code lines** — one statement plus an eight-line `if
flags.contains(HAS_ENTITY_OBSERVER)` block with two `for` loops in it — ⚠️ **MEASURED at EG2-R
round 4; "two lines" was this table's own estimate of its own diff, and it understated the ADD
half by 4.5×.** What differs between the rows is not the size but how many pre-existing `pub` APIs
move with them.

| helper | def | callers outside `migration_helpers.rs` | disposition |
|---|---|---|---|
| `migrate_entity_attach_ids_with_bytes` | `migration_helpers.rs:2419` | **one module** — `ecs_master/seam_by_id.rs:320` and `:462` | **REPAIRED here.** It is this rung's own `pub(crate)` D9 sibling with no other consumer, so the change alters no pre-existing behaviour: `world.migrate_entity_observer_bit(entity)` at `migration_helpers.rs:2696` (site 1, one line), the `HAS_ENTITY_OBSERVER` block at `migration_helpers.rs:2748-2755` (site 2, eight lines; the two fires are `migration_helpers.rs:2750` and `migration_helpers.rs:2753`), `added` iterated unfiltered for both kinds. |
| `migrate_entity_detach_ids` | `migration_helpers.rs:2022` | **two** — `ecs_master/tag_api.rs:234` (`remove_tag`) and `seam_by_id.rs:591` | **DEFERRED.** Repairing it changes `remove_tag` too. Measured: `grep -c` for `fire_entity_observers` and `migrate_entity_observer_bit` over its body returns **0** for both. |
| `migrate_entity_attach_ids` | `migration_helpers.rs:1724` | **one** — `tag_api.rs:170` (`add_tag`) | **DEFERRED, and it is the SAME change.** Same zero count over its body. |

So **`add_tag` and `remove_tag` have never fired entity-targeted observers either.** That is the real
size of the gap and the reason the remove half is not a follow-up line to this rung: two `pub` tag
APIs change behaviour with it, on the kernel's schedule rather than on reflection's — the same
disposition `f7c46c76` got.

**The gate that goes green when that change lands** is
`g17_entity_targeted_remove_observers_fire_through_the_by_id_seam_as_through_the_typed_one`
(`crates/boyko_ecs/tests/seam_by_id.rs:2217`), `#[ignore]`d with a `deferred:` reason and RED by
design: run with `--ignored` it prints `left: 0  right: 1` on the Remove counter, exit **101**
(observed, not predicted). Lift the `#[ignore]` when the kernel change lands and it turns green on
its own. Its landed twin `g16_entity_targeted_add_observers_fire_through_the_by_id_seam_as_through_the_typed_one`
is green and was red — `left: 0  right: 1` — on the unfixed tree.

⚠️ **A THIRD gap sits underneath both, and it must NOT be "fixed" on one side.** Dense components
receive entity-targeted observers on **no** route at all. `migrate_entity_insert`'s POST-dense block
(`migration_helpers.rs:1288-1302`) fires exactly four things — `trigger_on_add`,
`fire_on_add_observers`, `trigger_on_insert`, `fire_on_insert_observers` — and so does
`EcsMaster::dense_fire_add_insert` (`ecs_master/component_api.rs:142-150`); neither calls
`fire_entity_observers`, and `dense_remove_and_fire` (`:160`) is the same. The by-id dense arm
therefore matches the typed dense arm **exactly**, which is why the code landing deliberately did not
add the call there: parity is the target, and a one-sided repair would destroy it. Whether entity
observers should cover dense components at all is a design question for the dense plan's owner — a
**path-symmetric gap, not a parity defect**, and it is filed here so the next reader does not
mistake it for one.

## 2026-08-28 — TWELVE `.rs` → `.rs` line citations are out of bounds, in the direction no census read at all until this round

**The blind spot was structural, not an oversight.** The forward census scans `.md` → `.rs`/`.md`.
The reverse census (`md_citations_in_rust_sources`, `tests/internal_docs_anchors.rs:4105`) scans
`.rs` → `.md` and **filters its targets to `.md` by construction**. A Rust source citing a line of
another Rust source fell between the two and was read by nothing.

`rs_line_citations_written_inside_rust_sources_are_bounds_checked`
(`tests/internal_docs_anchors.rs:4491`) now reads it — named-only, line-local. Its own stdout,
re-taken **2026-08-29**:

```
cargo test -p boyko-engine --test internal_docs_anchors -- --nocapture rs_line_citations
242 `.rs:N` citation(s) bound on their own line inside .rs sources (11 skipped as string-literal data)
174 anchor(s) named a fragment matching several files (cap 176)
5 anchor(s) named a fragment matching NO file in the tree (cap 5)
```

⚠️ **The first line read `240` until 2026-08-29, and NOTHING in the tree said otherwise.** The
figure counts every `.rs:N` citation written in any `.rs` source, so it moves whenever any source in
the tree gains one — and the landing that wrote this transcript went on to write two of them itself,
in its own new gate. Re-running the command it prints answers **242** and did so before this entry
was touched, which makes it the same shape as the four record coordinates in (3) above: a reading
taken mid-landing, left in the present tense, and overtaken by the rest of the same edit set. **It
is now stamped with the date it was taken rather than with "at this round"**, because a round is not
a coordinate and cannot be re-run.

⚠️ **The last two lines are new, and their absence was the defect.** When this entry was written the
scan printed the bound count and the 5 and said nothing about **179 anchors it dropped in silence**
— a bare `continue` with no push, no cap and no report — while the `.md` direction pinned the
identical class at zero in all nine documents *with the rationale that an anchor naming no single
file is not checked at all*. One direction called this its most important ledger; the other did not
have one. The asymmetry was invisible from either side, because a census that prints a numerator and
no denominator reads as a small residue rather than as a share: **179 of 426 bound anchors, 42 %,
were outside the check.**

The 179 split in two, and the split is the difference between lost coverage and a dead path.
**174** name a fragment matching SEVERAL files — mostly a bare `mod.rs`, ambiguous in this tree by
construction — and are a ceiling, not a pin at zero, because no repair can make them unique.
**5** name a fragment matching NOTHING, and those are named rather than counted:

| citing file | line | fragment | why it matches nothing |
|---|---|---|---|
| `crates/boyko_ecs/src/ecs/core/component/hooks/mod.rs` | 33 | `component_registry.rs:67` | the module was split into a directory; the citation kept the pre-split file name |
| `crates/boyko_app/tests/vg_occ_split_timing.rs` | 226 | `vb_bench_totality_gate.rs:90` | no file of that name exists |
| `crates/boyko_app/tests/vg_occ_split_timing.rs` | 1523 | `vb_bench_totality_gate.rs:90` | the same citation, written twice |
| `crates/aether_tests/tests/a7_dx.rs` | 9 | `a7_probe.rs:21` | no file of that name exists |
| `crates/boyko_scene/tests/gpu3d_pack_miri_witness.rs` | 61 | `boyko_render/gpu3d_instance.rs:77` | ⚠️ **a different animal** — the basename DOES exist, under `boyko_render/src/`; the fragment omits that segment, so it ends no path in the tree |

The fifth is called out because a ledger that flattened it into *"a file that is not in the tree"*
would have been wrong about the one member a reader is most likely to check. They are ENUMERATED,
not repaired, for the same reason `RS_KNOWN_STALE` is: repairing a name needs to know which file the
sentence meant, which is a question for each one's owner.

⚠️ **And the scan was dropping shapes before its own binder ran.** A raw `.rs`-followed-by-a-colon
substring prefilter sat in front of the very function repaired to read a name through its
delimiters — so a backtick or a bold marker between the name and the colon put no such substring on
the line and the citation was never read. MEASURED with a citation 99 437 lines past the end of a
562-line file: both delimited forms exited 0 with the count unmoved; the undelimited form redded.
Live population **0** — latent, which is why nothing in the corpus could have shown it, and why the
invariant *"every shape the binder will BIND, the prefilter must READ"* is now asserted directly.

**Twelve of the 242 are past the end of the file they name.** They are pinned in `RS_KNOWN_STALE`
(`tests/internal_docs_anchors.rs:4497`) **rather than repaired**, and the pin has both halves: a
listed entry that stops reporting also reds, so a silent repair cannot quietly empty the list.

| citing file | line | target as the census reports it | that file's length |
|---|---|---|---|
| `crates/boyko_ui/src/layout.rs` | 206 | `ecs_master.rs:2096` | 1918 <!-- doc-anchor-ignore --> |
| `crates/boyko_ui/src/layout.rs` | 1724 | `ecs_master.rs:2121` | 1918 <!-- doc-anchor-ignore --> |
| `crates/boyko_ui/src/text/lower.rs` | 5 | `boyko_macros/src/lib.rs:3875-3971` | 650 <!-- doc-anchor-ignore --> |
| `crates/boyko_ui/src/reload/reconcile.rs` | 21 | `ecs_master.rs:2456` | 1918 <!-- doc-anchor-ignore --> |
| `crates/boyko_ui/src/reload/reconcile.rs` | 26 | `hierarchy/commands.rs:268-276` | 166 <!-- doc-anchor-ignore --> |
| `crates/boyko_ui/src/reload/system.rs` | 140 | `ecs_master.rs:2456` | 1918 <!-- doc-anchor-ignore --> |
| `crates/boyko_ecs/tests/phase13_local_systemparam.rs` | 28 | `ecs_master.rs:2548` | 1918 <!-- doc-anchor-ignore --> |
| `crates/boyko_ecs/src/ecs/core/iters/query/chunked_data.rs` | 422 | `data.rs:1383-1401` | 980 <!-- doc-anchor-ignore --> |
| `crates/boyko_ecs/src/ecs/core/iters/query/chunked_data.rs` | 459 | `data.rs:1230-1363` | 980 <!-- doc-anchor-ignore --> |
| `crates/boyko_ecs/src/ecs/core/iters/query/chunked_data.rs` | 597 | `data.rs:1493-1501` | 980 <!-- doc-anchor-ignore --> |
| `crates/boyko_ecs/src/ecs/core/iters/query/query_view.rs` | 153 | `data.rs:1155` | 980 <!-- doc-anchor-ignore --> |
| `crates/boyko_ecs/src/ecs/core/commands/migration_helpers.rs` | 685 | `boyko_macros/src/lib.rs:1062` | 650 <!-- doc-anchor-ignore --> |

The rows are in the gate's own print order. All three target names — `ecs_master.rs`, `data.rs`,
`boyko_macros/src/lib.rs` — resolve **uniquely** in the tree, so not one of the twelve is an
ambiguity; every one is simply a number the file outgrew. Lengths are as the census counts them
(`str::lines()`); `wc -l` reports **1916** for `ecs_master.rs`, which ends without a final newline.

⚠️ **EXACTLY SIX of the twelve are in `crates/boyko_ui/`, which another lane is rewriting as this is
written.** They are enumerated here and deliberately **not touched** — whoever lands that rewrite
owns those six, and repairing them from outside it would collide.

The other six are `boyko_ecs`'s own, and the first of them —
`crates/boyko_ecs/src/ecs/core/commands/migration_helpers.rs:685` — sits in a file the EG2 landing
modified, though the cited line is outside the ranges that landing added. All six are left pinned
so that their repair is one commit with one subject, rather than twelve numbers changed inside a
landing about something else. **Enumerated, not fixed.**

## 2026-08-28 — The round that gated the corpus's counts wrote two ungated ones of its own, and all four record coordinates were stale one round later

`documents_that_count_the_tree_are_re_measured` exists for exactly one class: **a sentence that
states a number about the tree.** Every anchor gate above it is blind to that class by
construction — those check where a sentence POINTS, never what it SAYS. This entry audits the
landing that built it. The audit found the class **inside** that landing, twice, and both times in
a paragraph whose subject was the mechanism itself.

### (1) A pre-state left in the present tense, dated to the landing that ended it

`REFLECTION-PLAN-ECS.md`'s gate-naming paragraph read *"Taking each of the 25 names `--list` prints
and grepping this file for it verbatim, **2 of 25** appear — MEASURED 2026-08-28"* and then, one
sentence later, *"The three extra legs written into entries 1, 5 and 12 at this round take that to
5."* The legs were already in the file when that was written. Re-measured here, unpiped:

```
cargo test -p boyko-ecs --test seam_by_id -- --list        -> 25 tests, 0 benchmarks
grep -F <name> docs/REFLECTION-PLAN-ECS.md, once per name  -> 5 hits
```

**Five, not two**: `g1c_size_zero_table_add_is_first_class`,
`g5_dead_entity_is_rejected_with_its_own_reason`,
`g12c_a_dense_carrying_victim_stays_visible_to_a_dense_query_after_a_table_attach`,
`g13_require_holds_through_the_by_id_seam_exactly_as_through_the_typed_one` and
`g18_a_dense_caller_with_require_seeds_the_target_archetypes_presence_bit`.

**This is the exact shape of the three name sweeps that same landing struck** — a figure taken
before its own edit set had finished, left in the present tense, stamped with the date of the
landing that falsified it. That it turned up in the paragraph NAMING the failure mode rather than
in one suffering it is the finding worth keeping: **a paragraph explaining a gate is the likeliest
place in a document to state an ungated number, because writing about a mechanism reads like using
it.** Repaired at the site — the live figure is `5`, `2` is written as history, and the reason the
count cannot carry a marker is now enumerated (its subject is a `cargo test` process, not text in
the tree).

### (2) A claim about the instrument's own REACH, false of four of the six figures it covered

Same document, the ownership sweep: *"Re-measured 2026-08-28 and re-derived on every run since: the
three plan documents and CORE are unchanged, and both OPEN-QUESTIONS files match 29 lines each"* —
carrying markers for the two OPEN-QUESTIONS figures **only**. The other four sat inside the word
*"unchanged"*, and **a word is invisible to the proximity guard**: there is no digit for it to
compare against, so a figure folded into one is ungated no matter what stands beside it. The four
are now digits with markers of their own — `REFLECTION-PLAN-BOUNDARY.md` **9**,
`REFLECTION-PLAN-GATES.md` **3**, `REFLECTION-ANALYSIS.md` **2**, `REFLECTION-PLAN-CORE.md` **0** —
and the sentence is true as written for the first time. The census now re-derives **18** markers
across four documents, up from 11.

### (3) Four record coordinates, each verified by reading its target — and all four had moved

Verified against the tree as this entry is written. Each is quoted from the line it names — and one
of them was not, until 2026-08-28; see item 1.

1. **The `g12c` description opens at `REFLECTION-PLAN-ECS.md:1279`** — *"**The third `#[test]`:"*.
   It does NOT open at `REFLECTION-PLAN-ECS.md:1265`, which reads *"the guard at all; see below.** Both guard legs are
   modelled line-for-line on the kernel gate"*.
   ⚠️ **CORRECTED 2026-08-28 — this item failed the heading it sits under.** It attributed the words
   *"pure-table `{ Pos }` entity which dedups into the same archetype"* to `REFLECTION-PLAN-ECS.md:1255`. Those words read
   at **`REFLECTION-PLAN-ECS.md:1269`**, fourteen lines lower — and fourteen is this round's own
   insertion, the same `+14` that carried `1265` to `1279` and `1250` to `1264` in the ⚠️ note
   below. The quotation was taken before the insertion and left in the present tense: **a pre-state
   written as a live reading**, in the one paragraph of this entry that promises the opposite, and
   it survived a clean EN/RU parity check because **both twins carried it identically**. Re-read
   now, the two ends of the older range `1255-1271` land in different entries: `REFLECTION-PLAN-ECS.md:1255` reads
   *"emptied was measured green at exit **0**, `1 passed`, the four landed items compiled by"* —
   entry **11**, its trybuild-corpus leg — and `REFLECTION-PLAN-ECS.md:1271` reads *"bystander); attach a table data id by
   id and assert `Attached`, the payload reads back, `Pos` keeps"*, which IS entry 12's dense leg.
   The range no longer begins where this item said it begins.
2. **"Two legs on the guard" is at `REFLECTION-PLAN-ECS.md:1264`** — *"guard.** **Two legs on the
   guard — and a THIRD `#[test]` under this entry whose subject is not"*. The older range
   `1239-1241` is inside entry 11 and says nothing about legs on the guard — but it is **not** in
   that entry's trybuild-corpus leg, as this item claimed until 2026-08-28. `REFLECTION-PLAN-ECS.md:1239` opens the bullet
   that takes `PLANNED_EXACT`'s `REFLECTION-PLAN-ECS.md` row to **0**, and `REFLECTION-PLAN-ECS.md:1241` reads *"is not
   self-checking (D25), so it is stated here as an act, not as a consequence."* Entry 11's
   trybuild-corpus leg is its `(ii)` bullet, which opens at `REFLECTION-PLAN-ECS.md:1257` —
   *"(ii) `tests/trybuild_corpus_compiler_witness.rs` needs no teaching for the glob, but its scan"*.
3. **`doc_misbound`'s push is at `tests/internal_docs_anchors.rs:1880`** — `doc_misbound.push(format!(`.
   Not the range `1638-1676`, which is the surrounding branch and its comment.
4. **The self-citation guard is at `tests/internal_docs_anchors.rs:1870`** — `&& self_path != target`.

⚠️ **All four numbers above differ from the ones measured one round earlier (1408, 1250, 1691,
1681), and every one of those was correct when it was taken.** This round inserted lines above each.
Had any of the four been written as a citation inside a GATED document, the census would have said
so — it said exactly that about **12** citations this round's line moves rotted (13 stale-definition
anchors and 5 doc-to-doc quotations, across `REFLECTION-PLAN-BOUNDARY.md`, `REFLECTION-PLAN-ECS.md`
and `REFLECTION-PLAN-CORE.md`), and every one was repaired against the census's own report. These
four were recorded only in a round's report, where nothing reads them; recording them here puts
them in a file no check reads either — the same blind spot that rotted **8** more citations of the
census's own source, four per twin, found only because this round went looking by hand.
**A coordinate written outside a gated document has a shelf life of one landing.** They are given
with the content beside each so the next reader re-derives rather than trusts.

The exposure is bigger than those four per twin. Each twin carries **8** by-line citations of the
census's own source in all — `grep -c "internal_docs_anchors[.]rs:[0-9]" docs/OPEN-QUESTIONS.md`,
and the same over the `docs/ru/` twin, both **8**; **16** between them.

⚠️ **CORRECTED 2026-08-29 — the sentence that stood here claimed the bracket classes were
load-bearing and that "the unescaped form answers **9** over its own text". Both halves are false,
and the second was a live figure that NO form of the command produces.** Re-measured over the
shipped twins, three forms each, six counts: the command exactly as printed → **8**; the same
command with its bracketed dot written as a plain dot → **8**; a fixed-string search for the name
and colon with the trailing digit class dropped → **8**. The unescaped regex cannot answer **9**
even in principle. The only line it could add is the one quoting it, and what follows the colon
*there* is the opening bracket of a character class — never a digit. The brackets buy nothing for
this particular command, and saying they did was a plausible mechanism attached to the wrong token.

**The 9 is real; it belongs to a THIRD form neither sentence named.** Reconstructed by copying each
twin, rewriting its quoted command unescaped, and counting again: the unescaped *regex* still
answers **8**, and only the *fixed-string* search — the one with the trailing digit class dropped —
answers **9**, on both twins. **A quoted command perturbs its own answer only when its pattern is a
PREFIX of its own text; one that keeps going past the point where its own text stops resembling it
cannot.** The escape convention stays, because a reader cannot see which case they are in by
looking; but it is the prefix property that decides, not the brackets, and the difference is the
whole reason the wrong half of it went unchallenged for a round.

Tree-wide the file is cited by line
**29** times at **12** distinct numbers, from six documents and from itself; **four** of the twelve
are cited from a `GATED_DOCS` document and **one** is the file quoting itself, so five of the twelve
RED when they move and the other **seven** live only in these two files. All 16 were re-resolved by
hand at this revision and all 16 land on the line they name. The census file now says so at its own
site, where the claim it replaces — *"twelve … six per twin"* — had itself gone stale.

### (4) `doc_misbound` exempts SELF-citations by design — stated non-coverage, now written at the site

The misbinding check asks the CITING document whether it carries the quoted words at the same
coordinates, which is the one piece of evidence separating *"un-checkable"* from *"bound to the
wrong file"*. It is guarded by `self_path != target`, so **a document citing its own lines is
outside it** — correctly, because a self-citation has no second document to have been misbound to,
and without the guard every self-citation whose quotation merely wraps past the cited length would
be reported as a misbinding it cannot be.

The corpus holds two, both pointing at C11's weaker-subject rule from inside the file that states
it: `REFLECTION-PLAN-CORE.md:512` and `REFLECTION-PLAN-CORE.md:2575`, each citing
`REFLECTION-PLAN-CORE.md:3835-3837` (*"Run this mutation in the fixture"* … *"a weaker subject
wearing the same verdict"* — both ends read). Both resolve correctly. **Neither reaches the branch
today for a SECOND and independent reason**: neither quotes anything, so both land in the
bounds-only ledger — MEASURED by dropping that ledger's cap for CORE to 29 and reading the two
entries out of the red, then restoring the cap and proving the file byte-identical. Two reasons
pointing the same way is the shape that hides a gate which cannot fail, so both are now written at
the guard rather than left to be re-derived.

### (11) Every measurable claim this rung wrote, with its disposition

**GATED — re-derived on every run (24 markers):**

| figure | where | value |
|---|---|---|
| the rung token's lines in `REFLECTION-PLAN-BOUNDARY.md` / `-GATES.md` / `REFLECTION-ANALYSIS.md` / `-CORE.md` | ECS ownership sweep | 9 / 3 / 2 / 0 |
| the same token's lines in both `OPEN-QUESTIONS.md` twins | ECS ownership sweep | 29 / 29 |
| `AddOutcome` / `RejectReason` / `migrate_entity_attach_ids_with_bytes` under `crates/` | ECS, the struck name sweep | **43** / 18 / **16**. ⚠️ **TWO of these three moved inside this one landing, and each was caught by its own marker rather than by a reader.** The first read **42** until the kernel gate written to close the presence-seed class was written — that gate uses the identifier. The third read **15** until the record pass named the fifth migration site in that gate's header, which is a `.rs` line under `crates/` and therefore inside the marker's population. Both times the marker RED-ed; both times both twins carried the stale copy in the meantime, **because the copy is not what the marker reads.** A tree-wide count of an identifier the campaign's own tests and headers use is a figure the campaign moves by working, and the only reason these two are correct here is that something failed the build until they were. |
| `AddOutcome::Added` under `crates/` | ECS, the S1 return-type row | 0 |
| `.dense_contains(` under `crates/` | ECS | 54 |
| `residency = "gpu"` under `crates/` | ECS F22 | 0 |
| `positional` in `REFLECTION-PLAN-BOUNDARY.md` | CORE, the paid C7 debt | 0 |
| `hwrt` in `.github/workflows/ci.yml` | CORE F17 | 0 |
| CORE's own self-anchor citations | both twins | 14 |
| `#[ignore` sites under `crates/` — the LIVE half | both twins | 328 |

**UNGATED, with the reason for each. None of these is an oversight; the vocabulary is closed on
purpose and widening it is a code change with a review.**

| figure | where | why it carries no marker |
|---|---|---|
| every `git show f7c46c76 … \| grep -c` figure | both twins | a frozen object cannot rot; **the stamp is the gate** |
| `g18`-or-`gate 18` lines in `docs/` — **2** at round 5 | ECS | alternation, and a marker carries one literal. ⚠️ The sentence reasons from the 2 in the present tense; the stamp keeps it true and it is named here so the next reader does not read it as current. **CORRECTED 2026-08-28: the claim's own printed command, `grep -rn "g18\|gate 18" docs/`, returned 13 that day** — 9 in `REFLECTION-PLAN-ECS.md`, 2 in each twin. The **9** this row carried one round earlier is the ECS-only count, a NARROWER population than the command the claim prints: the correction repeated, one row down, the mismatch it was correcting. And 2 of the 4 twin lines ARE this row — the correction is part of what it counts, which is why nothing here can be gated. ⚠️ **RE-TAKEN TWICE on 2026-08-29, and only the second re-taking stands: 22** — 10 in `REFLECTION-PLAN-ECS.md` and **6** in each twin, taken after the last edit of this landing rather than before the first. The intermediate reading of **16** was true for the hours between the two passes and false by the time it shipped: the record pass then wrote three more matching lines per twin while documenting the very gates that had moved it. **Of this twin's six, TWO are the act of counting** — this row, and item 9 of the disclosure list in (7) — **and four are substantive references to the three kernel gates.** That ratio is better than the round before and is still not one a reader should treat as current. **A self-referential alternation is re-taken with a date at every writing; it is not maintained, and this row has now proved that three times rather than argued it once.** |
| `#[ignore = "<lowercase-prefix>:` sites | both twins | needs a character class; **3** today, re-measured |
| `wc -l` — 903, **3283**, 1916, 4026 | both twins | ⚠️ **UNGATED BY A MARKER, GATED BY A TEST, AND THESE COPIES ARE NEITHER.** A file's LENGTH is inexpressible in the marker vocabulary: every kind counts lines CONTAINING a token, and the empty token that would count them all cannot be written. Since 2026-08-29 all four are re-derived from the tree on every run by a named test in the census file — **but that test reads the census file's OWN doc comment, never this row.** The second figure read **2572** here while the tree said 3054, then 3054 while the tree said 3074, then 3074 while the tree said 3283; ALL THREE times the landing that wrote the number was the landing that moved the file, which is now a pattern rather than an accident. The digits above are re-taken at the state this landing ships and are a DATED reading, not a checked one |
| `mark_arch_present` called **0** times over two function bodies | both twins | body-scoped; the unit here is a whole file or a whole directory |
| *"ten gates"* ×2 and *"six runnable reds"* ×1 inside D26 | ECS | section-scoped AND self-referential — a file-wide `grep` returns **3** and **2**, because the sentence counting them writes them |
| **5 of 25** harness names appearing verbatim | ECS | needs a `cargo test` process; the subject is `--list` output joined against a document |
| **six** defined symbols under `llvm-nm --defined-only` | ECS | needs a release build and a symbol table |
| 242 / 174 / 5 / 179 / 12 — the census's own output | both twins | gated where PRODUCED, by the caps and by `RS_KNOWN_STALE`; the documents quote a fenced transcript of a named run. ⚠️ Residual: `UNBINDABLE_RS_MAX` is **176** over a live **174**, so the cap gates the ledger and not the document's copy of the number. ⚠️ **And "gated where produced" gates the LEDGER, never the twins' COPY**: the first figure stood at **240** for a round while the shipped scan printed **242**, because the caps this row points at bound the other four and leave the bound count with no ceiling at all — a count that only grows cannot have one. The transcript is therefore dated, not stamped to a commit, and re-taking it is a manual act |
| *"a `grep` for a derive `attributes` list returns zero hits"* → **7**; *"`assert_bss_eligible` has zero hits in `crates/`"* → **2** | CORE D14, twin G22b | **SELF-REFERENTIAL and ungatable by any vocabulary**: a sentence asserting a token is absent must write the token to say so. Both re-measured; both are count errors created by writing the count, not engineering errors, and rewriting another round's ruling is not this rung's call |

### (6) The rung's stated limit — what the instrument measures, and what is outside it

The instrument is one target — `cargo test -p boyko-engine --test internal_docs_anchors`, **23
tests**, exit 0 at this revision. ⚠️ **That read *"21 tests"* until 2026-08-29 and was two behind:
the round that closed the empty-cap and stale-figure items added a test for each, and a count of the
tests in a target is exactly the figure a paragraph about that target forgets to re-take.** This
section is its REACH, written down so the next reader does not
have to infer it from a green run. **The debt below is enumerated, not fixed.** Each item names the
population it is a limit over, and every figure was taken at this revision by the command beside it.

**What it does check.**

| what | population | live at this revision |
|---|---|---|
| line anchors, EVERY target | the gated list — **9** of the **340** tracked `.md` under `docs/` | **1561** checked, 0 stale — **1495** of them into source files, **66** into other documents |
| path mentions | the same 9 | **1684** checked, 0 dead |
| document-to-document quotations | the same 9 | the **66** above — **23** content-checked, **43** bounds-only |
| bare continuations inheriting a source file across a line boundary | the same 9 | **377** |
| `.rs:N` citations written inside `.rs` sources | every file the repo walk sees | **242** bound on their own line, **5** skipped as string-literal data, **174** naming a fragment that matches several files, **5** naming a fragment that matches none |
| `.md:N` citations written inside `.rs` sources | every file the repo walk sees | **49** bound and bounds-checked, **8** naming no document on their own line (cap **8**) |
| figures re-derived from the tree | every `.md` under `docs/`, both twins included — **340** files | **18** markers in **4** documents |

⚠️ **The first row read *"line anchors into source files … 1561 anchors"* until 2026-08-29, and that
was a TOTAL presented as a PART.** The scanner counts an anchor once and then increments the
doc-to-doc tally as a SUBSET of it, so the **66** in the third row are **inside** the 1561, not
beside it — the two rows added up to 1627 anchors that do not exist. The source-file figure is
**1495**. The row is now labelled by what it counts, because the defect was not arithmetic: **1561
was the right number under the wrong noun**, and a reader summing a table has no way to see that two
rows overlap. Re-derived from the run's own per-document lines, which sum to 1561 and 66
respectively; **1561 − 66 = 1495**.

⚠️ **Two rows of that table moved WHILE it was being corrected, and both moves were caused by the
correction.** Path mentions read **1683** one revision ago and read **1684** now, because the
reconciliation written into `REFLECTION-PLAN-ECS.md` in this same edit set names one more path; the
`.rs:N` row moved for the same kind of reason, one landing earlier. **A table headed *"live at this
revision"* is a table whose own editor is a contributor to it**, and every figure in it was re-taken
after the last edit rather than before the first — which is the only order that produces a true row.

Two numbers in that table are the whole shape of the limit: **9 of 340** and **4 of 340**. Nine
documents have their citations checked; four have their arithmetic checked. Everything else under
`docs/` — this file included — is prose that no check in this tree reads.

**S5 — a measurement marker outside `docs/` is INERT, and nothing says so where one would be
written.** The marker scan admits `.md` files under `docs/` and stops there.
`git ls-files '*.md' | grep -cv '^docs/'` counts **71** tracked documents outside it, in four
directories plus one file at the repo root — `book/` **54**, `.claude/` **9**, `crates/` **5**,
`assets/` **2**. A figure written in any of them looks exactly like a gated one and is re-derived by
nothing. **0** markers live there today; **4** marker spans live inside the census's own `.rs`
source, and all four are definitional — the constant and the prose describing it — rather than
claims.

**S6 — one cap carries headroom, and it is the only one that does.** `UNBINDABLE_RS_MAX` is **176**
over a live **174**: two `.rs` citations naming an ambiguous fragment can land with no red. Every
other ceiling in the file sits exactly on its live count, which is the property that makes the
ledgers worth reading at all.

**S7 — the ignore-marker's own measurement re-scans a marked line WITHOUT its fence.** A marked line
is measured by re-scanning it as a one-line synthetic document, and that document carries no fence
state, so a marked line inside a fenced block would be measured against non-fenced rules. There are
**12** marked lines today — **10** in `REFLECTION-PLAN-CORE.md`, **2** in `REFLECTION-ANALYSIS.md` —
and **0** of them sit inside a fence. That is a cap over an empty set: the shape this file has now
found **four** times and closed **four** times with a permanent discriminator. ⚠️ **This sentence
said "three and three" for one revision, and the fourth was found by the instrument that counts
them, not by a reader**: completing the empty-cap census's own subject list — which had enumerated
seven per-document ledgers where there are nine — turned `stale_planned` into an immediate red, a
capped ledger empty in all nine documents with no discriminator. **A subject list is itself a claim,
and until this round nothing checked it.** There is still none here — and S7's empty set is not one
of the ledger rows the empty-cap census below enumerates, because there is no ledger for it at all.
It is an untested code path, which is the weaker position of the two.

**S8 — the fence flag is a bare toggle with no balance check.** An odd delimiter count puts a
document's whole tail into fence mode silently, and the "does this document yield mentions at all"
guard cannot see it, because the head still yields mentions. Across the nine gated documents there
are **174** delimiter lines and **all nine counts are even**; `FEATURE_MAP.md` has **0**, so eight of
the nine exercise the machinery at all.

**S9 — one marker counts over a population BROADER than the command its own prose prints.** The F22
row prints `grep -rn 'residency = "gpu"' crates/*/src/` — **714** `.rs` files — while its marker
counts every `.rs` file under `crates/` — **1555**. Both answer **0** today.

⚠️ **STRUCK 2026-08-29.** This entry went on to say *"the direction is fail-closed because the
marker's population contains the command's"*. **That containment is a property of today's marker
TEXT, not of the gate**, and one token reverses it. Re-measured directly, each edit applied alone to
the live document and each restored afterwards with `cmp` exit **0**:

| one-token edit | what the FIGURE check printed | suite |
|---|---|---|
| F22's population narrowed from the whole `crates/` tree to a single sub-crate — the containment reversed outright | *"written 0, re-measured 0  `[ok]`"* | exit **101** |
| F17's marker repointed from one readable workflow file to a different readable one that also answers 0, prose unchanged | *"written 0, re-measured 0  `[ok]`"* | exit **101** |

**The figure check certified BOTH mutations `[ok]`.** What reds is an exact inventory of WHAT each of
the **18** markers measures, landed 2026-08-29, which reports the mismatch in both directions — the
subject no row pins, and the pinned subject no marker measures. So a gate that was safe only while
nobody edited it has been given something that fails; but **the containment framing was never the
reason, and stating a mechanism that is not the one operating is how a limit gets recorded as
closed.** And the inventory is a **PIN, not a comparison**: nothing in this tree reads the command a
marker's prose prints, so **a reader re-running the printed command is still not re-running the
gate**, and the population and the prose can drift apart in the one direction the pin does not look.

**S3 — the census file's claim about its OWN reach was false, and is repaired.** Measured in (3)
above: **8** by-line citations per twin, **16** between them, against a doc comment that said twelve
and six. The repair sits in a `.rs` doc comment, which the marker scan structurally excludes, so it
is re-derivable only by re-running the two commands quoted beside it.

**S7 and S8 are enumerated rather than closed, and the reason is measured — but this entry quoted
the cost for two of the three sites a closure needs.** Every site either one touches sits above the
census file's own citation window: the tree cites that file by line at **12** distinct numbers, from
`REFLECTION-PLAN-BOUNDARY.md`, `REFLECTION-PLAN-ECS.md`, both twins and the file itself, and an
insertion renumbers every number below it. Re-derived 2026-08-29 by listing every by-line citation
of that file across the tracked `.md` and `.rs` files, reducing to distinct numbers, and counting
those greater than each site's own line:

| where a closure has to insert | of the **12**, renumbered | of those, living only in these twins |
|---|---|---|
| S7 — the marked-line re-scan (`silenced_by`) | **7** | **4** |
| S8 — the fence toggle inside `scan_text` | **7** | **4** |
| S8 — a balance field on the `DocScan` record | **9** | **6** |

⚠️ **CORRECTED 2026-08-29: the struck sentence read *"seven of the twelve"* and named only the first
two rows.** Seven is right for both of them. But reporting an unbalanced fence the way every other
ledger in that file is reported takes a field on the per-document scan record, and that record is
declared far above either code site — **nine**, with **six** of the nine living only in these two
twins, where nothing would red. **A cost quoted for two of the three sites a change needs is a cost
quoted for a change nobody is proposing**, and it understates in the direction that makes the debt
look cheaper than it is. The three sites are named by SYMBOL and not by line on purpose, twice over:
a coordinate written here has a shelf life of one landing, per (3) above — and writing one would
have added a thirteenth number to the very count this table measures.

Closing either limit line-count-neutrally means compacting unrelated code inside that window; that
is a separate landing, and it is not taken here.

**S10 — which greens mean something, and which are caps over an empty set. NOW COUNTED, every run.**
A ceiling at 0 over a population that is empty in every document reports the value it would report
if it were not running — lesson 1 below, in its ledger form. The census file derives that split from
its own scan and prints it on every run: **84** cap rows, **70** sitting at 0 over an EMPTY live set
and **14** over a live one. The 70 split in two, and only one half was ever a hazard:

* **36** rows belong to the **four** ledgers whose population is empty in EVERY gated document —
  cross-line document inheritance, doc-to-doc misbinding, the fenced note naming no single file, and
  the planned-path marker that waives nothing. Those are the caps that cannot fail, and **all four
  now carry a direct discriminator**: a test that builds a synthetic document exhibiting the defect
  and asserts the ledger catches it. Two landed 2026-08-29 — the fenced-note one, which had been
  gating a live fix, and the planned-path one, which existed only because completing the inventory
  turned it red.
* **34** rows belong to the **five** per-document ledgers that ARE live somewhere — planned paths
  (**4** live), an inherited alias binding (**1**), ignore-marked lines (**12**), unquoted doc-to-doc
  anchors (**43**), waivers that were not needed (**7**). A zero in one of those rows says *"this
  document is clean"*, and the mechanism behind it is exercised by the documents that are not. The
  remaining live rows are the **three** whole-tree ceilings the two source censuses own — unbound
  `.md` citations (**8**), ambiguous `.rs` fragments (**174**), dead `.rs` fragments (**5**).

⚠️ **Those three numbers read 66 / 54 / 12 over "seven per-document ledgers" until 2026-08-29, and
the arithmetic was SELF-CONSISTENT — 7×9+3 = 66 — which is exactly why it survived being read.** It
was not a miscount; it was a short SUBJECT LIST, missing two ledgers this same file caps. The
instrument written to find caps over nothing was blind to one of them for the same reason the corpus
was: nothing checked the list of things it was counting. **An internally consistent total is
evidence about the arithmetic and none at all about the population.**

**The census is itself a gate, not a note**: it fails the build if a fifth ledger becomes wholly
empty without a discriminator named beside it, and — since 2026-08-29 — if a named discriminator
does not RESOLVE to a `#[test]` in that file, which it previously did not check at all. That
distinction is the whole point: after this landing no wholly-empty ledger is left without one, and
the remaining 34 empty rows would gain nothing from a synthetic fixture, because a live mechanism
already stands behind each. ⚠️ **What it still does not do is judge the discriminator**, only
resolve it; and it says nothing about the fourteen live rows, whose hazard is the opposite one —
headroom.

**S11 — the same class exists in the KERNEL, in TWO data, and the enumeration that closed it covered
ONE.** Recorded here because it is the same shape and NOT the same instrument: the gates live in
`crates/boyko_ecs/tests/seam_by_id.rs` and no leg of the target this section is about would ever see
them. A dense-presence-bit seed on the migration path was covered by no test — the landing of
2026-08-28 swapped its archetype argument for the other one in scope, the kernel compiled clean, and
the whole ECS suite stayed green with the gate written to close that very class passing beside it.
`g18b` covers that one.

⚠️ **CORRECTED FOUR TIMES — 2026-08-29/-30/-31 and again: twice the repair PATCHED THE COUNT, twice the next
reading refuted it a site later; the fourth moved no number — the STRENGTH was wrong.** `mark_arch_present` has
**8** call sites under `crates/boyko_ecs/src/`; the criterion is SCOPE, not SIGNATURE — two archetype ids both
OPERAND-DERIVED (off THIS call's own entity / source / target / sibling, via a parameter or one accessor) AND able
to DIFFER at runtime. Minting a fresh id is writable at all eight and is not the class; nor is a second ROUTE to
one value. Re-derived site by site, each expressible one WRITTEN and RUN: **FIVE are expressible** — clone
materialiser (`g19`), migration helper (`g18b`), typed dense insert (`g18`; its caller passes the TARGET *before*
the migration), and **BOTH fresh-world load walks, which NO gate covers** — a member's own archetype against a
sibling's, and `arch_presence` is a bitSET because a store's members CAN span archetypes. **THREE are
inexpressible**: two spawns on one cached bundle archetype, and the in-place replace, whose routes are provably
EQUAL. ⚠️ **The two ungated are UNASSERTED, not UNREACHED**: four files reach these walks and three query the
store afterwards, but every store is fed from ONE bundle, so a member's archetype and a sibling's are the SAME
id; the swap leaves 14 serialize and 170 ECS targets green. A gate must CONSTRUCT a two-archetype load, not pin one.

**The class is TWO data wide, and the second had five ungated sites.** Measured 2026-08-29. The
class is *an archetype-keyed presence bitset consumed as a query's candidate set*, and it is closed
by a structural property rather than by a name: a candidate set reaches a query ONLY through
`QueryState::seed_from_candidates`, which is declared once, is `pub(crate)`, and has exactly
**four** call sites, all four in one file. Two of them read the dense store's `arch_presence`; the
other two read `EnablePresence::snapshot_present`. **So the class has exactly two members**, and a
third could only appear as a fifth call site of that one funnel — which the next reader re-checks
with one command rather than trusting a list. That is the difference between an enumeration and a
sample, and it is the thing the earlier wording claimed without having.

The second member is seeded by `EnablePresence::note_column_alloc`, reached only through
`ArchetypeMaster::note_enable_column_alloc`, whose migration-side caller is
`fire_enable_column_alloc_bookkeeping`. That helper is called from **five** migration functions,
each passing the target archetype while the source is live and, in four of the five, a named
parameter one token away — `migrate_entity_insert`, `migrate_entity_remove`,
`migrate_entity_attach_ids`, `migrate_entity_detach_ids`, `migrate_entity_attach_ids_with_bytes`.
**The fifth is the by-id attach this very rung added**, so the landing that gates the class is also
the landing that widened it. They are named by SYMBOL and not by line, deliberately: a coordinate
written outside a gated document has a shelf life of one landing, per (3) above.
`note_enable_column_alloc` has one further caller, the in-place enable path, and it is NOT a sixth
instance — a single archetype id is in scope there, so the mutation cannot be written, by the same
argument the eight-site sweep makes for three of its eight.

**DISPOSITION: gated, by `g18c`, beside the other two.** Not deferred, not filed, not waived. No
kernel source changed. Two findings the measurement produced that the gate does not remove:

* **The consuming side is as unforgiving as the first datum's.** The code at the enable seed says
  the candidate bitset *"IS the membership predicate (nothing to trim)"* — there is no per-row pass
  to recover a wrong id, so a bit on the wrong archetype is not a slow query, it is a silently empty
  one.
* **The site was UNREACHED, not merely unasserted, and that is the stronger half.** Under the
  swapped argument the whole ECS suite stayed green AND `note_column_alloc`'s own unconditional
  debug assertion — *"must be called only on a genuine first column"*, *"the caller already holds
  the bit"* — did not fire either. In a debug build its silence proves that no test in the suite
  reached that helper with a non-empty list on the by-id attach path AT ALL. The gate therefore had
  to CONSTRUCT the reaching case rather than pin an existing one. **A green over an unreached site
  and a green over a correct one are the same green**, which is lesson 1 below in its kernel form.

**The transferable half: a sweep over a SYMBOL cannot close a CLASS, and the two are told apart by
asking what a third member would have to LOOK like.** Here the answer was "a fifth call site of one
`pub(crate)` funnel" — mechanical, and therefore an enumeration. Had the answer been "any code that
sets a bit keyed by archetype", the same work would have been a sample, and the honest word for it
was available at the time.

**The figure-level limits stay where they were written.** The second table in (5) above lists every
measurable claim this rung wrote that carries no marker, with the reason for each: a frozen commit
stamp (which IS the gate), a regex-shaped count, a body-scoped count, a claim that is not a count, a
figure whose subject is a `cargo test` process or a symbol table, and two that are **self-referential
and ungatable by any vocabulary**. One row of it is a LIVE hazard rather than a closed one, and it
has now been wrong twice in the same direction: the claim reasons in the present tense from a figure
taken at round 5, and the round that flagged that wrote the ECS-only count where the claim's own
printed command spans all of `docs/`.

⚠️ **CORRECTED 2026-08-29 — and the correction is that this sweep had THREE simultaneous live
answers in one tree, all three written in the present tense.** The row in (5) above said **16**, this
paragraph said **13** four screens below it, and the census file's own doc comment said **nine** —
that last being the ECS-only population, which is not what the command prints. Only one of the three
could be right and none of them was checked, because an alternation is the one shape the marker
vocabulary cannot carry. Re-taken here after the last edit of this landing, which is the only order
that produces a true row: **22** — **10** in
`REFLECTION-PLAN-ECS.md` and **6** in each twin. **It is a DATED reading and it
is not maintained.** Every one of the three stale answers was produced by a pass that moved the
number while writing about it, this one included: the sentences you are reading are themselves
counted. The instruction that follows from three failures rather than one is to stop keeping a
self-referential count current and to re-take it, with a date, at the moment of writing — which is
what the row in (5) already concluded and what this paragraph did not do.

**Two lessons the 2026-08-28 pass produced, stated for transfer.**

1. **A measurement that returns a BARE COUNT cannot distinguish an absent token from an absent
   population.** `0` is the answer to *"the token does not occur"* and equally to *"there was nothing
   to look in"*, and a claim asserting absence wants to hear `0` either way. Two one-character
   edits — a filename that does not exist, an extension that selects no files — were OBSERVED GREEN
   over live false claims, until the measurement was given a *nothing to read* answer distinct from
   nought. This is the same shape as a cap over an empty set and as a floor over an emptied corpus:
   **the instrument reports the value it would report if it were not running.** In general form: a
   measurement whose *nothing* and whose *none* are the same value is not a measurement of the thing
   it names.
2. **Gating a claim FORCES THE TRUTH OUT — and it is the ATTEMPT that does it, not the gate.** Three
   sentences in this campaign's plan read as present-tense statements about the tree and resisted
   gating; trying to gate them is what revealed they were a PRE-STATE — true when taken, false of
   the tree that shipped, and false *because the work succeeded*. The identical shape produced (3)'s
   stale quotation: a line read before an insertion, written as a live reading, past a clean parity
   check. **A sentence that resists gating is not merely ungated; it is the first place to look for
   one that is already false.** The remedy is not always a marker — a figure stamped to a named
   commit is frozen, and a frozen figure cannot rot — but the choice has to be made deliberately, at
   the site, and written down there.

**Two more from the 2026-08-29 pass, which closed the items above.**

3. **A cap over an empty set is not a gate — and until this pass nothing in this corpus knew how
   many of them it had.** A ceiling of 0 over a population that is empty in every document passes
   for exactly the reason a deleted test passes, and in a report it is indistinguishable from a
   checked zero. Counted here for the first time and re-derived on every run — **84** cap rows in
   the census file, **70** of them at 0 over an EMPTY live set, **36** of those over the four
   ledgers empty EVERYWHERE, the other **34** live in some OTHER document, where a zero honestly
   says *"this document is clean"*. ⚠️ **The first counting published 66 / 54 over THREE such
   ledgers, and its own subject list was short by two** — the lesson recurring inside the sentence
   that states it. **The transferable half is the arithmetic, not the four discriminators**: a
   corpus that publishes ledgers without publishing which of them have a live population has
   published a coverage figure it never measured. Ask the ratio before trusting the greens.
4. **"Fail-closed" can be a property of the DATA rather than of the GATE, and from a green run the
   two are indistinguishable.** A limit here was recorded as safe because a marker's population
   CONTAINED the command its prose printed, so drift could only over-count. True of the marker's
   TEXT; false of the mechanism. One token narrowed the population and reversed the containment, and
   the figure check certified the result `[ok]` — the argument's premise was an editable field, and
   editing it cost nothing. **A safety argument that reasons from the current value of something a
   future editor can change is an argument about today's data, and it will be filed as an argument
   about the gate.** The test is mechanical and takes one minute: change the field and ask what
   reds. If the answer is *"nothing, but nobody would write that"*, the limit is open, and the
   sentence recording it as closed is the more expensive of the two defects.

**One more from the 2026-08-29 record pass, and it is the FIFTH recurrence of a single shape — which
is why it is stated once here, at the site where it keeps happening, rather than a sixth time in the
margin of whatever moves next.**

5. **A figure written about the EDIT SET IT LIVES INSIDE is a pre-state, and the words "re-measured
   correctly" beside it do not make it a measurement.** The recurrences, in order: round 3's own
   `git diff --stat` totals; the three struck name sweeps in `REFLECTION-PLAN-ECS.md`; the *"2 of
   25"* harness names; the ownership sweep's *"re-derived on every run since"*; and the by-id seam
   test's length, written **2572** by the same uncommitted landing that had already taken the file
   to 3054 — under the one phrase in its paragraph that claimed the check had been done. **Five
   times, in five different sentences, by four different passes, always in the direction that
   flatters the work.**

   ⚠️ **It then happened a SIXTH time, in this pass, and the sixth is the only one worth
   celebrating.** The record pass rewriting the section above also appended to that same test file,
   taking it 3054 → **3074**. It did not ship. The gate landed one round earlier printed *"the tree
   says 3074 and the sentence stating it does not write that number"* and the figure was re-taken
   before the pass finished. **Five prose repairs and one red is the whole difference the gate
   makes**, and it is also the honest limit of it: nothing here stops a figure from moving, it stops
   a figure from moving quietly.

   The transferable form is a question, not a rule: **"re-measured correctly" is a claim ABOUT a
   measurement, and the two are told apart by asking who else could have written the sentence.** A
   number some named check re-derives can be written by any reader; a number only its author can
   vouch for is a testimonial, and a testimonial about one's own edit set is the weakest evidence in
   this corpus. When the second kind cannot be avoided — a `git diff` total, an alternation, a count
   of one's own prose — the honest form is a DATE and a round, taken after the last edit, and never
   the present tense.

### (7) What this landing's commit message must disclose

Written here so the message can quote a record rather than re-derive one. Every item was measured
during this pass, at this revision, by the command named beside it; the four that reverse or
sharpen a previously recorded limit are marked.

**Limits that CLOSED, and must be reported as closed rather than left standing:**

1. **The fenced-note discriminator DOES cover the sticky-fallback route.** ⚠️ **Reverses the limit
   recorded at the previous round.** Measured by applying the regression the resolver's own doc
   forbids — keeping the unique-fragment arm and appending a sticky fallback after it — and running
   the discriminator alone: it reds, and it reds at the fixture whose message names that route
   (*"that is the one route on which a sticky fallback is reachable, and the five fence-on-line-1
   fixtures cannot see it"*), with the five earlier fixtures running first in the same function and
   not firing. ⚠️ Note for anyone re-running it: the *replacing* form of that edit — swapping the
   unique-fragment arm out for the sticky one — reds at an earlier fixture instead and proves
   nothing about this route. **The two edits are one token apart and answer different questions.**
   Restored afterwards, `cmp` clean.
2. **The empty-cap census RESOLVES its discriminator names against the file's own source, and
   requires a `#[test]` above each.** ⚠️ **Reverses the limit recorded as "named by string and never
   resolved".** Measured by renaming one entry: the run reds with *"names `…` as its discriminator,
   and no such test is defined in this file"*. It also now covers the planned-path ledger, which the
   earlier subject list omitted — that omission is what turned the ledger red and produced the
   fourth discriminator. **What it still does not do is judge the discriminator**; resolving a name
   is not assessing a test.

**Limits that remain OPEN, with the measured size of each:**

3. **Neither `g18` nor `g18b` distinguishes the shipped unconditional dense seed from one guarded on
   the insert's `newly_added` return.** MEASURED this pass: applying that guard to the dense
   migration seed and running the seam target leaves it at **26 passed, 0 failed, 2 ignored**. Both
   gates drive the newly-added case, where the two implementations agree; the divergent case — an
   entity already carrying the dense id when the migration re-inserts it — is built by neither.
   Kernel restored byte-identical afterwards, proved by `cmp`. This is a REAL gap in the gates that
   close the presence-seed class, and it is filed, not fixed.
4. **The marker-to-command comparison stays deferred, over a corpus in which 7 of 18 markers carry a
   machine-readable command in the window the proximity guard reads.** ⚠️ **Sharpens a figure that
   was recorded as 3.** The 3 was taken by naming the markers a reader had noticed rather than by
   scanning that window, and the sentence then reasoned from it down to *"two markers in eighteen"*.
   Seven is more than twice it, and the error ran in the direction that spares the work. The
   deferral survives the correction on its own merits — F22 writes its population in prose rather
   than in its command, and two rows print a pipeline the vocabulary has no shape for — but it must
   be quoted at **7**, not at 3, and **not at 6**.
5. **A figure gated where it is PRODUCED is not gated in these twins.** The four `wc -l` lengths are
   re-derived on every run by a named test — which reads the census file's own doc comment, never
   this document. The same is true of the census transcript and of every marker value copied into
   the tables in (5) above. **Every number in these two files is a dated reading unless it is one of
   the four marker values these files themselves carry.**

**Figures this landing MOVED, which is a disclosure and not a footnote:**

6. **TWO of the ECS plan's three name-sweep markers moved inside this one landing, in opposite
   halves of it.** `AddOutcome` went **42 → 43** when the kernel gate that closes the presence-seed
   class was written, because that gate uses the identifier; `migrate_entity_attach_ids_with_bytes`
   went **15 → 16** when this record pass named the fifth migration site in that gate's header — a
   `.rs` line under `crates/`, and therefore inside the marker's population. **Both were caught by
   their own marker and neither by a reader**, and in both windows the twins carried the stale copy,
   because a marker gates the plan and never the copy. A tree-wide count of an identifier the
   campaign's own tests and headers use is a figure the campaign moves by working.
7. **The by-id seam test moved three times inside this one uncommitted edit set** — 2572 → 3054 by
   the code landing, 3054 → 3074 by a record pass, 3074 → **3283** by the one after it. The first
   move shipped a stale number for a round; each later one was caught by the gate the first landed.
8. **The target's own test count went 21 → 23**, and the empty-cap census's split went 66 / 54 / 12
   → **84 / 70 / 14** over four wholly-empty ledgers rather than three. Both moved because this
   round's work added tests and completed a subject list.
9. **The `g18`-or-`gate 18` alternation sweep is re-taken and DATED**, not kept current: it had three
   simultaneous live present-tense answers in one tree before this pass, and every one of them was
   written by a pass that moved the number while writing about it — this one included.
10. **Two quotations in the census file attributed words to a source that never carried them, and
    both are repaired.** One quoted this row's own predecessor as *"all four re-measure correctly
    today"*; the sentence in these twins read *"All four re-measure correctly"*, and the trailing
    word occurs nowhere in the corpus. The other quoted a kernel test header as calling its
    enumeration *"complete rather than sampled"*; that phrase lives only here, in the question log,
    and was never written in that file. **Both are the recollection defect in miniature** — a claim
    copied from memory inside a paragraph about claims copied from memory — and both were found by
    searching for the quoted string rather than by reading. **Quote verbatim, or drop the quotation
    marks.**
