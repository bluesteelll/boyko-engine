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
- **graphify has been off-target for the render/VB path** for this entire session; every query
  returned `boyko_demo` internals. Grep/Read is the working path there.
- **`.claude/settings.local.json` is dirty** from earlier sessions and is deliberately never staged.
