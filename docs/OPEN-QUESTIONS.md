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
`config.enable_validation && BOYKO_DISABLE_VALIDATION unset` (`boyko_rhi_vulkan/src/device.rs:2350`)
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
- **The ECS's global query-type registry can exhaust under the full lib suite.**
  `MAX_QUERY_TYPES = 1024` is a process-global cap minted lazily, and `boyko-ecs --lib` runs 864
  tests in parallel. When scheduling happens to mint the 1025th distinct query shape, whichever test
  is unlucky dies with a TERMINAL panic naming the cap. Observed once, then 3 consecutive clean runs
  of the same binary. It is order-dependent, not a regression signal — check by re-running before
  bisecting anything.
- **`.claude/settings.local.json` is dirty** from earlier sessions and is deliberately never staged.
