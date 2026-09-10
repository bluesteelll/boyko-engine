# E3 — The transport: how a neural network actually issues a command

**Part of the editor campaign design.** Index and v1 ladder: `EDITOR-DESIGN.md`. **Revision 4.**
**Revision 4** did not change this file's decisions; the transport's producer-side contract it had left implicit is now stated as `E2.9` in `EDITOR-COMMANDS.md` §4.2.
Decisions `E3.1`–`E3.5`. **The transport is decided separately from and AFTER the architecture**
(`EDITOR-COMMANDS.md`), because deciding it first fixes the argument model to strings and the result
model to stdout before the registry exists.

| § | decision |
|---|---|
| §1 | `E3.1` — the wire |
| §2 | `E3.2` — the owner's CLI hypothesis, priced |
| §3 | `E3.3` — the agent surface, and the tool-count problem |
| §4 | `E3.4` — observation |
| §5 | `E3.5` — the safety posture |
| §6 | out of scope, and what this file did NOT establish |

---

## 1. The wire — `E3.1`

**Decision. JSON-RPC 2.0 over a loopback socket, in a new crate `boyko_remote`, behind an opt-in
cargo feature `remote`. The JSON codec is in-house (`EK11`).**

### 1.1 Shape

- A listener thread binds `127.0.0.1:<port>` (0 = OS-assigned; the bound port is printed on stdout as
  the first line, which is how `play.start` discovers a child's port).
- A request is decoded on the listener thread into a `CommandEnvelope` and pushed through an SPSC
  ring into `CommandInbox`. **The listener thread never touches the world** — it cannot, and that is
  the point: the world is touched only inside `apply_commands`'s exclusive window
  (`EDITOR-COMMANDS.md` §4).
- The response is written back by the listener when the envelope's result lands in the outbox, keyed
  by `RequestId`.
- `rpc.discover` returns the registry as an OpenRPC-shaped document, with argument AND result schemas
  from `EK4`'s `FieldTable`s.

**Why JSON-RPC 2.0 rather than a bespoke frame.** It is what MCP, LSP and BRP all speak, so the MCP
bridge (§3) is a translation rather than a design; request/response/notification covers everything v1
needs; and the error object already has the two-level split `E2.5` requires.

### 1.2 `EK11` — the in-house JSON codec, and its bound

The engine's no-third-party rule binds for engine libraries, so there is no `serde`. The scope-creep
risk is real — JSON invites completeness — so the bound is written down and anything past it is a
separate decision with its own justification:

**In:** objects, arrays, strings with the standard escapes (including `\uXXXX`), numbers decoded as
`i64` when integral and `f64` otherwise, `true`/`false`/`null`; one-indent pretty printing; a
borrowing parser over a `&[u8]` with no intermediate `String` for keys.

**Out:** streaming/incremental parsing, arbitrary-precision numbers, comments, trailing commas,
duplicate-key policies beyond last-wins, a serde-style derive (the `FieldTable` walk is the derive),
and any pretty-printing option beyond the one indent.

### 1.3 Alternatives, priced

| alternative | price | verdict |
|---|---|---|
| A binary protocol of our own | faster and smaller, and every client — including the MCP bridge and any debugging `nc` session — becomes bespoke. The commands are user-scale, not frame-scale: an agent issues tens per second, not millions | rejected |
| Named pipes / UDS | avoids the TCP stack and the port; two platform implementations instead of one, and no gain at this rate | deferred |
| Embedding an interpreter (Blender's and Unreal's route) | interactive and introspectable, and it costs a runtime plus a binding layer, and Unreal's own documentation records that its Python is EDITOR-ONLY — "not when your Project is running … including Play In Editor, Standalone Game, cooked executable" — which is exactly the split this design refuses | rejected |
| Reuse the existing env-var surface | ~20 launch-time knobs read ONCE at boot, delivering results as files or as process exit; it cannot observe or change a RUNNING world, which is most of what an agent needs | rejected — it is the baseline this replaces |

---

## 2. The owner's CLI hypothesis, priced honestly — `E3.2`

The owner's hypothesis, to be tested rather than adopted: *"I think we could make a CLI utility that
has a command for every engine function."*

**Verdict: right about the CATALOGUE, wrong as the ARCHITECTURE, and the difference is measurable.**

### 2.1 What a CLI supplies, against what an agent needs

MCP's tool specification is, in effect, the requirements document for "AI-callable". A tool declares
`name`, `description`, an `inputSchema` (JSON Schema, **REQUIRED**), an optional `outputSchema`, and
behavioural annotations; results carry structured content that MUST conform to the declared output
schema; errors split into PROTOCOL errors and TOOL-EXECUTION errors (`isError: true`) because the
spec says an execution error should reach the model for self-correction while a protocol error is
"less likely to result in successful recovery"; and `notifications/tools/list_changed` lets the
surface change at runtime.

| an agent needs | a CLI printing to stdout supplies |
|---|---|
| enumerable tools | ✔ (`--help`) |
| INPUT schemas | ✘ |
| OUTPUT schemas | ✘ |
| structured results | ✘ (prose) |
| protocol-vs-operation error split | ✘ (one exit code) |
| change notification / a live state stream | ✘ (no state between invocations) |

One of six. And the missing five are not polish: a tool whose arguments are undocumented text is a
tool the model must guess at, which is the failure mode a schema exists to prevent.

### 2.2 What ships: `boyko-ctl`

The CLI **ships**, and it costs almost nothing because it is GENERATED from the registry: one
subcommand per command, `--help` from the `CommandDesc::doc`, arguments parsed against the
`FieldTable`, results printed as JSON. It is genuinely useful — for a human at a prompt, for a shell
script, for a CI step — and it can never drift from the registry because it is not written by hand.

It is an **adapter**, not the architecture. So is the socket. So is the MCP bridge. **What cannot be
retrofitted is the registry** (`EDITOR-COMMANDS.md` §1).

### 2.3 The catalogue half, which is right

"A command for every engine function" as a REGISTRY is exactly right, and it is what `E2.7`'s
completeness gate is for. As an always-loaded TOOL LIST it is self-defeating (§3.1). The owner's
instinct and the measurement disagree only about the second half, and the design keeps the first.

---

## 3. The agent-facing surface, and the tool-count problem — `E3.3`

**Decision. An MCP bridge exposing ≤ 25 CURATED tools, plus `describe_commands(query)` and
`invoke(name, args)` over the FULL registry.**

### 3.1 The quantification the decision rests on

Four independent sources, all measuring the same degradation:

- **RAG-MCP** (arXiv 2505.03275): tool-selection accuracy **13.62 % → 43.13 %** with retrieval-based
  tool selection, plus > 50 % prompt-token reduction. The 13.62 % baseline IS the all-tools-loaded
  case.
- **Anthropic's platform documentation**, stated flatly: "Claude's ability to pick the right tool
  degrades once you exceed 30–50 available tools", with a concrete context cost — a five-server setup
  is ~55 k tokens of definitions before any work.
- **GitHub** cut Copilot's MCP integration from **40 tools to 13** for a reported 2–5 percentage-point
  gain on SWE-bench-Verified plus 400 ms of latency.
- **A shipping Blender MCP vendor** states that models choosing among **131 outperform models choosing
  among 404**, and ships 404 tools behind a 131-tool default profile plus a `describe_tools(query)`
  search.

This engine carries **188 `derive(… Component …)` occurrences under `crates/*/src`** at this checkout
(counted, not estimated; 54 files). One tool per component type alone blows past every threshold
above, and the command set is larger than the component set. Nothing about Rust or zero-overhead
changes this: it is a property of the model's context, not of the server.

### 3.2 The curated set (the v1 list, ≤ 25)

`session.ping` · `session.describe` · `describe_commands` · `invoke` · `world.query` · `world.hash` ·
`entity.spawn` · `entity.despawn` · `entity.get` · `component.list` · `component.get` ·
`component.get_field` · `component.set_field` · `selection.get` · `selection.set` · `ui.tree` ·
`ui.click` · `ui.type` · `undo` · `redo` · `document.save` · `play.start` · `play.stop` · `log.tail`
· `screenshot`.

Twenty-five. Everything else — and everything added later — is reachable through `invoke` after
`describe_commands`, which is the Blender vendor's shape and the one the measurements support.

**Naming.** Dotted, namespaced by SUBJECT (`entity.*`, `component.*`, `ui.*`), because Anthropic
reports "non-trivial effects" on eval performance from the prefix-vs-suffix choice alone, and because
a consistent prefix means one `describe_commands` query matches a whole group.

### 3.3 Schemas, because a name without a schema is half a tool

Every curated tool's `inputSchema` and `outputSchema` are generated from `EK4`'s `FieldTable`s. This
is the explicit inverse of Bevy's measured defect: its `rpc.discover` returns an OpenRPC document
whose parameter arrays are EMPTY, because the schemas live in the reflection type registry while the
methods are serde-based — two sources, and they drifted. Here there is one source per struct
(`EDITOR-COMMANDS.md` §3.1), and `V3`'s gate asserts a non-empty schema for every command, counted.

### 3.4 Response size

Anthropic's guidance measures the difference: a "concise" versus "detailed" response format at 72
versus 206 tokens for the same information; Claude Code caps tool responses at 25 000 tokens by
default; a Microsoft Research survey of 1 470 MCP servers found a median tool output of 98 tokens and
a maximum of 557 766.

So: `world.query` takes a `limit` (default 100) and returns a `next_cursor`; every list command
paginates; `entity.get` returns component NAMES plus byte lengths by default and full values only for
a named subset; `ui.tree` takes a `root` and a `depth`. Truncation is always explicit — a `truncated:
true` field and a count — never silent.

---

## 4. Observation — `E3.4`

**Decision. Observation is a first-class half of the surface, not an afterthought.**

Every transport that made agents effective supplies a read side: BRP's `+watch` variants, MCP's
Resources, Unreal Remote Control's `describe`. A CLI supplies none. And the AI-orientation
requirement set this repository already wrote argues the point with its own numbers: **"budget order
therefore: diagnostics > introspection > formatter > grammar tweaks"**, and "tools that provide
information the model cannot derive (what exists, what fired, what is canonical) never depreciate".

### 4.1 The read surface

| command | returns | why |
|---|---|---|
| `world.query` | entities matching a component filter, paginated | "what exists" |
| `entity.get` | an entity's component list (names + byte lengths), values on request | "what is this" |
| `component.get_field` | one field by path | "what is this value", the cheapest possible read |
| `ui.tree` | the UI tree with `UiName`, geometry (`ComputedRect`) and each node's `OnCommand`, if any | "what is on screen and what will it do if I click it" (`EDITOR-UI.md` §5.3) |
| `log.tail` | the last N diagnostic records with codes | "what fired" — the highest-value row, per the requirement set above |
| `log.explain` | a code's registered explanation | "what does that mean", already machine-fetchable (`boyko_log`'s `explain`) |
| `world.hash` | the current world digest | "did anything change" at one number; `EK16` (`EDITOR-REPLAY.md` §1.5) |
| `screenshot` | a PNG, downscaled, with an explicit scale | the courtesy, priced in §4.3 |

### 4.2 Change notification

v1 does not implement subscriptions. `world.hash` is the poll-shaped substitute and it is cheap
enough to be called every turn. It is also honest: the engine HAS change detection (per-row ticks),
so a `+watch`-style subscription is reachable later without new machinery, and stating that here
means the v2 shape is known rather than invented under pressure.

### 4.3 Pixels, priced

Screenshots are the courtesy, not the primary channel, and the numbers say why.

- **Token cost.** Claude bills images as 28×28 patches: a 1920×1080 frame is **1560 visual tokens**
  on the standard tier and 2691 on the high-resolution tier. Per-image cap 10 MB base64; a request
  with more than 20 image blocks applies a stricter per-image dimension limit to EVERY image in it.
- **The engine's only capture path today is unusable for this.** `BOYKO_HOST_DUMP=<path.bmp>` writes
  an UNCOMPRESSED BMP once and exits (`crates/boyko_app/src/host_dump.rs:1,45,61-68`). 1920×1080
  BGRA is ~8.3 MB raw and ~11 MB base64 — over the per-image cap before any budget discussion.
- **Encoding is a 20–90× lever.** A vendor measured a 640×480 capture at PNG 178 507 B / JPEG q82
  8 420 B / WebP q82 1 964 B and switched the default so the agent could look constantly rather than
  rarely.
- **`EK10`**: an in-house PNG **stored-block** encoder (no compression, valid PNG — `boyko_image` is a
  DECODER today) plus a box downscale, with `screenshot{scale}` defaulting to 0.5 and a hard byte
  budget. A real DEFLATE compressor behind it is out of scope and recorded.

**And grounding is why pixels are not the primary channel at all.** On OSWorld the best reported model
reached 12.24 % against 72.36 % human, with **over 75 % of failures attributed to mouse-click
inaccuracy** and a 60–80 % drop when layout shifted. `ui.tree` plus `ui.click(name)` removes the
dominant failure mode rather than optimising it (`EDITOR-UI.md` §5.1).

---

## 5. The safety posture — `E3.5`

- **The socket binds loopback only**, and a connect token is generated per session and printed on
  stdout beside the port. Unreal's Remote Control ships unauthenticated and its docs say so; this one
  is not better than that by accident.
- **Destructive commands from an AGENT require `confirm: true`** — `entity.despawn`,
  `document.save` over an existing file, `play.stop`, `replay.stop`. From the editor's own UI they do
  not, because a human clicked. A missing confirmation is `E1909 ConfirmRequired`, naming what would
  happen. MCP's spec asks for a human in the loop able to deny an invocation and marks server-side
  annotations UNTRUSTED, so the check lives on the SERVER, where it cannot be annotated away. Owner
  question 6 confirms the list.
- **Command text and arguments arriving over the transport are DATA.** Nothing in an argument is
  interpreted as a command name, a path outside the document root, or a shell fragment. `document.*`
  paths are resolved against a configured root and a traversal is a coded refusal.
- **`not(remote)` builds contain no listener**, asserted by `V3`'s symbol census.

---

## 6. Out of scope, and what this file did NOT establish

**Out of scope (recorded):** named-pipe / UDS transports; authentication beyond the per-session token;
TLS (loopback only); subscriptions / `+watch` (§4.2); a real DEFLATE compressor behind `EK10`; MCP
Prompts and Sampling (the bridge exposes Tools and, later, Resources); multi-client arbitration (v1
accepts ONE client at a time and refuses a second with a coded error, which is the policy a Godot MCP
server also settled on); rate limiting.

**NOT established:**

- **The size of the in-house JSON codec.** Bounded in writing (§1.2), unmeasured. If it exceeds the
  bound, that is a finding to report, not a bound to widen quietly.
- **Whether one MCP tool call per command is the right granularity for the curated 25**, or whether
  some should be consolidated (Anthropic's guidance is to build one high-leverage tool where a naive
  port gives three). `V6` will see the first real agent transcripts; the curated list is explicitly a
  starting set, and it is cheap to change because it is a list, not a mechanism.
- **The latency of the socket round trip under the editor's frame cadence.** A command waits for the
  next `apply_commands` window, so the floor is one frame; whether that is acceptable for an agent
  issuing tens of commands is unmeasured, and `V3` reports it rather than assuming.
- **Whether `world.hash` is cheap enough to be called every turn** on a real document — it walks the
  determinism set (`EDITOR-REPLAY.md` §1.5). `V4` reports the cost; if it is not cheap, the answer is
  a cached digest invalidated by the command counter, not a weaker digest.
