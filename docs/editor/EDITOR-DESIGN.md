# Editor campaign — the design (index)

**Status:** design, **revision 4**, 2026-09-03, branch `feat/threadpool-ke16`, HEAD `59009f8a`,
worktree `D:/wt/threadpool`. **This pass writes no code.** Revision 4 carries five blocking
corrections, and the shape of them is worth stating before the design: **two were gates that could
not fail, one was a gate that could not pass, one was a finding the tree had already fixed, and one
was a capability the editor needs and no rung provided.** Two of the five (`EK1`, `U6`/`EK17`) were
found by re-opening citations this corpus had inherited rather than read; two (`V2`'s oracle,
`V4`'s G-B) were found by asking of each gate the one question this repository has learned to ask —
*what change would make it red?* — and getting no answer. The record is §12.1.

Every code-level claim carries a `file:line` read at this checkout unless it is marked *(survey)*,
in which case one of the six survey passes read it at HEAD `51d9a9bf` and the file and symbol are
given without a line. No build, test or timing was run: the machine was under
load for the whole pass and a number taken under load is an artifact, so none was taken.

**What this is.** The structure of a GUI editor written ON this engine, drivable by a machine agent
through the same surface its own buttons use, with a replay system for bug reproduction, at a v1
scope that exists and can be driven rather than one that is complete. The seven decisions the brief
asked for are `E1`–`E7`; each has a file. The v1 ladder is §3. Cross-references are by **decision
id** (`E2.4`, `U2`, `V3`, `EK9`) and **file**, never by a plan file's line number.

| File | Holds |
|---|---|
| `EDITOR-DESIGN.md` (this file) | the problem (§1), the decision table (§2), the v1 ladder (§3), the kernel items (§4), out of scope (§5), owner questions (§6), stale doc comments (§7), risks (§8), what was verified and what was not (§9), where this disagrees with the brief (§10), the corpus link census (§11), the revision record (§12) |
| `EDITOR-BOUNDARY.md` | **E1** — process and World topology, the plugin-set composition that makes "the game is a mod for the editor" a compile-time fact, editor-only state that cannot reach the document, the fault model, cross-boundary entity naming |
| `EDITOR-COMMANDS.md` | **E2** + **E5** — the command registry, declaration, typed arguments, the execution point, results and errors, discovery, completeness, the UI→command carrier, and undo as the same design |
| `EDITOR-TRANSPORT.md` | **E3** — the wire, the owner's CLI hypothesis priced, the agent surface and the tool-count problem, observation, the safety posture |
| `EDITOR-REPLAY.md` | **E4** — the replay family, the gate, the build stamp, non-perturbation, sufficiency, the world digest, and what replay obliges the engine to promise |
| `EDITOR-DOCUMENT.md` | **E6** — what the editor edits, the persistent form, and where Gaia's territory begins |
| `EDITOR-UI.md` | **U1**–**U5** — the substrate: the production draw path that has never existed, the v1 widget set, the panel layout, chrome authoring, and how a machine addresses a widget |

---

## 1. The problem, in one paragraph

The owner wants a GUI editor **written on this engine as if it were a game** ("the editor is a mod
for making a game, or the game is a mod for the editor"), **callable by a neural network** in every
function, with a **replay system** for reproducing a bug an agent can then analyse, at **v1 = basic
functionality**. The engine is unusually well placed for three of those and unusually badly placed
for one. Well placed: multi-world exists and is test-pinned (`crates/boyko_ecs/tests/multi_world.rs`),
`boyko_serialize` saves and loads a whole world, `boyko_input` has exactly ONE seam every input
source passes through (`crates/boyko_input/src/raw/queue.rs:76`), `App::update_with_delta(raw)` takes
the frame delta as a parameter (`crates/boyko_ecs/src/ecs/core/app/app.rs:660`), and `boyko_log`
already enforces a machine-readable diagnostic vocabulary at build time
(`crates/boyko_log/src/codes.rs:1-40`). Badly placed: **`boyko_ui` has never been drawn by the
shipping windowed loop** — `boyko_app` declares no `boyko-ui` dependency
(`crates/boyko_app/Cargo.toml:17-42`), `boyko_render`'s edge to it is a `[dev-dependencies]` entry
annotated "GUI P6b screenshot test ONLY … Acyclic + TEST-ONLY"
(`crates/boyko_render/Cargo.toml:78,107-113`), the only function taking `ui: Option<&UiPass<'_>>` is
`Renderer::present_sampled` (`crates/boyko_rhi_vulkan/src/present/frame_driver.rs:696,706`) and its
only two callers in the whole tree are tests
(`crates/boyko_render/tests/ui_rect_swapchain_golden.rs:639`,
`crates/boyko_rhi_vulkan/tests/window_present_hybrid.rs:978`), while `run_windowed` calls
`render_gbuffer_frame` (`frame_driver.rs:821`), which has no `ui` parameter. **And the plugin that
would install the UI does not exist**: `UiPlugin::build` returns early without a `.ui` path and
otherwise registers exactly one system, `ui_hot_reload_system`
(`crates/boyko_ui/src/plugin.rs:84-127`); nothing anywhere in `crates/*/src` registers
`ui_layout_discovery` or `ui_layout_apply` — the only registration in the tree is a test harness
(`crates/boyko_ui/tests/common/mod.rs:96-105`), and the crate documents the ordering as *the host's
responsibility* (`crates/boyko_ui/src/text/measure.rs:27-28`, `crates/boyko_ui/src/lib.rs:19-30`).
**And there is a third gap, found in revision 4: the editor as specified would have no input at
all.** `RawInputQueue` and `PhysicalInput` are inserted, and the only system that drains one into the
other is registered, by `InputPlugin<A>` alone — generic over the GAME's action enum
(`crates/boyko_input/src/plugin.rs:110-140`, `crates/boyko_input/src/action/process.rs:51-77`).
`EnginePlugins` is not generic and does not carry it (`crates/boyko_app/src/plugins.rs:96,374`). An
editor App that adds no game systems therefore has no `PhysicalInput` resource — so `ui_focus_system`
panics on `world.resource::<PhysicalInput>()` at its first run
(`crates/boyko_ui/src/interaction/focus.rs:145-151`) — and no `RawInputQueue::begin_frame`, so the
1024-slot ring fills and trips its own `debug_assert` on the first eviction. `EK17` splits the
non-generic half out. So the editor's most load-bearing assumption is not one gap but **three**, and
rung `V0` exists to close them before anything is designed on top of them.

---

## 2. Decision table

Every row is decided here; the alternatives and their prices are in the named file. Nothing in this
table is an owner question — §6 holds only VALUES and SCOPE.

| id | question | decision | file |
|---|---|---|---|
| **E1.1** | one process or two; one World or two | **Edit = one process, one World. Play = a SECOND process** (the same executable re-launched `--play <doc>`) talking to the editor over the `E3` transport. No in-process PIE in v1 | `EDITOR-BOUNDARY.md` §1 |
| **E1.2** | what "the editor is a mod for the game" means concretely | **One executable, THREE plugin sets**: `EnginePlugins` + `GameTypes` + one of `{EditorPlugins, GameSystems}`, chosen by a launch flag. `EditorPlugins` lives in a new crate `boyko_editor` behind an opt-in cargo feature `editor`; **no engine crate depends on `boyko_editor`** | `EDITOR-BOUNDARY.md` §2 |
| **E1.3** | editor-only state that must not reach the document | **v1's per-document-entity annotation set is EMPTY** — selection is an editor-owned `Selection` resource keyed by `StableId`. Editor-OWNED entities (gizmos, panel roots) carry `#[component(no_serialize)]` + `#[require(EditorOnly)]` and are excluded by `EK3`'s two arms. The rule for a future annotation is written and gated: `no_serialize` + `storage = "dense"`, **never** `#[require(EditorOnly)]` | `EDITOR-BOUNDARY.md` §3 |
| **E1.4** | game code panics or hangs | **Seven rules**: no game system in the edit process; virtual clock paused; edit-time interaction reads REAL time; a frame-boundary `catch_unwind` under a policy resource that POISONS the document (autosave + coded refusal + orderly teardown) rather than continuing; device loss is uncatchable and the loss is bounded by autosave cadence; a hang has no edit-process remedy and the play link has a heartbeat plus kill escalation; `boyko_editor` refuses to build under `panic = "abort"` | `EDITOR-BOUNDARY.md` §4 |
| **E1.5** | naming an entity across a process boundary and across undo | **`StableId`, never `Entity`**, in the undo stack, the transport and the play link. The reverse index is Gaia's `GK-1`, of which the editor is the first consumer; if `G6` has not landed, `V2` lands the map **as `GK-1` in `boyko_ecs`**, never as an editor-local map | `EDITOR-BOUNDARY.md` §5 |
| **E2.1** | is the editor's UI a client of the command registry, or is automation a separate surface | **The registry IS the architecture.** The editor's own UI and the machine agent are both clients of one registry; a UI system may not mutate the world directly (`EK9` census) | `EDITOR-COMMANDS.md` §1 |
| **E2.2** | how a command is declared and registered | `#[command]` on a fn emits a `CommandDesc` static; each crate lists its commands in ONE `command_table!`; a plugin's `build` registers the table | `EDITOR-COMMANDS.md` §2 |
| **E2.3** | how arguments and results are typed | **`FieldTable`, a kernel type this campaign lands (`EK4`)**, emitted by `#[derive(CommandArgs)]` for a command's `#[repr(C)]` argument and result structs. The schema and the decoder come from ONE derive invocation per struct, so they cannot drift. **No Gaia dependency** | `EDITOR-COMMANDS.md` §3 |
| **E2.4** | where a command executes | ONE point: the exclusive system `apply_commands(world: &mut EcsMaster)` at the head of `CoreSchedule::Main`, draining a fixed-capacity `CommandInbox`. Overflow is REFUSAL, never drop | `EDITOR-COMMANDS.md` §4 |
| **E2.5** | results and errors to a non-human caller | Typed result structs; failures carry a code from the `boyko_log` registry (a new `E19xx` series — verified free, §9) plus a typed detail, with MCP's protocol-error / operation-error split | `EDITOR-COMMANDS.md` §5 |
| **E2.6** | discovery | The registry enumerated, plus a committed `docs/editor/COMMANDS.json` regenerated and compared by a test. Argument AND result schemas are non-empty for every command | `EDITOR-COMMANDS.md` §6 |
| **E2.7** | how the registry is kept COMPLETE | A TWO-LAYER gate: a source census over every `#[command]` site (in exactly one `command_table!`, registered by a plugin), and the manifest regenerate-and-compare. A **coverage** census, never an equality check — this repository has measured the equality shape going green over loss | `EDITOR-COMMANDS.md` §7 |
| **E2.8** | how a button invokes a command | **`OnCommand`** — a `#[repr(C)] Copy` component naming a `CommandId` with a declared argument PROVENANCE — dispatched by `ui_command_dispatch`, a system NOT generic over `Actionlike`. `ui.click(name)` materialises the identical envelope, so the human path and the agent path are the same call by construction. **Revision 4:** the provenance vocabulary is CLOSED and layer-clean — a provenance that needs editor state resolves through a kernel-owned provider fn-ptr table (`ArgKind::Provider(ProviderId)`), never by naming an editor type inside `boyko_ui` | `EDITOR-COMMANDS.md` §9 |
| **E2.9** | **(new, revision 4)** how a foreign thread reaches the inbox without a lock and without aliasing the world | The transport owns a heap-allocated SPSC ring built at plugin build and split into halves: the producer moves into the listener thread, the consumer is the world's `CommandInbox`. **The listener DECODES** — JSON in, `#[repr(C)]` argument bytes out, validated against the command's `FieldTable` — so the exclusive window pays a memcpy, never a parse | `EDITOR-COMMANDS.md` §4.2 |
| **E3.1** | the wire | JSON-RPC 2.0 over a loopback socket in a new crate `boyko_remote` behind an opt-in feature `remote`; the JSON codec is in-house (`EK11`, bounded) | `EDITOR-TRANSPORT.md` §1 |
| **E3.2** | the owner's CLI hypothesis | **Right about the CATALOGUE, wrong as the architecture.** `boyko-ctl` SHIPS, generated from the registry, as an adapter | `EDITOR-TRANSPORT.md` §2 |
| **E3.3** | the agent-facing surface | An MCP bridge exposing **≤ 25 curated tools** plus `describe_commands(query)` and `invoke(name, args)` over the full registry | `EDITOR-TRANSPORT.md` §3 |
| **E3.4** | how the agent OBSERVES | A first-class read half: `world.query`, `entity.get`, `ui.tree`, `screenshot`, `log.tail`, `world.hash` | `EDITOR-TRANSPORT.md` §4 |
| **E4.1** | the replay family | Input log + initial world image + per-frame `dt` + the command log + a per-frame **world digest**, with periodic world images every K frames (default 600) | `EDITOR-REPLAY.md` §1 |
| **E4.2** | how replay is gated | **HYBRID**: compiled in behind cargo feature `replay` (default off), ARMED at runtime | `EDITOR-REPLAY.md` §2 |
| **E4.3** | what identifies a build | An FNV-1a-128 hash of the executable image + `session_id` + the bytes of every determinism-affecting resource + the format version. A mismatch is a coded refusal naming the differing field. **Cross-version replay is refused** | `EDITOR-REPLAY.md` §3 |
| **E4.4** | does recording perturb what it records | It may not allocate, read a clock, or block on the frame path; overflow INVALIDATES the recording rather than stalling. **Non-perturbation is `V8`'s first gate** | `EDITOR-REPLAY.md` §4 |
| **E4.5** | are the three determinism properties sufficient | **Measured, not assumed**, per source, with event-lane order predicted RED **in advance** | `EDITOR-REPLAY.md` §5 |
| **E4.6** | what makes a recording analysable | The command log joined to the frame index, the diagnostic-code stream, and periodic images so a divergence is inspectable without re-simulating from zero | `EDITOR-REPLAY.md` §6 |
| **E4.7** | standing | **Under cargo feature `replay`, within-build bit-exact determinism of the simulation is a PRODUCT REQUIREMENT, not a testing convenience** | `EDITOR-REPLAY.md` §7 |
| **E5.1** | undo | Undo IS the command layer: every mutating command captures its INVERSE at execute time | `EDITOR-COMMANDS.md` §8 |
| **E5.2** | play-mode edits | Discarded on stop in v1 (the Unity and Unreal default); no write-back | `EDITOR-COMMANDS.md` §8.3 |
| **E6.1** | what the editor edits | **The document IS the edit World.** Its persistent form in v1 is the `boyko_serialize` binary written by `save_world` with the `EK3` exclusion | `EDITOR-DOCUMENT.md` §1 |
| **E6.2** | the Gaia boundary | Gaia owns grammar, baker, ids, composition and the `ui` profile. The editor mints no format, no id namespace, no grammar; it files requirements against the rungs that own them | `EDITOR-DOCUMENT.md` §2 |
| **U1** | what must be proved first | The production window loop must draw one `boyko_ui` panel with validation ON, **and the gate must be shown capable of going RED**. Rung `V0` | `EDITOR-UI.md` §1 |
| **U2** | the v1 widget set | Three new widgets only: an editable text field, scrolling, a tree | `EDITOR-UI.md` §2 |
| **U3** | the panel layout | ONE fixed layout built from the shipped units; the viewport is a HOLE, not a render target | `EDITOR-UI.md` §3 |
| **U4** | how chrome is authored | Rust systems in `boyko_editor`, **not** `.ui` documents | `EDITOR-UI.md` §4 |
| **U5** | how a machine addresses a widget | By `UiName`; geometry from `ui.tree`; activation through `ui.click(name)` → `OnCommand` (`E2.8`). **Never** by synthesising a pointer event at a coordinate | `EDITOR-UI.md` §5 |
| **U6** | **(new, revision 4)** how the editor gets input at all, given that the only queue drain is generic over the game's action enum | **`EK17` splits `boyko_input`'s non-generic half out**: `RawInputPlugin` inserts `RawInputQueue` + `PhysicalInput` and registers `ingest_physical_input` (`begin_frame` + drain + fold); `InputPlugin<A>` keeps the `ActionState<A>` half and is unchanged in behaviour and order. The editor adds `RawInputPlugin` and **no `Actionlike` at all** | `EDITOR-UI.md` §6 |

---

## 3. The v1 ladder

"Basic" is defined as a ladder whose every rung can be DEMONSTRATED. A rung that cannot be
demonstrated is not a rung. Each gate says what it counts, and where a gate could pass vacuously the
red-first calibration that proves it can fail is part of the rung.

| rung | delivers | depends on | gate |
|---|---|---|---|
| **V0** | **The UI reaches the screen.** `EK8`: (a) a new **`UiRuntimePlugin`** in `boyko_ui` registering the DRAW pipeline in the crate's own documented order — `UiViewport` + `LayoutScratch` + `FontTable` resources, `ui_bar_discovery`/`ui_bar_apply` → `ui_text_measure_system` → `ui_layout_discovery` → `ui_layout_apply` → `ui_bind_discovery`/`ui_bind_apply`. **Revision 4 splits the interaction half out**: `ui_focus_system` reads `Res<PhysicalInput>` and panics if it is absent (`focus.rs:145-151`), so it belongs with `EK17`'s input half in a second plugin, `UiInputPlugin`, at `V5` — `V0` proves DRAW and only draw, and a rung whose deliverable panics at boot in its own harness is not a rung; (b) host seeding of `UiViewport` on surface create AND resize; (c) a `ui: Option<&UiPass>` parameter threaded into `render_gbuffer_frame` (or a UI sub-pass after the composite); (d) an ECS→ring upload system over the existing `pack_ui_instance`; (e) the production `boyko_app → boyko-ui` dependency edge | nothing — it is first because it proves the campaign's most load-bearing assumption | A windowed run with validation ON dumps a frame showing a panel with a text label over the scene, compared to a golden AND the validation-message count asserted zero **and printed**. **Red-first calibration, load-bearing:** remove the UI pass's barrier and the run MUST go red; if it does not, the gate is declared non-measuring and the fallback is a `vkCmdPipelineBarrier` census over the recorded command stream. A second calibration: delete `ui_layout_apply` from the plugin and the golden MUST change — a pipeline that never laid out cannot pass. The UI pass's frame time is REPORTED, not gated (workstation, not a bench rig) |
| **V1** | **The command kernel.** `EK2` (`#[command]`, `CommandDesc`, `command_table!`, `App::register_commands`, `CommandInbox`, the exclusive `apply_commands`, `CommandError`, the `E19xx` series, the source census, the generated manifest) + **`EK4`** (`FieldTable`, `#[derive(CommandArgs)]`, `#[derive(Fields)]`) + `EK1` (**revision 4: narrowed to a DENSE arm for `get_component_changed_tick`** — the write half was already in the tree, §12.1 item 1). First commands: `session.ping`, `world.query`, `entity.spawn`, `entity.despawn`, `component.get`, `component.set` (whole value), `component.get_field`, `component.set_field` | nothing (headless-testable; `V0` only for the editor's own dispatch later) | Headless: a fixture world driven only through `apply_commands` spawns, moves and despawns; every count reported. **`EK1` red-first, rewritten in revision 4 because the old one was unsatisfiable:** `component.set` on a table-storage `Transform` already propagates at this checkout, so the old calibration could never go red. The real hole is the READ twin: write a **dense** component through `component.set`, then `get_component_changed_tick` on it — `None` before `EK1`'s dense arm (it resolves through `archetype_ptr()` → `pools.get_pool(id)?`, and a dense component is signature-excluded), `Some(current_tick)` after. A table component's tick is asserted in the same test as the control, so the test states both arms rather than one. The census goes red on a `#[command]` site absent from every `command_table!`; the manifest regenerate-and-compare goes red on a hand-edited manifest; `#[derive(CommandArgs)]` on a struct with a field of an undescribable type must fail to compile |
| **V2** | **Undo.** `StableId` + the `GK-1` reverse index (landed AS `GK-1` in `boyko_ecs` if `G6` has not landed), inverse capture on every `V1` mutating command, the per-document bounded undo ring (4096 entries / 64 MiB arena), groups, and `undo`/`redo`/`group.begin`/`group.end` | `V1` (and it now carries **`EK16`**, moved here from `V4`) | A proptest: a random sequence of N `V1` commands over a fixture world, then N undos, yields a world whose **`EK16` digest** equals the start's; then N redos reproduce the post-command digest. **Revision 4 replaced a byte-identity oracle that was a deterministic FALSE RED** (§12.1 item 2): `save_world` emits a block for EVERY archetype including empty ones and interns each column's type before any count is consulted, and no production caller of `remove_archetype` exists — so a spawn that mints a new archetype changes the file's block sequence and type table permanently, even after the undo empties it; and an undo of a despawn cannot restore the same `Entity`, which `E1.5` says itself. Neither is a defect and both are unconditional. Ring-bound behaviour asserted explicitly: the oldest entry is dropped and `undo` past the bound is a coded refusal, never a silent no-op. Counts reported |
| **V3** | **Transport.** `boyko_remote` behind feature `remote`, the in-house JSON codec (`EK11`), the socket listener thread feeding the inbox through an SPSC ring, `rpc.discover` carrying argument AND result schemas from the `FieldTable`s, and `boyko-ctl` generated from the registry | `V1` | `boyko-ctl invoke entity.spawn '{…}'` against a headless `run_n_with_delta` host returns a typed result; `rpc.discover`'s parameter arrays are non-empty for **every** command (the explicit inverse of Bevy's measured defect) and the test asserts one schema entry per declared argument, counted. A `not(remote)` build contains no socket code (symbol census) |
| **V4** | **The editor binary and the document.** Crate `boyko_editor`, feature `editor`, the `--editor <doc>` flag, `EnginePlugins + GameTypes + EditorPlugins`, `Time::pause` at boot, the `Selection` resource, `EditorOnly` + `EK3`'s archetype AND dense arms, `document.open`/`document.save`, the autosave cadence, and `EK14` (frame-boundary panic policy). `EK16` now lands at `V2` and `V4` consumes it | `V0`, `V2` | The three gates of `EDITOR-BOUNDARY.md` §3.4, each red-first and each counting. **G-A (nothing lost):** open a fixture, spawn editor chrome, select entities, save, reload into a fresh world and compare with `EK16`'s digest — the entity SET, each entity's component set and each component's bytes must match the fixture. **G-B (nothing extra), rewritten in revision 4 because the old oracle could not fail:** an editor component carries `#[component(no_serialize)]` ⇒ `Serializability::Ignore` ⇒ the column loop `continue`s BEFORE `intern_type` (`save.rs:174-186`), so its `stable_name` never enters the type table whether or not `EK3` exists — a type-table census passes with `EK3` entirely absent. G-B is now an **entity-id SET census**: the set of ids in the file (every archetype block's entity-row array ∪ every dense block's `s2e` array) must equal the document entity set EXACTLY — an editor-owned id anywhere is red, a missing document id is red, and order is irrelevant. Fixture includes an editor entity with a **dense** component, and G-B must fail before `EK3`'s dense arm exists. **G-C:** the source census over `crates/boyko_editor/**`. Plus the schedule census (zero game-crate systems in the editor App) and the `panic = "abort"` build refusal |
| **V5** | **Panels.** **`EK17`** (`RawInputPlugin`) and `UiInputPlugin` (`ui_focus_system` + `ui_command_dispatch`) — without them the editor has no cursor and its raw ring debug-panics; the text field, scroll and tree widgets; **`EK15`** (`OnCommand`, `ArgSource`, the provider table) and **`EK12`** (`pending_submit` carries its origin); the hierarchy panel; the inspector generated from `EK4`'s `FieldTable`s; the viewport with **`EK13`**'s editor camera on real time; selection via the existing world pick; `ui.tree`, `ui.click(name)`, `ui.type(name, text)`, `selection.set` | `V2`, `V4` | Driven entirely through the command layer, never through synthetic pointer events: `ui.click("hierarchy/entity-42")` → `selection.get` returns the id → `ui.tree` shows the inspector with `field_count == the FieldTable length` for the opted-in fixture component; `ui.type("inspector/Transform.translation.x", "3.5")` → `component.get_field` reads 3.5 → the next frame's `GlobalTransform` has moved (the `EK1` chain end to end). **Camera gate (from `EK13`), with revision 4's cause-isolation:** an `input.key` command holding `W` for 10 frames must move the viewport camera while `Time::is_paused()` is true. The gate asserts `PhysicalInput.keys_held` contains `W` FIRST — because without `EK17` the camera also fails to move, and a gate that reds for the wrong reason teaches the wrong fix. Then: red before `EK13`, green after. The 10-frame span is deliberate: `ingest_physical_input` and `apply_commands` are both on `Main` with no ordering edge between them, so an injected event may be observed this frame or the next, and the gate may not assume which. A name resolving to nothing, or to a node with no `OnCommand`, is a coded refusal and the test asserts the code. Every count reported |
| **V6** | **The agent surface.** The MCP bridge (`boyko-ctl mcp`, stdio ↔ socket), the curated ≤ 25 tool set, `describe_commands(query)`, `invoke(name, args)`, `screenshot` (`EK10`), `log.tail`, `world.hash` | `V3`, `V5` | `tools/list` returns ≤ 25 tools and every curated name resolves to a registry command (a name resolving to nothing is red). A scripted MCP session spawns an entity, moves it, takes `screenshot{scale: 0.5}` before and after, decodes both with `boyko_image`, and asserts the pixel at the projected position changed while the mean of the rest did not — counts reported, so a uniformly-changed frame cannot pass |
| **V7** | **Play.** The `--play <doc>` child process (`GameSystems` + `remote`), `play.start`/`play.stop`/`play.pause`, the heartbeat and kill escalation, a READ-ONLY remote inspector panel that is a client of the child's socket, and a coded error on child exit | `V3`, `V4` | `play.start` → the child answers `session.ping` within 5 s and `world.query` over its socket returns the document's entity count; `play.stop` → the process exits within 2 s. A fixture game system that panics on frame 10 kills the child while the editor reports `E1907 PlayProcessExited` with the exit code and its OWN document survives — proved by re-running `V4`'s G-A digest after the crash. A fixture game system that HANGS must be killed by the escalation within the stated timeout. Peak VRAM of both processes is RECORDED (the two-device cost is unmeasured today) |
| **V8** | **Replay.** Feature `replay`, the `push_raw` tee (`EK6`) and the injectable frame clock (`EK5`), the preallocated recorder ring plus drain, the build-identity stamp, `replay.record`/`replay.stop`/`replay.load`/`replay.step`/`replay.seek`, headless playback through `run_n_with_delta`, per-frame `EK16` digests with divergence detection, and periodic world images | `V7` (recording during play), `V2` (the command log is the undo log's twin, and `EK16` lands there) | **Non-perturbation FIRST, and it is the gate that matters:** the per-frame digest sequence of a recorded run is IDENTICAL to the unrecorded run of the same input — a red here means recording changes what it records, the worst failure this feature has. Then: record a play session, replay it headlessly, and the digest sequence matches frame for frame; a header mismatch is a coded refusal naming the differing field; and the sufficiency oracle (W=1 vs W=N over a recording) reports PER SOURCE, with event-lane order **expected RED** and reported as a confirmed finding rather than hidden. A `not(replay)` build contains no recorder (symbol census) |

**No rung waits on the Gaia campaign.** Revision 2 claimed `V5` alone did; revision 3's `EK4`
removes the dependency entirely (§12 item 4). The Gaia relationship is a coordination requirement
(`EK7`), not a blocker.

---

## 4. Kernel and cross-crate items this campaign lands (the `EK` series)

Items outside `boyko_editor`. Each names the crate that owns it, so an implementer does not
re-derive it inside the editor and so a sibling plan can see it is claimed.

| id | crate | what | rung |
|---|---|---|---|
| **EK1** | `boyko_ecs` | **REWRITTEN in revision 4, and it shrank.** A DENSE arm for `get_component_changed_tick`. The write twin this row used to demand does not exist because it is not needed: `set_component_raw` stamps the changed tick on **both** arms at this checkout, the dense arm inside `DenseStore::insert_or_replace` and the table arm through `ComponentPool::write_changed_tick`, with the doc naming the old defect in the past tense — "The table arm did NOT stamp before this was fixed" — and pointing at its gate, `crates/boyko_ecs/tests/change_tick_on_raw_write.rs` (`crates/boyko_ecs/src/ecs/core/ecs_master/component_api.rs:439-452`). The fix is commit `0f944f51` (2026-07-10), an ancestor of the survey's own HEAD `51d9a9bf`. What REMAINS is one-directional: `get_component_changed_tick` resolves through `inland.archetype_ptr()` → `pools.get_pool(component_id)?` (`:350-355`), and a dense component is signature-excluded from the archetype, so it returns `None` for every dense component — an inspector gating a row refresh on it shows a dense component as never changing | `V1` |
| **EK2** | `boyko_ecs` | the command kernel: `#[command]`, `CommandDesc`, `command_table!`, `App::register_commands`, `CommandInbox`, the exclusive `apply_commands`, `CommandError`, the `E19xx` code series | `V1` |
| **EK3** | `boyko_serialize` | the editor exclusion in `save_world`, in TWO arms: an archetype-signature arm and a DENSE arm. The dense pass is keyed by entity, not by archetype (`crates/boyko_serialize/src/save.rs:277-283`), so a one-arm exclusion leaks every editor entity holding a dense component | `V4` |
| **EK4** | `boyko_ecs` + `boyko_macros` | **`FieldTable`** — `{name, byte offset, scalar kind or nested table, is_entity_ref, declared default}` — plus `#[derive(CommandArgs)]` (command argument/result structs) and the opt-in `#[derive(Fields)]` (components, the `#[derive(Bindable)]` precedent at `crates/boyko_ui/src/binding/bindable.rs`). One TYPE, two emitters, each attached to the declaration it describes | `V1` |
| **EK5** | `boyko_app` | an injectable frame clock in `run_windowed` — today `dt = Instant::now() - last` is computed inline with no injection point (`crates/boyko_app/src/runner.rs:1223-1233`) | `V8` |
| **EK6** | `boyko_input` | the `push_raw` record/replay tee — a single tee on `RawInputQueue::push_raw` (`crates/boyko_input/src/raw/queue.rs:76`), whose own comment anticipates it (`crates/boyko_input/src/raw/event.rs:14-15`) | `V8` |
| **EK7** | *(requirement filed, not built)* | **Gaia `G1` coordination**: when `GK-4` lands the component field tables, it emits **`EK4`'s `FieldTable`** — extending it with the disposition column `GB-5` gates — rather than minting a second descriptor. Filed NOW rather than discovered at merge | — |
| **EK8** | `boyko_ui` + `boyko_render` + `boyko_app` | **`UiRuntimePlugin`** — the plugin that installs the UI DRAW pipeline, which does not exist today — plus host viewport seeding and the production UI draw path. Shared with Gaia `G7`'s prerequisite. Revision 4 moved `ui_focus_system` out of it and into `UiInputPlugin` (`V5`), because it hard-requires a resource `EK17` provides | `V0` |
| **EK9** | `boyko_editor` | the census gates: no `set_component_raw` / `Commands::insert` / `get_component_mut` outside `commands/`; every derived component carries `#[component(no_serialize)]`; `#[require(EditorOnly)]` appears on Class B names only; zero game-crate systems in the editor App | `V4` |
| **EK10** | `boyko_image` | an in-house PNG **stored-block** encoder (the crate is a decoder today) plus a box downscale | `V6` |
| **EK11** | `boyko_remote` | the in-house JSON codec, **bounded in writing**: values, strings with escapes, numbers as `f64`/`i64`, no streaming, no pretty printing beyond one indent | `V3` |
| **EK12** | `boyko_ui` | `UiPointerState::pending_submit` becomes `Option<(Entity, u16)>`, mirroring `click_fired`'s shape, and is stamped on ANY Enter-with-focus. **Behaviour-preserving for the existing consumer**: `ui_dispatch_system` skips `NO_ACTION` either way (`crates/boyko_ui/src/interaction/dispatch.rs:54-59`) | `V5` |
| **EK13** | `boyko_scene` | `camera::fly_step` becomes `pub` (it is `pub(crate)` today, `crates/boyko_scene/src/camera.rs:895`) so `boyko_editor` can drive the identical math from REAL time | `V5` |
| **EK14** | `boyko_app` | a frame-boundary `catch_unwind` in `run_windowed` around `app.update_with_delta(dt)` (`runner.rs:1233`), selected by a `FramePanicPolicy` resource whose default is `Propagate` — today's behaviour, so the shipped game is textually unchanged | `V4` |
| **EK15** | `boyko_ui` (+ the provider table in `boyko_ecs`) | `OnCommand` + `ArgSource` + `ui_command_dispatch` — the UI→command carrier `E2.1`'s thesis needs and the crate does not have. **Revision 4 removed a layering violation:** revision 3's `ArgKind::Selection` named an editor-owned resource from inside `boyko_ui`, which cannot depend on `boyko_editor` under `E1.2`'s one-way rule. The variant becomes `ArgKind::Provider(ProviderId)` over a fixed fn-ptr table `fn(&EcsMaster, &mut ArgWriter) -> Result<(), ErrorCode>` owned by the command kernel and filled by `App::register_arg_provider` at plugin build — the same cold fn-ptr-table shape the engine already uses for `CLONE` / `SERIALIZE` / `HOOKS`. `boyko_editor` registers the selection provider; `boyko_ui` never learns the name | `V5` |
| **EK16** | `boyko_ecs` | the order-independent, `StableId`-keyed world digest, behind feature `replay` for the per-frame use and available unconditionally to tests. **Moved to `V2` in revision 4**, where it becomes the undo oracle as well; `V4`'s document gate and `V8`'s replay oracles consume it. Its component enumeration must copy `save_world`'s two habits or it inherits two known traps: filter `archetype.component_ids()` by `component_pools().get_pool(id).is_some()` (the signature slice is not the pool set), and walk dense members through `dense_registry().dense_ids()` in a second pass keyed by entity | `V2` |
| **EK17** | `boyko_input` | **NEW in revision 4, and the editor cannot boot without it.** Split `InputPlugin<A>` in two around a shared `fn install_raw_input(app)`: **`RawInputPlugin`** (non-generic — inserts `RawInputQueue::with_capacity(RAW_QUEUE_CAP)` and `PhysicalInput`, registers `ingest_physical_input` = `queue.begin_frame()` + `physical.begin_frame()` + the drain loop) and `InputPlugin<A>` (unchanged surface: adds `ActionState<A>` + `InputMap<A>` + `register_action_names::<A>()` + `process_action_state::<A>` ordered after the ingest and after `clear_consumed_fixed_edges::<A>`). Behaviour-preserving for every game: the same systems, in the same order, doing the same work per frame — the split is where the code lives, not what it does. Today all of it is one generic system, `update_action_state<A>` (`crates/boyko_input/src/action/process.rs:51-77`), registered only by `InputPlugin<A>` (`crates/boyko_input/src/plugin.rs:110-140`) | `V5` (and `V0` may need it earlier — see `EDITOR-UI.md` §6.3) |

---

## 5. Out of scope (recorded, not built here)

| item | why not this pass | where recorded |
|---|---|---|
| Window embedding of the play process (Godot 4.4's Win32/X11 reparenting) | buys "the game appears inside the editor" without the in-process risk; it is Win32/X11 work after `V7`, not a structural change | `EDITOR-BOUNDARY.md` §1.4; owner question 1 |
| In-process Play-in-Editor (a second App rendered by the same host) | needs a world-parametric frame loop plus the process-global hazards (hooks, `EVER_ARCHETYPED`, the device singleton); Unreal does this and still leaks statics across sessions | `EDITOR-BOUNDARY.md` §1.4 |
| Live edit of the play world; write-back of play-mode edits | the remote inspector is READ-ONLY in v1; write-back needs the per-field "editable" classification Unreal implements with UObject flags and nobody here has | `EDITOR-COMMANDS.md` §8.3 |
| Docking, splitters, drag-and-drop, context menus, dropdowns, sliders, checkboxes | L each; none gates a v1 demonstration | `EDITOR-UI.md` §2.3, §7 |
| Clipboard in the text field; multi-pointer; a saved panel layout | an OS call the engine makes nowhere; `MAX_POINTERS = 1` is a fixed array shaped for widening; a saved layout needs docking first | `EDITOR-UI.md` §8 |
| **Rebindable editor shortcuts** | `ACTION_NAMES` is a process-global, write-once, single-`Actionlike` table (`crates/boyko_input/src/action/names.rs:22-42`), so an editor vocabulary and a game vocabulary cannot coexist. `EK17` gives the editor a cursor without one; giving it REBINDABLE shortcuts needs a per-World action registry, which is a kernel change to a process-global. v1 editor keys are direct `PhysicalInput` reads, which is what `ui_focus_system` already does for Tab and Enter | `EDITOR-UI.md` §6.4 |
| The Gaia TEXT save path and the id→text-range manifest consumer | waits on Gaia `G2`–`G3`; the editor's command names and argument tables do not change across that transition | `EDITOR-DOCUMENT.md` §2.3 |
| Per-root `UiViewport` / nested render targets (Godot's `SubViewport`) | the general fix for editor UI and game UI coexisting in one World; unnecessary once play is a child process | `EDITOR-UI.md` §8 |
| Windowed frame-exact replay with the GPU in the loop | GPU readback is not part of the determinism contract; v1 replay is headless with rendering as a courtesy | `EDITOR-REPLAY.md` §8 |
| Deterministic event-lane order in the kernel | a scheduler change with a real cost; pay it only if `E4.5`'s oracle is red AND the owner wants full-frame replay | `EDITOR-REPLAY.md` §5.1; owner question 4 |
| **Code hot reload of the game crate — REFUSED, not deferred** | every Rust mechanism forbids changing the layout of a type shared across the boundary, which is what a component IS; and this engine's fn-ptr tables (`CLONE`, `MAP_ENTITIES`, `SERIALIZE`, `HOOKS`) point into code a dylib unload would invalidate. Bevy removed `bevy_dynamic_plugin`; Fyrox ships CHR and calls it "wildly unsafe"; subsecond explicitly does not hot-reload structs. The reachable substitute is DATA hot reload, which already ships for `.ui` | `EDITOR-BOUNDARY.md` §6 |
| `boyko_reflect` | the design does not depend on it — `EK4`'s tables are the mechanism. If the reflection branch merges later, `TypeInfo` and `FieldTable` carry the same information and one must become a view of the other; that is a decision for the merge | §10 |
| A named-pipe / UDS transport; authentication beyond the per-session connect token; a real DEFLATE compressor behind the PNG stored-block encoder | scope | `EDITOR-TRANSPORT.md` §6 |
| A `boyko_ui` widget-density benchmark | the machine is a workstation under load and a number taken now is an artifact; `V0` REPORTS the UI pass's frame time so `V5` has a baseline rather than a first measurement taken too late to act on | §9 |

---

## 6. Owner questions — VALUES and SCOPE only

Every architecture and performance fork is decided above. These eight are not.

1. **The play window is a SEPARATE OS window in v1** (`E1.1`); embedding the child's window inside
   the editor (Godot 4.4's route) is v2. Is a separate window acceptable for v1, or is "the game
   appears inside the editor" a v1 VALUE that moves embedding into the ladder as one rung of
   Win32/X11 work after `V7`?
2. **Edits made while playing are DISCARDED on stop** (`E5.2`) — the Unity and Unreal default.
   Acceptable for v1, or must v1 carry an opt-in per-entity "keep play changes" (Unreal's *Keep
   Simulation Changes*), which needs a per-field editable classification and therefore lands after
   `EK4` is extended?
3. **v1 documents are `boyko_serialize` BINARIES** until Gaia `G2` lands the text form (`E6.1`). Is
   a binary-only, non-diffable, non-reviewable document acceptable for v1? The alternative is making
   `V4` depend on an unstarted campaign whose language spec is itself marked ratified-stale.
4. **Replay sufficiency** (`E4.5`): the whole-frame oracle is EXPECTED RED on event-lane order,
   because two systems' events interleave per worker thread. If it is, v1 replay is bit-stable for
   physics and transforms and best-effort for gameplay events. Acceptable for v1, or must the kernel
   first make lane order deterministic — a scheduler change, priced but not started?
5. **Which scene is v1's demonstration DOCUMENT** — the demo's room scene, a physics fixture, or a
   new one? This is the fixture every `V4`–`V8` gate opens, so it is a scope call.
6. **Destructive commands issued by an AGENT** (`entity.despawn`, `document.save` over an existing
   file, `play.stop`) require an explicit `confirm: true` argument over the transport, and never when
   issued by the editor's own UI. MCP's spec asks for a human in the loop and marks server-side
   annotations untrusted. Confirm this posture, or name the commands that need no confirmation.
7. **(new, from revision 3)** The edit viewport is **FROZEN**: the virtual clock is paused, so
   particles do not play and nothing simulates while editing (`E1.4` rule 2, and the clock census in
   `EDITOR-BOUNDARY.md` §4.2 shows exactly two affected systems). The camera still moves, because
   `EK13` drives it from real time. Acceptable for v1, or is a "preview" toggle that unpauses the
   virtual clock a v1 VALUE? It is one command and one resource write, so the cost is small; the
   question is whether a running preview inside an editor with no game systems is meaningful.
8. **(new, from revision 4)** **The editor's keys are FIXED in v1** — no rebinding, no `.keys`
   override file, no shortcut editor. `EK17` gives the editor a cursor and keyboard without an
   `Actionlike`, and it deliberately does not give it one: `ACTION_NAMES` is process-global and
   write-once (`crates/boyko_input/src/action/names.rs:22-42`), so an editor enum and the game's
   enum cannot coexist, and whichever registers first would silently win in the `--play` child too.
   Editor key handling is therefore direct `PhysicalInput` reads inside editor systems — which is
   what `ui_focus_system` already does for Tab and Enter. Acceptable for v1, or is rebindable
   editor input a v1 VALUE? If it is, the work is a per-World action registry, i.e. a kernel change
   to a process-global table, and it lands before `V5` rather than after.

---

## 7. Doc comments the pass found stale (to correct in the same commit as the code they describe)

| where | what it says | what is true |
|---|---|---|
| `crates/boyko_serialize/src/lib.rs` (`# Phase S2 boundary`) | "S2 round-trips a world with NO cross-entity references. Entity fields load with their RAW saved ids; the saved→fresh remap (`map_entities_fn`) is deferred to S2.5" | S2.5 has landed — `crates/boyko_serialize/tests/entity_remap.rs` exists and is titled "Phase S2.5 — ENTITY-REMAP on load". The capability is present; the header is stale |
| `crates/boyko_ui/src/plugin.rs:35-58` (`UiPlugin` doc) | reads as *the* UI plugin | it is the `.ui` document loader and hot-reloader; it registers no layout, no measure and no hit-test. `EK8` adds `UiRuntimePlugin` beside it and this doc comment must say which is which, or the next reader repeats revision 2's mistake |
| `crates/boyko_render/src/ui/mod.rs:55` | describes the `LoadOp::Load` UI sub-pass in `present_sampled` as the path | true, and unreachable from the production host until `EK8`; the comment should name `render_gbuffer_frame` as the production caller once it takes the parameter |
| `docs/REFLECTION-PLAN-ECS.md` §S3 / note D26 — **not in this worktree; it lives on `feat/reflection`** | "`set_component_raw` does not bump the change tick on the TABLE path" | **False since `0f944f51` (2026-07-10)**, which is an ancestor of the survey's own HEAD. Both arms stamp, gated by `crates/boyko_ecs/tests/change_tick_on_raw_write.rs`. The mirror half of D26 — no dense arm on the read twin — is still true and is what `EK1` now is. This row is here because the stale claim reached revision 3 through two survey passes and a brief, and the correction has to be findable from the place that repeated it |
| `crates/boyko_input/src/plugin.rs:35-58` and `action/process.rs:25-50` (`update_action_state` doc) | read as *the* input ingest | it is the ingest **and** the per-`A` action fold, welded together and reachable only through `InputPlugin<A>`. Any consumer that wants a cursor but no action enum — the editor, a headless UI test, a tool — cannot have one. `EK17` splits them; both doc comments must then say which half is which |

---

## 8. Risks, each with the mitigation the ladder carries

| risk | why | mitigation |
|---|---|---|
| The UI sub-pass on the production frame has a barrier/layout hazard nobody has exercised, and the instrument that would catch it is blind | `sync-validation` is documented as not live — a deliberately removed barrier produces zero `SYNC-HAZARD` output and an unchanged golden. The obvious gate cannot see the defect class the rung exists to find | `V0` is FIRST, runs with validation on, asserts a printed message count, and carries TWO red-first calibrations (remove the barrier; delete `ui_layout_apply`). If neither reds, the gate is declared non-measuring and the fallback is a barrier census over the recorded command stream |
| `V0` is larger than it looks and slips, taking `V4`/`V5` with it | revision 2 sized it as "add the existing `UiPlugin`"; the plugin that installs the UI does not exist, `.ui` is refused for chrome by `U4`, and the ordering contract lives in prose (`measure.rs:27-28`) rather than in code | `EK8` is restated in §3 as five named deliverables including the plugin itself; the ordering contract is transcribed into `EDITOR-UI.md` §1.4 from the crate's own docs and the test harness that already encodes it |
| An editor component added after `V4` is silently written into the player's document | `#[component(no_serialize)]` is opt-OUT under the derive — the trait default is `Ignore` but `#[derive(Component)]` classifies `SerializeViaFn` for any `Clone` type (`crates/boyko_ecs/src/ecs/core/component/component.rs:161-168`) — and a forgotten attribute produces no error at all | Gate G-C, a source census over `crates/boyko_editor/**` — the same trade `boyko_log`'s `codes!` registry already makes. Without G-C, `EK3` is a convention rather than a mechanism, and that is stated in the file |
| A future Class A annotation is given `#[require(EditorOnly)]` "for symmetry", deleting annotated document entities from the saved file | `#[require]` keys enter the archetype signature (`component.rs:312-328`), which is what `EK3`'s archetype arm reads. It reads as tidier and is silent data loss | v1's Class A set is EMPTY, so the case does not arise; the rule is written and G-C pins which names may carry the require. The G-A oracle is a content digest against the FIXTURE, not a save-vs-save compare, so a lost entity is caught rather than mirrored |
| `EK3`'s archetype-signature exclusion leaks every editor entity holding a dense component | the saver gathers dense stores in a SEPARATE pass keyed by entity (`save.rs:277-283`) | `EK3` has two arms; gate G-B's fixture includes an editor entity with a dense component and must fail before the dense arm exists |
| `EK4` overlaps Gaia's `GK-4` and the two mint different descriptors | this repository has recorded "a sibling plan may already own your rung", and `GK-4` is Gaia `G1`'s | `EK4` lands ONE type with a shape `GB-5` does not touch (it omits the disposition column that ballot gates), and `EK7` files the requirement that `G1` emits it rather than minting a second. `#[derive(Fields)]` is opt-in per type, so no shipped game binary gains a table it did not ask for |
| Event-lane order makes full-frame replay red and the campaign quietly narrows the promise | events of two systems interleave per worker thread, and the temptation on a red is to redefine the oracle | `E4.5` measures per SOURCE with a row each and states in advance that event-lane order is EXPECTED red — a prediction on the record, so a red is a confirmed finding rather than a discovery to be reframed |
| The command census is bypassed by a UI system writing a component directly | it is one `set_component_raw` call away, and the moment one panel mutates the world outside the registry, the agent surface and the undo stack are both wrong with nothing saying so | `EK9`'s census forbids `set_component_raw`, `Commands::insert` and `get_component_mut` outside `commands/` — the same shape as `clippy.toml`'s disallowed-types gate |
| A future `panic = "abort"` release profile silently voids the fault model | `E1.4` rests on `catch_unwind`, which is only a mechanism while the build unwinds. The workspace declares only `[profile.bench]` and no `panic =` key today (`Cargo.toml:71-72`) | `V4` asserts the strategy — `#[cfg(panic = "abort")] compile_error!` in `boyko_editor` |
| A device-lost or a hang in the edit process destroys unsaved work | `report_terminal_device_error` logs `E3003` "- exiting" and the frame loop returns (`crates/boyko_app/src/diag.rs:150-163`, `crates/boyko_app/src/runner.rs:1252,2710`); neither is catchable, and `catch_unwind` cannot catch a hang | The loss is BOUNDED, not prevented: autosave on a wall-clock cadence AND after every mutating command group, with the recovery path printed by the coded exit. Stated as a bound, never as recovery |
| Two Vulkan devices under play exceed VRAM on the owner's machine | unmeasured and unmeasurable in this pass; it is the principal cost of `E1.1` | `V7`'s gate RECORDS peak VRAM of both processes; if the sum exceeds budget, play runs with the editor's viewport rendering paused via a `render.pause` command, which costs nothing to build |
| The agent floods the input ring and kills dev builds | `RAW_QUEUE_CAP = 1024` (`crates/boyko_input/src/constants.rs:8`) with drop-oldest and a debug panic on first eviction | the agent's vocabulary is `ui.click(name)` through the command layer, which never touches the ring (`U5`); the command inbox uses the opposite policy — refusal with a coded error, never drop |
| The in-house JSON codec grows into a project | JSON invites completeness and the no-third-party rule imposes no ceiling | `EK11` is bounded in writing; anything past the bound is a separate decision with its own justification |
| A finding survives its own fix, and a rung is built to close a hole that is already closed | measured here, not hypothetical: `EK1`'s write half was inherited from a plan document on an unmerged branch describing the tree as it was before 2026-07-10, restated by two survey passes and by the brief's own "three findings", and carried into revision 3 as a `V1` deliverable with a red-first calibration that could never go red. Nobody re-opened `component_api.rs`; the doc comment there says the defect is past tense and names its gate | every `EK` row now carries the line it was READ from at this checkout, not the line a survey reported. The general rule the campaign adopts: **a red-first calibration is a claim about today's tree and must be re-checked against today's tree at the rung that ships it** — if the calibration cannot be made to fail, the finding is gone, not the gate |
| The editor boots with no input, and the symptom names the wrong cause | `PhysicalInput` and `RawInputQueue` exist only if `InputPlugin<A>` was added, and `ui_focus_system` unwraps the first (`focus.rs:145-151`) while the ring's `begin_frame` lives inside the second's generic system. The editor's failure would be a panic at boot or a `debug_assert` after ~1024 mouse events — and if the panic were dodged, a camera that does not move, which `V5` would have read as an `EK13` failure | `EK17` splits the non-generic half out; `U6` states the editor's input path end to end; and `V5`'s camera gate asserts `PhysicalInput` observed the key BEFORE asserting the camera moved, so the two causes are distinguishable at the bench |
| A gate is written over the artefact that is easy to read rather than the property that matters | twice in one revision: `V2` compared save BYTES when it meant world CONTENT, and `V4`'s G-B named the type TABLE when it meant the entity SET. Both are the same mistake — reaching for the nearest observable — and one produced a gate that always fails while the other produced one that never can | the corpus rule from revision 4 on: **for every gate, write down the change that would make it red.** If that change is not a defect (`V2`) or does not exist (`V4` G-B), the gate is wrong before it is ever run. `EDITOR-BOUNDARY.md` §3.4 and `EDITOR-DOCUMENT.md` §4 now carry that line per gate |
| The design corpus itself rots | measured, not hypothetical: `EDITOR-UI.md` was referenced ten times across six files and did not exist for a full revision | the link census, §11, is a campaign requirement rather than a habit |

---

## 9. What was verified and what was not

**Verified at this checkout, by reading the file** (the load-bearing set; every other claim carries
its own citation in its file): the absence of a `boyko-ui` production edge
(`crates/boyko_app/Cargo.toml:17-42`) and its `[dev-dependencies]` position in `boyko_render`
(`Cargo.toml:78,113`); `present_sampled`'s `ui` parameter and its two test-only callers
(`frame_driver.rs:696,706,821`); `UiPlugin::build`'s actual registrations
(`crates/boyko_ui/src/plugin.rs:84-127`) and the absence of any layout registration outside the test
harness; the fly camera's virtual delta (`crates/boyko_scene/src/camera.rs:879-888`) against
`Time::pause`'s zero-delta contract (`crates/boyko_ecs/src/ecs/core/time/time.rs:91-97`); the
complete `Res<Time>` consumer census outside `boyko_ecs` (two systems); `click_fired`'s stamp of the
ORIGIN regardless of `OnClick` (`crates/boyko_ui/src/interaction/focus.rs:475-500`) and
`pending_submit`'s loss of it (`focus.rs:556-562`); `save_world`'s per-archetype `entity_ids_base`
blit (`crates/boyko_serialize/src/save.rs:85-92,170-175`) and its separate entity-keyed dense pass
(`:277-321`) against `LoadEntityPolicy::Remap` as the only load variant
(`crates/boyko_serialize/src/load.rs:69-78`); the dense pass's `Ignore` skip (`save.rs:286-296`);
dense `With`/`Without` support in the query filter
(`crates/boyko_ecs/src/ecs/core/iters/query/filter.rs:143-152,451-463`); `storage = "dense"` as a
derive attribute (`crates/boyko_macros/src/component.rs:797`); `MapEntitiesFn` /
`LoadMapEntitiesFn` signatures (`.../component_registry/clone.rs:55-58`,
`.../component_registry/serialize.rs:87-90`); the terminal device-error exit path
(`crates/boyko_app/src/diag.rs:150-163`, `runner.rs:1252,2710`) and the frame-update call site
(`runner.rs:1233`); the `E19xx` code range being unused (the used two-digit prefixes in
`crates/boyko_log/src/codes.rs` are `00 01 02 05 07 08 09 13 15 18 20 21 22 26 30 92`); the absence
of any Gaia crate (**27** crate directories under `crates/`, none of them Gaia or `boyko_reflect`);
and the component-derive count the tool-budget argument rests on (**188** `derive(… Component …)`
occurrences across 54 files under `crates/*/src`, counted rather than estimated).

**Verified in revision 4, by re-opening citations this corpus had inherited rather than read** —
each of these changed a decision or a gate:

- `set_component_raw` stamps the changed tick on **both** arms, and its own doc says the table-arm
  defect is past tense and names its gate (`component_api.rs:439-452`;
  `crates/boyko_ecs/tests/change_tick_on_raw_write.rs` exists). The fix `0f944f51` (2026-07-10) is an
  ancestor of `51d9a9bf`. `get_component_changed_tick` still has no dense arm — it resolves through
  `pools.get_pool(component_id)?` (`:350-355`), which a signature-excluded dense component fails.
- `save_world` iterates **every** archetype (`iter_archetypes` is a plain `self.archetypes.iter()`,
  `archetype_master.rs:815-817`) and pushes an `ArchetypePlan` unconditionally, and `intern_type` runs
  per column before any count is consulted (`save.rs:174-190,265-273`). `remove_archetype` exists
  (`archetype_master.rs:664`) but **has no production caller** — the only call sites in `crates/` are
  its own two unit tests. An emptied archetype is therefore permanent in the file.
- An `Ignore`-classified component is `continue`d BEFORE `intern_type` (`save.rs:174-186`), so a
  `no_serialize` editor component can never appear in a saved type table under any `EK3`.
- `update_action_state<A: Actionlike>` is the sole owner of `queue.begin_frame()` and of the
  `RawInputQueue` → `PhysicalInput` drain (`action/process.rs:51-77`); `InputPlugin<A>` is its sole
  registrar and the sole inserter of both resources (`plugin.rs:110-140`); `EnginePlugins` is not
  generic and carries neither (`crates/boyko_app/src/plugins.rs:96,374`); `ui_focus_system` unwraps
  `world.resource::<PhysicalInput>()` (`focus.rs:145-151`).
- The execution point is sound as specified: an exclusive system is `fn(world: &mut EcsMaster)`
  (`ui_layout_apply`, `crates/boyko_ui/src/layout.rs:152`), the scheduler runs it INLINE on the
  dispatcher under `running == 0` (`schedule.rs:1085-1140`), and — the part that mattered — it runs
  **outside** `InSystemRunGuard`, which is entered only inside `scope.spawn` for worker-dispatched
  systems (`schedule.rs:1296-1299`). So a handler may call `run_system_once`, whose
  `drain_deferred_hook_queue` `debug_assert!(!is_in_system_run())` (`ecs_master.rs:672-678`) would
  otherwise have panicked in every debug build.
- The `E19xx` range is free, re-counted at this revision: the four-digit `E` codes present in
  `crates/boyko_log/src/codes.rs` are `E0104 E0106 E0107 E0109 E0110 E0115 E0118 E0201 E0801 E2001
  E2101 E2103 E2203 E3001 E3002 E3003 E3004 E3010 E9204 E9213`.
- `Time::real_delta()` / `pause()` / `is_paused()` are public (`time.rs:81,95,110`), so `E1.4`'s
  rules 2–3 and `EK13` rest on shipped API; `camera::fly_step` is `pub(crate)`
  (`camera.rs:894`) and `fly_camera_system` reads `time.delta_secs()` (`camera.rs:884`).
- `dense_registry()` and `DenseRegistry::dense_ids()` are public
  (`ecs_master.rs:590`, `dense_registry.rs:156`), so `EK16`'s dense pass needs no new accessor.

**NOT established, so it is not silently assumed downstream:** whether `boyko_ui` sustains an
editor's node count at interactive rates; the VRAM cost of two Vulkan devices; whether the frame is
replay-stable outside physics; the cost of the ECS→ring UI upload at editor densities; the size of
the in-house JSON codec; the per-frame cost of `EK16`'s digest over a real document; whether
`feat/reflection` still compiles against today's `boyko_ecs`. Each is a measurement, each is assigned
to the rung that needs it, and each rung REPORTS its number rather than gating on a guess.

**Method note.** `graphify` is not installed in this worktree (`graphify: command not found`, and
`graphify-out/` does not exist; the PreToolUse hook's premise is stale here), so the fallback to
`Grep`/`Read` was taken once, as the rule permits, and not retried.

**On the HEAD move between revisions.** Revision 3 read at `98ede3e0`; revision 4 reads at
`59009f8a`. The two intervening commits are `98ede3e0` (a physics no-FMA census fix) and `59009f8a`
(docs only — `docs/RUST-*`), so no file this corpus cites changed between them and revision 3's
`file:line` citations remain valid where revision 4 did not replace them. That is checked, not
assumed: it is the same class of question as `EK1`, where a citation was inherited across a
seven-week-old fix and nobody looked.

---

## 10. Where this design disagrees with the brief

1. **"boyko_reflect: a reflection crate whose CORE IS CLOSED."** It is not in this workspace at all —
   there is no `crates/boyko_reflect`. On `feat/reflection` the CORE ladder is closed but the ECS
   GLUE ladder, which is where an editor lives, stops at enumeration (`EG1`); the read (`EG4`), write
   (`EG5`) and construct (`EG6`) rungs are unimplemented and the kernel seam all three stand on was
   `8cf5d886 docs(reflect): EG2 audited and REFUSED`. **The design does not stand on it.** `EK4`'s
   macro-emitted `FieldTable` is the mechanism, and it needs no reflection at build time or run time.
2. **"boyko_ui: an ECS-native UI, complete per its own plan."** Complete per its plan — and its plan
   never included being drawn by the production host, nor a plugin that installs it. Both gaps are
   `V0`.
3. **"The Gaia campaign: a DATA language."** Docs only: four `.md` files, no crate, no workspace
   member, `G0`–`G8` unstarted, `LANGUAGE.md` marked ratified-stale. The favourable half is that the
   FORMAT boundary is already ruled in this campaign's favour — Gaia's baker prints the existing
   `boyko_serialize` format rather than minting a second one.
4. **"`set_component_raw` does not bump the change tick on the table path… This is live on THIS
   branch."** It is not, and this is the brief's own finding (b) — one of the three it put in front of
   the architecture phase. Both arms stamp; the fix predates the survey's HEAD by seven weeks; the
   test that gates it is in the tree. The claim's provenance is a plan document on the unmerged
   `feat/reflection` branch, which is not even present in this worktree. **The editor's by-id write
   path therefore works today**, which is a small piece of good news and a large piece of method:
   nothing here should be believed because a survey said it, including this document. What survives
   of the finding is its mirror, the dense-blind read twin, and that is `EK1`.
5. **"The engine has NO replay."** Confirmed, and the brief's own correction is adopted in full:
   replay is opt-in at the build level, the obligation is WITHIN a build, and cross-version replay is
   refused explicitly. What the brief left open is answered in `EDITOR-REPLAY.md`: the build stamp is
   an exe-image hash rather than a commit hash; the recorder's non-perturbation is `V8`'s FIRST gate;
   sufficiency is measured per source rather than assumed; and the digest that makes all three
   checkable is `EK16`, which is not the `save_world` byte image (§12.2 item 5).

---

## 11. The corpus link census

Every `[...](FILE.md)` and every bare `EDITOR-*.md` reference in `docs/editor/` must resolve to a file
that exists. This is a campaign requirement, not a habit: `EDITOR-UI.md` was referenced ten times
across six files and did not exist for a whole revision. Run after every edit to this corpus; it is
one `grep` and one `test -f` per hit. Run at this revision: all seven `.md` references resolve.

**The one deliberate exception:** `docs/editor/COMMANDS.json` is referenced by `E2.6` and does not
exist yet — it is GENERATED at `V1` by the manifest test. The census covers `.md` files only, and
`COMMANDS.json`'s absence is a rung that has not run rather than a broken link. When `V1` lands, the
manifest test is itself the census for that file.

---

## 12. Revision record

### 12.1 Round 4 — five blocking corrections (this revision)

Each was verified by opening the file at this checkout, not by trusting the citation that named it.
Four of the five changed a gate; three changed a deliverable; one deleted work the ladder was going
to do twice.

| # | the defect, and how it was found | what changed |
|---|---|---|
| 1 | **`EK1`'s write half was a finding that had already been fixed.** Revision 3 declared `mark_component_changed` a `V1` deliverable and gave it a load-bearing red-first calibration — "a `component.set` on a table-storage `Transform` must NOT propagate before it exists". It does propagate: `set_component_raw` stamps both arms, its doc names the old defect in the PAST tense and cites `crates/boyko_ecs/tests/change_tick_on_raw_write.rs`, and the fix `0f944f51` (2026-07-10) precedes the survey's own HEAD by seven weeks. The claim came from `docs/REFLECTION-PLAN-ECS.md`, a plan on the unmerged `feat/reflection` branch that **does not exist in this worktree**. Found by asking what the calibration would actually do, and opening the function | `EK1` shrinks to the one hole that IS real — no dense arm on `get_component_changed_tick` (`component_api.rs:350-355`) — and `V1`'s calibration is rewritten around a dense component, with a table component asserted in the same test as the control. §7 gains the stale-doc row; §10 gains item 4, which refutes the brief's own finding (b) |
| 2 | **`V2`'s undo gate was a deterministic FALSE RED.** "N commands, then N undos, yields a world byte-identical to the start under `save_world` — legal here, no reload intervenes, so ids are preserved." Two independent reasons it fails with nothing wrong: `save_world` emits a block for EVERY archetype and interns every column's type before consulting a count (`save.rs:174-190,265-273`), while `remove_archetype` has **no production caller**, so a spawn that mints an archetype changes the file permanently even after the undo empties it; and an undo of a despawn cannot restore the same `Entity` — which `E1.5` states in its own reasoning. The justification contradicted the corpus one section over, where revision 3 had **already** removed byte-identity as an oracle for the same class of reason | `V2`'s oracle becomes `EK16`'s `StableId`-keyed digest, and **`EK16` moves from `V4` to `V2`**. One mechanism now serves undo, the document round trip and the replay oracle, which is also why it is worth building well |
| 3 | **`V4`'s G-B could not fail.** "The saved file's type table names no editor component's `stable_name`." An editor component carries `#[component(no_serialize)]` ⇒ `Serializability::Ignore` ⇒ the column loop `continue`s BEFORE `intern_type` (`save.rs:174-186`). Its name cannot reach the type table whether `EK3` exists or not — so the gate that was supposed to prove the exclusion passes with the exclusion entirely absent. What `EK3` actually prevents is an editor ENTITY's row and its serializable columns (a gizmo's `Transform`) being written, and no part of the old gate looked at that | G-B becomes an **entity-id SET census**: archetype blocks' entity rows ∪ dense blocks' `s2e`, compared for equality with the document entity set. Red on an editor id present, red on a document id missing, order-independent. `EDITOR-BOUNDARY.md` §3.4 and `EDITOR-DOCUMENT.md` §4 carry it |
| 4 | **The editor would have had no input at all, and no rung provided it.** `RawInputQueue` and `PhysicalInput` are inserted, and the only drain between them registered, by `InputPlugin<A>` — generic over the game's action enum (`plugin.rs:110-140`, `action/process.rs:51-77`). `EnginePlugins` carries neither. Revision 3 said the editor "does not register `UiInteractionPlugin<A>` at all, which is what removes the single-`Actionlike` constraint" — true, and it removes the input with it: `ui_focus_system` unwraps `Res<PhysicalInput>` and panics at boot, and without `begin_frame` the 1024-slot ring trips its own `debug_assert` after about a thousand mouse events. Found by asking who calls `begin_frame` HERE | **`EK17`** splits `boyko_input`'s non-generic half into `RawInputPlugin`, behaviour-preserving for every game. `EK8` sheds `ui_focus_system` into a second plugin `UiInputPlugin` so `V0` proves draw without needing input. New decision **`U6`** (`EDITOR-UI.md` §6) states the editor's input path end to end, and `V5`'s camera gate asserts the key was OBSERVED before asserting the camera moved |
| 5 | **`EK15`'s `ArgKind::Selection` was a layering violation.** `ArgSource` lives in `boyko_ui`, and `Selection` is a `boyko_editor` resource; `E1.2`'s one-way rule forbids the dependency that would make the variant compile, so the mechanism the design's own thesis rests on could not have been built as written | `ArgKind::Provider(ProviderId)` over a cold fn-ptr table owned by the command kernel and filled by `App::register_arg_provider` — the `CLONE` / `SERIALIZE` / `HOOKS` shape the engine already uses. Revision 4 also closes the adjacent hole revision 3 left open: **`E2.9`** states how a foreign thread reaches the inbox (an SPSC ring split at plugin build; the listener decodes, so the exclusive window never parses) |

**What revision 4 did NOT change, and why that is worth saying.** The seven decisions of `E1`–`E7`
stand: two processes, three plugin sets, the registry as the architecture, JSON-RPC with the CLI as a
generated adapter, hybrid-gated replay with cross-version refused, undo as inverse capture, and the
document as the edit World. Every correction above is a gate, a citation or a mechanism — which is
the right distribution for a fourth revision, and would have been the wrong one for a first.

### 12.2 Round 3 — the critic's REVISE of revision 2 (kept for the record)

| # | blocking item | decision | where |
|---|---|---|---|
| 1 | `Time::pause` at boot composed with "`V5` — viewport with the existing fly camera": `fly_camera_system` reads `time.delta_secs()` (`camera.rs:879-888`), which `Time::pause` makes ZERO (`time.rs:91-97`). The user presses W and the camera does not move, with no error and no failing gate | **Adopted; the design changed.** Rule 2 is kept — it is a second barrier and it is what makes "nothing simulates while editing" checkable — and a THIRD rule is added: **every edit-time interactive system reads REAL time.** The clock census the critic asked for is written out and is short: outside `boyko_ecs` exactly two systems consume `Res<Time>` — `fly_camera_system` (`camera.rs:880`) and `particle_system` (`crates/boyko_render/src/particle_system.rs:308`). The editor does NOT add `FlyCameraPlugin`; it adds `editor_camera_system` driving the SAME pure `fly_step` from `time.real_delta()`, which makes `fly_step` public a one-line kernel item (`EK13`). Particles are frozen at edit time — stated as intended, and raised as owner question 7 because it is a VALUE. `V5` gains a camera gate that is red before `EK13` | `EDITOR-BOUNDARY.md` §4.2; §3 (`V5`), §4 (`EK13`), §6 Q7 above |
| 2 | `UiPlugin` is not the plugin that installs the UI: `build` returns early without a path and otherwise registers only `ui_hot_reload_system` (`plugin.rs:84-127`); nothing in `crates/*/src` registers `ui_layout_discovery`/`ui_layout_apply`; `.ui` is refused for chrome by `U4`, so the one thing `UiPlugin` does is unusable by the editor | **Adopted in full; `V0` is re-sized.** `EK8` is restated as five deliverables, the first of which is a NEW `UiRuntimePlugin` in `boyko_ui` — the crate that owns the ordering contract it is transcribing (`lib.rs:19-30`, `measure.rs:27-28`, `widgets.rs:250-253`) — registering resources and the full pipeline in order, plus host seeding of `UiViewport` on create AND resize. A second red-first calibration is added to `V0`'s gate: delete `ui_layout_apply` and the golden must change. `UiPlugin`'s doc comment is filed as stale (§7) so the next reader does not repeat the mistake | `EDITOR-UI.md` §1.4, §1.5; §3 (`V0`), §4 (`EK8`), §7, §8 above |
| 3 | There is no carrier for "this button invokes command X with arguments Y": the shipped press path ends in `OnClick(u16)` → `ActionState::ui_press`, the surface `E2.1` itself rejects (256 fieldless variants, write-once, argument-free). So `ui.click(name)` cannot be "the same path" as a human click, and the thesis has no attachment point | **Adopted; a mechanism was added, `E2.8`/`EK15`.** `OnCommand` is a `#[repr(C)] Copy` component `{cmd: CommandId, argv: ArgSource}` whose argument PROVENANCE is declared at the binding site (Emacs's `interactive` split, which the survey named as the resolution to Blender's documented defect #1); `ui_command_dispatch` is NOT generic over `Actionlike`, so an editor vocabulary and a game vocabulary coexist. **The click path needs no change to `ui_focus_system`**: `resolve_pointer` stamps `click_fired = Some((origin, action))` for ANY hovered node, reading `OnClick` only for the action word (`focus.rs:475-500`), so the origin is already there. The SUBMIT path does need one: `pending_submit` is `Option<u16>` and is `None` when the focused node has no `OnSubmit` (`focus.rs:556-562`), so the Enter edge is lost — `EK12` widens it to `Option<(Entity, u16)>` and is behaviour-preserving for the existing consumer, which skips `NO_ACTION` either way. `U5` is re-derived from `OnCommand` | `EDITOR-COMMANDS.md` §9; `EDITOR-UI.md` §5.2; §2 (`E2.8`), §4 (`EK12`, `EK15`) above |
| 4 | `CommandDesc` embeds Gaia `GK-4`'s field tables while `E2.3` refuses a second carrier, so `V1` and `V3` stand on the same unstarted rung `V5` was said to be alone in waiting for — leaving either a blocked first rung or the two-carrier drift the design cites Bevy for | **Adopted; the carrier changed.** The conflation was the defect: `GK-4` describes COMPONENT fields, while a command's argument struct is not a component. `EK4` lands **`FieldTable` as a kernel type** with two emitters — `#[derive(CommandArgs)]` for argument/result structs and an opt-in `#[derive(Fields)]` for components (the `#[derive(Bindable)]` precedent, already in the tree and explicitly reflection-free). Each descriptor comes from the SAME derive invocation as its decoder, which is exactly what Bevy's `rpc.discover` lacked, so the anti-drift argument survives intact. `EK7` inverts: Gaia `G1` consumes `FieldTable` and extends it with the disposition column `GB-5` gates, rather than the editor waiting on `G1`. **NO rung waits on Gaia**, and every "only rung that waits" sentence is deleted. The inspector's coverage becomes opt-in per type, with an opaque whole-value row for a type that has not opted in — stated as the price | `EDITOR-COMMANDS.md` §3; `EDITOR-DOCUMENT.md` §2.3; §2 (`E2.3`), §3, §4 (`EK4`, `EK7`) above |
| 5 | The replay world hash was an FNV over the `save_world` byte image, which is not invariant under the id remap replay performs: the saver blits raw `EntityId`s (`save.rs:85-92`) and `s2e` ids (`:310-321`) while `load_world` allocates fresh ids under the only policy (`load.rs:69-78`). `V8`'s fidelity gate would red deterministically for a non-divergence reason, and `replay.seek` would disagree with continuous playback | **Adopted; the oracle changed, and became `EK16`.** The frame digest is no longer a byte image. It is an **order-independent, `StableId`-keyed set digest**: per live entity a row digest over `(stable_id, sorted (component stable-name hash, bytes))`, combined commutatively at both levels — so archetype creation order, row order after swap-remove, dense slot order and entity-id remap are all irrelevant by construction. A component with an installed entity-remap fn is REFUSED from the determinism set at registration with a coded error, because raw `Entity` bytes are not remap-invariant; the mitigation is that a re-parent is still observable through the `GlobalTransform` values that ARE hashed. `V4`'s document gate uses the same machinery over the full serializable set, so one mechanism serves both | `EDITOR-REPLAY.md` §1.5, §5.2; `EDITOR-BOUNDARY.md` §3.4; §4 (`EK16`) above |
| 6 | Gate G-A's byte-identity half is unsatisfiable: `Selected` is Table storage, so attaching it migrates the entity into a different archetype and changes the file's block sequence and intra-block ordering (`save.rs:170-175`). The implementer meets a red that is not a defect and redefines the oracle at the bench; deleting the byte half loses real coverage. The storage kind of Class A is an unexamined fork | **Adopted, and the cause was removed rather than the gate weakened.** Two changes. (a) **v1's Class A set is EMPTY**: selection is an editor-owned `Selection` resource keyed by `StableId`, and lock/hide are cut from v1, so no annotation is attached to a document entity and no migration happens. The Class A RULE is kept for when one is needed and is now specific: `#[component(no_serialize)]` **plus `storage = "dense"`** — a dense component is signature-excluded (`save.rs:277-280`) so it does not migrate the entity, is skipped by BOTH save passes when `Ignore` (`:186-193`, `:286-296`), and is still queryable, `With`/`Without` having a dense arm (`filter.rs:143-152,451-463`) — and **never** `#[require(EditorOnly)]`. (b) The gate is split into the two properties it was conflating, each with an oracle that survives fragmentation: **G-A (nothing lost)** is an `EK16` content digest of the RELOADED world against the FIXTURE, not a save-vs-save compare; **G-B (nothing extra)** is a direct census of the file's type table and dense blocks. Byte-identity is dropped as an oracle and the reason recorded | `EDITOR-BOUNDARY.md` §3.2, §3.4; `EDITOR-DOCUMENT.md` §4; §2 (`E1.3`), §3 (`V4`) above |
| 7 | The fault model has no rule for three of the failure modes the brief names — GPU device loss (`report_terminal_device_error` exits, `diag.rs:150-163`), a hang, and a panic that reaches the frame loop rather than a command handler | **Adopted; three rules added and one widened.** Rule 3 widens from the command handler to the **frame boundary**: `EK14` puts one `catch_unwind(AssertUnwindSafe(…))` around `app.update_with_delta(dt)` (`runner.rs:1233`) under a `FramePanicPolicy` resource defaulting to `Propagate`, so the shipped game is textually unchanged; the editor selects `PoisonAndReport`, which autosaves, logs, runs the ordinary D2 teardown and exits — which also removes this repository's recorded "worker panic freezes the window" mode, since the window is destroyed rather than abandoned mid-unwind. New rule 5: **device loss is not catchable**; the loss is BOUNDED by the autosave cadence and the coded exit prints the recovery path. New rule 6: **a hang has no edit-process remedy** and is stated as such; the PLAY link does have one — a heartbeat and a kill escalation, gated by a hanging fixture system at `V7` | `EDITOR-BOUNDARY.md` §4.3–§4.6; §2 (`E1.4`), §3 (`V4`, `V7`), §4 (`EK14`) above |

**One correction the critique did not raise, found while checking its citations:** revision 2 said
`present_sampled`'s only caller in the tree is `ui_rect_swapchain_golden.rs:639`. There are **two** —
`crates/boyko_rhi_vulkan/tests/window_present_hybrid.rs:978` is the other. Both are tests, so the
conclusion (`U1`) is unchanged; the count was wrong and is fixed everywhere it appeared.

### 12.3 Round 2 — the critic's first REVISE (revision 2, kept for the record)

| # | blocking item | decision |
|---|---|---|
| 1 | `E1.3` asserted "a component is non-serializable until it opts in" from the `SERIALIZABILITY` default of `Ignore`. That default applies to a hand-written `impl Component` and is INVERTED by the derive (`component.rs:161-166`; macro arm at `crates/boyko_macros/src/component.rs:1607-1690`) | The mechanism is the explicit `#[component(no_serialize)]` attribute; exclusion is opt-OUT and a forgotten attribute is silent, hence gate G-C. The claim that this is "Godot's `owner` by another route" was struck: `owner` is structural and unforgeable, this is an attribute a developer can omit |
| 2 | `EditorOnly` was `#[require]`d by EVERY editor component while `Selected` was listed as an editor component on a DOCUMENT entity. `#[require]` keys enter the archetype signature (`component.rs:312-328`), so selecting an object and saving would have DELETED it from the document | Two disjoint vocabularies, and a gate that compares the entity SET rather than `len()`. (Revision 3 goes further and empties Class A for v1) |
| 3 | The `V4` gate was a save-vs-save byte compare, which passes when bytes are missing on BOTH sides because both come from the same saver — exactly the shape defect 2 produces | The gate compares the reloaded entity SET against the input fixture. (Revision 3 replaces the compare with `EK16`'s digest, for the reason in §12.2 item 6) |
| 4 | `EDITOR-UI.md` was referenced ten times and did not exist | Written, and the link census (§11) filed as a campaign requirement |
