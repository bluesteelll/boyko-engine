Confirmed: `query_entities` allocates a fresh `Vec` every call (Major valid — must early-return before any tree read). The cascade reads live `Children` at apply-time and the unlink is itself deferred; ordering between a same-window reparent unlink and the doomed parent's cascade is NOT guaranteed safe — the robust fix is `clear_children`/`remove_children` on doomed parents (or `despawn_without_children` + explicit subtree handling). I have all facts I need to revise. Writing the full revised plan now.

# Architecture: GUI Phase P3 — the `.ui` text format + hot-reload

## Goal

A hand-written, serde-free `.ui` text format that lowers AT RUNTIME (via `Commands`) to the **byte-identical entity tree** the P2 `ui!` macro produces (same entities, same component set, same component byte-values, same archetype, same `ChildOf`/`Children` topology + initial `Children` order, same `UiName`), plus mtime/size-poll hot-reload that diffs by `#name` (`UiName`) and patches in place while **preserving transient runtime state** (focus/scroll/hover — components the text never mentions).

Performance posture: the parser/loader is a **cold, dev-time path** (load once, reload on file change at ~250 ms poll cadence). It allocates freely; it is NOT on the per-frame hot path. The only per-frame concern is the watch system's throttle + `metadata()` syscall, which is bounded to one `metadata()` call per poll interval AND must early-return with **zero allocation and zero tree read** on the no-change path (Decision 7, gated by a `zero_alloc` test). The set-if-changed discipline on patches is the one place where correctness touches the Principle-10 change-detection hot path: a spurious tick re-runs layout/render every reload.

## Context and constraints

Affected subsystems:
- `boyko_ui` — new modules `text/` (parser, AST, dispatch, lowering, serializer) and `reload/` (watch resource + system + reconciler); new `UiPlugin`. **One change to an existing `boyko_ui` file**: `components.rs` gains a documented `Ord`/`PartialOrd` on `UiName` (Decision 9 / C1-fix) and a new private `UiSourceOrder(u32)` component (C-anon fix). Both are additive.
- `boyko_ecs` — **zero core changes** (mirrors P18 Plugin and the InputPlugin precedent — uses only the public `Commands`/`EntityCommands`/`Res`/`ResMut`/`App`/`Plugin`/`query_entities` API). The reconcile uses the existing `EntityCommands::clear_children` / `remove_children` / `despawn` / `despawn_without_children` / `set_parent` surface verified at `entity_commands.rs:329-412`.
- `boyko_macros` — **no changes for P3.** Dispatch is Pattern A (closed `match`), not a derive (Decision 3).

Invariants that MUST be preserved:
1. **Initial-load** `.ui`-tree ≡ `ui!`-tree (the GATE). Same entities, same component-id SET per node, same component byte-values, same archetype id per node, same `ChildOf`/`Children` topology, same INITIAL `Children` order, same `UiName`.
2. The P2 lowering's FIFO ordering: parent `spawn` enqueued before child `add_child` (`ChildOf` insert) so the dangling-parent guard passes at the drain (`LinkChildCommand::apply`, `hierarchy/commands.rs:100`).
3. `UiNodeBundle` fast path: spawn the 2-component bundle when {`UiLayout`,`ComputedRect`} both present (Phase-8.5 static-archetype-cache hit); else spawn `UiLayout` and inject `ComputedRect::default()` (`boyko_macros/src/lib.rs:3900-3938`).
4. Recoverable per-line parse (the `.keys` `ParseReport` contract): one malformed line is reported + skipped, the rest parses; the file-level parse never fails (`grammar.rs:6-8`, `:85-137`).
5. Set-if-changed on reload patches (Principle 10): an unchanged component must not bump its `Tick`.
6. The Phase-19 consistency window: links materialize at the apply drain; all spawn/insert/link/despawn go through `Commands`, applied in the engine's drain windows.

Target metrics:
- Parse: O(lines), zero backtracking, single pass, no lookahead (Decision 1 + C3-fix). Allocations bounded by AST node count + per-line scratch (free — cold path).
- Reconcile: O(N) where N = live + new node count, via a per-parent key index (sorted `Vec<(UiName, Entity)>` + binary search over a documented `Ord` — never value-indexed, the `LoadEntityMap` F1 anti-DoS lesson).
- Watch: ≤ 1 `metadata()` syscall per poll interval (default 250 ms); **zero allocation AND zero tree read** on the no-change path (gated).
- Reload patch: zero spurious change-ticks on an unchanged-component survivor (gate-tested).

## Key decisions

### Decision 1: Structural model — indentation = node nesting; EVERY node names its head with a `#name` OR carries an inline component; lookahead-free classification

**What**: A `.ui` file is an off-side-rule indentation tree. The grammar is pinned so a one-pass stack machine classifies every line with **zero lookahead** (C3-fix). The rules:

- A node's **head line** is either `#name [inline-component]` OR a bare `Component {…}` line. A head line opens a node and a nesting level.
- A line beginning with `#` is **always** a node head (the `#` sigil is unambiguous).
- A line beginning with a component name (an `IDENT`) is classified by the **stack-top node's recorded attach-indent** (set when that node opened): a `Component` line at **exactly** the stack-top node's `head_indent + STEP` is an **attached component** of that node; a `Component` line at any **deeper** indent (a child-node indent) is an **anonymous child node head**. Equality vs deeper is a pure integer compare of the line's indent against `stack.top().head_indent + STEP` — no lookahead, no backtracking.
- Canonical form: 4-space `STEP`, spaces only; `version=1` first; a node head at indent D; its attached components at D+STEP; its child-node heads at D+2·STEP (and a child's own attached components at D+3·STEP).

```
version=1
#root  UiLayout { layout_type: Column, width: Stretch(1), height: Stretch(1) }
    UiSpacing { padding_left: Px(8), padding_top: Px(8) }
    #row  UiLayout { layout_type: Row, column_gap: Px(4) }
        #ok      UiLayout { width: Px(80), height: Px(24) }
        #cancel  UiLayout { width: Px(80), height: Px(24) }
```

**Why**: The GUI plan demands "flat, every widget named, explicit, ONE canonical form; a `.ui` and the equivalent `ui!` mutually obvious." Indentation maps 1:1 to the macro's `children:` block nesting; a `Component { field: value }` line maps 1:1 to a macro component literal. This is the most LLM-authorable shape (research §3, recommendation (i)). The C3-fix — classifying purely by the stack-top's recorded attach-indent — removes the lookahead the original "tentative attach vs child" rule needed: because each open node stores its own `head_indent`, the classifier never has to peek at the next line to decide whether the current line attaches or opens a child.

**Alternatives**: (ii) one node per line, inline components (bsn-like) — rejected: cramming components on one line hurts readability/diff-ability and diverges visually from the macro's per-component literal. Brace-delimited nesting like the macro — rejected: braces + indentation is redundant; indentation alone is the `.keys`-precedent-extending minimal grammar.

**Trade-off**: Indentation is significant → mixed tabs/spaces and off-step indent are error classes the parser detects (recoverable, Decision 6). A multi-component node spans multiple lines (slightly more verbose than one mega-line), accepted for readability. An anonymous child node is permitted (head line is a bare `Component`) but the parser WARNS on any anonymous node carrying state-bearing structure (C-anon fix, Decision 11).

### Decision 2: Comment lead is `//`, NOT `#`; exact two-byte quote-aware rule

**What**: Comments start at the first unquoted `//` (two consecutive `/`). `#` is reserved exclusively for the `#name` prefix. The precise rule (C/strip-comment fix): scan bytes; track `in_quotes` toggled by `"`; a comment begins at index `i` iff `bytes[i] == b'/' && bytes[i+1] == b'/' && !in_quotes`. A lone `/` (e.g. a future path-like token `a/b`) is literal. P3 has NO quoted string values; the `in_quotes` machinery is retained verbatim from the `.keys` `strip_comment` shape so P5b (quoted SDF text) inherits it without a rewrite, and is tested now with a synthetic quoted line.

**Why**: `.keys` uses `#` for comments (`grammar.rs:453`); `.ui` uses `#` as the name sigil (matching `ui!`'s `#name`). Reusing `#` for both would force token-boundary disambiguation on every line; `//` eliminates the collision. The two-byte lookahead is the one off-by-one risk and is pinned + tested (Decision 6 / metrics).

**Alternatives**: token-boundary disambiguation of `#` — rejected: fragile. `;` lead — rejected: less familiar.

**Trade-off**: `strip_comment_slashslash` is a new function (the `.keys` single-byte `#` version is NOT reused verbatim — it is the structural template). Minor.

### Decision 3: Reflection-free dispatch — closed-vocabulary `match` (Pattern A), NOT a derive table (Pattern B)

**What**: A single hand-written `parse_and_insert(name, fields, &mut EntityCommands, line_no, &mut report) -> Result<(), LineError>` dispatching on the component's text name over the closed `boyko_ui` builtin set. Each arm calls a per-component `parse_<component>(fields, line_no, &mut report) -> Result<T, LineError>` then `ec.insert(T)`. An unknown name returns `Err(UnknownComponent)` (recoverable). The dispatch keys on the TEXT name, which by invariant EQUALS the component's Rust type name (so the string key and the type stay in lockstep — enforced by an exhaustiveness test, metrics).

```rust
fn parse_and_insert(name: &str, fields: &str, ec: &mut EntityCommands,
                    line_no: usize, rep: &mut UiParseReport) -> Result<(), LineErr> {
    match name {
        "UiLayout"     => { ec.insert(parse_ui_layout(fields, line_no, rep)?); }
        "UiSpacing"    => { ec.insert(parse_ui_spacing(fields, line_no, rep)?); }
        "UiAlign"      => { ec.insert(parse_ui_align(fields, line_no, rep)?); }
        "UiAbsolute"   => { ec.insert(parse_ui_absolute(fields, line_no, rep)?); }
        "ContentSize"  => { ec.insert(parse_content_size(fields, line_no, rep)?); }
        "ComputedRect" => { ec.insert(parse_computed_rect(fields, line_no, rep)?); }
        "StackIndex"   => { ec.insert(parse_stack_index(fields, line_no, rep)?); }
        "ComputedClip" => { ec.insert(parse_computed_clip(fields, line_no, rep)?); }
        "UiRoot"       => { /* ZST tag; fields MUST be empty else recoverable err */ ec.insert(UiRoot); }
        // UiName is NOT dispatched here — it comes from the #name sigil only.
        other => return Err(LineErr::UnknownComponent(other.into())),
    }
    Ok(())
}
```

**Why**: (1) The text face deliberately parses a **restricted literal grammar**, not arbitrary Rust. (2) `.ui` files are user-editable/untrusted; a closed match makes "this file can only construct UI components" a structural safety property. (3) The builtin UI vocabulary is small, fixed, engine-owned (10 components) — exactly the `.keys` `parse_spec` closed-`match` precedent. (4) Zero new derive/registry machinery, no global-registration ordering hazard, compiler-enforced exhaustiveness, monomorphized `insert::<T>` per concrete `T`. No `Any`/downcast/`TypeId`/serde. This is genuinely codegen-not-reflection at the cheapest point on the spectrum.

**Alternatives**: Pattern B — a `#[derive]`-emitted per-component `TextParseFn` installed into a `ComponentId`-indexed table keyed by `stable_name` (the `SerializeInfo`/`install_serialize_fn` precedent, `component_registry.rs`). Rejected for P3: it buys open-vocabulary at the cost of a name→`ComponentId` resolver, the ability for untrusted `.ui` text to construct arbitrary components, and derive machinery P3 does not need. **Deferred**; the `parse_<component>` leaves are reusable as that table's entries.

**Trade-off**: Adding a new builtin widget component requires touching the match (acceptable — the vocabulary is engine-owned; matches the P2 model where widget vocabulary is P6).

### Decision 4: Per-field value parsing is TYPE-DIRECTED by the destination field; there is NO standalone "parse a value" function (C2-fix)

**What**: Each `parse_<component>` constructs `T::default()`, then `split_inner_fields(fields)` (Decision 5) → for each `key: value` part, splits key/value at the first `:`, then a `match key` writes the field via a **type-specific leaf parser selected by the `match key` arm**. The `match key` arm statically knows the field's destination type and is the SOLE arbiter of how the value is interpreted:

- A `Unit` field arm calls `parse_unit(value)` → `Px(`→`Unit::Px(f)`, `Pct(`, `Stretch(`, bare `Auto`→`Unit::Auto`. Anything else → recoverable per-field error.
- An `AlignCross` field arm calls `parse_align_cross(value)` → bare/qualified ident match: `Start`/`Center`/`End`/`Stretch` (and `AlignCross::Stretch`). A `Px(…)`/`Auto` here → recoverable per-field error.
- `LayoutType`/`PositionType`/`AlignMain` arms call their own closed-ident matchers (bare or `Type::`-qualified, prefix stripped).
- `f32` arm calls `parse_f32` (the `grammar.rs:424` pattern: `value.trim().parse::<f32>()`).
- `u32` arm calls `parse_u32`.

This resolves the genuine token-shape collision (`units.rs`: `Unit::Stretch(f32)` line 23 AND `AlignCross::Stretch` line 101; `Unit::Auto` line 25 is a bare ident exactly like an enum variant). `Stretch` resolves to `Unit::Stretch` ONLY in a `Unit` field and to `AlignCross::Stretch` ONLY in an `AlignCross` field. A unit literal in an enum field (or vice versa) is a recoverable per-field error, never a silent mis-parse. Omitted fields keep their `Default` value — the `ui!` `..Default::default()` semantics AND what makes reload set-if-changed deterministic.

**Why**: The collision makes token-shape-blind value parsing impossible. Binding the leaf parser to the statically-known destination type is the only correct design and is also the leaf-codec analog of the `.keys` `Wire`/`parse_spec` per-key dispatch. Default-then-overwrite gives stable, deterministic bytes for round-trip + diff.

**Alternatives**: a single `parse_value(&str) -> Value` enum then a per-field downcast — rejected: the `Stretch`/`Auto` ambiguity is unresolvable without the destination type, so the enum would have to carry both interpretations and the downcast would re-introduce the same per-field type match — strictly more machinery.

**Trade-off**: ~10 small hand-written `parse_<component>` fns + 6 leaf parsers (`parse_unit`, `parse_layout_type`, `parse_position_type`, `parse_align_main`, `parse_align_cross`, `parse_f32`/`parse_u32`). Each is ~15 lines.

### Decision 5: Field extraction is a two-stage scan: brace-matching for the component span, paren/quote `split_top_level` for the INNER list ONLY (C1-fix)

**What**: `split_top_level` (verified `grammar.rs:471-493`) tracks only paren depth `()` and quote state `""` — it is **NOT brace-aware**. P3 therefore uses it in a precisely bounded way and **does not call it "verbatim reuse" for the whole line**:

1. **Component-span extraction (brace-matching scan, NEW)**: on a head/attached line, after the optional `#name`, scan for the component `IDENT`, then a brace-matching scan finds the matching `}` for the opening `{` (depth counter over `{`/`}`, quote-aware) to isolate the inner field span. A line may carry at most one component (the head's inline component, or one attached component). This is a separate scan, not a comma split.
2. **Inner-list split (`split_top_level`, reused verbatim)**: the already-extracted inner field span (between the outer `{` and `}`) is split with `split_top_level`, which is sufficient because every P3 field value is one of: a number, a `Unit` call `Px(f)`/`Pct(f)`/`Stretch(f)` (paren-balanced — `split_top_level` handles it), a bare ident enum, or an int. **The P3 inner field grammar provably never contains `{`, `[`, or a quoted comma** (the closed value set in Decision 4 has no brace/bracket/quoted-string value), so paren+quote awareness is sufficient for the inner split. This is documented as an invariant and locked with a test that feeds a value containing braces/brackets and asserts it is rejected as a recoverable per-field error (so a future grammar extension that adds such a value cannot silently mis-split — it fails loudly until the split is upgraded).

**Why**: The original plan's "reuse `split_top_level` verbatim to split the field list" was correct ONLY for the inner list, never for isolating the component span on a head line (which is brace-delimited). Making the two stages explicit closes the silent-mis-split hole the critic identified while still reusing the verified paren/quote splitter where it is valid.

**Alternatives**: extend `split_top_level` to be brace+bracket aware up front — rejected for P3: unnecessary (no brace/bracket value exists), and copying-then-extending the verified function risks diverging from its tested behavior; the bounded reuse + the rejection test is the smaller, safer surface. Re-evaluate when P5b adds quoted strings/nested literals.

**Trade-off**: one new brace-matching scan (~25 lines, quote-aware). The inner-list reuse stays verbatim.

### Decision 6: Recoverable per-line/per-field errors via a `UiParseReport` mirroring `ParseReport`, with `(line, col, reason)` for field errors

**What**: `UiParseReport { version: u32, errors: Vec<(usize /*line*/, u16 /*col*/, String)>, warnings: Vec<(usize, u16, String)> }` (extending the `grammar.rs:39-50` shape with a column, the line:col-locality fix). Line is 1-based; col is a 0-based byte offset into the line (0 for whole-line errors like an unknown component; the field's offset for a bad-value error — the leaf parsers thread the field's byte offset so the column is real). Every malformed construct — bad field, unknown component, unknown enum variant, over-CAP `#name`, duplicate `#name`, inconsistent/off-step indentation, dedent-to-never-opened-column, a unit literal in an enum field — records `(line, col, reason)` and **skips that line/node**, keeping the rest. The parser never fails at the file level. `version=N` higher than `UI_FORMAT_VERSION` → warning, best-effort load; a malformed version value (`version=abc`) → recoverable error, keep `UI_FORMAT_VERSION` default (mirrors `grammar.rs:122-124`).

**Why**: The `.keys` contract: a hand-edited config can never brick the game. A `.ui` file is hand- and LLM-edited; the same guarantee is mandatory. The col extension serves the review's line:col-locality requirement for field-level errors (a bad float in the 3rd of 5 fields is locatable without eyeballing the line).

**Alternatives**: line-only attribution — rejected: the review requires line:col for field errors and the leaf parsers already have the offset in hand. Hard-fail on first error — rejected, violates the locked constraint.

**Trade-off**: the error tuple gains a `u16`. Negligible (cold path, bounded by error count).

**Recovery policy (pinned)**:
- Malformed **head/node** line → the node AND its subtree are dropped (the indent stack pops the level on the next shallower line; the subtree's lines fall through as orphans and each records nothing further once their parent is gone — see §A for the precise stack handling). Recorded once at the head line.
- Malformed **component** line → only that component is dropped; the node and siblings survive.
- Malformed **field** within a component → that field keeps its `Default`; the rest of the component parses. Recorded at `(line, field_col, reason)`.
- **Duplicate `#name`** (within the document) → record the error at the second occurrence's line; **keep the first, treat the duplicate node as anonymous** (strip its name, reconcile it positionally per Decision 11). This keeps the reconcile key index a true unique map (the macro rejects duplicates at compile time, `boyko_macros/src/lib.rs:3744-3751`; the text path enforces uniqueness at parse time by demotion).
- **Dedent-to-never-opened-column** → see §A (record, skip the line WITHOUT mutating the stack so siblings still parse against the correct parent).

### Decision 7: Hot-reload via std mtime+size poll on a throttled ECS system — no external crate, no FFI, no thread; no-change path is zero-alloc / zero-tree-read

**What**: A `UiHotReload` `Resource` holds `{ path: &'static str, last_mtime: Option<SystemTime>, last_size: u64, pending: Option<(SystemTime, u64)>, last_poll: Instant, poll_interval: Duration }`. A `ui_hot_reload_system(ResMut<UiHotReload>, Commands, /* live-tree query, lazily materialized */)` runs on `CoreSchedule::Main`. The system's strict sequence (the zero-alloc/zero-read invariant, alloc-on-no-change fix):

1. If `last_poll.elapsed() < poll_interval` → **return immediately** (no syscall, no alloc, no tree read).
2. Set `last_poll = now`. Call `std::fs::metadata(path)` and read `(modified(), len())`. On error → return (Decision 8).
3. If `(mtime, size) == (last_mtime, last_size)` → **return immediately** (no tree read, no `query_entities`, no `UiTreeView` build). This is the no-change path; a `zero_alloc` test asserts zero allocations here.
4. Settle check (Decision 8 / torn-read fix): if `(mtime, size) != pending` → record `pending = Some((mtime, size))` and **return** (wait one interval for the file to settle). Only when a subsequent poll observes `(mtime, size) == pending` (the file has been stable for ≥ one interval) does it proceed to reconcile.
5. On a confirmed-settled change: read the file, debounce (Decision 8), `parse_ui`, then build the `UiTreeView` (ONE `query_entities` call reused across the whole reconcile, not per-parent) and `reconcile_ui`. On success set `last_mtime`/`last_size` to the settled values and clear `pending`.

**Why**: `std::fs::metadata().modified()` is stable, cross-platform, zero-dep. The reload path is cold/dev-time; 250 ms (×2 for the settle interval) latency is imperceptible for design iteration. An OS-native watcher needs the banned `notify` crate or per-OS FFI + a background thread + a `!Send` channel surface, and silently fails on network/FUSE filesystems. The strict early-return sequence is what makes the per-frame no-change path truly free (the original plan implied but did not pin this; `query_entities` allocates a fresh `Vec` every call, `ecs_master.rs:2096`, so it MUST be gated behind the change branch).

**Alternatives**: `notify` crate — rejected (dep ban). Hand-rolled FFI watcher — deferred, feature-gated only. Reconcile on the first poll that sees a change (original) — rejected: it cannot distinguish "finished writing" from "mid-write at coincidentally-final size" (Decision 8).

**Trade-off**: up to ~2× poll latency (one extra settle interval) and one throttled syscall per interval. Both negligible for a dev-time feature.

### Decision 8: Torn-read / half-written-file safety via a two-poll settle + a validity net (torn-read fix)

**What**: Two independent guards:
- **Settle (Decision 7 step 4)**: a detected `(mtime, size)` change is held in `pending` and only acted on when a SUBSEQUENT poll observes the SAME `(mtime, size)` — the file has been stable for ≥ one interval. This converts "half-written file" from a best-effort content check into a deterministic 2-poll settle that catches a torn read where the content is non-empty and even structurally-valid-but-incomplete (the case mtime+size alone cannot detect: a poll firing between two `write()` syscalls at coincidentally-final size).
- **Validity net (secondary)**: on the settled read, if the read errors OR the content is zero-length OR the parse produced zero usable nodes with errors present, **skip and do not update `last_mtime`/`last_size`** (retry next interval), so a genuinely-broken state is never reconciled.

**Why**: Editors write via truncate-then-write or temp-rename, briefly producing empty/partial/missing files. Reconciling a torn file would see tail nodes as "vanished" → despawn them (losing transient state) → respawn next poll (flicker). The settle makes the common torn read invisible; the validity net catches the residual empty/broken case.

**Trade-off**: a genuinely-emptied file (user deleted all content) is treated as "not yet settled/usable" and not applied — acceptable (an empty `.ui` is not a meaningful authoring state; the user reloads by saving a `version=1`-only file). One extra `(mtime,size)` pair (`pending`) on the resource.

### Decision 9: `UiName` gains a documented total order for the per-parent diff index (C1-fix)

**What**: `UiName` (verified `components.rs:222-232`, `#[repr(C, align(64))]`, derives `Clone, Copy, Debug, PartialEq, Eq` — **no `Ord`**) gains a hand-written `Ord`/`PartialOrd` whose comparison is **`self.bytes[..self.len] cmp other.bytes[..other.len]`** (compare the meaningful UTF-8 prefix, tie-break on `len`). This is a valid total order consistent with `Eq` (two `UiName`s are `Eq` iff `len` equal and `bytes[..len]` equal; the chosen `Ord` returns `Equal` exactly then because the trailing bytes are always zero and `_pad` is always `[0;3]`). It is reflection-free (a memcmp over a POD column slice; Principle 1/5). The per-parent reconcile index is a `sorted Vec<(UiName, Entity)>` + `binary_search_by(|(k,_)| k.cmp(&target))`.

**Why**: The original plan's "sorted Vec + binary search" is not expressible without `Ord` on `UiName`, which the shipped type does not provide — it would not compile. Adding an explicit, documented `Ord` (rather than `#[derive(Ord)]` over the fixed buffer, which would also order by `_pad`) makes the comparison intentional and correct. This is the ONE change to an existing `boyko_ui` file beyond module decls, and it is additive (no existing call site changes).

**Alternatives**: keep the existing `Eq` and do a **linear scan** per parent (K = children per parent is tiny on a cold reload) — viable and needs no type change, but rejected as the primary because the sorted index is O(N log K) vs O(N·K) and the `Ord` is a 6-line documented impl; the linear scan is retained as the **debug-mode cross-check** (a `debug_assert!` that the binary-search hit equals a linear-scan hit). A fixed-capacity hash over `UiName` bytes — rejected: more machinery than a sorted slice for a cold path.

**Trade-off**: one new trait impl on an existing type. Documented and test-covered (an `Ord` ordering test: `"a" < "ab" < "b"`, and `Ord`-consistent-with-`Eq`).

### Decision 10: The reconcile operates over a SCOPED document domain stamped at spawn; live-side duplicate `UiName` is handled (live-duplicate fix)

**What**: The loader stamps every entity it spawns from a given `.ui` document with a private `UiDocRoot`-derived ownership marker so the reconciler operates ONLY over entities belonging to THIS document's tree, never over foreign `UiName`-bearing entities (host-app entities, or a sibling `ui!` invocation using overlapping names). Concretely: the document's root entities are tracked in the `UiHotReload` resource (a small inline-capacity `SmallRoots` of the root `Entity` ids returned by the initial `spawn_ui_tree`); the reconcile walks down from exactly those roots via `Children`, so the scope is structural (the subtree under the document's roots) and needs no per-entity tag beyond the roots themselves. Within that scope:
- The per-parent key index is built ONLY from the scoped parent's `Children`. Because the parser guarantees document-unique names (Decision 6 demotes duplicates), and the scope is one document, the live index keys are unique by construction for an unmodified-by-foreign-code tree.
- **Defensive**: if two scoped children under one parent nonetheless share a `UiName` (e.g. a prior reload bug, or external mutation), the index build records a recoverable report error and resolves deterministically by **lowest `Entity` id wins the key**; the other duplicate is NOT silently despawned — it is left untouched (treated as unmatched-but-preserved for that pass) and the error surfaces the corruption.

**Why**: The diff key assumes uniqueness the original plan did not control on the live side. Scoping the reconcile to the document's own subtree removes foreign-entity interference; the defensive lowest-id rule + "do not despawn the loser" prevents the binary-search-returns-arbitrary-match → despawn-the-wrong-entity failure from silently corrupting the tree.

**Alternatives**: a per-entity `UiDocId` tag on every node — rejected: redundant when the document's roots are already tracked and the subtree is reachable via `Children`; one root list is cheaper than a per-node tag column.

**Trade-off**: the `UiHotReload` resource carries the document's root `Entity` list (small, inline). The reconcile is root-anchored rather than global-`UiName`-indexed.

### Decision 11: Anonymous-node reconcile is by a STABLE stored positional key, never by `Children` slice order (C-anon / anon-soundness fix)

**What**: `Children` sibling order is **explicitly unspecified and order-perturbing on removal** (verified `hierarchy/mod.rs:94-99`, `swap_remove`). Positional matching against the `Children` slice is therefore UNSOUND after any prior structural reload. P3 instead:
- Stamps every node (named AND anonymous) at spawn with a private `UiSourceOrder(u32)` component = the node's **declaration ordinal among its siblings** in the parse tree (0-based, per-parent). This is an additive private component in `boyko_ui::components` (or `boyko_ui::reload`), `#[repr(transparent)] struct UiSourceOrder(u32)`, `Copy`/`Eq`.
- The reconcile matches an anonymous new node to the live child under the same parent whose `UiSourceOrder` equals the new node's declaration ordinal (a keyed lookup by the stored ordinal, NOT a walk of the `Children` slice in slice order). Named nodes still match by `UiName` (the stronger key); the ordinal is the fallback key for unnamed nodes.
- On reload, `UiSourceOrder` is itself patched set-if-changed if a surviving node's declaration ordinal shifted (so the stable key tracks the new layout). Because matching is by the STORED ordinal first, an insertion that shifts ordinals re-keys deterministically rather than cascading spurious despawns.
- Additionally (Decision 6 / nudge): the parser emits a WARNING for any anonymous node that has children or attached non-`UiLayout` components (a node likely to carry state), nudging authors toward `#name`.

**Why**: The critic verified that positional reconcile against `Children` patches the WRONG live entity after a swap_remove-perturbed reload — silent tree corruption, not mere "fragility." A stored, stable per-sibling ordinal makes anonymous matching deterministic and correct regardless of `Children` order. This preserves transient state on anonymous nodes across reloads as long as the declaration order is stable, and degrades gracefully (re-key) when it shifts.

**Alternatives**: (b) forbid anonymous-node reconcile entirely — despawn+respawn the whole anonymous-sibling run on every reload (accept transient-state loss for unnamed nodes) — simpler but loses state on every reload for any unnamed node, contradicting the central preservation promise; rejected as the primary, kept as documented behavior for the degenerate "ordinal collision after corruption" case. (c) synthesize a name — rejected: pollutes the `UiName` space and the round-trip.

**Trade-off**: one additive private `UiSourceOrder(u32)` component on every node (4 B, cold). It is part of the node's component SET, so the equivalence gate must account for it — see Decision 12 (the gate compares the AUTHOR-VISIBLE set; `UiSourceOrder` is stamped by BOTH paths so it cancels — the macro lowering must also stamp it, OR the gate excludes it; chosen: **the gate excludes `UiSourceOrder` from the compared set** because it is a P3-reload-only private key the macro path does not need, and excluding a private key from the equivalence comparison is sound — see Decision 12).

### Decision 12: Equivalence rationale corrected — archetype identity is SET-based, not insert-order-based; the gate is initial-load byte/topology, post-reload is value/topology (archetype-rationale + post-reload fixes)

**What**: Two corrections to the original equivalence claims:

1. **Archetype identity follows from the component-id SET + the spawn base**, NOT from insert order. Archetype identity is a set-based `ComponentMask` (verified `archetype.rs` `filtered_signature_mask`); insert ORDER does not affect the final archetype id. Insert order is load-bearing for exactly ONE observable: the FIFO `add_child` order that determines the INITIAL `Children` slice order. So the gate's "same archetype id" is satisfied by (same spawn base: bundle vs UiLayout-only) + (same component-id set); "same initial `Children` order" is satisfied by (same FIFO `add_child` order = pre-order child declaration order). The original plan over-constrained (claimed order drives the archetype) and under-specified the real constraint.
   - **Duplicate `UiLayout`/`ComputedRect` in text**: the macro picks the FIRST by `position()` and tolerates duplicates (`validate_node` only checks `any`, `boyko_macros/src/lib.rs:3760`; `lower_node` uses `position()`, `:3894-3895`). The loader MIRRORS this: `find_first("UiLayout")` / `find_first("ComputedRect")` select the first occurrence for the spawn base; subsequent duplicate `UiLayout`/`ComputedRect` lines become chained `insert`s (last-write-wins on the component, exactly as the macro's `insert` chain would). This is pinned so the gate compares against the macro's first-position semantics.

2. **The GATE is INITIAL-LOAD equivalence only** (byte + topology + archetype-id). **Post-reload, the contract is value + topology equivalence, NOT archetype-id identity.** A patched survivor reaches its final state via incremental insert/remove migrations, which legitimately differ from a fresh spawn in archetype-creation trajectory and may carry a retained-but-empty `Children` (verified `hierarchy/mod.rs:101-109`: removing the last child does not remove `Children`). Asserting post-reload archetype-id identity would flake on these. So: initial-load test asserts archetype-id identity; reload tests assert (component values + `ChildOf`/`Children` topology + entity-id stability for survivors), NOT archetype-id identity. Forcing post-reload archetype identity would require despawn+respawn on every structural change, which conflicts with transient preservation — so the value/topology contract is the deliberate, correct choice.

`UiSourceOrder` (Decision 11) is EXCLUDED from the gate's compared component set: it is a P3-reload-only private key; the macro path does not stamp it, and a private bookkeeping component that is not author-visible and does not affect layout/render output is sound to exclude from an author-intent equivalence comparison. The gate documents this exclusion explicitly.

**Why**: The critic verified that archetype identity is set-based (`archetype.rs:278`), so the original "insert order drives the archetype" rationale was wrong, and that post-reload incremental migration legitimately differs from fresh spawn (retained-empty-`Children`, migration order). Pinning the gate to initial-load byte/topology and post-reload value/topology makes the contract provable and non-flaky.

**Trade-off**: two distinct equivalence contracts (initial vs reload) must be documented and tested separately. Clearer than one over-strong contract that would flake.

### Decision 13: Move-vs-despawn ordering is an APPLY-TIME guarantee via clearing survivors from doomed parents, not reconcile bookkeeping (CRITICAL move-vs-despawn fix)

**What**: The reconcile must guarantee that a node relocated to a new parent whose OLD parent is deleted in the same reload is RELOCATED, not destroyed by the despawn cascade. The cascade reads the doomed parent's LIVE `Children` at apply time and despawns each child unconditionally (verified `children_on_replace`, `hierarchy/commands.rs:298-367`); the reparent's unlink is itself a deferred `UnlinkChildCommand` whose apply-time ordering relative to the cascade's read is NOT guaranteed (both are deferred; the cascade reads `Children` when the despawn applies, which may precede the unlink). The original plan's "despawn after the loop" is reconcile BOOKKEEPING, not an apply-time ordering guarantee.

P3 makes it an apply-time guarantee by, **before** emitting any `despawn` of an unmatched parent, explicitly **unlinking every MATCHED (surviving) child from that doomed parent**, so the cascade cannot reach a survivor:

```
// reconcile, despawn phase for a parent P being deleted (P is unmatched OR P's
// subtree is being removed):
let survivors_under_P = [c for c in Children(P) if matched[c]];  // moved-away survivors
if !survivors_under_P.is_empty():
    cmds.entity(P).remove_children(&survivors_under_P);   // deferred ChildOf removal per survivor
                                                          // (entity_commands.rs:375) — unlinks them
// THEN despawn P; its cascade now sees only the non-survivor children
cmds.entity(P).despawn();
```

Because `remove_children` removes `ChildOf` from each listed survivor (which the reparent's `set_parent` will RE-insert with the new parent — and FIFO drain orders the survivor's old-link removal before its new-link insert, `entity_commands.rs:353-355`), the survivor is no longer in P's `Children` by the time the despawn cascade runs (the removal is enqueued BEFORE the despawn on the same queue, and a `ChildOf` removal's `on_replace` unlink applies before a later despawn on the same FIFO drain). The cascade then despawns only the genuinely-vanished children. The survivor's `set_parent(new_parent)` (emitted during its own reconcile-match) links it under the new parent.

**Ordering pinned**: per parent being deleted, emit in this order on the command queue: (1) `remove_children(survivors)`, (2) `despawn(P)`. Across the whole reconcile, all survivor `set_parent(new)` reparents are emitted during the top-down match pass (before the bottom-of-tree despawn pass), and each doomed parent's `remove_children` precedes its `despawn`. This makes the unlink-before-cascade-read a FIFO-drain consequence, not a hope. A dedicated test (metrics) asserts: "move #x to B AND delete its old parent A in the same reload → #x survives under B with transient state intact, A and its other children are gone."

**Why**: The critic verified the cascade reads live `Children` unconditionally and the reparent unlink is deferred; the original ordering was bookkeeping-only. Explicitly clearing survivors from a doomed parent before despawning it converts the guarantee from "hope the queue interleaves favorably" to "the survivor is provably absent from P's `Children` when the cascade reads it."

**Alternatives**: (a) a separate apply window between reparents and despawns (drain, then emit despawns into a second `Commands` batch) — viable but requires the reconcile to span two systems / two drain windows (more invasive, and the reconcile runs as one system); rejected in favor of the same-window `remove_children`-then-`despawn` ordering, which is a single-window FIFO consequence. (b) `despawn_without_children` on doomed parents + manual subtree despawn — rejected: it would leave genuinely-vanished children with dangling `ChildOf` unless every one is manually despawned, more error-prone than `remove_children(survivors)` + `despawn`.

**Trade-off**: one extra `remove_children` command per doomed parent that has surviving (moved-away) children — cold path, negligible.

### Decision 14: Transient-state preservation is "write only the closed text-owned set"; survivors keep transient components ACROSS archetype migration (transient-migration fix)

**What**: The patcher's binding invariant: **`patch_components` writes ONLY the closed text-owned component set** (the 10 builtin layout components minus `ComputedRect`, which is layout output — see §D). Everything else on a survivor — transient components (`UiFocus`/`UiScroll`/`UiHover`, P4+) and the private `UiSourceOrder` — is preserved by OMISSION (never written, never removed). When a reload ADDS or REMOVES a text-owned component on a survivor, the entity MIGRATES archetypes; the verified migrate path byte-copies ALL other columns (including transient ones and `UiSourceOrder`) into the new archetype, so transient state survives the migration intact — not merely "survives not being written." The §D pseudocode is corrected to guard the live read (never read a non-present column):
- text-owned component absent live, present in text → `insert` (no compare),
- present live AND in text → compare-then-conditional-`insert` (set-if-changed),
- present live, absent in text → `remove::<C>()` (deleted from the file).

**Why**: The original plan asserted transient preservation only as "not written," never as "rides the byte-copy migrate." The critic correctly flagged that a real edit ADDING a component to a focused node forces a migration; the plan must (and does) assert the transient column rides it. The §D read-guard fix prevents reading a column that does not exist on an added component.

**Trade-off**: none — this is the natural behavior of the migrate path; the plan now states and tests it.

### Decision 15: `StackIndex` and transparent newtypes use the tuple-literal text form `StackIndex(n)`, mirroring the macro (StackIndex fix)

**What**: `StackIndex` is `#[repr(transparent)] struct StackIndex(pub u32)` (verified `components.rs:180-182`); the macro emits the Rust tuple literal `StackIndex(10)`. The `.ui` text form is the **same tuple-literal spelling**: `StackIndex(10)`. The parser recognizes a component whose body is a paren-delimited single value (no `{`/`}`) as a tuple newtype: after the `IDENT`, if the next non-space is `(`, parse a single value to the matching `)` (paren-aware, reuses the existing paren scanning) and construct `StackIndex(parse_u32(inner))`. This reuses paren-aware scanning (no brace parsing), is visually identical to the macro literal, and gives the equivalence gate a single author-intent spelling on BOTH sides. The originally-proposed `StackIndex { 0: 10 }` / `{ 10 }` forms are REJECTED: `{ 0: 10 }` violates `field := IDENT ":" value` (`0` is not an `IDENT`) and has no macro counterpart, making the gate map by hand.

**Why**: The critic verified `{ 0: 10 }` has no field-splitting rule and no macro analog. The tuple-literal form `StackIndex(10)` mirrors the macro exactly, so the gate authors `StackIndex` identically on both sides and asserts byte-equal `StackIndex` columns with no hand mapping.

**Trade-off**: the grammar gains a `tuple_component := IDENT "(" value ")"` production alongside `struct_component := IDENT "{" field_list "}"`. Both are paren/brace-matched scans; `StackIndex` is the only P3 tuple newtype.

## Data structures

```rust
// ── text/ast.rs — transient parsed-node tree (cold, freely allocates) ─────────

/// A parsed component literal: its text name + the raw field/arg span. The body
/// is parsed to the typed component at lowering time (kept as a span so the
/// serializer can round-trip from the same AST). `kind` distinguishes the brace
/// struct form from the paren tuple form (Decision 15).
pub struct ParsedComponent {
    pub name: String,        // "UiLayout" | "StackIndex" — closed-vocabulary key (Decision 3)
    pub body: String,        // "width: Px(80), height: Px(24)"  OR  "10" (tuple arg)
    pub kind: CompKind,      // Struct { … } | Tuple( … )
    pub line_no: usize,
    pub body_col: u16,       // byte col of the body start (for field-error locality)
}
pub enum CompKind { Struct, Tuple }

/// A node in the transient parse tree (arena-flat: children by index, so the
/// recursive walk is index-based and the tree is one contiguous Vec — no Box,
/// no per-node heap node). Cold path; the Vec growth is free.
pub struct ParsedNode {
    pub name: Option<UiNameStr>,        // the `#name` (bounded to CAP=60 at parse; demoted to None on duplicate)
    pub components: Vec<ParsedComponent>,
    pub children: Vec<usize>,           // indices into the arena, in declaration order
    pub sibling_ordinal: u32,           // declaration ordinal among its siblings (Decision 11)
    pub head_indent: u32,               // for lookahead-free classification (Decision 1)
    pub line_no: usize,
}

/// The parse result: a flat arena + the root indices + the report.
pub struct ParsedTree {
    pub nodes: Vec<ParsedNode>,         // arena; nodes[roots[i]] are roots
    pub roots: Vec<usize>,
    pub report: UiParseReport,
}

/// A name validated to fit UiName::CAP at parse time (the runtime bound-check the
/// `UiName::new` doc-comment at components.rs:240-244 demands of the text path).
pub struct UiNameStr { pub text: String }   // len <= UiName::CAP guaranteed by the parser

// ── text/report.rs — the recoverable-error channel (mirrors ParseReport) ──────

#[derive(Clone, Debug, Default)]
pub struct UiParseReport {
    pub version: u32,
    pub errors: Vec<(usize, u16, String)>,   // (1-based line, 0-based byte col, reason)
    pub warnings: Vec<(usize, u16, String)>,
}
impl UiParseReport { pub fn is_clean(&self) -> bool { self.errors.is_empty() } }

// ── reload/state.rs — the watch Resource (Principle-0 engine storage) ─────────

#[derive(Resource)]   // a normal Resource, like UiViewport (resources.rs:27)
pub struct UiHotReload {
    pub path: &'static str,
    doc_roots: SmallRoots,                       // this document's root Entity ids (Decision 10 scope)
    last_mtime: Option<std::time::SystemTime>,
    last_size: u64,
    pending: Option<(std::time::SystemTime, u64)>,   // settle buffer (Decision 8)
    last_poll: std::time::Instant,
    poll_interval: std::time::Duration,          // default 250 ms
}

// ── boyko_ui::components additions (additive; the ONE existing-file change) ────

// Decision 9: documented total order for the diff key.
impl PartialOrd for UiName { fn partial_cmp(&self, o: &Self) -> Option<Ordering> { Some(self.cmp(o)) } }
impl Ord for UiName {
    fn cmp(&self, o: &Self) -> Ordering {
        // Compare the meaningful UTF-8 prefix, tie-break on len. Consistent with
        // the derived Eq (trailing bytes + _pad are always zero). memcmp over a
        // POD column slice — reflection-free (Principle 1/5).
        self.bytes[..self.len as usize].cmp(&o.bytes[..o.len as usize])
            .then(self.len.cmp(&o.len))
    }
}

// Decision 11: private stable per-sibling positional key for anonymous reconcile.
#[repr(transparent)]
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct UiSourceOrder(pub u32);
```

Note on eager vs lazy body parsing: `ParsedComponent.body` is a `String` parsed at lowering time. This keeps the AST a pure syntactic tree (so the serializer round-trips from the same AST). The per-field parse happens once per node at spawn/patch; cold path, negligible.

No `#[repr]`/alignment/false-sharing concerns on the AST: every struct here is cold-path, single-threaded (Decision Multithreading). `UiName` (the on-entity diff key) is the cache-line-aligned, `Copy`/`Eq`, memcmp-comparable shipped type; the diff reads it directly off the SoA column.

## Public API

```rust
// ── boyko_ui::text ────────────────────────────────────────────────────────────

/// The current `.ui` format version. A higher file version loads best-effort
/// with a warning (mirrors KEYS_FORMAT_VERSION, grammar.rs:33).
pub const UI_FORMAT_VERSION: u32 = 1;

/// Parse `.ui` source into a transient tree + a recoverable error report.
/// Never fails at the file level.
pub fn parse_ui(src: &str) -> ParsedTree;

/// Lower a parsed tree into the live world via `Commands`, producing the SAME
/// entity tree as the `ui!` macro (Decision 12 initial-load equivalence).
/// Returns the root entities in declaration order. Stamps UiSourceOrder.
pub fn spawn_ui_tree(tree: &ParsedTree, cmds: &mut Commands) -> SmallRoots;

/// Serialize a live UI subtree (rooted at `roots`) back to canonical `.ui` text.
/// `parse_ui(serialize_ui(t))` value/topology-equals `t` on canonical input.
pub fn serialize_ui(world_view: &UiTreeView, out: &mut String);

// ── boyko_ui::reload ──────────────────────────────────────────────────────────

/// Reconcile a freshly-parsed tree against the live tree, diffing by UiName
/// (named) / UiSourceOrder (anonymous), patching survivors set-if-changed,
/// spawning new keys, despawning vanished keys, relinking moved keys. Operates
/// over the document's scoped roots (Decision 10). Transient components are
/// never touched (Decision 14).
pub fn reconcile_ui(parsed: &ParsedTree, hot: &mut UiHotReload, live: &UiTreeView, cmds: &mut Commands);

// ── boyko_ui::plugin ──────────────────────────────────────────────────────────

/// Loads a `.ui` file at build and (optionally) hot-reloads it. Mirrors
/// InputPlugin (boyko_input plugin.rs).
pub struct UiPlugin { /* path, hot_reload: bool, poll_interval */ }
impl UiPlugin {
    pub fn new() -> Self;
    pub fn with_ui_path(self, path: &'static str) -> Self;     // like with_keys_path
    pub fn with_hot_reload(self, enabled: bool) -> Self;
    pub fn with_poll_interval(self, d: std::time::Duration) -> Self;
}
impl Plugin for UiPlugin { fn build(&self, app: &mut App); }
```

`SmallRoots` is a small inline-capacity root list (most files have 1 root); no internal type leaks. `UiTreeView` is a read-only borrow facade over the live UI components (a thin wrapper over query results; see Integration), built from ONE `query_entities` call per reconcile. No `dyn`, no `Box`, no `Vec<Box<_>>` in any signature.

## Algorithms for critical paths

### A. Parse (one pass, indent-stack off-side machine, ZERO lookahead)

State: `stack: Vec<StackFrame { head_indent: u32, node_index: usize }>` (open-ancestors path; a bottom sentinel = root level with `node_index = usize::MAX`, `head_indent = 0` reserved so the first real node opens at indent 0), `nodes: Vec<ParsedNode>` (arena), `roots`, `report`, `version_seen`, and a per-parent `next_ordinal` recovered from the parent frame.

```
for (idx, raw_line) in src.lines().enumerate():
    line_no = idx + 1
    content = strip_comment_slashslash(raw_line)        // pre-trim slice; '//' two-byte, quote-aware (Decision 2)
    indent  = leading_ws_width(content)                 // measured on the UNTRIMMED comment-stripped slice
    body    = content.trim()
    if body.is_empty(): continue                        // blank: no INDENT/DEDENT

    // version: mirror .keys exactly (Major version-fix)
    if !version_seen:
        if let Some(rest) = body.strip_prefix("version"):
            let rest = rest.trim_start()
            if let Some(num) = rest.strip_prefix('='):
                match parse_u32(num.trim()):
                    Some(v): report.version = v; if v > UI_FORMAT_VERSION { warn(...) }; version_seen = true; continue
                    None:    report.errors.push((line_no, col, "invalid version value")); continue
            // else: a token literally starting "version" but not "version=" → fall through to node parsing

    // indentation consistency (Decision 6): mixed tab/space, or indent not a multiple of STEP
    if indent_inconsistent(content): report.errors.push((line_no, 0, "inconsistent indentation")); continue

    // resolve nesting via the indent stack (DEDENT / EQUAL / INDENT)
    while stack.len() > 1 and indent < stack.top().head_indent: stack.pop()    // DEDENT
    // post-DEDENT mismatch check (Major dedent-fix): the line's indent must now
    // equal the stack-top's head_indent (sibling) OR be exactly top.head_indent+STEP
    // (deeper: attached component or child). Anything else = dedent to a column
    // that never opened → record + SKIP WITHOUT mutating the stack, so following
    // siblings still parse against the correct parent.
    top = stack.top()
    rel = indent as i64 - top.head_indent as i64
    if rel != 0 and rel != STEP: report.errors.push((line_no, indent, "indent does not align to a sibling or a single nesting step")); continue

    // classify with ZERO lookahead (Decision 1):
    if body.starts_with('#'):
        // ── node head (named) ──
        // rel==0 → sibling of stack-top node; rel==STEP → child of stack-top node
        parent = if rel == 0 { stack.pop(); stack.top().node_index } else { top.node_index }
        node = ParsedNode { name: parse_opt_name(body, line_no, &report)?, head_indent: indent, sibling_ordinal: next_ordinal(parent), ... }
        if head carries an inline component: node.components.push(parse_component_syntax(rest_after_name, line_no))   // brace/paren matched (Decision 5/15)
        link node under parent (roots.push or nodes[parent].children.push); stack.push(StackFrame{indent, node_idx})
    else:
        // body starts with an IDENT (a component). Classify by rel ONLY (no lookahead):
        if rel == STEP:
            // attached component of the stack-top node (Decision 1)
            nodes[top.node_index].components.push(parse_component_syntax(body, line_no))
            // do NOT push a stack frame (a component is not a nesting node)
        else:   // rel == 0 → an ANONYMOUS sibling node head whose head IS this component
            stack.pop(); parent = stack.top().node_index
            node = ParsedNode { name: None, head_indent: indent, sibling_ordinal: next_ordinal(parent),
                                components: [parse_component_syntax(body, line_no)], ... }
            link under parent; stack.push(StackFrame{indent, node_idx})
            if anonymous node later gains children or non-UiLayout components: WARN (Decision 11)
```

Duplicate-`#name`: a second occurrence of a name already seen in the document → record `(line, name_col, "duplicate ui name; demoted to anonymous")` and set `node.name = None` (Decision 6). Over-CAP name → record + truncate-to-anonymous (or skip the node — pinned: demote to anonymous, name dropped).

- **Complexity**: O(total lines); each line one push/pop-bounded stack op (amortized O(1)). No backtracking, NO lookahead (the rel-vs-{0,STEP} compare classifies every line locally).
- **Cache**: sequential scan of `src`; the arena `Vec` grows append-only.
- **Branching/SIMD**: cold path; irrelevant.

`strip_comment_slashslash`, `leading_ws_width`, `indent_inconsistent`, `parse_component_syntax` (the brace/paren-matching span extractor, Decision 5/15), `classify` are new; `split_top_level` is reused VERBATIM for the inner field list ONLY (Decision 5); `parse_f32`/`parse_u32` mirror the `grammar.rs` patterns.

### B. Lower (runtime mirror of `lower_node`, `boyko_macros/src/lib.rs:3875-3971`)

Per node, pre-order (cite the macro arm-by-arm in the module doc-comment so drift is detectable by line):
```
// 1. spawn base (mirror boyko_macros/src/lib.rs:3900-3938; first-by-position, Decision 12)
let layout = parse_first("UiLayout")     // required; missing → recoverable error, skip node (mirror the (None,_) arm :3935)
let rect   = parse_first("ComputedRect") // optional
let id = if rect.is_some() {
    cmds.spawn(UiNodeBundle { layout, rect: rect.unwrap() }).id()      // bundle fast path :3901-3917
} else {
    let id = cmds.spawn(layout).id();                                   // spawn UiLayout literal :3919-3922
    cmds.entity(id).insert(ComputedRect::default());                    // inject default :3923-3924
    id
};
// 2. chained inserts for the rest in declaration order, skipping the FIRST UiLayout/ComputedRect;
//    a DUPLICATE UiLayout/ComputedRect line is a normal insert (last-write-wins, mirrors macro)
for comp in node.components where not (first UiLayout / first ComputedRect):
    parse_and_insert(comp.name, comp.body, &mut cmds.entity(id), comp.line_no, &report)?   // Decision 3/4/15
// 3. #name → UiName LAST (mirror :3940-3945)
if let Some(name) = node.name:
    cmds.entity(id).insert(UiName::new(name.text))   // name.text already bound to CAP
// 4. UiSourceOrder stamp (Decision 11; NOT compared by the gate, Decision 12)
cmds.entity(id).insert(UiSourceOrder(node.sibling_ordinal));
// 5. children pre-order, then link (mirror :3959-3961)
for child_idx in node.children:
    let child_id = lower(child_idx, cmds)
    cmds.entity(id).add_child(child_id)   // ChildOf insert; FIFO drain → parent first
return id
```

- **Complexity**: O(nodes + components). Cold path; ordering is load-bearing, not perf.
- **Equivalence proof (corrected, Decision 12)**: the spawn base choice (bundle vs UiLayout-only) + the component-id SET determine the archetype (set-based `ComputedMask`, `archetype.rs`); the FIFO `add_child` order = pre-order child declaration order determines the INITIAL `Children` order. The loader matches the macro on BOTH: same first-position spawn base, same component set (modulo the gate-excluded `UiSourceOrder`), same pre-order `add_child` sequence. Value identity follows from Decision 4's default-then-overwrite producing the same field bytes for the same authored values, plus the pinned float canonicalization (Decision 16).

### C. Reconcile (diff-by-`UiName` for named, by-`UiSourceOrder` for anonymous; scoped, soundness-fixed)

Run top-down from the document's scoped roots (Decision 10). The despawn phase per parent applies the move-vs-despawn guarantee (Decision 13).
```
reconcile_children(live_parent: Option<Entity>, new_children: &[idx], live: &UiTreeView, cmds):
    // build per-parent key indices (Decision 9/10/11), from this scoped parent's Children
    live_named   = sorted Vec<(UiName, Entity)> over scoped children that have UiName   // binary_search (Decision 9 Ord)
    live_anon    = map<u32 ordinal, Entity> over scoped children WITHOUT UiName, keyed by UiSourceOrder (Decision 11)
    // defensive (Decision 10): duplicate UiName under one parent → record error, lowest-id wins, loser preserved
    matched: bitset over the scoped children

    for new in new_children:
        live = if let Some(name) = new.name:  binary_search live_named for name → Some(e) | None
               else:                          live_anon.get(new.sibling_ordinal) → Some(e) | None   // STORED ordinal, NOT slice order
        match live:
          Some(e):
            patch_components(e, new, live, cmds)             // §D set-if-changed (Decision 14)
            set_source_order_if_changed(e, new.sibling_ordinal, cmds)   // re-key anon (Decision 11)
            mark matched[e]
            reconcile_children(Some(e), new.children, live, cmds)       // recurse
            relink_if_moved(e, live_parent, cmds)            // set_parent(new) if its live parent != live_parent
          None:
            spawn_subtree(new, parent=live_parent, cmds)     // full §B lowering, stamps UiSourceOrder

    // despawn phase for THIS parent's unmatched children, WITH the move-vs-despawn
    // guarantee (Decision 13): a child being despawned that itself has matched
    // (moved-away) descendants must first remove_children(survivors) before despawn.
    for child in scoped Children(live_parent):
        if !matched[child]:
            survivors = matched descendants currently linked under `child`   // moved out of this subtree
            if !survivors.is_empty(): cmds.entity(child).remove_children(&survivors)   // unlink before cascade (Decision 13)
            cmds.entity(child).despawn()    // cascade now cannot reach a survivor
```

- **Complexity**: O(N log K) named (K = children per parent) + O(N) anon; O(N) total. Cold path.
- **Cache**: live reads hit the `Children` `Vec` + `UiName`/`UiSourceOrder` SoA columns (sequential per parent).
- **Relink**: `#name` is document-unique (Decision 6); a name found under a different live scoped parent ⇒ moved → `cmds.entity(e).set_parent(new_parent)` (`ChildOf` overwrite; old-link unlink before new-link insert at FIFO drain, `entity_commands.rs:353-355`). Never despawn-and-respawn (that loses transient state).
- **Despawn ordering**: despawn unmatched AFTER processing all new children (so a relocated node is matched first), and apply Decision 13's `remove_children(survivors)`-before-`despawn` per doomed parent (the apply-time guarantee).

### D. Patch a survivor (set-if-changed, read-guarded, Decision 14)

For the matched entity, the patcher writes ONLY the closed text-owned set (10 layout components minus `ComputedRect`):
```
for C in text_owned_set:
    in_text = new node declares C?
    live_has = live.has::<C>(e)?
    match (in_text, live_has):
      (true,  false): cmds.entity(e).insert(parse C)                       // added → insert, NO live read
      (true,  true ): let nv = parse C; let lv = live.get::<C>(e);
                      if nv != lv: cmds.entity(e).insert(nv)               // set-if-changed (bumps Tick only on real change)
      (false, true ): cmds.entity(e).remove::<C>()                        // deleted from the file
      (false, false): nothing
// Transient components (UiFocus/UiScroll/UiHover) and UiSourceOrder are NOT in
// text_owned_set → never written, never removed → preserved by omission, and
// they RIDE the byte-copy archetype migration when an add/remove above migrates
// the entity (Decision 14).
```
Float comparison is exact `==` on the parsed value (re-parsed identical canonical literals → identical bits — Decision 16; NaN cannot arise, the text never produces it). `ComputedRect` is layout-owned output, EXCLUDED from the patch set: an authored `ComputedRect` is a spawn-time seed only (Decision 5 / §B), never patched on reload (documented; layout overwrites it next pass). `UiSourceOrder` is patched separately (Decision 11) but is not part of the author-visible text-owned set.

## Multithreading model

- **Single-threaded throughout.** Parser, lowering, watch system, and reconciler all run as ONE ECS system (`ui_hot_reload_system`) on `CoreSchedule::Main`, plus the one-shot build-time load in `UiPlugin::build`. No shared state, no atomics, no thread spawned (the deliberate avoidance of the `notify`-crate background-thread + `!Send`-channel surface — Decision 7).
- **No file-watch thread.** `std::fs::metadata().modified()` is called inline on the poll interval.
- **`Send`/`Sync`**: `UiHotReload` is `Send + Sync` (it holds `&'static str`, `SystemTime`, `Instant`, `Duration`, `u64`, an inline `SmallRoots` of `Entity` — all `Send + Sync`) so it is a valid `Resource`. `ParsedTree`/`ParsedNode`/`UiParseReport` are function-local, never stored in a `Resource`.
- **Conflict graph**: the system takes `ResMut<UiHotReload>` + `Commands` + a read query over the live UI tree. The scheduler serializes it against any other `Commands`-using or UI-component-writing system via the existing Phase-9 conflict graph — no new synchronization. Reads of the live tree happen before the system's `Commands` are drained (the apply window), so the reconcile sees a consistent snapshot.
- **Data-race freedom**: trivially — single system, no shared mutable state across threads. All structural changes deferred through `Commands`, applied in the engine's drain windows (Phase-19 consistency window); the move-vs-despawn ordering (Decision 13) is a same-window FIFO-drain consequence, not a cross-thread concern.

## Integration

Interacts with:
- `boyko_ecs` public API only: `Commands`/`EntityCommands` (`spawn`, `entity`, `insert`, `remove`, `despawn`, `add_child`, `set_parent`, `remove_children` — `system/params/commands.rs`, `system/params/entity_commands.rs:329-412`), `Res`/`ResMut`, `App`/`Plugin`/`CoreSchedule`, `query_entities` (`ecs_master.rs:2096`), `ChildOf`/`Children` (`hierarchy/mod.rs`). **No `boyko_ecs` core changes.**
- `boyko_ui` existing: `UiNodeBundle` (`bundles.rs`), every component + `Unit`/enum (`components.rs`, `units.rs`), `UiName::new`/`CAP` (`components.rs:237,245`), `UiRoot` for live-root enumeration (`components.rs:201-208`). **One additive change to `components.rs`**: the documented `Ord`/`PartialOrd` on `UiName` (Decision 9) + the private `UiSourceOrder(u32)` (Decision 11).
- `boyko_input::persist` as the **structural template** (not a code dependency): `split_top_level` is COPIED into `boyko_ui::text` (a ~20-line pure fn; copying avoids a `boyko_ui → boyko_input` dependency) and used ONLY on the inner field list (Decision 5). `strip_comment` is the template for the new `//` variant (Decision 2). `ParseReport` shape is mirrored as `UiParseReport` (+ a col). `write_f32` is the template ONLY for shortest-round-trip `{}` formatting — NOT for the `.0` rule (Decision 16).

Changes to existing code: **none** to `boyko_ecs` or `boyko_macros`; to `boyko_ui`: the additive `components.rs` changes above + module declarations in `lib.rs` + a `prelude` re-export of `UiPlugin`.

New modules (all in `boyko_ui/src/`):
- `text/mod.rs`, `text/ast.rs`, `text/report.rs`, `text/parser.rs`, `text/dispatch.rs` (closed match + `parse_<component>` + leaf parsers), `text/lower.rs`, `text/serialize.rs`, `text/split.rs` (the copied `split_top_level` + `strip_comment_slashslash` + `leading_ws_width` + the brace/paren span extractor).
- `reload/mod.rs`, `reload/state.rs` (`UiHotReload`), `reload/system.rs` (the watch system), `reload/reconcile.rs` (the diff), `reload/tree_view.rs` (`UiTreeView` read facade).
- `plugin.rs` (`UiPlugin`, mirroring `boyko_input/src/plugin.rs`).

## Implementation plan (for the developer)

1. **`components.rs` additions (additive)** — `Ord`/`PartialOrd` on `UiName` (Decision 9) with the prefix-then-len comparison; the private `UiSourceOrder(u32)` component (Decision 11). Unit-test the `Ord` (`"a" < "ab" < "b"`, consistent-with-`Eq`).
2. **`text/split.rs`** — copy `split_top_level` verbatim from `grammar.rs:471`; write `strip_comment_slashslash` (two-byte `//`, quote-aware, returns pre-trim slice — NOT the `.keys` strip-then-trim sequence, the strip-comment-flow fix); `leading_ws_width` (on the untrimmed slice) + tab/space-and-STEP consistency check; the brace/paren-matching component-span extractor (Decision 5/15). Unit-test each: `//` cases (`#a // c`, `a/b` no-comment, `//`-only line, `"a//b"` quoted), inner-split paren cases, a value-with-braces rejection test.
3. **`text/report.rs`** — `UiParseReport` (mirror `ParseReport` + a `u16` col, Decision 6), `UI_FORMAT_VERSION`.
4. **`text/ast.rs`** — `ParsedComponent` (+ `kind`, `body_col`), `ParsedNode` (+ `sibling_ordinal`, `head_indent`), `ParsedTree`, `UiNameStr`, `CompKind`.
5. **`text/dispatch.rs`** — `parse_and_insert` closed match (Decision 3); the 10 `parse_<component>` fns; the TYPE-DIRECTED leaf parsers `parse_unit`, `parse_layout_type`, `parse_position_type`, `parse_align_main`, `parse_align_cross`, `parse_f32`, `parse_u32` (Decision 4); the tuple-newtype path for `StackIndex(n)` (Decision 15). Unit-test each leaf + the collision cases: `width: Stretch(1)`→`Unit::Stretch(1.0)`, `cross: Stretch`→`AlignCross::Stretch`, `width: Auto` accepted, `cross: Px(3)`→recoverable error, `cross: Auto`→error.
6. **`text/parser.rs`** — the indent-stack state machine (§A): version-line (mirror `.keys` strip_prefix+`=`), DEDENT + post-DEDENT mismatch check, zero-lookahead classify by `rel ∈ {0, STEP}`, duplicate-name demotion, anonymous-node warning. The biggest piece; test the example corpus + the malformed corpus + the ambiguous-indent cases.
7. **`text/lower.rs`** — `spawn_ui_tree` (§B), the exact runtime mirror of `lower_node`, citing `boyko_macros/src/lib.rs:3875-3971` arm-by-arm in the module doc-comment; first-by-position spawn base; UiName LAST; UiSourceOrder stamp.
8. **`text/serialize.rs`** — `serialize_ui` (§6 / Decision 16): walk a `UiTreeView`, emit `version=1` + canonical indented form; the `.ui`-SPECIFIC float formatter (Decision 16), NOT `write_f32` verbatim.
9. **`reload/tree_view.rs`** — `UiTreeView`: a read facade built from ONE `query_entities` call per reconcile (Decision 7); exposes `has::<C>`/`get::<C>`/`children`/`child_of`/`ui_name`/`source_order` over the live UI components, scoped to the document roots (Decision 10).
10. **`reload/state.rs`** — `UiHotReload` Resource (+ `doc_roots`, `pending`) + ctor/builders.
11. **`reload/reconcile.rs`** — `reconcile_ui` (§C) + `patch_components` (§D): named index (binary search, Decision 9), anon index (by `UiSourceOrder`, Decision 11), scoped domain + duplicate defense (Decision 10), move-vs-despawn `remove_children`-before-`despawn` (Decision 13), set-if-changed read-guarded (Decision 14).
12. **`reload/system.rs`** — `ui_hot_reload_system`: the strict early-return sequence (throttle → metadata → `(mtime,size)` compare → settle → debounce → parse → reconcile), zero-alloc/zero-read on no-change (Decision 7/8).
13. **`plugin.rs`** — `UiPlugin` (mirror `InputPlugin::build`): build-time `read_to_string` + `parse_ui` + `spawn_ui_tree` (capture `doc_roots`); if `with_hot_reload`, `insert_resource(UiHotReload)` + add `ui_hot_reload_system` to `CoreSchedule::Main`. Build-time read uses the graceful-fallback pattern: missing/unreadable file → empty tree, never panic.
14. **`lib.rs`** — declare `text`, `reload`, `plugin`; re-export `UiPlugin`, `parse_ui`, `serialize_ui`, `UI_FORMAT_VERSION` in the prelude.

Dependencies: Step 1 independent. Steps 2–4 independent (parallelizable after 1 for the `UiSourceOrder` reference). Step 5 depends on 2–4. Step 6 depends on 2–4. Step 7 depends on 4–5. Steps 8–9 depend on 4 (+1 for `UiSourceOrder`/`Ord`). Steps 10–11 depend on 5,7,9. Step 12 depends on 11. Step 13 depends on 7,12. Step 14 last.

### Decision 16: Canonical `.ui` float rule — pinned, INVERSE of `write_f32`, proven fixed point (C4 / float-canonicalization fix)

**What**: The `.ui` canonical float rule is stated independently of `write_f32` (verified `writer.rs:214-223`, which ALWAYS appends `.0` — the OPPOSITE of what `.ui` wants):
- **Serialize**: an integral `f32` is emitted with NO decimal point (`320`); a fractional `f32` via Rust's shortest round-trip `{}` (`0.15`). A `.ui`-specific formatter implements this — it reuses the `{}` shortest-round-trip technique from `write_f32` but inverts the integral rule (strips, does not append, the `.0`). It is NOT `write_f32` verbatim; the module doc-comment states the inversion explicitly so no one "fixes" it back.
- **Parse**: `str::parse::<f32>` accepts ALL forms an author/LLM/macro might write — `320`, `320.0`, and inside a unit call `Px(320)`, `Px(320.0)`. For integral values `parse::<f32>("320")` and `parse::<f32>("320.0")` yield bit-identical `f32`s, and the macro literal `Unit::Px(320.0)` produces the same bits — so the value-identity gate holds across spellings. For fractional values, shortest-round-trip `{}` is the unique spelling `parse::<f32>` re-reads to identical bits.
- **Fixed-point proof obligation (test)**: `serialize → parse → serialize` is byte-identical on canonical input, and `parse(authored) Debug == macro(authored) Debug` on the SAME authored values (matching the gate's `format!("{x:?}")` comparison method, verified `ui_macro_equiv.rs:58-74`). Covered values: whole (`320`), fractional (`0.15`), the `f32::MAX` sentinel, `0` (→ `0`, no `.0` in `.ui`).

**Why**: The original §6 specified `.0`-free integers while citing `write_f32`, which does the reverse — a self-contradiction. Because the equivalence gate compares by Debug string and most layout components do not derive `PartialEq` (verified: `ComputedRect`/`ComputedClip` derive `PartialEq`, but the gate compares by `{:?}` uniformly so the float spelling must be pinned regardless), the float canonicalization MUST be exact. Pinning the rule + proving the fixed point closes the round-trip soundness gap.

**Trade-off**: a `.ui`-specific float formatter (~10 lines) instead of reusing `write_f32`. Required by the inverted integral rule; the cost is one small function with its own round-trip test.

## §1 — The `.ui` grammar (canonical) + 1:1 mapping to `ui!`

```
file          := version_line? ( blank | comment | node_block )*
version_line  := "version" ws? "=" ws? INT                 // FIRST significant line (recoverable error if later)
comment       := "//" .*                                    // first UNQUOTED "//"; '#' is the name sigil (Decision 2)
node_block    := node_head ( INDENT attached_or_child+ )?
node_head     := indent ( "#" IDENT )? component?           // named-head (+opt inline comp) OR bare component head
attached_or_child := indent ( component | node_block )      // component at head+STEP attaches; node_block at head+2*STEP is a child
component     := struct_component | tuple_component         // (Decision 15)
struct_component := IDENT "{" field_list "}"                // "UiRoot" (ZST) is the bare IDENT, no braces
tuple_component  := IDENT "(" value ")"                     // StackIndex(10) — the ONLY P3 tuple newtype
field_list    := field ( "," field )* ","?                  // inner split via split_top_level (paren/quote aware, Decision 5)
field         := IDENT ":" value
value         := number | unit_call | ident_enum | INT      // type-directed at the destination field (Decision 4)
unit_call     := ("Px"|"Pct"|"Stretch") "(" number ")" | "Auto"
ident_enum    := ( IDENT "::" )? IDENT                      // Row  OR  LayoutType::Row
number        := f32 in shortest-round-trip or integral form (Decision 16)
```
Canonical form: `version=1` first; spaces-only, 4-space `STEP`; node head at indent D; attached components at D+STEP; child-node heads at D+2·STEP. `#name` unique per document (duplicates demoted to anonymous, Decision 6). Comments and blank lines allowed anywhere, stripped on rewrite. Classification is lookahead-free: `#`-lead ⇒ named head; IDENT-lead at `rel==STEP` ⇒ attached component; IDENT-lead at `rel==0` ⇒ anonymous child head (Decision 1).

### Example 1 — panel + row + buttons
```
version=1
#panel  UiLayout { layout_type: Column, width: Px(320), height: Px(200) }
    UiSpacing { padding_left: Px(12), padding_right: Px(12), padding_top: Px(12), padding_bottom: Px(12), row_gap: Px(8) }
    UiAlign { main: Center, cross: Stretch }
    #toolbar  UiLayout { layout_type: Row, height: Px(28), column_gap: Px(6) }
        #save    UiLayout { width: Px(72), height: Px(24) }
        #load    UiLayout { width: Px(72), height: Px(24) }
        #quit    UiLayout { width: Px(72), height: Px(24) }
```
Equivalent `ui!`:
```rust
ui!(cmds,
  #panel UiLayout { layout_type: LayoutType::Column, width: Unit::Px(320.0), height: Unit::Px(200.0), ..Default::default() },
         UiSpacing { padding_left: Unit::Px(12.0), padding_right: Unit::Px(12.0), padding_top: Unit::Px(12.0), padding_bottom: Unit::Px(12.0), row_gap: Unit::Px(8.0), ..Default::default() },
         UiAlign { main: AlignMain::Center, cross: AlignCross::Stretch },
    children: [
      #toolbar UiLayout { layout_type: LayoutType::Row, height: Unit::Px(28.0), column_gap: Unit::Px(6.0), ..Default::default() },
        children: [
          #save UiLayout { width: Unit::Px(72.0), height: Unit::Px(24.0), ..Default::default() },
          #load UiLayout { width: Unit::Px(72.0), height: Unit::Px(24.0), ..Default::default() },
          #quit UiLayout { width: Unit::Px(72.0), height: Unit::Px(24.0), ..Default::default() },
        ]
    ]
);
```
Mapping: `#name`↔`#name`; `Component { f: v }`↔component literal (text `Px(320)`↔`Unit::Px(320.0)` — both bit-identical, Decision 16; `Column`↔`LayoutType::Column`; `cross: Stretch`↔`AlignCross::Stretch` resolved by the AlignCross field arm, NOT confused with `Unit::Stretch`, Decision 4); attached lines at D+STEP↔extra component literals; child heads at D+2·STEP↔`children: [ … ]`; omitted fields↔`..Default::default()`.

### Example 2 — every node named (LLM-canonical), tuple newtype
```
version=1
#hud  UiLayout { layout_type: Overlay, width: Stretch(1), height: Stretch(1) }
    #healthbar  UiLayout { position_type: Absolute, width: Px(200), height: Px(16) }
        UiAbsolute { left: Px(16), top: Px(16) }
    #minimap  UiLayout { position_type: Absolute, width: Px(128), height: Px(128) }
        UiAbsolute { right: Px(16), top: Px(16) }
        StackIndex(10)
```
`StackIndex(10)`↔macro `StackIndex(10)` (Decision 15 — the tuple-literal form, NOT `{ 0: 10 }`). `Stretch(1)`↔`Unit::Stretch(1.0)` (Unit field arm); `Overlay`↔`LayoutType::Overlay`.

### Example 3 — full field coverage
```
version=1
#card  UiLayout { layout_type: Column, position_type: Relative, width: Px(280), height: Auto, min_width: Px(120), min_height: Px(40), max_width: Px(400), max_height: Pct(80) }
    UiSpacing { padding_left: Px(10), padding_right: Px(10), padding_top: Px(8), padding_bottom: Px(8), border_left: Px(1), border_right: Px(1), border_top: Px(1), border_bottom: Px(1), row_gap: Px(6), column_gap: Px(0) }
    ContentSize { width: 0, height: 0 }
```
`height: Auto`→`Unit::Auto` (Unit field arm accepts bare `Auto`); `Pct(80)`→`Unit::Pct(80.0)`; `ContentSize.width: 0` is an `f32` field (`0`→`0.0` bits). `ComputedRect` is NOT authored (layout owns it) → the loader injects `ComputedRect::default()` via the non-bundle spawn path (Decision 5 / §B).

### Example 4 — nested, mixed named/anonymous, root marker
```
version=1
#screen  UiLayout { layout_type: Column, width: Stretch(1), height: Stretch(1) }
    UiRoot
    #list  UiLayout { layout_type: Column, row_gap: Px(2) }
        UiLayout { layout_type: Row, height: Px(20) }          // anonymous row 0 (no #name) → UiSourceOrder(0)
        UiLayout { layout_type: Row, height: Px(20) }          // anonymous row 1 (no #name) → UiSourceOrder(1)
        #footer  UiLayout { layout_type: Row, height: Px(24) }
```
`UiRoot` is a ZST attached component of `#screen` (a bare IDENT at `rel==STEP`, no braces). The two anonymous `UiLayout {…}` lines sit at `#list`'s child indent (`rel==0` relative to the `#footer` sibling level, i.e. a child-node head indent) → they open anonymous child nodes (Decision 1 classify), each stamped with `UiSourceOrder` (0, 1) so reload reconciles them by stored ordinal, NOT by `Children` slice order (Decision 11). The parser WARNS that the anonymous rows are state-fragile if they later gain children/non-`UiLayout` components.

## §6 — Round-trip serializer

`serialize_ui(view, out)`: clear `out`; emit `// boyko-engine .ui — generated; edits below the version line are canonicalized on rewrite`; emit `version=1`; walk the document's scoped roots in entity-creation order; per node emit the head line `<indent>#<name>  UiLayout { <canonical_fields> }` (UiLayout is always present — the macro requires it), then one attached line per remaining component in a FIXED canonical order (`UiSpacing`, `UiAlign`, `UiAbsolute`, `ContentSize`, `StackIndex`, `ComputedClip`, `UiRoot`; `ComputedRect` OMITTED when it equals `default()` — layout output, not authored; `UiSourceOrder` NEVER serialized — private), then recurse children at +STEP (head) / +2·STEP (their attached). `StackIndex` serializes as the tuple form `StackIndex(n)` (Decision 15). Floats via the `.ui`-specific formatter (Decision 16): integral → no decimal (`Px(320)`), fractional → shortest `{}` — chosen so `parse_ui` re-reads identical bits.

Round-trip guarantee: `parse_ui(serialize_ui(view))` is value+topology-equal to the source `ParsedTree` on canonical input (same nodes, names, component sets, field values; comments + non-canonical whitespace dropped). The strong form — `serialize → parse → serialize` byte-identity on canonical input — is the gate (mirrors `.keys` `canonical_round_trip_is_byte_identical`, `i5_persist.rs:42`). The canonical field order + canonical float formatting (Decision 16) + canonical indent make the output a normal form.

## §8 — Out of scope + seams

- **Widget vocabulary / interaction / bindings → P4.** The closed `match` (Decision 3) is the seam: P4 adds widget-component arms (or promotes to Pattern B's derive table for open vocabulary). Actions/bindings stay in `.keys`; `.ui` carries no behavior.
- **Rendering → P5.** `ComputedRect`/`ComputedClip`/`StackIndex` are author-or-layout-owned data; the renderer reads them. `.ui` authors layout inputs only.
- **Text/SDF content → P5b.** `ContentSize` is the seam; `.ui` authors a fixed `ContentSize`; P5b replaces its source. P5b also re-enables `strip_comment`/`split` quote-awareness for quoted string values (the machinery is retained now, Decision 2/5).
- **Open-vocabulary `.ui` → deferred.** Seam: Pattern B (the derive-emitted `TextParseFn` table keyed by `stable_name`) + a name→`ComponentId` resolver. The `parse_<component>` leaves (Decision 4) are reused as the table entries.
- **OS-native file watcher → deferred, feature-gated.** Seam: `UiHotReload` encapsulates the change-detection; a native backend swaps the poll for an event source behind a cargo feature without touching the reconciler.
- **`#name`-RENAME preservation → out of scope (Decision/minor rename-fix).** A `#name` change is treated as remove+add (new identity): the old name is unmatched (despawned via Decision 13's safe path), the new name has no live match (spawned). Transient state is intentionally NOT carried across a rename (a rename is a new identity). Pinned + tested so the behavior is deliberate, not emergent.
- **`ChildOf`-cycle / sibling-order determinism.** `Children` order is unspecified under removal (`hierarchy/mod.rs:94-99`); P3 relies on FIFO link order for the INITIAL-load gate (both macro and loader produce the same insertion order) and on STORED `UiSourceOrder`/`UiName` keys (NOT `Children` slice order) for reconcile — so it does NOT depend on order stability across removals. P5a sorts by `StackIndex` for deterministic draw order (the documented consumer-sort seam).

## Metrics and validation

Benchmarks (`cargo bench`, cold-path — informational, not gated on ns):
- `bench_parse_ui` — parse a 200-node `.ui` (allocations + time, for regression tracking).
- `bench_reconcile_nochange` — reconcile an identical tree: assert ZERO spawns/despawns/inserts emitted (the spurious-tick guard).
- `bench_reconcile_one_field` — change one field on one node: assert exactly one `insert` emitted, zero on every other node.
- `bench_watch_nochange` (zero-alloc, mirrors `crates/boyko_ui/tests/zero_alloc.rs`) — the watch system on a no-change poll: assert ZERO allocations AND zero `query_entities` calls (Decision 7).

Mandatory unit tests:
- `split_top_level`/`strip_comment_slashslash`/`leading_ws_width`/brace-span-extractor — port the `.keys` tests + `//` cases (`#a // c`, `a/b`, `//`-only, `"a//b"` quoted) + the value-with-braces rejection (Decision 5) + tab/space + off-STEP.
- `UiName` `Ord`: `"a" < "ab" < "b"`, `Ord`-consistent-with-`Eq` (Decision 9).
- Each `parse_<component>` + each leaf (`parse_unit` all 4 incl. `Auto`; every enum variant bare + qualified; `f32`/`u32`); the collision cases (Decision 4): `width: Stretch(1)`, `cross: Stretch`, `width: Auto`, `cross: Px(3)`→err, `cross: Auto`→err.
- `StackIndex(10)` parse → `StackIndex(10)` (Decision 15).
- Parser: INDENT/DEDENT/EQUAL transitions, blank-line tolerance, `version=N` first-line (and `versionX` fall-through, malformed `version=abc` recovery), dedent-to-never-opened-column (record + skip without stack mutation), zero-lookahead classify (attached vs anonymous-child at `rel==STEP` vs `rel==0`).
- Float canonicalization fixed point (Decision 16): `serialize→parse→serialize` byte-identical for whole/fractional/`f32::MAX`/`0`.

THE GATE — INITIAL-LOAD `.ui`-tree ≡ `ui!`-tree (Decision 12):
- For each of the 4+ example files, build the tree TWO ways: (a) the equivalent `ui!` invocation, (b) `parse_ui` + `spawn_ui_tree`. After the apply drain, assert IDENTICAL: same entity count; per-entity same component-id SET **excluding `UiSourceOrder`** (Decision 12); same component byte-values compared by `format!("{x:?}")` (matching `ui_macro_equiv.rs:58-74`); same `UiName` (memcmp); same `ChildOf`/`Children` topology AND same INITIAL `Children` order (FIFO link order identical); **same archetype id per matched node** (set-based, follows from same spawn base + same set, Decision 12). Include a `StackIndex` node (Decision 15) and a duplicate-`UiLayout` node (Decision 12: first-by-position) in the corpus.

POST-RELOAD equivalence (Decision 12 — value+topology, NOT archetype-id):
- Reload an identical file → ZERO inserts/spawns/despawns, ZERO changed ticks (the Principle-10 gate). Assert value + topology equal to the pre-reload tree; do NOT assert archetype-id identity (incremental-migration legitimate divergence).

Malformed-line recovery corpus (with `(line, col, reason)` assertions):
- Unknown component, unknown enum variant, bad float (assert the field col), over-CAP `#name` (demoted), duplicate `#name` (demoted, error at 2nd occurrence), mixed tab/space, off-STEP indent, dedent-to-never-opened-column, a unit literal in an enum field (`cross: Px(3)`). Assert every well-formed line still produced its node/component and the file-level parse succeeded.

LLM-authorability corpus:
- "What an LLM would plausibly emit" files (varied whitespace within spaces-only, comments, blank lines, inline vs attached components). Assert all parse clean (zero errors).

Hot-reload transient-state preservation (Decision 14):
- Spawn a tree; attach a transient marker (`UiFocus`/`UiScroll` test marker the text never mentions) to a `#named` node; reload a version that changes a sibling AND the focused node's `UiLayout`. Assert: focused node's transient markers still present, unchanged tick (never written); its `UiLayout` patched (changed value, bumped tick); sibling change applied; SAME entity id.
- **Migration-survival**: focus a node, reload a file that ADDS a component to that node (forces archetype migration). Assert the transient marker survives the migration with unchanged tick (Decision 14 / byte-copy migrate).

Add/remove/relink/move-vs-despawn correctness (Decision 13):
- Add: reload adds a `#named` child → new entity spawned + linked, others same ids.
- Remove: reload deletes a `#named` child → that entity + subtree despawned, others same ids.
- Relink: reload moves a `#named` node to a different parent → SAME entity id, `ChildOf` now the new parent, old parent's `Children` no longer contains it, transient state survived.
- **MOVE-VS-DESPAWN (Decision 13 critical test)**: reload that moves `#x` to parent `B` AND deletes `#x`'s old parent `A` in the same reload → `#x` survives under `B` with transient state intact; `A` and its genuinely-vanished children are gone.
- **Live-duplicate defense (Decision 10)**: construct a scoped live tree with two same-`UiName` children under one parent (simulating corruption); reload → the error is reported, the lowest-id wins the key, the OTHER is NOT despawned.
- **Anonymous reload (Decision 11)**: spawn anonymous siblings, attach a transient marker to anon ordinal 1, delete anon ordinal 0 via a sibling edit + reload → ordinal-1's transient state survives (matched by STORED `UiSourceOrder`, not slice order). A second reload after the deletion still matches deterministically.
- **Rename (rename-fix)**: reload that renames `#old`→`#new` → new entity id, old despawned (deliberate remove+add).
- Set-if-changed: reload identical file → ZERO inserts/spawns/despawns, ZERO changed ticks.

Round-trip: as §6 — value/topology equality after `parse_ui(serialize_ui(view))`; byte-identity of `serialize→parse→serialize` on canonical input.

`debug_assert!` invariants:
- Parser: indent stack never underflows below the root sentinel; a `UiNameStr` never exceeds `UiName::CAP`.
- Reconcile: the per-parent named index is sorted before binary search; the binary-search hit equals a linear-scan hit (Decision 9 cross-check); the `matched` bitset covers exactly the scoped children of the parent; `UiSourceOrder` keys are unique per parent (Decision 11).
- Lower: a node reaching the bundle fast path has both `UiLayout` and `ComputedRect` parsed (mirror the macro's `(None,_)` unreachable arm, `boyko_macros/src/lib.rs:3935`).
- Reconcile despawn phase (Decision 13): before despawning a doomed parent, every matched (survivor) descendant has had `remove_children` enqueued ahead of the `despawn` on the same queue.

## Open questions for the critic / user

1. **`UiSourceOrder` placement**: I put the private positional key in `boyko_ui::components` (a `pub(crate)` component) so it rides the standard component machinery. Acceptable, or prefer it in `boyko_ui::reload` (still a `Component`, just module-local)? Functionally identical; this is a code-organization call.
2. **Anonymous-node policy strength**: Decision 11 reconciles anonymous nodes by stored `UiSourceOrder` (correct + state-preserving while declaration order is stable) and WARNS on stateful anonymous nodes. The stricter alternative (forbid anonymous reconcile entirely → always despawn+respawn anonymous runs) is simpler but loses anonymous transient state every reload. I chose the preserving variant; confirm that is the desired trade (more code, better state preservation) vs the strict variant (less code, anonymous state always reset).
3. **`UiSourceOrder` in the macro path**: the gate EXCLUDES `UiSourceOrder` from the compared set because the macro does not stamp it (Decision 12). Alternatively the `ui!` macro could ALSO stamp `UiSourceOrder` so both paths are byte-identical including it (no gate exclusion). That is a `boyko_macros` change (out of P3's "no macro changes" scope). I chose the gate-exclusion to keep P3 macro-free; confirm, or authorize the small macro change for stricter equivalence.

## Changes from review

Resolved every critical and major; the three minors are folded into decisions or out-of-scope seams. All source claims re-verified against the actual files (line citations corrected throughout).

**Critical fixes**
- **C1 (split_top_level not brace-aware)** → Decision 5: field extraction is now a TWO-STAGE scan — a NEW brace/paren-matching component-span extractor isolates the component body, then `split_top_level` is reused VERBATIM on the INNER list ONLY (proven free of `{`/`[`/quoted-comma in the P3 value set, locked by a value-with-braces rejection test). Dropped the "verbatim reuse" claim for the whole line.
- **C2 (Stretch/Auto token-shape collision)** → Decision 4: value parsing is now explicitly TYPE-DIRECTED by the destination field's `match key` arm; there is NO standalone value parser. `Stretch` resolves to `Unit::Stretch` only in a Unit field, `AlignCross::Stretch` only in an AlignCross field; cross-type literals are recoverable per-field errors. Verified the collision against `units.rs:23/25/101`. Added the collision test matrix.
- **C3 (head vs attached ambiguity needing lookahead)** → Decision 1 + §A: classification is now ZERO-lookahead via each open node's recorded `head_indent` and a pure `rel ∈ {0, STEP}` compare (`#`-lead ⇒ named head; IDENT-lead at `rel==STEP` ⇒ attached; IDENT-lead at `rel==0` ⇒ anonymous child head). Added ambiguous-indent cases to the malformed corpus.
- **C4 (float round-trip self-contradiction)** → Decision 16: pinned a `.ui`-specific float rule (integral → no decimal, fractional → shortest `{}`), stated it is the INVERSE of `write_f32` (which always appends `.0`), and added a fixed-point proof obligation (serialize→parse→serialize byte-identity + parse-Debug == macro-Debug on the same values). Stopped citing `writer.rs:214` as the `.0` precedent.
- **C1-bis (UiName lacks Ord)** → Decision 9: added a documented `Ord`/`PartialOrd` to `UiName` (prefix-then-len memcmp, consistent with `Eq`), making the sorted-Vec+binary-search index compile. This is the one additive change to an existing `boyko_ui` file; a linear-scan debug cross-check is added.
- **Anon-soundness (Children order non-contract)** → Decision 11: anonymous reconcile is now by a STABLE stored `UiSourceOrder(u32)` (declaration ordinal per sibling), NEVER by the `swap_remove`-perturbed `Children` slice order. Verified the non-contract at `hierarchy/mod.rs:94-99`.
- **Move-vs-despawn (cascade reads live Children)** → Decision 13: the relocate-before-delete guarantee is now an APPLY-TIME consequence — before despawning a doomed parent, `remove_children(survivors)` is enqueued ahead of the `despawn` so the cascade (verified `hierarchy/commands.rs:298-367`) cannot reach a moved survivor. Added the dedicated move-vs-despawn test.

**Major fixes**
- **Citations (lib.rs:38xx wrong crate)** → all macro-lowering citations corrected to `boyko_macros/src/lib.rs` (lower_node 3875-3971, bundle arms 3900-3938, UiName-LAST 3940-3945, child link 3959-3961, validate/dup_name 3696-3771). §B doc-comment cites the macro arm-by-arm.
- **Version-line handling** → §A mirrors `.keys` exactly: `strip_prefix("version")` + require `=`, fall through on `versionX`, recover malformed `version=abc` (keep default).
- **Comment `//` edge cases** → Decision 2 pins the two-byte quote-aware rule; `strip_comment_slashslash` returns the PRE-TRIM slice (the `.keys` strip-then-trim flow is NOT reused — strip-comment-flow fix); added `//`/`a/b`/`"a//b"` tests.
- **Dedent-to-mismatch + stack underflow** → §A adds the explicit post-DEDENT `rel ∈ {0, STEP}` check; a dedent to a never-opened column records + skips WITHOUT mutating the stack.
- **Error line:col** → Decision 6: `UiParseReport` errors gain a `u16` col; leaf parsers thread the field offset.
- **StackIndex syntax** → Decision 15: the tuple-literal form `StackIndex(10)` (mirrors the macro), replacing the proposed `{ 0: 10 }`/`{ 10 }`; added the tuple-component grammar production + a gate case.
- **Archetype-vs-order rationale** → Decision 12: corrected — archetype identity is SET-based (`archetype.rs`), so it follows from spawn base + component set, NOT insert order; insert order drives only the INITIAL `Children` order. Duplicate `UiLayout`/`ComputedRect` mirrors the macro's first-by-position.
- **Per-poll allocation / no-change path** → Decision 7: pinned the strict early-return sequence (throttle → metadata → `(mtime,size)` compare → settle → reconcile); the no-change path does ZERO allocation and ZERO tree read (`query_entities` allocates, verified `ecs_master.rs:2096`); added a `zero_alloc`-style watch test.
- **Torn-read race** → Decision 8: added a deterministic two-poll SETTLE (`pending` buffer) on top of the validity net, so a partial write at coincidentally-final size is not reconciled.
- **Live-side duplicate UiName** → Decision 10: the reconcile is SCOPED to the document's own roots (tracked in `UiHotReload`); a scoped duplicate records an error, lowest-id wins, the loser is preserved (never silently despawned).
- **Transient survival across migration** → Decision 14: stated and tested that transient components ride the byte-copy archetype migration; the §D pseudocode is read-guarded (never read an absent column).

**Minor fixes**
- **Duplicate-#name recovery policy** → Decision 6: pinned — keep first, demote the duplicate to anonymous (positional reconcile via Decision 11), error at the second occurrence.
- **Anonymous-node fragility** → Decision 11: upgraded from "document it" to a correct stored-key reconcile + a parser WARNING on stateful anonymous nodes.
- **#name-rename semantics** → §8 + a test: pinned as remove+add (new identity, transient state not carried) — deliberate, not emergent.
- **Post-reload equivalence contract** → Decision 12: pinned as value+topology (NOT archetype-id identity) to avoid flaking on incremental-migration / retained-empty-`Children` (verified `hierarchy/mod.rs:101-109`).
- **Dispatch string-key/type lockstep** → Decision 3: added the exhaustiveness test over the 10-component vocabulary and the "text name MUST equal the type name" invariant.

Relevant files: the plan targets `D:\claude\BoykoEngine\crates\boyko_ui\src\` (new `text/`, `reload/`, `plugin.rs`; additive `components.rs` change). Source of truth for the gate: `D:\claude\BoykoEngine\crates\boyko_macros\src\lib.rs:3875-3971`. Templates: `D:\claude\BoykoEngine\crates\boyko_input\src\persist\grammar.rs` (`split_top_level:471`, `strip_comment:453`, version parse `:106`) and `writer.rs:208-223` (`write_f32`, shortest-`{}` technique only).