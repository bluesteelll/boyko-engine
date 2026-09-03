# E2 and E5 — The command registry, and undo as the same design

**Part of the editor campaign design.** Index and v1 ladder: `EDITOR-DESIGN.md`. **Revision 5.**
**Revision 5** re-tensed §7's evidence: the `serialize_ui` rot it argues from was repaired
2026-09-03, and the repair took the shape §7 prescribes — one declaration, an exhaustive match, and
a behavioural census beside it for the arm exhaustiveness cannot see. The census's second half is
now stated for commands too. Claims added by revision 5 are read in the WORKING TREE at HEAD
`59009f8a`.
**Revision 4** corrected three things: the `ByteInverse` row no longer asks for a kernel change the tree already made (§8.1), `V2`'s undo oracle is `EK16`'s digest rather than save bytes and the reason is §8.2b, and `ArgSource` no longer names an editor type from inside `boyko_ui` (§9.2). It also closes the transport→inbox hole as `E2.9` (§4.2).
Decisions `E2.1`–`E2.8`, `E5.1`–`E5.2`. Every code claim carries a `file:line` read at HEAD
`98ede3e0`.

| § | decision |
|---|---|
| §1 | `E2.1` — the registry IS the architecture |
| §2 | `E2.2` — declaration and registration (`EK2`) |
| §3 | `E2.3` — arguments and results: `FieldTable` (`EK4`) |
| §4 | `E2.4` — the execution point |
| §5 | `E2.5` — results and errors to a non-human caller |
| §6 | `E2.6` — discovery |
| §7 | `E2.7` — completeness |
| §8 | `E5.1`, `E5.2` — undo |
| §9 | `E2.8` — the UI→command carrier (`EK15`) |
| §10 | alternatives to the whole design, priced |
| §11 | out of scope, and what this file did NOT establish |

---

## 1. The registry IS the architecture — `E2.1`

**Decision. The editor's own UI and the machine agent are both CLIENTS of one command registry.
There is no second automation surface, and a UI system may not mutate the world directly** —
enforced by a `boyko_editor` census (`EK9`) forbidding `set_component_raw`, `Commands::insert` and
`get_component_mut` outside `commands/`.

The survey produced a clean split with no overlap between the piles.

**Systems whose UI dispatches through the same named, typed, eligibility-gated, undo-aware registry
their automation uses are automatable to the depth of their UI**: Blender's `wmOperatorType`
(one global registry, `WM_operatortype_find(idname)`, an RNA property struct per operator, `poll`
separated from `execute`, undo declared as `bl_options={'UNDO'}`), Maya's `MPxCommand`
(`doIt`/`redoIt`/`undoIt`, `isUndoable` deciding whether the instance is retained by the undo
manager), Photoshop's Action Descriptors (`batchPlay` over an array of descriptor OBJECTS, so
recording, replay, batching and inspection fall out for free), Fyrox's `CommandTrait`
(`execute`/`revert`/`name`, `CommandGroup`, `CommandStack`).

**Systems that added automation BESIDE the UI drifted, in documented user-facing ways**: Unity's
`EditorApplication.ExecuteMenuItem(string menuItemPath) -> bool`, where the only stable handle to a
large slice of editor functionality is its POSITION IN A MENU — a presentation artefact, with no
arguments and no result beyond a bool; Godot's `EditorCommandPalette.add_command`, which takes a
`Callable` with arguments PRE-BOUND at registration and therefore invokes a nullary thunk, sitting
beside a separate manual `EditorUndoRedoManager` so undoability is a property of DISCIPLINE rather
than of the operation; Unreal's four partially-overlapping surfaces (console/exec down a fixed
object chain, Blueprint-callable UFUNCTIONs, editor-only Python, Remote Control HTTP) each with a
different reachability rule and a different undo story; and AutoCAD's permanent parallel
dash-commands (`-LAYER`, `-INSERT`), invented because dialogs left the command language.

**The correction the survey also produced, and which this design takes.** The winners are not
automatable *because the UI calls a command layer*. They are automatable because ONE registry
carries **name + typed arguments + eligibility + result + undo**, and both the UI and the machine
are clients of it. A CLI, a socket and an MCP server are then adapters (`EDITOR-TRANSPORT.md`).
What cannot be retrofitted is the registry.

**Alternatives, priced.**

| alternative | price | verdict |
|---|---|---|
| Expose the engine through the existing action-name table — the one name→invocation path that exists end to end | not extensible into this: write-once and single-`Actionlike` (the first `register_action_names` wins and a second is a silent no-op, `crates/boyko_input/src/action/names.rs:22-42`), capped at 256 fieldless variants (`crates/boyko_input/src/constants.rs:10-16`), argument-free, return-free, and not enumerable at all because `ACTION_NAMES` is a private static | rejected — an editor vocabulary and a game vocabulary cannot coexist in it |
| Take Blender's operator layer wholesale | rejected on Blender's OWN documented grounds: operators cannot be passed the data to operate on (they read ambient context), return only success/cancel, and their `poll` fails without saying why. §3 and §4.3 avoid all three by taking explicit typed arguments, returning typed results, and returning `Ineligible { code, detail }` rather than a bare `false` | adapted, not copied |
| A hand-written command list (what Godot's palette and Unity's menu paths amount to) | rots, and this repository has MEASURED the rot (§7) | rejected |

---

## 2. Declaration and registration — `E2.2`, kernel item `EK2`

```rust
/// Spawn an empty entity and return its stable id.
#[command(name = "entity.spawn", kind = Mutating, undo = Structural)]
fn entity_spawn(world: &mut EcsMaster, args: &EntitySpawnArgs)
    -> Result<EntitySpawnOut, CommandError> { … }
```

`#[command]` emits a `CommandDesc` static beside the function:

```rust
pub struct CommandDesc {
    pub name:        &'static str,     // "entity.spawn" — dotted, namespaced by subject
    pub doc:         &'static str,     // the fn's own /// comment, verbatim
    pub kind:        CommandKind,      // ReadOnly | Mutating
    pub undo:        UndoClass,        // None | ByteInverse | Structural | Custom
    pub args:        &'static FieldTable,   // EK4
    pub out:         &'static FieldTable,   // EK4
    pub eligible:    fn(&EcsMaster, &[u8]) -> Eligibility,
    pub invoke:      fn(&mut EcsMaster, &[u8], &mut CommandOut) -> Result<(), CommandError>,
}
```

Each crate lists its commands in ONE table, and a plugin's `build` registers it:

```rust
command_table!(EDITOR_COMMANDS = [entity_spawn, entity_despawn, component_set, …]);
// in EditorPlugins::build:
app.register_commands(&EDITOR_COMMANDS);
```

`register_commands` interns each name into a process-global sorted `&'static str → CommandId` table
— the same shape the engine already uses three times (component `stable_name`s, action names,
diagnostic codes), all cold-path, all enumerable, all sorted once and binary-searched. A duplicate
name is a coded panic at registration, not a silent last-wins.

**Why a table per crate rather than inventory-style auto-collection.** Auto-collection needs either a
linker-section crate (a third-party dependency) or a `ctor`-style global constructor, and it makes
"which commands exist" depend on link order. An explicit table is one line per command and is what
the completeness census (§7) reads.

---

## 3. Arguments and results — `E2.3`, kernel item `EK4`

**Decision. Arguments and results are `#[repr(C)]` structs described by `FieldTable`, a KERNEL type
this campaign lands, emitted by `#[derive(CommandArgs)]`. The schema and the decoder come from ONE
derive invocation per struct, so they cannot drift. There is no Gaia dependency.**

```rust
#[derive(CommandArgs)]
#[repr(C)]
pub struct ComponentSetFieldArgs {
    pub entity:    StableId,
    pub component: StableNameRef,   // the component's stable name
    pub field:     FieldPath,       // "translation.x"
    pub value:     Scalar,
}
```

`FieldTable` is a `&'static [FieldDesc]` with

```rust
pub struct FieldDesc {
    pub name:          &'static str,
    pub offset:        u32,               // byte offset within the #[repr(C)] struct
    pub kind:          FieldKind,         // scalar kind, or Nested(&'static FieldTable)
    pub is_entity_ref: bool,
    pub default:       Option<&'static [u8]>,
}
```

### 3.1 What changed in revision 3, and why the anti-drift argument survives

Revision 2 said `CommandDesc` embeds **Gaia `GK-4`'s** field tables, and refused a second carrier on
the grounds that a second carrier is precisely Bevy's defect. That was wrong twice.

- **`GK-4` describes COMPONENT fields.** A command's argument struct is not a component. Conflating
  them was sloppy on its own terms.
- **It made `V1` and `V3` depend on an unstarted campaign.** `GK-4` is Gaia's `G1` rung; there is no
  Gaia crate in this workspace, `G0`–`G8` are unstarted, and `G1` additionally carries an open ballot
  `GB-5`. So the first rung of the ladder — and the transport rung whose gate asserts non-empty
  parameter schemas — stood on the same unstarted thing that revision 2 said `V5` was ALONE in
  waiting for.

**The anti-drift argument was never about using Gaia's table; it was about one source per thing.**
Bevy's `rpc.discover` ships method names with EMPTY parameter arrays *because its schemas live in the
reflection type registry while its methods are serde-based* — two sources for the same thing, and
they drifted. Here, the argument struct's schema and the argument struct's decoder are emitted by the
SAME `#[derive(CommandArgs)]` invocation on the SAME struct. There is one source per thing, which is
the property that matters.

### 3.2 Component fields — the inspector's half

The inspector needs the field decomposition of a COMPONENT, which is `GK-4`'s subject. `EK4` supplies
it with a second emitter of the same type: **an opt-in `#[derive(Fields)]`** a component type may
also carry, emitting a `FieldTable` for it.

The precedent is in the tree and is exact: `boyko_ui`'s `#[derive(Bindable)]` already generates
per-field accessors (`FIELD_COUNT`, `fmt_field(u8, &mut dyn Write)`, `value_field(u8) -> f32`,
`field_id(&str) -> Option<u8>`, `register_bind_accessor()` into a `ComponentId`-keyed table) and is
documented as explicitly reflection-free
(`crates/boyko_ui/src/binding/bindable.rs`). `#[derive(Fields)]` is that pattern generalised from
`f32`-and-string to the full `FieldKind` set, and put in the kernel where both consumers can reach
it.

**The price, stated.** Coverage is opt-in per type. A component without `#[derive(Fields)]` renders
in the inspector as ONE opaque row showing its type name and byte length, and `component.set`
(whole value) still works on it. That is a real limitation and it is the right one for v1: it costs
nothing in a shipped game binary (no type gains a table it did not ask for, which is what the Gaia
`F7`/`F1` "no reflection in the shipped binary" constraint requires), it needs no feature-unification
reasoning, and the fixture components the `V5` gate opts in are enough to demonstrate the chain.

### 3.3 `EK7` — the Gaia coordination requirement, filed now

When Gaia's `G1` lands `GK-4`, it **emits `EK4`'s `FieldTable`** — extending it with the disposition
column that `GB-5` gates — rather than minting a second descriptor. This is filed against `G1` NOW
rather than discovered at merge, because this repository has recorded the failure of finding out late
that a sibling plan already owns your rung.

Two facts make the filing cheap. First, `EK4`'s shape omits exactly the column `GB-5` blocks: Gaia's
own campaign row says "the table may be designed and emitted; the **disposition column for
engine-derived fields may not be minted** until `GB-5` is answered", and `EK4` mints no disposition
column. Second, `#[derive(Fields)]` is opt-in, so `G1` extending it to every component under its own
gate is additive rather than a redefinition.

### 3.4 Validation

Decoding an argument struct from the wire validates against the `FieldTable`: an unknown field name
is a coded error naming it; a missing field with a declared default is filled; a missing field
without one is a coded error naming it; a type mismatch names the field and both types. This is a
`FieldTable` walk, not a schema language — the table IS the schema, so there is nothing to keep in
sync.

**Alternatives, priced.**

| alternative | price | verdict |
|---|---|---|
| Untyped JSON arguments (VS Code's `...args: any[]`, LSP's own `Command`) | cheap; and for an agent barely better than no command at all — MCP REQUIRES an `inputSchema`, and a tool whose arguments are undocumented JSON is a tool the model must guess at | rejected |
| Derive schemas from `boyko_reflect` | not on this branch; the ECS glue ladder stops at enumeration and the kernel seam its read/write rungs stand on was audited and REFUSED | unavailable |
| A command-specific descriptor type distinct from the component one | two types where one suffices, and the merge with `GK-4` would then be a translation rather than an extension | rejected |

---

## 4. The execution point — `E2.4`

**Decision. ONE point: an exclusive system `apply_commands(world: &mut EcsMaster)` at the head of
`CoreSchedule::Main`, draining a fixed-capacity `CommandInbox` resource fed by the UI's dispatch and
by the transport thread. Handlers hold `&mut EcsMaster` and may call `run_system_once`; they never
run a `Schedule`.**

### 4.1 Why exactly one point, and why it is exclusive

`APP4` is a structural invariant, not a convention. `run_system_once` / `run_system` /
`run_cached_system` all take `&mut self`, exclusive for the whole call, and the source states that
"no `apply` re-entry into `run_system` / `run_cached_system` / `run_system_once` is reachable
(Rust's borrow checker rejects the nested `&mut`)"
(`crates/boyko_ecs/src/ecs/core/ecs_master/system_api.rs:101-106`; also `:40,111,142`). An editor
command such as "recompute this subtree's transforms now" wants to run a system, so it CANNOT be
applied from inside another system's command `apply`. An exclusive system holding `&mut EcsMaster` is
the engine's own shape for that context and is already used for exactly this reason —
`ui_layout_apply` is exclusive because it is "the only form that expresses nested parent↔child
mutable row access without unsafe aliasing" (`crates/boyko_ui/src/lib.rs:15-18`).

A schedule is bound to the world it was built on and panics `boyko-B9101` off it, which is a second
reason a handler never runs one.

### 4.2 The inbox

`CommandInbox` is a fixed-capacity ring of `CommandEnvelope { id: RequestId, cmd: CommandId, args:
ArgSlice, source: Source }`, preallocated at plugin build. Two producers:

- the UI, from `ui_command_dispatch` (§9), on the main thread, earlier in the same frame;
- the transport listener thread, through an SPSC ring (`E3.1`).

Drain order is stated: UI first, then transport, each in arrival order. That removes the ordering
ambiguity an agent racing a human makes real.

**`E2.9` — how the foreign thread reaches the inbox without a lock and without aliasing the world.**
Revision 3 said "through an SPSC ring" and stopped, which left the load-bearing part unsaid: a world
resource must not be touched from a thread that is not the dispatcher, and `Mutex` is forbidden on
this path. The ring is therefore **not** the resource. At plugin build, `RemotePlugin` allocates one
`CommandRing` on the heap and splits it into two halves: the **producer** half moves into the
listener thread; the **consumer** half is what `CommandInbox` holds. The shared allocation is kept
alive by one `Arc<CommandRing>` per half — boot plumbing, which is a named legitimate exception in
this repository's own hot-path rules, and the only atomic traffic on the frame path is the
consumer's `Acquire` load of the producer's tail, one line, once per frame when the ring is empty.
Head and tail sit on separate cache lines; the argument bytes live in a preallocated arena inside the
ring, so an envelope carries `{offset, len}` and no allocation happens on either side after boot.

**The listener DECODES; the exclusive window never parses.** JSON arrives on the socket thread, is
validated there against the target command's `FieldTable` (`EK4`), and is written into the ring's
arena as the command's `#[repr(C)]` argument struct. `apply_commands` therefore pays a bounds check
and a memcpy, never a parse — which matters because that window is exclusive and everything else in
the frame is waiting on it. A malformed request is refused **on the listener thread**, with the code
returned over the wire and no ring slot consumed; only well-formed work crosses into the frame.

**Overflow is REFUSAL** — `E1901 InboxFull`, the request rejected with a code the caller can act on
— **never drop-oldest.** This is the deliberate opposite of `RawInputQueue`'s policy
(`crates/boyko_input/src/raw/queue.rs:76-99`), and the asymmetry is the point: dropping the oldest
INPUT event loses a keystroke, while dropping the oldest COMMAND silently loses a mutation, which is
the failure this whole design exists to avoid.

### 4.3 Eligibility, and the Blender defect it avoids

`CommandDesc::eligible` returns

```rust
pub enum Eligibility { Ok, Ineligible { code: ErrorCode, detail: &'static str } }
```

Blender's documented defect is that `poll` "can fail where an API function would raise an exception
giving details on exactly why", producing `RuntimeError: … poll() failed, context is incorrect` with
no statement of what context was needed and the advice to go read the C source. Returning a CODE and
a detail costs one enum and removes the defect. The separation itself is VS Code's — existence
(registry) is separate from eligibility (`when`) is separate from presentation (menus) — and it is
what lets an agent be TOLD why a command is unavailable rather than discovering it by failing.

Eligibility is evaluated for a machine caller exactly as for the UI. Blender's answer to that
question is yes (a script's `poll` fails too), and any other answer would mean the two surfaces are
not the same surface.

### 4.4 The price, stated

One exclusive system at the head of `Main` per frame, whose cost when the inbox is empty is one
`is_empty` check. A command that runs a system pays that system's cost inside the exclusive window,
serialised against everything else — which is correct (it is a mutation of the document) and is the
reason a command is not the right shape for anything per-frame.

---

## 5. Results and errors to a non-human caller — `E2.5`

Results are typed structs described by `CommandDesc::out`. Failures carry a code from the
**`boyko_log` registry** plus a typed detail.

**Why the existing registry rather than a new error enum.** `boyko_log`'s `codes!{}` generates a
`pub const` per code, a dense sorted table, an index and `explain`; a `"boyko-…"` literal outside the
registry is a BUILD FAILURE; the code CLASS is a type (`warn!` takes `WarnCode`, `error!` takes
`ErrorCode`), so a class mismatch does not compile; and lifecycle status is enforced — `Live`
requires at least one use AND a `docs/diagnostics/<code>.md` page, `Pending` requires ZERO uses so
the row must flip in the same commit that emits it
(`crates/boyko_log/src/codes.rs:1-40`). That is a machine-readable, build-enforced, documented error
vocabulary with `explain` already fetchable — everything MCP's and LSP's error models ask for. A
parallel enum beside it is the second-surface trap applied to errors, and it would forfeit the
build-enforced doc page.

**The `E19xx` series.** Verified free at this checkout: the two-digit prefixes in use across
`codes.rs` are `00 01 02 05 07 08 09 13 15 18 20 21 22 26 30 92`, so `19xx` is unclaimed. Initial
rows:

| code | meaning |
|---|---|
| `E1901` | `InboxFull` — the command inbox is at capacity; the request is refused, never queued |
| `E1902` | `UnknownCommand` — the name resolves to no registry entry |
| `E1903` | `ArgumentInvalid` — a `FieldTable` decode failure; the detail names the field |
| `E1904` | `Ineligible` — `eligible` refused; the detail is the eligibility reason |
| `E1905` | `EntityNotFound` — a `StableId` resolves to no live entity |
| `E1906` | `UndoUnavailable` — the ring is empty or the entry aged out |
| `E1907` | `PlayProcessExited` — with the exit code and reason |
| `E1908` | `DocumentPoisoned` — a caught frame-boundary panic; mutating commands refused until restart |
| `E1909` | `ConfirmRequired` — a destructive command from an agent without `confirm: true` |

**The two-level split**, which MCP requires: a PROTOCOL error (unknown command, undecodable
arguments) is a JSON-RPC error; an OPERATION error (the entity does not exist, the command is
ineligible) is a normal result with `isError: true` and the code inside, because the spec says an
operation error should reach the model for self-correction while a protocol error is "less likely to
result in successful recovery".

`world.hash` and LSP's `ContentModified` shape the third case: an agent issuing a command against a
world state a frame has since advanced past. v1 does not implement optimistic concurrency, but
`CommandOut` carries the post-command `world.hash`, so an agent can detect that the world moved
without a protocol for it.

---

## 6. Discovery — `E2.6`

Two forms of the same data.

**Live:** `session.describe` enumerates the registry — name, doc, kind, undo class, argument table,
result table — and `describe_commands(query)` filters it (`E3.3`).

**Committed:** `docs/editor/COMMANDS.json`, regenerated by a test that compares the file against the
registry and reds on a difference. It exists so a reader (human or agent) can see the surface without
running the engine, and so a diff shows when the surface changes.

### 6.1 What the manifest is FOR, stated so it is not over-built

It is a snapshot for review and for offline tooling. It is NOT the source of truth (the registry is)
and NOT a code generator's input in v1. `boyko-ctl` is generated from the registry at build time in
the same crate, not from the JSON.

### 6.2 The v1 command set (roughly 35; the catalogue grows later)

| namespace | commands |
|---|---|
| `session` | `ping`, `describe` |
| `world` | `query`, `hash` |
| `entity` | `spawn`, `despawn`, `get`, `set_parent`, `children` |
| `component` | `list`, `get`, `set`, `get_field`, `set_field`, `add`, `remove` |
| `selection` | `get`, `set`, `add`, `clear` |
| `document` | `open`, `save`, `save_as`, `is_dirty` |
| `undo` | `undo`, `redo`, `group.begin`, `group.end`, `history` |
| `ui` | `tree`, `click`, `type`, `focus`, `scroll` |
| `play` | `start`, `stop`, `pause`, `status` |
| `replay` | `record`, `stop`, `load`, `step`, `seek` |
| `log` | `tail`, `explain` |
| `app` | `exit`, `render.pause` |

---

## 7. Completeness — `E2.7`

**Decision. A TWO-LAYER gate: (a) a SOURCE CENSUS over every `#[command]` site, asserting each is in
exactly one `command_table!` and each table is registered by a plugin in the editor's set; (b) the
manifest regenerate-and-compare.**

**Why a coverage census and not an equality check.** A hand-maintained list rots HERE, measurably —
and the measurement is now a closed case rather than a live one, which is what makes it usable as
evidence. Until 2026-09-03 `serialize_ui` wrote 8 of the 20 non-exempt components its own parser
accepts (`crates/boyko_ui/src/text/serialize.rs:21-23`, HEAD `59009f8a`), and **its round-trip gate
stayed GREEN over the loss**, because `serialize → parse → serialize` is byte-identical when both
sides are equally lossy. The same one vocabulary was spelled out by hand in FOUR places; the reload
reconcile's copy had rotted in the other direction — it patched 10 of the 21 members and ignored ten
of the eleven it left out (only `ComputedRect` was deliberate), so an edited `Button` action was
dropped on any node that survived a reload.

The repair took exactly the shape this section prescribes, which is why the tense change strengthens
the argument rather than removing it: the vocabulary became ONE declaration
(`crates/boyko_ui/src/text/vocab.rs:163-205`), every consumer matches on it EXHAUSTIVELY so a new
member is an `E0004` rather than a review item, and — because exhaustiveness cannot see an arm that
exists but does nothing — the compile gate is PAIRED with behavioural censuses that re-read the
whole vocabulary out of a second world (`crates/boyko_ui/tests/p3_world_round_trip.rs`,
`crates/boyko_ui/tests/p3_reload_patch_vocabulary.rs`). Both halves transfer here: the census over
the DECLARATION SITES is the `E0004` equivalent for a set the type system cannot close, and it needs
its own "the arm is not empty" half — a command that is declared, tabled and registered but whose
body does nothing is exactly the arm exhaustiveness cannot see. So the gate must be a coverage
census over the declaration sites, not an equality check over the output. This engine already has
the working shape one layer down: `boyko_log`'s `codes!` fails the build on a `"boyko-…"` literal
outside the registry, and enforces a doc page per `Live` code.

The census is one `grep`-shaped test: enumerate `#[command]` attributes across
`crates/boyko_editor/**` and `crates/boyko_ecs/**`, enumerate `command_table!` members, and assert
the two sets are equal. A command declared and never tabled is red; a table never registered is red;
a name in a table with no `#[command]` site is red.

**Can the macro machinery generate the registry?** Yes for the DESCRIPTOR (that is `#[command]`), no
for the COLLECTION without a linker-section dependency or a global constructor (§2). The census is
what makes the manual collection safe, and it is cheaper than either alternative.

---

## 8. Undo — `E5.1`, `E5.2`

**Decision. Undo IS the command layer: every mutating command captures its INVERSE at execute time.**

That is Maya's contract in its own words — "the `doIt` method should collect whatever information is
required to do the task, and store it in local class data … the `redoIt` method should do the actual
work, using only the local class data" — and it is the same property that makes a command loggable
and replayable, so **undo and the replay command log are one artefact rather than two**.

### 8.1 The three undo classes

| class | inverse | applies to |
|---|---|---|
| `ByteInverse` | the component's whole-value bytes BEFORE the write, read through `get_component_raw` (`crates/boyko_ecs/src/ecs/core/ecs_master/component_api.rs:176`) and restored through `set_component_raw` (`:462`), which **stamps the changed tick on both storage arms by itself** (`:439-452`) — revision 4 removed the `mark_component_changed` this row used to require, because the hole it was written for was fixed in the tree on 2026-07-10 (`0f944f51`, gated by `crates/boyko_ecs/tests/change_tick_on_raw_write.rs`) | `component.set`, `component.set_field` |
| `Structural` | the operation's structural mirror — despawn↔spawn (with the full component image captured), add↔remove, reparent↔reparent-back | `entity.spawn`, `entity.despawn`, `component.add`, `component.remove`, `entity.set_parent` |
| `None` | nothing | every `ReadOnly` command, and `selection.*` (selection is editor state; the survey's engines all keep selection out of the document undo stack, and so does this one) |

**Byte inverses are reachable TODAY without any reflection**, which is what lets `V2` land before
`V5`. `#[derive(Fields)]` refines the *presentation* of a field write; it does not change the
inverse, which is the whole component's bytes either way.

### 8.2 The ring, groups, and scope

One undo ring per document, bounded: 4096 entries and a 64 MiB byte arena for captured images,
whichever binds first. Exceeding either drops the OLDEST entry and `undo` past the bound is a coded
refusal (`E1906`), never a silent no-op — asserted explicitly by `V2`'s gate, because a silent no-op
is how an agent concludes it undid something it did not.

### 8.2b The oracle: what "the undo restored the world" is allowed to mean

**`V2`'s proptest compares `EK16` digests, not save bytes.** Revision 3 compared the `save_world`
image of the post-undo world with the image of the starting world and argued that byte-identity was
legal here "because no reload intervenes, so ids are preserved". It is not, for two reasons that are
unconditional and are not defects:

1. **An emptied archetype is permanent.** `save_world` walks `iter_archetypes()`, which is a plain
   `self.archetypes.iter()` (`crates/boyko_ecs/src/ecs/core/archetype/archetype_master.rs:815-817`),
   pushes an `ArchetypePlan` for every archetype it sees, and calls `intern_type` per column before
   any row count is consulted (`crates/boyko_serialize/src/save.rs:174-190`). `remove_archetype`
   exists (`archetype_master.rs:664`) and **has no production caller** — the only call sites in
   `crates/` are its own two unit tests. So a proptest sequence that spawns a bundle the fixture did
   not have permanently adds an archetype block and its type-table entries, and the undo cannot take
   them away.
2. **An undo of a despawn cannot restore the same `Entity`.** `E1.5` says exactly this, in its own
   reasoning for why `StableId` exists. The re-spawned row carries a bumped generation, and
   `save_world` blits the archetype's raw `EntityId` column (`save.rs:85-92`).

Both would have fired on the first proptest run, on a correct implementation, and the predictable
response at the bench is to weaken the oracle — which is how this repository has lost gates before.
`EK16`'s digest is keyed on `StableId` and combines commutatively at both levels, so it is invariant
under archetype creation order, row order, dense slot order and id remap, and it still reds on a
component whose bytes did not come back. It is the same mechanism `V4`'s G-A and `V8`'s replay
oracles use, which is why **`EK16` lands at `V2`** rather than `V4`.

**Groups** matter specifically because an agent submits batches: `group.begin` / `group.end` bracket
N commands into one undo unit, executed forward and reverted in REVERSE order (Fyrox's
`CommandGroup`). A drag in the viewport is one group. A batch that half-applies and cannot be undone
as a unit is the liability the brief names.

**Scope.** One ring per open document. The editor has one World and one document in v1, so there is
one ring; the structure is per-document because Godot's `EditorUndoRedoManager` scopes histories
(scene / global / remote) and multi-document is the obvious v2, not because v1 needs it.

### 8.3 Play mode

**No undo recording in play mode, and no write-back of play-mode edits in v1.** This is the Unity and
Unreal DEFAULT, and both ecosystems then grew per-field copy-back hacks: Cinemachine's
`SaveDuringPlay`, which explicitly "doesn't save structural changes, like adding or removing a
behavior", and Unreal's *Keep Simulation Changes*, which "only works for Actors which were already
placed in your level before you began simulating" and copies with an explicit
`OnlyCopyEditOrInterpProperties` filter. Both need a per-field "editable" classification that nobody
here has. Owner question 2.

### 8.4 What undo does NOT cover, stated

- **Document open/save.** `document.open` discards the ring; `document.save` is not undoable.
- **Play start/stop.** Not undoable; the play world is not the document.
- **Anything a UI system did outside the registry.** Which is why `EK9`'s census exists: the moment
  one panel mutates the world directly, the undo stack is silently wrong.
- **External file changes.** The editor does not watch the document file in v1.

---

## 9. The UI→command carrier — `E2.8`, kernel item `EK15`

**Decision. `OnCommand` — a `#[repr(C)] Copy` component naming a `CommandId` with a declared
argument PROVENANCE — dispatched by `ui_command_dispatch`, a system NOT generic over `Actionlike`.**

### 9.1 The gap this closes

`E2.1`'s thesis is that the button and the agent are the same call. Revision 2 asserted it and had no
mechanism for it. The shipped press path is: `OnClick(pub u16)` where the `u16` is an `Actionlike`
dense index resolved at authoring time
(`crates/boyko_ui/src/interaction/action.rs:15-22`), consumed by `ui_dispatch_system::<A>` which
writes `ActionState::<A>::ui_press(index)`
(`crates/boyko_ui/src/interaction/dispatch.rs:42-63,100-103`). That is exactly the surface §1 rejects
as non-extensible — 256 fieldless variants, write-once registration, no arguments, no return, no
eligibility, no undo. There is no carrier anywhere in the tree for "this node invokes command X with
arguments Y", so `ui.click(name)` could not have been "the same path" as a human click, and the two
surfaces would have diverged at `V5` — the precise drift §1 exists to prevent.

### 9.2 The mechanism

```rust
#[repr(C)]
#[derive(Component, Clone, Copy)]
pub struct OnCommand {
    pub cmd:  CommandId,   // u16 into the registry; NO_COMMAND = u16::MAX
    pub argv: ArgSource,   // where this node's arguments come from
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ArgSource {
    pub kind:   ArgKind,   // Unit | InlineId | NodeText | NodeValue | Provider(ProviderId)
    pub inline: [u8; 8],   // an inline StableId for InlineId; a ProviderId for Provider
}
```

**Revision 4 replaced an `ArgKind::Selection` variant that could not have been compiled.**
`ArgSource` lives in `boyko_ui`; `Selection` is a `boyko_editor` resource; `E1.2`'s one-way rule
forbids the dependency that would let one name the other, so the mechanism this whole section
exists to provide was, as written, unbuildable. The fix is an indirection the engine already uses
four times over: `ArgKind::Provider(ProviderId)` indexes a **cold fn-ptr table**

```rust
type ArgProviderFn = fn(&EcsMaster, &mut ArgWriter) -> Result<(), ErrorCode>;
```

owned by the command kernel in `boyko_ecs` and filled by `App::register_arg_provider(id, f)` at
plugin build — the same shape as the `CLONE`, `MAP_ENTITIES`, `SERIALIZE` and `HOOKS` tables.
`boyko_editor` registers the selection provider under a `ProviderId` its own `command_table!` names;
`boyko_ui` never learns the word "selection". A provider that is not registered is a coded refusal
(`E1903 NoArgProvider`) rather than a panic, because an unregistered provider is a composition
mistake an agent can be told about. The table is read once per fired edge — cold by construction, no
`dyn`, no `HashMap`, no allocation.

`ui_command_dispatch(world: &mut EcsMaster)` runs where `ui_dispatch_system` runs — after
`ui_focus_system`, inside the input window — and for each fired edge: resolve `OnCommand` on the
origin node; materialise the argument bytes according to `kind` (nothing; the inline `StableId`; the
node's `UiTextBuffer`; the node's `UiValue`; whatever the registered provider writes); push one `CommandEnvelope`
into `CommandInbox`.

**The argument PROVENANCE is the load-bearing idea**, and it is the survey's own resolution to
Blender's documented defect #1 ("can't pass data such as objects, meshes or materials to operate on —
operators use the context instead"). Emacs splits ONE implementation into TWO argument sources: an
`interactive` spec says how to obtain each argument from the human, while a machine caller supplies
the same arguments directly, and `called-interactively-p` distinguishes them. `ArgSource` is that
spec, in a fixed vocabulary sized for an editor: it says *where the human's value comes from*, while
the transport supplies the same typed struct directly.

### 9.3 What the click path needs from `boyko_ui`: nothing

Checked rather than assumed. `resolve_pointer` clears `click_fired`, stamps `pending_click =
Some((origin, action))` at press for ANY hovered node — reading `OnClick` only for the ACTION word,
with `read_on_click` returning `NO_ACTION` when the component is absent — and fires
`click_fired = Some((origin, action))` on a release over the same origin
(`crates/boyko_ui/src/interaction/focus.rs:462-511`). **The origin is already there regardless of
`OnClick`.** So `ui_command_dispatch` reads `click_fired.0`, looks up `OnCommand` on it, and ignores
the action word entirely. No change to `ui_focus_system`, no change to hit-testing, no `Actionlike`
anywhere on the editor's path.

### 9.4 What the SUBMIT path needs: `EK12`, one field

The Enter edge does not carry its origin. `ui_focus_system` writes
`pending_submit = world.get_component::<OnSubmit>(f).map(|s| s.0)` and `None` otherwise
(`focus.rs:556-562`), so a focused node without `OnSubmit` loses the edge entirely — and an editor
text field committing with Enter is exactly that node.

**`EK12`: `UiPointerState::pending_submit` becomes `Option<(Entity, u16)>`**, mirroring
`click_fired`'s shape and stamped on ANY Enter-with-focus. It is **behaviour-preserving for the
existing consumer**: `ui_dispatch_system` fires only when `index != NO_ACTION`
(`dispatch.rs:54-59`), so `Some((f, NO_ACTION))` is skipped exactly as `None` was. One field, two
call sites, no behaviour change to game UI.

### 9.5 What this buys, stated as the property `U5` depends on

`ui.click(name)` resolves the name to a node through `UiName`, reads its `OnCommand`, materialises
the argument the SAME way `ui_command_dispatch` does, and enqueues the SAME `CommandEnvelope`. A
node with no `OnCommand` is a coded refusal (`E1902`), which is the honest answer and is what `V5`'s
gate asserts. The human path and the agent path are the same call **by construction**, not by
discipline — which is the difference between this design and Godot's palette.

`OnClick` / `OnHover` / `OnSubmit` are untouched and remain the GAME UI's action surface. The editor
registers `ui_focus_system` and `ui_command_dispatch` — as `UiInputPlugin` — and does not register
`UiInteractionPlugin<A>` at all, which is what removes the single-`Actionlike` constraint from the
editor's path.

**And revision 4 has to add the half that statement was missing, because it is the half that makes
the editor work at all.** Dropping `UiInteractionPlugin<A>` also drops the editor's INPUT: the
`RawInputQueue` → `PhysicalInput` drain, and the `begin_frame` that keeps the raw ring from
overflowing, live inside `update_action_state<A>` and are registered only by `InputPlugin<A>`
(`crates/boyko_input/src/action/process.rs:51-77`, `crates/boyko_input/src/plugin.rs:110-140`).
`ui_focus_system` reads `world.resource::<PhysicalInput>()` unconditionally
(`crates/boyko_ui/src/interaction/focus.rs:145-151`), so an editor App composed as revision 3
described would have panicked on its first frame. `EK17` splits the non-generic half out as
`RawInputPlugin`, and `EDITOR-UI.md` §6 states the resulting path end to end. The lesson is the one
this repository keeps re-learning: **"the editor does not register X" is a claim about what X was
carrying, and X was carrying two things.**

---

## 10. Alternatives to the whole design, priced

| alternative | price | verdict |
|---|---|---|
| Automation as a separate surface beside the UI (the owner's CLI hypothesis taken literally) | the drift measured in §1's second pile, in four independent products | rejected; the CLI ships as an adapter (`EDITOR-TRANSPORT.md` §2) |
| A palette of pre-bound nullary thunks (Godot) | no call-time arguments, no result, no schema, no undo coupling | rejected |
| Menu-path strings as the handle (Unity) | the stable handle is a presentation artefact; reorganising a menu breaks callers | rejected |
| An immediate-mode command applied on the UI thread at click time | removes the inbox and the ordering statement, and re-introduces the human-vs-agent race | rejected |
| Undo by whole-world snapshot per command | simple and correct, and it costs a full `save_world` per keystroke | rejected |
| Undo as a separate manual API the author must call (Godot's `EditorUndoRedoManager`) | makes undoability a property of DISCIPLINE rather than of the operation — the drift §1 refuses | rejected |

---

## 11. Out of scope, and what this file did NOT establish

**Out of scope (recorded):** command aliases and user keybindings; a macro/record-and-replay of
command sequences as an editor feature (the replay system covers the debugging case); optimistic
concurrency on `world.hash`; multi-document undo rings; per-command permission scopes beyond the
destructive-confirm posture; command timeouts (a handler is assumed bounded — see the fault model's
rule 6).

**NOT established:**

- The per-frame cost of the empty-inbox check and of `apply_commands` as an exclusive system head of
  `Main`. Predicted negligible; `V1` reports it rather than assuming.
- Whether `#[derive(Fields)]` can describe every component the inspector will meet, or whether a
  common shape (an enum, a `Quat`, a fixed array) needs a `FieldKind` variant the first draft lacks.
  `V1` will find this out on the fixture set; the risk is a wider `FieldKind`, not a redesign.
- Whether the `#[command]` census can see a site inside a macro expansion. If a command is ever
  declared by another macro, the census's `grep` shape breaks; v1 forbids it and `EK9` should state
  that as a rule rather than discover it.
- The size of the generated `COMMANDS.json` and whether it is reviewable at ~35 commands (expected
  yes; at 300 it is not, and the manifest's purpose changes).
