# E1 — The editor/game boundary

**Part of the editor campaign design.** Index and v1 ladder: `EDITOR-DESIGN.md`. **Revision 4.**
**Revision 4** changed one gate and made one omission explicit: G-B (§3.4) was an oracle that could not fail and is now an entity-id set census, and §2 now states what each plugin set must CONTAIN, because `EditorPlugins` was missing the input plugin the editor cannot boot without.
Decisions `E1.1`–`E1.5`. Every code claim carries a `file:line` read at HEAD `98ede3e0` in
`D:/wt/threadpool`.

| § | decision |
|---|---|
| §1 | `E1.1` — process and World topology |
| §2 | `E1.2` — "the game is a mod for the editor" as a compile-time fact |
| §3 | `E1.3` — editor-only state that cannot reach the document |
| §4 | `E1.4` — the fault model |
| §5 | `E1.5` — addressing entities across a boundary |
| §6 | code hot reload — refused, with the reason |
| §7 | out of scope for `E1` |
| §8 | what this file did NOT establish |

---

## 1. Process topology — `E1.1`

**Decision. Edit runs in ONE process on ONE World. Play runs in a SECOND process** — the same
executable re-launched as `--play <document>` — talking to the editor over the `E3` transport.
**No in-process Play-in-Editor in v1.**

### 1.1 The three facts that decide it

Not taste. Three code-level facts, each read at this checkout.

1. **The host renders exactly one world.** `run_windowed` is installed through `App::set_runner`,
   whose type is `Box<dyn FnOnce(&mut App) -> AppExit>`
   (`crates/boyko_ecs/src/ecs/core/app/app.rs:536`) — structurally ONE App — and it inserts the
   Vulkan device as a **NonSend** resource of that one world
   (`crates/boyko_app/src/runner.rs:158,240`). NonSend resources are per-world and cannot be shared,
   and the frame loop reads and writes that one world at ~30 `app.world_mut()` sites. A second
   in-process World cannot be rendered without rewriting the frame loop; that is a rewrite of the
   most heavily wired code in the host, not an addition.
2. **The Vulkan device is a process singleton.** `boot_singleton` returns
   `VulkanError::SingletonAlreadyBooted` on a second boot, and `destroy_singleton` reclaims exactly
   once (`crates/boyko_rhi_vulkan/src/device.rs:767-903`). A CHILD process gets its own device for
   free; a second in-process World would have to share the first world's.
3. **`save_world` cannot exclude entities by identity.** `SaveOptions::include_filter` keys on
   `ComponentId` (`crates/boyko_serialize/src/save.rs:47-60`) and the save loop walks
   `world.archetype_master().iter_archetypes()` with no entity predicate anywhere (`:170-190`). A
   shared World would save the editor's own chrome into the player's level file, and the filter
   available cannot prevent it — editor entities carry ordinary `Transform`s. §3 turns that from a
   blocker into a per-COMPONENT rule, but only because the two worlds are separate processes and
   the editor's chrome is a small, enumerable set.

Three of the four mature engines surveyed also run play out of process: Godot spawns a child and
keeps a pid list (`editor/run/editor_run.h`, `List<ProcessID> pids`), Fyrox shells out to `cargo`
and says why ("ensures that the editor won't crash if your game does"), and Bevy's archived editor
prototype's stated default was in-process-with-no-inspection, with the process question still listed
as open when it was archived. The one engine that keeps play in-process (Unreal) pays with world
duplication, an `EWorldType` enum threaded through the runtime, a separate UI object model — and
still leaks statics across sessions, which its own community documents.

### 1.2 Alternatives, priced

| alternative | what it buys | what it costs | verdict |
|---|---|---|---|
| **ONE World with editor-only components** (Bevy's own multi-world research recommends this for editors, and `EnableTag` was built for it) | no IPC, no id resolution, shared memory, one device | `save_world`'s filter is per-component-type while editor entities carry ordinary `Transform`s; and `boyko_ui` has ONE global `UiViewport` resource with every `UiRoot` seeded from it (`crates/boyko_ui/src/resources.rs:19-37`), so editor UI and a played game's UI would share one layout pass, one hit-test space and one root enumeration | rejected on the second fact, which is independent of the first and which §3's per-component rule cannot touch |
| **TWO Worlds in one process** | reachable at the ECS layer and test-pinned (`crates/boyko_ecs/tests/multi_world.rs`), which puts this engine ahead of Bevy on an axis Bevy called years of work | the renderer cannot serve the second world (fact 1); hooks are process-global per component type and registering them panics if the type is already archetyped in another world; a `Schedule` panics `boyko-B9101` off its build world; and `Entity` is not world-tagged, so a crossed selection handle silently reads the wrong row — the aliasing is PINNED as documented behaviour with the first entity of two worlds asserted equal | rejected for v1; revisit only if the transport proves too slow |
| **Two processes** (chosen) | crash isolation for free; a second device for free; `save_world` needs no entity predicate; the editor's UI owns its viewport alone | two Vulkan devices (VRAM unmeasured — `V7` records it); every editor↔game interaction becomes a message rather than a pointer; the editor's remote inspector is READ-ONLY in v1 | **taken** |

### 1.3 What the two processes exchange

The play child is a `boyko_remote` server exactly like the editor. The editor is its client. The
vocabulary is the command registry's, not a second protocol: `session.ping`, `world.query`,
`entity.get`, `log.tail`, `world.hash`. `play.start` spawns the child with `--play <doc>
--remote-port 0` and reads the bound port from the child's first line on stdout; `play.stop` sends
`app.exit` and escalates (§4.6). The editor never writes into the child's world in v1.

### 1.4 Out of scope here, recorded

- **Window embedding** (Godot 4.4 reparents the child's OS window into the editor's, keeping the
  processes separate; Windows 10/11 and X11, macOS "will require a different approach"). This buys
  the whole UX of in-process play with none of the risk and is the natural v2 answer. Owner
  question 1.
- **In-process PIE.** Needs a world-parametric frame loop plus the process-global hazards above.
- **A per-root `UiViewport`** (Godot's `SubViewport`) — the general fix for editor and game UI in
  one World; unnecessary once play is a child.

---

## 2. "The game is a mod for the editor" as a compile-time fact — `E1.2`

**Decision. One executable, THREE plugin sets:** `EnginePlugins` (exists,
`crates/boyko_app/src/plugins.rs:96,374`) + `GameTypes` + one of `{EditorPlugins, GameSystems}`,
selected by a launch flag. `EditorPlugins` lives in a NEW crate `boyko_editor` behind an opt-in
cargo feature `editor`. **No engine crate depends on `boyko_editor`.**

```
main() --editor <doc>  ->  App + EnginePlugins + GameTypes + EditorPlugins
main() --play   <doc>  ->  App + EnginePlugins + GameTypes + GameSystems + RemotePlugin
main()                 ->  App + EnginePlugins + GameTypes + GameSystems     (the shipped game)
```

**What each set must contain, made explicit in revision 4** — because the interesting failure of
this composition is not what it runs but what it silently omits.

| set | owns | note |
|---|---|---|
| `EnginePlugins` | window, device, render, scene, log, diag | not generic; carries **no** input plugin (`crates/boyko_app/src/plugins.rs:96,374`) |
| `GameTypes` | component / event / bundle registration, asset types, `#[derive]`d types | no systems. Present in ALL THREE launches so a document's component ids resolve identically in editor, play and shipped game |
| `GameSystems` | the game's logic, **and `InputPlugin<GameAction>`** | the `Actionlike` enum is the game's, and `register_action_names` is write-once, so this is the only set that may hold one |
| `EditorPlugins` | `RawInputPlugin` (`EK17`), `UiRuntimePlugin` (`EK8`), `UiInputPlugin`, the command tables, `Selection`, the panels, `FramePanicPolicy::PoisonAndReport` | **no `Actionlike` at all.** `RawInputPlugin` is what gives the editor a cursor without one — see `EDITOR-UI.md` §6 |

The `RawInputPlugin` row is the one revision 4 added, and it is not tidiness: without it the editor
App has no `PhysicalInput` resource, `ui_focus_system` panics on its first run
(`crates/boyko_ui/src/interaction/focus.rs:145-151`), and nothing ever calls
`RawInputQueue::begin_frame`, so the ring fills and trips its own `debug_assert` on the first
eviction. The composition was right and its contents were unstated, which is the same defect one
level down.

The owner's framing becomes a compile-time fact rather than a slogan. Splitting a game plugin into
**TYPES** (component registration, derives, asset types) and **SYSTEMS** (logic) is what Godot
achieves by never instantiating a non-`@tool` script and what Fyrox achieves by loading the plugin
for its types while explicitly not calling `Plugin::init`. Expressed as two plugin values it is
stronger than either: the editor App **cannot** run game logic because the systems were never
added — no per-type opt-in flag, no `@tool` escape hatch, and therefore none of the danger class
Godot's own documentation records ("the editor does not include protections for potential misuse of
`@tool` scripts"; "modifications in the editor are permanent, with no undo/redo possible").

The one-way dependency rule (`boyko_editor` → engine, never the reverse) is the single structural
decision every surveyed project that got it right shares — Godot's `editor/ → scene/ → servers/ →
core/`, Jackdaw's `jackdaw_runtime` / `jackdaw_editor` split, and this repository's own
`docs/REFLECTION-ANALYSIS.md` §1 rule that no shipping crate depends on `boyko_reflect`. Cargo
enforces it better than `#ifdef TOOLS_ENABLED` does.

### 2.1 Feature shape: opt-in, not default-on

`editor` is opt-in, matching the `hwrt` precedent (`crates/boyko_app/Cargo.toml:44-48`: default OFF,
and a `not(hwrt)` build is TEXTUALLY the pre-feature code). A default feature cannot be turned off
from inside this workspace, and the editor's whole promise is that the shipped game binary carries
none of it.

### 2.2 The gate that keeps the split honest — `EK9`

A schedule census in the editor App: enumerate every registered system's defining crate and assert
that no system comes from a game crate. This is the mechanical form of "the editor cannot run game
logic", and it is checkable because plugin registration is explicit. It goes red the moment someone
adds `GameSystems` to the editor set "just to see the physics move".

### 2.3 Alternatives, priced

| alternative | price | verdict |
|---|---|---|
| A separate editor binary linking the game as a library | forfeits "the editor is the game with a different plugin set" — the owner's actual framing — and doubles the boot path | rejected |
| A `@tool`-style per-component edit-time opt-in | imports Godot's documented danger class; the rejection of the split-scripts alternative in Godot's proposal #4306 does not apply here, because plugin composition costs the author one extra plugin VALUE rather than writing the logic twice | rejected |
| O3DE's `BuildGameEntity` (an editor component constructs the runtime component at bake) | the cleanest published answer, and NOT rejected — **deferred**, because it is exactly what the Gaia text path becomes when `G2` lands. v1's binary path needs an exclusion, not a transform | deferred |

---

## 3. Editor-only state that cannot reach the document — `E1.3`

**Decision, in three parts.**

- **v1's per-document-entity annotation set is EMPTY.** Selection lives in an editor-owned
  `Selection` resource keyed by `StableId`; lock and hide are cut from v1. Nothing the editor does
  attaches a component to a document entity.
- **Editor-OWNED entities** (gizmos, panel roots, the chrome tree) carry `#[component(no_serialize)]`
  **and** `#[require(EditorOnly)]`, and are excluded by `EK3`'s two arms.
- **The RULE for a future annotation is written and gated**, because someone will want one:
  `#[component(no_serialize)]` **plus `storage = "dense"`**, and **never** `#[require(EditorOnly)]`.

### 3.1 Why v1 has no annotations, and why that is a design change rather than a dodge

Revision 2 put `Selected`, `LockedInEditor`, `HiddenInEditor` and `GizmoTarget` on document entities
as "Class A" annotations. Three consequences, all avoidable:

1. **Archetype fragmentation.** A Table-storage component attaches by MIGRATING the entity into a
   new archetype. `save_world` emits one block per archetype with that archetype's own entity-id
   array (`crates/boyko_serialize/src/save.rs:85-92,170-175`), so selecting one entity changes the
   file's block sequence and its intra-block ordering even though no datum is lost. That made the
   `V4` gate's byte-identity half unsatisfiable (§3.4).
2. **Cost.** Selecting 1000 entities migrates 1000 rows, twice (select, deselect).
3. **Risk.** Every annotation is one forgotten attribute away from being written into the player's
   document, and one "for symmetry" `#[require]` away from DELETING the annotated entity from it.

`Selection` as a resource removes all three. It is a `Resource`-owned dense array of `StableId`s —
Principle 0 permits a `Resource`-owned column, and this is one — with a sorted index for membership.
`GizmoTarget` is derived from `Selection` each frame rather than stored. Lock and hide are cut
because, on inspection, they are properties a user expects to PERSIST across sessions, which makes
them document data rather than editor data, and v1 does not need them.

### 3.2 The rule when an annotation IS needed, and why `dense`

A future annotation on a document entity carries `#[component(no_serialize)]` **and**
`storage = "dense"`. Three properties, each read at this checkout:

- **A dense component is signature-excluded**, so attaching it does not migrate the entity's
  archetype — the saver's own comment states it: "A dense component is signature-excluded, so its
  slot→entity association is the store's `s2e`" (`save.rs:277-280`).
- **`no_serialize` classifies it `Ignore`, and BOTH save passes skip an `Ignore` component** — the
  archetype pass at `save.rs:186-193` and the dense pass at `:286-296`. Double exclusion, on two
  independent paths.
- **It stays queryable.** Dense `With<C>` / `Without<C>` have a real arm in the query filter (Dense
  plan D3: `crates/boyko_ecs/src/ecs/core/iters/query/filter.rs:143-152,451-463`), so a system can
  still ask "which entities are annotated".

**Never `#[require(EditorOnly)]` on such a component.** `#[require]` keys are expanded at
archetype-expansion time and enter the signature
(`crates/boyko_ecs/src/ecs/core/component/component.rs:312-328`), which is exactly what `EK3`'s
archetype arm reads — so annotating a document entity would EXCLUDE it from the file. That is
silent data loss: no error, no log, and the editor's own inspector still showing the entity, because
it is alive in the world and merely absent from the file. Revision 2 shipped that rule; revision 3
forbids it and gate G-C pins which names may carry the require.

### 3.3 `EK3` — the exclusion, in two arms

`save_world` gains an `EditorOnly` exclusion:

- **Archetype arm.** An archetype whose signature contains `EditorOnly`'s `ComponentId` emits no
  block. This is where Class B (editor-owned entities) is excluded, and `#[require(EditorOnly)]` is
  what puts the id in the signature.
- **Dense arm.** The dense pass is keyed by ENTITY, not by archetype — "a dense store's slot→entity
  map is NOT the archetype entity list" (`save.rs:277-283`) — so the dense arm resolves each live
  member's archetype and skips an excluded one. Without it, an editor entity holding any dense
  component has its dense row written with its id, and the loader materialises a member for an entity
  the file never otherwise mentions.

The mechanism is opt-OUT (an attribute a developer can omit), not structural like Godot's `owner`.
That is why gate G-C exists, and it is stated here rather than claimed away.

### 3.4 The three gates `EK3` must pass, and what each one counts

Revision 2's gate was a save→select→save byte compare. It is dropped, for two independent reasons:
it is unsatisfiable under a Table-storage annotation (§3.1), and — the more important one — **a
save-vs-save byte compare is blind in the direction that matters**, because both sides come from the
same saver and both omit the same thing. That is the `.ui` lesson restated: `serialize → parse →
serialize` is byte-identical when both sides are equally lossy. The replacement splits the two
properties the old gate conflated.

| gate | property | oracle | red-first |
|---|---|---|---|
| **G-A** | **nothing is LOST** | Open the fixture document, spawn editor chrome, run a selection, `document.save`; then load the saved file into a FRESH world and compare it to the FIXTURE with `EK16`'s order-independent, `StableId`-keyed digest — the entity SET, each entity's component set, and each component's bytes. Order-independent by construction, so archetype creation order, row order and id remap cannot make it red for a non-reason | remove one document entity from the saver's output and G-A must go red naming the missing `StableId` |
| **G-B** | **nothing EXTRA is written** | An **entity-id SET census over the saved FILE** (revision 4): read every archetype block's entity-row array and every dense block's `s2e` array out of the file, union them, and require set equality with the document's entity set. An editor-owned id present ⇒ red; a document id absent ⇒ red. Reads the file's own tables, so order is irrelevant | the fixture includes an editor entity carrying a **dense** component; G-B must FAIL before `EK3`'s dense arm exists. Without that fixture the arm is untested |
| **G-C** | **the convention is a mechanism** | A source census over `crates/boyko_editor/**`: every derived component carries `#[component(no_serialize)]`; `#[require(EditorOnly)]` appears on Class-B names only; and (v1) the Class-A list is empty | add a derived component without the attribute and G-C must go red |

Each gate REPORTS its counts (entities compared, components compared, file entity rows, census
sites), so a gate that inspected nothing cannot read as a pass.

**Why G-B is not the gate revision 3 wrote, stated so nobody restores it.** Revision 3's G-B read
the file's TYPE TABLE and required that no `boyko_editor` component's `stable_name` appear in it.
That gate cannot fail. An editor component carries `#[component(no_serialize)]`, which classifies it
`Serializability::Ignore`, and `save_world`'s column loop `continue`s on `Ignore` **before** it calls
`intern_type` (`crates/boyko_serialize/src/save.rs:174-186`) — so the name is absent from the type
table with `EK3` present, absent with `EK3` removed, and absent in a build where `EK3` was never
written. Meanwhile the thing `EK3` actually prevents — a gizmo entity's row and its perfectly
serializable `Transform` column reaching the player's level file — was not observed by the old oracle
at all. The general rule this corpus adopts from revision 4: **write down, per gate, the change that
would make it red.** For revision 3's G-B there was no such change, and that is discoverable in one
sitting by anyone who asks.

---

## 4. The fault model — `E1.4`

Seven rules. Rules 1–3 are barriers that keep game code out of the edit process; rules 4–6 say what
happens when something fails anyway; rule 7 protects the mechanism rules 4 rests on.

### 4.1 Rule 1 — no game system runs in the edit process

By composition (`E1.2`): `EditorPlugins` is added instead of `GameSystems`, so game logic is not
registered and cannot run. Gated by the schedule census (`EK9`).

### 4.2 Rules 2 and 3 — the clock, and the census this owes

**Rule 2. The edit World's virtual clock is PAUSED at boot.** `Time::pause` makes subsequent frames
see a zero virtual delta "and therefore zero fixed substeps", and accumulates no backlog
(`crates/boyko_ecs/src/ecs/core/time/time.rs:91-106`), so even a mis-composed plugin set cannot tick
fixed game logic. It is a second barrier, independent of rule 1, and it is what makes "nothing
simulates while editing" a checkable statement (`Time::is_paused()`) rather than an inference from
the plugin list.

**Rule 3. Every edit-time INTERACTIVE system reads REAL time**, `Time::real_delta()`
(`time.rs:81-84`), never `delta_secs()`.

Rule 2 without rule 3 is a defect, and it is worth writing down exactly why, because revision 2
shipped rule 2 alone. `fly_camera_system` computes `let dt = time.delta_secs();` — the VIRTUAL delta,
whose own doc says "ZERO while paused" (`crates/boyko_scene/src/camera.rs:841-843,879-888`). Under
rule 2 the editor's viewport camera does not move when the user presses W. No error, no log, and no
failing gate, because the `V5` gate drives selection and inspector edits and never touches the
camera. That is this repository's own named failure class — the mechanism exists but not on the path
being ruled about — recurring inside the file that catalogues it.

**The census rule 3 owes.** Outside `boyko_ecs` itself, exactly **two** systems in the workspace
consume `Res<Time>`:

| system | site | at zero virtual delta | disposition |
|---|---|---|---|
| `fly_camera_system` | `crates/boyko_scene/src/camera.rs:880` | the camera does not move | **replaced.** The editor does NOT add `FlyCameraPlugin`; it adds `editor_camera_system` in `boyko_editor`, which calls the identical pure `fly_step` (`camera.rs:895`, `pub(crate)` today — `EK13` makes it `pub`) with `dt = time.real_delta().as_secs_f32()`. Same math, same `Mut<Transform>` change-detection pin, different clock |
| `particle_system` | `crates/boyko_render/src/particle_system.rs:308` | particles do not advance | **kept frozen.** An editor showing a static scene is the intended behaviour, and it is what "nothing simulates while editing" means. Raised as owner question 7 because it is a VALUE, not an architecture fork |

Any system `boyko_editor` adds that integrates over time reads `real_delta` by the same rule; a
census over `crates/boyko_editor/**` for `delta_secs()` joins `EK9`.

`V5`'s camera gate is the red-first proof: an `input.key` command holding `W` for ten frames must
move the viewport camera while `Time::is_paused()` is true. It is red before `EK13` and green after.

### 4.3 Rule 4 — a panic POISONS the document; it never continues

`EK14`: `run_windowed` wraps its frame update —
`catch_unwind(AssertUnwindSafe(|| app.update_with_delta(dt)))` at
`crates/boyko_app/src/runner.rs:1233` — under a `FramePanicPolicy` resource:

- `Propagate` (the DEFAULT): today's behaviour exactly, so the shipped game binary is textually
  unchanged.
- `PoisonAndReport` (inserted by `EditorPlugins`): autosave the document to `<doc>.recovery`, log
  `E1908 DocumentPoisoned` with the recovery path, run the ordinary D2 teardown (window destroyed,
  device destroyed), exit non-zero. **Never continue.**

Revision 2 wrapped only the command handler. The widening matters for two reasons. First, a panic in
a *scheduled system* does not reach the command handler at all — and it DOES reach the frame
boundary, which was checked rather than assumed: the executor runs concurrent systems through
`pool.install(|scope| …)` and `scope.spawn`
(`crates/boyko_ecs/src/ecs/core/schedule/schedule.rs:28,433,672-676`), i.e. the scope-spawn
discipline, whose panics are caught in the task and re-raised on the JOINING thread — the dispatcher,
which is the thread inside `App::update_with_delta`. The schedule itself has no catch: the source
says so in as many words, "There is NO schedule-level `catch_unwind` around [the] `apply` call"
(`schedule.rs:756`). So `EK14`'s wrap is the first and only catch a system panic meets. (The
fire-and-forget `ThreadPool::spawn` discipline, which aborts, is a different path and is stated
below.) Second, this repository has RECORDED that a worker panic under the windowed runner does not
kill the process cleanly and leaves a window that "does not respond"; catching at the frame boundary
and running the ordinary teardown destroys the window instead of abandoning it mid-unwind, so the
recorded mode is removed rather than tolerated.

**Why poison rather than continue.** A mid-frame unwind through `Commands::apply` can leave a
half-migrated archetype. The world state after a partial apply is not characterised, and "keep
going" is how an editor writes a corrupt file. Godot's instance of the alternative is its own
documentation saying the editor has no protection against `@tool` misuse.

**The abort path that remains, stated.** A fire-and-forget `ThreadPool::spawn` task panic calls
`std::process::abort()` by design (rayon's policy, chosen so an unwind does not permanently shrink
the pool), and `catch_unwind` cannot catch it. The honest statement is that the edit process contains
one abort path and it is not reachable from the editor's own code; the `boyko_editor` census (`EK9`)
is where a hook that spawns a pool task at edit time would be caught.

### 4.4 Rule 5 — device loss is not catchable; the loss is BOUNDED

A terminal device error at the frame fence wait or at render calls
`report_terminal_device_error`, which logs `E3003 "terminal device error at {} ({}) - exiting"` and
returns from the frame loop (`crates/boyko_app/src/diag.rs:150-163`;
`crates/boyko_app/src/runner.rs:1252,2710`). It runs the ordinary teardown, but the unsaved document
is gone.

There is no recovery to design here, only a bound. The editor autosaves on a wall-clock cadence
(default 60 s) **and** after every mutating command group, so the loss window is bounded by the
smaller of the two; `E3003`'s message is extended to print the recovery path. Stating this as a
bound rather than as a remedy is the point: an editor that claims to survive device loss and does not
is worse than one that says how much it can lose.

### 4.5 Rule 6 — a hang has no edit-process remedy; the play link has one

`catch_unwind` cannot catch an infinite loop, and a command handler runs on the same thread as the
frame. **The edit process does not defend against a hang in v1**, and the mitigation is rule 5's
autosave cadence plus the fact that handlers are bounded-work by construction (they do not loop over
unbounded external input). This is stated as a gap, not covered by a claim.

The PLAY process is different, because it is a child. `play.start` records the child's pid; the
editor requires a `session.ping` response within a heartbeat interval (default 2 s, three misses);
`play.stop` sends `app.exit` and, if the child has not exited within a timeout (default 2 s),
terminates it and reports `E1907 PlayProcessExited` with the reason `Killed`. `V7`'s gate includes a
fixture game system that HANGS, and the escalation must kill it within the stated timeout.

### 4.6 Rule 7 — `boyko_editor` refuses to build under `panic = "abort"`

`#[cfg(panic = "abort")] compile_error!("boyko_editor's recovery model requires panic = unwind")`.
Rule 4 is only a mechanism while the build unwinds. The workspace declares `[profile.bench]` and no
`panic =` key anywhere (`Cargo.toml:71-72`), so unwind is in force today — and a future
`panic = "abort"` for size or speed, plausible for an engine with this constitution, would silently
void the whole fault model. One line, and it is the same shape as the engine's other build-enforced
invariants.

---

## 5. Addressing entities across a boundary — `E1.5`

**Decision. Entities are addressed by a STABLE ID, never by `Entity`**, in the undo stack, the
transport and the play link. The reverse index is `GK-1` — Gaia's cross-load map — of which the
editor is the first consumer. If `G6` has not landed when `V2` needs it, `V2` lands the map **AS
`GK-1` in `boyko_ecs`**, never as an editor-local map.

`Entity` fails as an external name three separate ways:

1. **It is not world-tagged.** A handle crossed between worlds silently resolves to the local world's
   row at the same slot, pinned as documented Bevy-parity behaviour with the first entity of two
   worlds asserted EQUAL (`crates/boyko_ecs/tests/multi_world.rs`, item 7). Two processes make this
   moot for play, and it would return the moment anything in-process held a cross-world handle.
2. **It does not survive open/save.** `load_world` allocates fresh ids under
   `LoadEntityPolicy::Remap`, the only variant (`crates/boyko_serialize/src/load.rs:69-78`).
3. **It does not survive undo.** An undo of a despawn cannot restore the same `Entity`.

Independently, Anthropic's own tool-design guidance finds that cryptic identifiers cause
hallucination and that semantic or indexed ids "significantly improve precision" — and a generational
index is exactly the cryptic shape.

**Why route through `GK-1` rather than mint an editor map.** An editor-local `HashMap<StableId,
Entity>` is forbidden by the hot-path rules and by Principle 0, and it would be a second id namespace
beside Gaia's. `GK-1` carries six requirements that `F5`'s ruling moved into `G6` (a global key
rather than a file-local id; a legal insert after `finalize`; removal plus a reverse edge; a
world-owned lifetime; lifting the fresh-world contract whose guard is a `debug_assert!` that vanishes
in release; a second `LoadEntityPolicy` variant). **The editor adds none of them — it consumes the
map**, and if it must land the map it lands it under `GK-1`'s name and shape.

---

## 6. Code hot reload — refused, with the reason

Not deferred. **Refused**, and the reason is a language fact this engine cannot route around: the
Rust Reference guarantees only alignment and non-overlap under `repr(Rust)`, states that field
ordering need not match declaration order, and that "type layout can be changed with each
compilation". Every hot-reload scheme therefore forbids changing the layout of a type shared across
the boundary — which is what an ECS component IS.

This engine's position is worse than average on three counts: component ids and archetype signatures
are minted into PROCESS-GLOBAL registries at first touch; storage is address-stable `VmReservation`
memory whose provenance is baked into query state; and the hot path is full of fn-ptr tables
(`CLONE`, `MAP_ENTITIES`, `SERIALIZE`, `HOOKS`) holding pointers INTO the code a dylib unload would
invalidate — exactly Fyrox's documented dangling-vtable problem, one level deeper.

The field agrees: Bevy deprecated and removed `bevy_dynamic_plugin` (unstable `TypeId` across
compilation units, no stable ABI, no guaranteed vtable layout); Fyrox ships CHR and calls it "a very
new and experimental feature … based on wildly unsafe functionality which could result in memory
corruption"; subsecond (and therefore Bevy 0.17's `hotpatching`) explicitly does not hot-reload
structs and tracks only the tip crate — so the edits an engine developer actually wants are the
excluded set.

**The reachable substitute is DATA hot reload**, which already ships for `.ui`
(`crates/boyko_ui/src/reload/`: an mtime+size poll, a keyed diff by `UiName`, transient runtime
components preserved by never being written, and a two-phase apply with a forced drain barrier whose
single-batch predecessor is documented as unsound) and which Gaia generalises.

---

## 7. Out of scope for `E1` (recorded)

| item | why not |
|---|---|
| Window embedding of the play child | owner question 1; Win32/X11 work after `V7` |
| In-process PIE | §1.2's third row |
| Live edit of the play world (Godot's `LiveEditor` mutation vocabulary) | the remote inspector is READ-ONLY in v1 |
| Making `Entity` world-tagged | the 8-byte handle is a deliberate trade; not this campaign's to reopen |
| Recovering from a hang in the edit process | §4.5, stated as a gap |
| O3DE's `BuildGameEntity` shape | deferred to the Gaia text path (§2.3) |

---

## 8. What this file did NOT establish

- The VRAM cost of two Vulkan devices. Unmeasured; `V7`'s gate records it.
- Whether `EK14`'s `catch_unwind` is reachable for every panic inside `update_with_delta`. The
  SCHEDULED-SYSTEM path was checked and is covered (§4.3: `scope.spawn`, re-raise on the dispatcher,
  no schedule-level catch). What was NOT checked is whether any engine plugin reaches
  `ThreadPool::spawn` (fire-and-forget, which aborts) from a system body on the edit path; `V4` must
  run that census before rule 4 is claimed as total, and if such a site exists the file must name it
  rather than widen the claim.
- Whether the world left behind by a caught unwind is safe to AUTOSAVE. Rule 4 saves to
  `<doc>.recovery` from a state that is by definition not characterised, so the recovery file may
  itself be corrupt. It is written under a distinct extension and never over the document precisely
  because of that; whether `save_world` can panic a second time on a half-migrated archetype is
  unestablished, and `V4` should wrap the recovery save in its own catch and degrade to "no recovery
  file, coded" rather than loop.
- Whether `Selection`-as-a-resource scales to a large multi-select (it is a sorted `StableId` array;
  the cost is a binary search per membership test, unmeasured).
- Whether the `boyko_editor` schedule census can see systems registered by a plugin's transitive
  plugin adds. If plugin composition hides the defining crate, `EK9`'s schedule arm needs a different
  key than "defining crate", and `V4` must find that out rather than assume it.
