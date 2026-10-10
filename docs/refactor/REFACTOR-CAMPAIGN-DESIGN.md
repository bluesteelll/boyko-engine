# Refactoring campaign - split convention, tooling and verification protocol (design v6)

> **Status (2026-09-17): parked by the owner's ruling (unified plan Q-4): the campaign runs after everything else.**
> Six architecture-critique passes; closed after pass 6 by orchestrator ruling (only Important remarks remained;
> the rulings are appended at the end). Census: [REFACTOR-CENSUS.md](REFACTOR-CENSUS.md), taken at `d552be05`.
> Partial tooling (`tools/refactor/`) sits uncommitted in the worktree `D:/wt/refactor`
> (branch `refactor/split-oversized-files`); it was stopped mid-write and must be re-checked before use.
> Both documents describe the tree at `d552be05`; every line number must be re-derived when the campaign resumes.

# Architecture: the oversized-file split campaign, Rev 6 — split convention, tooling, verification protocol, and the pilot (`crates/aether_lang/src/expand.rs`)

**This message is the whole of Rev 6.** Nothing comes before or after it, so the workflow's `design` value is this message. It follows Rev 5's structure and length budget.
- **Kept:** every rule from Rev 5, except the ones changed under "Changelog Rev 5 → Rev 6" below.
- **Removed as history:** Rev 5's "Status of the pass-4 remarks" table and its "Changelog, Rev 4 → Rev 5" (CH54–CH59). Their rules are now in the body. The single wording error in them is fixed by CH65.
- **Where Rev 5 is:** the workflow's `DESIGN v5` value.

**How this revision was produced:** read-only, with no build, test, timing or file write.
- **HEAD:** `d552be05be4b4f063b6cb39ddfd63eb4688f83fd`, read from the worktree ref files. `git status` was not run because this role has no shell. `tools/refactor/` does not exist yet.
- **Re-read for this revision:** the tree-sitter-rust v0.24.0 `grammar.js` rules; `tests/internal_docs_anchors.rs:406-494` and `:676-1060`; the matchers at `ffi.rs:266-267` and `:299-300`, and at `filter.rs:1547-2612`; `cull_diagnostic.rs:3308` and `:3316`.

## Status of the pass-5 remarks

| Remark | Resolution |
|---|---|
| W1 (item-position invocation shapes) | CH60: every node shape is defined, the `;` is inside the unit's extent, every item-level node kind is classified, a byte-coverage check is added, plus shape fixtures N19a–h, mutations M40/M41, and a restated Q10. CH61 extends D4(a) to the real matchers: `ffi.rs`'s is not flat, and `filter.rs`'s define nothing. |
| W2 (FX-SEED kind; `sticky-parent` never reached) | CH62: N18 is corrected to `base-parent` depth 1. Route and depth are now defined from `resolve_fragment` itself. New fixtures FX-STICKY, FX-DEAD and FX-PREC. A table maps every trace kind to the fixture that reaches it. |
| Critic Q1 (checking the port's binding of each anchor against the gate's) | CH63: a gate oracle run on the real tree (B16, S10) and in the fixtures. It uses sentinel line numbers and file lengths that are unique per file, so the gate itself reports each anchor's binding. M44 shows it catches a port bug that count parity misses. |
| Critic Q2 (should a pin also stop its use site moving?) | CH64: yes. A pin is a co-location constraint. The item containing the opaque token tree is pinned too. |
| O1 (CH59 wording) | CH65. |
| O2 ("lengthen"; fenced `(N)`) | CH66. |
| O3 (parity of over-waived lines) | CH67. |
| O4 (mentions on fence lines) | CH68. |

**How R1 is applied (restated on the corrected definition).**
- A name is declared only by a real item as tree-sitter parses it. Tokens inside any macro token tree — `quote!`/`quote_spanned!` templates, `macro_rules!` bodies, or any other invocation — declare nothing where they are written, and never make `split.py` refuse a file.
- **An item-position invocation is itself a real item.** It is `MacroInvocationSemi` in the Reference's `Item` grammar. Tree-sitter parses it in one of three shapes (F20):
  - (A) an `expression_statement` whose only named child is a `macro_invocation`: `a!(…);`, `b![…];`, and at file scope `c!{…};`;
  - (B) a bare `macro_invocation`: `c!{…}`, and `a!(…)` inside a `declaration_list`;
  - (C) shape B followed by a sibling `empty_statement` holding its `;`.

  In every shape the `;` is inside the unit's extent. The invocation remains a partition unit.
- The names an invocation defines come from the macro's **definition** (D4 (a)/(b)), never from its tokens. An invocation that neither rule covers is **pinned** in its module. A pin constrains placement only; it never refuses a file.
- **For the orchestrator (Q10):** this keeps check 8's view of the 104 `embed_spirv!` statics (`compute.rs`), the 23 handle types (`ffi.rs:319-418`) and the criterion roots. If R1 was also meant to remove D4 (a)/(b), that is a different ruling and it is not made here.

---

## Changelog, Rev 5 → Rev 6

**CH60 — W1 (the shape of an item-position invocation).**
- **Removed from "How R1 is applied":** "- An **item-position** invocation such as `embed_spirv! { … }` is itself a real item: `MacroInvocationSemi` in the Reference's `Item` grammar, and a `macro_invocation` node directly under `source_file`/`declaration_list` in tree-sitter. It stays a partition unit."
- **Removed from D4:** "  - item-position macro invocations (`macro_invocation` directly under `source_file`/`declaration_list`)."
- **Removed from §7.1:** "2. Parse. Units are named items, impls, satellites, twins and item-position invocations (D4). The header stays."
- **Removed from §16:** "| **Q10** | orchestrator | R1 as applied here (top section, R21): confirm that item-position invocations stay units whose names come from the macro definition (D4 (a)/(b)). This is not a request to re-open R1, only to confirm its scope. |"
- **Replaced by:**
  - the item-level node table in D4, with the shapes A/B/C;
  - check 1b (byte coverage);
  - `rsitems.py shapes`, run in B10;
  - facts F20 and F21;
  - fixtures N19a–h, mutations M40 and M41;
  - the restated Q10.
- **Sections to re-read:** D4, D16, §7.1, §7.2 checks 1 and 5.

**CH61 — D4(a) against the real matchers.**
- **Removed from D4:** "  - (a) A workspace `macro_rules!` with a flat matcher: arguments bind by position, and names are read from items in the transcriber (e.g. `embed_spirv!` at `compute.rs:84-88` → `static $name`)."
- **Why:** the handle macros' matcher is `($(#[$meta:meta])* $name:ident)` (`ffi.rs:267`, `:300`), which contains a repetition, so it is not flat. `filter.rs`'s six matchers are all repetitions (`:1548`, `:1823`, `:2134`, `:2212`, `:2514`, `:2612`). Under Rev 5, all 95 invocations would have been pinned.
- **Replaced by:** (a1) the flat rule; (a2) the trailing-name rule; (a3) the rule that a transcriber holding only `impl`/`const _` defines nothing. Transcriber repetitions are re-parsed once. The check-8 rule for transcriber names is made explicit. Fixtures N20 and N21, mutation M42.

**CH62 — W2 (fence routes and fixtures).**
- **Removed from §7.3:** "| `A-fence-frag/<how>` | `how` ∈ {`sticky`, `base-dir`, `base-parent`, `sticky-parent`}, plus the ancestor depth |"
- **Removed from §7.3:** "  - `A-fence-frag/x` → `A-fence-frag/y` (fragment lengthened)."
- **Removed from the fixture table:** "| **FX-SEED** (N18) | the FX-LABEL run followed by a fence holding a fragment-less anchor bound to `r.rs` | `F-open` seed after = `r/c.rs` ≠ T′.path → fragment `r.rs` inserted → `A-fence-frag/sticky-parent` with T′ |"
- **Removed from the FX-BASE row:** "before: `A-fence-frag/base-parent` → `src/b.rs`; after the relink, a naive result is `src/a/b.rs`, and the seeding rule lengthens the fragment to `src/b.rs`, giving T_after == T′; Δmentions = 0 (relink); `M-label` is printed"
- **Removed from §14:** "anchor row kind in the §7.3 trace table, each reached in a two-repo fixture;"
- **Replaced by:**
  - route and depth defined from `resolve_fragment` (`:701-729`);
  - `sticky-self` and `sticky-dir` added as route names;
  - corrected expected kinds and depths for FX-H3, FX-BASE and FX-SEED;
  - new fixtures FX-STICKY (N22), FX-DEAD (N23), FX-PREC (N24);
  - the kind-coverage table.

**CH63 — critic Q1 (the gate oracle).**
- **Removed from §7.3 step 4:** "- **(e) Gate twin.**" was the only gate-side evidence. Its text is kept, and a new post-condition (f), the gate oracle, is added.
- **Removed from §10, the "Stays GREEN" cells:** M36 "**gate twin green** (waived → bounds only)", M37 "gate twin green", M39 "gate twin green (both `b.rs` files are definition-shaped)".
- **Replaced by:** those rows now name the oracle as red and the plain twin as green. New: B16 oracle row, S10 oracle clause, M44.

**CH64 — critic Q2 (a pin co-locates).**
- **Removed from D16:** "- A pinned name's definition keeps the module that the use site resolves it from, so no generated line ever serves the name."
- **Removed from D16 (ii):** "**If the fragment does not parse, the names used inside it are pinned.**"
- **Removed from D16 (iii):** "**every name used inside it is pinned.**"
- **Replaced by:** the co-location pin, which covers both the enclosing item and the family definitions. M43.

**CH65 — critic O1.** Rev 5's CH59 said: "A new `anchors.py` invariant: it never edits a line whose bindings all point outside the plan's family."
- That sentence was wrong, because FX-BASE edits a fence line bound to `src/b.rs`, which is outside the family.
- The only operative statement is §7.3 step 3's scope invariant, which permits a fence repair.

**CH66 — critic O2.**
- **Removed from §7.3 step 3:**
  - "  - if it is fragment-less → insert the **shortest path suffix** of T′.path that `resolve_fragment(·, current_at_fence, base_at_line)` resolves exactly to T′.path, immediately left of the `:`;"
  - "  - if it already has a fragment → lengthen that fragment to the shortest suffix that resolves exactly;"
- **Replaced by:** the fragment is *replaced* by the shortest exact suffix, and the fenced `(N)` form gets an insertion rule of its own.

**CH67 — critic O3.**
- **Removed from §7.3 step 0:** "- the over-waived lines."
- **Removed from the B5 cell:** "over-waived lines".
- **Replaced by:** the over-waived *count*. The set of over-waived lines is compared port to port, in (d).

**CH68 — critic O4, plus the fixture that prints a dead mention.**
- **Removed from post-condition (b):**
  - "  - mentions_after = mentions_base + Δ, where Δ = Σ over edited lines of `len(scan_line(new).mentions) − len(scan_line(old).mentions)`;"
  - "  - dead = 0;"
- **Replaced by:** Δ is summed over edited **prose** lines only (`:935` discards fence mentions), and dead_after = dead_base (0 on the real tree).

---

## 0. Facts the design rests on

| # | Fact (file:line) | Used by |
|---|---|---|
| F1 | `expand.rs` has 0 gated anchors, and 4 ungated line anchors (`docs/aether-v2/DECISIONS.md:90`, `docs/gaia/DECISIONS.md:100`, `docs/OPEN-QUESTIONS.md:620`, `docs/ru/OPEN-QUESTIONS.md:479`), plus symbol and path citations (§9) | `anchors.py drift` |
| F2 | 3 of the 4 gated docs are owner-dirty | R5, ordering |
| F3 | The owner's branch has `runtime-data-ledger.tsv` (57 moving rows) and `macros-aether.md` (264 citations) | merge-time drift, Q2 |
| F4 | `crates/aether_tests/tests/ui/` holds 38 trybuild `.rs`/`.stderr` pairs | S8 |
| F5 | `query/data.rs` uses glob re-exports and `//! Split from` lines | copy its layout only |
| F6 | Release builds use fat LTO; the bench profile uses `codegen-units = 1` (`Cargo.toml:111-127`) | R7 |
| F7 | 6 tests read one census file by path; the repo-root walkers scan every `.rs` | D17 |
| F8 | `store.rs`: no hot-path registry rows; its only atom is `profiling-analysis`; `default = []` (`boyko_ecs/Cargo.toml:54`); cfg'd uses sit inside items that carry no cfg (L275-292, 475-538, 594-694, 1318-1395, 1462); `const _` satellites at L69, 151-152, 224-225, 320, 386, 625-651; `impl Profiler` spans L670-1398 | D16, D2, §5 |
| F9 | CI runs its tests with `debug_assertions` off (`ci.yml:109,216`) | release leg |
| F10 | `BOYKO_PROFILE` changes values only; no `build.rs` emits `rustc-cfg` (the only hit is doc text at `boyko_diag/build.rs:39`) | not a leg; re-checked in B10 |
| F11 | `Component::stable_name` defaults to `type_name` and is the on-disk key (`component.rs:183-197`; `serialize.rs:211,238,390,410-423`; `boyko_serialize/src/load.rs:396`). The derive overrides it only when `stable_name` is given (`boyko_macros/src/component.rs:1641-1647`). `LAYOUT_FINGERPRINT` contains no name (`component.rs:1523-1530`) | D15 |
| F12 | Pilot: its only atom is `test` (L1395). Captures at L516/522/533/575/856/1017/1080/1389. `quote_spanned!` at L96/102/111/188/206/219/250/696. No item-position macro, including inside `mod tests`. Intra-doc links at L40/493/680/778/1094/1096/1098/1102/1365/2499 | check 8 |
| F13 | Every CI cargo job runs on linux (`ci.yml:79,98,180`). Miri uses `-D warnings` without v3 (`:229-238`). loom uses `--cfg loom` (`:314`). `--cfg force_alloc_panic` runs in release (`:211,216`). `boyko_ecs` gates `libc` on `cfg(unix)` (`Cargo.toml:43-44`) and `loom` on `cfg(loom)` (`:50-51`) | D16, D19 |
| F14 | Among the 29 free census files, host atoms appear only at `ffi.rs:36` (`#[cfg(windows)] pub mod os`) and `component_registry/mod.rs:132` (`target_pointer_width = "64"`) | D19 |
| F15 | Cargo discovers `benches/*.rs` and `benches/<dir>/main.rs`, even alongside explicit `[[bench]]` entries. `#[path]` precedents: `boyko_render/tests/light_generation.rs:13`, `boyko_log/benches/code_idx_cost.rs:33-34` | D18 |
| F16 | The hot-path script excludes `/tests/ /benches/ /examples/ /target/` but not `/src/bin/` (`check_hotpath_exceptions.py:49`). The walker marks all four (`walker/mod.rs:335`) | check 13 |
| F17 | Template token trees that contain declaration-shaped tokens: `expand.rs:188-195`, `expand.rs:206-210`, `boyko_macros/src/component.rs:369` | R1, N13 |
| F18 | Gate mechanics, `tests/internal_docs_anchors.rs` (listed below) | §7.3 |
| F19 | The gate is std-only (`:224-225`). Its roots are `CARGO_MANIFEST_DIR` and `…/docs` (`:256-262`) | twin and oracle |
| **F20** | tree-sitter-rust v0.24.0 `grammar.js`: `source_file = optional(shebang), repeat(_statement)`; `_statement = expression_statement \| _declaration_statement`; `declaration_list = '{' repeat(_declaration_statement) '}'`. `_declaration_statement` = `const_item, macro_invocation, macro_definition, empty_statement, attribute_item, inner_attribute_item, mod_item, foreign_mod_item, struct_item, union_item, enum_item, type_item, function_item, function_signature_item, impl_item, trait_item, associated_type, let_declaration, use_declaration, extern_crate_declaration, static_item`. `expression_statement = seq(_expression, ';') \| _expression_ending_with_block`, where the second set contains no `macro_invocation`. `macro_definition` includes its own `;`. Corpus `macros.txt` (v0.24.0): `a!(); b![]; c!{};` at file scope parse as `expression_statement(macro_invocation)`. Inside a `declaration_list`, the `;` is an `empty_statement` sibling | CH60 |
| **F21** | `dispatchable_handle`/`non_dispatchable_handle` matcher `($(#[$meta:meta])* $name:ident)` (`ffi.rs:267,300`); 23 paren-form invocations at L319-418. `filter.rs`: 6 repetition matchers (e.g. `( $( ($F:ident, $s:ident, $f:ident) ),* )`, `:1548`) with SAFETY comments and `impl` blocks in the transcribers; 72 file-scope `impl_*!(…);` invocations. `cull_diagnostic.rs:3308` `criterion_group!(`, `:3316` `criterion_main!(cull_diagnostic);` | CH61 |

F18 in detail (`tests/internal_docs_anchors.rs`):
- A heading resets `current`; only a heading of level ≤ 2 also resets `fence_base` (`:973-978`).
- Every **existing** mention under `crates/` sets `fence_base` (`:1001-1003`, `:1043-1045`). Every **file-shaped** mention sets `current` to `(raw, resolved, exists)`, dead mentions included (`:1004-1006`, `:1046-1048`).
- A fence opens seeded from `current` only if the file exists (`:917-923`). Mentions inside a fence are discarded (`:935`).
- `resolve_fragment(frag, sticky, base)` (`:701-729`):
  - it first returns `sticky` when `sticky.ends_with(frag)` (component-wise) and `sticky` is a file;
  - otherwise it starts at `base.or(sticky)`: the path itself if it is a directory, else its parent;
  - it then joins `frag` onto the start directory and each ancestor up to the root; the first existing file wins.
- An unresolved fragment, or a missing one, inherits `fence_target` (`:952-954`). With no `fence_target`, the anchor is skipped.
- Anchor columns: `:` for the suffix form, `(` for the bare form. `fragment_before` walks path bytes left from that column (`:681-692`). Line numbers are an unbounded `usize` (`:433`).
- An out-of-range line prints ``"  {doc}:{lineno}  `{raw}:{N}` is past end of file ({len} lines)"`` and returns before the waiver branch (`:815-822`). A waived anchor gets only that bounds check (`:846-860`). Classes are decided before the bounds check (`:800-813`). An unreadable target yields nothing (`:796`).
- A link label that starts with `crates/` is a mention in its own right (`:345-405`). `SYSTEMS.md:1019` holds 2 mentions.

---

## 1. Goal

Split the 58 lib/bin files of ≥ 1500 lines (171,250 lines) into modules named by responsibility. A split must:
- change no behaviour, and no body, signature, attribute, comment or formatting of moved text;
- prove that without relying on the compiler;
- repair gated anchors in the same change, and record every other drift.

| Metric | Target |
|---|---|
| Moved-item identity | 100% byte- and token-identical; every byte of the file lies in exactly one unit or the header |
| Runtime, allocation, codegen | unchanged by construction; the pilot is compile-time only |
| File size | target ≤ 800; ceiling ≤ 1500 unless waived |
| Tests | same leaf-name multiset and count in every leg (pilot: 56 lib tests) |
| Gated anchors | every anchor's tuple equals its required tuple, confirmed by both the port and the gate oracle; 0 stale; mentions = base + predicted Δ |
| Configurations | predicates exact over all atoms; every gating truth value compiled in some leg, or the split is refused |
| Public surface | same (path, kind, definition) set; equivalent existence predicates |
| Identity observations | no `persist`/`key` change; `display` changes recorded |

## 2. Context and constraints

**The pilot touches:**
- `crates/aether_lang/src/expand.rs`;
- 6 new files under `expand/` in stage 1, 12 after stage 2;
- `tools/refactor/**`.

**Invariants:**
- Moved text is byte-identical, apart from the uniform de-indent of extracted inline modules.
- The only added lines are:
  - `mod`, `use` and re-export lines, each with its computed `#[cfg]`;
  - D18 `#[path]` lines;
  - listed `pub(super)` widenings.
- No new file-top attribute, no `#[allow]`, no rustfmt.

**Environment:**
- Line endings: CRLF everywhere; no `.gitattributes`.
- Missing tools: no rustfmt gate, no `cargo-public-api`, no `cargo-modules`.
- Toolchain: `stable-x86_64-pc-windows-gnu`; `RUSTFLAGS` is never set; `.cargo/config.toml:84-91` sets `target-cpu=x86-64-v3` per target.
- Workspace lints: `unexpected_cfgs` (check-cfg `loom`, `force_alloc_panic`) and `clippy::disallowed_types = deny` (`Cargo.toml:25-38`). Members are listed explicitly (`Cargo.toml:2`).

**Legs available now:**
- the host;
- `--target x86_64-pc-windows-msvc`;
- crate-local `cargo rustc -- --cfg miri|force_alloc_panic`.

**Legs pending Q4:** linux-gnu, and the `--config` rustflags form.

## 3. Key decisions

### D1 — The parent survives; 2018 layout (`x.rs` + `x/`); children are private
- **What:** `x.rs` stays as the parent and gains a sibling directory `x/`. Test modules use `c.rs` + `c/tests.rs`. Target roots follow D18 instead.
- **Why:**
  - existing mentions stay valid;
  - `git log --follow` works on the parent;
  - `git blame -C` recovers moved lines.
- **Rejected:** renaming to `x/mod.rs` (forbidden, and it breaks mentions); one item per file.
- **Trade-off:** the tree keeps two module layouts (69 vs 4).

### D2 — Sizes: target 800, ceiling 1500, floor 60; one item is never split
- A file over 1500 lines FAILs unless waived.
- A child under 60 lines is merged, unless it is a whole entry of the construct registry.
- An item over 800 lines gets its own file, with a stated reason.
- **Why:** at about 23.3 tokens per line, 800 lines is about 18.6k tokens — one Read with a 25% margin. 1500 is the census definition of "oversized".
- An inherent `impl` is one item. Splitting one would repeat its header, which needs Q8(a).

### D3 — A split unit is a responsibility cluster, contiguous by default
- Names come from `desc/*.json` `key_items` and from the file's own section markers.
- A **break** is two consecutive items of one destination that were not adjacent in the source. Breaks are counted.
- A **reorder** is listed, with a reason.
- **Pilot:** 0 reorders; 14 breaks (1 in stage 1, 13 in stage 2).

### D4 — Name flow, stable paths, and the declaration-key rule
1. **Child header.**
   - `use super::*;` is emitted only if some child item resolves a name through it.
   - `use super::<sibling>::<name>;` is emitted only for a sibling edge the parent does not bind. D5 makes such edges the exception.
2. **Parent binds.** `use <child>::{…};` lists exactly the moved names that kept parent code uses, minus names a parent re-export already binds (E0252).
3. **Re-exports.**
   - (a) An item exported from the crate gets `pub use child::{…}`, with P(L) = EP_rel(def).
   - (b) An item not exported but still reached through the parent gets a re-export at its original visibility, with P(L) = the disjunction of its consumer sites.
   - (c) Anything else gets no re-export and is listed in `reexport_skipped`.
   - A glob re-export always FAILs. Evidence: `check_use_tree` exempts only exported imports, which B12 confirms.
4. Children never re-bind parent names.

**Declaration-key rule (R1, CH60).** Partition units and declared names come only from real items. The children of a `source_file`, or of an inline module's `declaration_list`, are classified by this table. The shapes are those of F20; `rsitems.py shapes` checks them against the pinned parser in B10.

| Item-level node | Class |
|---|---|
| `shebang`, `inner_attribute_item`, `use_declaration`, `extern_crate_declaration` | header kind; stays in place in the parent; never a unit |
| `line_comment`, `block_comment` (including doc comments), `attribute_item` | trivia of the next unit (or of the header) |
| `function_item`, `function_signature_item`, `struct_item`, `enum_item`, `union_item`, `trait_item`, `type_item`, `const_item`, `static_item`, `mod_item`, `foreign_mod_item`, `macro_definition`, `impl_item` | unit: named item, impl, satellite (`const _`) or twin |
| shape A: `expression_statement` whose only named child is `macro_invocation` | item-position invocation; the `;` is inside the node |
| shape B: bare `macro_invocation` | item-position invocation |
| shape C: an `empty_statement` that is the next non-comment sibling of a shape-B node | absorbed into that invocation's extent (the comments between them are included) |
| any other `empty_statement`; an `expression_statement` with any other child; `let_declaration`; `associated_type`; `ERROR`/`MISSING`; any kind not listed | exit 2 (tool error, never a guess) |

- The listed kinds are exactly F20's choice set plus comments. `rsitems.py` refuses to start if its table omits a kind from that set.
- Tokens inside any token tree declare nothing where they are written, and never cause a refusal (F17, N13).

**Names defined by an item-position invocation.** They come from the macro's definition:
- **(a1) Flat matcher.** A workspace `macro_rules!` whose matcher is a sequence of depth-0 `$x:frag` and literal tokens. Arguments bind by position.
- **(a2) Trailing name.** The metavariable that names a transcriber item (`$name` after `fn|struct|enum|union|trait|type|const|static|mod`) is the matcher's last depth-0 element, followed only by literal tokens. It binds to the invocation's last top-level token after those literals are stripped. That token must be an identifier; otherwise the rule does not apply. This covers `ffi.rs:267`/`:300` (N20).
- **(a3) Defines nothing.** The re-parsed transcriber contains only `impl` blocks and `const _` items. This covers `filter.rs`'s six macros (N21). Such an invocation is a satellite that defines no names; since it references no family item, it is placed explicitly.
- **Transcriber re-parse.** Each repetition group `$( … ) sep? op` is kept once, with its delimiters removed. Metavariables become placeholders. A re-parse failure means (a) does not apply.
- **(b)** A row in `tools/refactor/macro-names.toml`, with template and file:line evidence. Seed rows: `thread_local!`; `criterion_group!` (defines the fn named by its first argument); `criterion_main!` (defines `main`).

Defined names join the unit, the vacated-name set and `pubsurface`. An invocation covered by neither (a) nor (b) is **pinned**: it stays in its module, contributes no names, and `split.py check` reports `pinned-move` only against a plan that moves it. Derive and attribute macros are handled the same way.

**Lemma L1.** The convention adds only these bindings: `mod c;`, the child glob, explicit imports of moved definitions, and parent binds and re-exports. It removes only the moved definitions. So a name can resolve differently only if it is vacated, is a new module name, or sits in a module whose depth changed. Check 5 enforces this premise, and check 8 relies on it.

### D5 — Visibility and placement
- **Shared layer.**
  - An item used by at least two destinations other than its own stays in the parent.
  - An item used by exactly one other destination, and not by the parent, moves to that destination.
  - A sibling-only edge needs Q8(b).
- **Widening.**
  - A private top-level item becomes `pub(super)` only if kept parent code uses it after the move. Each widening is listed.
  - Widening a kept item so that moved code can use it (for example test helpers) needs Q8(c).
- **Associated items and fields** are never edited. A depth-relative visibility rewrite needs Q8(d).
- **Private members.** A type moves only together with every user of its private members (check 8b).
- `pub(crate)` is never used as a shortcut.

**Lemma L2.** For a move from P to P::C:
- `pub(super)` in P::C covers exactly what "private" covered in P;
- access to private members declared in P is unchanged;
- for a moved type, the set of places that can see its private members only shrinks, and check 8b rejects any shrink that removes a real use;
- `// SAFETY:` arguments that rely on module privacy keep the same set of code (`store.rs:1322-1325`, `:1347-1351`).

### D6 — Tests follow their subject
- A child that has pinned tests ends with `#[cfg(test)]` immediately followed by `mod tests;`. Both walkers recognise this form (`walker/mod.rs:346-400`, `check_hotpath_exceptions.py:130-158`).
- Shared helpers stay in the parent's test module as `pub(super)` (Q8(c)).
- A test child's header is the source test module's `use` lines, plus `use crate::<path>::tests::{helpers};`.
- The tool never adds `use super::*` to a test child. A `use super::…` carried over from the source makes the module depth-changed, which check 8 then compares in full.

### D7 — Proof of a pure move, in two layers
- **Byte layer:** each item's text is equal, allowing only the uniform de-indent, and never inside literals or block comments.
- **Token layer:** hashes of signature, body, attributes, docs and comments, with EOLs normalised. Visibility is compared separately.
- **Why:** M1 edits a comment and every cargo gate stays green.

### D8 — Plans are keyed by item and committed; maps are generated and committed
- Per split: `plans/<crate>/<stem>.toml`, plus `<stem>.map.json`, `<stem>.anchors.json` and `<stem>.drift.json`.
- **Why:** a rebase re-runs the plan, merge-time repairs are mechanical, and D14 reads the maps.

### D9 — The tree-sitter layer is vendored from `extract.py`, with a parity check
- `rsitems.py` copies these helpers: `T`, `collapse`, `parse_attr`, `attrs_of`, `tt_items`, `cfg_requires`, `string_value`, `summarize_attrs`, `vis_of`, `strip_raw`, `path_segments`, `expand_use`, `count_lines`, `scan_mods`, plus the mod-rs/`#[path]` rules.
- Pinned versions: tree_sitter 0.25.2, tree_sitter_rust 0.24.2, Python ≥ 3.11.
- `rsitems.py parity` must reproduce `extract/modules.json` (`id`, `file`, `lines`, `decl_line`, `fn_count`). Pilot anchor: 3542 lines / decl 55 / 107 fns.

### D10 — Public API without `cargo public-api`
- `census.py pubsurface` computes (path, kind, definition key, existence predicate E), where E is the conjunction along the declaration and re-export chains.
- The pass condition has two parts:
  - the (path, kind, def) sets are equal;
  - for every entry, E_before ≡ E_after under X1–X3, evaluated for every relevant matrix row.
- Macro-defined names from D4 (a1–a3)/(b) are included.
- Every crate in the closure compiles in every host leg, and the `cargo doc` warnings are a subset of the baseline.
- **Trade-off:** names generated by proc macros that `macro-names.toml` does not list are covered only by the compile.

### D11 — One commit per file; no rename; no `.git-blame-ignore-revs`
- The commit message names `git blame -C` and `git diff --color-moved=zebra --color-moved-ws=allow-indentation-change`.

### D12 — Anchors
- `anchors.py` ports these gate functions with `repo_root` as a parameter: `scan_line`, `scan_doc`, `check_anchor`, `leading_decl_name`, `looks_like_definition`, `fragment_before`, `resolve_fragment`.
- It must reproduce the gate's printed counts on the real tree, and pass the oracle (CH63), before it is allowed to rewrite anything.
- It rewrites gated docs per §7.3.
- Ungated forms are recorded only, never rewritten: bound forms, bare `name.rs:N[-M][~]`, `name.rs::ident`, module paths, path mentions, `.stderr` paths, TSV rows.

### D13 — No rustfmt
- Header order: self < super < crate < ident < glob < list.
- `mod` lines are alphabetical.
- Groups are separated by one blank line.
- `rustfmt --check` is advisory only.

### D14 — The size gate caps only files the campaign produced
- `tests/file_size_census.rs` caps `source` ∪ `outputs[].file`, taken from the committed maps, at ≤ 1500 lines, unless `size-waivers.toml` lists the file with a reason of at least 40 characters.
- Other files over 1500 lines are reported only. The owner's untracked `playground.rs` (1872 lines) therefore passes.
- **Anti-vacuity:** if any map exists, the capped set must be non-empty and some capped file must exist. The line counter is self-checked on CRLF input and on input without a final newline.
- Needs orchestrator approval.

### D15 — Identity and location observers
**What changes when code moves.**
- A changed module path changes `type_name`, `module_path!`, printed paths, symbol names and `TypeId`.
- A changed file:line changes `file!`/`line!`/`column!`/`Location::caller`, implicit `#[track_caller]` locations (`expect`, `unwrap`, `assert*`, indexing, overflow), and `boyko_log` site records (`boyko_log/src/macros.rs:109-116`). This applies to moved lines and to kept lines below a removed block.

**The scan.** `census.py observers` scans the workspace, including derive token text, for four kinds of site:
- **O-T:** `type_name::<X>()`/`type_name_of_val` where X is `Self` or a generic;
- **O-M:** `module_path!()`;
- **O-L:** `file!`/`line!`/`column!`/`Location::caller`, plus consumers of implicit locations (`set_hook` bodies, `.location()`);
- **O-I:** a `TypeId` flowing into ordering or a fixed-seed hash.

Every site needs a row in `identity-observers.toml` (file, owner, kind, class, and evidence of at least 40 characters). A site without a row makes `split.py check` refuse.

| Class | Meaning | Known rows |
|---|---|---|
| `persist` | outlives the process | the default `Component::stable_name` (F11) |
| `key` | an in-process key that affects control flow | none in census files; `asset/server.rs:376` recorded |
| `display` | text only | `Component::debug_type_name`, `Resource::debug_type_name`, `Plugin::name`, `SystemSet::set_name`, registry panic text, every `boyko_log` site, panic hooks (`boyko_log/src/sink/crash.rs:160`, `lifecycle.rs:625`), silencing hooks (`command_queue.rs:1573`, `rhi_vulkan/device.rs:4000`) |
| `unstable-by-contract` | depends on `TypeId` order | none found |

**O-T reach.**
- A moved type reaches a trait-default observer unless every impl or derive for it overrides the method (`[[derive_override]]`: `Component` + `stable_name`; enum `SystemSet` + `set_name`).
- A blanket impl or generic-fn observer reaches every moved type.

**Check 11.**
- `persist` reached through O-T → FAIL, unless either the type stays put, or a two-commit key pin exists:
  - commit A adds a test asserting the base `type_name` literal;
  - commit B adds `#[component(stable_name = "<same literal>")]`;
  - generic types are refused.
- `key`/`persist` O-L/O-M rows whose observed value can change → FAIL unless covered by an `[[ack]]` (reason ≥ 40 characters).
- `key` reached through O-T → FAIL unless acknowledged.
- `display` sites and implicit locations → recorded as drift.
- O-I sites → reported.

**Ruling (Q7).** Text consumed only by humans or logs is not behaviour: paths, file and line numbers in panic, error and log text, log-site records, and implicit panic locations.
- Line numbers shift below every removed block, so any other ruling would forbid every split.
- Nothing keys on this text.
- Pinned text is caught by S6 and by reader class R6.

### D16 — cfg on generated lines, decided exactly
**Atoms.** An atom is any predicate atom that appears in:
- the family's `#[cfg]`/`#[cfg_attr]` attributes;
- the existence predicates of the names bound by generated lines;
- plus `test`.

No atom is ever replaced by a host value.

**Axioms.**
- **X1:** `target_arch`, `target_os`, `target_env`, `target_vendor`, `target_abi`, `target_pointer_width`, `target_endian` and `panic` each take at most one value.
- **X2:** the crate's `[features]`: `a = ["b"]` gives a → b; `dep:` and `crate/feat` entries are ignored.
- **X3:** the rows of `cfg-axioms.toml`: `target_os="windows" → windows`, `windows → not(unix)`, `target_os="linux" → unix`.
- `census.py axioms --verify` checks every row against `rustc --print cfg --target T` for all targets (M35).
- All other atoms are independent.

**Use sites.**
- A use site u of line L is a reference that resolves through L: a syntactic reference, a strong token-tree hit, an implicit capture, or a `quote!` interpolation.
- For a glob line, every hit resolved through the glob.
- For names inside a family-local `macro_rules!` body, the invocation sites.

**Effective predicate.** EP(u) is the conjunction, from the family root down to u, of the cfgs on:
- in-family `mod` declarations;
- items, impls and associated items (including the `attribute_item` trivia of an invocation unit);
- fields and variants;
- match arms and statements;
- expression attributes;
- fn parameters;
- enclosing `cfg_attr(P, …)` conditions.

`cfg!` and `debug_assert!` contribute nothing. EP_rel factors out P_m.

**P(L)** = ⋁ EP_rel(u) over the use sites of L, with two exceptions: a D4.3(a) re-export takes EP_rel(def), and a child `mod` line takes ⋁ EP_rel(item).

**Emission** (exact under X1–X3 and P_m):
- P ≡ true → no attribute;
- P ≡ EP_rel(u₀) → emit u₀'s predicate texts verbatim;
- otherwise → `any(<distinct texts in source order>)`;
- names that need different predicates get separate `use` lines.

**Why this is exact:** on every assignment consistent with X1–X3, L exists exactly when one of its use sites is compiled, so E0432, E0425 and `unused_imports` cannot arise.

**Traced examples:**
- `schedule.rs:89-90`/`:769-770` → `#[cfg(not(miri))]` (N9);
- `ffi.rs:36-37` → `#[cfg(windows)] pub use <child>::os;` (N10);
- `store.rs:1318-1328`/`:514-520` → `#[cfg(feature = "profiling-analysis")]` (N6).

**cfg inside macro token trees** (R1: these never declare anything and never cause a refusal).
- **(i)** In the quote family and in fully opaque macros, `#[cfg` tokens are output text and are ignored.
- **(ii)** In a family-local `macro_rules!` transcriber, the fragment is re-parsed as in D4. Its cfgs join the EP of the names they enclose, at each invocation site. If the fragment does not parse, the **co-location pin** applies to that transcriber's invocation sites.
- **(iii)** Any other macro whose token tree contains `#[cfg` or `#[cfg_attr` (for example `proptest!`) gets the **co-location pin**.

**Co-location pin (CH64).** For a token tree T covered by (ii) or (iii):
- the item that contains T keeps its source module;
- so does the family definition of every name used inside T.

Thus no generated line, glob included, ever has to serve a name whose use-site predicate is invisible. `split.py check` reports `pinned-move <key>` for a plan that moves either one (M32, M43). A plan that keeps both passes. Today no free file has such a tree: the `proptest!` blocks at `assets.rs:1714`, `:1835`, `:2592` and `compute/tests.rs:1225` contain no `#[cfg`.

**Refusals (FAIL):**
- P(L) ⇏ EP_rel(def) (a counterexample is printed);
- more than 12 atoms after X1 grouping;
- `any(…test…)` on a `mod` line;
- `cfg_attr(P, cfg(Q))`;
- anti-blindness: an unattached `#[cfg` in ordinary code → exit 2;
- a generated `mod` attribute that differs from the computed P_mod.

An `inline_to_file` `mod` line carries its original attributes verbatim. Only atoms that already exist are used, so no new check-cfg names appear. **cfg twins** form one unit, keyed without an ordinal.

### D17 — Reader census: code that reads a split file
**Scope.** Every tracked file except:
- `target`, `.git`, `graphify-out`, `node_modules`, `tools/refactor/**`;
- docs (`docs/**`, `book/**`, `*.md`), unless code reads them (R5).

Binary files are scanned only for R6.

| Class | Finds |
|---|---|
| R1 | a string literal containing the family basename or a child path |
| R2 | `include_str!`/`include_bytes!`/`include!` of a family file |
| R3 | a `<stem>` literal in a `.join(`/`.with_extension("rs")`/`format!("{}.rs")` chain |
| R4 | a directory walker rooted at an ancestor, recursive or flat; its host crate joins S7 |
| R5 | a data file that code reads and that mentions a family path (`HOT-PATH-EXCEPTIONS.md`, `print_allowlist.txt`) — `path-gate` |
| R6 | a literal or byte string with a moved item's base Rust path — `identity-literal` |

**Dispositions:** `presence-exact`, `count-exact`, `absence`, `count-upper-bound`, `walker-flat`, `walker-recursive`, `path-gate`, `identity-literal`. Each finding needs a `[[reader]]` row.
- **Silent classes** need a prerequisite commit that widens the reader and carries its own mutation.
- **Loud classes** need `pin_in_parent` or that same prerequisite.
- **`path-gate`** is re-pathed from the map.
- **`identity-literal`:** listed if it is in a test assertion; if it is in a data file, FAIL unless a key pin covers it.

**`.stderr` files.** A file may be re-blessed only if its diff is limited to:
- mapped source paths;
- mapped Rust paths of moved items;
- the impl-sample block, provided every listed type implements the trait at base (M25).

`docs/threadpool/receipts/*.stderr` are recorded as drift, never rewritten.

**Anti-vacuity:** B11 (the known readers are found); N8 (the campaign's own plans yield no finding).

**Invariants:** no `Cargo.toml` and no `src/` under `tools/refactor/` (`manifest_no_third_party_log.rs:60-69`; `production_reachability_census.rs:882-893`).

**Pilot:** 0 readers.

**Known recursive walkers** (a lower bound):
- root package: `production_reachability_census.rs:904-910`, `ignore_reasons_census.rs:320-330`, `gpu_blocking_reader_census.rs:103-113`, `fill_reject_routing_census.rs:492-501`, `trybuild_corpus_compiler_witness.rs:127-131`;
- `boyko-log`: `walker/mod.rs:34-57`;
- `boyko-app`: `profiling_artifact_roundtrip.rs:776-796`.

### D18 — Target roots (needs Q8(e))
- **What counts:** the `src_path` of any non-lib Cargo target.
- **Layout:**
  - children go in `<root dir>/<root stem>/<child>.rs`, each declared by `#[path = "<root stem>/<child>.rs"]` directly above `mod <child>;`;
  - never create `<stem>/main.rs`;
  - children are leaves;
  - `<stem>/` must not already exist;
  - no stage 2;
  - an inline `#[cfg(test)] mod tests {}` moves only through `inline_to_file` in the same form.
- **Rules:**
  - each gate classifies the children exactly as it classifies the root;
  - legs add `required-features`;
  - a crate-level `#![cfg]` removes the children along with the root.
- The root's `criterion_group!(…);`/`criterion_main!(…);` are shape-A units (F21) whose names come from the (b) seed rows. They stay in the root: `criterion_main` defines `main`.
- **Rejected:** `<root dir>/<child>/mod.rs`; `autobenches = false` + D1; renaming.
- **Trade-off:** one extra attribute line per child, hence Q8(e).

### D19 — Configuration matrix, legs, coverage

`configs.toml`:

| Row | Source | Target | Mode | Features | cfg | Gating | Builds |
|---|---|---|---|---|---|---|---|
| `ci.check` | `ci.yml:77-84` | linux, v3 | check dev | ecs/profiling-analysis | — | yes | workspace minus demo, bevy-bench; all targets |
| `ci.test.debug` | `:96-111` | linux, v3 | test dev | same | — | yes | same |
| `ci.test.release` | `:109` | linux, v3 | test release | same | — | yes | same |
| `ci.profile-legs` | `:135-151` | linux, v3 | check dev | default | — | yes | same |
| `ci.profile-census` | `:166-175` | linux, v3 | test | default | — | yes | `profile-fixture` |
| `ci.clippy` | `:177-189` | linux, v3 | clippy | profiling-analysis | — | yes | as `ci.check` |
| `ci.bench-compile` | `:191-198` | linux, v3 | bench | default | — | yes | workspace minus demo |
| `ci.force-alloc-panic` | `:200-216` | linux, v3 | test release | profiling-analysis | `force_alloc_panic` | yes | as `ci.check` |
| `ci.miri` | `:218-262` | linux, baseline ISA | miri test | default | `miri` | yes | ecs, utils, threadpool, serialize, math, sdf_math, image |
| `ci.loom` | `:310-370` | linux, v3 | test release | default | `loom` | yes | `boyko-threadpool --test loom_pool` |
| `docs.doc` | `docs.yml:83-102` | linux | nightly doc | default | `doc` | no | workspace minus four |
| `docs.wasm` | `docs.yml:148-167` | wasm32 | release | default | — | no (red at base) | `boyko_demo` |
| `local.host` | CLAUDE.md | windows-gnu, v3 | check/test/clippy dev + release | per leg | — | yes | per split |
| `local.msvc` | platform notes | windows-msvc, v3 | check | per leg | — | yes | per split |

**Drift guard.** `census.py configs --verify` parses every job in `.github/workflows/*.yml`: `runs-on`, `RUSTFLAGS` (cfg, cpu), toolchain, subcommand, `--release`, `--features`, `-p`/`--workspace`/`--exclude`, `--target`, `continue-on-error`.
- It exits 1 if a job has no row or disagrees with its row (M34).
- It runs in B10 and S0.

**Relevant rows** are those whose resolved graph builds the split crate.

**Legs.**
- Foreign legs: the realised assignment is measured by B14.
- Host legs: the feature assignment is read from the command's flags.

| Leg | Command (with the standing prefix) | Realises | Status |
|---|---|---|---|
| L-host | `cargo check`/`clippy`/`test -p <crate> --all-targets [--features …]` | windows-gnu, v3, dev, a feature subset, both `test` values | available |
| L-host-rel | + `--release` | `debug_assertions` = false | available |
| L-msvc | `cargo check -p <crate> --all-targets --target x86_64-pc-windows-msvc` | `target_env="msvc"` | available |
| L-cfg(a) | `cargo rustc -p <crate> <target> --profile check\|test\|bench [--release] -- --cfg a`, a ∈ {`miri`, `force_alloc_panic`} | `a` for the split crate only | only if `a` gates none of the crate's dependencies |
| L-noisa | L-cfg + `-C target-cpu=x86-64` | baseline ISA | only if B14 shows the later flag wins |
| L-linux | `--target x86_64-unknown-linux-gnu` | `unix`, not(`windows`) | Q4(a) |
| L-config(a) | `--config 'target.x86_64-pc-windows-gnu.rustflags=["--cfg","a"]'` | `a` for the whole graph | Q4(b) |

**Coverage (check 14), over the relevant gating rows:**
- **C-a:** every non-tautological P(L) (and EP(def) for re-exports) reaches, in some compiled leg, every truth value that any gating row reaches.
- **C-b:** every cfg-bearing node inside moved text is compiled in some leg.
- **C-c:** a leg counts only if the crate compiles clean in it at base (B15/S0b).
- Otherwise FAIL with `needs-leg <atom>=<value> (rows …)`.

**Exemptions:**
- **E1:** an inline module moved to a file at the same path, recorded as `compile-unverified`.
- **E2:** truth values reached only by non-gating rows, recorded as `symbolic-only`.

**Limits:**
- L-cfg compiles only the split crate; dependents are covered by D10.
- At most 10 legs per split.
- Foreign legs run last (S4f).

**Worked cases:**
- pilot: host only;
- `store.rs`: default, and `+profiling-analysis`;
- `schedule.rs`: L-cfg(miri);
- `ffi.rs`: moving `os` needs L-linux (Q4(a)); keeping `os` needs nothing; extracting it in place is E1;
- `component_registry/mod.rs:132`: E2.

---

## 4. The convention (deliverable 1)

**Unit and naming.**
- A child is a responsibility cluster, named in `snake_case`, placed per D5.
- A child name must not collide with:
  - an extern-prelude crate;
  - a type-namespace name in the parent;
  - an existing child;
  - a symbol of at least 3 characters that is paired with a gated anchor bound to a family file.
- A production child of a file that is not a target root is never named `tests`. No produced path gains a `tests/`, `benches/`, `examples/` or `bin/` segment.
- Target-root children follow D18. Test children are named `tests`.

**Parent layout, in order:**
1. the unchanged `//!` and `use` blocks;
2. a blank line;
3. the `mod` group (private, alphabetical, computed cfg; plus `#[path]` for target roots);
4. a blank line;
5. binds;
6. re-exports;
7. the remaining units, in source order;
8. the test pair.

If a child invokes a parent `macro_rules!`, the `mod` group goes after the last such definition.

**Stable paths.**
- Exported items get explicit `pub use` lists with P = EP_rel(def).
- Non-exported items are re-exported only if something still reaches them through the parent.
- Skipped re-exports are listed.

**Travelling text.**
- A unit carries its outer attributes, its docs, its leading free comments and a trailing same-line comment. For an invocation it also carries its `;` (shape A inside the node; shape C absorbed).
- Twins travel together.
- Satellites (`const _`, and invocations that define nothing) travel with the single family item they reference. Otherwise they are placed explicitly.
- Pinned units stay in place.
- `// SAFETY:` comments travel with their item.
- Intra-doc links must resolve to the same item.
- A `macro_rules!` definition stays above every bare invocation.
- File-top `#![…]` stays in the parent. An inherited blanket `#![allow(clippy::disallowed_types)]` is recorded in the scope cell of its registry row.
- An extracted inline module's inner attributes and `//!` lines become the new file's top lines. An inline module whose body starts with `#![cfg` is never extracted.

**Never changes:**
- bodies;
- signatures (except listed visibility changes);
- attributes, doc text, comments, blank lines inside items, literals;
- in-destination order (except listed reorders).

No rustfmt, no provenance `//!`, no `#[allow]`.

---

## 5. Data structures

```toml
# tools/refactor/plans/<crate>/<stem>.toml — "split-plan/6" (committed)
schema = "split-plan/6"
source = "crates/aether_lang/src/expand.rs"
base_rev = "d552be05be4b4f063b6cb39ddfd63eb4688f83fd"   # stage 1: blob(source) == blob(base_rev:source)
after_plan = ""                                        # stage 2: the plan whose output this source is
target_root = false                                    # computed (D18); pinned
legs = ["host"]                                        # computed (D19); pinned
[parent]
# KEY GRAMMAR (declaration-key rule, D4 / R1): keys name REAL ITEMS only (the item-level table).
#   named item:  "<kind> <name>[#k]"    kind ∈ fn struct enum union trait type const static macro_rules mod extern
#   cfg twins:   "<kind> <name>"        (no ordinal)
#   impl block:  "impl[<generics>] [<trait path> for ]<self type>[#k]"
#   satellite:   implicit; explicit form "const _@<8-hex body hash>"
#   macro item:  "macro <path>!@<8-hex token hash>"   (any shape A/B/C; names from D4 a1-a3/b; else pinned)
#   #k = source-order ordinal among identical keys
keep = ["fn expand", "..."]
inline_to_file = [{ item = "mod tests", file = "crates/aether_lang/src/expand/tests.rs" }]
bind = [{ use = "data::{bundle, component, event, tag}", cfg = "" }]
reexport = []            # [{ vis, use, cfg }]; globs refused
reexport_skipped = []    # [{ item, consumers = 0 }]
[[child]]
module = "data"; file = "crates/aether_lang/src/expand/data.rs"; declare_in = ""
mod_cfg = ""; mod_path = ""
items = ["fn component", "fn tag", "fn bundle", "fn event"]
order = []; widen = ["fn component", "fn tag", "fn bundle", "fn event"]; vis_rewrites = []
imports = [{ use = "super::*", cfg = "" }]
expect_lines = 95; reason = ""
[[reader]]  file = "..."; fn = "..."; class = "absence"; disposition = "prereq"; prereq = "<commit>"; needle = ""
[[keypin]]  item = "struct X"; literal = "crate::m::X"; test_commit = "<sha>"; pin_commit = "<sha>"
[[stderr]]  fixture = "..."; classes = ["source-path", "impl-sample"]
[[ack]]     module = "..."; name = "..."; item = "..."; reason = "..."
[[waive]]   kind = "e1"; atoms = ["windows"]; reason = "..."
```

**Map (`split-map/6`, generated and committed).** Fields:
- `plan`, `source`, `source_blob`, `eol`;
- `outputs[]` (file, blob, lines);
- `items[]` (key, unit, **shape** (item/A/B/C), old/new ranges and declaration lines, module, vis, ep, pick);
- `coverage_bytes` (non-whitespace bytes, split into header/unit/trivia; must sum to the total);
- `generated[]` (file, line, text, cfg, path_attr, use_sites with EP, per-row truth);
- `legs[]`, `coverage[]`, `exemptions[]`, `pins[]` (key, reason, co-located keys), `observers[]`, `doctests[]`, `tests[]`;
- `lines` (old line → [file, line] or null).

**Anchor plan (`<stem>.anchors.json`).** Per gated doc:
- `base` (mentions, dead, anchors, class vector, over-waived count and set);
- `rows[]` (`id` = (doc line, ordinal); base T; required T′; `route` and `depth` for fenced rows; `edit` ∈ none/digits/relink/convert/fragment);
- `mention_delta` (per prose line);
- `oracle` (sentinel per row, length per file, expected set);
- `refusals[]`.

**Tables:**
- `configs.toml`;
- `cfg-axioms.toml`;
- `identity-observers.toml` (`[[site]]`, `[[derive_override]]`);
- `macro-names.toml`;
- `size-waivers.toml`.

**Per-item census record:** key, kind, shape, name, ordinal, vis, ep, attrs, hashes, byte_ok, `unsafe`/`SAFETY` counts, relative paths, hits, use sites, private members, observers, macro calls, pin reason.

---

## 6. Tooling (deliverable 2): `D:/wt/refactor/tools/refactor/`

| File | Role |
|---|---|
| `rsitems.py` | Vendored layer (D9). **Item-level classification table and shapes A/B/C (CH60); refuses to start if the table misses a F20 kind.** Item model with trivia-inclusive extents, attribute attachment on every carrier, twins, satellites, co-location pins, hashes, literal/comment ranges, relative paths, intra-doc links, macro definitions/invocations with defined names (a1–a3/b), repetition-aware transcriber re-parse, captures, interpolations. Crate module tree including `#[path]`. Exact predicate engine (≤ 12 atoms; X1–X3). Per-configuration binding tables; local scopes. Subcommands `parity` and **`shapes`** (parses the N19 corpus and one node of each F20 kind with the pinned parser, and asserts the expected shape per case). |
| `split.py` | `list`, `check`, `apply`, `clean` — see Rev-5 duties below |
| `census.py` | `snapshot`; `verify --base R --plan P… [--json]` or `--before DIR --after DIR --map M`; `pubsurface`; `readers`; `observers`; `legs`; `configs --verify`; `axioms --verify`; `sizes`; `--self-test`. Exit 0 = pure move, 1 = violation, 2 = tool error. `--json` lists the ids of failing checks. |
| `anchors.py` | `parity --root R --gate-log LOG`; `trace --root R DOC` (prints the branch trace with route and depth); `plan --map… --before R0 --after R1`; `apply --plan A`; `twin --root R`; **`oracle --root R --plan A --side before\|after`** (§7.3 (f)); `drift`; `--self-test`. The root is always a parameter. |
| `mutate.py` | `apply` and `revert`, SHA-verified; asserts the exact set of failing ids; no git |
| `probe.py` | Throwaway crates under `D:/wt/_targets/refactor/probes/`, each with an empty `[workspace]` and its own `[lints.rust] unused_imports = "deny"` (plus per-row lints). Runs any D19 leg, the twin, and the oracle. |
| `fixtures/**` | `.rs.in`, `.md.in`, `.toml.in`, `.yml.in` only. Materialised under `D:/wt/_targets/refactor/selftest/`. Bytes that are not valid UTF-8 (FX-DIR) are written at materialisation time and never committed. |
| `plans/aether_lang/` | `expand.toml`, `expand-tests.toml`, generated maps, `expand.drift.json` |
| tables | `configs.toml`, `cfg-axioms.toml`, `identity-observers.toml`, `macro-names.toml`, `size-waivers.toml` |

**Duties unchanged from Rev 5:**
- **`split.py check`:** partition, twins, satellites, pins; names; widen/import/bind/re-export/cfg sets; D5; child names; D18; sizes and waivers; macro order; readers; observers; key pins; `.stderr` classes; legs and coverage.
- **`split.py clean`:** deletes only outputs whose blob matches the map.

**Script reuse.**
- `census.py` imports `scripts/check_hotpath_exceptions.py` via `importlib` (guarded by `__main__`, `:396`) and uses `test_only_files`, `cfg_test_spans` and `item_is_cfg_test`, plus a verbatim port of the `scan_sources` loop (`:252-286`).
- Base files are materialised from `git show <base>:<path>` under `D:/wt/_targets/refactor/base/<rev>/`.
- The walker's `test_only_files` (`walker/mod.rs:318-406`) is ported. Its parity anchor is the documented count of 7; a mismatch is escalated.

---

## 7. Algorithms for the critical paths

### 7.1 `split.py apply`
1. Verify the blob and EOLs. Refuse mixed EOLs, a BOM, tabs, or non-UTF-8 input.
2. **Parse (CH60).**
   - Classify every child of `source_file`, and of each inline module's `declaration_list`, with the D4 table. An unclassifiable node → exit 2.
   - Units are named items, impls, satellites, twins and item-position invocations in shapes A, B and C.
   - Header-kind nodes stay where they are.
3. Compute extents, including trivia. A shape-C unit's extent ends at its `empty_statement`.
   - **Coverage precondition:** every non-whitespace byte lies in exactly one header node, unit extent, or trivia run attached to one. Otherwise exit 2.
   - Group units.
4. Plan:
   - resolve keys;
   - check the partition, pins (co-location) and D5;
   - compute the widen/import/bind/re-export sets, use sites and predicates (D16), and rows, legs and coverage (D19);
   - compare the results with the plan.
5. Write the children: header groups, a blank line, then the units. Stage 2 appends a blank line, `#[cfg(test)]` and `mod tests;`.
6. Write the parent:
   - the unchanged header, then the generated groups;
   - where a unit is removed, keep the gap before it and drop the gap after it;
   - gaps between kept, adjacent units are never touched;
   - an inline module becomes its outer attributes followed by `mod name;`.
7. Extract inline modules:
   - drop `mod x {` and `}`;
   - de-indent, except for lines that start inside a literal or block comment;
   - refuse other under-indented lines;
   - refuse a body that starts with `#![cfg`.
8. Write with the source EOL, then the map.

The whole pass is O(file) and single-threaded.

### 7.2 `census.py verify` — checks 1–16 (FAIL unless noted)

**1. Partition.** The multiset of units is equal, including twins, satellites, pinned units, invocation units by shape, and macro-defined names. Pilot anchor: 107 fns.

**1b. Byte coverage (CH60).** On both sides, the §7.1 step 3 coverage holds. The base's unit bytes, minus uniform de-indent whitespace, equal the family's unit bytes. A classifier that drops a node kind fails here (M40).

**2. Identity.** Byte layer and token layer.

**3. Visibility.** Unchanged, except `widen` and Q8(d).

**4. Placement.**
- Module paths match the plan.
- D5 holds.
- Pinned units and their co-located keys stay put.
- Reorders are listed; breaks are reported.

**5. Header whitelist.**
- Added non-blank, non-unit lines are limited to:
  - `mod x;`;
  - plan imports, binds and re-exports;
  - the test pair;
  - directly above a generated line: its computed `#[cfg]`, and `#[path]` for target roots.
- Removed non-unit lines are limited to the extracted `mod x {`/`}`.
- **A `;` line or trailing `;` is never a header line.** A split that leaves an invocation's `;` behind fails here (M41).
- Blank lines are ignored.
- The source's `//!` and `use` lines appear verbatim in their destination.
- This check is L1's premise.

**6.** No glob re-export.

**7.** Relative paths and intra-doc links resolve to the same item, including in `cfg(test)` text.

**8. Resolution preservation.**
- **Scope:**
  - every family module;
  - every in-crate module that glob-imports one, transitively;
  - consumer paths through the family, resolved fully to the same definition key.
- **Names:** vacated names and new module names. Depth-changed modules are compared over all names.
- **Tables:** built per relevant row × `test` × feature subset, and holding:
  - explicit items and imports;
  - the glob fixpoint;
  - the extern prelude, the edition-2024 std prelude, and built-ins;
  - out-of-crate globs marked OPAQUE.
- **Local scopes:** parameters, `let`, `if let`/`while let`, arms, `for`, closures, inner item blocks.
- **Hits:**
  - syntactic: path heads, `type_identifier`, pattern-capable patterns, macro paths;
  - captures in std format macros are strong, elsewhere weak;
  - quote family: `#ident` (including inside `#(…)`) is strong. The `quote_spanned!` prefix before the top-level `=>` is an expression. Every other template token gives no hit (R1);
  - fully opaque macros give no hits;
  - other macros:
    - `n ::` is a strong type hit;
    - `n !` is a strong macro hit;
    - `n (` or `n {` is a strong hit;
    - a bare `n` is a weak hit.
  - **Invocation of a family-local `macro_rules!` (CH61):** the re-parsed transcriber's identifiers, with metavariables substituted by the invocation's arguments where (a1)/(a2) bind them, are hits in the *invoking* module under the same rules. Moving an invocation therefore compares every name its expansion resolves.
- **Verdicts:**
  - strong, pre ≠ post → FAIL;
  - strong, pre = def and post OPAQUE-possible or unresolved → FAIL;
  - weak, pre = def and post ≠ def → FAIL unless acknowledged;
  - weak, pre unresolved or local, and post a pattern-capable def → FAIL;
  - otherwise OK.
- Traits in scope must be preserved.
- **Pilot predictions:**
  - `#data` at L348–353 → the local at L338 (these lines stay in the parent);
  - `scene.name` at L1189 → local;
  - the F12 captures and `quote_spanned!` prefixes → locals;
  - `core::system::…` → not a path head;
  - `let bundle` at L1177 → local.

**8b. Privacy regions.** For every private or restricted field and associated item in the family, every candidate use must stay inside its region:
- `self.F`, `Self::F`, `Type::F`;
- literals and patterns;
- name-matched `x.F`, `x.F(` and `::F`.

B13 decides whether fall-through is silent. Pilot: empty.

**9.** Macro textual order and `#[macro_use]`. `#[path]` only in D18 form or inside moved text. `include_*!` paths still resolve.

**10. Orphans.** Every family file is reached by a `mod` line. No child starts with `#![cfg`. No forbidden path segment appears. D18 rules hold.

**11. Observers.** The D15 rule applies. Pilot: nothing reached; a drift note lists implicit locations.

**12. Sizes.** D2 plus waivers.

**13. Accounting.**
- Lines per file.
- Equal `unsafe`/`SAFETY` totals.
- The hot-path prediction from the script's own classifiers.
- Per-gate classification invariance under the script and under the ported walker (F16).
- The test path map.
- `reexport_skipped`.

**14. cfg consistency and coverage.**
- Generated attributes equal the exact predicates.
- P(L) ⇒ EP_rel(def).
- Anti-blindness finds 0 unattached cfgs.
- Pins are respected.
- No `any(…test…)` on a `mod` line; ≤ 12 atoms.
- `configs --verify` and `axioms --verify` pass.
- C-a/C-b/C-c hold, or E1/E2 are recomputed.
- The truth table is reported per row and per leg.

**15. Readers.** D17, re-run on the split tree.

**16. Doctests.**
- Fenced doctest text lies inside identity-verified items.
- `compile_fail` crate paths resolve to the same definition key.
- WARN when such a path is unguarded (`filter.rs:955-967` is guarded by `:938-950`).

### 7.3 `anchors.py` — rewriting gated anchors

**Step 0 — parity on the real tree.** `parity --root D:/wt/refactor` reproduces B5's printed per-document values:
- mentions, dead, anchors, stale;
- the four-way decomposition;
- the **over-waived count** (a green run prints only the count, `:1330`) (CH67).

**Step 1 — branch trace.** Each doc line yields rows naming the gate branch they reached:

| Kind | Branch |
|---|---|
| `H≤2` | heading; resets `current` and `fence_base` |
| `H3+` | heading; resets `current` only |
| `IGN` | ignore-marker line |
| `M-file` | existing file under `crates/` (sets base and current) |
| `M-dir` | existing directory under `crates/` (sets base only) |
| `M-other` | existing file outside `crates/` (sets current only) |
| `M-dead` | missing path (sets current with exists = false if file-shaped) |
| `M-label` | a `crates/…` link label, in addition to its target row |
| `A-prose` | bound to `current` |
| `A-prose-unbound` | no `current` |
| `A-prose-missing` | `current` does not exist |
| `F-open` | fence opened; seed = `current` if it exists, else none |
| `A-fence-frag/<route>/<depth>` | fragment resolved (routes and depth below) |
| `A-fence-frag-unresolved` | fragment present, unresolved; inherits `fence_target` |
| `A-fence-inherit` | no fragment; inherits |
| `A-fence-unbound` | no `fence_target` |
| `A-unreadable` | target unreadable (not counted) |

**Routes (CH62).** Taken literally from `resolve_fragment` (`:701-729`):
- `sticky-self`: the early return (`sticky.ends_with(frag)` and `sticky` is a file); depth 0.
- Otherwise the start is `base.or(sticky)`:
  - `base-dir`: `base` is a directory; start = `base`;
  - `base-parent`: `base` is not a directory; start = its parent;
  - `sticky-dir`, `sticky-parent`: the same, with `base` = None.
- **Depth** = the number of `parent()` steps taken after the start before the join succeeded (0 = found at the start).
- `sticky-dir` needs a file-shaped mention that resolves to a directory. If a trace ever prints it, the tool exits 2; no fixture exists for it.

Each row carries T = (kind, bound path, N, M, waived, class), plus route and depth for fenced rows.

**Step 2 — required table.**
- `id` = (doc line, ordinal on the line) is stable, because edits change substrings only and never add or remove an anchor.
- For each base row:
  - **bound outside the family:** T′ = T.
  - **bound inside the family:**
    - (path′, N′) = `map.lines[N]`;
    - M′ must map to the same file as N′ and be contiguous there, else FAIL `range-split`;
    - waived is unchanged;
    - class changes only by an attributed flip — F1 (the declaring file changed) or F3 (a Q8(d) rewrite). F2 (a new `mod` line declares the paired symbol) is a FAIL and is prevented by naming.
  - **line removed by the split:** FAIL.
  - **unbound or unreadable:** stays unbound or unreadable.
- **Allowed kind changes:**
  - among the fenced bound kinds (`A-fence-inherit`, `A-fence-frag-unresolved`, `A-fence-frag/*`), in either direction, **only if** T_after.path == T′.path, with route and depth recorded;
  - no other kind may change.

**Step 3 — edits (minimal).**
- **digits:** same file; only N and M change.
- **relink:** a link whose target and label are rewritten together (R3). A `crates/…` label becomes the new full path; a basename label becomes the new basename. `SYSTEMS.md:1019` is the worked example: 2 mentions before and after.
- **convert:** in a sticky run, every bare `(N)` from the first anchor whose required path differs from the current binding, up to the run's end (the next heading, the next mention that sets `current`, or the next fence), becomes `([L](rel):N…)`. L copies the governing label's style: a `crates/` label adds +2 mentions, any other adds +1.
- **fragment (the fence seeding rule, CH66).** After all prose edits, re-simulate on the after root. For each fenced row whose after-binding ≠ T′.path (because the seed or `fence_base` moved, or its own file split), let S be the **shortest path suffix of T′.path** that `resolve_fragment(S, current_at_line, base_at_line)` resolves exactly to T′.path. Then:
  - suffix-form row: **replace** the existing fragment (or the empty one) with S, immediately left of the `:`;
  - bare fenced `(N…)` row: rewrite to `(S:N…)`. Here `(` followed by a letter opens no anchor (`:420`, `:432`), the `:` anchor forms, and `fragment_before` returns S (`:681-692`). The port verifies that the line's anchor count is unchanged and that the new anchor's fragment is exactly S.
  - No such S → FAIL `fence-unrepairable` at plan level.
- **Scope invariant (CH65).**
  - An edited line must either hold a binding to a family file, or be a fenced row the seeding rule must repair, whatever file it is bound to.
  - An edit touching a line bound to an S0a-blocked file (for example the headers of `archetype.rs`/`archetype_master.rs`) is refused.
  - Ungated docs are never edited.

**Step 4 — post-conditions** (checked by `apply`, and again in S10).
- **(a)** Every row: T_after == T′, including prose, fenced and waived rows.
- **(b)** Mentions (CH68). For each doc:
  - mentions_after = mentions_base + Δ, where Δ = Σ over edited **prose** lines of `len(scan_line(new).mentions) − len(scan_line(old).mentions)`. Fence lines contribute 0 (`:935`);
  - dead_after = dead_base (0 on the real tree);
  - every relinked label resolves to the same file as its target.
- **(c)** Anchor count unchanged; stale = 0; decomposition = predicted.
- **(d)** Over-waived: the count equals base and stays ≤ `OVER_WAIVED_MAX` (`:1302-1317`). The port's over-waived set equals the base set mapped through the map. This is a port-to-port comparison, since line text is byte-identical.
- **(e) Gate twin.** A probe crate at `<root>`:
  - `[package]` with `autobins = autotests = autobenches = autoexamples = false`;
  - `[lib] path` pointing at an empty file;
  - one `[[test]]` for a byte copy of `tests/internal_docs_anchors.rs`;
  - an empty `[workspace]`.

  It runs `cargo test --test internal_docs_anchors -- --exact internal_docs_cite_paths_that_exist internal_docs_line_anchors_land_on_definitions --nocapture`. The printed counts must equal the port's. On the real tree, the twin is B5/S10 itself.
- **(f) Gate oracle (CH63; answers critic Q1).** The gate itself reports each anchor's binding. Materialise `D:/wt/_targets/refactor/oracle/<side>/`:
  - every tracked path of that side (the base via `git ls-tree`/`git show`; the after side from the working files). Omit `.cargo/`;
  - every file that some row of the port binds is copied, then padded with k_f appended empty lines, with k_f distinct per file, so its line count L_f is unique;
  - every other tracked file becomes an empty placeholder (the gate reads only its existence);
  - the gated docs are copied with each row i's N replaced by S_i = 1 + max_f L_f + i. A range becomes `S_i-(S_i+1)`, and `~` keeps its side. Digit edits cannot create or remove an anchor, change pairing, or touch a mention (`:410-489`);
  - the root `Cargo.toml` is replaced by the twin manifest from (e).

  Then run `internal_docs_line_anchors_land_on_definitions`; it fails by construction.
  - Parse every ``{doc}:{lineno}  `{raw}:{S_i}…` is past end of file ({L} lines)`` line (`:816-820`; printed before the waiver branch, so waived rows are included).
  - The oracle binding of row i is the unique file with length L.
  - **Pass condition:** the set of (doc, lineno, i, file) equals the port's bound rows exactly, with file = T.path on the base side and T′.path on the after side. Unbound and unreadable rows print nothing.

  Count parity cannot see a port that binds an anchor to the wrong file with the class unchanged (M44). The oracle can.

**Step 5.** Path-gate re-paths (D17 R5) go into the same change.

**Fixtures (R2).** Each fixture is a before repo and an after repo, `D:/wt/_targets/refactor/selftest/anchors/<case>/{before,after}/crates/<crate>/src/…`, with docs at `…/<side>/docs/` under all four `GATED_DOCS` names (each with ≥ 1 mention and ≥ 1 anchor).
- The before side is split with `split.py`.
- `plan`, `apply`, `trace`, `twin` and `oracle` run on both sides.
- Each fixture must print its listed kinds.

| Case | Shape | Required trace / result |
|---|---|---|
| **FX-H3** (N14) | `## S` + `[core/x/](../crates/fx/src/core/x/)`; `### S.1`; a fence with `// :3`, then `// y.rs:N`, then `// nosuch.rs:K`. The split moves N and K into `y/c.rs` | `H≤2`, `M-dir`, `H3+`, `F-open` (seed none), `A-fence-unbound` (`:3`), `A-fence-frag/base-dir/0` (`y.rs`), `A-fence-frag-unresolved` (inherits `y.rs`). After: `y.rs` is replaced by `y/c.rs` → `base-dir/0`; the inheriting K row then binds `y/c.rs` = T′ (a digits edit) |
| **FX-BASE** (N15) | `## S`; prose `[crates/fx/src/a.rs](../crates/fx/src/a.rs):N`; a fence with fragment `b.rs`; both `src/b.rs` and `src/a/b.rs` exist. The split moves `a.rs:N` into `a/c.rs` | Before: `M-label`, `M-file`, `A-prose`, `A-fence-frag/base-parent/0` → `src/b.rs`. After the relink the naive binding is `src/a/b.rs`; S = `src/b.rs` → `base-parent/2` → `src/b.rs` = T′. Δmentions = 0 |
| **FX-DIR** (N16) | the directory mention followed by a bare `(N)`; a line carrying the gate's `IGNORE_MARKER` and an anchor; a mention of `crates/fx/src/blob.rs` (non-UTF-8 bytes) followed by `:3` | `M-dir`, `A-prose-unbound`, `IGN`, `A-unreadable`; all unchanged after the split |
| **FX-LABEL** (N17) | a run under `**File:** [crates/fx/src/r.rs](../crates/fx/src/r.rs):N`; rows 2–4 move to `r/c.rs` | `M-label` + `M-file`; rows 2–4 converted with `crates/` labels; Δ = +6; twin mentions = base + 6 |
| **FX-SEED** (N18, corrected) | the FX-LABEL run, then a fence containing `// :J` and then `// r.rs:K` (J and K stay in `r.rs`) | Before: `F-open` seed `r.rs`, `A-fence-inherit`, `A-fence-frag/sticky-self/0`. After: seed `r/c.rs` ≠ T′.path, so S = `r.rs` is inserted into `// :J`. `sticky-self` fails (`c.rs` ≠ `r.rs` component-wise) and `base` = `r/c.rs` (a file), giving **`A-fence-frag/base-parent/1`** for both rows. The K row needs no edit; only its route changes |
| **FX-STICKY** (N22) | `## S`; `[tests/t.rs](../tests/t.rs)`; a fence with `// crates/fx/src/q.rs:N`; N moves to `q/c.rs` | `M-other`, `F-open` (seed `tests/t.rs`), `A-fence-frag/sticky-parent/1` (from `tests/` up to the root). After: the shorter suffixes resolve nowhere, so S = `crates/fx/src/q/c.rs` → `sticky-parent/1`. **Δ = 0**, because fence lines add no mentions (O4) |
| **FX-DEAD** (N23) | `## S`; `[gone/g.rs](../gone/g.rs)` then `(5)`; a fence with `// crates/fx/src/q.rs:N` (same split) | `M-dead`, `A-prose-missing`, `F-open` (seed none, since `current` does not exist), `A-fence-frag/sticky-parent/1` (start = `gone/`) on both sides. dead_after = dead_base = 1 |
| **FX-PREC** (N24) | prose `[crates/fx/src/r.rs](../crates/fx/src/r.rs)`, then a later line `[core/x/](../crates/fx/src/core/x/)`; a fence with `// y.rs:N`; both `src/core/x/y.rs` and `src/y.rs` exist; no split of `y.rs` | `base-dir/0` → `core/x/y.rs`, although `current` = `r.rs`. The binding must be unchanged on both sides. This fixture is the M44 target |

**Kind coverage.** Every kind in the trace table is reached as follows:

| Kind(s) | Fixture(s) |
|---|---|
| `H≤2` | FX-H3, FX-BASE, FX-STICKY, FX-DEAD |
| `H3+` | FX-H3 |
| `IGN` | FX-DIR |
| `M-file` | FX-BASE, FX-LABEL, FX-PREC |
| `M-dir` | FX-H3, FX-DIR, FX-PREC |
| `M-other` | FX-STICKY |
| `M-dead` | FX-DEAD |
| `M-label` | FX-BASE, FX-LABEL |
| `A-prose` | FX-BASE, FX-LABEL |
| `A-prose-unbound` | FX-DIR |
| `A-prose-missing` | FX-DEAD |
| `F-open` (seed / none) | FX-SEED / FX-H3 |
| `sticky-self` | FX-SEED |
| `base-dir` | FX-H3, FX-PREC |
| `base-parent` | FX-BASE, FX-SEED |
| `sticky-parent` | FX-STICKY, FX-DEAD |
| `sticky-dir` | no fixture (exit 2) |
| `A-fence-frag-unresolved` | FX-H3 |
| `A-fence-inherit` | FX-SEED |
| `A-fence-unbound` | FX-H3 |
| `A-unreadable` | FX-DIR |

`self-test` fails if a kind other than `sticky-dir` is reached by no fixture.

---

## 8. Verification protocol (deliverable 3)

**Setup.**
- `ENV='PATH="$HOME/.cargo/bin:$PATH" RUSTUP_TOOLCHAIN=stable-x86_64-pc-windows-gnu CARGO_TARGET_DIR=D:/wt/_targets/refactor CARGO_BUILD_JOBS=6'`
- `CIF='--features boyko-ecs/profiling-analysis --exclude boyko_demo --exclude bench-bevy-vs-boyko'` (only together with `--workspace`)
- `GIT='git --no-optional-locks'`
- Receipts go under `D:/wt/_targets/refactor/receipts/<rev>/`.
- No `RUSTFLAGS`, everything under `timeout`, nothing timed.
- **Closure** (from `cargo metadata`): the crate, its reverse dependencies (normal and build transitively, dev one level), `boyko-engine`, and the host crate of every R4 walker.
  - Pilot closure: {aether-lang, aether, aether-tests, boyko-engine, boyko-log, boyko-app} ∪ R4.
- `HLEGS`/`FLEGS` are taken from the plan.

**B — Baseline (once per base revision)**

| Step | Command | Record / pass |
|---|---|---|
| B0 | `$GIT -C D:/wt/refactor rev-parse HEAD; $GIT -C D:/wt/refactor status --short` | `d552be05…`; status empty or tools only |
| B1 | `$ENV timeout 4h cargo check --workspace --all-targets $CIF` | warning set |
| B2 | `$ENV timeout 4h cargo clippy --workspace --all-targets $CIF -- -D warnings` | pass/fail |
| B3 | serial binaries (a test root whose `//!` header contains `--test-threads=1`, confirmed by reading it) | list (none in `aether*`) |
| B4 | `$ENV timeout 6h cargo test --workspace --all-targets --no-fail-fast $CIF -- --test-threads=6`; serial binaries run alone | per-target results |
| B4r | B4 with `--release`, before the first `debug_assertions` family and at milestones | per-target results |
| B5 | `$ENV timeout 1h cargo test -p boyko-engine --all-targets --no-fail-fast -- --test-threads=6 --nocapture` | per doc: mentions and dead; anchors 223/336/6/177 with 0 stale; decomposition; **over-waived count**; the root censuses |
| B6 | `python scripts/check_hotpath_exceptions.py`; `python scripts/check_doc_contracts.py` | stdout and exit code; the hot-path check is **red at base** (`entity_reservoir.rs:405`) |
| B7 | `$ENV cargo doc -p <crate> --no-deps`, with and without `--document-private-items` | warning sets |
| B8 | per host leg: `cargo test -p <crate> --lib -- --list` and `--doc -- --list` | pilot: 56 lib tests plus the doctest list |
| B9 | `$ENV cargo test -p aether-lang --lib -- --nocapture expansion_volume` | 5 lines |
| B10 | self-tests; `rsitems.py parity`; **`rsitems.py shapes`**; `census.py snapshot`; `observers --all`; build-script cfg census (0); `configs --verify`; `axioms --verify` | all pass; `shapes` prints the node shape per N19 case, and every F20 kind is classified |
| B11 | reader canary: the F7 readers; the R5 data files; R4 finds `profiling_artifact_roundtrip.rs:776-796` and `trybuild_corpus_compiler_witness.rs:127-131`; N8; walker-port parity | all found |
| B12 | re-export lint probe (A: unused `pub(crate) use`; B: `pub use` in a module that is not exported; C: glob `pub use` of `pub(super)` items) | A, B and C each fail under `unused_imports = "deny"`; otherwise revise D4.3 |
| B13 | privacy fall-through probe | recorded; sets M24's green column |
| B14 | leg feasibility sentinels (L-msvc; L-cfg(miri) check/test/bench; L-cfg(force_alloc_panic) release; L-noisa; after Q4, L-linux and L-config(loom)) | available/unavailable per leg; realised assignments recorded |
| **B16** | **Anchor tooling:** `anchors.py parity --root D:/wt/refactor --gate-log <B5 log>`; **`anchors.py oracle --root D:/wt/refactor --side before` over every gated doc**; FX-H3/BASE/DIR/LABEL/SEED/STICKY/DEAD/PREC with twin and oracle on both sides; M12, M26, M36–M39, M44 | parity exact; **oracle set equal to the port's bound rows on all 742 base anchors**; every fixture prints its kinds, routes and depths; twin counts = port counts; each mutation fires its listed ids |

**S — Per split (stop at the first red)**

| Step | Gate | Pass |
|---|---|---|
| S0a | re-derive the blocked set: `owner-dirty-files.txt`; `$GIT diff --name-only <merge-base>...<branch>` for each unmerged branch; `$GIT -C <lane> status --short`; the defining files of lane-owned symbols the family calls | no hit (split 2: `tls.rs:425` untouched by A6) |
| S0 | `configs --verify`; `readers`; `observers`; `legs`; `split.py check P…` | green; coverage satisfiable; no `pinned-move` |
| S0b | B15: each FLEGS leg compiles the unsplit crate clean | otherwise mark the leg unavailable and re-run S0 |
| S1 | `split.py apply P1 && split.py apply P2` | exit 0 |
| S2 | `census.py verify --base <rev> --plan P1 --plan P2` | exit 0 (checks 1, 1b, 2–16, 8b) |
| S3 | `anchors.py plan --map … --before <base materialisation> --after D:/wt/refactor`; `anchors.py apply --plan …`; `anchors.py drift … --out …drift.json` | no refusals; post-conditions (a)–(d) hold |
| S4 | `$ENV cargo check -p <crate> --all-targets HLEGS` | no new warnings in any host leg |
| S5 | `$ENV cargo clippy -p <crate> -p <closure…> --all-targets HLEGS -- -D warnings` | green |
| S6 | `$ENV cargo test -p <crate> --all-targets --no-fail-fast HLEGS -- --test-threads=6`, plus `--list` diffs for lib tests and doctests | same pass set; same leaf multiset and count per leg; mapped paths; doctests: same count, text hashes, mapped (file, line). **`running 0 tests` or a lower count is RED** |
| S7 | `$ENV cargo test -p <closure…> --all-targets --no-fail-fast -- --test-threads=6` (+ `--release` for `debug_assertions` families) | equals B4/B4r restricted to the closure |
| S8 | crate-specific gates | identical to baseline |
| S9 | `$ENV cargo check --workspace --all-targets $CIF`; if the closure is ≥ 50% of members, also workspace clippy and test | equals B1 (B2/B4) |
| **S10** | root package (B5), plus `anchors.py plan --after` re-run, plus **`anchors.py oracle --side after`** | per doc, on the real tree: anchors = base, stale = 0, decomposition as predicted, mentions = base + Δ, dead = base, over-waived count = base. Every row has T_after == T′ in the port. **Oracle set = the T′ bound rows.** All other root results equal B5 |
| S11 | the two scripts | stdout identical except the predicted diff |
| S12 | `cargo doc` ×2 | warning set ⊆ baseline |
| S13 | `census.py pubsurface --crate <dir> --rev <base>`, plus S9 and S12 | holds; never skipped |
| S4f | FLEGS on the split tree | warnings equal to S0b; predicted truth values realised |
| S14 | `$GIT status --short` equals the expected set; `$GIT diff --color-moved=zebra --color-moved-ws=allow-indentation-change --stat`; rustfmt advisory | nothing else touched |
| M | milestones (after the pilot, after split 2, then every 5 splits): workspace B4 + B4r | equal to baseline |

**Pilot-specific gates (S8):**
- (a) 54 token-exact unit tests;
- (b) 38 trybuild goldens byte-exact, and `$GIT status -- crates/aether_tests` empty;
- (c) the B9 lines;
- (d) `a7_dx` under clippy `-D warnings`;
- (e) `a0`…`a7`, `demo_arena` and `r2_chart_arbitration` in S7;
- (f) `cargo test -p boyko-macros --lib -- --test-threads=6 snake`;
- (g) `trybuild_corpus_compiler_witness` equal to baseline.

**Not applicable to the pilot:** GPU pins, foreign legs, readers, observers, check 8b, D18, invocation units. S3 and the S10 oracle are empty (0 gated anchors); B16 proves the machinery.

---

## 9. Pilot plan (deliverable 4): `crates/aether_lang/src/expand.rs` (3542 lines, CRLF)

**At base.**
- L1–16: the header.
- L18–1393: 48 top-level fns, 1 enum, 4 consts.
- L1395–3542: `#[cfg(test)] mod tests {…}` (54 tests, 4 helpers). `fn_count` = 107.
- Absent: `unsafe`, `macro_rules!`, file-top attributes, features, includes, readers, observers.
- **No item-level node outside the D4 unit and header kinds.** No `expression_statement` or bare `macro_invocation` at file scope or in the `mod tests` body (F12). Check 1b therefore covers the file with named items, trivia and the header only.
- The template token trees at L188-195 and L206-210 declare nothing (F17, R1).

**Use graph (D5).**
- The dispatcher uses every emitter.
- `sys_param_tokens` (L310) is used by `system_fn` (L297) and `handler_params` (L783). It stays in the parent together with its callees (`param_ty_and_mut`, `query_type`, `type_mentions_mut`, `stream_mentions_mut`, L312-367; the doc at L317-318 says "shared").
- `machine_registrations` (L794) is used only by `plugin_impl` (L462), so it moves to `plugin.rs`.
- `snake` stays in the parent.

**Result.**
- No sibling edges.
- 41 vacated names.
- New modules: `data`, `machine`, `material`, `plugin`, `scene`, `system`.
- `legs = ["host"]`.

**Stage 1 — `plans/aether_lang/expand.toml`** (hand-derived; `split.py` is authoritative)

| Destination (lines, stage 1 / 2) | Responsibility | Items (old range incl. trivia → new) | Widened | Header |
|---|---|---|---|---|
| **`expand.rs`** (285 / 285) | dispatcher; §7.3 recovery; shared layer (param-sugar table, generated-name spelling) | header 1–16 → 1–16; `expand` 18–29 → 32–43; `recovered` 31–71 → 45–85; `stub_item` 73–116 → 87–130; `expand_inner` 118–125 → 132–139; `expand_v1` 127–152 → 141–166; `scene_awaits_a_broken_material` 154–162 → 168–176; `sys_param_tokens` 306–315 → 178–187; `param_ty_and_mut` 317–333 → 189–205; `query_type` 335–355 → 207–227; `type_mentions_mut` 357–362 → 229–234; `stream_mentions_mut` 364–370 → 236–242; `snake` 801–839 → 244–282; inline `#[cfg(test)] mod tests {` 1395–1396 → `#[cfg(test)]` + `mod tests;` 284–285 (verbatim) | — | L18–23 `mod data; mod machine; mod material; mod plugin; mod scene; mod system;`; L25–30 `use data::{bundle, component, event, tag};` `use machine::machine_items;` `use material::material_fn;` `use plugin::plugin_impl;` `use scene::scene_fn;` `use system::system_fn;` (no cfg) |
| **`expand/data.rs`** (95 / 98) | §3.1/§3.2/§3.4 emitters | `component` 164–196 → 3–35; `tag` 198–211 → 37–50; `bundle` 213–225 → 52–64; `event` 227–256 → 66–95 | all 4 | `use super::*;` |
| **`expand/system.rs`** (49 / 49) | §3.3 `system` construct | `arity_allow` 258–289 → 3–34; `system_fn` 291–304 → 36–49 | `system_fn` | `use super::*;` (whole registry entry, D2) |
| **`expand/plugin.rs`** (287 / 290) | §3.3 plugin registration incl. machine registrations | `enum ResolvedOrder` 372–383 → 3–14; `bucket` 385–388 → 16–19; `plugin_impl` 390–491 → 21–122; `resolve_order` 493–538 → 124–169; `bucket_stmts` 540–637 → 171–268; `key_ident` 639–642 → 270–273; `machine_registrations` 787–799 → 275–287 (1 break) | `plugin_impl` | `use super::*;` |
| **`expand/machine.rs`** (144 / 147) | §3.5 lowering to `state_chart!` | `machine_items` 644–670 → 3–29; `state_tokens` 672–704 → 31–63; `interleaved_body` 706–739 → 65–98; `min_decl_index` 741–749 → 100–108; `transition_tokens` 751–775 → 110–134; `handler_params` 777–785 → 136–144 | `machine_items` | `use super::*;` |
| **`expand/material.rs`** (67 / 70) | §3.6 material builder | `material_fn` 841–882 → 3–44; `expr_or` 884–887 → 46–49; `rgba_array` 889–898 → 51–60; `rgb_array` 900–905 → 62–67 | `material_fn` | `use super::*;` |
| **`expand/scene.rs`** (489 / 492) | §3.7 scene spawn fn | `scene_fn` 907–1026 → 3–122; `collect_materials` 1028–1054 → 124–150; `unknown_symbol` 1056–1083 → 152–179; `material_local` 1085–1088 → 181–184; `SCENE_PARAM_*` 1090–1099 → 186–195; `scene_param` 1101–1105 → 197–201; `node_local` 1107–1110 → 203–206; `emit_node` 1112–1163 → 208–259; `spawn_call` 1165–1327 → 261–423; `at_tokens` 1329–1342 → 425–438; `tuple3` 1344–1355 → 440–451; `tuple3_or` 1357–1363 → 453–459; `scalar` 1365–1371 → 461–467; `scalar_or` 1373–1379 → 469–475; `missing_slot` 1381–1393 → 477–489 | `scene_fn` | `use super::*;` |
| **`expand/tests.rs`** (2145, stage 1) | old L1397–3541 de-indented by 4; L3366 (`\` continuation) keeps its bytes | — | — | its own `//!` + `use quote::quote;` (moved) |

**Parent gaps.** Kept: L163→177, L371→243, L840→283. Dropped: L305, L800, L1394. The gaps inside the parameter table are untouched.

**Intra-doc links.**
- L778 `[sys_param_tokens]` resolves through the glob to the kept item.
- L680 resolves through the glob.
- L493, L1094–1102 and L1365 resolve within the same child.
- L40 is absolute.

**Stage 2 — `plans/aether_lang/expand-tests.toml`** (only after Q8(c); old line ranges; 0 reorders; 13 breaks)

| Destination (lines, tests) | Subject | Items | Header |
|---|---|---|---|
| `expand/tests.rs` (543, 15) | shared helpers; block-level and whole-expander invariants | 1397–1401; `expands_to` 1403–1409 [widen, Q8(c)]; `fails_with` 1476–1484 [widen, Q8(c)]; 1549–1556; 1558–1579; `emits_in_order` 2059–2068 [widen, Q8(c)]; 3066–3114; 3116–3541 | unchanged |
| `expand/data/tests.rs` (224, 11) | component/tag/bundle/event | 1411–1440, 1442–1463, 1465–1474, 1486–1520, 1522–1547, 1581–1607, 1609–1622, 1624–1640, 2143–2158, 2271–2275, 2277–2284 | `use quote::quote;` `use crate::expand::tests::{expands_to, fails_with};` |
| `expand/plugin/tests.rs` (252, 7) | §3.3 | 1642–1810, 2209–2269, 2861–2876 | `use quote::quote;` `use crate::expand::tests::{emits_in_order, expands_to, fails_with};` |
| `expand/machine/tests.rs` (372, 7) | §3.5 | 1812–2057, 2070–2141, 2160–2207 | the same three helpers |
| `expand/material/tests.rs` (212, 6) | §3.6 | 2286–2493 | `{expands_to, fails_with}` |
| `expand/scene/tests.rs` (557, 8) | §3.7 | 2495–2859, 2878–3064 | `{expands_to, fails_with}` |

If Q8(c) is rejected, stage 2 is not applied, and `size-waivers.toml` gets:

```toml
{ file = "crates/aether_lang/src/expand/tests.rs", cap = 2145, reason = "test distribution needs pub(super) on 3 kept helpers; awaiting Q8(c)" }
```

**Totals.**
- **Widenings:** 9 production (`component`, `tag`, `bundle`, `event`, `system_fn`, `plugin_impl`, `machine_items`, `material_fn`, `scene_fn`) + 3 helpers = 12.
- **Tests:** 54 = 15 + 11 + 7 + 7 + 6 + 8.
- **Lines:** stage 1 = 3561 in 8 files; stage 2 = 3591 in 13 files; largest file 557.
- **Breaks:** 14.
- **No depth changes:** L3399 `super::snake` still lands in `expand`; L2499 `[super::scene_fn]` lands in `expand::scene` on the same item.
- **Generated cfg:** only the stage-2 `#[cfg(test)]`.
- **Empty or not triggered:** re-exports, pins, invocation units; checks 8b, 11, 15 and 16; D18; foreign legs.
- **Hot-path prediction:** unchanged; test lines stay test.
- **Pinned `--list`:** 56 = 15 `expand::tests::`, 11 `expand::data::tests::`, 7 `expand::plugin::tests::`, 7 `expand::machine::tests::`, 6 `expand::material::tests::`, 8 `expand::scene::tests::`, 2 `diag::tests::`.

**Drift note (D15).**
- Parent lines after L176 renumber: the parameter table goes from L306–370 to L178–242, and `snake` from L801–839 to L244–282.
- Implicit locations that move:
  - `expand.rs:1156` → `expand/scene.rs:252`;
  - the indexing panics in `resolve_order`/`bucket_stmts` (L504–629) → `expand/plugin.rs`;
  - `query_type`'s `fs[0]` (L350) → `expand.rs:222`.
- All of these are `display`; there are no explicit `file!`, `line!` or log sites.

**Anchors.**
- **Gated:** 0.
- **Ungated** (written to `expand.drift.json`, not edited):
  - `docs/aether-v2/DECISIONS.md:90`: `expand.rs:329-330` → `expand.rs:201-202`;
  - `docs/gaia/DECISIONS.md:100`: `:891-905` → `expand/material.rs:53-67`;
  - `docs/OPEN-QUESTIONS.md:620`: `:891-898` → `expand/material.rs:53-60`;
  - `docs/ru/OPEN-QUESTIONS.md:479`: frozen.
- **Book** (`book/src/aether/reference.md`, for doc-writer):
  - `:1035`, `:2510` (`arity_allow` → `system.rs`);
  - `:1061` (→ `expand::plugin::tests`);
  - `:2189` (`resolve_order` → `plugin.rs`);
  - `:2191` (`unknown_symbol` → `scene.rs`);
  - `:2282`, `:2293`, `:2304` (the test module is now distributed);
  - `:2576` ("the crate is flat");
  - `:2105`, `:2155`, `:2208`, `:2697` (counts and sizes);
  - still true: `:22`, `:2471`, `:2635`, `:2649`, `:741`;
  - `:2190` was already drift at base.
- **Code:** `a7_dx.rs:18`; `state_chart/mod.rs:535`. Mentions still true at family level: `a7_diagnostics.rs:71`, `gaia_storage_blindness.rs:65`.
- **Owner branch at merge:** 57 ledger rows (the `machine_registrations` rows → `plugin.rs`) and 264 citations.
- All four line ranges stay contiguous.

---

## 10. Mutations that must turn a gate red (deliverable 5)

**Legend.**
- **Live:** applied to the split pilot tree.
- **Fixture:** a materialised `.in` case, run through `census.py verify --before/--after`, `probe.py` or `anchors.py`.
- `mutate.py` asserts the exact set of failing ids and the cargo outcome, then reverts and re-verifies.

| ID | Kind | Mutation | Must be RED (exact ids) | Stays GREEN |
|---|---|---|---|---|
| **M1** | live | edit one word of a `//` comment in `bucket_stmts` | census {2} | every cargo gate |
| M2 | live | `"__aether_k_{}"` → `"__aether_key_{}"` | census {2}; S6 | — |
| M3 | live | `fn bucket` → `pub(super) fn bucket` | census {3} | cargo, tests |
| **M4** | live | delete `use material::material_fn;` | census {5}; E0425 | — |
| M4b | fixture | delete an exported `pub use child::{X}` | census {13 pubsurface}; dependent E0432 | — |
| M5 | live | de-indent old L3366 | census {2} (byte layer) | token hash, cargo |
| M6 | live (stage 2) and fixture | remove the test pair from `material.rs` | census {10}; S6 count 50 ≠ 56 | check, clippy |
| M7 | live | add `//! Split from expand.rs` | census {5} | the rest |
| M8 | live | add `pub use data::*;` | census {5, 6}; clippy | tests |
| M9 | fixture | a doc-only `super::x` link lands where `x` is unbound | census {7} | cargo |
| M10 | live | the plan omits `fn expr_or` | `split.py check` | — |
| M11 | live (stage 2) and fixture | drop `pub(super)` on `fails_with` | census {3}; E0603 | — |
| M12 | fixture (two-repo) | the map is applied, `anchors.py apply` is skipped | `plan --after` row mismatch; twin stale > 0; oracle mismatch | — |
| **M13** | fixture | a vacated `helper` falls through to `use crate::ext::*` | census {8} | probe check |
| **M14** | fixture | child `data` captures `data::f()` from `use crate::other::*` | census {8} | cargo |
| M15 | fixture | `matches!(x, LIMIT)` after `const LIMIT` moves unbound | census {8} | cargo |
| N1–N5 | fixture, PASS | quote, format, path-head, local-shadow, capture shapes | green | — |
| M16 | fixture + live canary | an unlisted `.join("solver").join("simd.rs")` absence reader; drop simd from B11 | `split.py check`; B11 | cargo |
| M17 | fixture | strip the computed cfg from a feature-gated re-export | census {14}; probe default E0432 | feature-on leg |
| M18 | fixture | cfg twins split across destinations | `split.py check` | — |
| M19 | fixture | D14: capped 1501-line file; uncapped 1872-line file | red on the first; the second only reported | — |
| M20 / N6 | fixture (`store.rs` shape) | strip the bind cfg for a use inside a gated method | census {14}; probe default E0432 | `--features x` |
| M21 / N7 | fixture | strip the cfg from a bind used only under `debug_assertions` | census {14}; probe `--release` deny | debug leg |
| M22 | fixture | a serializable component moved without a `[[keypin]]`; variant with a data file | {11}; variant {15} | cargo, in-process round trip |
| M23 | fixture | an out-of-family glob consumer switches to `other::helper` | census {8} | cargo |
| M24 | fixture | a private `len`/`v` shadowed by a `Deref` target | census {8b} | per B13 |
| M25 | fixture | a `.stderr` re-bless changes a `help:` line outside the impl sample | census {15} | trybuild after re-bless |
| M26 | fixture (two-repo) | `:N-M` split across two children | `anchors.py plan` `range-split` | twin (bounds only) |
| N8 | live, PASS | `readers` over a tree holding `plans/aether_lang/*.toml` | 0 findings | — |
| N9 / M27 | fixture (`schedule.rs` shape) | N9: `#[cfg(not(miri))] const K` moved with the correct plan. M27: the bind loses its cfg | M27: census {14}; probe L-cfg(miri) E0432 | host leg |
| N10 / M28 | fixture (`ffi.rs` shape) | N10: `#[cfg(windows)] pub mod os` moved with a correct `#[cfg(windows)] pub use c::os;`. M28: cfg stripped | M28: census {14}, {13} | host leg |
| N11 | fixture, PASS | `#[cfg(windows)] mod os {…}` extracted in place | green; E1 recorded | — |
| M29 | fixture | a forged E1 waiver on a real move | census {14} | — |
| N12 | fixture, PASS | bench-root split with `#[path = "root/c.rs"] mod c;` | green; one bench target | — |
| M30 | fixture | the same child as `mod c;` at `benches/c.rs` | census {5, 10}; probe extra target | — |
| M31 | fixture | `benches/root/main.rs` created | census {10} | — |
| **M32** | fixture | name `n` used only inside a `proptest!`-shaped tree with `#[cfg(feature = "x")]`; plan A moves `n`'s definition, plan B keeps it | A: `pinned-move n`. **B: `split.py check` and census green (file accepted)** | cargo |
| **N13** | fixture, PASS | the F17 template shapes moved to a child | green; no unit, name or refusal from template tokens | — |
| M33 | fixture | an O-L `line!()` site classed `key`, below a removed block, no ack | {11} | cargo, tests |
| M34 | fixture `.yml.in` | a job with `--cfg foo` and no row | `configs --verify` exit 1 | — |
| M35 | fixture | axiom row `unix → target_os = "linux"` | `axioms --verify` rejects | — |
| N14–N18, N22–N24 | fixture (two-repo), PASS | FX-H3, BASE, DIR, LABEL, SEED, STICKY, DEAD, PREC | the kinds, routes and depths from §7.3; T_after == T′; twin = port; oracle = port | — |
| **M36** | fixture (two-repo) | FX-LABEL with a waived `(N~)` row; `anchors.py` writes the sibling child's path for it | post-condition (a); **oracle** | plain twin (waived → bounds only) |
| **M37** | fixture (two-repo) | FX-SEED with fragment insertion disabled; the wrong file has a definition-shaped line at N′; unpaired | (a); **oracle** | plain twin |
| **M38** | fixture (two-repo) | FX-LABEL with Δ computed as +1 per insertion | (b); S10 mention comparison | the twin's own tests |
| **M39** | fixture (two-repo) | FX-BASE with fragment re-resolution disabled | (a) (binds `src/a/b.rs`); **oracle** | plain twin |
| **N19a–h** | fixture, PASS (CH60) | `a!(…);` (A), `b![…];` (A), `c!{…}` (B), `d!{…};` (A, or B + C per the parser) at file scope; the same four inside an inline `mod m { … }` (B + C for `;` forms, B for brace). Each invocation's macro is flat (a1) and defines `fn nX`; the plan moves it to a child | `shapes` prints the parsed shape; `split.py check`, `apply` and census green; each `nX` is in the vacated set; the `;` travels; the parent has no stray `;`; probe check green | — |
| **N20** | fixture, PASS (CH61) | `macro_rules! h { ($(#[$meta:meta])* $name:ident) => { $(#[$meta])* pub struct $name(u64); } }`; `h!(#[doc = "d"] VkA);` moved, exported | `VkA` defined by (a2); re-export emitted; pubsurface lists `VkA` on both sides | — |
| **N21** | fixture, PASS (CH61) | a repetition-matcher macro whose transcriber holds only `impl` blocks referencing `QF`; 3 file-scope invocations placed explicitly into a child; the parent keeps `trait QF` | (a3): no names, not pinned; check 8 resolves the transcriber's `QF` in the child through the glob | — |
| **M40** | fixture (tool mutation) | `rsitems` classifier skips `expression_statement` on N19a | census {1b}, `split.py apply` exit 2 (coverage) | — |
| **M41** | fixture (tool mutation) | `apply` excludes the shape-C `;` from the extent on N19 (the inline-mod case) | census {5}; probe `cargo check` "expected item" | — |
| **M42** | fixture | N20 with the parent re-export of `VkA` omitted | census {13 pubsurface}; dependent E0432 | — |
| **M43** | fixture (CH64) | the item that contains a cfg-bearing `proptest!`-shaped tree is moved; the definition of the name used inside it stays | `split.py check` `pinned-move <item>` | cargo (the glob serves the name in the host leg) |
| **M44** | fixture (two-repo), tool mutation (CH63) | the port's `resolve_fragment` start precedence is swapped to `sticky.or(base)` on FX-PREC | **oracle** (the port binds `src/y.rs`, the gate `core/x/y.rs`) | parity counts; decomposition (both files definition-shaped, unpaired) |

---

## 11. Concurrency model

- **Writers:** a single writer (one developer agent) in `D:/wt/refactor`; at most 6 cargo jobs; single-threaded tools.
- **Lane trees:** read only through `git --no-optional-locks … status`, which takes no index lock.
- **Owner's checkout:** read-only.
- **Scratch output:** probes, twins, oracle copies, fixtures, base materialisations and receipts all go under `D:/wt/_targets/refactor/`.
- **Foreign legs:** share the target directory and run last.
- No runtime code changes, so there is no data-race surface.

## 12. Integration

- **New:**
  - `tools/refactor/{rsitems,split,census,anchors,mutate,probe}.py`;
  - `tools/refactor/{configs,cfg-axioms,identity-observers,macro-names,size-waivers}.toml`;
  - `fixtures/**` (`.in` files only);
  - `plans/aether_lang/*`;
  - `crates/aether_lang/src/expand/*` (6 files, then 12).
- **Modified:** `crates/aether_lang/src/expand.rs` only.
- **Unchanged:** `lib.rs`, the gated docs, `.stderr` files, `HOT-PATH-EXCEPTIONS.md`, all `Cargo.toml` files, the scripts and the gate test (imported or copied, never edited), the workflows.
- **Later:** D14 (needs approval); `light.rs` key pins (Q9); linux and loom legs (Q4).
- **Follow-ups:** doc-writer (the book lines in §9), the owner (the ledger), graph regeneration.
- **Not touched:** `Arena`, `ComponentPool`, `UnitId`.

## 13. Implementation plan

1. **Tools**, in this order:
   - `rsitems.py`: the item-level table and shapes A/B/C; byte coverage; attribute attachment; the declaration-key rule; co-location pins; the exact predicate engine; token-tree rules; captures and interpolations; defined names (a1–a3)/(b) with repetition-aware re-parse; per-configuration tables; `#[path]`; `shapes`.
   - `split.py`.
   - `census.py`: checks 1, 1b, 2–16; `readers`, `observers`, `legs`, `configs`, `axioms`; `pubsurface`; the script import.
   - `anchors.py`: port with `--root`, `trace` (routes and depth), `plan`, `apply`, `twin`, `oracle`, `drift`.
   - `mutate.py`, `probe.py`.
2. **Tables and fixtures.**
   - `configs.toml` (`configs --verify`) and `cfg-axioms.toml` (`axioms --verify`).
   - `identity-observers.toml` (from `observers --all`) and `macro-names.toml` (seed rows).
   - Fixtures N1–N24 and M1–M44, with the FX cases in two-repo form and the kind-coverage table.
3. **Gates:** B10–B14, then B16. Stop and escalate if:
   - B12 disagrees with D4.3;
   - a parity check fails;
   - `shapes` disagrees with F20;
   - B14 shows a wrong assignment;
   - the twin or the oracle disagrees with the port.
4. **Baseline:** B0–B9.
5. **Plans:** `split.py list`, write `expand.toml` per §9, then `split.py check`.
6. **Stage 1:** S0a–S14.
7. **Stage 2** (only after Q8(c)): `expand-tests.toml`, check, apply, S0a–S14. If Q8(c) is rejected, add the waiver row and stop.
8. **Mutations:** each with its exact-id assertion, then revert and re-verify. M6 and M11 run live only after stage 2; their fixture forms always run.
9. **Hand-off (no commit).** The message lists: plan paths; widenings (9, +3 with stage 2); 0 reorders; breaks (1, +13); the test path summary; the blame and `--color-moved` hints; the drift file with the D15 note. Then run milestone M.
10. **After approval:** D14, then split 2.

## 14. Validation

**Self-tests:**
- one fixture for each of checks 1, 1b, 2–16 and 8b;
- one for each D15 class, derive override and O-L consumer;
- one for each reader class;
- one for each D16/D19 case: statement, field, arm, array element, literal field, `cfg_attr`, invocation site, the three token-tree rules and the co-location pin, twin tautology, `any(test…)`, implication refusal, anti-blindness, X1/X3, C-a/b/c, E1, E2;
- one for each D18 rule;
- one for each item-level node class and each invocation shape (N19a–h);
- one for each D4 definition rule (a1 in N19; a2 in N20; a3 in N21; b as criterion seed rows; pinned);
- **every anchor trace kind, route and depth listed in the §7.3 coverage table** (the self-test fails when a kind other than `sticky-dir` is unreached);
- EOL cases.

**Property tests (seeded):**
- random partitions → apply → verify passes;
- a single-byte mutation of a moved unit fails;
- random invocation shapes (A/B/C) inserted between items → coverage holds and apply keeps each `;` with its unit;
- a vacated name colliding with an out-of-family glob fails check 8 unless bound;
- random host and non-host cfgs on statements and methods: the census predicates equal a brute-force evaluator over all X1–X3-consistent assignments, and every available leg compiles clean;
- random move maps over the FX docs: T_after == T′ whenever `plan` reports no refusal, and the twin and the oracle agree with the port.

**Parity:** `rsitems` vs `modules.json`; `shapes` vs F20; `anchors` vs B5; the oracle vs the port (B16); B11–B14; the walker port; `configs`/`axioms --verify`.

**Benchmarks:** none (timings are forbidden). Hot-path files are tagged perf-unverified (R7).

**`debug_assert!`:** N/A.

## 15. Risks and unblock order (deliverable 6)

| # | Risk | Mitigation |
|---|---|---|
| R1 | blame | no rename; `git blame -C`; no ignore-revs |
| R2 | open branches | nothing touches `aether_lang`; rebase re-runs the plan; S0a before every split |
| R3 | doc links | all resolve; no import exists only to serve a link |
| R4 | ungated drift | recorded |
| R5 | gated docs are owner-dirty | zero-anchor files go first; at merge, take the owner's text and re-run `plan/apply` |
| R6 | test identity | gated per leg |
| R7 | code layout under fat LTO | symbols covered by R6 and D15 |
| R8 | pre-existing reds | every gate compares against baseline |
| R9 | CRLF | handled |
| R10 | vacuous tooling | self-tests, exact-id mutations, parities, B11–B16, per-leg counts, twins, the oracle, kind coverage |
| R11 | rustc glob-use behaviour | proven in-tree |
| R12 | cold, release and foreign builds | structural runs only; foreign legs last |
| R13 | fixtures seen by walkers | `.in` files only; no `Cargo.toml` or `src/` under `tools/refactor/`; twins and oracle copies under `D:/wt/_targets` |
| R14 | host-uncompilable configurations | exact predicates plus coverage; E1/E2 only; linux and loom need Q4. Among free files only moving `os` out of `ffi.rs` is affected. Blocked files that need Q4: `runner.rs`, `component_pool.rs`, `archetype.rs`, `scope.rs`, `block.rs`, `device.rs`, `resources.rs`, `gpu_column.rs`. `schedule.rs` needs only L-cfg(miri) |
| R15 | toolchain or parser drift | re-run B12–B14, `axioms --verify` and `rsitems.py shapes` |
| R16 | path readers | D17; `simd.rs`, `systems.rs` and `colored.rs` need prerequisites |
| R17 | observer table lags the code | `check` refuses unrowed sites |
| R18 | diagnostic-text ruling | Q7 |
| R19 | CI matrix drift | `configs --verify` (M34) |
| R20 | partial foreign legs | dependents are covered symbolically; CI's Miri and alloc-panic jobs remain the full check |
| R21 | R1's reading of item-position invocations | stated at the top; Q10 |
| R22 | an anchor edit the tool cannot make | `fence-unrepairable`/`range-split` refuse at plan level |
| **R23** | a parser node shape not foreseen | the item-level table exits 2 on any unlisted kind; byte coverage (1b); `shapes` in B10 |
| **R24** | the oracle materialisation is large | text copies only for bound files; empty placeholders for everything else; on D:, never timed |

**Unblock order.**
1. **Pilot:** `aether_lang/src/expand.rs`. Stage 1 now; stage 2 after Q8(c).
2. **Split 2:** `boyko_ecs/src/ecs/core/profiling/store.rs`.
   - Profile: 1595 lines; free; 0 anchors; no readers; no host, miri or loom atoms; no item-position invocations.
   - Exercises: D16 with `profiling-analysis` (2 host legs); D4.3(a) (the `profiling/mod.rs:109-116` re-exports stay); `unsafe` accounting; satellites; 8b; `display` observers.
   - `impl Profiler` stays whole unless Q8(a).
   - Precondition: A6 does not touch `tls.rs`.
3. **Free, CPU, zero gated anchors, not target roots:** `aether_lang/src/parse.rs`, `asset/assets.rs` (release leg), `profiling/tests.rs`, `commands/command_queue.rs`, `sdf_math/brick.rs` + `brick/tests.rs`.
   - **3b. Target roots, after Q8(e):**
     - `shaderdsl/src/bin/emit_particles.rs` (`--features emit`);
     - `boyko_ecs/benches/cull_diagnostic.rs` (criterion shape-A units stay in the root; `bench-alloc` leg);
     - the §1.2 test and bench files (GPU leg for the two harness tests).
4. **Free, with gated anchors,** after the owner's doc commits (Q5); anchors handled by §7.3 with the oracle:
   - `query/filter.rs` (15 anchors; 72 shape-A invocations under (a3); 6 macro definitions stay above them);
   - `query/state.rs` (8 anchors, 2 waived; release leg);
   - `query/iter.rs`;
   - `component_registry/mod.rs` (30; `:132` is E2);
   - `ecs_master.rs` (11; presence reader; `SYSTEMS.md:1019`);
   - `render_path_config.rs`.
5. **Free, GPU leg** (owner device window): `rhi_vulkan` and `render` files.
   - `compute.rs`: `embed_spirv!` via (a1).
   - `ffi.rs`: 23 handles via (a2), N20; `os` stays in place unless Q4(a).
   - `light.rs`: needs Q9.
6. **Blocked,** in census §5.4 order, each subject to R14:
   - A4/A5 prerequisites;
   - the scope-cell amendment for `emit/mod.rs` and `schedule_builder.rs`;
   - owner-dirty files last.

## 16. Open questions

| # | For | Question |
|---|---|---|
| Q1 | owner | D14: should new oversized files the campaign did not produce also fail? Default: report only. |
| Q2 | owner | Re-path the owner-branch ledger at merge, or leave it as drift? |
| Q3 | owner | Install `cargo public-api`? |
| Q4 | orchestrator/owner | (a) Install the linux-gnu std into the pinned toolchain? (b) Allow the Cargo `--config` rustflags append form? (c) Confirm that crate-local `cargo rustc -- --cfg miri\|force_alloc_panic` legs are within the standing rule. |
| Q5 | orchestrator | Gated-anchor files: wait for the owner's commits, or merge under R5? |
| Q6 | owner | Teach the hot-path script about the module tree, or keep the scope-cell record? |
| Q7 | owner | Confirm D15's ruling, including implicit panic locations. |
| Q8 | orchestrator | Unlisted text changes: (a) a repeated inherent-`impl` header; (b) `pub(super)` for a sibling-only edge; **(c) `pub(super)` on 3 kept test helpers (pilot stage 2)**; (d) depth-relative visibility rewrites; (e) D18 `#[path]` lines. |
| Q9 | owner | Key-pin prerequisites for serializable components (`light.rs` first), or keep them in the parent? |
| **Q10** | orchestrator | R1 as applied (top section, R21), on the corrected definition. An item-position invocation is a real item in any of three tree-sitter shapes (A: `expression_statement(macro_invocation)`; B: bare `macro_invocation`; C: B plus its `empty_statement` `;`), with its `;` inside the unit. Its names come only from the macro's definition (D4 a1–a3/b); otherwise it is pinned. Tokens inside any token tree declare nothing and never cause a refusal. This asks to confirm scope, not to re-open R1. It covers `compute.rs` (104, brace form), `ffi.rs` (23, paren + `;`), `filter.rs` (72, paren + `;`) and the criterion roots. |

## 17. Readiness checklist

- **Goal and metrics:** done.
- **Decisions:** D1–D19, each with its reason, alternatives and trade-off.
- **Schemas:** done (`repr` is N/A).
- **CLI:** minimal.
- **Concurrency:** single writer.
- **Edge cases covered:**
  - CRLF, BOM, tabs; literals; twins; satellites; co-location pins;
  - every item-level node kind and all three invocation shapes, with `;` extents;
  - matcher forms (flat, trailing name, impl-only); template token trees;
  - cfg on every node kind and inside token trees; host atoms in both polarities; non-gating rows; release-only code;
  - depth-relative visibility; privacy regions; opaque globs; captures;
  - readers; identity observers; `.stderr`; doctests; split ranges; target roots;
  - every anchor branch: heading levels, fence seeds, all resolution routes and depths, directory, dead and non-`crates` mentions, labelled links, waived rebinding, fenced `(N)`.
- **Generation checks:** done.
- **Drop order:** N/A.
- **`unsafe`:** accounting plus L2.
- **Integration:** done.
- **Tests:** unit, property, parity, canary, twin, oracle, exact-id mutations.
- **Benchmarks:** N/A (timings are forbidden).
- **`debug_assert!`:** N/A.

**Key paths:**
- `D:/wt/refactor/crates/aether_lang/src/expand.rs` (L188-210, 287-370, 462, 740-839, 1150-1160)
- `D:/wt/refactor/tests/internal_docs_anchors.rs` (L224-262, 406-494, 676-729, 780-884, 886-1060, 1299-1349)
- `D:/wt/refactor/crates/boyko_rhi_vulkan/src/ffi.rs` (L36-37, 266-317, 319-418)
- `D:/wt/refactor/crates/boyko_rhi_vulkan/src/compute.rs` (L84-107)
- `D:/wt/refactor/crates/boyko_ecs/src/ecs/core/iters/query/filter.rs` (L1547-2612, 1747ff, 2079ff)
- `D:/wt/refactor/crates/boyko_ecs/benches/cull_diagnostic.rs` (L3308, 3316)
- `D:/wt/refactor/crates/boyko_macros/src/component.rs:369`
- `D:/wt/refactor/docs/SYSTEMS.md:1019`
- `D:/wt/refactor/crates/boyko_ecs/src/ecs/core/profiling/store.rs`
- `D:/wt/refactor/.github/workflows/ci.yml`
- `D:/wt/refactor/.github/workflows/docs.yml`
- `D:/wt/refactor/.cargo/config.toml`
- `D:/wt/refactor/Cargo.toml`
- `D:/wt/refactor/scripts/check_hotpath_exceptions.py`
- `D:/wt/refactor/crates/boyko_log/tests/walker/mod.rs`
- `D:/wt/_graph/tools/extract.py`
- `D:/wt/_graph/extract/modules.json`

Sources:
- [tree-sitter-rust grammar.js (v0.24.0)](https://raw.githubusercontent.com/tree-sitter/tree-sitter-rust/v0.24.0/grammar.js)
- [tree-sitter-rust corpus macros.txt (v0.24.0)](https://raw.githubusercontent.com/tree-sitter/tree-sitter-rust/v0.24.0/test/corpus/macros.txt)
- [Rust Reference — Items (MacroInvocationSemi)](https://doc.rust-lang.org/reference/items.html)
- [Rust Reference — Macros by example (matchers, repetitions, textual scope)](https://doc.rust-lang.org/reference/macros-by-example.html)
- [Rust Reference — Modules (mod-rs files, `#[path]`)](https://doc.rust-lang.org/reference/items/modules.html)
- [Rust Reference — Conditional compilation](https://doc.rust-lang.org/reference/conditional-compilation.html)
- [rustc_resolve check_unused.rs (`check_use_tree`)](https://doc.rust-lang.org/nightly/nightly-rustc/src/rustc_resolve/check_unused.rs.html)
- [cargo rustc](https://doc.rust-lang.org/cargo/commands/cargo-rustc.html)
- [Cargo configuration](https://doc.rust-lang.org/cargo/reference/config.html)
- [Cargo targets — auto-discovery](https://doc.rust-lang.org/cargo/reference/cargo-targets.html)
- [git-status `--no-optional-locks`](https://git-scm.com/docs/git-status)
- [git-blame](https://git-scm.com/docs/git-blame), [git-diff `--color-moved`](https://git-scm.com/docs/git-diff)

## Orchestrator rulings applied during the design loop

- After pass 4 (R1-R3): macro token trees declare nothing; anchor fixtures live in two mirrored temporary repositories; a `crates/` link label is a mention in its own right.
- After pass 6 (W1-W2): an attribute-repetition-then-`$name:ident` macro invocation (`embed_spirv!`) declares `$name` and carries its passed `cfg`s; an item containing tree-sitter ERROR/MISSING nodes moves only as verbatim bytes and is compared by hash.
