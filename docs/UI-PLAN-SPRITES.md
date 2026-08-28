# UI-PLAN-SPRITES — sprites, atlas and the draw path

**Campaign:** advanced UI/GUI for `boyko_ui` · **Branch:** `feat/ui-advanced` (worktree `D:/wt/ui`)
**Date:** 2026-08-21 · **Status:** plan, pre-implementation, revision 1
**Authority:** [`docs/UI-ADVANCED-ARCHITECTURE.md`](UI-ADVANCED-ARCHITECTURE.md) (revision 2) — every
`D<n>` below is that document's decision, unchanged unless this plan says so at the point of change.
**Evidence:** [`docs/UI-ADVANCED-RESEARCH-SPRITES.md`](UI-ADVANCED-RESEARCH-SPRITES.md).
**Siblings:** [`docs/UI-PLAN-ANIMATION.md`](UI-PLAN-ANIMATION.md) ·
[`docs/UI-PLAN-INTERACTION.md`](UI-PLAN-INTERACTION.md) · [`docs/UI-PLAN-AETHER.md`](UI-PLAN-AETHER.md).

This is the ladder a developer walks. It is not a design document: where a decision already exists in
the architecture it is cited, not restated. Where this plan **adds** a decision — because building the
thing surfaced something reading it could not — the decision is numbered `S-D<n>`, carries a reason,
and names what was rejected.

---

## 0 · What this plan owns, and what it does not

The architecture's §11 sequences eight rungs. This plan owns four of them and states its dependency
on the other four rather than duplicating them.

| §11 rung | Owner | This plan's relation |
|---|---|---|
| 1 · D7 `.ui` registration table | ~~**`UI-PLAN-AETHER.md`**~~ **NOBODY — S-D20 (7)** | **Dependency, now DISCHARGED: S6 LANDED 2026-08-26 on the fallback and D7 blocks nothing here.** Blocked exactly ONE rung here (S6), and S6 carried a fallback so the sprite ladder was never blocked behind D7's risk **R4**. *(owner struck 2026-08-26 at the S6 pre-build audit: `UI-PLAN-AETHER.md:73` files D7 as its own INBOUND dependency — "D7 does not gate any rung here" — and lands no registration table in U0–U8; [`UI-PLAN-ANIMATION.md` §8](UI-PLAN-ANIMATION.md#8--dependencies-stated-explicitly) *(pre-split `:3105`)* points back at THIS file, the option this section Rejected *(read at its target 2026-08-28 by the tenth pass — the D7 row, whose owner cell is verbatim "`docs/UI-PLAN-SPRITES.md` (rung 1)". It read `:846`, which no longer resolves — blank, under `## 3 · The rung ladder` — and then `:2530`, **read on the part-10 tree as prose about an unrelated axis, and carried by four citing sites at once**. ⚠️ **Part 11 re-read `:2530` and that description had itself rotted** — on the tree part 11 inherited it was *"*N+1* renders a strictly-moved value…"*, with part 10's quote 200 lines below; **the "four citing sites" count was also wrong — enumerated by grep, this description alone has EIGHT carriers**, because two documents carry it twice)*; `UI-PLAN-INTERACTION.md:501-504` names no owner. D7 exists only as `UI-ADVANCED-ARCHITECTURE.md` §11 item 1. **The fallback is not a contingency — it is the path.** SCOPE call filed in `docs/OPEN-QUESTIONS.md`.)* |
| 2 · D31 + D6 + D32 — seam, gate, observer | **this plan (S0)** | The gather feeds the pack; the pack is the draw path. Animation and interaction both **extend** what S0 spells — see §7. |
| 3 · D30 — the eDSL migration | **this plan (S1)** | |
| 4 · D1 — the 80 B instance | **this plan (S2)** | Both feature halves depend on it; neither may widen it twice. |
| 5 · Sprites — D2, D3, D8a–e, D4 | **this plan (S3–S5)** | |
| 6 · Animation — D9–D15 | `UI-PLAN-ANIMATION.md` | **Dependency at S5** (the flipbook clock) and **consumer of S0** (`UiVisual` joins the pack-input spelling). |
| 7 · Interactivity — D16–D24 | `UI-PLAN-INTERACTION.md` | **Consumer of S0** (the gather's DFS is the hit-test's traversal). |
| 8 · Aether `ui` | `UI-PLAN-AETHER.md` | **Consumer of S3–S5** (it can only name components that exist). |

**S-D1 — the boundary is drawn at the pack-input seam, not at the crate edge.** Everything that
decides *what the GPU sees* is here: the gather, the generation gate, the instance record, the shader,
the pipeline's descriptor sets, the two recorders, and the five sprite components. Everything that
decides *what writes those components* belongs to a sibling.

**Reason.** The alternative boundary — "sprites = the five sprite components, the draw path is
shared" — leaves the draw path unowned, and the draw path is where every one of this campaign's
lockstep failures lives (**R1**). One plan must be answerable for the 80 B record and both recorders.

**Rejected:** (a) *sprites owns only §11 rung 5*; the widening (rung 4) and the eDSL migration (rung 3)
then have no owner, and they exist **because** of sprites. (b) *sprites also owns D7*; D7 is a `.ui`
authoring-surface refactor whose consumers are all four subsystems, and putting it behind a feature
plan makes the feature plan's schedule the campaign's schedule.

---

## 1 · Verified in-tree facts

Read in this worktree, not assumed. Every rung below is built on these; a rung that contradicts one is
wrong.

| Fact | Where |
|---|---|
| `UiInstance` is `#[repr(C, align(16))]`, **64 B**, with **9** per-field `offset_of!` const-asserts | `boyko_render/src/ui/instance.rs:35,69,92-99` |
| `corner_radius` is REINTERPRETED as the glyph UV under `FLAG_TEXT`; the source names the exit condition verbatim — a rect needing both a radius and a UV "retires this alias and widens `UiInstance` to 80 B" | `instance.rs:44-56` |
| Three flag bits are used (`BORDER_ANY`, `CLIP_PRESENT`, `TEXT`); bits 3..31 are free | `instance.rs:73,77,84` |
| `PackInput` has **no** texture, **no** tint, **no** slot field; `text_uv: Option<[f32;4]>` is the only UV | `ui/pack.rs:21-46` |
| `UiRenderScratch.last_seen_generation` is a **single `u64`** and is **read by nobody**; `UiRenderGeneration::bump` has no production caller; `pack_sort_upload` contains no compare and repacks unconditionally | `pack.rs:141-199`, `upload.rs:153-180` |
| The seam order today is `gather_nodes(world, node_buf)` **then** `pack_sort_upload` — so a gate inside the pack still pays the whole world read | `upload.rs:255-274` |
| The fused seam predecessor (`host_upload_frame_from_world`, **DELETED 2026-08-21**) had **zero callers** outside its own doc comments — and the CAUSE, established after this table was first written: its parameter list demanded a live `WorldView` AND a `&mut RhiContext` at one call site, which the token's M1 discipline forbids by borrowck. All three call routes died in the compiler — in-schedule: **E0502** (view = `&self` of the token, `nonsend_resource_mut` = `&mut self`); host owning adapter: **E0277** (`System: Send + Sync + 'static` vs `RhiContext`'s raw pointers); host borrowing adapter: **E0521/E0505** (the `'static` bound) — and the shapes are exhaustive because `WorldView` has exactly ONE constructor (`DispatcherToken::world`) and `DispatcherToken::new` is `pub(crate)` with two mint sites (scheduler dispatch, `run_system_once`), both inside a `Send + Sync + 'static` system. Superseded by S0's **two-phase seam** (Phase 1 gather / Phase 2 upload, sequenced in one `run_dispatcher`) | `docs/OPEN-QUESTIONS.md` entry 2026-08-21 (RESOLVED); `upload.rs` |
| The UI pipeline is built through the **one-set** `create_graphics_pipeline`; set 0 has three bindings (StorageBuffer @0 VERTEX\|FRAGMENT, CombinedImageSampler @1 FRAGMENT, UniformBuffer @2 FRAGMENT) | `ui/resources.rs:188-243` |
| `create_graphics_pipeline_bindless(desc, set1_layout)` exists and is used by the textured gbuffer, Forward and particle paths | `boyko_rhi_vulkan/src/rhi_impl/device.rs:2112` |
| `BINDLESS_TEXTURE_CAPACITY = 4096`; the allocator issues `1..capacity`, so the maximum live slot is **4095** | `boyko_rhi_vulkan/src/bindless.rs:72`, `boyko_render/src/bindless.rs:80-93` |
| **The bindless set's sampler is ONE IMMUTABLE sampler: trilinear (LINEAR mag/min/mip), 16× anisotropic, `REPEAT` on all three axes, `maxLod = 1000`** | `boyko_rhi_vulkan/src/bindless.rs:139-172` |
| `BindlessTextureTable` is owned by `boyko_app::runner`, inserted as a NonSend resource. `RhiContext` does **not** own it and `ui_setup` never sees it | `boyko_app/src/runner.rs:212`, `boyko_render/src/gpu_column.rs:163-176` |
| **`boyko_rhi`'s generic encoder has NO set-index bind verb.** `bind_descriptor_set` binds at **set 0** and says so | `boyko_rhi/src/encoder.rs:174-186` |
| `DescriptorKind` has no `Sampler` variant; `BindGroupEntry` has no sampler-only variant | `boyko_rhi/src/enums.rs:717-745`, `boyko_rhi/src/device.rs:352-420` |
| There are **two** UI recorders: the `RhiApi`-generic `record_ui_rects` (offscreen golden) and the concrete inline recording in `present_blit.rs` (on-screen). They are separate code | `ui/draw.rs:41`, `present/passes/present_blit.rs:201-329` |
| `ui_setup` requires `&BakedFont` **unconditionally** — a sprite-only UI cannot boot | `gpu_column.rs:269-292` |
| The UI's own set-0 binding-1 sampler is bilinear + **ClampToEdge** + no-mip | `ui/resources.rs:494-496` |
| `ui_rect.{vs,fs}.hlsl` have **no** `// === GENERATED ===` sentinels, no eDSL leaf, no manifest row, no `*_edsl_sync` / `*_spv_sync` test. The only pin is `SpirvBlob<2368>` / `SpirvBlob<7060>` — a byte **length** | `ui/mod.rs:122,129`; `docs/SHADER-VARIANT-MANIFEST.md` |
| **`goldens/PINS.toml` contains ZERO UI rows.** All 32 pins are scene screenshots | `goldens/PINS.toml` |
| The four UI GPU goldens assert **individual texels**, not images, and skip gracefully on a device-less host | `ui_rect_gpu_golden.rs:19-39` |
| `ui_hud_screenshot.rs` is `#[ignore]`d **8** times | `boyko_render/tests/ui_hud_screenshot.rs` |
| `boyko-ui` is a **dev**-dependency of `boyko_render`, annotated TEST-ONLY; `boyko_app` has no `boyko-ui` edge at all | `boyko_render/Cargo.toml:106-118`, `boyko_app/Cargo.toml` |
| PNG assets exist in-tree (`boyko_app/assets/pbr_fixtures/…`) but `boyko_image` is a **decoder only** — there is no encoder | `boyko_render/src/loaders/png_texture.rs`; crate-wide grep |
| `create_solid_color_texture` builds a texture from raw bytes with no asset file | `boyko_render/src/bindless.rs:366` |

Two of these were not in the architecture or the research and change decisions below: the **immutable
anisotropic REPEAT sampler** (S-D4) and the **absent set-index bind verb** (S-D3).

---

## 2 · Invariants this ladder may not break

1. **One pipeline, one `draw(6, N, 0, 0)`, one global z-sort, clip in the instance record.** No batch
   list, no per-texture rebind, no second pipeline. This is the crate's single best property and
   sprites are not permitted to spend it (D2, research §6).
2. **`ComputedRect` keeps its single-writer invariant.** The pack *reads*; it never writes geometry
   (D5, `widgets.rs:6-9`).
3. **Capability is component presence.** No sprite flag on a general component; absence is the
   structural skip.
4. **No side store.** The sheet table is a `Resource`-owned dense column; the cursor is a dense
   component. No `Vec`/`HashMap` keyed by element (Principle 0).
5. **Zero steady-state allocation in the pack.** `clear()` + `extend` into the reused scratch; the
   nine-slice expansion writes into that same scratch and must not change this (`ui_no_realloc.rs`).
6. **Every `unsafe` carries a `// SAFETY:` comment with concrete invariants.**
7. **Shaders are eDSL-authored and re-spliced between sentinels from S1 onward.** Before S1 the UI
   shaders are outside that rule; after S1 they are inside it and a hand edit is a red test.

---

## The parts of this plan

**A plan is always split across several files.** This file is the index: it holds §0–§2, the §4
ladder gate, and §5–§9. The decisions and the rungs live in the files below, moved on 2026-08-28 —
**verbatim apart from 9 lines**, and those 9 are one class: a citation of the SIBLING animation plan,
re-pointed at the part file that now holds its target and rewritten as an anchor link. *(The
animation plan's own figure is **8**, stated there. Per plan rather than as one total, because a
single number hides which file carries what.)* ⚠️ **9 + 8 = 17, and the gate-id note under "Every ID,
and the anchor that resolves it" below also says 17 — they are NOT the same 17.** This one counts
changed LINES inside the two plan documents; that one counts FILES under `crates/**` whose header the
split re-pointed. Nothing connects them but the digits.

**What the integrity check prints, before anyone runs it.** Reassembling this plan and diffing it
against the pre-split blob is the right check, and it does **not** come out empty — a reader who
inherits "no prose changed" and then sees nine differences concludes the split corrupted content:

```sh
# 2a10f3a4 is the split commit's parent — the last tree on which this plan was one file.
git show 2a10f3a4:docs/UI-PLAN-SPRITES.md | sort > /tmp/pre.s
cat docs/UI-PLAN-SPRITES.md docs/UI-PLAN-SPRITES-DECISIONS.md docs/UI-PLAN-SPRITES-S0-S2.md \
    docs/UI-PLAN-SPRITES-S3.md docs/UI-PLAN-SPRITES-S4.md docs/UI-PLAN-SPRITES-S5.md \
    docs/UI-PLAN-SPRITES-S6-S7.md | sort > /tmp/post.s
comm -23 /tmp/pre.s /tmp/post.s | wc -l                     # 9 — lines the split did not carry through
comm -23 /tmp/pre.s /tmp/post.s | grep -c UI-PLAN-ANIMATION # 9 — every one of them cites the sibling
```

Both sides are **sorted** on purpose: the split reorders whole sections, so an ordered `diff` reports
the entire file and proves nothing. The reverse direction (`comm -13`) is **not** a clean count of
anything — it mixes those 9 rewritten counterparts with this index section, which is prose the split
added rather than moved.

| file | holds |
|---|---|
| [UI-PLAN-SPRITES-DECISIONS.md](UI-PLAN-SPRITES-DECISIONS.md) | §3 decisions `S-D2`–`S-D21` (`S-D1` stays in §0 above) |
| [UI-PLAN-SPRITES-S0-S2.md](UI-PLAN-SPRITES-S0-S2.md) | §4 rungs `S0`, `S1`, `S2` |
| [UI-PLAN-SPRITES-S3.md](UI-PLAN-SPRITES-S3.md) | §4 rung `S3` and `S3 · LANDED` |
| [UI-PLAN-SPRITES-S4.md](UI-PLAN-SPRITES-S4.md) | §4 rung `S4` and `S4 · LANDED` |
| [UI-PLAN-SPRITES-S5.md](UI-PLAN-SPRITES-S5.md) | §4 rung `S5` and `S5 · LANDED` |
| [UI-PLAN-SPRITES-S6-S7.md](UI-PLAN-SPRITES-S6-S7.md) | §4 rung `S6`, `S6 · LANDED`, and `S7` |

Still in this file: §0 scope · §1 verified facts · §2 invariants · §4's unconditional gate and disk
discipline · §5 measurement · §6 what this exposes to siblings · §7 risks · §8 open questions · §9
sources.

**Every ID, and the anchor that resolves it.** This campaign names decisions and rungs by bare ID —
`S-D16`, `S4` — in running prose, and before the split those names resolved because everything was one
file. They are **not** rewritten as links at each of their ~880 mention sites, and that is the
decision, not an omission: a generated heading anchor encodes the *whole heading text*, so re-wording
one heading would break every link into it — the same failure the line numbers had, one level up. The
table below is the single place a heading's wording is written down twice. Re-word a heading and
repair **one row**, not the mention sites.

| ID | resolves to |
|---|---|
| `S-D1` | this file, [§0](#0--what-this-plan-owns-and-what-it-does-not) — it was never in §3 |
| `S-D2` | [DECISIONS](UI-PLAN-SPRITES-DECISIONS.md#s-d2--the-flags-bit-budget-is-fixed-here-once-with-the-assert-beside-it) |
| `S-D3` | [DECISIONS](UI-PLAN-SPRITES-DECISIONS.md#s-d3--the-set-1-bind-needs-a-generic-verb-it-is-added-to-boyko_rhi-not-routed-around) |
| `S-D4` | [DECISIONS](UI-PLAN-SPRITES-DECISIONS.md#s-d4--the-ui-owns-its-sprite-sampler-the-shared-bindless-sampler-is-not-the-uis-to-choose) |
| `S-D5` | [DECISIONS](UI-PLAN-SPRITES-DECISIONS.md#s-d5--the-first-sprite-texture-is-procedural-not-a-checked-in-png) |
| `S-D6` | [DECISIONS](UI-PLAN-SPRITES-DECISIONS.md#s-d6--the-ui-gets-image-pins-because-it-has-none-and-s2-is-the-change-that-needs-them) |
| `S-D7` | [DECISIONS](UI-PLAN-SPRITES-DECISIONS.md#s-d7--tile-nine-slice-requires-a-whole-texture-sprite-with-a-sheet-it-is-a-hard-error-retired-2026-08-21--the-hazard-was-an-artifact-of-a-mechanism-that-will-never-ship-see-s-d11) |
| `S-D8` | [DECISIONS](UI-PLAN-SPRITES-DECISIONS.md#s-d8--the-default-off-ladder-stated-rung-by-rung) |
| `S-D9` | [DECISIONS](UI-PLAN-SPRITES-DECISIONS.md#s-d9--the-two-recorders-are-gated-separately-on-purpose) |
| `S-D10` | [DECISIONS](UI-PLAN-SPRITES-DECISIONS.md#s-d10--the-edsl-generator-owns-the-whole-hlsl-for-both-ui-stages-and-spv-byte-identity-is-s1s-end-condition) |
| `S-D11` | [DECISIONS](UI-PLAN-SPRITES-DECISIONS.md#s-d11--tiling-is-frac-inside-the-sub-rect-it-belongs-to-s5-and-the-nine-sub-quads-are-added-to-the-background-rect-not-substituted-for-it) |
| `S-D12` | [DECISIONS](UI-PLAN-SPRITES-DECISIONS.md#s-d12--a-nine-sliced-nodes-slices-are-its-image-sub-10-is-suppressed-the-source-split-is-an-authored-border_uv-and-a-slice-with-no-texture-is-a-structural-skip) |
| `S-D13` | [DECISIONS](UI-PLAN-SPRITES-DECISIONS.md#s-d13--the-ruling-that-had-no-red-the-loop-that-has-no-caller-and-ten-sentences-that-could-not-be-written-as-spelled) |
| `S-D14` | [DECISIONS](UI-PLAN-SPRITES-DECISIONS.md#s-d14--the-ten-corrections-landing-found-and-the-two-reds-that-could-not-fire-as-ruled) |
| `S-D15` | [DECISIONS](UI-PLAN-SPRITES-DECISIONS.md#s-d15--tile-needs-a-per-instance-repeat-count-fract-alone-is-the-identity-function) |
| `S-D16` | [DECISIONS](UI-PLAN-SPRITES-DECISIONS.md#s-d16--the-flipbook-writes-uispritesheetindex-and-it-must-a-dense-changedc-inside-or-is-measurably-dead) |
| `S-D17` | [DECISIONS](UI-PLAN-SPRITES-DECISIONS.md#s-d17--s5-carries-am6s-clamp-itself-and-the-seam-is-a-resource-read-not-a-world-call) |
| `S-D18` | [DECISIONS](UI-PLAN-SPRITES-DECISIONS.md#s-d18--the-s5-gate-table-what-each-device-row-samples-and-where-the-clamp-counter-lives) |
| `S-D19` | [DECISIONS](UI-PLAN-SPRITES-DECISIONS.md#s-d19--the-six-corrections-building-s5-found-and-the-one-red-that-did-not-fire) |
| `S-D20` | [DECISIONS](UI-PLAN-SPRITES-DECISIONS.md#s-d20--the-s6-pre-build-audit-the-cursor-hole-closes-with-a-hook-and-six-of-the-rungs-own-sentences-did-not-survive-the-tree) |
| `S-D21` | [DECISIONS](UI-PLAN-SPRITES-DECISIONS.md#s-d21--the-six-corrections-building-s6-found-and-the-pre-existing-bug-the-rung-could-not-build-around) |
| `S0` | [S0-S2](UI-PLAN-SPRITES-S0-S2.md#s0--the-seam-the-gate-the-observer--size-l) |
| `S1` | [S0-S2](UI-PLAN-SPRITES-S0-S2.md#s1--the-ui-shader-into-the-edsl-both-sync-gates-manifest-rows--size-m) |
| `S2` | [S0-S2](UI-PLAN-SPRITES-S0-S2.md#s2--uiinstance-64-b--80-b-all-ten-sites-one-commit--size-m) |
| `S3` | [S3](UI-PLAN-SPRITES-S3.md#s3--the-textured-lane-set-1-the-ui-sampler-uiimage-finally-renders--size-l) · [S3 · LANDED](UI-PLAN-SPRITES-S3.md#s3--landed-2026-08-21--measurements-defect-ledger-reconciliations) |
| `S4` | [S4](UI-PLAN-SPRITES-S4.md#s4--nine-slice-cpu-expansion--the-d4-emission-contract--size-m) · [S4 · LANDED](UI-PLAN-SPRITES-S4.md#s4--landed-2026-08-21--the-landed-set-the-red-ledger-the-golden-and-what-the-build-found) |
| `S5` | [S5](UI-PLAN-SPRITES-S5.md#s5--sprite-sheets-and-the-flipbook--size-m) · [S5 · LANDED](UI-PLAN-SPRITES-S5.md#s5--landed-2026-08-26--the-landed-set-the-red-ledger-the-goldens-and-what-the-build-found) |
| `S6` | [S6-S7](UI-PLAN-SPRITES-S6-S7.md#s6--the-ui-authoring-landing-for-the-sprite-vocabulary--size-s) · [S6 · LANDED](UI-PLAN-SPRITES-S6-S7.md#s6--landed-2026-08-26--the-landed-set-the-red-ledger-the-goldens-and-what-the-build-found) |
| `S7` | [S6-S7](UI-PLAN-SPRITES-S6-S7.md#s7--measurement-gated-dispositions--size-s-may-be-dropped-entirely) |

⚠️ **Two things this table cannot give an anchor, both because the target is not a heading.**
Sub-item locators (`S-D20 (6)`, `S-D12 (1)`) stop at the section — except inside `S-D12` and `S-D13`,
whose numbered items **are** headings. And the gate ids `G0-*`…`G6-*` exist only as inline labels in a
rung's gate table; they resolve to the rung file named for their digit — `G4-*` → `S4`, `G5-*` → `S5`,
and so on — and no finer. **The population that rule governs, named so the next reader can re-measure
it rather than inherit it: a file under `crates/**` whose leading module-doc block (`//!`) carries a
gate id AND a `docs/UI-PLAN-SPRITES-*.md` filename ANYWHERE IN THAT SAME BLOCK — 15 files on
2026-08-28**, 13 under `tests/` and 2 under `src/` (`boyko_render/src/ui/gather.rs`,
`boyko_shaderdsl/src/ui.rs`). Measure it by intersecting `grep -rlE '\bG[0-6]-[0-9]+\b' crates/` with
`grep -rl UI-PLAN-SPRITES crates/` and keeping the hits whose leading `//!` block carries both; the
un-narrowed intersection is 19 files and includes `Cargo.toml`.

⚠️ **"Same block", never "same line" — the two readings differ, and the wrong one looks like a
refutation.** Requiring both tokens on ONE physical line gives **8**, not 15, because most of these
headers name the plan file in their opening sentence and their gate ids several lines further down.
8 is not the population meant — a header that routes to the plan routes for every gate id in it,
wherever in the block the id sits — but it is exactly what a reader who takes an adjacency word at
face value will measure, and then read the 15 as wrong. Both figures are therefore written here.
*(Same-line reading: keep only the hits where one line matches both patterns.)*

**The 15 is a SUBSET of what the split re-pointed, not the same set — do not use it as the re-point
checklist.** The 2026-08-28 split re-pointed **17** headers under `crates/**` from `UI-PLAN-SPRITES.md`
to a `UI-PLAN-SPRITES-*.md` part file; **15** of those 17 also carry a gate id, and **2 do not**:
`crates/boyko_shaderdsl/src/bin/emit_ui.rs` and `crates/boyko_ui/src/sprite.rs`. This split did
re-point all 17 — but a reader who takes the 15 as the re-point checklist never opens those two, so
the next re-split leaves them naming a file that no longer holds their target. **Check the 17.**
Measure it with `grep -rl 'UI-PLAN-SPRITES-' crates/` narrowed to leading `//!` blocks — every such
mention is necessarily new, because no part file existed before the split. *(The same commit
re-pointed a further **6** headers at `UI-PLAN-ANIMATION-*.md`; those belong to that plan's count, so
the split re-pointed 23 headers in all and this plan owns 17 of them.)*

⚠️ **A gate id with no plan filename in its header needs no routing at all**, so counting every
mention answers a different question and gives 239 occurrences on 214 lines in 35 files
(`grep -rhoE '\bG[0-6]-[0-9]+\b' crates/ | wc -l`) — a figure this sentence used to carry unnamed.

---

## 4 · The rung ladder

**Unconditional gate on every rung:** `cargo clippy -p <crate> --all-targets -- -D warnings`;
`cargo test -p <crate> --all-targets --no-fail-fast` for every crate touched, plus
`cargo test --workspace --all-targets --no-fail-fast` before the rung is called done; Miri where new
`unsafe` lands; **the `*_spv_sync` tests run locally with `dxc` present and the result reported — a
SKIP is not a pass**; all 32 `goldens/PINS.toml` hashes unchanged (no rung here touches a scene
render); author-only commit.

> Disk discipline in this worktree: build with `-p <crate>`, never `--workspace`, except for the
> final pre-commit sweep. An `os error 112` or a compiler ICE is the disk —
> `rm -rf target/debug/incremental`.

---

## 5 · Measurement obligations owned by this plan

Every number is named with its instrument and its **discriminating comparison**. None of them exists
today. The recorded failure mode this table exists against is a gate that could not fail and a number
that was not measurable.

| # | Claim under test | Instrument | Discriminating comparison | Rung |
|---|---|---|---|---|
| **10.1** | D2's `NonUniformResourceIndex` divergence is affordable | GPU timestamp around the UI pass | N textured quads at **1 / 8 / 64** distinct slots vs **all on one slot**, N ∈ {256, 2048} | S3 |
| **10.2** | D1's widening is affordable | ring bytes/frame (arithmetic: 128 KB → 160 KB at N=2048) **plus** criterion over pack+sort | same scene, 64 B vs 80 B build | S2 |
| **10.3** | The D6 gate does what its doc says — **and what it cannot do** | the repack counter | static frames: repacks before vs after; **and** a changing frame, reported **unchanged** | S0 |
| **10.7** | The eDSL migration is faithful | `ui_rect_edsl_sync` + `ui_rect_spv_sync` | byte identity of re-emitted HLSL and re-DXC'd `.spv` | S1 |
| **10.8** | **The gather** — the one cost this campaign adds to every node of every frame | a probe counter in `gather_ui_nodes` **plus** wall-clock over the gather alone, separated from pack+sort | probes/node/frame and gather µs at N ∈ {256, 2048} in four states: **(a)** today's rect-only baseline; **(b)** + `UiVisual`; **(c)** + the sprite components; **(d)** a **static** frame with the D6 compare hoisted — which must be **zero probes** | (a),(d) S0 · (b),(c) S3–S5 |

*(clarified 2026-08-21 at the S4 audit: leg **(c)** is an INCREMENT PER RUNG, not one number at the
end — S3 landed 5.00 → 6.00, S4 owes 6.00 → 7.00 (`UiNineSlice`), ~~S5 owes 7.00 → 10.00 (its
three)~~ **S5 owes 7.00 → 8.00 (`UiSpriteSheet`, and it alone)**. S4 and S5 were the only rungs in
this plan with no measurement paragraph of their own; ~~S4's is now written into the rung, and S5
inherits the same obligation.~~ **both now carry one.**)* *(the 10.00 corrected 2026-08-21 at the S5
audit — **S-D16 (2)(3)**: of the three components S5 was to add, only `UiSpriteSheet` is read at
pack. `UiSpriteAnim` is author configuration the flipbook consumes and `UiSpriteCursor` is the
flipbook's private state; listing them would have charged two dead probes to every node of every
changed frame, and `UiSpriteCursor` — being dense — would additionally have sat in the discovery
`Or` as a term MEASURED never to fire.)*

§10.4, §10.5, §10.6 and §10.9 belong to the sibling plans and are not restated here.

**Where a rung reports a number, it reports the instrument's own resolution too.** The particles
campaign's recorded lesson stands: a delta smaller than the instrument's floor is not a small effect,
it is no measurement — and the floor of a GPU zone is not a constant across sessions.

---

## 6 · What this plan exposes to its siblings

Named explicitly, because three other plans build on these and a silent change here is a silent break
there.

| Exposed at | Surface | Consumer |
|---|---|---|
| **S0** | **`ui_pack_inputs!`** — the single spelling of the pack-input set. Adding a visual component to it wires the discovery filter **and** the gather read list together, or fails to compile. ⚠️ **THAT IS TRUE FOR TABLE COMPONENTS ONLY** *(added 2026-08-21 — **S-D16 (1)**, MEASURED: a dense `Changed<C>` inside `Or<..>` can never be true, because the `Or` `QueryFilter` impl overrides none of the dense hooks (`HAS_DENSE`, `resolve_dense`, `dense_include_candidates`), so the inner term's `dense` pointer stays the `init_fetch` NULL and `filter_fetch` returns `false` on its first line. The gather READS the dense component correctly; the discovery filter never sees it change. **A dense component put in this list gets half the wiring and no diagnostic.** Any plan adding one must bump `UiRenderGeneration` at its own writer, or keep the datum in a table column.)* **The list holds SIX after S4 added `UiNineSlice`** (S4 Lands item 6), so the derived census is `ui_pack_inputs!(count) + 1` = **7.00 probes/node/frame** — **and SEVEN / 8.00 after S5 adds `UiSpriteSheet`** *(added 2026-08-21 — S-D16 (3); S5's other two components are deliberately NOT listed)*. **Arity headroom:** the expansion is a FLAT `Or<(Changed<C1>, …)>` and `Or` caps at **12** (arity 13+ are `panic!` stubs that TYPE-CHECK and die at first-frame `init_state`, `filter.rs:2161-2185` — `UI-PLAN-ANIMATION.md` R4). Six today, **seven after S5**, eight with `UiVisual`, nine with the interaction plan's scroll datum. R4's projection of twelve counted `UiText` and `Children`, neither of which is a list member, and counted two sprite components rather than S5's one. R4's mitigation — *"the D31 macro must emit the nested form unconditionally"* — is still owed by this plan's seam rung, which landed the flat form. | **Animation** adds `UiVisual`. **Interaction** adds its scroll datum. Neither may add a component to the gather without adding it here — and neither may write the count down a second time: `ui_s0_discovery`'s length assertion and `ui_s0_measure`'s report both DERIVE it, because S3 already turned a hand-written `5 * 5` into a red with nothing wrong. |
| **S0** | **`gather_ui_nodes`** — the DFS over `UiRoot`/`Children` carrying the inherited clip on its stack. Its pre-order **is** paint order. | **Interaction**: this DFS and `collect_candidates` are the same traversal; D19a's traversal-folded scroll offset rides this stack. |
| **S0** | `UiRenderGeneration` + the per-slot gate, hoisted ahead of the gather. | **Animation**: an animating frame bumps the generation every frame and the gate cannot help it — §10.3 reports that number unchanged, so the animation plan inherits an honest baseline rather than a claim. |
| **S0** | The **two-phase seam** (`gather_into_staging` / `upload_staging`, G0-5-pinned signatures) + the host drive protocol (`stage_frame` → `run_system_once` → `take_frame_output`). The `boyko_app` UI rung (D32's floor) is **deferred** to the Renderer-as-ECS-resource rung (2026-08-21 ruling) — the observer S0 ships is Phase 1, device-free. | **All three.** The device-free observer makes the seam's behaviour falsifiable on any machine; the human-visible floor arrives with the windowed rung, and until then **R1**/**R2**'s visual half rests on the owner-run windowed leg. |
| **S2** | `UiInstance` at 80 B with `uv` and S-D2's bit map. ~~**Bits 5..19 are free; bit 4 is reserved.**~~ ⚠️ **AFTER S5 THE FREE BITS ARE GONE: bit 4 stays reserved for S7, and bits 5..19 are spent by `FLAG_TILED` (bit 5) plus the two 7-bit tile counts (bits 6..=12, 13..=19).** *(amended 2026-08-21 — **S-D15**: `Tile` needs a per-instance repeat count, `uv`'s four floats are all consumed as `sub_min`/`sub_max`, and the record has no spare word — so the count takes the budget the budget exists for. Fifteen bits free, fifteen spent.)* | **Animation**: D5 folds the visual transform at pack and costs **zero** GPU bytes, so animation needs none of these bits. ~~If it ever does, it takes bits 5..19 and says so here.~~ **If it ever does, there is nothing left to take: the next per-instance datum WIDENS the record (the S2 decision, again) or aliases a field, which S2 spent 16 B per record to stop doing. Say so here either way.** |
| **S3** | `FLAG_TEXTURED`, the bindless slot lane, the UI sampler binding, font-optional boot. | **Aether**: the `ui` construct's `image` / `sheet` vocabulary can only name what S3–S5 built. |
| **S5** | The `u16 sheet_id` dense-handle mint — **`UiSheetTable::register(UiSheet) -> SheetId`, the `FontTable::load` verb** *(named 2026-08-21 — S-D16 (3): Lands item 1 landed the struct and the column and no registration verb, so this row exposed a surface no line of the rung created — and no gate constructed one either)*. **Also: `UiSpriteSheet` is a MODIFIER of `UiImage`, not a replacement — a sheet-bearing node still carries `UiImage`, whose `texture`/`uv_*` the gather substitutes.** | **Aether**: the sheet-id mint is the natural thing for the construct to own at expand time (research §11 item 6). **U5's precondition is stated purely in texture-naming terms (`UI-PLAN-AETHER.md:561-566`) and therefore does not cover the sheet route; under the ruling above it does not have to — a `sheet:` prop still emits a `UiImage`.** *(U5's own field-list note also says `nine_slice:` is "**five** keys, not four" and then lists four; the fifth would be `_pad`, which is not authorable. Corrected in that file.)* |
| **S4** | D4's pinned emission order — ~~**over the three terms S4 emits (background → nine-slice TL..BR → image)**~~ **over the terms S4 emits: background → EITHER the nine-slice sub-quads (row-major TL..BR) OR the image**, with `UI_RECORDS_PER_NODE = 11` as the sub-record **stride** (the maximum emission is 10) *(amended 2026-08-21: S4 pins what it emits; the contract's last two terms are pinned by their own rungs, because at S4 neither exists — see the S4 audit ledger. **Amended again the same day — S-D12 (1): the nine-slice and image terms are ALTERNATIVES, since `UiNineSlice` suppresses the image record it slices.** A sibling adding a term takes a free sub code and raises the stride; it does not renumber these.)*. | **Interaction**: the focus ring is the last quad of the contract, so a focused node's ring is never painted under its own glyphs — **but S4 does not gate that, because `FocusRing` has zero occurrences in `crates/` and I9 is the rung that emits it. Interaction inherits the obligation to extend the ~~`append`-lane~~ **STAGED** order assertion (G4-2's shape) when it lands the ring, and to raise `UI_RECORDS_PER_NODE` for it — ~~raise~~ **it takes the next free sub code and `UI_RECORDS_PER_NODE` follows, because S-D13 (3) derives the stride as `UI_IMAGE_SUB + 1` rather than authoring it**.** *(both corrected 2026-08-21 — S-D13 (5)(2) and (3). "`append` lane" is the noun S-D12 struck from G4-1 as unobservable — `UiUploadSystem.keys` is private (`upload.rs:160`) — and this sentence was propagating it into the sibling plan, which is where a struck claim does the most damage: Interaction would have written a gate against a lane it cannot read.)* Likewise the glyph term: D4 itself records that glyph order "is decided purely by the order the host appends them", so it is a HOST APPEND DISCIPLINE, not a property of this lane. |

**And what this plan needs from them:**

| Needed | From | Blocks | Fallback if it is late |
|---|---|---|---|
| **D7's registration table** | ~~`UI-PLAN-AETHER.md`~~ **unowned — S-D20 (7)** | ~~**S6 only**~~ **NOTHING — S6 LANDED 2026-08-26 on the fallback** | S6 lands with ~~fifteen~~ **about thirty** hand-written landings (S-D20 (6)); S0–S5 are unaffected. **Taken: 27 landings (9 × 3) plus 4 new leaf parsers plus 6 comparator rows.** D7 is still unowned and now blocks nothing in this plan |
| **The UI clock (D15)** | `UI-PLAN-ANIMATION.md` | nothing | ~~S5 reads `Time`'s real delta through a one-function seam the animation plan later replaces~~ **S5 takes `Res<Time>` and applies AM6's clamp itself at the one site (`UI_FALLBACK_MAX_DELTA = 0.1`, AD1's own number); the replacement swaps the parameter for `Res<UiClock>` and reads **`dt_virtual`** ([`UI-PLAN-ANIMATION-DECISIONS.md` AD9](UI-PLAN-ANIMATION-DECISIONS.md#ad9--which-field-a-consumer-reads-is-decided-by-whether-it-carries-d15s-flags-bit-and-the-clamp-has-exactly-one-definition), 2026-08-26 — the field was unnamed on both sides, and the documented default `dt_real` would have reversed this rung's pause and scale semantics), deletes the clamp, and is owned by animation rung **A0b**, which is where it was finally sequenced** *(corrected 2026-08-21 — **S-D17**: the struck fallback is the option AD1 rejects by name and for this consumer by name, and `Time`'s clamp provably does not reach the real delta. The animation plan's exposure table lists `UiClock` as consumed by this flipbook and records no fallback, so the dependency was declared satisfied on one side and waived on the other.)* |
| **`UiVisual`** in `ui_pack_inputs!` | `UI-PLAN-ANIMATION.md` | nothing | S0's macro is already shaped to take it; §10.8 leg (b) simply does not run until it exists |
| **The `paint_seq` agreement** | `UI-PLAN-INTERACTION.md` | G0-4 | G0-4 asserts the gather's pre-order against `collect_candidates` as it exists **today**; if the interaction plan changes that traversal, G0-4 is its gate too |

---

## 7 · Risks

### SR1 — the widening is a ten-site lockstep edit on a path with no live host and no binary gate

This is the architecture's **R1**, restated because it is this plan's risk and its whole ordering
exists to answer it. Today: no production host draws any UI; the GPU goldens **skip gracefully** on a
device-less host, so a green CI run may have exercised nothing; `ui_hud_screenshot.rs` is `#[ignore]`d
eight times; and the only pin on the committed `.spv` is a byte **length**, which cannot see a
re-compile drift at the same size.

*Mitigation, and it is the ladder itself:* **S0 before S1 before S2.** The observer exists before the
gate; the gate exists before the edit. **M2-b** is the mutation that says whether the mitigation
worked, and if M2-b does not red, S2 is not done. *Status 2026-08-21: S0's seam landed (two-phase,
observer + G0-2/G0-3/G0-5 green, measurement legs run), so the sequencing argument holds and* ***S2
is unblocked as written*** *(S1 first, per the ladder). The observer's half of this mitigation is
device-free; the visual half rides the completion criterion below.*

*Residual:* every gate here needs a GPU. On a device-less machine S2's gate is vacuous and reports
green. **The rung's completion criterion therefore includes "run on the RTX 3060 with the four hashes
reported", not "CI is green".**

### SR2 — S1 may not achieve `.spv` byte identity, and the fallback weakens the gate it was built for

If the generator's formatting shifts DXC's output, S1 lands as a re-bless (S-D10) — and a re-bless is
exactly the operation the gate exists to prevent being casual. The mitigation is that it happens
**once**, before any semantic change, with the four UI goldens compared by a human in the same commit;
after that the gate is live for every subsequent edit, which is the state S2 needs.

*What would make this worse and must not happen:* re-blessing during S3, when the shader is also
changing semantically. If S1's identity fails, the re-bless commit contains **nothing else**.

### SR3 — `NonUniformResourceIndex` is the first non-uniform thing in this shader

The design note on the text branch is explicit that it is "a uniform-per-instance branch, so the rect
majority is unregressed". A bindless sample is not. On some hardware a divergent descriptor index
becomes a waterfall loop **per quad**, on a pass that is otherwise trivially uniform.

*Mitigation:* §10.1 is a gate, not a note, and Model A stays reachable **without changing one
component** — which is the entire argument for starting with D2 rather than a claim that D2 wins.

*Counter-evidence, recorded because it is substantially right:* WebRender — the one team whose whole
job is drawing UI, and the only one that reasoned about this fork in public — chose atlases. Three of
its reasons transfer even though the driver-bug one does not: UI textures are small, few and
long-lived; the 4095 slots are shared with world materials; and the divergence is real. §10.1 is what
turns that from an argument into a number.

### SR4 — the slot budget is shared and nothing reserves anything

D3 refuses a UI reservation, so a UI that registers 500 icons individually steals 500 slots from the
scene. The correct answer costs nothing at runtime — pack the icon set offline with the existing
skyline packer (`boyko_fontbake/src/atlas.rs:138`) and spend one slot — but nothing enforces it.

*Mitigation:* a diagnostic counter of UI-held slots, reported by the observer rung, and S-D2's
const-assert so that the *response* to slot pressure (raising the capacity) cannot silently truncate
the field. Neither prevents exhaustion; both make it visible before it is a magenta screen.

### SR5 — the nine-slice expansion multiplies the instance count by up to ~~9~~ **10**

A UI whose panels are all nine-sliced pays ~~9×~~ **10×** the ring traffic and the sort. At the
2 048-node figure §10.2 uses, that is 160 KB → ~~1.44 MB~~ **1.6 MB** per frame.

*(corrected 2026-08-21 — S-D12 (1). A nine-sliced imaged node emits **10** records (background + nine
regions) where a plain rect emits 1, so the multiplier against the §10.2 baseline is 10, not 9. It is
**5×** rather than 10× against an already-imaged node, whose S3 cost is 2 records. The ruling did not
raise this risk — it made it countable: under the ADD arithmetic the same node emitted 11 records, one
of which was invisible under the other ten.)*

*Mitigation:* G4-4 pins that the scratch does not reallocate, and M4-d records the tempting wrong fix.
The real answer if it ever bites is `fill_center = false` (**9 records**) and authoring fewer sliced
panels — neither is a renderer change, which is why nothing is built for it now.

---

## 8 · Open questions for the owner (VALUES / SCOPE — also to be filed in `docs/OPEN-QUESTIONS.md`)

These are not perf or architecture forks; those are decided above with numbers or with reasons.

1. **`UiSamplerMode` at boot, or per-sprite from the start?** S-D4 ships one mode chosen at
   `ui_setup` and reserves bit 4 for the per-sprite extension. A UI that must mix pixel-art icons and
   photographic images in one pass needs the extension on day one, and that is a product call, not a
   perf one.
2. **How much demo above D32's floor.** The architecture's §13 Q5, unchanged: the minimal
   `boyko_app` rung is a v1 deliverable (S0), but whether a richer showcase scene belongs inside this
   campaign is scope.
3. **Is a checked-in sprite asset wanted at all?** S-D5 makes every *gate* procedural. The *demo*
   sprite could stay procedural too — which would mean the repo still contains no UI sprite, and the
   first real one arrives with a game.

---

## 9 · Sources

**In-tree, read for this plan** (worktree `D:/wt/ui`, branch `feat/ui-advanced`) —
`crates/boyko_render/src/ui/{instance,pack,upload,plan,draw,resources,mod}.rs` ·
`crates/boyko_render/src/gpu_column.rs` (`ui_setup`, `ui_upload`, `RhiContext`) ·
`crates/boyko_render/src/bindless.rs` · `crates/boyko_render/src/texture.rs` ·
`crates/boyko_render/shaders/ui_rect.{vs,fs}.hlsl` ·
`crates/boyko_render/tests/{ui_rect_gpu_golden,ui_pack_cpu,ui_no_realloc}.rs` ·
`crates/boyko_render/Cargo.toml` · `crates/boyko_ui/{Cargo.toml,src/components.rs}` ·
`crates/boyko_rhi_vulkan/src/bindless.rs` (the capacity **and** the shared sampler) ·
`crates/boyko_rhi_vulkan/src/rhi_impl/device.rs` (`create_graphics_pipeline_bindless`) ·
`crates/boyko_rhi_vulkan/src/present/{scene_types.rs (UiPass), passes/present_blit.rs, passes/gbuffer.rs}` ·
`crates/boyko_rhi/src/{encoder,device,enums}.rs` · `crates/boyko_app/{Cargo.toml,src/runner.rs}` ·
`crates/boyko_shaderdsl/src/{particle.rs,bin/emit_particles.rs}` (the leaf + generator idiom) ·
`crates/boyko_rhi_vulkan/tests/particle_edsl_sync.rs` (the sync-test plumbing) ·
`goldens/PINS.toml` · `docs/SHADER-VARIANT-MANIFEST.md`.

**Design authority:** `docs/UI-ADVANCED-ARCHITECTURE.md` rev 2 §3 (D1–D7, D31, D32), §4 (D8a–e),
§8 (D30), §9, §10, §11, §12.
**Evidence:** `docs/UI-ADVANCED-RESEARCH-SPRITES.md` (six-implementation survey; §7's live findings;
§10's argument against the recommendation), which carries the external citation list rather than
duplicating it here.
**Register shape:** `docs/PARTICLES-PLAN.md` (rung ladder, gate/red-mutation discipline, the F15
"a skipped `*_spv_sync` is not a pass" rule).
