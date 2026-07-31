# VG-R11 — the upper-bound instrument, and why it is still UNSOLVED

**Status: UNSOLVED, and the statement of why is now much sharper than "nobody has designed it yet."**

`[k1].k1_fire_instrument_status` has read `"UNSOLVED -- needs a non-saturating upper bound on VISIBLE
density"` since Rev 25. This document does not change that value. What it adds is a **catalogue of
seven adjudicated deaths** and **four structural results**, two of which close whole families rather
than individual candidates — and one of which shows that on `assets/vg_corpus` the question the
instrument was funded to answer **cannot be answered by any instrument at all**.

It is written after R0 completed, against R0's own measured rows, and it exists because the owner's
pre-registered disposition for the UNDECIDED outcome is `k1_outcome.undecided_disposition =
"fund_upper_bound"`. The honest answer to that disposition is below, and it is not a build order.

---

## 0. Provenance — who established what

This campaign's standing rule is that an author's account of their own work has been wrong more often
than right, so the origin of every claim is marked.

* **ORCHESTRATOR-VERIFIED** — re-derived or re-grepped by the author of this document, from the files,
  this session. Everything in §3 and the engine facts in §4 carry this mark.
* **REVIEWED** — produced by one agent and refuted or confirmed by an independent adversarial pass.
  All seven §4 rows carry this mark.
* **UNVERIFIED** — stated once, never checked. Enumerated in §8 rather than left to be found.

Seven candidate families were adjudicated across two runs. **Every one is dead.** The first run lost
three of its four angle records to a tooling failure (structured-output validation, not a design
finding) and produced a *recommendation* that no reviewer had seen; the second run recovered the three
angles and put that recommendation through three independent lenses. **All three lenses refuted it.**

---

## 1. What R0 established, and the one thing it left open

R0 censused a 2 279 237-triangle licence-clean corpus at full detail, over two committed camera paths
and a four-rung ladder ([VG-R0-DENSITY-CENSUS.md](VG-R0-DENSITY-CENSUS.md)):

| path | covered @1080p | visible @1080p | @1440p | @2160p | `D_est` |
|---|---|---|---|---|---|
| `orbit_mid` | 167 573 | 69 517 | 103 819 | 176 397 | **1.0527** |
| `approach_close` | 459 878 | 146 965 | 185 065 | 234 094 | **0.5090** |

`submitted` reads **2 279 237 on all eight rows** — this engine has no geometry frustum culling, so
every triangle is submitted every frame. The modal bucket is 0 at every rung on both paths. Neither
path converged (residuals 0.4114 / 0.2094 against a 0.05 margin). Verdict: `min` over paths = 0.5090
< `[k1].d_est_min` = 1.0 ⇒ **UNDECIDED, escalate**.

`D_est` is frozen as a **lower** bound (`[k1_instrument].d_est_bound_direction`,
`d_est_may_fire_k1 = false`). Firing K1 needs an **upper** bound `U` with `U < 1.0` provable. That is
the whole of what R0 left open.

---

## 2. The requirement, as an inequality with units

For a committed path `p`:

* `A(p)` = `covered_pixels(p, [census].decision_resolution)` **[px]** — mesh-covered texels only.
* `N(p)` **[tri]** = in-frustum, front-facing-*or-not* (see §3.3), unoccluded triangles with nonzero
  visible area. Resolution-independent.
* `D(p) = N(p)/A(p)` **[tri/px]**. `1.0` is the regime boundary by definition, not by tuning.

```
R-1  DIRECTION       U(p) ≥ D(p) for every p, PROVABLY
R-2  NON-SATURATING  U has no construction ceiling
R-3  REACHABILITY    U(p) < 1.0 attainable on genuinely sparse content
R-4  SITE            measures the SHIPPED VisibilityBuffer chain
R-5  AGGREGATION     one verdict over P — AND THIS DOES NOT EXIST YET (§6 Q1)
```

⚠️ **There is no firing aggregation anywhere in the frozen file, and `k1_decision_rule` has no FIRE
branch at all.** `[k1].k1_path_aggregation = "min_over_committed_camera_paths"` is justified
*explicitly and only* for refutation. Choosing the firing fold **after** seeing the numbers is exactly
the defect the two-document split exists to prevent, so it is raised as an owner question in §6 and
deliberately **not** settled here.

---

## 3. Four structural results

### 3.1 The governing theorem — K1's FIRE branch is closed on this corpus, whatever the instrument

**ORCHESTRATOR-VERIFIED.**

Winning a texel *proves* visibility, so `visible_tris(p, 2160p) ⊆ N(p)`, hence `D_est(p) ≤ D(p)` —
which is exactly what `d_est_bound_direction = "lower"` asserts. On `orbit_mid`:

```
D_est = 176 397 / 167 573 = 1.052658  ⇒  D(orbit_mid) ≥ 1.052658 > 1.0
```

Any **sound** upper bound therefore reads `U(orbit_mid) ≥ 1.052658`. It cannot go below 1.0. To fire,
a survivor counter would have to return fewer than **167 573** survivors against a proven floor of
**176 397** — short by 8 824 triangles, **5.27 % over the bar**.

Under a `max`-over-paths firing fold — which K1's own universal phrasing implies (*"The corpus **never**
approaches ~1 triangle/pixel"*, [MESHLET-VIRTUAL-GEOMETRY-PLAN.md](MESHLET-VIRTUAL-GEOMETRY-PLAN.md)
§6's kill table) — **K1 can never fire on `assets/vg_corpus`, whatever the instrument.** Under `min`
it could fire on `approach_close` while `orbit_mid` is *proven* dense: a verdict contradicting a proof
on the same census.

**Consequence: `fund_upper_bound` cannot change the K1 verdict on this corpus.** It can narrow
`approach_close` from `[0.509, ∞)` to `[0.509, U]`. That is a real result and it is not the question
the disposition was funding.

### 3.2 The sampling barrier — closed in BOTH directions

**REVIEWED, and the second half is new.**

*Forward half (from the refuted `supersampled-limit` angle, whose theorem survived its own refutation).*
For **any** sample set `S` — pixel centres, MSAA positions, k-winners-per-texel, an 8× internal id
target — the recorded set `V(S) ⊆ V_true`, because winning a sample proves visibility. Enrichment
raises the ceiling and **never changes the bound direction**. So the frozen file's *"must come from
OUTSIDE `vb_id`"* is understated: it must come from outside **sampling**.

*Reverse half (from the recommendation's soundness lens).* An instrument that *rejects* using sampled
data inherits the same blindness **with the sign reversed**. A depth buffer records the nearest surface
**at pixel centres only**. A triangle visible solely through a sub-pixel gap — a hairline crack between
two nearer triangles, a picket fence, a silhouette sliver — is farther than the stored depth at *every
sampled centre in its footprint*, so a correctly-implemented conservative HZB **rejects it**. It is in
`N_true` and not in `S`. `V ⊄ S`: the bound breaks, and it breaks toward falsely firing.

**Together these close the sampling family entirely.** You cannot get an upper bound *from* sampling,
and you cannot repair a lower bound *by more* sampling. Any surviving candidate must be analytic at
both the admit and the reject stage.

### 3.3 This engine does not backface-cull — so the named candidate is unsound here

**ORCHESTRATOR-VERIFIED.** [`crates/boyko_app/src/gpu_scene/mod.rs`](../crates/boyko_app/src/gpu_scene/mod.rs)
builds `vb_raster_pipeline` — the pass that writes `vb_id` and the depth ring, the pass that produced
the entire R0 census — with `cull_mode: CullMode::None`. A grep over `crates/` for every `cull_mode:`
site returns exactly **two** non-`None` values in the whole tree, both `CullMode::Front`, both in the
CSM shadow passes.

The instrument named in two live texts is *"a counter of triangles surviving frustum **+ backface**"*.
In this engine that numerator is **unsound**: a back-facing triangle can and does win `vb_id` texels on
open geometry, so R0's own `visible_tris` **contains back faces**, and rejecting them gives `V ⊄ S`.

Removing the backface conjunct is the only sound repair. What remains is frustum + occlusion. And with
occlusion closed by §3.2, what remains is frustum alone — which is `submitted_per_covered_pixel`
restricted to the frustum, i.e. **refuted candidate (1)**, whose measured reading at the decision rung
is 13.601 (`orbit_mid`) / 4.956 (`approach_close`).

⚠️ This was not noticed in 37 plan revisions. It is not a subtle point once looked at; it was simply
never looked at, because the candidate was always argued about and never sited against the code.

### 3.4 Tightness is not the binding constraint

**REVIEWED, arithmetic re-derived here.** The corpus is laid out as a 3-column grid with
`GRID_SPACING = 1.30` and each asset normalised into a unit cube (half-size ≤ 0.5), all at `z = 0`
([`crates/boyko_app/tests/vg_corpus_scene/mod.rs`](../crates/boyko_app/tests/vg_corpus_scene/mod.rs)).
Lateral separation 1.30 > 0.5 + 0.5, so **the assets do not overlap and there is essentially no
inter-asset occlusion.** An occlusion stage therefore removes almost nothing.

So even a **zero-slack** instrument — perfect occlusion culling, perfect footprints, no mip
granularity — reads on the order of **6.8 tri/px** on `orbit_mid`, against a ceiling of 13.601 and a
bar of 1.0. No amount of tightening reaches the threshold. This is §3.1 reached from the other side,
and it means an engineering effort spent on making the bound *tighter* is spent on the wrong axis.

---

## 4. The catalogue of deaths — seven families

| # | Family | Verdict | The fatal objection |
|---|---|---|---|
| 1 | `submitted_per_covered_pixel` | **REFUTED** (frozen record) | Sound and non-saturating, but counts backface-culled and off-screen geometry, conflating *"triangles are small"* with *"the level has lots of geometry"*. **Measured at the decision rung: 13.601 / 4.956.** ⚠️ It *falls* with resolution (approach_close: 4.956 → 2.788 → 1.239 across rungs 1–3), so any future refutation of a submitted-side family must be derived against **4.956**, not the 13.6 the record quotes as its headline. |
| 2 | Frustum+backface survivor counter in `vb_raster.fs.hlsl` | **REFUTED ×3** | Wrong stage (a fragment shader sees approximately the visible set `vb_id` already caps); inert anyway (survivors include every occluded in-frustum triangle); and **§3.3: unsound in this engine**, which is new and is fatal on its own. Also needs `fragmentStoresAndAtomics`, not enabled at device creation. |
| 3 | Any per-fragment / incidence statistic | **DEAD BY IDENTITY** | (triangle–pixel incidences)/covered_pixels **≥ 1 identically**, since every covered pixel is covered by at least one triangle. Kills stencil-increment depth-complexity buffers, Nanite-style overdraw visualisations, and raw `FRAGMENT_SHADER_INVOCATIONS` **without measurement**. |
| 4 | The size histogram / modal bucket | **RETIRED to `report_only`** | It is a distribution over the same `vb_id` readback, hence capped by one-winner-per-texel: it bounds nothing from above. R0 measured its cross-rung shift at **+0 on all four adjacent pairs**, confirming the left-censoring prediction by measurement. |
| 5 | `supersampled-limit` — extrapolate the ladder | **REFUTED** | §3.2's forward half: enrichment cannot change the bound direction, and a finite ladder of lower bounds is consistent with unboundedly large `N`. The reviewer built the counterexample the author did not: 5 000 000 sub-lattice slivers leave every measured row byte-identical while the instrument reads `1.17e-4 < 1.0` ⇒ **fires** against a true 5.0 tri/px. The failure is **anti-correlated** — sub-sample blindness is maximal exactly in the micro-polygon regime. Also: `q = 1.5012 > 1` on `orbit_mid` makes the tail integral diverge, so the instrument is **undefined** on one of the two committed paths at the data it was designed against. |
| 6 | Analytic projected area | **REFUTED** | §3.3 kills the winding conjunct; the near-plane conjunct is **vacuous** (`vb_near_clip` has no reject channel — it clamps rather than rejects). Dropping both leaves `S` = every submitted triangle, i.e. family 1 exactly, at 13.6015. The reviewer also found the dispatch needs a flat `thread_id → (instance, local_tri)` map that **does not exist**: `VbInstanceRow` carries no triangle-count prefix sum, so *"no new upload, no new binding"* is false. |
| 7 | Per-primitive visibility set / compute survivor count with a conservative HZB | **REFUTED by all three lenses** | **Soundness:** §3.2's reverse half — the HZB reject is sampling-derived and rejects sub-pixel-visible triangles; plus §3.3's backface problem; plus nothing in the design handles a vertex behind the near plane, where `x/w` flips sign and the screen rect lands on the *wrong side* — under-coverage, the fatal direction. **Engine:** the HZB **cannot be built on this RHI** — `create_texture` mints views at `base_mip_level: 0` unconditionally ([`crates/boyko_rhi_vulkan/src/texture.rs`](../crates/boyko_rhi_vulkan/src/texture.rs)), so a view at mip *N* does not exist and cannot be produced; writing a pyramid needs new RHI surface, which is not in any cost model. **Reach:** §3.4 — a zero-slack instrument still reads ~6.8 tri/px. |

**Two families were also priced and set aside, not refuted:**

* **Mesh shaders / `SV_CullPrimitive`.** **ABSENT — ORCHESTRATOR-VERIFIED**: `grep -E
  'mesh_shader|MESH_SHADER|meshShader|drawMeshTasks|VK_EXT_mesh_shader'` over `crates/` returns
  **zero occurrences**. No extension request, no feature struct, no cap field, no degrade rule. To use
  them, first probe the way `supports_ray_query` already does.
* **`VK_EXT_conservative_rasterization`.** **ABSENT — ORCHESTRATOR-VERIFIED**: zero occurrences over
  `crates/`. And the trap if it were added: `degenerateTrianglesRasterized` is a device *property*,
  not a switch; where it is false, primitives quantising to zero area are culled before any counter
  sees them — **the sub-pixel population the campaign exists to count is dropped, and the "upper
  bound" silently inverts on the one population that matters.** Structurally the same trap as the
  Frostbite small-primitive reject.

**A gate defect worth recording separately.** The refuted recommendation proposed gate part (c),
`|S| ≥ visible_tris` on the real corpus, as its cross-instrument soundness check. It is **structurally
blind to the exact failure its own design constraint names**: the set left behind by the forbidden
small-primitive reject is *{triangles covering at least one pixel centre}*, and **every texel winner
covers a pixel centre by construction**, so `visible_tris ⊆ S` even on the broken instrument and (c)
passes. A gate that cannot see the failure its sibling clause exists to prevent is not a gate.

---

## 5. What would have to change

Nothing here is a recommendation to build. These are the only doors that are not closed.

1. **A different corpus.** §3.1 is a statement about `assets/vg_corpus`, not about content in general.
   On content whose densest committed framing sits below 1 tri/px, the FIRE branch is open — and there
   is an external prior that such content exists: Frostbite's published per-filter triangle census
   (GDC 2016 — 443 429 triangles, 78 % culled, 95 404 rasterised) points *toward* K1 on 2016-era AAA
   content. This is the only route on which a funded instrument can return a verdict. **UNVERIFIED:**
   that figure is second-hand and its coverage denominator was estimated, not measured.
2. **An analytic reject stage.** §3.2 forecloses sampling at both ends, so a surviving instrument must
   admit and reject analytically. Exact analytic visibility (Auzinger 2013; Nirenstein 2002) is the
   reference point for what *exact* costs — demonstrated orders of magnitude below 2.28 M triangles,
   or at offline PVS timescales. Not a frame instrument.
3. **A covered-fraction floor.** Both corpus frames cover **8.1 %** and **22.2 %** of the screen. A
   firing verdict from an 8 %-covered frame inherits every criticism the refuting side already carries.
   `[k1_instrument].representativeness_floor_status` records this axis UNSOLVED, and R11 would inherit
   it unclosed.
4. **The RHI would need per-mip image views** before any hierarchical-depth design is even expressible.

---

## 6. Owner questions

**Q1 — SCOPE: freeze `k1_fire_aggregation` before anything else, or not at all.**
No firing fold exists in the frozen file. §2 argues `max` from K1's own universal phrasing, and it is
stated here so it is on the record, **not** so it is settled. *If `max`* — the sound reading — K1
cannot fire on this corpus and Q2 becomes the live question. *If `min`* — K1 could fire on
`approach_close` while `orbit_mid` is proven dense at 1.0527, producing a verdict that contradicts a
proof on the same census.

**Q2 — VALUES: `fund_upper_bound` is shown futile on this corpus. Which branch?**
*(a) Fund it anyway, for a future corpus.* Returns a bracket on `approach_close` and a reusable
instrument; returns **no K1 verdict here**. And §4 row 7 shows the concrete design that reached
recommendation status is refuted on all three axes, so this branch starts from a blank sheet, not from
a spec.
*(b) Change the target content class and re-run R0b–R0d.* The only route on which the instrument can
return a verdict. Cost: a new corpus, a new manifest + fetch + hashes, and a full census re-run.
*(c) Accept the premise unadjudicated and proceed.* Costs nothing now; means the campaign proceeds
knowing K1 was never tested — which is exactly what the UNDECIDED disposition exists to make explicit
rather than silent. Note that §3.1 proves the corpus **is** dense on one committed framing, which is
evidence *for* the mechanism, though not the evidence `k1_decision_rule` asks for.

**Q3 — SCOPE: the two text repairs in §7 need a frozen-file amendment. Authorise it?**

---

## 7. Text repairs owed — and why they are BLOCKED

Two live texts still offer a refuted instrument in the present indicative, and one states a property
its own document contradicts 170 lines later. **Both repairs are blocked, for the same reason, and the
block is the finding.**

| Claim | Stated in the plan | Stated in the FROZEN file |
|---|---|---|
| *"the tight one available is a counter of triangles surviving frustum + backface"* | §5.6 | `[k1]`'s pre-`k1_fire_at_r0` comment |
| `visible_tris(R)` is *"monotonically increasing"* — contradicted by §5.7's own *"not strictly monotone"* (non-nested sample lattices, depth-tie flips) | §5.5 | `[k1_instrument]`'s derivation comment |

Each claim lives in **two** texts, the second of which is `VG-CAMPAIGN-THRESHOLDS.toml`, frozen on
2026-07-31 at Rev 34. §14.1(a) requires a repair of a stated fact to reach **every text stating it**.
Repairing only the plan is precisely the recorded Rev 31 defect — *"the repair reached 1 of its 2
stating texts"* — so a partial repair would be worse than none.

Editing the frozen file is an amendment: a new plan revision, a dated §11.1 row, and the recorded
sha256 updated **in the same commit** (which reds `vg_thresholds_freeze.rs` and
`vg_density_census.rs::the_thresholds_file_is_the_one_r0a_froze` until it is). That is an owner-visible
act, so **neither repair is made here.** Q3 asks for it.

⚠️ **One "repair" from the first synthesis is REJECTED rather than carried.** It called the figure
*"~2.07 M covered pixels"* an error repeated in four texts. It is not: the plan writes *"the screen has
covered pixels, **at most** ~2.07 M at 1080p"* — the screen size used as an **upper bound** on covered
pixels, which is the conservative direction and *strengthens* the refutation it serves. Substituting
the measured 167 573 / 459 878 does not correct the argument; it strengthens it **4.5×–12×**. The
distinction between "the record is wrong" and "the record is cautious" is exactly the kind this
campaign has been burned by, in both directions.

---

## 8. Explicitly UNVERIFIED

* The Frostbite GDC-2016 density figure in §5 — second-hand, denominator estimated.
* The "~6.8 tri/px zero-slack" figure in §3.4 — derived from the grid geometry by a reviewer and
  re-checked for *direction* only, not re-derived numerically here. What **is** orchestrator-verified
  is the input it rests on: the corpus grid spacing exceeds the asset diameter, so there is no
  inter-asset occlusion.
* Every engineering cost estimate in the refuted designs. None survived its review, so none was
  re-priced.
* Whether `maxImageDimension2D` and this box's real `device_local_heap_bytes` admit any higher SSAA
  rung. The host prints these **only on failure**, so a successful 2× arm leaves no record of either.
* The three §4 rows adjudicated in the first run (1, 3, 4) rest on the frozen record plus one
  reviewer; rows 2, 5, 6, 7 carry a full independent refutation pass.
