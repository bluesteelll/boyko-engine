# E4 — Replay

**Part of the editor campaign design.** Index and v1 ladder: `EDITOR-DESIGN.md`. **Revision 4.**
**Revision 4** moved `EK16` from `V4` to `V2`, where it also becomes the undo oracle, and wrote down the two component-enumeration traps its walk must avoid (§1.5).
Decisions `E4.1`–`E4.7`. Every code claim carries a `file:line` read at HEAD `98ede3e0`.

| § | decision |
|---|---|
| §1 | `E4.1` — the family, and the world digest (`EK16`) |
| §2 | `E4.2` — compile-time, runtime, or hybrid |
| §3 | `E4.3` — what identifies a build |
| §4 | `E4.4` — the recorder must not perturb what it records |
| §5 | `E4.5` — is determinism sufficient? |
| §6 | `E4.6` — what makes a recording ANALYSABLE |
| §7 | `E4.7` — standing, in the words the brief asked for |
| §8 | out of scope, and what this file did NOT establish |

---

## 1. The family — `E4.1`

**Decision. Input log + initial world image + per-frame `dt` + the command log + a per-frame world
digest, with optional periodic world images every K frames (default 600).**

### 1.1 Why this shape and not the two obvious ones

| family | price | verdict |
|---|---|---|
| **Snapshot-only** (every frame a world image) | no determinism obligation at all, which is genuinely attractive; and unaffordable in bytes, and useless for reproducing a bug's causal chain — a snapshot stream shows WHAT the world was, never WHY | rejected |
| **Input-only, no images** | the cheapest; but a divergence at frame 9000 costs a 9000-frame re-simulation to inspect, and there is nothing to compare against to LOCATE the divergence | rejected on its own |
| **Input + initial image + digests + periodic images** (chosen) | the digest makes a divergence detectable at the frame it happens; the periodic images make it inspectable without re-simulating from zero; the command log makes the agent-issued half of the causal chain readable | **taken** |

### 1.2 The seams the engine already has

Unusually clean, and it is why this feature adds little engineering.

- **Input has exactly ONE seam.** Every frontend translates its native events into `RawInputEvent`
  and pushes them through `RawInputQueue::push_raw`
  (`crates/boyko_input/src/raw/queue.rs:76`), documented as "the single load-bearing seam". The
  event enum is `#[repr(C)] Clone + Copy` with 9 POD variants, and its own comment says `#[repr(C)]`
  was chosen for "any future record/replay log that blits the ring"
  (`crates/boyko_input/src/raw/event.rs:14-15`). The anticipation is in the code; the feature is not.
  Recording is a tee on that function (`EK6`); replay is pushing the log back in.
- **The frame function already takes its delta as a parameter.** `App::update_with_delta(raw:
  Duration)` is THE frame function (`crates/boyko_ecs/src/ecs/core/app/app.rs:660`); `update()` is
  its self-clocked shell (`:769`); `run_n_with_delta` already exists as the deterministic driver
  (`:793`), so `Instant::now` jitter stays out of measured loops. `Time` supports pause and
  `relative_speed`, and `fixed_advance` consumes the already-clamped virtual delta.

### 1.3 The one seam that does NOT exist — `EK5`

The windowed runner does not use the injectable path: it computes `let dt = now - last;` inline and
calls `app.update_with_delta(dt)` with no injection point
(`crates/boyko_app/src/runner.rs:1223-1233`). `EK5` adds a frame-clock source the runner reads —
`Wall` (today's behaviour, the default) or `Scripted(&[Duration])` — so a replay can be stepped under
the real window. v1's replay is HEADLESS through `run_n_with_delta`; `EK5` exists so that windowed
playback is a configuration rather than a rewrite, and so that the recorder's `dt` capture and the
player's `dt` injection are the same seam.

### 1.4 What a recording file contains

| section | content |
|---|---|
| header | magic, format version, the build stamp (§3), `session_id`, the determinism-set manifest (§5.2), K |
| `image[0]` | a `save_world` image of the world at record start |
| per frame | `dt` (the raw duration as passed to `update_with_delta`), the input events pushed that frame, the command envelopes applied that frame, the frame digest |
| every K frames | a full `save_world` image |
| trailer | the frame count, the digest of the final frame, and the recorder's own counters (events recorded, events dropped, bytes written) |

### 1.5 The frame digest — `EK16`, and why it is NOT a `save_world` byte hash

**Decision. The per-frame digest is an ORDER-INDEPENDENT, `StableId`-KEYED SET DIGEST over a declared
determinism set of components. It is not a hash of the `save_world` byte image.**

**Revision 4 moved `EK16` down the ladder, from `V4` to `V2`, and it now serves THREE oracles.**
`V2`'s undo proptest had been written against `save_world` byte-identity with a bespoke argument that
ids are preserved when no reload intervenes; that argument is false twice over
(`EDITOR-COMMANDS.md` §8.2b), and the failure mode was the same deterministic false red this section
was written to remove. So the undo oracle, the document round-trip oracle (`V4` G-A) and the replay
fidelity oracle (`V8`) are now ONE mechanism, built once at `V2`, and the reason to build it well is
correspondingly stronger. It also means `EK16` is exercised by a headless proptest two rungs before
anything windowed depends on it.

Revision 2 said "an FNV-1a-64 hash of the `save_world` byte image". That is not invariant under the
id remap a replay performs, and it fails in the direction that produces a deterministic FALSE RED:

- the saver blits each archetype's raw `EntityId` column
  (`crates/boyko_serialize/src/save.rs:85-92`, "Pass 2 blits `entity_count` little-endian `u64`s from
  it in one memcpy") and each dense store's `s2e` owning ids (`:310-321`);
- `load_world` allocates FRESH ids for every saved entity — `LoadEntityPolicy::Remap` is the ONLY
  variant (`crates/boyko_serialize/src/load.rs:69-78`).

So a replay that starts by loading `image[0]` produces a world whose ids differ from the recorded
live world's, and frame 0's hashes differ for a reason that is not divergence. The `V8` fidelity gate
would red on every run, and the predictable response under pressure is to weaken the oracle — the
exact failure this design predicts and guards against for event-lane order (§5).

**The digest, defined.** For each live entity `e` with stable id `s`:

```
row(e)  = D( s , Σ_{c ∈ set, present on e}  D( stable_name_hash(c) , bytes(c, e) ) )
H(world) = Σ_{e live}  row(e)
```

where `D` is a 128-bit FNV-1a and `Σ` is wrapping 128-bit addition. Both combinations are commutative
and associative, so the digest is invariant under:

- **archetype creation order** (a live world builds archetypes incrementally; a loaded world builds
  them in file order),
- **row order within an archetype** (swap-remove ordering depends on operation history),
- **dense slot order** (`for_each_live` yields insertion order, which differs after a reload),
- **entity-id remap** (the key is the `StableId`, which is the thing that survives `Remap` by
  construction — that is what `StableId` IS, `EDITOR-BOUNDARY.md` §5).

**Enumerating an entity's components: two traps, both already solved in the tree, both easy to walk
back into.** `EK16`'s walk must copy `save_world`'s habits rather than invent its own.

1. `archetype.component_ids()` is the archetype's id slice, and it is NOT the same thing as "the
   components whose bytes live in this archetype's pools" — `save_world` reads it and then does
   `component_pools().get_pool(component_id)` with `None => continue`
   (`crates/boyko_serialize/src/save.rs:172-190`). A digest that trusts the slice alone hashes ids
   that have no column here, and what it hashes then depends on spawn order.
2. Dense components are signature-excluded, so they are absent from every archetype's slice
   entirely. `save_world` handles them in a SECOND pass over `world.dense_registry().dense_ids()`,
   keyed by entity rather than by archetype, and its own comment says why ("a dense store's
   slot→entity map is NOT the archetype entity list", `save.rs:277-283`). A one-pass digest is blind
   to every dense component — including the physics solver state that a replay oracle most needs to
   see. `dense_registry()` and `dense_ids()` are public
   (`crates/boyko_ecs/src/ecs/core/ecs_master/ecs_master.rs:590`,
   `.../component/dense/dense_registry.rs:156`), so this costs an accessor nobody has to add.

The same two traps are why `V4`'s G-B is an entity-id census over the FILE rather than a walk over
the world: reading the file's own tables sidesteps both.

**The determinism SET is declared, and one class is refused from it.** A component whose entity-remap
fn is installed — `MapEntitiesFn` (`crates/boyko_ecs/src/ecs/core/component/component_registry/clone.rs:55-58`)
or `LoadMapEntitiesFn`
(`.../component_registry/serialize.rs:87-90`) — carries `Entity` fields, whose raw bytes are NOT
remap-invariant. Such a component is REFUSED from the determinism set at registration with a coded
error naming it, rather than silently hashed wrong.

**What that costs, stated with its mitigation.** `ChildOf` and any `#[entities]`-bearing component
are outside the digest, so a re-parent is not directly observable by the oracle. It is INDIRECTLY
observable, and that is not a consolation prize: re-parenting changes the resulting `GlobalTransform`
values, which ARE in the set. What is genuinely invisible is a re-parent that changes no transform —
a real hole, stated rather than papered over, and the fix (substituting referent `StableId`s through
the remap fn into a scratch copy of the row) is recorded in §8 as the v2 shape.

**One mechanism, two consumers.** The same digest, taken over the full serializable set rather than
the determinism set, is `V4`'s document gate G-A (`EDITOR-BOUNDARY.md` §3.4) — the oracle that
survives the archetype fragmentation a byte compare cannot. Landing it once serves both.

---

## 2. The gate: compile-time, runtime, or hybrid — `E4.2`

**Decision. HYBRID — compiled in behind cargo feature `replay` (default off), ARMED at runtime by a
command or by `BOYKO_REPLAY_RECORD=<path>`.**

The engine has both patterns and the design must pick one deliberately.

| pattern | in the tree | what it can promise | what it cannot |
|---|---|---|---|
| **Compile-time** | `hwrt` is a default-off cargo feature; a `not(hwrt)` build is TEXTUALLY the pre-feature code (`crates/boyko_app/Cargo.toml:44-48`) | a shipping game binary carries NOTHING: no code, no cost, no obligation | a player or a QA build cannot turn recording on without a rebuild — which, for a bug-reproduction feature, may be the whole point |
| **Runtime** | `PhysicsConfig`'s `simd` / `simd_solve` / `parallel_solve` / `colored` / `broadphase` are default-off runtime flags: the code ships, a branch decides | anyone can turn it on in the field | it cannot make the shipping-binary promise, and a branch on the frame path is a cost the `not(replay)` case should not pay |
| **Hybrid** (chosen) | — | both halves: `not(replay)` is textually the pre-replay code, and a `replay` build (a QA build, a developer build, an opt-in beta) arms at runtime with no rebuild | it is two mechanisms rather than one, and the `replay` build pays an armed/disarmed branch |

The two pure forms each fail one of the owner's requirements, which is why the hybrid is taken rather
than assumed. The `V8` gate includes a symbol census asserting a `not(replay)` build contains no
recorder.

---

## 3. What identifies a build — `E4.3`

**Decision. An FNV-1a-128 hash of the executable image, plus `session_id`, plus the bytes of every
determinism-affecting resource, plus the format version. A mismatch on load is a coded refusal naming
which field differs.**

### 3.1 Why an exe-image hash and not a commit hash

A commit hash is not enough, and the reason is concrete rather than theoretical:

- uncommitted changes alter the arithmetic and leave the commit hash unchanged;
- **the ISA baseline changes which kernels compile** — this engine's `target-cpu` baseline was changed
  recently, and `RUSTFLAGS` REPLACES rather than extends the config `rustflags`, which this
  repository has recorded as one cause with three symptoms (AVX2 missing from CI, loom "crashing", a
  red ISA census). A recording made under one flag set replayed under another is a different program.

Hashing the executable image covers all of it in one number, and it is cheap: the recorder hashes the
mapped image once at arm time, not per frame.

### 3.2 The rest of the stamp

| field | why |
|---|---|
| `session_id` | already available (`boyko_diag::clock::session_id`, stamped into profiling artifacts) so a reader can prove which process wrote a file |
| `PhysicsConfig` bytes | `simd` / `simd_solve` / `parallel_solve` / `colored` / `broadphase` change the arithmetic; they are runtime flags, so the exe hash does not cover them |
| `Time` config bytes | `max_delta`, `relative_speed` |
| worker count | the {1, N}-worker determinism property is a property, not an assumption; recording the count makes a mismatch a stated difference rather than a silent one |
| determinism-set manifest | the ordered list of `(stable_name, stable_name_hash)` the digest covers, so a replay under a build with a different set refuses instead of comparing different things |
| format version | the file layout |

### 3.3 The refusal, and what it dissolves

**Cross-version replay is REFUSED, explicitly.** A stamp mismatch is `E19xx` naming the differing
field — "recorded by a different executable image", "PhysicsConfig.colored differs", "determinism set
differs at entry 7: `Velocity`" — never a best-effort replay that diverges silently.

That single refusal dissolves the "every future numerical change is a breaking change" problem
entirely. The obligation is WITHIN a build: a recording made by build X is replayed by build X,
usually minutes later, for the stated purpose of reproducing a bug for an agent to analyse. Nobody
asked for a recording from last month to play in today's binary, and the debugging-oriented recorders
in the field make the same refusal.

---

## 4. The recorder must not perturb what it records — `E4.4`

**The worst possible failure mode of this feature is a bug that vanishes when you record it.** So
non-perturbation is not a quality goal, it is `V8`'s FIRST gate.

**The rules the recorder obeys on the frame path:**

- **No allocation.** The recorder owns a preallocated ring, sized at arm time, drained by a writer
  thread. A full ring INVALIDATES the recording (counted, coded) rather than stalling the frame —
  because a stall changes timing, and timing under load changes scheduling.
- **No clock read.** The recorder never calls `Instant::now`. The `dt` it records is the one the
  runner already computed and passed in; it is copied, not re-measured.
- **No blocking.** The drain is SPSC; the frame path only writes.
- **No new scheduling.** The tee is a straight-line append inside `push_raw`, which the runner already
  calls serially before the scheduler window ("no atomics needed"), so it introduces no
  synchronisation.
- **The digest is the one honest per-frame cost.** It walks the determinism set once per frame. It is
  inside `cfg(replay)` and inside the armed branch, and `V8` REPORTS its cost rather than gating on
  it.

**The gate.** The per-frame digest sequence of a RECORDED run must be IDENTICAL to the UNRECORDED run
of the same input. A red here means recording changes what it records; nothing further is worth
measuring until it is green.

### 4.1 The clock census this owes

Anything on the frame path that consults a clock is a divergence source. Outside `boyko_ecs`, exactly
two systems consume `Res<Time>` — `fly_camera_system` (`crates/boyko_scene/src/camera.rs:880`) and
`particle_system` (`crates/boyko_render/src/particle_system.rs:308`) — and both read the VIRTUAL
delta, which is a function of the recorded `dt`, so both are replay-stable by construction. What is
NOT covered by that census and must be checked at `V8`: any direct `Instant::now` / `SystemTime` call
inside a system body or an asset path in the play process. `V8` runs the census and reports it as a
row per site rather than asserting there are none.

---

## 5. Sufficiency — is determinism enough? — `E4.5`

**Decision. Sufficiency is MEASURED, per source, not assumed. Event-lane order is predicted RED in
advance.**

Input determinism is necessary and nowhere near sufficient. Asserting the three physics properties
suffice would be the failure this repository has recorded as checking a mechanism for EXISTENCE
rather than REACHABILITY — and it is the assumption whose failure is silent.

### 5.1 The divergence-source table (`V8` (3) reports one row each)

| source | why it could diverge | prediction |
|---|---|---|
| **physics run-to-run** | already gated on one machine | green (it is an existing, passing gate) |
| **physics {1, N} workers** | the colored solve is lock-free; this gate is its race detector, which Miri cannot replace | green, and its standing changes (§7) |
| **SIMD vs scalar oracle** | an existing bit-identity gate | green |
| **event-lane order** | events of two systems interleave PER WORKER THREAD, so lane order follows scheduling | **RED, predicted here in advance.** This repository's own history includes a colored-solve data race and a `par_iter` serialisation finding in exactly this area |
| **command-lane drain order** | the inbox has a stated drain order (UI, then transport, each in arrival order), so it is deterministic BY DESIGN — but only if the transport thread's arrivals are themselves recorded, which they are (the command log) | green by construction; verified, not assumed |
| **asset residency frame** | an asset that becomes resident on a different frame changes what a system sees | unknown; measured |
| **padding bytes in POB digests** | uninitialised padding inside a `PlainOldBytes` component would make the digest nondeterministic within one run | unknown; the digest walk must be checked against padding, and if it is a problem the fix is per-field hashing for the affected type rather than weakening the oracle |
| **any `Instant` read on the frame path** | §4.1's census | census reports per site |

### 5.2 The oracle

The whole-frame digest at **W = 1 vs W = N over the same recording**, reported PER SOURCE by
narrowing the determinism set: physics-only, transforms-only, then the full set. A red on the full
set with a green on the narrow ones LOCATES the source instead of merely detecting it.

**Event-lane order is expected red, and that prediction is on the record here so that a red is a
CONFIRMED FINDING rather than a discovery to be reframed.** If it is red, v1 replay is bit-stable for
physics and transforms and best-effort for gameplay events, and that is what the feature promises.
Making lane order deterministic is a scheduler change with a real cost; it is recorded out of scope
with its price (`EDITOR-DESIGN.md` §5) and is owner question 4, not something to absorb silently.

---

## 6. What makes a recording ANALYSABLE — `E4.6`

Replayable and analysable are different axes, and the second is what the owner actually asked for
("so that a neural network can replay it and analyse what is happening").

### 6.1 The loop

1. `replay.load(path)` — validates the stamp, refuses a mismatch naming the field.
2. `replay.seek(frame)` — loads the nearest preceding periodic image and replays forward to `frame`.
   Because the digest is order-independent and `StableId`-keyed (§1.5), a seek and a continuous
   playback agree at the same frame; under revision 2's byte hash they would not have.
3. `replay.step(n)` — advances n frames, returning the digest and any diagnostic codes emitted.
4. `world.query` / `entity.get` / `component.get_field` at any point — the agent inspects the world
   through the SAME command surface it uses in the editor, so nothing new has to be learned or built.

### 6.2 The log join

Every frame's diagnostic-code emissions are recorded alongside the frame, so `log.tail` over a
replay is joined to the frame index. Combined with `explain`, an agent can ask "what went wrong at
frame 8412" and get a code, a documented meaning and the world state at that frame — which is the
information the AI-orientation requirement set argues never depreciates ("tools that provide
information the model cannot derive — what exists, what fired, what is canonical").

### 6.3 The command log

The command envelopes applied each frame are the agent-issued half of the causal chain, and they are
the SAME artefact the undo stack captures (`EDITOR-COMMANDS.md` §8): a command with its inverse is a
command that can be logged, replayed and explained. One mechanism, three consumers.

### 6.4 Rendering during playback

v1 playback is headless; rendering during playback is a courtesy, not a contract. **GPU readback is
not part of the determinism contract** and the digest does not cover it, so a frame that looks
different on screen during playback is not a divergence unless the digest says so. Stated here so
nobody builds a screenshot-comparison gate on top of a promise that was never made.

---

## 7. Standing — `E4.7`

In the words the brief asked for:

> **Under cargo feature `replay`, within-build bit-exact determinism of the simulation is a PRODUCT
> REQUIREMENT, not a testing convenience.**

The consequences, stated so the owner sees them:

1. **The physics oracles change standing.** Run-to-run bit-identity, {1, N}-worker bit-identity and
   the SIMD/scalar oracle stop being merely debugging instruments and become the guard on a shipped
   feature. A change that reds them is a breaking change **of that feature**, never of the engine.
2. **The worker-count oracle acquires a second job.** It is today the race detector for a lock-free
   solve Miri cannot check; it additionally becomes the guarantee that a bug recorded on a QA machine
   reproduces on a developer's machine with a different core count. That is the property that makes
   the feature useful at all.
3. **The scope of the promise is exactly the scope of the digest.** Not rendering, not audio, not
   anything outside the declared determinism set — and the set is written into the recording's
   header, so a reader can see what was promised.
4. **The engine is NOT obliged to freeze its numerics.** The promise is within a build; cross-version
   replay is refused (§3.3). This is the whole point of the refusal and it should be read together
   with it.
5. **A `not(replay)` build carries none of this.** No code, no cost, no obligation — which is what
   the compile-time half of the hybrid gate buys.

---

## 8. Out of scope, and what this file did NOT establish

**Out of scope (recorded):**

| item | why |
|---|---|
| Cross-version replay | §3.3 — refused, and the refusal is load-bearing |
| Windowed frame-exact replay with the GPU in the loop | GPU readback is not in the determinism contract; `EK5` makes windowed playback a configuration, not a promise |
| Deterministic event-lane order in the kernel | a scheduler change with a real cost; pay it only if §5's oracle is red AND full-frame replay is wanted (owner question 4) |
| Entity-reference normalisation in the digest (running a component's remap fn into a scratch row and substituting referent `StableId`s) | the v2 shape that closes §1.5's stated hole; it needs a mapper the current `LoadEntityMap` type does not expose, so it is a kernel change rather than a digest change |
| Recording the RENDER path (draw calls, GPU timings) | a different feature with a different oracle |
| Compressing recordings | the format is append-only and the sections are blittable; compression is additive later |
| Network/lockstep determinism | not asked for; the engine has none, and every `lockstep` hit in `docs/` uses the word in an unrelated sense |

**NOT established:**

- **Whether the frame is replay-stable outside physics.** That is §5's whole point: it is measured at
  `V8`, per source, and one row is predicted red.
- **The per-frame cost of the digest** over a real document. Reported at `V8`, not gated.
- **Whether any `PlainOldBytes` component carries uninitialised padding** that would make the digest
  nondeterministic within a single run. This is the one source in §5.1 that would invalidate the
  ORACLE rather than reveal a divergence, so `V8` must check it FIRST — a digest that is not stable
  run-to-run in the same process cannot measure anything.
- **The recorder ring's right size.** Sized at arm time from a configured budget; the drop policy is
  correct either way (invalidate, not stall), but a ring that invalidates on a normal session is
  useless and `V8` reports the high-water mark.
- **Whether `image[0]` at record start is affordable for a large document** (it is one `save_world`;
  the cost scales with scene bytes, and the POB path is memcpy-bound by its own bench).
