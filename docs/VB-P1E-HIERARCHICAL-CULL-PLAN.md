# VB-P1e — hierarchical froxel light cull (implementation plan, Rev 6)

**Status:** DESIGN, Rev 6 — **DESIGN SURFACE APPROVED; PROSE FROZEN.** Rev 6 was reviewed after this
block was written; see the ERRATA + PROSE FREEZE section at the end of this status block for the
outcome, the corrections, and the one open P0 (which is discharged in code, not here). **Rev 7 will
not be written.** Implementation proceeds rung by rung: H1 → H1.6 → H2 → H3 → H4, each against its own
gate; **no rung is authorized by this document alone** — H1.6 in particular re-pins a committed `.spv`
and carries a zero golden-move budget.

Rev 5 was reviewed and came back **0 × P0, 5 × P1, 6 × P2**. **Rev 6 fixes exactly those eleven and
changes nothing else.**

**The remaining defect source is the EDIT PROCESS, not the design — and that is what bounds this
revision.** Every revision so far has introduced new P1s **at precisely the lines it edited**: Rev 5
fixed six P1s and created four new ones, all four inside the six paragraphs it touched, none anywhere
else. Rev 6 therefore changes **only** what a finding names by line, and rewrites no paragraph a
finding does not name. The load-bearing surface — §4's thread map, §5 Steps 0–2, D7's bound, D8's
constants and the `NoContraction` construction — survived **two independent re-derivations and a real
DXC compile** this round with **zero** findings, and is **not reopened**.

**What is now measured rather than modelled (and one of the two cuts against the design):**

* **§10 ABORT clause 1 does NOT fire.** H1's selectivity minimum on a gated config is **7.27x** against
  the **4x** kill line (`hier_cull_abort_clause_1_does_not_fire`,
  `crates/boyko_rhi_vulkan/tests/lighting_l1_host_oracle.rs`).
* **The honest caveat, stated at the top rather than buried in §7.** On the denser **in-frustum** rig
  the shipping bench config (M2) sits at **7.27x / 7.45x / 7.41x** at `N` in {128, 512, 1022} — i.e.
  **below the 8x** of this plan's own design gate. That gate (§8.6 assertion 5, `pairs_hier/pairs_flat
  <= 1/8`) is **scoped to the bench Kronecker rig**, where M2 measures 20.30x/38.60x and passes, so
  nothing is failing a gate — but the denser rig would **not** clear 8x if the gate were applied to it,
  and the predicted win is correspondingly **about 6.3x, not 15.8x** (§7, Rev 6 note). Nothing is
  loosened to accommodate that: §2's `<= 250 000 ns` still clears with 3.2x margin on that rig.
* **Neither rig is a true dense-scene sample.** Both draw from the *same* collinear point set — the
  in-frustum rig calls the identical `light_position` formula (`lighting_l1_host_oracle.rs:134`) via
  `push_point_spot_lights` (`:159`) and differs **only** in pinning the placement-volume scale
  (`:193`). It fixes §1.4's **volume-growth** defect and **not** its **collinearity** defect.

---

### ERRATA + PROSE FREEZE (appended after Rev 6 was reviewed — this is the last prose revision)

Rev 6 came back **0 × P0** on the design and ~4 × P1, and **every one of those P1s sits at a line Rev 6
itself edited**, in the prose↔measurement synchronization layer. That is the fifth consecutive
occurrence of the pattern named at the top of this block, and it triggers the hard stop that was set
before Rev 6 was written.

**DECISION: the design surface is approved and the prose is FROZEN. Rev 7 will not be written.** The
residual precision moves out of prose and into code, where the compiler and the runtime are the oracle
instead of a reviewer's eye. Concretely: §7's table is pinned as literals in
`crates/boyko_rhi_vulkan/tests/lighting_l1_host_oracle.rs` under the same "MEASURED values — do not
edit these literals to make a failing run pass" discipline the occupancy table already uses, so a
drift becomes a **test failure** rather than a review finding. The `NMax(NaN,NaN)` question, the
e5/e6 selection counts and mutation (vii)'s observable-effect assertion likewise become runtime
assertions. Corrections below are recorded here as errata; the sections they correct are NOT rewritten.

**Errata (measured; each verified twice independently):**

1. **`N = 64` is present and measured.** §7 and the Rev 6 changelog state that `HIER_MATRIX_N` "dropped
   `N = 64`" and that §7's break-even is "unsupported pending an N=64 row". Both are stale — the row
   was restored and measured in the same session. §7's `N = 64` cell reads "not measured"; it is
   measured: coarse 1536, fine 20 304, hier 21 840.
2. **§7's `N = 128` row is off by one in both measured columns.** It prints fine **18 719** / hier
   **21 791**; the measured values are **18 720 / 21 792**. (`N = 8` and `N = 512` are exact.)
3. **Two new samples close the break-even.** `N = 16`: coarse 384, fine 6768, hier 7152. `N = 20`:
   coarse 480, fine 8208, hier 8688. Against §7's own constants the froxel arm loses by ~0.72 µs at
   `N = 16` and wins by ~3.3 µs at `N = 20`, so the crossing is **N ≈ 16.7** — comfortably inside §2's
   `<= 40` gate, and no longer an interpolation.
4. **`N = 1022` hier is 78 528, not 78 530.**
5. **§8.10 item 5b's requirement (b) prose does not describe what makes it work.** Its measured value
   on the `inject_nan_froxel = None` mirror is `group_coarse_accept[0] == 0` — group 0's coarse box
   rejects **all** 128 lights, and `< ps_n` admits that fully-degenerate cell. Mutation (vii) stays
   detectable (coarse 0 against a flat cell holding all 128 ⇒ RED), but for a different reason than
   "rejects ≥ 1" states.

**One open P0 against this document, to be discharged in code rather than prose:** §8.9's structural
checks **e5** and **e6** select by "a `NoContraction`-decorated `OpFAdd`" / "`NoContraction`-decorated
`OpFSub` only". The committed base module has **`NoContraction` = 0** (measured), so both selectors
select the empty set and both quantifications are **vacuously true** — they would go green on an
arbitrarily divergent module. Since e5/e6 are exactly the detectors D4's byte-identity premise rests
on under `-O3` independent inlining, each must additionally **pre-register a non-empty selection
count** (e5: exactly 2 windows; e6: exactly 2 decorated `OpFSub`; an empty selection is RED). This
applies under either compile option, not just the one that re-pins the base.

**Rung status at the freeze.** H0 shipped (`0597531`). HP shipped (`34258c2`). H1 is implemented and
green. **H2 was correctly NOT built**: the developer stopped and escalated because D5/§8.1/§8.9 make
**H1.6** — the single re-pin of the base `.spv` to the `NoContraction` construction, census
**12 616 B / 7 `NoContraction` / 8 `OpDot`** against the repo's current **12 392 B / 0 / 9** — a real,
unmet prerequisite. That escalation is **sustained**: H1.6 runs as its own rung against §8.8's gate
(zero golden-move budget, 3-run-before/3-run-after perf gate), and must **not** be folded into H2,
because a zero-move budget and a perf gate are precisely what get waved through when bundled.

### Rev 6 changelog

| # | Sev | What was wrong in Rev 5 | What Rev 6 does |
|---|---|---|---|
| 1 | P1 | §8.10 item 3 pre-registered mutation (v)'s RED evidence against **assertions 1 and 2** — the detector labels **A1/A2** misread as table rows. Assertion 1 is `alloc_total < index_list_cap`, a precondition (v) **cannot** falsify, so a reviewer discharges `[P0-B]` off assertion 2 while **assertion 6 — the guard tail, the detector that exists because of the Rev 3 P0 — goes unchecked** | The numbers become **assertion 5** (138 in-range sentinel cells) and **assertion 6** (138 cleared guard-tail cells) at §8.10 item 3, §9 `[P0-B]` and this changelog's Rev 5 row 4. A **CONTROL ARM** is pre-registered: the BASE arm on that config clears **16** tail cells by itself (boot-sourced `ceil(3312/64) = 52` groups = 3328 threads, `passes/vb.rs:215`, against its live-dims guard at `cluster_cull.hlsl:109-114`), so an assertion-6 firing **16** times means the mutation did nothing |
| 2 | P1 | The per-arm readback protocol named **two** buffers and forbade "any assertion reading a buffer both arms wrote" — which makes assertions **3** and **8** unimplementable, since both arms write `LightIndexList`. Assertion **8** was in **no** evaluation class at all. Result: false RED on a correct pair (the 64- and 256-wide dispatches claim different `InterlockedAdd` offsets, `cluster_cull.hlsl:183`) and false GREEN on detector (C), `[P0-4b']`'s sole discharge | **Three** buffers are named; `LightIndexList` is read back **per arm**, after that arm's dispatch and before the next arm's; assertion 8 joins the **per-arm** class; the rule is restated executably: *every assertion is evaluated on readbacks captured after exactly one arm's dispatch, and no assertion may mix one arm's `ClusterGrid` offsets with the other arm's `LightIndexList`* (§8.2(A), §8.10) |
| 3 | P1 | Mutation (vii)'s rig precondition failed in **all three** of its readings: **unsatisfiable** (with the injection mitigated, group 0 rejects **zero**, so every GREEN run reports INVALID), **insufficient** (at `N = 512` the per-froxel clamp at `cluster_cull.hlsl:170` lets both arms emit the identical 256-index prefix, so the RED arm passes and "rig-independence needs only rejects >= 1" is false), and it **cited a quantity that does not exist** (H1 exposes `groups`/`ps_n`/`valid_lanes`/`pairs_coarse`/`pairs_fine`, no per-group coarse-accept count) | The (vii) run is **pinned at `N = 128`** (`ps_n < MAX_LIGHTS_PER_CLUSTER = 256`, `light.rs:53`), which makes "coarse rejects >= 1 ⇒ assertion 2 RED" a **theorem**; the precondition's source is the **`inject_nan_froxel = None`** mirror, and its conservatism is *derived* (the un-injected box contains the unmitigated box, so §5 Step 2's monotonicity carries the implication one way only); the **injection point** is pinned **after phase 0's AABB build, before phase 1's finiteness test**; **§8.6 gains deliverable 8**, the per-group coarse-accept count |
| 4 | P1 | §5 Case B's step "`F(d) = 0 <= r·r` **for every `r`**" is **false for a reachable input**: at `r·r = NaN` the **ordered** `OpFOrdLessThanEqual` makes `0.0 <= NaN` FALSE and the coarse test **rejects**. `L.range` is unvalidated (`light.rs:884-885`, `:1008`, `:1012`) and the repo already ships a non-finite `.w` in that lane (`:994`). §5.2's "exactly three named premises" under-counted, since Premise F named only `L.pos` | The step is scoped to "every `r` whose `r·r` is not NaN" plus the NaN case written out — **both levels reject, so the implication holds DIRECTLY rather than vacuously and byte-identity is preserved** (the flat arm emits nothing for that light either). The Corollary is amended; **Premise F is widened** to `L.pos` finite **AND** `L.range` not NaN, with the premise count restated as a conjunction. **A proof-text defect, not an algorithm defect** — no constant, branch or gate moves |
| 5 | P1 | §7's fine column was labelled "the number H1 replaces with a measurement", but §8.7's re-derivation and §10 ABORT clause 2 both **execute** the literal `32 300`. H1 measured: `N=8` → **3 648**, `N=128` → **21 791** (plan **2.63x LOW**), `N=512` → **45 840** (plan **1.42x LOW**). §7's break-even (`N ~ 17–19`) interpolates between `N = 8` and `N = 64`, but `HIER_MATRIX_N` **dropped `N = 64`** and has **no sample in 16–20** | §7's table carries the **measured** fine/hier columns (with Rev 4's model kept beside them so the sign of the error stays visible); ABORT clause 2 and §8.7's recipe execute **45 840**; the break-even interpolation is declared **unsupported pending an `N = 64` and a 16–20 sample**. The corrected numbers **still clear §2** — `13 939 + 0.2736 x 45 840 = 26.5 us` (kronecker), `78.9 us` (in-frustum) against the `250 000 ns` gate — **and no gate is loosened** |
| P2-1 | P2 | §8.3 asserted `NMax(NMax(NaN,NaN),0) = 0` as fact where §5.1 calls the both-NaN case **undefined**; H3 assertion 4's (vii) row rides on it | Closed by a **one-dispatch device probe** (one froxel, one light, all-NaN AABB, assert `sq_dist == 0.0`), per this plan's own measured-not-argued standard. Failure direction is false-RED, i.e. safe |
| P2-2 | P2 | §5.1's fold enumeration still listed **two** stored values; Rev 5 added a **third** (`±FLT_MAX`) in the same pass | The enumeration lists all three. `FLT_MAX` is finite, so the conclusion is untouched |
| P2-3 | P2 | §9's `[P0-4b]` chain printed `== MAX_LIGHTS` on a groupshared-**WRITE** bound row, where `MAX_LIGHTS` carries the device **READ** bound | Trimmed to `HIER_MASK_WORDS*32 == 1024`, with `MAX_LIGHTS`'s role (D6's equality pin) named separately |
| P2-4 | P2 | §8.3's saturation check said froxel 168 "claims `N` indices" three lines above "clamped by 256" | `min(N, max_lights_per_cluster)`, with the pinned `N = 128` case spelled out |
| P2-5 | P2 | Rev 5's P2-3 insertion landed **inside** the HLSL code fence, between `load_light_header` and `ps_begin` | Moved out verbatim, below the fence |
| P2-6 | P2 | Rev 5's changelog row 2 cites "§5 `:1592-1595`", which is **§4 prose** in the file it shipped in — a Rev-4-relative, unlabelled line citation | Re-cited by **section and construct** rather than by line number, which is also why Rev 6 adds no new doc-internal line citations |

---

*Rev 5's own header text, retained verbatim — it carries the Rev 4 review provenance:*

> Rev 4 was reviewed by three independent adversarial lenses and consolidated to **0 × P0, 6 × P1** and
> a P2 list. One lens — the **out-of-bounds/UB lens, the class that already shipped a real GPU-UB bug in
> this campaign (VB-P1b C1)** — returned **APPROVED outright**, after splicing §4's HIER arm into the
> real `cluster_cull.hlsl`, compiling it under the frozen recipe, and reproducing every row of §8.3's
> simulation exactly.
>
> **Rev 5 is a bounded edit pass, not a revision of the design.** It fixes exactly those six P1s plus
> ten P2s and changes nothing else. §4, §5 Steps 0–2, §D3, §D7, §D8–§D11, §8.2's guard-tail derivation
> and §7's model were independently re-derived and reproduced by two lenses and are **not reopened** —
> except at the three points a P1 names by line: §4/§D8's absorbing *constant*, §5 Case B's one algebra
> step, and §8.2's fourth honest limit.

### Rev 5 changelog (retained; two rows corrected inline by Rev 6)

| # | Sev | What was wrong in Rev 4 | What Rev 5 does |
|---|---|---|---|
| 1 | P1 | §8.3's mutation **(vii)** was broken in **both** arms: its detector (H3 assertions 2/3) is an **arm-vs-arm** comparison but the injection was one-sided, so the GREEN arm cannot be green regardless of the mitigation; `lane == 7` names **disjoint froxel sets** in a 64-wide and a 256-wide module; and the stated RED mechanism does not follow (`coarse_min.x` is the min over 144 froxels and froxel `x=0` supplies it, and with only `aabb_min.x` poisoned `d` still rejects on y/z/+x) | (vii) is re-specified on a **froxel-identity** predicate (`fi == 168u`), poisons **all six** AABB components, and is **mirrored in the HIER module, the base module and the host mirror**. The GREEN/RED derivation is written out and is **rig-independent**, with an explicit rig requirement in (ii)/(iii)'s form (§8.3, §8.10 item 5) |
| 2 | P1 | The absorbing constant `±1e30` **inverts enclosure for any finite light centre with `\|c_j\| > 1e30`** — the coarse level rejects (`c_j − 1e30 > 0` ⇒ `F = inf`) while the poisoned lane's own all-NaN fine test gives `F = 0` and accepts. ~~§5 `:1592-1595`~~ **[Rev 6 P2-6: the correct citation is §5 Step 3's *Case B* bullet and its Rev 5 note — `:1592-1595` was Rev-4-relative and unlabelled, and lands in §4 prose in the file Rev 5 shipped]**, §5's "unconditional", and D4 clause (c)'s deletion all rest on that step | The absorbing element becomes **`±FLT_MAX`** (§4, §D8, §5 Case B), which still absorbs against the `±1e30` identity and holds for **every** finite centre. The `1e30` **finiteness threshold** is a different constant with a different job and is **deliberately left alone** — §5 says so explicitly so a later reader does not "unify" them. The one surviving premise — the light centre must be finite — is named (**Premise F**, §5.2) instead of being left implicit |
| 3 | P1 | §9's `[P0-4b]` row claimed mutation (iv) can make `(j>>5)` exceed 31. Arithmetically impossible: (iv) loops `j < HIER_MASK_BITS == 32*32 == 1024`, so `j>>5 ≤ 31` — in range for `gs_mask[32]`. The plan proves this itself at D7 `:843-845` and routes (iv) to detector (C) at both `:2046` and D7 | The row answers **NO**, and names what carries the bound instead: `ps_n ≤ ps_room ≤ HIER_MASK_BITS == MAX_LIGHTS == HIER_MASK_WORDS*32`, with `MAX_LIGHTS = 1024` pinned at `crates/boyko_render/src/light.rs:51`, carried by D6's equality assert plus H2(f)'s `#error`. Precedent for a **NO** in that column already exists in four other rows |
| 4 | P1 | H3 assertion 10 forbids post-boot `ClusterConfig` edits, but mutation (v) **requires** boot 16×9×23 with live 16×9×24, and the RED-if blankets all ten assertions on every mutation run — so the (v) run aborts at the precondition and `[P0-B]`'s only discharge had **no executable gate** | An explicit **`allow_skew` driver flag** scopes assertion 10 to the equality/totality runs; the (v) run records ~~assertion 1's~~ **assertion 5's** 138 gaps and ~~assertion 2's~~ **assertion 6's** 138 cleared tail cells **individually** (§8.10 item 3, §9 `[P0-B]`) — **[Rev 6 P1: Rev 5 wrote "assertion 1 / assertion 2", reading the detector labels A1/A2 as table rows; assertion 1 is a precondition (v) cannot falsify, so that wording left assertion 6 unchecked]** |
| 5 | P1 | §8.2(A)'s pre-fill was bound to the **allocation**, but the matrix is **arm-vs-arm** — two dispatches per config sharing one buffer. Mutation (vi) at E2 then leaves 6144 in-range cells holding the *base* arm's correct result (assertions 2/3/4/5 all false-GREEN), and `alloc_total == capacity` false-REDs on whichever arm runs second | A **fourth honest limit**: the driver re-fills/zeroes **immediately before each arm's dispatch** and reads back **before** the next fill; per-arm assertions are evaluated on that arm's own readback, comparison assertions on two separately captured readbacks, and **no assertion may read a buffer both arms wrote** (§8.2(A), §8.10) |
| 6 | P1 | H0's deliverable "print the device's SM count from `VkPhysicalDeviceProperties`" is unimplementable — core `VkPhysicalDeviceProperties` exposes no SM count (it needs `VK_NV_shader_sm_builtins::shaderSMCount`, which this device never enables), and `grep -rniE "sm_count\|shaderSMCount\|multiprocessor\|SHADER_SM_BUILTINS" crates/` returns **zero** hits | The bullet is **deleted**; §D3's 28 SMs is relabelled an **owner-stated device fact** everywhere it appears (§D3, §7.0), and the occupancy prose no longer reads as if it were measured |
| P2-1 | P2 | Three §7/§D3 numbers do not reproduce — all **conservative** (they make the design look worse) and no gate consumes them | `≈ 51 µs` / `≈ 50 µs` → **31.6 µs** (a 15.8× win, not 10×); break-even at the 2×-pessimistic rate `≈ 25–30` → **19**; `q = 1.295563` → **1.2955587** (and `q²`, `q³` with it). The corrections move the prediction **in the design's favour**, which is a reason for more caution, not less: no gate is loosened |
| P2-2 | P2 | Four citation slips | `g³ 0.236067977` → **`0.236_067_977_5`** (this matters: the printed value violates §1.4's own collinearity identity 1023/1024 times, the real literal gives the claimed 0/1024); `light_table.hlsli:317-319` → **`:318-320`**; `sdf_gbuffer_hybrid.rs:5415-5425` → **`:5415-5426`**; §P1-3/e5's SPIR-V ids are **dropped** rather than re-quoted (the selector itself is unaffected — see the note there) |
| P2-3 | P2 | `HIER_TPG` had no `#error` pin though D9's radix-16 fold hardcodes `16`/`256u` | A third `#error` guard, in both places the guard block appears (§4, §D7) |
| P2-4 | P2 | D11's `self.dim_x.mul(self.dim_y)` will not compile in a stable `const fn` | Replaced by the arithmetic form, which also **mirrors the shader's `gps` token-for-token** |
| P2-5 | P2 | H2 check e6 was stated unscoped in two of the three places it appears; the module carries ~24 undecorated `OpFSub` in ray-gen whose operands *are* `OpFMul` results, so a naive form false-REDs | e6 is scoped everywhere to **`NoContraction`-decorated** `OpFSub` only, with the false-RED named |
| P2-6 | P2 | Mutation (iv)'s "no-op on the mask-capacity-boundary config" precondition lived only in D7 prose, not in the protocol a reviewer actually executes | Added as §8.10 protocol item 6 |
| P2-7 | P2 | `vb_p1d_cull_shade_bench.rs:47` still documents the pre-split print line | Noted as a one-line follow-up **in the H0 rung text**; Rev 5 does not edit that file |
| P2-8 | P2 | The permutation probe ran only where the map is degenerate (`gps = 1`) | Also run at **E3 (16×17×24)** — the only `gps ≥ 2` config with `G > 0` — so exactly-once is device-measured where the map is non-degenerate |
| P2-9 | P2 | Rev 4's `vb.rs` line citations are exact against `git show HEAD:` but go stale the instant H0 commits | Re-anchor note added to §8.5 and to the appendix's VB-record-site row |
| P2-10 | P2 | §12's provenance rung was Rev 4's own approval blocker, stated as prose | Kept as a blocker, with a **precise** acceptance criterion (file, function, what it asserts, what makes it red) — and it is being implemented in parallel right now |

---

*Everything below this line is Rev 4's text, retained. Rev 5 edits it only at the points the changelog
above names; where a Rev 4 claim was corrected, the correction is marked inline as "Rev 5".*

### What Rev 4 is mostly about: a gate layer that can actually go RED

Rev 3's P0 was not a bug in the algorithm. It was that **the safety property the design exists to
establish had no detector that could fail.** Simulated against §4's own thread map, dropping the
`valid` guard on phase 6 writes **2 688 cells (21 504 B) past the end of `ClusterGrid` while every
one of Rev 3's assertions stays GREEN**.

This is the **third** time in this campaign that a gate which cannot fail was shipped or nearly
shipped (VB-P1-0's "regression guards" ran through a host oracle that already masked the bug;
VB-P1b's C1 GPU-UB was masked by a golden that captures ~frame 30). Rev 4 therefore treats
*"what mutation turns this assertion red, and does it actually?"* as the **primary design question
for every assertion in §8/§9**, answered by simulation or by running the toolchain — never by
reading the assertion and finding it plausible.

Consequences, all of them stated as deletions or replacements rather than additions:

* **Three Rev 3 assertions are DELETED because no mutation turns them red** — "exactly one
  `OpReturn`", "every `OpControlBarrier` sits in a merge block", and the pair of §5/§8 claims that
  plain validation reports a `ClusterGrid` overrun. Each deletion is backed by a measurement below.
* **Two Rev 3 mutations were mis-specified and are replaced** — (v) was arithmetically inert and
  (vi) was pre-registered to stay green on a config where it in fact goes red. Both were simulated.
* **Three new detectors are added, each verified to fire**: a device-visible **guard tail** on
  `ClusterGrid`, a **permutation probe** that establishes exactly-once with no shader change, and a
  **poison tail** on the light table that turns an out-of-range light read into a detectable index
  instead of UB.

### Rev 4's own measurement log (what was actually run, and where)

Every experiment ran under the frozen recipe `-spirv -T cs_6_0 -E main -fspv-target-env=vulkan1.3`
(no `-D`, no `-O`) with `dxc 1.4.350.0` / `spirv-dis` from `C:/VulkanSDK/1.4.350.0/Bin`, in a
scratch directory. **No committed `.spv` was written.**

| # | What was run | Result |
|---|---|---|
| M1 | Recompile unmodified `cluster_cull.hlsl` under the frozen recipe | **byte-identical** to the committed `cluster_cull.comp.spv` (12 392 B) — the gate is reproducible outside the repo |
| M2 | Ext-inst histogram of the committed module | `NMax` **18**, `NMin` **8**, `Normalize` 4, `UMax` 2, `Pow` 2, `UMin` 1; **`FMin` = 0, `FMax` = 0** |
| M3 | `precise float sd` only (D10 as Rev 3 wrote it) | 12 592 B, **5** `NoContraction` (3 `OpFMul` + 2 `OpFAdd`), `OpDot` 8 |
| M4 | `precise float3 d` **and** `precise float sd` | 12 616 B, **7** `NoContraction` (adds the 2 `OpFSub`), `OpDot` 8 — **no leak into the AABB construction**; Rev 3's "16 ops" does **not** reproduce |
| M5 | Ext-inst histogram of M3 and M4 | identical to M2 — `precise` does **not** turn `NMin`/`NMax` into `FMin`/`FMax` |
| M6 | Two call sites of the M4 form | **14** `NoContraction` from the two bodies (**7 per call site**) — plus 2 more when an *argument is an expression* (`aabb_min * 1.5`), i.e. `precise` back-propagates into argument expressions |
| M7 | Correct 256-lane probe (no early return, 3 top-level barriers) vs. a deliberately broken one (early `return` **and** a barrier under `if (lane < 16)`) | **both emit exactly 1 `OpReturn`** — DXC canonicalises to one exit block |
| M8 | Merge-block test on the same two probes | the **correct** shader's first barrier is **not** in a merge block (false RED) and **all three** of the **broken** shader's barriers **are** (false GREEN) — unsound in both directions |
| M9 | New **top-level-chain** test on three probes | correct → **GREEN** (3/3 barriers on the chain); early `return` alone → **RED** (0/3); early `return` + divergent barrier → **RED** (0/3) |
| M10 | Full mutation simulation against §4's thread map (§8.3) | every row of §8.3's table; the three Rev 3 blind spots reproduced exactly |
| M11 | `all(abs(v) <= 1.0e30)` lowering | `FAbs` + `OpFOrdLessThanEqual %v3bool` + `OpAll` per vector — the finiteness compares are **`%v3bool`**, distinguishable by result type from the `%bool` cull compare |

### The findings, and what Rev 4 does about each

**P0-1 — the out-of-bounds `ClusterGrid` write had no gate that could turn red. RESOLVED by
rebuilding §8/§9, not by strengthening prose.** Simulated (M10):

| H3 mutation | capacity | OOB writes | unwritten | dup | Rev 3 gate | Rev 4 gate |
|---|---|---|---|---|---|---|
| baseline 16×9×24 | 3456 | 0 | 0 | 0 | green (correct) | green (correct) |
| (i) drop `valid` on phase 6 | 3456 | **2688** (`fi` 3456–6143) | 0 | 0 | **GREEN** | **RED** (guard tail: all 2688 tail cells lose the sentinel) |
| (v) as Rev 3 wrote it | 3312 | 0 (max `fi` 3311) | 0 | 0 | **GREEN** | **inert — mutation re-specified** |
| (v) re-specified (§8.3) | 3312 | **138** | **138** | 0 | — | **RED twice** (guard tail *and* totality) |

Four changes, all of them in §8.3 / §8 H3 / §9:

1. **(v) is re-specified** as *"delete `fi < capacity` **and** re-source `bdx/bdy/bdz` from
   `load_cluster_params(LightBuf)`"* — the state D11 exists to prevent. Rev 3's form was
   arithmetically inert and the plan **proved that itself two sections away** ("the first two already
   imply `fi < bdx·bdy·bdz` algebraically"; and with boot-sourced dims a live-`ClusterConfig` edit
   "cannot move `fi` at all"), while §9 still answered "can it actually fail? — yes, mutations (v),
   (vi)". Both statements cannot be true; the mutation was the wrong one.
2. **A device-visible guard tail.** H3's driver allocates `capacity + G` cells with
   `G = (HIER_TPG·gps − dim_x·dim_y)·dim_z`, pre-fills **all** of them with `0xFFFFFFFF`, and asserts
   the tail is intact after the dispatch. `G` is not a safety margin — it is **exactly the image of
   the invalid lanes** under §4's map (proved and simulated in §8.2), so mutation (i) clears
   precisely the tail and nothing else. **No validation layer is involved.**
3. **A uniqueness check that needs no shader change** — the *permutation probe* (§8.2): one
   frustum-covering light makes every froxel claim exactly one `LightIndexList` slot, so the
   `offset` values in `ClusterGrid` must be a permutation of `[0, capacity)`. A duplicate in-range
   write loses an offset and the permutation breaks.
4. **Assertion 5 is restated as TOTALITY (at-least-once) only.** Rev 3 wrote that the sentinel proves
   "every froxel was written exactly once"; it does not. Exactly-once is now *derived* in §8.2 from
   totality + tail integrity + the CPU-pinned lane count, and *separately measured* by the
   permutation probe. **Plain validation is no longer cited as an overrun detector anywhere** —
   `crates/boyko_rhi_vulkan/src/device.rs:2087` enables only
   `VK_VALIDATION_FEATURE_ENABLE_SYNCHRONIZATION_VALIDATION_EXT`, a repo-wide grep for
   `GPU_ASSISTED` / `debug_printf` returns **zero** hits, and `robustBufferAccess` is off.

**P1-1 — §5's NaN analysis argued against instructions this shader never emits. §5 is rewritten and
the mitigation is REPLACED, not merely re-justified.** Measured (M2, M5): `FMin` 0, `FMax` 0,
`NMin` **8**, `NMax` **18**. `NMin`/`NMax` return the *non-NaN* operand, so:

* `max(max(NaN, NaN), 0.0)` reaches an outer `NMax` against a non-NaN `0.0` and yields `0.0` ⇒
  `F(d) = 0 ≤ r·r` ⇒ the test **accepts every light**. Rev 3's stated alternative ("`NaN <= r*r` is
  false so the group rejects everything") is **unreachable**, and its claimed order-dependence is a
  property of `FMin`/`FMax`, which are not emitted.
* A single NaN lane is **already dropped from the fold** by `NMin`/`NMax`. So Rev 3's
  "blast radius is one group (144 froxels)" — the entire basis on which it declared Rev 2's
  assessment FALSE — **is not established**, and that declaration is withdrawn.
* Rev 3's mitigation (substitute the `min`/`max` **identity**) is therefore a **no-op in the mixed
  case** (the NaN was already dropped) and **actively harmful in the all-NaN case**: unmitigated it
  matches the flat arm (both accept everything); mitigated it rejects every light for all 144
  froxels. It also does not repair the one place enclosure genuinely fails — the NaN lane's **own
  fine test**, which is on the fine side, not the coarse side.
* **Rev 4's replacement: the ABSORBING element, not the identity.** *(Rev 5 corrects the constant to
  `±FLT_MAX`; Rev 4 wrote `±1e30`, which inverts enclosure for a finite centre with `|c| > 1e30` —
  §5 Case B.)* A `valid` lane whose AABB is
  non-finite stores `(−FLT_MAX, +FLT_MAX)` — which *wins* every `NMin`/`NMax` — so the group's coarse box
  becomes the whole universe, the coarse mask becomes the full punctual set, and **every froxel in
  that group degrades to exactly the flat arm's walk**. `!valid` lanes keep the identity
  `(+1e30, −1e30)`. Two constants, two conditions, one review-gate item. This makes §5's claim true
  by construction *and* **lets D4 scope clause (c) be deleted** — byte-identity now holds
  unconditionally in the AABB's finiteness (§5, §D8).
* **The `valid`-vs-`contrib` inconsistency is gone**, because there is no longer a second predicate:
  phases 1, 5 and 6 all gate on `valid`, and finiteness selects *which constant a lane stores*, never
  *whether a lane participates*.
* §5's "unconditional" is **re-earned rather than asserted**: §8's mutation (vii) is now a
  **two-sided** fault-injection test — the run must be GREEN with the finiteness substitution present
  and RED with it deleted. Rev 3 never tested that the mitigation did anything. *(Rev 5 re-specifies
  the injection itself: **froxel identity `fi == 168u`, all six AABB components, mirrored in the HIER
  module, the base module AND the host mirror**. Rev 4's one-arm single-component `lane == 7` form
  could not go green in the GREEN arm — the detector is an arm-vs-arm comparison — and `lane == 7`
  names disjoint froxel sets in a 64-wide and a 256-wide module. §8.3, "Mutation (vii) in full".)*

**P1-2 — which `precise` placement ships. DECIDED, with the census.** Measured (M3/M4/M6):
Rev 3's `precise float sd` emits **5** decorations; `precise float3 d` + `precise float sd` emits
**7** (adding the two `OpFSub`), 12 616 B, **with no leak into the AABB construction**. Rev 3's
"16 ops including the AABB construction" does **not** reproduce and is withdrawn. **Rev 4 ships the
7-decoration form**, because §5 Step 2's monotonicity chain *traverses* those two `OpFSub` and the
5-decoration form leaves a residual shader-structure-dependent argument in the proof (§D10). The
per-node audit of the chain is written out in §5 Step 2.

**P1-3 — H2(e)'s two control-flow assertions are DELETED and replaced by one that discriminates.**
Measured (M7/M8/M9): "exactly one `OpReturn`" is **vacuous** (the correct probe, the early-return
probe and the fully-broken probe all emit exactly 1); "every `OpControlBarrier` sits in a merge
block" **false-REDs a correct shader and false-GREENs a broken one**. Replacement: **every
`OpControlBarrier` lies on the entry function's top-level block chain** (§8 H2(e)) — verified GREEN
on the correct shape and RED on an isolated early `return`, which is D8's named "single most likely
implementation bug". The `OpFOrdLessThanEqual` assertion is also re-posed: it is split by **result
type** (`%bool` for the cull compares, `%v3bool` for §5's finiteness compares, M11), the window is
selected by *"scalar `OpFOrdLessThanEqual` whose first operand is a `NoContraction`-decorated
`OpFAdd`"* (verified M4: the base module's scalar compare has an `OpFAdd` first operand and an
`OpFMul` second — *Rev 5 drops the concrete SPIR-V ids Rev 4 printed here; they were mis-transcribed,
no reviewer can re-verify an id without re-running dxc, and the selector does not depend on them*),
and a **producer assertion on the two `NoContraction`-DECORATED `OpFSub`** is added, because an
id-normalised window erases operand provenance and cannot detect a Premise-P violation. *(The
decorated-only scope is mandatory: ~24 undecorated `OpFSub` in ray-gen do have `OpFMul` operands.)*

**P1-4 — mutations (vi) and (iv) are replaced.** Simulated (M10): Rev 3's transposed form
`slice = gid%gps; s = (gid/gps)·256+lane` goes **RED at 16×9×24** (144 of 3456 cells written), not
green as pre-registered, and at E2 it fails by **coverage** (1024 of 12288, max `fi` 12265, **0
OOB**), not by "driving `fi` far out of range". Both halves of Rev 3's pre-registration are
falsified, so **P1-F's discharge no longer rests on it**. The genuinely `gps`-degenerate form is
**`slice = gid; s = lane`** — bit-identical at 16×9×24 (correctly GREEN) and RED by coverage at E2
(6144/12288), E3 (6144/6528) and E4 (6144/18432). Mutation (iv) could not produce an out-of-range
read as written (phase 4 sets bits only for `j < ps_n`); it is replaced by a **producer** mutation
plus a **light-table poison tail** that converts the resulting read into a detectable index rather
than UB (§8.3).

**P1-5 — two false claims corrected.** (a) "Three evaluations of one `u32`" was not literal: the
allocation uses full-precision `cluster_config.cluster_count()`
(`crates/boyko_app/src/gpu_scene/mod.rs:4317`, `crates/boyko_render/src/light.rs:728`) while the
shader re-derived `capacity` from three **8-bit** fields whose `≤ 255` contract is a `debug_assert!`
only (`light.rs:763-769`) with no masking on the OR. **Fix: `cluster_capacity: u32` becomes a second
HIER push word**, minted from the same `cluster_count()` binding that sizes the buffer (push 20 → 24
B, against a shared COMPUTE range of **80 B**, `compute.rs:2956`), *and* the `≤ 255` contract is
promoted to a release `assert!` in `build_froxel_light_cull` — the push word makes the write bound
safe even out of contract, the assert keeps the *mapping* honest. (b) "The HIER arm cannot fault …
only mis-shape the grid" is true of the cull's own writes and **false at frame level**: all four
`ClusterGrid` consumers index with the **live** header dims behind only a non-zero test —
`vb_resolve.comp.hlsl:359`, `vb_shade.comp.hlsl:527`, `deferred_pbr.hlsl:1237`,
`forward_opaque.fs.hlsl:333` — so live-dims-*grow* skew is an out-of-range `StructuredBuffer` read
with `robustBufferAccess` off. **Pre-existing, not introduced here** (the repo names the class at
`vb_resolve.comp.hlsl:343-348` and `plugins.rs:355-361`), but the false framing is exactly what let
**VB-P1k** be filed as "Owner/VALUES call, not a safety requirement". The claim is scoped in §D11 and
**VB-P1k is re-filed as a safety follow-up** (§11).

**P2 (tracked, not gating) — all applied.** §2's zero-tolerance `N_ps=64` row now reads §10's 10 %;
H1.6's prose gate is given a numeric form and a **stated golden-move budget of zero**; ABORT clause 2
is expressed in H1.5's own ±25 %-of-`0.2736 ns/pair` terms; §5's Setup now **states** that
`coarse_min`/`coarse_max` must be group-uniform and D8 carries it as a review item; §5's ±0 bullet
and D9's "exactly the componentwise extremum" are reconciled; H2(f) is made **mechanical** (compile a
scratch copy with `HIER_MASK_WORDS 64u` and assert dxc *fails*) instead of "executed once during
review"; H1 assertion 7 is labelled a Rust re-implementation, not a pin on the HLSL; the citation
slips are fixed (`sdf_gbuffer_hybrid.rs:5415-5426` — *Rev 4 wrote `:5415-5425`, one line short of the
`write_words(mapped, &[0u32])` it names; corrected in Rev 5* — is the host **zero-write-before-submit**, the
post-fence mapped reads are at `:6202-6211` and `:6219-6228`; `INDEX_LIST_CAP` is `light.rs:61`, not
`:57`); and D3's non-reproducing arithmetic is **withdrawn rather than re-fitted** (§D3).

### What Rev 3 discharged, and Rev 4 PRESERVES intact

* **P0-A — §5's proof no longer needs the premise it used to disclaim. DISCHARGED by removing the
  ambiguity, not by budgeting for it.** `dot(d,d)` lowers to a single `OpDot`, and Vulkan's
  *Precision and Operation of SPIR-V Instructions* specifies `OpDot` only as **"inherited from"** a
  formula, with the same appendix permitting that formula to *"be transformed using the mathematical
  associativity, commutativity, and distributivity of the operators involved"*. Two `OpDot`s in one
  module may therefore be lowered to different summation orders — and a census over 9 modules emitted
  by the pinned dxc 1.4.350.0 found **zero `Fma`** in every one, so contraction is decided by the
  driver *below* the `.spv`, where no byte gate and no `spirv-dis` gate can observe it. **Fix
  (D10):** the one shared `sq_dist_point_aabb` computes a written-out, `precise` sum whose every node
  is specified as **"Correctly rounded"** and carries `NoContraction`. Both call sites then evaluate
  *one function `F`*, and the missing link `A(d_fine) ≤ B(d_fine)` becomes vacuous because
  `A ≡ B ≡ F`. Re-verified in Rev 4 (M1, M4). A `spirv-dis` structural gate is retained in H2 as a
  **tripwire, never as the proof**.
* **P0-B — the lost total bound. DISCHARGED, and the naive fix was measured to be insufficient.**
  Transplanting the base arm's `fi < cluster_count` (live header) into D3's re-derived mapping still
  writes `fi = 13 807` into a **3 456-cell** `ClusterGrid`. **Fix (D11):** the BOOT grid dims *and*
  (new in Rev 4) the boot capacity travel in `#ifdef HIER`-only push tail words; the dispatch size,
  the `ClusterGrid` allocation and the in-shader write bound become **three evaluations of one u32**
  minted in `build_froxel_light_cull`, and `valid` gains a three-term predicate. Two hazards D8's "no
  early return" newly created are also closed: an integer **divide-by-zero** on a degenerate
  `packed_dims == 0` header and a `% dim_x` by zero.
* **P0-C — the unclamped coarse mask write and the index-space disagreement. DISCHARGED.** The mask
  is **l0a-relative**; a single group-uniform clamp `ps_n = min(ps_total, ps_room)` bounds the
  groupshared **write** and both device **reads** simultaneously, with no device value in the
  derivation. D6's defensive tail is **deleted as unfixable**. The host pin is an **equality**
  (`MAX_LIGHTS == HIER_MASK_WORDS * 32`).
* **P1-E — H0's framegraph access. DISCHARGED by DELETING it.** The `LightIndexAlloc[0]` readback
  leaves the present path entirely; §9's "no new GPU-visible resource" claim is *true* instead of
  *defended*.
* **P1-F — the test matrix was blind to its own mapping.** Every config Rev 2 named is 16×9×24, so
  `gps = 1` and D3's map degenerates. Four new grid entries (E1–E4), two of which must run on device.
  *(Rev 4 keeps the matrix and replaces the mutation that demonstrates the blindness — see P1-4.)*
* **P1-G — the barrier count is 3, not "three-to-eight" and not 11.** A **radix-16 in-place
  reduction** (2 barriers) plus folding the summary bit into phase 3's atomic. Barrier elision by
  wave size is **not** available and is not assumed (`device.rs:2584`).
* **P1-H — H1's overclaim withdrawn.** H1 falsifies the **pair-count** premise — a *necessary*
  condition — and nothing more. **H1.5** bounds thread-count scaling on the existing flat arm.
* **Arithmetic.** Rev 3's seven corrections stand, except the two §D3 rows P1-4/P2 falsified, which
  Rev 4 **withdraws** rather than re-fits (§D3).

### What remains open after Rev 4

1. **Nothing here has run on a GPU.** Every P0/P1 resolution is a compile-time, disassembly,
   CPU-simulation or arithmetic result. Device claims are deferred to H1.5/H3/H4, each with a named
   closing test.
2. **The D10 edit perturbs the base arm by ≤ 1–2 ULP and requires a one-time re-pin of
   `cluster_cull.comp.spv`.** Rung **H1.6** isolates it, now with a numeric gate and a zero-move
   golden budget.
3. **The ALU cost of D10 and of §5's finiteness substitution is unmeasured.** H1.6 and H4 measure
   them; the fallback if D10 regresses is named and its constant derived.
4. **The HIER module's decoration count (14) is a prediction, not a measurement.** M6 measured 14 on
   a two-call-site probe; the real HIER arm does not exist yet. H2 pins it.
5. **§1.3's provenance still does not exist in the repository** (verified again: `git ls-files` has
   no `cap_probe` / `occupancy_probe` match). Rev 4 turns §12's precondition into an **actionable
   rung, HP**, with files, assertions and a RED-if — it is no longer a paragraph of prose.
6. **The hot-group latency model is not written down.** §7 pre-registers what H4 must hit under each
   reading.
7. **A pre-existing latent hole in the BASE arm**, tracked as **VB-P1j** (§11).
8. **A pre-existing frame-level hole**: the four `ClusterGrid` consumers read with live dims and no
   capacity bound. Tracked as **VB-P1k**, now filed as a **safety** follow-up (§11).
9. **One term of `valid` is not independently falsifiable.** Simulated (M10): under D11's
   boot-sourced dims and the host-derived group count, deleting `slice < bdz` *or* deleting
   `fi < capacity` **alone** is inert — 0 OOB, 0 gaps. They go red only in combination with a second
   fault (§8.3). Rev 4 states this rather than pretending each term has its own mutation.

Sub-plan of
[VB-PERFORMANCE-TRACK.md](VB-PERFORMANCE-TRACK.md) §4 (VB-P1). Sibling of
[VB-P2-CLASSIFICATION-PLAN.md](VB-P2-CLASSIFICATION-PLAN.md). Base commit `dc0684e`
(`feat/multi-paradigm-render`).

**One-line verdict:** the cull is *pure rejection work* — at `N_ps=512` only **85 of 3456 froxels
(2.5 %)** hold any light, yet all 3456 threads scan all 512 lights and the pass costs **498 µs**. A
**single-dispatch, workgroup-local two-level cull** removes ~95 % of the (froxel, light) pair tests
with an **exactness proof that needs no floating-point epsilon**, and it introduces **no new GPU
buffer, no second dispatch, and no new framegraph resource**. Predicted cull at `N_ps=512`:
**≈ 23 µs (nominal) / ≈ 31.6 µs (2× pessimistic)**, break-even **≈ 17–19** (from the measured ≈ 103).
**Single-digit break-even is arithmetically impossible at the present fixed-cost floor** — §7.1
proves it across every fit and every point in the error bar. **The pair-count premise — a necessary
condition, and the campaign's cheap kill switch — is falsifiable on the CPU in 0.45 s at rung H1.
The sufficient condition (wall clock) is not: thread-count scaling is bounded at H1.5 with no new
shader, and the rest is settled only at H4.**

### Rung disposition (Rev 5)

| Rung | Start now? | Why |
|---|---|---|
| **HP** §1.3 provenance pin | **IN FLIGHT** | Being implemented in parallel, right now, as a real `#[test]` (§8.4, §12). Pure host, zero dependencies. Nothing else may be *approved* until it lands |
| **H0** timing bracket split | **IMPLEMENTED — awaiting GPU arbitration of a measurement defect** | The split landed (`CullReset` / `CullDispatch`, `VB_PASS_COUNT` 2→3, both pairs written outside their own `if let` gate per the HANG WARNING). **Defect:** `CullDispatch`'s begin is a `TOP_OF_PIPE` write placed *after* the `TRANSFER→COMPUTE` barrier, and a `TOP_OF_PIPE` timestamp is **not ordered** by that barrier — it may latch before the fill retires, so the split can **over-count `CullDispatch` by `t_fill`**. Being measured at `N=8` and `N=512` right now. **`N=512` alone structurally cannot detect it**: the fixed cost is **2.80 %** of the 498 µs cull there against **70.6 %** of the 19.7 µs cull at `N=8`, so only the low-`N` point has the dynamic range to expose the double-count |
| **H1** CPU oracle + selectivity + **permutation/exactly-once pin** | **UNBLOCKS THE MOMENT REV 5 LANDS** | Pure host arithmetic, and its coverage assertion is exactly the check that catches P0-1 on the CPU. It is gated on Rev 5 rather than Rev 4 because the host mirror must implement **the same absorbing constant** (`±FLT_MAX`, changelog 2) and **the same (vii) injection** (changelog 1); written against Rev 4 it would bake in both defects |
| **H1.5** transfer probe | **YES** | Existing flat arm, no new shader |
| **H1.6** `precise` + base re-pin | **Unblocked by Rev 4; gated on Rev 4's review** | P1-2 is settled by measurement (7-decoration form, M4/M6). It re-pins a *shipped* artifact, so it waits for a reviewer — not for another finding |
| **H2** `-D HIER=1` variant + dis-gate | **Unblocked by Rev 4; gated on Rev 4's review** | The dis-gate assertions are rebuilt and each was verified to discriminate (M7–M9, M11) |
| **H3** GPU equality oracle | **Unblocked by Rev 4; gated on Rev 4's review** | The mutation-and-gate layer is rebuilt and every mutation simulated (M10) |
| **H4 / H5** | **NO** | Sequentially gated on H3 |

---

## 1. The problem (measured, not assumed)

`crates/boyko_rhi_vulkan/shaders/cluster_cull.hlsl` dispatches **one thread per froxel**
(`[numthreads(64,1,1)]`, `:107`; the host dispatches `ceil(cluster_count / 64) = 54` groups,
`present/passes/vb.rs:215`). Each thread builds its world AABB from 8 corner unprojections
(`:126-153`) and then **linearly scans every point/spot light** (`:161-175`), testing
`sq_dist_point_aabb(L.pos, aabb_min, aabb_max) <= r*r` (`:102-105`). Cost is `O(froxels × lights)`.

VB-P1d measured it on RTX 3060 through GPU timestamps
(`crates/boyko_app/tests/vb_p1d_cull_shade_bench.rs`; the table is committed as the provenance
doc-comment on `CLUSTER_LO`, `crates/boyko_render/src/light_policy.rs:44-63`):

| `N_ps` | `flat_shade` ns | `froxel_cull` ns | `froxel_shade` ns | `froxel_total` ns |
|---|---|---|---|---|
| 8   | 32 799  | 19 741  | 27 075 | 46 816  |
| 32  | 60 815  | 42 253  | 29 747 | 71 999  |
| 64  | 95 877  | 72 748  | 29 973 | 102 720 |
| 128 | 167 322 | 134 920 | 28 119 | 163 039 |
| 256 | 315 044 | 252 154 | 25 508 | 277 662 |
| 512 | 592 015 | 498 067 | 25 303 | 523 370 |

Read it correctly: **`froxel_shade` is FLAT in `N` (25–30 µs) — the clustering payoff is already
fully realized in the shade.** The cull is 95 % of `froxel_total` at `N=512` and is the entire reason
the break-even sits at ≈ 103 instead of near zero. `CLUSTER_LO=64` / `CLUSTER_HI=128`
(`light_policy.rs:64,74`) is the measured hysteresis band.

### 1.1 A disproven hypothesis — do NOT repeat it

A prior attempt theorised the `uint local[256]` per-thread array (`cluster_cull.hlsl:159`) was
spilling to scratch and dominating. A two-pass count-then-write rewrite eliminating `local[]` was
implemented and measured: **cull 8 µs → 24 µs, total 498 µs → 1014 µs (2.04× REGRESSION)**, output
byte-identical. Reverted. That experiment is not just a dead end — **it is the strongest confirmation
of the cost model**: it doubled the number of (froxel, light) pair tests and the time went up 2.04×.

> **The cull's wall-clock is proportional to the number of (froxel, light) pair tests, and to
> nothing else we have been able to move.** Register/scratch pressure is refuted by measurement.
> The only lever with evidence behind it is *reducing the number of pairs tested.*

This is also the standing rule Rev 4 applies to *itself*: every number below that has not been
measured is labelled a model, and every rung that could have asserted a hypothesis instead measures
it (H0, H1.5, H1.6, H4). **Rev 4 extends the same rule to the gates**: an assertion whose failure
mode was not simulated or executed is not an assertion, it is a comment.

### 1.2 The empirical cost model (fitted, ≤ 8.9 % error over 6 samples)

```
cull_ns(N)       ≈ 13 939 + 0.2736 · (froxels · N)   = 13 939 + 945.6·N   at froxels = 3456
flat_shade_ns(N) ≈ 23 922 + 1 109.6·N
froxel_shade_ns  ≈ 26 500 ± 2 200   (no trend in N)
```

**The two fits are anchored differently. Stated so a reader with a calculator can reproduce them:**

* `flat_shade` is the exact **`N=8` ↔ `N=512` secant**: slope `(592 015 − 32 799)/504 = 1 109.5556`,
  intercept `32 799 − 8·1 109.5556 = 23 922.56`. It is *not* the 128/512 fit — only the `N=8` and
  `N=512` rows read 0 %.
* `cull` is **neither fit cleanly**. Its slope is the **128 ↔ 512** secant expressed per pair —
  `(498 067 − 134 920)/384 = 945.6953 ns/N`, i.e. `945.6953/3456 = 0.273638 ns/pair` — **rounded to
  `0.2736`**; the intercept is then re-fitted so `N=512` is exact *using the rounded rate*:
  `498 067 − 0.2736·3456·512 = 13 939.5`. The error table below evaluates at `945.6·N`.
* For reference, the exact 128/512 secants are `cull = 13 871 + 945.70·N` and
  `flat = 25 758 + 1 106.0·N`. **§7.1 is robust to either choice** (floor 14.89 vs 13.21).

| `N` | model cull | measured | err | model flat | measured | err |
|---|---|---|---|---|---|---|
| 8   | 21 504  | 19 741  | +8.9 % | 32 799  | 32 799  | 0 % |
| 32  | 44 198  | 42 253  | +4.6 % | 59 429  | 60 815  | −2.3 % |
| 64  | 74 457  | 72 748  | +2.3 % | 94 936  | 95 877  | −1.0 % |
| 128 | 134 976 | 134 920 | +0.04 % | 165 951 | 167 322 | −0.8 % |
| 256 | 256 013 | 252 154 | +1.5 % | 307 980 | 315 044 | −2.2 % |
| 512 | 498 086 | 498 067 | 0 % | 592 037 | 592 015 | 0 % |

Two constants matter for everything below:

* **`0.2736 ns` per (froxel, light) pair test** — the marginal cost.
* **`13.9 µs` fixed cost per cull invocation** — independent of `N`. It is *not* the 3456 AABB
  builds (8 unprojections × 3456 threads ≈ 0.8 Mflop ≈ 0.1 µs at peak). The `LightCull` timestamp
  bracket (`present/passes/vb.rs:140-247`) spans `cmd_fill_buffer(alloc)` → graph-derived
  `TRANSFER→COMPUTE` barrier → dispatch, so the fixed cost is almost certainly **fill + pipeline
  barrier + dispatch ramp**. **This is a hypothesis, and rung H0 measures it instead of assuming it.**

> **Model validity caveat.** `0.2736 ns/pair` is calibrated on *this* dispatch shape (3456 threads,
> warp-uniform light index, balanced across all 54 groups). The hierarchical arm changes the shape
> (6144 threads, lane-varying light index in the coarse phase, group-uniform candidate list in the
> fine phase, deliberately *imbalanced* across groups). The model is used below for *sizing
> decisions and go/no-go bounds only*. **Rung H1.5 tests the one part of that transfer which can be
> tested without writing a shader.** The shipping decision is a measurement (H4), and the abort
> threshold in §10 is expressed in measured nanoseconds.

### 1.3 The occupancy profile (CPU probe)

A CPU probe over the repo's own host oracle `golden_cluster_cull`
(`crates/boyko_rhi_vulkan/src/goldens.rs:3510`), replicating the VB-P1d camera (eye `(0,1.1,7.8)` →
`(0,0.55,0)`, `fov_y 52°`, aspect 1.0, 512×512) and its procedural rig
(`vb_p1d_cull_shade_bench.rs:124,142`) against the default `ClusterConfig` (16×9×24 = 3456 froxels,
`z_near 0.1`, `z_far 50.0`, `MAX_LIGHTS_PER_CLUSTER 256`, `INDEX_LIST_CAP 16384`):

| `N_ps` | total indices | % of 16384 cap | non-empty froxels | max per froxel |
|---|---|---|---|---|
| 8    | 789  | 4.8 %  | 514 | 3 |
| 14   | 1239 | 7.6 %  | 543 | 5 |
| 32   | 1916 | 11.7 % | 557 | 10 |
| 64   | 2063 | 12.6 % | 364 | 15 |
| 128  | 1654 | 10.1 % | 143 | 24 |
| 256  | 2072 | 12.6 % | 115 | 40 |
| 512  | 2597 | 15.9 % | 85  | 64 |
| 1024 | 2709 | 16.5 % | 55  | 109 |

**This is the single most important table in the document.** At `N=512` the cull performs
`3456 × 512 = 1 769 472` pair tests and **2 597 of them succeed — 0.147 %**. The pass is 99.85 %
rejection work, and 97.5 % of froxels are empty. A level that rejects *whole blocks of froxels
against whole ranges of lights* attacks exactly that.

**Provenance — still absent, now with a rung rather than a promise.** Rev 2 anchored this table on
`scratchpad/cap_probe.rs.txt`, a session-ephemeral file that **is not in the repository** (re-verified
for Rev 4: `git ls-files | grep -i 'cap_probe\|occupancy_probe'` returns nothing). Since §7's fine-pair
column, §6's saturation discharge and §10's abort criterion rest on this table, prose cannot
re-derive it. **Rung HP (§8) lands it as a `#[test]`, and Rev 4 may not be approved until HP is
committed.** After HP lands, this paragraph is re-anchored on the test's `file::test_name` and this
table is a pin, not a one-off print.

### 1.4 A defect in the bench rig that bounds what we may claim

`light_position` (`vb_p1d_cull_shade_bench.rs:124-137`) claims its three Kronecker multipliers
(`g = 0.618_033_988_75`, `g² = 0.381_966_011_25`, `g³ = 0.236_067_977_5`) are "mutually irrational, so the
sequence never repeats/aliases across the three axes". **That claim is false, and provably so:**

* `g + g² = 1` exactly ⇒ `frac(i·g²) = 1 − frac(i·g)` for every non-integral `i·g` ⇒ **`fy = 1 − fx`.**
* `g³ = g − g²` ⇒ `frac(i·g³) = frac(2·i·g)` ⇒ **`fz = frac(2·fx)`.**

So the "3-D low-discrepancy volume fill" is a **one-dimensional locus**: with `fx` sweeping `[0,1)`,
`x`, `y` and `z` are each affine in `fx` on each of the two halves `fx < ½` and `fx ≥ ½`. **All
`N_ps` lights lie on exactly two straight 3-D segments.** This explains §1.3 exactly: the segments
run diagonally out of the frustum, so as `N` grows (and the placement volume grows as `cbrt(N/14)`)
the lights are pushed laterally outside the view cone — `non-empty froxels` collapses 514 → 55 while
`max per froxel` climbs 3 → 109. The doc-comment's "keeps the AVERAGE per-froxel light density
roughly constant" is refuted by its own data.

**Numerically verified** (over `i ∈ [1, 1024]` at the literals the source uses):
`g + g² = 1.0` exactly; `g − g² = 0.23606797750000003` vs the source's `g³ = 0.2360679775`;
`fy = 1 − fx` and `fz = frac(2·fx)` hold with **0 violations / 1024** at a max deviation of
`1.99e-13` (pure float round-off). The dependency is real, not an approximation.

**Consequences (all binding on this plan):**

1. The high-`N` rows of the VB-P1d table measure a scene whose lights are *mostly out of frustum*.
   The hierarchy's win on this rig is a **best case** (rejection-dominated).
2. VB-P1e must therefore report **two rigs**: the existing one (unchanged — it is the provenance of
   `CLUSTER_LO`/`CLUSTER_HI`) *and* a new in-frustum rig where lights stay inside the view volume as
   `N` grows (§8, H4).
3. `CLUSTER_LO=64`/`CLUSTER_HI=128` were calibrated on this rig and may shift for dense in-frustum
   scenes. **VB-P1e does not re-tune them** — it publishes new numbers and flags the re-tune as
   VB-P1f (§11). Consequence to state openly: in `Auto` mode a 64 < `N` < 128 scene keeps the flat
   path and sees no VB-P1e benefit until VB-P1f lands.

---

## 2. Goal

Make the cull's pair count **sublinear in the froxel count** (not in `N` — see §7.1), so the
break-even collapses and the froxel path wins across a far wider range.

**Success is defined numerically (§10 restates these as the abort criterion):**

| Metric | Today | Required to ship |
|---|---|---|
| `froxel_cull_ns` @ `N_ps=512`, existing rig | 498 067 | **≤ 250 000** (≥ 2×; predicted ≈ 23 000) |
| `froxel_cull_ns` @ `N_ps=64`, existing rig | 72 748 | **≤ 80 023** (≤ +10 %) |
| `froxel_cull_ns` @ `N_ps=8`, existing rig | 19 741 | **≤ 21 715** (≤ +10 %, the fixed-cost floor) |
| break-even (`froxel_total < flat_shade`) | ≈ 103 | **≤ 40** measured (predicted 17–19) |
| per-froxel index SET vs the flat arm | — | **exactly equal**, order included (§9) |
| `vb_mesh_froxel` / `vb_mesh_tex_froxel` pins | green | **byte-identical, no re-pin** |

**The `N_ps=64` row changed in Rev 4.** Rev 3 wrote `≤ 72 748` — zero tolerance on a single sample —
while §10 ABORT 4 allowed 10 % and §10's prose said "a 5 % loss at `N=8` is not an abort". H4's
RED-if ("any §2 threshold missed") made the contradiction operative: a 1 ns run-to-run wobble would
have aborted the rung. **§10's reading is adopted here and in §10, and the two now agree by
construction** — every regression row in this table is `measured × 1.10`, rounded up.

Note the one target Rev 3 *removed* and Rev 4 keeps removed: the base `cluster_cull.comp.spv` is
**not** byte-frozen across the whole rung. D10 changes the shared distance function, so that blob is
re-pinned exactly once, at H1.6, and is byte-frozen from that commit onward (D5).

---

## 3. Key decisions

### D1 — Two levels, gather-side, **inside one dispatch, in groupshared memory**

**What.** Keep one thread per froxel. Make the **workgroup** the coarse cell: the group's threads
first co-operatively reduce their own froxel AABBs into a **group AABB**, test the light table
against *that* once (striped across all lanes), record the survivors as a **groupshared bitmask**,
and only then run today's exact per-froxel test over the mask's set bits.

**Why.**
* The coarse level costs `groups × N` pair tests instead of `froxels × N` — a `froxels/groups`
  reduction on the rejection work (144× at the chosen size, §D3).
* **No second dispatch.** §1.2's fixed cost is ≈ 13.9 µs *per cull invocation*; a separate coarse
  dispatch would plausibly add another one, which at low `N` is the entire budget. A groupshared
  hierarchy adds **zero** dispatch-level overhead.
* **No new GPU buffer** ⇒ no new framegraph resource, no seeding decision, no cross-frame WAR
  surface, no stale-data hole `[P0-1]`, `[P0-4]`. Rev 3 made this claim *true* rather than
  *defended* by removing H0's `LightIndexAlloc` readback from the present path (§P1-E, H0, §9);
  Rev 4 keeps that removal.
* Groupshared is per-dispatch by construction: the "stale mask" failure mode cannot exist.

**Alternatives rejected.**
* *A coarse pass writing a global bitmask buffer, consumed by a second dispatch* (Rev 1's shape).
  Rejected on the fixed cost above, plus it would need a new `add_buffer_seeded` resource, an
  unconditional every-word-write totality argument, and a cross-TU FP-margin proof. Every one of
  those problems is *deleted*, not solved, by moving the level into the group.
* *Screen-space XY column hierarchy (one coarse cell per (x,y) over all z).* Rejected on numbers:
  the z-slab is the dominant discriminator in this scene (§1.4 — the in-frustum lights occupy 3 of
  24 slices); collapsing z makes the coarse AABB span `view_z in [0.1, 50]` and reject nothing.
* *Light-centric scatter (rasterize each light's sphere into the froxel grid).* Genuinely
  output-sensitive (approximately 2 600 writes instead of 1.77 M tests at `N=512`) and it is what
  Doom-2016-class clustered pipelines do — **but** it produces per-froxel lists in atomic order, and
  per-froxel **table order is load-bearing**: the shipped flat-vs-froxel equality golden
  (`vb_mesh_froxel.rs`, `BOYKO_VB_FROXEL_FORCE_OFF`) holds only because the froxel list is the flat
  loop's order with exact-zero contributions elided (`x + 0.0 == x`), and FP addition is not
  associative. Restoring table order requires a per-froxel bitmask plus a compaction pass, whose
  cost (`froxels × ceil(N/32)` word scans) lands within a few percent of D1's total anyway. Rejected
  as strictly more machinery for no modelled gain and a live regression risk to a shipped gate.
* *`WaveActiveMin`/`WaveActiveBallot` (SM 6.0 wave intrinsics) instead of groupshared.* Would remove
  every barrier, but requires `VK_KHR_shader_subgroup_ballot`/`_arithmetic` support in COMPUTE, which
  this raw-FFI engine does not query today. Rejected for portability; re-openable as a measured
  follow-up (§11) because it is output-neutral by D4.

**Trade-off.** **Three** `GroupMemoryBarrierWithGroupSync()` per group:

* **B1** after the per-lane AABB store + mask init;
* **B2** after the radix-16 in-place fold (D9);
* **B3** after the coarse mask/summary publish (§4 phase 4).

…plus a strict uniform-control-flow obligation (§D8) — the single most likely implementation bug in
the rung. The count is stated **once**, here, and §4 carries a `Barriers: 3 total` footer so the two
cannot drift apart again. **Rev 4 adds the mechanical half**: H2 gate (e) asserts all three barriers
lie on the entry function's *top-level block chain*, an assertion verified (M9) to go RED on an
isolated early `return` and GREEN on the correct shape — which the two assertions Rev 3 wrote in this
slot provably could not do (M7, M8).

**The barrier count is not reducible by wave-synchronous elision, and this plan does not assume it
is.** The RHI enables no subgroup feature (`crates/boyko_rhi_vulkan/src/device.rs:2584` —
`subgroup_size_control: VK_FALSE`) and queries `subgroupSize` nowhere (a grep over
`crates/boyko_rhi_vulkan/src` + `crates/boyko_rhi/src` returns only raw FFI field declarations at
`ffi.rs:2623,2624,2691,2703` and that one `VK_FALSE`). Without an enabled subgroup guarantee,
dropping the tail steps of a reduction on an assumed wave width is UB under the Vulkan memory model,
and NVIDIA's post-Volta independent thread scheduling has made the idiom unsound even where it once
worked. **The portable lever is fewer reduction *steps*, not skipped barriers** — which is exactly
what D9's radix-16 fold buys (9 barriers to 2).

### D2 — The coarse AABB is the **componentwise min/max of the children's own AABBs**

**What.** The group AABB is *not* recomputed from block-corner geometry. It is a reduction over the
values each lane already computed for its own froxel.

**Why.** It makes the conservative-enclosure property a **tautology, exact in IEEE-754, with no
epsilon and no dilation** (§5 proves it). This is the direct discharge of the critic's `[P1]`
("D4 reduces to an unmeasured inequality"): there is no second computation of the same geometric
quantity, hence no discrepancy to bound. It is also *cheaper* than an independent coarse AABB build
(no extra unprojections).

**Corollary (very strong, state it in the shader header):** **any** assignment of froxels to groups
is correct. The grouping affects performance only — never the output set. This removes the entire
class of "is the block decomposition conservative?" review questions, including for partial blocks,
ragged grid dimensions, and hardware-dependent wave sizes. It also covers D9's change of reduction
*shape* from a halving tree to a radix-16 fold: `min`/`max` remain exactly associative and
commutative, so §5 Step 1 is untouched.

**Trade-off.** The coarse AABB is the union bound of the children's *AABBs*, which is slightly
looser than the true block hull; irrelevant, since the fine test is unchanged and exact.

### D3 — Group = 256 threads = one z-slice of the froxel grid (at the default 16x9x24)

**What.** `TPG = 256`. `gps = ceil(dim_x·dim_y / 256)` groups per z-slice; total groups
`= gps · dim_z`. For the default grid: `gps = ceil(144/256) = 1`, **24 groups**, 6144 threads
(3456 valid + 2688 idle-for-fine-work but fully used by the coarse phase).

**Thread-to-froxel map (every dim comes from the BOOT push, never the live header; see D11):**

```hlsl
uint bdx = pc.cluster_dims_packed & 0xFFu;
uint bdy = (pc.cluster_dims_packed >>  8) & 0xFFu;
uint bdz = (pc.cluster_dims_packed >> 16) & 0xFFu;
uint capacity = pc.cluster_capacity;                // NEW in Rev 4: the pushed boot u32 (D11),
                                                    // NOT re-derived from the three 8-bit fields
uint gps   = max(1u, (bdx * bdy + 255u) / 256u);    // max(1) => OpUDiv can never divide by 0
uint slice = gid.x / gps;
uint s     = (gid.x % gps) * 256u + lane;
uint x = (bdx != 0u) ? (s % bdx) : 0u;              // % 0 is UB on a degenerate header
uint y = (bdx != 0u) ? (s / bdx) : 0u;
uint z = slice;
uint fi = cluster_linear_index(x, y, z, bdx, bdz);  // light_table.hlsli:329 — UNCHANGED
bool valid = (s < bdx * bdy) && (slice < bdz) && (fi < capacity);
```

`fi` is computed **unconditionally** — it is pure uint arithmetic and touches no memory, so no lane
skips a barrier (D8). `valid` is a data predicate guarding phases 1, 5 and 6, **and it is the only
predicate that does so** (Rev 4 removes Rev 3's second `contrib` predicate — see §5). The HIER arm
**does not call `load_cluster_params` at all**; it consults the live header only for
`l0a_count`/`light_count` via `load_light_header`, exactly as the base arm does.

*Why all three terms are there, and which of them a mutation can reach.* `s < bdx·bdy` bounds `y`
(`x = s % bdx` is bounded by construction). `slice < bdz` bounds `z`. `fi < capacity` is the hard
device-write bound and the only term naming the buffer's real size.

> **Under-claim, stated because Rev 3 over-claimed here.** With push-sourced dims *and* a
> host-derived group count, the first two terms already imply the third algebraically
> (`fi = s·bdz + slice <= (bdx·bdy − 1)·bdz + bdz − 1 = capacity − 1`), and the host never dispatches
> more than `gps·bdz` groups, so `slice = gid/gps < bdz` holds by construction. **Simulated (M10):
> deleting `slice < bdz` alone, or `fi < capacity` alone, is inert — 0 OOB writes, 0 gaps, 0
> duplicates.** Neither term has a single-fault mutation that turns any gate red. They are kept
> because each becomes load-bearing the moment a *second* fault appears (re-sourcing the dims from
> the header, or a hand-edited group count), and §8.3's mutation (v) is exactly that two-fault
> combination. They are **defence in depth, not independently gated invariants**, and §9 says so.

No product can wrap: every dim is 8-bit packed (`light_table.hlsli:318-320`,
`ClusterConfig::packed_dims` `light.rs:763-769`), so `bdx·bdy <= 65 025` and the 8-bit-derived
product is `<= 16.6 M`, all below 2^24. **`capacity` no longer depends on that argument at all** —
it is the pushed full-precision `cluster_count()` (D11).

**Why this shape (arithmetic — Rev 4 withdraws two Rev 3 rows rather than re-fitting them).**
For a block of `nz` consecutive z-slices starting at slice `k`, exp-Z gives a depth extent
`near·q^k·(q^nz − 1)` with `q = (far/near)^(1/dim_z) = 500^(1/24) = 1.2955587`. **At a fixed
starting slice** the ratio against a single-slice block is `(q^nz − 1)/(q − 1)`:

| `nz` | 1 | 2 | 4 |
|---|---|---|---|
| depth-extent ratio at fixed `k` | 1x | **2.296x** (`q+1`) | **6.148x** (`q^3+q^2+q+1`) |

Reproducible with a calculator: `q = 500^(1/24) = 1.2955587`, `q^2 = 1.6784723`, `q^3 = 2.1745594`
(so `q+1 = 2.296` and `q^3+q^2+q+1 = 6.149`, the two table entries). *Rev 5 corrects Rev 4's
`1.295563 / 1.678483 / 2.174371`, which were a rounded `q` propagated into its powers; the table's
two ratios were already right.*

**What Rev 4 deletes here, and why.** Rev 3 printed `1.9115 / 14.424` depth terms and `3.85x / 29.1x`
ratios from a `q^(3nz)(1 − q^(−nz))` expression that **compares blocks at different depths**, so the
ratio it reports mixes the block-thickness effect with the exp-Z position effect. It also printed a
`TPG=1024, nz=4 -> 1.04x` total row that is **not derivable from this section's own hot-group
method**. Both are withdrawn. The conclusion is unchanged and does not depend on them: exp-Z means
depth extent grows *geometrically* with slice count, so **single-slice blocks are decisively best**.
Note also that `nz >= 2` **is not expressible under this section's own thread-to-froxel map** at
`TPG=256`, so this table is a sanity check on the map rather than a live design fork, and **no
pair-count estimate below is derived from it.**

Given `nz=1`, the remaining choice is how many froxels of the slice per group. Modelled totals at
`N=512` on the measured occupancy profile (§1.3), with `gps` printed so every row is checkable
against the formula above (`gps = ceil(144/TPG)`, `groups = gps·24`, flat = 1 769 472):

| `TPG` | `gps` | groups | coarse pairs (`groups·N`) | fine pairs (est.) | total | vs flat |
|---|---|---|---|---|---|---|
| 64  | 3 | 72 | 36 864 | approx 12 000 | approx 48 900 | 36x |
| 128 | 2 | **48** | **24 576** | approx 17 300 | **approx 41 900** | **42x** |
| **256** | **1** | **24** | **12 288** | approx 20 000 | **approx 32 300** | **55x** |
| 1024 (`nz=1`, *this section's map*) | 1 | **24** | **12 288** | approx 20 000 | approx 32 300 | 55x |

Rev 2's `TPG=128` row said 36 groups / 18 432 / 35 700 / 50x; the correct row is **48 / 24 576 /
41 900 / 42x**, which **widens** `TPG=256`'s margin (42x to 55x, not 50x to 55x). Rev 2's `TPG=1024`
row ("6 groups, spans 4 slices") was arithmetically self-consistent only under an `nz=4`
decomposition the section had already excluded — it contradicts `ceil(144/1024)·24 = 24` — and
Rev 4 **deletes** that row rather than printing a total it cannot derive.

`TPG=256` also wins under the *opposite* (uniform in-frustum) assumption — a Steiner/Minkowski
model over a frustum-filling light field gives `TPG=64: approx 65 900` vs `TPG=256: approx 52 600`
pairs — because the coarse term shrinks 3x while the fine term grows only 1.39x.

**Occupancy — and the SM count is an OWNER-STATED DEVICE FACT, not a measurement.** The RTX 3060's
**28 SMs** is stated by the owner and is **not** verifiable from this stack: core
`VkPhysicalDeviceProperties` exposes no SM count (it requires `VK_NV_shader_sm_builtins::shaderSMCount`,
which this engine never enables), and a repo-wide
`grep -rniE "sm_count|shaderSMCount|multiprocessor|SHADER_SM_BUILTINS" crates/` returns **zero** hits.
*(Rev 4 promised H0 would print it; that deliverable was unimplementable and is deleted — §8.5.)*
Read the following as a shape argument whose only hardware input is that figure: 24 groups x 8 warps
lands 8 warps on **24 of 28 SMs and leaves 4 idle**; today's `ceil(3456/64) = 54` groups x 2 warps
feed **all 28** (26 SMs x 4 warps, 2 SMs x 2 warps). The hierarchical shape therefore trades **14 % of
the machine** for 2x the warps per active SM — a net win only if the pass is latency-bound, which
**H1.5** measures. **No gate consumes the 28**; if it is wrong, only this paragraph's percentages move,
and H4's `dim_z` sweep (§7.0) decides the question empirically either way.

**Alternatives rejected.** `TPG=64` (would keep `[numthreads(64,1,1)]` and the existing
`LIGHT_CULL_LOCAL_SIZE_X` constant untouched) — rejected: 3x the coarse cost for a modelled **1.51x**
worse total (`48 900 / 32 300`; Rev 3 printed 1.35x, which its own table contradicts). `TPG=1024`
under this section's own map — rejected **not** on pair count (it ties `TPG=256` exactly, same
one-slice coarse box) but on three other grounds: 880 of 1024 lanes idle (86 %), **32.9 KB** of
groupshared (`2 x 1024 x 16 + 132`), which exceeds the Vulkan-required minimum
`maxComputeSharedMemorySize` of 16 384 B — and the engine queries that limit **nowhere** (a grep for
`max_compute_shared_memory` over `crates/` returns no hits), so nothing would catch it at runtime —
and 1 group/SM. Blocks spanning 2 or more z-slices — rejected by the depth-extent ratio above. A 3-D
block (e.g. 8x4x2) — modelled within 5 % of 16x4x1 and strictly worse than one-slice-per-group, and
it needs a 3-D delinearization for no gain.

**Trade-offs (three, all stated).**
1. 2688 of 6144 lanes hold no froxel at the default grid (**43.75 %**). They cost nothing in phases 1
   and 5 (guarded by `valid`), and in phase 4 they are **productive** — the coarse light scan is
   striped across all 256 lanes regardless of `valid`. *(§8.2 shows this same 2688 is exactly the
   width of the `ClusterGrid` guard tail — the idle lanes and the detector are the same set.)*
2. With 24 groups on 28 SMs the dispatch is deliberately *imbalanced* (3 hot groups, 21 near-empty
   at `N=512`); wall clock is then set by the hot group, which is exactly the work that cannot be
   avoided. §7 pre-registers what that means for H4.
3. **Write locality changes, and neither H1 nor H1.5 can see it.** Today `fi = tid.x` and
   `cluster_linear_index = (y·dim_x + x)·dim_z + z` (`cluster_cull.hlsl:116-121`,
   `light_table.hlsli:329`), so consecutive lanes write consecutive `fi` — a 64-lane group writes 512
   contiguous bytes (8 cache lines). Under this map consecutive lanes share `z` and stride `fi` by
   `dim_z = 24`, i.e. **192 B apart**, so a 256-lane group touches 256 distinct lines. Worst case the
   full-grid `ClusterGrid` write traffic goes 27 KiB to 216 KiB. At Ampere bandwidth that is well
   under 1 microsecond against a 23 microsecond target and L2 will coalesce most of it, so it does
   **not** threaten the design — but it is dispositioned here rather than discovered at H4, and it is
   why §4's "token-identical" claim for phase 6 is scoped to the *source text*, not the access
   pattern.

### D4 — Output is bit-identical to the flat arm (under the stated scope), by construction

Four properties, each independently required:

1. **Same set** — the coarse level never rejects a light the fine test would accept (§5, exact,
   with its evaluation-function premise *discharged* by D10 rather than disclaimed).
2. **Same order** — the fine loop walks `gs_summary` with `firstbitlow` ascending and, within a
   word, `gs_mask[w]` with `firstbitlow` ascending, so `j = (w<<5)|b` is strictly ascending and
   `i = ps_begin + j` is strictly monotone in `j`, hence **table order**, identical to today's
   `for i in [l0a_count, light_count)`. The coarse phase's striped `InterlockedOr` order does not
   matter: bitwise-OR is commutative and associative, so the final mask is deterministic. This is
   what preserves the shipped flat-vs-froxel equality golden.
3. **Same clamp** — `max_lights_per_cluster` truncates the same ascending prefix (`:170`). Preserved
   because the fine walk visits a *subset* of the flat loop's indices that still contains every
   ACCEPTED index (§5), applies the token-identical predicate and the token-identical
   `nlocal < pc.max_lights_per_cluster && nlocal < 256u` guard, and never `break`s early — so both
   arms keep the same ascending prefix of the same accepted set.
4. **Different slice offsets only** — the global `InterlockedAdd` claim order changes, so
   `ClusterGrid[fi].offset` differs. Offsets do not affect any shaded pixel; the resolve reads
   `[offset, offset+count)`.

**Verified by simulation** (20 000 randomized trials sweeping `l0a_count` in {0,1,2,3,7,31,32,33,100,
512,1000,1023}, point/spot spans up to the 1024 budget, caps in {1,2,5,256,unbounded}, with a
conservative coarse superset and injected non-punctual rows): **0 mismatches**, every emitted
sequence equal to its sorted form, mask word 31 exercised in 196 runs.

**Scope of the claim — Rev 4 has FOUR clauses, one fewer than Rev 3.**

* **(a)** Non-saturating configurations only — under global-cap saturation the arms may legitimately
  differ in *which* froxel loses its tail (§6).
* **(b)** `boot_dims == live_dims` — the no-skew precondition (D11). Under skew the HIER arm's `fi`
  is still in-bounds and total, but its geometry no longer matches what the resolve (which reads the
  live header) expects; that is the pre-existing skew class, and H3 asserts the precondition loudly
  like §6's `alloc_total`.
* **(c)** ~~Finite AABBs only.~~ **DELETED in Rev 4.** Under §5's absorbing-element substitution a
  group containing any non-finite lane degrades to *exactly* the flat arm's walk for every one of
  its froxels — same punctual index set, same ascending order, same clamp — so byte-identity holds
  **unconditionally in the AABB's finiteness**. This is a strengthening, and it is not asserted: §8's
  mutation (vii) is a two-sided test that must be GREEN with the substitution and RED without it.
  *Rev 5 scope note:* the strengthening is in the AABB's finiteness **only**. It rests on §5 Case B,
  which needs the **light centre** to be finite (**Premise F**, §5.2) — a different quantity, a
  pre-existing hazard for the base arm too, and NOT a reinstatement of clause (c).
* **(d)** Both arms compiled from the same `sq_dist_point_aabb` (D10). This is why D10 must change
  the **shared** function and cannot be an `#ifdef HIER`-only refinement.
* **(e)** Byte-identity is a claim about the *output buffers*, not about the memory access pattern
  (D3 trade-off 3).

### D5 — `#ifdef HIER` two-compile; the base `.spv` is re-pinned **exactly once**, then frozen

`cluster_cull.hlsl` is hand-authored (no `// === GENERATED ... ===` sentinels), so it is edited
directly. The hierarchical body is compiled in only under `-D HIER=1`, producing a **new**
`cluster_cull_hier.comp.spv`. Rationale: the cull pipeline is shared by Deferred
(`passes/gbuffer.rs`), ForwardPlus (`passes/forward.rs`) and VB (`passes/vb.rs`) — a frozen base
removes all risk to those paths while the variant is proven, and it gives the equality oracle a
*free A/B*: both arms are runnable in the same process against the same inputs. Precedent: the
`-D FROXEL=1` family (`tests/vb_froxel_spv_sync.rs:98-105`,
`docs/SHADER-VARIANT-MANIFEST.md:91-97`).

Rev 2's title — "base `.spv` byte-frozen" — is not achievable: D10 changes the *shared*
`sq_dist_point_aabb`, so `cluster_cull.comp.spv` changes. The honest statement:

> `cluster_cull.comp.spv` is **re-pinned exactly once, at rung H1.6**, in a commit that contains no
> HIER code at all, and is byte-frozen from that commit onward.
> `cluster_cull_spv_sync.rs` continues to gate it under the unchanged frozen recipe
> (`-spirv -T cs_6_0 -E main -fspv-target-env=vulkan1.3`, no `-D`, no `-O`,
> `cluster_cull_spv_sync.rs:45-53`).

Three measured facts make the seam itself safe:

* **(M1, re-run for Rev 4)** An **unmodified** copy of `cluster_cull.hlsl` compiled with the frozen
  recipe reproduces the committed `cluster_cull.comp.spv` **byte-for-byte** (12 392 B). The gate is
  reproducible outside the repo, so every experiment in this document was run in a scratch directory
  and **no committed `.spv` was written**.
* Adding the `#ifdef HIER` push members **and** a full ~130-line HIER arm leaves the no-`-D` compile
  at the *identical* 12 392 B / sha256 `dbb924967b1176af…`. The seam is physically inert; H2 gate (b)
  survives a widened push. *(Rev 3 measured this with a one-word push tail; Rev 4 widens the tail to
  two words — H2(b) re-measures it, and the byte-identity of the `#else` arm is the assertion, not
  the assumption.)*
* Incidental, and it simplifies §4: the **shared** 3-parameter entry point
  `void main(uint3 tid : SV_DispatchThreadID, uint3 gid : SV_GroupID, uint lane : SV_GroupIndex)`
  also compiles byte-identically with no `-D` — DXC dead-strips the unused SV parameters — so the
  signature does **not** need to be `#ifdef`-split.

### D6 — Mask capacity: **equality**, and there is no defensive tail

`HIER_MASK_WORDS = 32`, plus one `summary` word whose bit *j* marks "mask word *j* is non-zero" (so
the fine walk visits only non-empty words). The host pin is an **equality**, not Rev 2's `<=`:

```rust
// crates/boyko_render/src/light.rs, beside MAX_LIGHTS:51
pub const HIER_MASK_WORDS: u32 = 32;
const _: () = assert!(MAX_LIGHTS == HIER_MASK_WORDS * 32,
    "invariant: the hier mask covers the table EXACTLY — one clamp bounds both the \
     groupshared write and the device read");
```

plus a text pin test that reads `cluster_cull.hlsl` (the `shaders_dir()` + `read_to_string` idiom
already used at `cluster_cull_spv_sync.rs:20-22` and `field_probe_gate.rs:102`) and asserts the file
contains `#define HIER_MASK_WORDS 32u`.

**Equality is load-bearing.** With `<=` (say `MAX_LIGHTS = 512` against 32 words) `ps_room` would
exceed the table's row count and the single clamp of D7 would no longer bound the *device read*.
Stating the cost openly so it is not discovered during VB-P1f: any future `MAX_LIGHTS` change is now
a compile error that forces a shader edit and a `.spv` re-bake. That is the intended price of one
clamp covering two bounds.

**There is no defensive tail.** Rev 2 promised "any index `i >= l0a_count + 1024` is tested
exhaustively rather than trusted to the mask". That cannot be made safe: the tail's whole point is
to `load_light(i)` for `i` past the mask capacity, and with `HIER_MASK_BITS == MAX_LIGHTS == 1024`
those rows **do not exist in the device buffer** (`LIGHT_TABLE_CAPACITY = LIGHT_HEADER_BYTES +
MAX_LIGHTS * GPU_LIGHT_BYTES`, `crates/boyko_app/src/gpu_scene/mod.rs:205-207`), and
`robustBufferAccess` is OFF. The tail would **relocate** the out-of-range read from the coarse phase
to the fine phase, not prevent it — while adding a review obligation and a second copy of the fine
test that must be kept token-identical for D4. It is replaced by an explicit, documented truncation
at `ps_n` (D7), made unreachable through the host fold: `fold_light_table_slotted`
(`crates/boyko_render/src/light_system.rs:212-300`) gates **every** write with a saturating
`if written == MAX_LIGHTS { return finish_folded_overflow(..) }` (`:263, :272, :282, :291`) and its
doc-comment `:199-210` states that this clamp is present "in ALL build profiles … the sole bounds
enforcement … so it must NOT be a debug-only guard". (The complementary checks
`l0a_count + point_spot_count <= MAX_LIGHTS` at `:300` and `light_count <= MAX_LIGHTS` at
`light.rs:1082` are `debug_assert!` only — which is precisely why the shader must not trust the
header, and does not.)

**Corollary used by §5 and D4, stated here so it is not re-derived three times.** Because the host
fold clamps `light_count <= MAX_LIGHTS` in every build profile, `ps_total = light_count − l0a_count
<= MAX_LIGHTS − l0a_count = ps_room`, hence **`ps_n == ps_total` on every well-formed frame** and the
HIER arm scans exactly the flat arm's range. The `min` in D7 exists for the *malformed* header, which
is a trust-boundary case, not a normal one.

### D7 — One clamp `ps_n` bounds the groupshared write and both device reads `[P0-4b]`

**Index convention (this was Rev 2's P0-C disagreement, settled in Rev 3): the mask is
l0a-RELATIVE.** Mask bit `j` corresponds to table index `ps_begin + j`, where
`ps_begin = hd.l0a_count`. Three reasons, in order of weight:

1. It is already the convention of two of the three sites Rev 2 wrote — D6's tail and D7's
   reconstruction (`i = l0a_count + (w<<5) + firstbitlow(bits)`) were both relative; only §4 phase 3
   was absolute. Relative fixes the inconsistency at its single source.
2. Under relative, **the bit index IS the coarse loop counter**, so the write bound is a one-line
   syntactic consequence of the loop condition *in the same basic block*, with no device value in the
   derivation: `j < ps_n <= HIER_MASK_BITS = HIER_MASK_WORDS*32` implies `(j>>5) < HIER_MASK_WORDS`.
   Under absolute the reviewer must additionally reason that `light_count <= 1024`, which is device
   data at the trust boundary — exactly the argument this decision exists to avoid.
3. Capacity headroom is expressed in the quantity the mask actually indexes: relative needs
   `point_spot_count <= 1024`, absolute needs `l0a_count + point_spot_count <= 1024`. Absolute
   overflows first.

**The clamp** (evaluated once, before any barrier; group-uniform because every lane reads the same
header words):

```hlsl
#define HIER_TPG        256u
#define HIER_MASK_WORDS 32u
#define HIER_MASK_BITS  (HIER_MASK_WORDS * 32u)   // 1024 == MAX_LIGHTS (pinned EQUAL, D6)
#if (HIER_MASK_WORDS) > 32
#error "HIER_MASK_WORDS > 32: gs_summary is a SINGLE uint, one bit per mask word"
#endif
#if (HIER_MASK_WORDS) > (HIER_TPG)
#error "HIER_MASK_WORDS > HIER_TPG: phase 1 inits exactly one mask word per lane"
#endif
#if (HIER_TPG) != 256u
#error "HIER_TPG != 256: D9's radix-16 fold hardcodes 16 folding lanes x 16 entries == 256"
#endif

LightHeader hd = load_light_header(LightBuf);

uint ps_begin = hd.l0a_count;
uint ps_room  = (ps_begin < HIER_MASK_BITS) ? (HIER_MASK_BITS - ps_begin) : 0u;
uint ps_total = (hd.light_count > ps_begin) ? (hd.light_count - ps_begin) : 0u;
uint ps_n     = min(ps_total, ps_room);
```

*(**Rev 6 P2-5 — placement fix, no content change.** The paragraph below is Rev 5's P2-3 note. Rev 5
spliced it **inside** this fenced HLSL block, between `load_light_header` and `ps_begin`, where it is
not a comment in any language: an implementer copying the fence gets five lines of prose in the middle
of the shader. It is moved out verbatim.)*

*(The third guard is new in Rev 5 (P2). `HIER_TPG` was the one `#define` with no `#error` behind it,
yet D9's fold hardcodes both `16` and `256u` and §4 phase 1's `if (lane < HIER_MASK_WORDS)` init
assumes `HIER_TPG >= HIER_MASK_WORDS`; a silent `HIER_TPG` edit would leave the fold reading
uninitialised slots. Two lines, and H2(f) already has the mechanism to test it — the same scratch-copy
rewrite-and-expect-a-compile-failure it runs for `HIER_MASK_WORDS`.)*

Both branches are uint-underflow-proof by construction. For **any** 32-bit header bytes whatsoever:

* **WRITE:** `j < ps_n <= ps_room <= HIER_MASK_BITS` implies `(j>>5) <= 31` — `gs_mask[]` is never
  left.
* **READ (capacity):** `ps_begin + j < ps_begin + ps_room <= HIER_MASK_BITS == MAX_LIGHTS` — never
  past the table's row capacity.
* **READ (live span):** `ps_begin + j < ps_begin + ps_total == hd.light_count` — never past the live
  table, which is the bound the smaller test-harness light buffers need.

Both read bounds hold simultaneously because `ps_n` is the `min`. **This is strictly stronger than
the base arm**, whose flat loop (`cluster_cull.hlsl:161-162`) has no clamp at all.

**The fine arm re-checks the identical bound**, `if (j >= ps_n) { continue; }`, applied *before* the
reconstruction `i = ps_begin + j`. It implies `i < light_count` (`ps_n <= ps_total`) and
`i < MAX_LIGHTS` (`ps_n <= ps_room`) in one line each, so no second runtime compare is needed, and a
reviewer checks **one** bound for **both** phases. `robustBufferAccess` is OFF in this engine; an
out-of-range `StructuredBuffer<uint>` read is real UB, and this exact class already shipped one
GPU-UB bug this campaign (VB-P1b C1). The clamp makes the impossibility **local and auditable**
instead of a cross-phase argument.

**How this clamp is actually gated (Rev 4).** Rev 3's mutation (iv) — "force `ps_n = 0` in the fine
arm and delete the clamp" — **cannot produce an out-of-range read**, because phase 4 sets mask bits
only for `j < ps_n`, so the walk visits the same in-range bits either way. Rev 4 mutates the
**producer** instead and adds a detector so the fault is observable rather than UB:

* **Producer mutation:** loop `for (uint j = lane; j < HIER_MASK_BITS; j += HIER_TPG)` in phase 4
  **and** delete the fine `j >= ps_n` clamp.
* **Detector — the light-table POISON TAIL (new).** H3's cull-only driver always allocates
  `MAX_LIGHTS` rows (plus 1024 spare) and fills every row at index `>= light_count` with a poison
  light: `kind = POINT`, `pos = camera eye`, `range = 1e6` — a light **every** froxel accepts. The
  driver then asserts **no emitted `LightIndexList` value is `>= light_count`**.
* **Why it fires:** at any config with `light_count < ps_begin + HIER_MASK_BITS`, the mutated coarse
  loop reads poison rows, accepts them, sets their bits, and the unclamped fine walk emits their
  indices. The assertion sees indices `>= light_count` and goes RED. **No UB is involved in the
  test** — the rows are really allocated; only the *header* says they are not live. That is the same
  trick as the `ClusterGrid` guard tail (§8.2), applied to the other buffer.

**Deliberately NOT `hd.point_spot_count`** (`light_table.hlsli:248`): the base arm's range is
`[l0a_count, light_count)` (`cluster_cull.hlsl:161`), so the span must be derived from the **same two
header words** or D4's byte-identity breaks. A header whose word 3 disagreed with
`light_count - l0a_count` would otherwise make the two arms scan different ranges — and word 3 can
only *shrink* the range, i.e. silently drop lights the flat arm keeps.

**Rejected:** clamping at the write (`gs_mask[min(j>>5, 31u)]` or `& 31u`) — actively **wrong**, not
merely inelegant: saturating/masking the word index aliases out-of-capacity bits onto word 31, so the
fine walk reconstructs a *different* `j` than the one tested and can emit a light the coarse phase
never accepted. Guarding at the write (`if (j < HIER_MASK_BITS) InterlockedOr(...)`) — correct but
strictly worse: it adds a compare to the hot coarse loop, it leaves the **device read**
`load_light(ps_begin + j)` unbounded (the read precedes the guard), and it turns a loop-level
truncation into a silent per-bit drop.

### D8 — Uniform control flow around every barrier (the mandatory review checkpoint)

Today's shader early-returns on `fi >= cluster_count` (`:112-114`). The hierarchical body **must
not**: every lane, valid or not, must reach every `GroupMemoryBarrierWithGroupSync()`. The
out-of-range condition becomes a `bool valid` that guards work, never control flow across a barrier.
An early `return` here is undefined behaviour and typically a device hang.

**Rev 4 gives this rule a mechanical gate that was verified to discriminate.** Rev 3 asserted
"exactly one `OpReturn`" and "every `OpControlBarrier` sits in a merge block"; measured (M7, M8),
**neither can distinguish the correct shader from a deliberately broken one**, and the second gets it
wrong in both directions. The replacement is H2 gate (e)'s **top-level-chain** assertion, measured
(M9) GREEN on the correct shape and RED on an isolated early `return`. Both Rev 3 assertions are
deleted; §9 records the deletion rather than quietly dropping them.

**Corollary — TWO substitution constants, and they are not interchangeable.** A lane that holds no
froxel and a lane whose AABB is non-finite are different cases and get different values:

| lane state | stored AABB | effect on the fold | why |
|---|---|---|---|
| `!valid` (no froxel) | `(+1e30, −1e30)` — the **identity** of `min`/`max` | contributes nothing | padding lanes must not widen the coarse box |
| `valid && !finite` | `(−FLT_MAX, +FLT_MAX)` — the **absorbing** element | forces the box to the universe | the group degrades to exactly the flat walk (§5) |
| `valid && finite` | its own AABB | normal | — |

**The absorbing constant is `±FLT_MAX`, not Rev 4's `±1e30` (Rev 5 P1 fix).** With `±1e30` the coarse
level *rejects* for any finite light centre with `|c_j| > 1e30` (`c_j − 1e30 > 0` ⇒ `F = inf`) while
the poisoned lane's own all-NaN fine test accepts — enclosure inverted in exactly the case the
substitution exists to handle. `FLT_MAX` absorbs unconditionally over the finite floats, and it still
wins against the `±1e30` identity, so the two rows above remain a strict ordering. **The `1e30` in the
finiteness *predicate* (§4 phase 1) is NOT the same constant and must not be unified with this one:**
it is the classification envelope, and it is what makes the identity row true (a finite-classed lane
has `|AABB| ≤ 1e30`, so the identity can never narrow the box). §5 Case B carries the derivation.

**Swapping them is a real, asymmetric hazard, and each direction has a different detector:**

* Giving the *non-finite* lanes the identity is Rev 3's mitigation; it breaks enclosure for that lane
  and is caught by §8.3 mutation (vii)'s RED arm.
* Giving the *invalid* lanes the absorbing element is **output-neutral** — every group with padding
  lanes simply degrades to the flat walk — so **no equality gate can see it**. It is caught only by
  H1's selectivity assertion, which at the default 16x9x24 grid (where all 24 groups carry 112
  invalid lanes) collapses from 55x to **1.0x**. Rev 4 states this rather than implying the output
  gates cover it.

**Two obligations the "no early return" rule newly creates** (the base arm's `return` masks both
today, because it fires *before* `z = fi % cp.dim_z` at `:118`), both promoted to code-review gate
items alongside the barrier rule:

* `gps` **must** be `max(1u, …)`. A degenerate live header with `packed_dims == 0` — the value
  `sync_cluster_light_gate` writes on an unarmed path (`light.rs:836-840`) — gives
  `ceil(0/256) = 0` and an integer **divide-by-zero** in `slice = gid.x / gps`.
* `x`/`y` **must** be `bdx != 0u`-guarded, for the same reason on `s % bdx`.

**The review-gate list for D8, in full** (each is a line a reviewer checks, and each names its gate):

| # | Obligation | Gate |
|---|---|---|
| 1 | No `return` anywhere in the `#ifdef HIER` arm | H2(e) top-level chain + H2(g) source-text pin |
| 2 | All three barriers at the function's top level | H2(e) top-level chain |
| 3 | `gps = max(1u, …)` | H3's degenerate-header config (hang/div-0 probe) |
| 4 | `x`/`y` guarded by `bdx != 0u` | same |
| 5 | Identity for `!valid`, **absorbing** for `valid && !finite` | H1 selectivity (identity/absorbing swap), H3 mutation (vii) |
| 6 | `coarse_min`/`coarse_max` are **group-uniform** after phase 3 | §5's Setup premise; H3 mutation (ii) |

Item 6 is new in Rev 4: §5's proof requires the coarse box to be the *same value in every lane*, and
§4 phase 3 delivers that (every lane folds the same 16 published slots) — but Rev 3 presented it as a
performance choice and never stated it as a proof premise. If a future edit lets lane 0 broadcast
instead, the proof silently loses its quantifier.

### D9 — Reduction by a groupshared **radix-16 in-place fold** over **scalar** arrays

**Storage.** Six `groupshared float` arrays, not two `float3` arrays:

```hlsl
groupshared float gs_min_x[HIER_TPG], gs_min_y[HIER_TPG], gs_min_z[HIER_TPG];
groupshared float gs_max_x[HIER_TPG], gs_max_y[HIER_TPG], gs_max_z[HIER_TPG];
groupshared uint  gs_mask[HIER_MASK_WORDS];
groupshared uint  gs_summary;                     // bit j <=> gs_mask[j] != 0
```

Footprint: `6 x 256 x 4 = 6 144 B` + `32 x 4 = 128 B` + `4 B` = **6 276 B, exact by construction**.

*Why not `float3`.* Rev 2 asserted "6 KB" and §9 said "6.3 KB/group"; **both are unsupported by the
artifact**. Compiled under the frozen recipe and disassembled, DXC 1.4.350.0 emits
`%_arr_v3float_uint_256 = OpTypeArray %v3float %uint_256` with a `Workgroup` pointer and **no
`ArrayStride` decoration** (the module's only `ArrayStride` is the `4` on the `StructuredBuffer`'s
runtime array), and declares no `VK_KHR_workgroup_memory_explicit_layout` / no
`WorkgroupMemoryExplicitLayoutKHR` capability. Workgroup storage therefore carries **no explicit
layout**: the `float3` stride (12 B or 16 B, giving 6 276 B or 8 324 B) is chosen by the driver and
is not derivable from the `.spv`. A `float` has no padding ambiguity in any layout, so scalarizing
removes a driver-dependent variable from a plan whose selling point is that its correctness argument
needs no unverifiable premise. It also makes lane-indexed access **4-byte-strided
(bank-conflict-free)** instead of 12- or 16-byte-strided.

*Why not float-as-int atomics.* `InterlockedMin/Max` are integer-only; the order-preserving
float-to-uint key trick would work but adds a lemma to review, and 256 lanes contending on 6
addresses serialize anyway.

*Why radix-16 in place, and not Rev 2's 8-step halving tree.* 256 lanes give strides 128, 64, 32, 16,
8, 4, 2, 1 = **8 steps, 8 barriers** (Rev 2 counted these correctly; it was D1's summary line that
was wrong). The radix-16 fold needs **two**:

1. every lane stores its 6 scalars; **B1**;
2. lanes `l` in `[0,16)` each serially fold the 16 entries `gs[l + 16k], k = 0..15` and write
   `gs[l]`. **Race-free in place**: every active writer has `l < 16`, while every read address
   `l + 16k` for `k >= 1` is `>= 16`, and `k = 0` is the writer's own slot. **B2**;
3. **every** lane then folds `gs[0..16)` itself — 16 group-uniform broadcast reads, no write — and
   lands `coarse_min`/`coarse_max` in registers. **No third barrier**: B2 already published and
   nobody writes afterwards. *(Step 3 is what makes the coarse box group-uniform — D8 review item 6.)*

Cost: 32 serial `min`/`max` per active lane in step 2 plus 16 broadcast reads x 6 components in step
3 — roughly 96 extra scalar ops per lane, against **seven barriers deleted**. `min`/`max` are
**exact** in IEEE-754 and associative/commutative, so the fold order is irrelevant and the result is
**exactly the componentwise extremum of the stored values, with one stated exception: when both a
`+0.0` and a `−0.0` are present in a component, which of the two signs the result carries is
unspecified.** That exception is immaterial — `+0.0` and `−0.0` compare equal in every downstream
operation of §5 — and it is called out here only because Rev 3's §5 and its D9 disagreed about it
(§5 said "±0", D9 said "exactly the componentwise extremum" with no qualifier). They now agree.

*Rejected:* a separate 16-entry destination array (also 2 barriers) — in-place is provably race-free
at radix 16, so the extra 384 B and the extra symbol buy nothing. A serial fold by every lane over
all 256 entries (1 barrier) — approximately 1 536 ops/lane against approximately 96, to save one
barrier.

*Note on `[unroll]`.* The frozen recipe passes no `-O`, and a probe confirms the reduction loop is
left **rolled** in the emitted module (5 static `OpControlBarrier` for 12 dynamic ones in the probe's
8-step form, with `OpLoopMerge … Unroll` as a hint only). The barrier counts in this document are
**dynamic** counts; a `spirv-dis` gate must count executions, not instructions. **Rev 4 consequence:**
H2(e)'s top-level-chain assertion counts *static* barrier blocks and is therefore an assertion about
§4's three top-level barriers only — it says nothing about a barrier inside a group-uniform loop,
which would be legal HLSL but is not part of §4's shape. The assertion is a **design-conformance**
check on §4, not a general legality check, and it is labelled that way in H2.

### D10 — The cull distance is a **written-out `precise` sum**, never `dot()` `[P0-A]`

**The one shared `sq_dist_point_aabb` is replaced verbatim** (`cluster_cull.hlsl:102-105`):

```hlsl
// Squared distance from a point to an AABB (0 inside). The canonical clustered-cull test:
// a sphere (center, r) intersects the AABB iff this <= r^2.
//
// The sum is WRITTEN OUT and `precise`, not `dot()`, on purpose. Vulkan specifies OpFAdd /
// OpFSub / OpFMul as "Correctly rounded" (one legal fp32 result), but specifies OpDot only as
// "inherited from a formula", and the same appendix permits that formula to "be transformed
// using the mathematical associativity, commutativity, and distributivity of the operators
// involved". Two OpDot instructions in one module may therefore be lowered to different
// summation orders (or to different FMA-contracted forms) by the driver -- and DXC emits no
// Fma at all, so contraction is decided BELOW the .spv, where no byte- or disassembly-gate can
// see it. VB-P1e's coarse->fine enclosure proof needs the two call sites to evaluate the SAME
// function of their operands; correctly-rounded ops plus NoContraction (what `precise` emits)
// deliver exactly that, unconditionally. `precise` is on BOTH `d` and `sd` so that every node
// the monotonicity chain of the proof traverses -- the two OpFSub included -- is decorated;
// see docs/VB-P1E-HIERARCHICAL-CULL-PLAN.md section 5 step 2. It also makes the GPU match the
// host oracle `golden_sq_dist_point_aabb` (goldens.rs:3491), which accumulates `s += d*d` in
// the identical ((dx^2+dy^2)+dz^2) order and never fuses.
float sq_dist_point_aabb(float3 c, float3 aabb_min, float3 aabb_max) {
    precise float3 d  = max(max(aabb_min - c, c - aabb_max), 0.0.xxx);
    precise float  sd = d.x * d.x + d.y * d.y + d.z * d.z;
    return sd;
}
```

**Rev 4 changes which placement ships, and the reason is the proof, not the byte count.** Rev 3
prescribed `precise` on the scalar only and claimed a variant with `precise` on `d` "leaks,
decorating 16 ops including the AABB construction — do not use it". **That does not reproduce.**
Measured under the frozen recipe on the real shader:

| form | bytes | `NoContraction` | decorated opcodes | `OpDot` |
|---|---|---|---|---|
| committed today | 12 392 | **0** | — | 9 |
| `precise float sd` only (Rev 3's D10) | 12 592 | **5** | 3 `OpFMul` + 2 `OpFAdd` | 8 |
| **`precise float3 d` + `precise float sd` (Rev 4 ships this)** | **12 616** | **7** | **2 `OpFSub`** + 3 `OpFMul` + 2 `OpFAdd` | 8 |

There is **no leak into the AABB construction** in either form — the two added decorations are
exactly the two `float3` subtractions *inside* `sq_dist_point_aabb`, and the ext-inst histogram
(`NMax` 18, `NMin` 8, `Normalize` 4, `UMax` 2, `Pow` 2, `UMin` 1) is **identical across all three
modules**, so `precise` does not perturb the ray-gen or the AABB fold at all.

**Why the extra two decorations are worth 24 bytes.** §5 Step 2's monotonicity chain traverses
`OpFSub -> NMax -> NMax -> OpFMul -> OpFAdd -> OpFAdd`. Under the 5-decoration form the two `OpFSub`
are *undecorated*, and the proof then needs a side argument — "neither `OpFSub` operand is produced
by an `OpFMul`, so no contraction partner exists in this shader" — which is true today (`aabb_min` is
an `NMin` result, `c` is a load) but is a **property of the surrounding code, not of the function**,
and is exactly the kind of premise an id-normalised disassembly window cannot check (P1-3). The
7-decoration form deletes the side argument. Runtime cost is zero: `NoContraction` only *forbids*
fusion, and there is nothing here to fuse.

**Per-node audit of the chain (this is what P1-2 asked for, and it is now complete):**

| node in §5 Step 2 | SPIR-V | correctly rounded? | `NoContraction`? |
|---|---|---|---|
| `lo_j − c_j` | `OpFSub` (v3) | yes | **yes** (Rev 4) |
| `c_j − hi_j` | `OpFSub` (v3) | yes | **yes** (Rev 4) |
| inner `max` | `GLSL.std.450 NMax` | returns an operand exactly — no rounding | n/a (nothing to contract) |
| outer `max(·, 0)` | `GLSL.std.450 NMax` | returns an operand exactly | n/a |
| `d_j · d_j` (x3) | `OpFMul` | yes | yes |
| the two sums | `OpFAdd` | yes | yes |

Every node is either correctly rounded **and** contraction-forbidden, or returns one of its operands
bit-exactly. That is the whole of Premise P.

**`r * r` at `:168` stays undecorated deliberately:** it is a lone `OpFMul` whose result feeds only
`OpFOrdLessThanEqual`, so it has no contraction partner, and being correctly rounded it is
bit-identical at both sites for the same `L.range`.

**One newly measured hazard, recorded because it is the real form of the "leak" Rev 3 warned about.**
`precise` on `d` **propagates backwards into the argument expressions**. A two-call-site probe whose
second site passed `aabb_min * 1.5` emitted **16** decorations — 14 from the two function bodies plus
**2 `OpVectorTimesScalar` from the argument expression**. In §4 both call sites pass plain locals
(`coarse_min`/`coarse_max` and `aabb_min`/`aabb_max`), so the count must be exactly 14. **H2(e)'s
`NoContraction == 14` assertion is therefore also the guard against an accidental argument
expression** — which is why it is pinned as an exact integer and not a lower bound.

**Measured opcode census** (frozen recipe, dxc 1.4.350.0):

| module | `OpDot` | `NoContraction` | status |
|---|---|---|---|
| base, today | 9 | 0 | **measured** |
| base + D10 (7-decoration form) | **8** | **7** | **measured (M4)** |
| two call sites, plain-local args | 8 | **14** | **measured (M6)** |
| HIER + D10 | 8 | 14 | **predicted; H2 pins it** |

The 8 residual `OpDot`s are the `dot(rd, cam_forward.xyz)` in `view_z_to_t` (`:87`), 4 corners x
near/far — i.e. **zero `OpDot` in the cull comparison**.

**This also repairs an existing, currently-unbacked claim.** `golden_sq_dist_point_aabb`
(`crates/boyko_rhi_vulkan/src/goldens.rs:3491-3498`) accumulates `s += d * d` in Rust `f32` — never
fused, association `((dx^2+dy^2)+dz^2)`, i.e. exactly the tree D10 emits. Two shipped tests call
`golden_cluster_cull` "the bit-exact source of truth" / "bit-exact to what the GPU cull writes"
(`tests/sdf_gbuffer_hybrid.rs:5187-5188`, `:6291-6293`) and assert GPU `ClusterGrid` occupancy
froxel-for-froxel against it (`:5199-5202`). **Today that bit-exactness is an accident of NVIDIA's
lowering; after D10 it is structural.** The repo already wrote this argument down once, for DDGI
(`shaders/ddgi_resolve.hlsli:136-143`: "DXC by default CONTRACTS the blend MACs … `precise` forbids
contraction/reassociation … matching the host to bits").

**Rejected alternatives** (each was tested, not argued):

* ***`precise` on `dot()`.*** DXC does propagate it and decorates the `OpDot` itself
  (`OpDecorate %19 NoContraction` on a `%19 = OpDot`), and `spirv-val` accepts it. But SPIR-V defines
  `NoContraction` as constraining *combination across instructions*; it says nothing about the
  internal accumulation of a single `OpDot`, which the Vulkan precision rule continues to leave
  reassociable. **Validator acceptance is not a specified guarantee.** This is the trap that looks
  like a fix.
* ***The `spirv-dis` structural assertion as the discharge.*** Empirically satisfiable — the two
  sites are instruction-for-instruction identical even with plain `dot()` — but it proves the wrong
  thing. DXC emitted **zero `Fma`** in all 9 modules, so fusion happens in the driver's SPIR-V-to-ISA
  pass, strictly below what `spirv-dis` can see. A gate asserting "both sites are `OpDot`" is
  compatible with a driver lowering one as `FFMA(dy,dy,FFMA(dx,dx,0))` and the other as
  `dx*dx + (dy*dy + dz*dz)`. It would make the proof true *on today's compiler*, not unconditional.
  **Retained in H2 as a tripwire only.**
* ***Written-out sum without `precise`.*** Both sites emit `FMul, FMul, FAdd, FMul, FAdd`
  identically, but with **no** `NoContraction` — a driver may contract one site and not the other.
  Not sufficient alone.
* ***An owned bounded slack on the coarse comparison only.*** Viable, and retained as the **named
  fallback** if H1.6 measures a regression. The bound is derivable: `OpDot`'s legal return set is
  `{v : |v−e| <= |E−e|}` for exact `e` and any correctly-rounded association `E`; for 3 products +
  2 sums, `|E−e|/e <= (1+u)^5 − 1` which is about `5u` with `u = 2^-24`, so the coarse comparison
  needs relative slack `>= 10u = 5.96e-7`, and `r*r*(1.0 + 0x1p-20)` (`9.5e-7`) covers it with 1.6x
  margin. Its selectivity cost is nil (a 10 m light grows by 9.5 micrometres against a coarse box
  spanning a whole z-slice). It loses on rigour: it reintroduces the epsilon §5 boasts of not
  needing; the `5u` figure is inferred from spec *prose* rather than a spec-stated ULP number for
  `OpDot`; and it leaves the GPU-vs-host bit-exactness claim still unbacked.
* ***A per-axis coarse test (`d.x*d.x <= rr && …`) leaving the fine test untouched.*** Attractive
  because it would leave the base `.spv` and every existing golden alone. Its soundness argument
  ("a sum of non-negatives is at least each of its terms") survives any association order *and* FMA
  contraction — but **not** `OpDot`'s licensed accuracy interval: a conforming `OpDot` may return up
  to about `5u` *below* the exact sum of squares, hence below `fl(d_x^2)`, and then fine accepts while
  coarse rejects. It still requires the fine site to stop being an `OpDot`, at which point D10 has
  already been paid for.
* ***An `#ifdef HIER`-only `precise` distance function, leaving the base arm bit-frozen.*** Breaks
  D4: D4.1 needs the HIER fine test to compute the same value as the base arm's test. The shared
  function must change for both arms; the base re-pin is the honest cost.

### D11 — The total bound is **boot-sourced**, and the capacity is **pushed, not re-derived** `[P0-B]`

**Invariant.** *The dispatch size, the `ClusterGrid` allocation and the shader's write bound are
three evaluations of one `u32`, minted once in `build_froxel_light_cull`.*

**Rev 4 makes that sentence literally true.** Rev 3 asserted it while the shader re-derived
`capacity = bdx * bdy * bdz` from three **8-bit** header-pack fields, whereas the host allocated from
full-precision `cluster_config.cluster_count()` (`gpu_scene/mod.rs:4317`,
`crates/boyko_render/src/light.rs:728-730`). The `<= 255`-per-dim contract that makes those two agree
is a **`debug_assert!` only** (`light.rs:763-769`), with no masking on the OR, so in a release build
with `dim_x = 300` the pack silently aliases and `capacity` stops describing the buffer. Out of
contract today (there is no production `ClusterConfig` writer) and the same assert already governs
the shipped base path — so this is **not a new hazard** — but a P0 discharge must not rest on a
debug-only guard. Rev 4 does both cheap fixes:

1. **`cluster_capacity: u32` becomes a second HIER push word**, minted from the *same*
   `cluster_config.cluster_count()` binding that sizes the buffer. The shader uses it directly. Push
   goes 16 B (base) / 20 B (Rev 3 HIER) to **24 B (Rev 4 HIER)** — against a shared COMPUTE range of
   **80 B** (`COMPUTE_PUSH_CONSTANT_RANGE_BYTES = COMPOSITE_PUSH_CONSTANT_BYTES == 80`,
   `compute.rs:2956`), so there is ample headroom.
2. **The `<= 255` contract is promoted to a release `assert!`** in `build_froxel_light_cull`. It is a
   boot-time, once-per-process check on a setup path — Principle 1 is not engaged. The push word
   keeps the *write bound* safe even out of contract; the assert keeps the *mapping* honest, which
   the push word does not.

**Why the boot/live split is needed at all.** The two sources genuinely diverge:

* **BOOT.** `runner.rs:636-643` reads `ClusterConfig` from the World once
  (`try_resource::<ClusterConfig>().copied().unwrap_or_default()`) and passes it to
  `build_froxel_light_cull` (`gpu_scene/mod.rs:4241`), which sizes `ClusterGrid` at
  `cluster_count * 8` bytes (`:4317-4324`) and freezes `self.cluster_count` (`:4346`).
* **LIVE.** `sync_cluster_light_gate` (`light.rs:830-851`) runs **every frame**, reads
  `Res<ClusterConfig>`, and writes `cfg.cluster_packed_dims` into the light header, which the shader
  would read via `load_cluster_params` (`light_table.hlsli:313-323`). Its own doc-comment says it is
  "stale the moment the owner changes the grid/near/far without also touching a light"
  (`light.rs:783-786`). The arm bit `ResolvedRenderPath::froxel_light_cull` is boot-frozen
  (`light.rs:792-793`) while the **dims stay live** — the skew vector is real and documented.

`ClusterConfig` has **no production writer today** (a repo-wide grep for `ResMut<ClusterConfig>` /
`resource_mut::<ClusterConfig>` / `insert_resource(ClusterConfig` returns 4 hits, all
`insert_resource` at App-build time: `plugins.rs:195` plus three test setups), but the Resource is
`pub` and world-mutable, so one owner system reaches it. The campaign has already ruled on this
hazard class **for this exact buffer**: `plugins.rs:352-364` justifies the gate's
`.before_set(LightCollectSet)` edge because a stale dims lane "would then underflow to an
out-of-bounds `ClusterGrid` index — real GPU UB with `robust_buffer_access` disabled" (same wording
at `light_system.rs:410`).

**Measured bounds** (host simulation over the boot/live matrix; buffer = boot `cluster_count`,
dispatch = host-derived from boot; `max fi` written vs capacity):

| case (boot to live) | buffer | base arm | D3 as Rev 2 wrote it | D3 + `fi < cluster_count` (live) | **D3 + D11** |
|---|---|---|---|---|---|
| 16x9x24 to 16x9x24 | 3456 | 3455 ok | 3455 ok | 3455 ok | 3455 ok, dup 0, unwritten 0 |
| 16x9x24 to **32x18x24** | 3456 | 3455 ok | **13 807 OOB** | **13 807 OOB** | 3455 ok, dup 0, unwritten 0 |
| 16x9x24 to 16x9x48 | 3456 | 3455 ok | **6 887 OOB** | **6 887 OOB** | 3455 ok, dup 0, unwritten 0 |
| 32x18x24 to 16x9x24 | 13824 | 3455 ok | 3503 ok | 3455 ok, **3 432 aliased cells** | 13823 ok, dup 0, unwritten 0 |
| **16x9x23 to 16x9x24** | 3312 | **3327 OOB** | **3454 OOB** | **3454 OOB** | 3311 ok, dup 0, unwritten 0 |
| 16x9x24 to 0x0x0 | 3456 | −1 (early return) | **div-by-0** (`gps==0`) | **div-by-0** | 3455 ok, dup 0, unwritten 0 |

Three results the design must account for: `13 807` reproduces the critic's worked example exactly;
**the naive transplant `fi < cluster_count` does not fix it** (13 807 < live 13 824), proving the base
arm's guard is the wrong *shape* for a re-derived `fi` — the base arm is safe only because
`fi = tid.x` is bounded by the *dispatch*, and D3 deletes that bound; and the degenerate-header
divide-by-zero is created purely by D8's "no early return" (D8 carries both guards). The
`16x9x23 to 16x9x24` row is also **the state §8.3's re-specified mutation (v) reproduces** — it is
what makes that mutation non-inert, and the row is cited there rather than left as background.
The same row exposes the pre-existing base-arm hole tracked as VB-P1j.

**Transport — `#ifdef HIER`-only push tail words.**

```hlsl
struct ClusterCullPush {
    float z_near; float z_far; uint max_lights_per_cluster; uint index_list_cap;
#ifdef HIER
    uint cluster_dims_packed;   // BOOT snapshot: dim_x | dim_y<<8 | dim_z<<16   (the MAPPING)
    uint cluster_capacity;      // BOOT cluster_count() in FULL precision        (the WRITE BOUND)
#endif
};
```

Measured on the Rev 3 probe (one tail word): the `-D HIER=1` push block has 5 members with
`Offset 16` on `cluster_dims_packed`, and the no-`-D` compile is byte-identical to the committed blob.
Rev 4 adds a sixth member at `Offset 20`; **H2 gate (b) re-measures the no-`-D` byte-identity rather
than inheriting Rev 3's measurement**, because the struct changed. Widening the *shared* struct
instead — no `#ifdef` — was also measured: it changes the base module's push-constant block type and
would fail H2 gate (b).

**Precedent (this is why a push is the right transport):** the push *already* carries a boot-snapshot
buffer capacity used as a device-write bound — `pc.index_list_cap` clamps the `LightIndexList`
scatter at `cluster_cull.hlsl:184-190`, and that same `cluster_config.index_list_cap` sized the
buffer at `gpu_scene/mod.rs:4325-4331`. `cluster_capacity` is that identical pattern applied to
`ClusterGrid`, which today has no such bound. *Rejected:* a specialization constant — the RHI exposes
it (`ComputePipelineDesc { spec_constants: &[] }`, `gpu_scene/mod.rs:4304`) and its lifetime is
arguably better (baked at pipeline create), but it introduces a second transport mechanism to review
for 8 bytes and cannot be const-asserted the way push offsets are (`compute.rs:3467-3471`).
Re-openable as an output-neutral follow-up. *Rejected:* `vkCmdDispatchIndirect` with a GPU-computed
group count — it deletes the skew at the root but pays a new device buffer, a new framegraph resource
with a seeding decision and a new barrier against §1.2's **13.9 microsecond fixed cost**, which is
70 % of the measured 19.7 microsecond cull at `N_ps=8`. *Rejected:* doing nothing on the grounds that
no production writer exists — the base arm carries its bound unconditionally, and the fix is three
comparisons on a path whose cost model is "(froxel, light) pair tests and nothing else" (§1.1).

**Host plumbing (Principle 0: one derived accessor + one activation struct, no side store).**

* `crates/boyko_rhi_vulkan/src/compute.rs`: a **second** `#[repr(C)]` mirror
  `ClusterCullHierPush { z_near, z_far, max_lights_per_cluster, index_list_cap, cluster_dims_packed,
  cluster_capacity }` + `CLUSTER_CULL_HIER_PUSH_BYTES = 24`, with the same `offset_of!`
  const-asserts as `ClusterCullPush` (`compute.rs:3467-3471`), plus
  `const _: () = assert!(CLUSTER_CULL_HIER_PUSH_BYTES <= COMPOSITE_PUSH_CONSTANT_BYTES);`.
  **`ClusterCullPush` (16 B) is not widened** — the base pipeline's push range and the base `.spv`
  stay as they are.
* `crates/boyko_render/src/light.rs`, beside `cluster_count()` (`:728`):
  `pub const fn hier_group_threads() -> u32 { 256 }` and
  `pub const fn hier_group_count(&self) -> u32 { ((self.dim_x * self.dim_y + 255) / 256) * self.dim_z }`.
  *(Rev 5 P2 fix: Rev 4 wrote `self.dim_x.mul(self.dim_y).div_ceil(256)`, which does not compile in a
  stable `const fn` — `mul` is a trait method, and `div_ceil`'s const-stability is a separate question
  this plan should not depend on. The arithmetic form has a second, better property: it is the shader's
  `gps = (bdx * bdy + 255u) / 256u` **token-for-token**, so the host/shader mirror is checkable by eye.
  The shader's extra `max(1u, …)` has no host counterpart on purpose — with `dim_x * dim_y == 0` this
  host dispatches **zero** groups, so the shader's guard is unreachable from here; it exists to make the
  SHADER total against a hand-written dispatch, and D8 obligation 3 keeps it.)*
* `crates/boyko_rhi_vulkan/src/present/scene_types.rs`: **one** new `GBufferScene` field in the
  existing activation-struct idiom (`BrickActivation` `:438`, `ViewtFromDepthActivation`,
  `ViewtFromVbDepthActivation`):
  ```rust
  pub struct ClusterCullHierDispatch { pub groups: u32, pub push: [u8; CLUSTER_CULL_HIER_PUSH_BYTES as usize] }
  /// `Some` IFF the HIER variant is the pipeline in `cluster_cull`.
  pub cluster_cull_hier: Option<ClusterCullHierDispatch>,
  ```
  `cluster_count` and `cluster_cull_push` are untouched, so the base arm is byte-identical. The four
  test literals (`window_present_gbuffer.rs:2387, 3434, 8420, 9905`) each gain
  `cluster_cull_hier: None`.
* `crates/boyko_app/src/gpu_scene/mod.rs`: `build_froxel_light_cull` (`:4241`) is the **single
  writer** — it already receives `cluster_config: ClusterConfig` and already sizes `cluster_grid`
  from `cluster_config.cluster_count()` (`:4317-4324`). It stores the activation beside the existing
  `self.cluster_count = cluster_count` (`:4346`) and `scene()` threads it beside `cluster_count`
  (`:5237`, `:5307`). **The `cluster_count` local at `:4317` is the single mint point** — the buffer
  size, `h.groups` and `push.cluster_capacity` all read that one binding, which is what makes the
  invariant checkable by inspection of ten adjacent lines.
* Record sites (`vb.rs:215`, `gbuffer.rs:1583`, `forward.rs:359`) — **one `match`**, so the group
  count, push pointer and push length can never be mixed across arms:
  ```rust
  let (cull_groups, push_ptr, push_len) = match scene.cluster_cull_hier.as_ref() {
      Some(h) => (h.groups, h.push.as_ptr(), CLUSTER_CULL_HIER_PUSH_BYTES),
      None    => (scene.cluster_count.div_ceil(LIGHT_CULL_LOCAL_SIZE_X),
                  scene.cluster_cull_push.as_ptr(), CLUSTER_CULL_PUSH_BYTES),
  };
  ```

**Why a boot/live disagreement is harmless FOR THIS DISPATCH — and what it is NOT harmless for.**
`groups`, `gps` and `capacity` are three evaluations of the same u32; the HIER arm never reads the
header's dims lane, so a live `ClusterConfig` edit **cannot move `fi` at all**. In debug it is caught
at the per-frame `scene()` call site (`runner.rs:1951`, which already holds `world`) with the same
host-authoritative-lock pattern the SSAA arm uses twelve lines above (`runner.rs:1919-1940` —
"resolution is a boot commitment … so the per-frame mode MUST agree with it, never the reverse"):
`debug_assert_eq!(world.try_resource::<ClusterConfig>().map(|c| c.packed_dims()), Some(boot_packed_dims), "invariant: ClusterConfig dims are a boot commitment (cull buffers are boot-sized)")`.

> **Scope correction (P1-5b) — Rev 3's "the HIER arm cannot fault … only mis-shape the grid" is
> true of the cull's own writes and FALSE at frame level.** All four `ClusterGrid` **consumers**
> index with the **live** header dims behind only a non-zero test —
> `vb_resolve.comp.hlsl:352-359`, `vb_shade.comp.hlsl:520-527`, `deferred_pbr.hlsl:1236-1237`,
> `forward_opaque.fs.hlsl:332-333` (each is `cluster_linear_index(tile.x, tile.y, zsl, cp.dim_x,
> cp.dim_z)` followed by an unbounded `ClusterGrid[cluster]`). If the live dims *grow* past the boot
> dims, `cluster` exceeds the boot-sized allocation and the read is out of range with
> `robustBufferAccess` off. This is **pre-existing and not introduced by VB-P1e** — the repo names the
> class in the consumers' own comments (`vb_resolve.comp.hlsl:343-348`) and at `plugins.rs:355-361` —
> and VB-P1e changes nothing about it. But it means the release-silence above is a **frame-level
> safety gap, not a cosmetic one**, and **VB-P1k is re-filed in §11 as a safety follow-up rather than
> an "Owner/VALUES call"**. The cheapest closure inside this rung is H3's no-skew assertion, so no
> test can silently run skewed.

**The `// SAFETY:` comment at the VB record site must be rewritten.** Today it reads (verbatim,
`vb.rs:216-221`, and byte-for-byte identically at `gbuffer.rs:1587-1588` and `forward.rs:363-364`):

```
// SAFETY: recording is open; the cull pipeline + its layout (declaring `cull_layout`
// at set 0 + the 16-byte COMPUTE push range) are live on this device (caller
// contract); the cull set binds the camera UBO + light table + the cluster buffers;
// `cull_groups` covers `cluster_count` froxels at the 64-wide group; the push bytes
// are exactly `CLUSTER_CULL_PUSH_BYTES` (16) at offset 0; ...
```

"`cull_groups` covers `cluster_count` froxels" is a **coverage** property — the wrong obligation for
an `unsafe` FFI block, and false for the HIER map. Replacement clause:

```
// the dispatch size and the push image are the SAME `Option` arm (base: `cluster_count`
// froxels at the 64-wide group + the 16-byte `ClusterCullPush`; hier: `h.groups` groups of
// 256 + the 24-byte `ClusterCullHierPush`), so the group count can never be paired with the
// other arm's push range; NO invocation of either arm can write outside `ClusterGrid`,
// because every `ClusterGrid[fi]` write is guarded by `fi < pc.cluster_capacity`, and
// `cluster_capacity` is the BOOT `ClusterConfig::cluster_count()` the buffer itself was
// allocated from (`gpu_scene/mod.rs:4317-4324`, same binding) -- never the live header's dims
// lane, which `sync_cluster_light_gate` (`light.rs:830`) may move behind this dispatch's back.
// SCOPE: this bounds THIS dispatch's writes only. The ClusterGrid *readers*
// (vb_resolve/vb_shade/deferred_pbr/forward_opaque) still index with the live dims and carry
// the pre-existing skew exposure tracked as VB-P1k.
```

The base arm's clause must **also** be tightened, because today's wording over-claims for a
non-64-aligned grid (VB-P1j). H4 fixes `vb.rs`; H5 fixes the other two.

---

## 4. Shader structure (both arms, one file)

```hlsl
// cluster_cull.hlsl — base arm unchanged EXCEPT the shared D10 distance function;
// the HIER arm is compiled in only under -D HIER=1.
#ifdef HIER
#define HIER_TPG        256u        // host mirror: ClusterConfig::hier_group_threads()
#define HIER_MASK_WORDS 32u         // MAX_LIGHTS / 32  (D6, pinned EQUAL)
#define HIER_MASK_BITS  (HIER_MASK_WORDS * 32u)
#define HIER_FMAX       asfloat(0x7F7FFFFFu)   // +FLT_MAX exactly, by bit pattern (D8, section 5 B)
#if (HIER_MASK_WORDS) > 32
#error "HIER_MASK_WORDS > 32: gs_summary is a SINGLE uint, one bit per mask word"
#endif
#if (HIER_MASK_WORDS) > (HIER_TPG)
#error "HIER_MASK_WORDS > HIER_TPG: phase 1 inits exactly one mask word per lane"
#endif
#if (HIER_TPG) != 256u
#error "HIER_TPG != 256: D9's radix-16 fold hardcodes 16 folding lanes x 16 entries == 256"
#endif
groupshared float gs_min_x[HIER_TPG], gs_min_y[HIER_TPG], gs_min_z[HIER_TPG];
groupshared float gs_max_x[HIER_TPG], gs_max_y[HIER_TPG], gs_max_z[HIER_TPG];
groupshared uint  gs_mask[HIER_MASK_WORDS];
groupshared uint  gs_summary;       // bit j <=> gs_mask[j] != 0
[numthreads(256, 1, 1)]
#else
[numthreads(64, 1, 1)]
#endif
void main(uint3 tid : SV_DispatchThreadID, uint3 gid : SV_GroupID, uint lane : SV_GroupIndex)
```

`HIER_TPG`/`HIER_MASK_WORDS` are `#define`, not `static const uint`, because the `#if`/`#error`
guards need preprocessor constants. The **shared** 3-parameter signature is verified byte-neutral
for the base compile (D5), so it is not `#ifdef`-split.

**Phase −1 — the group-uniform prologue** (before phase 0, before any barrier). Evaluate D7's clamp
(`ps_begin`, `ps_room`, `ps_total`, `ps_n`) from header words 0 and 2 — **not** from
`point_spot_count` (word 3), because the base arm's range is `[l0a_count, light_count)`
(`cluster_cull.hlsl:161`). Evaluate D3/D11's mapping (`bdx/bdy/bdz`, `capacity` from the push,
`gps`, `slice`, `s`, `x/y/z`, `fi`, `valid`) from the push. Both are group-uniform: every lane reads
the same header words and the same push.

Phases of the `HIER` arm (each lane; `valid` per D8):

| # | Work | Barrier after | Cost |
|---|---|---|---|
| 0 | **unchanged** froxel AABB build — 4 `generate_ray` x {near, far} + `view_z_to_t` + `expand_aabb` (`:126-153`), only when `valid` | — | 8 unprojections |
| 1 | `finite = all(abs(aabb_min) <= 1e30) && all(abs(aabb_max) <= 1e30)`; store the 6 scalars per the D8 table — own AABB when `valid && finite`, the **absorbing** `(−FLT_MAX, +FLT_MAX)` when `valid && !finite`, the **identity** `(+1e30, −1e30)` when `!valid`; lanes 0..31 set `gs_mask[lane] = 0`, lane 0 sets `gs_summary = 0` | **B1** | 6 stores + 6 SPIR-V ops/lane (measured, M11) |
| 2 | radix-16 in-place fold: lanes 0..15 each serially fold `gs[l + 16k], k = 0..15` into `gs[l]` (D9) | **B2** | 32 min/max x 6 on 16 lanes |
| 3 | **every** lane folds `gs[0..16)` into registers `coarse_min`/`coarse_max` — broadcast reads, no write. This is what makes the coarse box **group-uniform** (D8 review item 6, §5 Setup) | — | 16 reads x 6 |
| 4 | coarse scan: `for (uint j = lane; j < ps_n; j += HIER_TPG)` then `load_light(LightBuf, ps_begin + j)`, `light_kind` filter, `sq_dist_point_aabb(CL.pos, coarse_min, coarse_max) <= cr*cr`, then `InterlockedOr(gs_mask[j >> 5], 1u << (j & 31u))` **and** `InterlockedOr(gs_summary, 1u << (j >> 5))`. **All 256 lanes, `valid` or not.** | **B3** | `ceil(ps_n/256)` per lane |
| 5 | fine walk (**`valid` only**): for each set bit of `gs_summary` ascending, for each set bit of `gs_mask[w]` ascending, `j = (w<<5)\|b`; **`if (j >= ps_n) continue;` (D7)**; `i = ps_begin + j`; then the **token-identical** fine test + `local[]` append (`:161-175`) | — | `E_coarse` tests |
| 6 | **unchanged** `InterlockedAdd` claim + scatter + `ClusterGrid[fi] = uint2(offset, write_count)` (`:180-194`), **`valid` only** — and it is `fi < pc.cluster_capacity` (D11) that makes the write in-bounds | — | 1 atomic |

**Barriers: 3 total (B1, B2, B3).** This footer exists so the count cannot drift from D1 again.
Rev 2's table summed to 11 (1 + 8 + 1 + 1); the radix-16 fold removes 7 and folding the summary bit
into phase 4's atomic removes the 8th along with a whole phase. The extra groupshared atomic in
phase 4 fires only on an *accepted* coarse light — rare by construction, which is the whole premise.

**ONE gating predicate, and Rev 4 fixed an inconsistency here.** Rev 3's phase 1 excluded
non-`contrib` (= non-finite) lanes from the reduction while phases 5 and 6 gated on `valid`, so a
lane could be excluded from the fold and still run its own fine test against a coarse box that
provably did not enclose it — which falsified §5's own "for EVERY froxel in that group". Rev 4
removes the second predicate entirely: **finiteness selects which constant a lane stores, never
whether a lane participates.** Phases 1, 5 and 6 all gate on `valid`, and a non-finite lane's
absorbing store makes the coarse box enclose *everything*, so the quantifier is restored by
construction rather than by scope-clause. See §5.

**Exact HLSL for the two hot loops** (so the review gate reads code, not prose):

```hlsl
// --- phase 1, the substitution (D8's table, verbatim) --------------------------------
// The ABSORBING element is +/-FLT_MAX (HIER_FMAX, exact by bit pattern -- no decimal
// literal has to be trusted to round to it). The FINITENESS THRESHOLD on the line below
// is 1e30 and is a DIFFERENT constant with a different job: it classifies the lane, and
// it is what makes the !valid identity a true identity. Do NOT unify the two -- section 5
// Case B derives why +/-1e30 as the absorbing element inverts enclosure for a finite
// light centre with |c| > 1e30.
bool finite = all(abs(aabb_min) <= 1.0e30) && all(abs(aabb_max) <= 1.0e30);
float3 store_min, store_max;
if (!valid)          { store_min = ( 1.0e30).xxx;    store_max = (-1.0e30).xxx;    }  // identity
else if (!finite)    { store_min = (-HIER_FMAX).xxx; store_max = ( HIER_FMAX).xxx; }  // ABSORBING
else                 { store_min = aabb_min;         store_max = aabb_max;         }
gs_min_x[lane] = store_min.x; /* ... 6 scalar stores ... */
if (lane < HIER_MASK_WORDS) { gs_mask[lane] = 0u; }
if (lane == 0u)             { gs_summary   = 0u; }
GroupMemoryBarrierWithGroupSync();                              // B1

// --- phase 4, coarse ---------------------------------------------------------------
for (uint j = lane; j < ps_n; j += HIER_TPG) {
    LightElem CL = load_light(LightBuf, ps_begin + j);
    uint ck = light_kind(CL);
    if (ck != LIGHT_KIND_POINT && ck != LIGHT_KIND_SPOT) { continue; }
    float cr = CL.range;
    if (sq_dist_point_aabb(CL.pos, coarse_min, coarse_max) <= cr * cr) {
        // j < ps_n <= ps_room <= HIER_MASK_BITS == HIER_MASK_WORDS*32  =>  (j>>5) < HIER_MASK_WORDS
        InterlockedOr(gs_mask[j >> 5], 1u << (j & 31u));
        InterlockedOr(gs_summary, 1u << (j >> 5));
    }
}
GroupMemoryBarrierWithGroupSync();                              // B3

// --- phase 5, fine -----------------------------------------------------------------
uint summary = gs_summary;
while (summary != 0u) {
    uint mw = firstbitlow(summary);
    summary &= ~(1u << mw);
    uint bits = gs_mask[mw];
    while (bits != 0u) {
        uint mb = firstbitlow(bits);
        bits &= ~(1u << mb);
        uint j = (mw << 5) | mb;
        if (j >= ps_n) { continue; }        // D7: the SAME bound phase 4 wrote under
        uint i = ps_begin + j;
        LightElem L = load_light(LightBuf, i);
        uint k = light_kind(L);
        if (k != LIGHT_KIND_POINT && k != LIGHT_KIND_SPOT) { continue; }
        float r = L.range;
        if (sq_dist_point_aabb(L.pos, aabb_min, aabb_max) <= r * r) {
            if (nlocal < pc.max_lights_per_cluster && nlocal < 256u) {
                local[nlocal] = i; nlocal += 1u;
            }
        }
    }
}
```

**Note the substitution's two-branch shape is deliberate and is a review-gate item.** `!valid` and
`valid && !finite` must not be merged; D8's table gives the two constants and the two detectors.
Also note the ordering: the `finite` test reads `aabb_min`/`aabb_max`, which are only *built* under
`valid` in phase 0 — an invalid lane's values are the `(1e30, −1e30)` initialisers from `:143-144`,
which satisfy `abs(v) <= 1e30`, so `finite` is `true` for them and the `!valid` branch wins first.
The branch order is therefore load-bearing and must stay as written.

Structurally verified on the Rev 3 probe: the coarse write lowers to
`OpShiftRightLogical %uint %j %uint_5` then `OpAccessChain %gs_mask` then `OpAtomicOr`, with **no
clamp instruction at the write site** — the bound is entirely the loop condition, which is the
"locally evident" form D7 asks for. `%gs_mask` is declared as
`OpVariable %_ptr_Workgroup__arr_uint_uint_32 Workgroup`, adjacent to `gs_min`/`gs_max`/`gs_summary`
in the same storage class, which is exactly why an unclamped word write would silently corrupt the
coarse box rather than fault.

**What is token-identical, and what that buys.** Phases 0, 5-tail and 6 are *token*-identical to the
base arm — including the D10 `sq_dist_point_aabb`, which is the **same shared function** for both
arms and both levels. That is what makes D4's byte-identity a construction rather than a hope, and
after P0-A it is **load-bearing for the §5 proof**, not merely convenient. Two honest qualifications:
phase −1 substitutes `bdx/bdy/bdz` for `cp.dim_x/…`, which is *value*-identical under D4 scope clause
(b) (both are the same 8-bit fields of the same encoding); and phase 6's memory **access pattern** is
not identical (D3 trade-off 3).

**Host side.** `GBufferScene` carries `cluster_count: u32` and **no dims**
(`present/scene_types.rs:1409-1411`), and `ClusterConfig` is a `boyko_render` Resource not reachable
from the RHI crate — so "the three record sites dispatch `hier_group_count()`" could not be written.
The replacement is D11's plumbing in full: `ClusterConfig::hier_group_threads()/hier_group_count()`
beside `cluster_count()` (`light.rs:728`); one `GBufferScene` field
`cluster_cull_hier: Option<ClusterCullHierDispatch>`; written **only** in `build_froxel_light_cull`
(`gpu_scene/mod.rs:4241`, beside `:4346`); consumed by one `match` at each of `vb.rs:215`,
`gbuffer.rs:1583`, `forward.rs:359`; four test literals updated. The host-to-shader pin test is a
**pure-arithmetic CPU test** (H1 assertion 7) — and it is a **Rust re-implementation of the shader's
walk, not a pin on the HLSL**: if the shader and the mirror drift, only H3 (device) sees it. Rev 4
labels it that way rather than letting §9 imply the HLSL is pinned.

---

## 5. The exactness proof (re-derived against the real code and the **real emitted instructions**)

**Claim.** If the coarse test rejects light `L` for a group, then the fine test rejects `L` for every
`valid` froxel in that group — in IEEE-754 arithmetic, with no epsilon.

**Setup.** Lane `i` computes `(min_i, max_i)` by expanding from `(+1e30, −1e30)` over 8 world points
`ro + rd·t`, where `(ro, rd) = generate_ray(...)` (`ray_gen.hlsli:44`) and
`t = view_z_to_t(slice_view_z(z|z+1), rd)` (`cluster_cull.hlsl:77-91`). What each lane **stores** is
D8's three-way substitution (§4 phase 1), not necessarily its own AABB. The group values are
`MIN = min_i(stored_min_i)`, `MAX = max_i(stored_max_i)`, componentwise (D2/D9).

**Setup premise, now stated (P2).** After phase 3 `coarse_min`/`coarse_max` are **group-uniform** —
every lane folds the same 16 published slots and lands the same registers. The proof quantifies over
"every froxel in the group" and therefore *needs* all lanes to be testing against one box; §4 phase 3
delivers it, D8 review item 6 gates it. Rev 3 presented this as a performance choice and never listed
it as a premise.

**Step 0 — the evaluation function is spec-determined (P0-A's discharge, preserved from Rev 3).**
Under D10, `sq_dist_point_aabb` computes the explicit tree

```
F(d) = ((fl(d.x·d.x) + fl(d.y·d.y)) + fl(d.z·d.z))
```

as `OpFMul`/`OpFMul`/`OpFAdd`/`OpFMul`/`OpFAdd`, every node of which Vulkan's *Precision and
Operation of SPIR-V Instructions* specifies as **"Correctly rounded"** — one legal fp32 result — and
every node of which carries `NoContraction` (emitted by `precise`). **Therefore both call sites
evaluate one function `F`, not two schemes**, and the link the critic identified as missing —
`A(d_fine) <= B(d_fine)` for a coarse scheme `A` and a fine scheme `B` — is **vacuous, because
`A == B == F`.** D10's per-node audit table is the artifact; the measured decoration counts are
M3/M4/M6.

*The counter-fact that motivates it.* `dot(d,d)` lowers to a single `OpDot`, whose Vulkan precision
is only **"inherited from"** a formula, and the same appendix permits that formula to *"be
transformed using the mathematical associativity, commutativity, and distributivity of the operators
involved to yield an equivalent formula"*. Two `OpDot` instructions in one module may therefore be
lowered to different summation orders. And **DXC emits zero `Fma`** in every variant measured (9
modules), so contraction is a *driver-side* decision at SPIR-V-to-ISA lowering — invisible to any
`.spv` byte gate and to any `spirv-dis` gate. That is why a structural gate alone **cannot** discharge
this, and why H2's gate is labelled a tripwire.

*`r*r` needs no decoration:* it is a lone `OpFMul` feeding only `OpFOrdLessThanEqual`, so it has no
contraction partner, and being correctly rounded it is bit-identical at both sites for the same
`L.range`.

**Step 1 — the reduction is exact, and (Rev 4) it never sees a NaN.** `min`/`max` on floats introduce
no rounding: the result is one of the inputs. Measured (M2/M5) — and this is the fact Rev 3 got
wrong — HLSL `min`/`max` lower here to **`GLSL.std.450 NMin`/`NMax`** (`NMin` 8, `NMax` 18), **not**
`FMin`/`FMax` (both 0), in the committed module and in every `precise` variant.

Because §4 phase 1 stores one of exactly **three** values — the `!valid` identity `(+1e30, −1e30)`,
the non-finite lane's absorbing `(−FLT_MAX, +FLT_MAX)`, or a lane's own **finite** AABB — **every
value entering the fold is finite**, so `NMin`/`NMax` are the exact componentwise extremum with no
appeal to their NaN clauses at all. *(Rev 6 P2-2: Rev 5 introduced the `±FLT_MAX` store in the same
pass that fixed this proof but left the enumeration here at two values. `FLT_MAX` is finite, so the
conclusion is unchanged; the enumeration was simply incomplete inside the proof it belongs to. All
three are read off the phase-1 HLSL at §4 — `store_min`/`store_max`'s three branches.)* Hence, componentwise and exactly, `MIN_j <= stored_min_{i,j}` and
`MAX_j >= stored_max_{i,j}` for every lane `i` and axis `j`. Fold order is irrelevant: `min`/`max`
are exactly associative and commutative, so D9's radix-16 shape is as sound as any tree, and D2's
corollary already covers it. (Sign-of-zero: when a component holds both `+0.0` and `−0.0` the
returned sign is unspecified; immaterial, since they compare equal everywhere downstream. D9 states
the same exception in the same words.)

**Step 2 — `F` is monotone in the box.** With `d_j = max(lo_j − c_j, c_j − hi_j, 0)` and result
`F(d)`:

* `lo_j <= lo'_j` implies `fl(lo_j − c_j) <= fl(lo'_j − c_j)` — `OpFSub` is correctly rounded, and
  IEEE-754 round-to-nearest is **monotone** (a <= b implies `fl(a) <= fl(b)` for the same operation
  and rounding).
* `hi_j >= hi'_j` implies `fl(c_j − hi_j) <= fl(c_j − hi'_j)`, same reason.
* `NMax(·, ·)` is monotone and returns an operand exactly; the result is non-negative.
* `d_j >= 0`, and `x -> fl(x·x)` is monotone on non-negatives; `fl(a + b)` is monotone in each
  argument.

Therefore, componentwise `d_coarse <= d_fine` and hence `F(d_coarse) <= F(d_fine)` **as computed**,
not merely in exact arithmetic. **Every node in that chain is covered by D10's audit table** — the
two `OpFSub` included, which is why Rev 4 moved `precise` onto `d`.

**Step 3 — conclusion, by cases on the group.**

* **Case A — every `valid` lane in the group is finite.** Then each lane stored its own AABB (or, for
  `!valid` lanes, the identity, which cannot narrow the box). By Step 1 the coarse box encloses every
  `valid` lane's AABB; by Step 2, fine accepts implies `F(d_coarse) <= r·r`, i.e. coarse accepts.
  Contrapositive: coarse rejects implies fine rejects, for **every** `valid` froxel in the group.
* **Case B — at least one `valid` lane is non-finite.** That lane stored the **absorbing**
  `(−FLT_MAX, +FLT_MAX)`, so `MIN_j = −FLT_MAX` and `MAX_j = +FLT_MAX` on every axis. For any
  **finite** light centre `c` we have `−FLT_MAX <= c_j <= FLT_MAX` by definition of `FLT_MAX`, hence
  `−FLT_MAX − c_j <= 0` (or `−inf` on overflow, still `<= 0`) and `c_j − FLT_MAX <= 0`, so
  `d_j = NMax(NMax(−FLT_MAX − c_j, c_j − FLT_MAX), 0) = 0` and `F(d) = 0 <= r·r` for every `r` **whose
  `r·r` is not NaN**. **The coarse test accepts every such punctual light with `j < ps_n`, so its
  hypothesis is never satisfied and the implication holds vacuously.**

  > **Rev 6 P1 fix — the `for every r` was false for a REACHABLE input, and the conclusion survives
  > anyway.** The cull compare is the **ordered** `OpFOrdLessThanEqual` (measured: the committed module
  > carries exactly one, and §5.1's M11 census distinguishes it from the `%v3bool` finiteness compares
  > by result type), so `0.0 <= NaN` is **FALSE** and at `r·r = NaN` the coarse test **rejects**.
  > `L.range` is not validated anywhere on the host path: `PointLight::new` stores the caller's `range`
  > verbatim (`crates/boyko_render/src/light.rs:884-885`) and `GpuLight::from_point` copies it into
  > `pos_range[3]` unchanged (`:1008`, `:1012`), and the repository already ships a **non-finite** `.w`
  > in that same lane — `f32::INFINITY` for directional lights (`:994`, pinned by the unit test at
  > `:1448`). *(That directional row never reaches the compare — the cull `continue`s on kind,
  > `shaders/cluster_cull.hlsl:163-165` — so it is evidence that the lane is unvalidated, not that a NaN
  > range reaches the compare today. `r = inf` is harmless: `r·r = inf` and `0.0 <= inf` is true.)*
  >
  > **What changes: nothing but the proof text.** At `r·r = NaN` the **fine** test rejects for the same
  > reason (same ordered compare, same NaN), so coarse-rejects ⇒ fine-rejects holds **directly** rather
  > than vacuously, and D4's byte-identity holds because the flat arm emits nothing for that light
  > either. This is a proof-text defect, not an algorithm defect: no constant, no branch and no gate
  > moves. The premise it adds is recorded as the widened **Premise F** (§5.2), not as a scope clause.

  > **Rev 5 P1 fix — this exact step was WRONG in Rev 4, and three other claims rested on it.** Rev 4
  > wrote the absorbing element as `(−1e30, +1e30)` and asserted `d_j = 0` "for any finite light centre
  > `c`". **False whenever `|c_j| > 1e30`**, which is a perfectly finite float: then `c_j − 1e30 > 0`,
  > `F = +inf`, and the **coarse level rejects**, while the poisoned lane's own all-NaN fine test gives
  > `F = 0` and **accepts** — enclosure inverted, in precisely the case the substitution exists to
  > handle. The fix is one token in three places (§4's phase-1 HLSL, D8's table, this step):
  > **`±FLT_MAX`**. Verified: `−FLT_MAX <= c <= FLT_MAX` holds for every finite `c`, and `FLT_MAX`
  > still absorbs against the `±1e30` identity, so D8's two-row ordering is unchanged.
  >
  > **The `1e30` FINITENESS THRESHOLD (§4 phase 1) is a different constant serving a different purpose
  > and is deliberately left untouched.** It classifies a lane, and it is also what makes the `!valid`
  > identity `(+1e30, −1e30)` a true identity: every *finite-classed* lane has `|AABB| <= 1e30`, so the
  > identity can never narrow the box, while the absorbing `±FLT_MAX` still wins against it. A later
  > reader must **not** "unify" the two — they are load-bearing in opposite directions.

  > **The one residual premise that survives (Premise F, §5.2): the light centre must be finite.** If
  > some `c_j` is `±inf`, the coarse test rejects (`d_j = inf`) while the poisoned lane's own fine test
  > still yields `F = 0`, and enclosure genuinely fails. This is **not** an AABB-finiteness clause
  > returning by the back door: it is a property of the *light table*, not of the froxel geometry, it
  > is undischarged in the repository today, and it is **pre-existing** — a non-finite centre already
  > makes the BASE arm's flat test accept every froxel. It bounds D4's byte-identity claim, never
  > memory safety. §5.2 names it; §11 carries the closure alongside the other NaN-hygiene items.

Q.E.D. — and the claim is **unconditional in the AABB's finiteness** (Premise F aside, which is a
condition on the *light*, not on the AABB), not scoped by a clause. ∎

**Lemma (all-`!valid` group).** An all-identity group yields the inverted box `(+1e30, −1e30)`;
`sq_dist_point_aabb` then computes `d` about `1e30` per axis and `F(d)` overflows to `+inf` (finite x
finite gives inf, never NaN), and `inf <= r*r` is false, so the group rejects everything.
Well-defined, no NaN, no UB. A fully-invalid group cannot occur by construction: the host dispatches
`gps·bdz` groups with `gps = ceil(bdx·bdy/256)`, so every group's `s` range starts below `bdx·bdy`.

**Corollary (this is what makes D4 clause (c) deletable).** In Case B the coarse mask ends up
containing every punctual index `j < ps_n` **whose `r·r` is not NaN** — and a `r·r = NaN` light is
absent from the flat arm's emission too, for the same ordered compare (Rev 6 P1) — so the fine walk
visits exactly the flat arm's **emitted** index range, in ascending order, applying the token-identical
predicate and clamp. Every froxel in that
group therefore emits **exactly the flat arm's sequence** — including the non-finite froxel itself,
whose own fine test computes the same `sq_dist` from the same NaN AABB in both arms. Byte-identity is
preserved, not excused.

### 5.1 The NaN analysis, rebuilt against the instructions this shader actually emits

**Rev 3's analysis argued about `FMin`/`FMax`. This shader emits none.** Measured (M2): `FMin` 0,
`FMax` 0, `NMin` **8**, `NMax` **18** — 26 `N`-form ops, and `precise` does not change that (M5).
`NMin`/`NMax` are specified to return the **non-NaN operand** when exactly one operand is NaN (the
both-NaN case is left undefined). Consequences, each of which contradicts something Rev 3 wrote:

1. **`max(max(NaN, NaN), 0.0)` yields `0.0`, so the test ACCEPTS.** Whatever the inner both-NaN
   `NMax` returns, the outer `NMax` has a non-NaN operand `0.0`; if the inner result is NaN the outer
   returns `0.0`. Then `F(d) = 0 <= r·r`. **Rev 3's stated alternative — "if the NaN reaches the
   compare then `NaN <= r*r` is false and the group rejects everything" — is unreachable**, and its
   claimed order-dependence ("`min(NaN,x)` vs `min(x,NaN)` differ under the common `b<a ? b : a`
   lowering") is a property of `FMin`/`FMax`, not of the emitted instruction. Both are withdrawn.
2. **A single NaN lane is already dropped from the fold.** `NMin(NaN, finite) = finite`. So an
   *unmitigated* hierarchical arm does not get a NaN coarse box from one poisoned lane; it gets the
   extremum over the *other* lanes. **Rev 3's "the flat arm's blast radius is one froxel; the
   hierarchical arm's is one group (144 froxels)" is therefore not established, and the declaration
   that Rev 2's assessment was FALSE is withdrawn.** What actually goes wrong is narrower and lives
   on the *fine* side: the poisoned lane's own fine test computes `d = 0` and would accept every
   light, but it is filtered by a coarse mask built from a box that does not enclose it — so **that
   one froxel** may emit fewer lights than the flat arm. One froxel, not 144.
3. **Rev 3's mitigation was a no-op where it was aimed and harmful where it was not.** Substituting
   the `min`/`max` **identity** for a non-finite lane changes nothing in the mixed case (the NaN was
   already dropped by `NMin`/`NMax`), and in the reachable all-NaN case it **inverts a conservative
   outcome into a maximally divergent one**: unmitigated, an all-NaN group yields `d = 0` and accepts
   every light — *matching the flat arm*; mitigated, the identity gives the inverted box, `F = +inf`,
   and **all 144 froxels reject every light** while the flat arm keeps every light. It also does not
   repair (2), because (2) is a fine-side failure.
4. **Rev 4's replacement is the ABSORBING element** `(−FLT_MAX, +FLT_MAX)` for `valid && !finite` lanes
   *(Rev 4 wrote `±1e30`; Rev 5 corrects the constant — see Case B)*
   (D8, §4 phase 1). It (a) restores enclosure by making the coarse box the universe, (b) reproduces
   the *conservative* outcome in the all-NaN case rather than inverting it, (c) fixes (2) — the
   poisoned lane's fine walk now sees the full mask — and (d) makes the group's output **exactly** the
   flat arm's (Corollary above), so D4 clause (c) is deleted rather than relied on.

**Two reachable NaN sources**, both cited, and the first is invisible to any host finiteness assert:

1. `crates/boyko_scene/src/camera.rs:325-327` normalizes the three camera basis vectors, and
   `crates/boyko_math/src/vec.rs:226-233` — `Vec3::normalize` returns `Self::ZERO` when
   `len_sq <= f32::MIN_POSITIVE`. A singular/zero-scale camera `GlobalTransform` therefore yields
   `cam_forward = (0,0,0)`, which is **finite**. `camera.rs:331-336` gives `view` an identity fallback
   ("the degenerate camera renders the identity view rather than NaN") but gives the **basis** none.
   The finite zeros are uploaded verbatim (`compute.rs:3005-3015`), and on device
   `ray_gen.hlsli:63-67` computes `dir = cam_fwd + right·(..) + up·(..)` = exactly `(0,0,0)` and then
   `normalize(dir)`, which is **undefined** per GLSL.std.450 (in practice `0 * rsqrt(0)` = NaN).
   `cluster_cull.hlsl:151-152` feeds that `rd` to `expand_aabb`. Every uploaded float is finite, so a
   host assert sees nothing.
2. `cluster_cull.hlsl:77-79`'s `slice_view_z` is `z_near * pow(z_far/z_near, k/dim_z)`, and
   `ClusterCullPush::new` (`compute.rs:3473-3483`) validates neither. The only `z_far > z_near > 0`
   check is a `debug_assert!` in a **different** function (`ClusterConfig::z_scale`,
   `light.rs:738-743`) that is not on the push path. With `z_near == 0.0`: `+inf`, then
   `0.0 * inf = NaN`, i.e. a NaN AABB even under ORTHO, with no ray-gen involved.

**Both sources are group-global**, which is worth stating plainly because it bounds what can be
tested: a degenerate camera or a zero `z_near` poisons *every* lane, so a **partial** (within-group)
NaN is not reachable from either. §8.3's mutation (vii) therefore reaches it by **fault injection**,
which is the only way to exercise the mixed case at all — and it is run **two-sided**, so it
demonstrates that the substitution does something rather than asserting it. **Rev 5 re-specifies the
injection predicate:** it is a **froxel identity** (`fi == 168u`), *not* Rev 4's `lane == 7`, because
`lane` selects **disjoint froxel sets** in a 64-wide and a 256-wide module — and the injection is
mirrored in **all three** implementations (HIER, base, host mirror), because the detector is an
arm-vs-arm comparison. §8.3 carries the full derivation.

**The predicate.** `all(abs(v) <= 1.0e30)` and not `isfinite(v)`: an ordered compare is false for NaN
**and** for ±inf, so the predicate is exactly "finite and inside the sentinel envelope"; the identity
sentinel satisfies it (`abs(1e30) <= 1e30`), which is what makes §4's branch order safe; and it is
**measured cheaper** — 6 SPIR-V instructions (`FAbs`, `OpFOrdLessThanEqual %v3bool`, `OpAll`, x2;
measured M11) against `isfinite`'s 10. Applied **only to the choice of stored value**, never to the
fine test, so §4's token-identity — and hence D4 — is untouched.

*Stated limitation, because it is a real behaviour and not a bug:* a `valid` lane whose AABB is
finite but exceeds `1e30` in magnitude is treated as non-finite and degrades its whole group to the
flat walk. Output stays correct (byte-identical, by the Corollary); only performance degrades, and
only in a scene whose froxel AABBs span more than `1e30` world units.

**Cost is a MODEL number, not a fact** (§1.1's rule applied to this plan): about 6–12 scalar ops per
lane, once per lane per dispatch, i.e. 6144 times at the default grid — `O(froxels)`, independent of
`N`. Against §1.2's 0.2736 ns/pair and a ~25-op pair test that is about 0.4–0.9 microseconds per
invocation, at most about 4 % of the predicted 22.7 microseconds at `N=512` and about 5 % of the
predicted 15.9 microseconds at `N=8`. **H4 gate (e) measures it**; the plan does not assert it.

### 5.2 What the proof needs, and what it does not

* **It does not need** a dilation constant, an epsilon, or an assumption that two *sites* compile
  identically. (They need not — they need only be `F`.)
* **It does not need** a finiteness scope clause any more. Case B carries it.
* **It does need exactly three named premises — one of which is a CONJUNCTION of two conditions on the
  light row (Rev 6 P1). Two are discharged in the artifact; the third is named and explicitly NOT
  discharged:**

  > **Premise P.** *Every arithmetic node in `sq_dist_point_aabb` is a correctly-rounded,
  > `NoContraction`-decorated SPIR-V op, and the two `NMax` nodes return an operand bit-exactly.*
  >
  > **Discharged in the shader source** (D10's body is the artifact, with the per-node audit table),
  > **and tripwired** by H2 gate (e): `OpDot == 8` (zero in the cull comparison), `NoContraction ==
  > 14` **exactly** (which is also the guard against `precise` back-propagating into an argument
  > expression, M6), the two scalar `OpFOrdLessThanEqual` windows id-normalised equal, and a
  > **producer assertion, scoped to the `NoContraction`-DECORATED `OpFSub` feeding the `d` chain**,
  > that neither of their operands is an `OpFMul` result. **The scoping is mandatory, not tidiness**
  > (Rev 5 P2): the module carries roughly two dozen *undecorated* `OpFSub` in ray-gen whose operands
  > **are** `OpFMul` results, so an unscoped form false-REDs a correct module. The gate is defence in
  > depth; the proof is Step 0.

  > **Premise U.** *`coarse_min`/`coarse_max` are group-uniform.*
  >
  > **Discharged by §4 phase 3** (every lane folds the same 16 slots), gated by D8 review item 6 and
  > by H3 mutation (ii), which replaces the fold with lane 0's value and must go RED.

  > **Premise F (NEW in Rev 5; WIDENED in Rev 6).** *Every punctual light's centre `L.pos` is finite
  > **and** its `L.range` is not NaN.*
  >
  > **Rev 6 P1 — why the second conjunct is here.** Rev 5 named only `L.pos`, and §5 Case B's algebra
  > then read "`F(d) = 0 <= r·r` for every `r`", which is false at `r·r = NaN` under the **ordered**
  > compare. The two conjuncts are the same kind of fact (an unvalidated light-table lane —
  > `light.rs:884-885` / `:1008` / `:1012`) and cost the same closure, so they are one premise rather
  > than a fourth. They differ in consequence, and the difference is stated rather than smoothed: a
  > non-finite **centre** breaks Case B's enclosure and **costs D4's byte-identity**; a NaN **range**
  > does **not** — both levels reject it, so the arms still agree (§5 Case B's Rev 6 note). Premise F
  > is therefore load-bearing only in its `L.pos` half; the `L.range` half is named so the proof text
  > is true as written.
  >
  > **NOT discharged.** No host path validates light-position finiteness today, and Rev 5 does not add
  > one — a repo-wide claim to the contrary would be exactly the kind of unbacked assertion this
  > document exists to avoid. What it costs, precisely: with a `±inf` component in `c`, Case B's
  > vacuity argument fails and a non-finite-AABB lane can be accepted by its own fine test while the
  > coarse level rejects, so **D4's byte-identity between the arms is lost for that light**. It costs
  > nothing in memory safety (`ps_n` bounds every read and `fi < capacity` every write, neither of
  > which involves `c`), and it is **pre-existing in kind** — a NaN centre already makes the base arm's
  > flat test accept every froxel. Two consequences that are gates rather than prose: **H3's rigs must
  > not contain a non-finite light centre** (the equality assertions would legitimately go red), and
  > the closure — host-side finiteness validation of `LightElem::pos` — is filed in §11 next to the
  > `safe_normalize` and `ClusterCullPush::new` validation items, which address the *other* two NaN
  > sources.

* **Where Rev 4 under-claims relative to Rev 3.** Rev 3 wrote "after the mitigation §5's proof is
  unconditional". That was an overclaim at the time: the mitigation did not touch the fine side and
  §4's phase-5 gate contradicted it. Rev 4's proof *is* unconditional in finiteness, but by a
  different mechanism (Case B), and the word is earned by the case analysis above plus a two-sided
  test — not asserted.

---

## 6. `index_list_cap` saturation `[P1-3]`

The critic is right that reordering is **not** output-neutral under saturation: slices are claimed
by a single global `InterlockedAdd(LightIndexAlloc[0], nlocal)` (`:183`), and when the claim runs past
`index_list_cap` the tail is dropped (`:184-191`) — *which* froxel loses its tail depends on claim
order, and the hierarchical arm changes claim order.

**Discharge, with numbers (§1.3):** on the VB-P1d rig, peak total claim is **2 709 words = 16.5 %**
of the 16 384-word cap (at `N_ps=1024`), and peak per-froxel count is **109 vs the 256 cap**. Neither
cap is reached anywhere in the swept range, so the drop path is never taken and claim order cannot
affect any surviving index. Byte-identity between the arms **is** achievable on this rig.

**But the plan does not rest on that estimate remaining true.** Three mechanisms:

1. **An exact runtime detector.** After the cull, `LightIndexAlloc[0]` holds the *total claimed*
   (pre-clamp) count, because `InterlockedAdd` bumps even when the write is dropped. So
   `alloc_total <= index_list_cap` if and only if **no index was dropped anywhere**. One `u32`
   settles it exactly, per run, with no modelling.
   **Where that `u32` is read (this changed in Rev 3, and it is what makes §9's `[P0-1]` row true):**
   **not** from the production present path. It is read in **H3's cull-only driver**, which creates
   `LightIndexAlloc` as `MemoryLocation::HostVisibleCoherent` and reads it through
   `buffer_mapped_ptr` **after the fence**. *Citation corrected in Rev 4:* the post-fence mapped-read
   idiom is `crates/boyko_rhi_vulkan/tests/sdf_gbuffer_hybrid.rs:6202-6211` (ClusterGrid) and
   `:6219-6228` (LightIndexList) — each a `buffer_mapped_ptr` + `copy_nonoverlapping` guarded by a
   `// SAFETY:` comment that names the fence. Rev 3 cited `:5415-5425`, which is the **host
   zero-write BEFORE the submit** — a different idiom for a different purpose. *(Rev 5 P2: that span
   is `:5415-5426`; the `write_words(mapped, &[0u32])` call is on `:5426`, so Rev 4's range stopped one
   line short of the write it names.)* No `vkCmdCopyBuffer`, no staging buffer, no framegraph resource, no seed
   decision. §1.3's CPU oracle computes the same `total_indices` exactly, on the CPU, in 0.45 s.
2. **The equality oracle asserts it as a precondition** (§9, H3): if `alloc_total >= index_list_cap`
   the test **fails loudly** rather than silently comparing two differently-clamped results.
3. **The honest caveat, stated in the shader header and the test:** under saturation the arms may
   legitimately differ in *which* froxel loses its tail; byte-identity is claimed only for
   non-saturating configurations (D4 scope clause (a)), and the saturating case is pinned only
   against itself.

The per-froxel cap (`max_lights_per_cluster`, `:170`) is *not* order-sensitive — it truncates the
ascending prefix identically in both arms (D4.3) — so only the global cap needs this treatment.

**A note on the permutation probe (§8.2).** That probe deliberately runs a configuration in which
`alloc_total == capacity` exactly (one light, every froxel claims one slot). It therefore sets
`index_list_cap = 2 * capacity` so assertion 1 above is satisfied with margin, and it is the one H3
configuration where `alloc_total` is a *pinned expected value* rather than a bound.

---

## 7. Predicted win (the pair-count half is falsifiable at H1 before any shader work)

Using §1.2's model and §1.3's occupancy profile, `TPG=256` gives 24 groups:

```
pairs_hier(N) = 24·N                       (coarse, phase 4)
              + sum over froxels of E_coarse(parent)   (fine, phase 5)
```

**Rev 6: the fine and hier columns are now MEASURED, not modelled** (H1, config **M2** — 16x9x24
PERSPECTIVE, the VB-P1d bench camera, capacity 3456, 24 groups — on the bench **Kronecker** rig
`lights_for`, `crates/boyko_rhi_vulkan/tests/lighting_l1_host_oracle.rs:177`). `fine = hier − coarse`;
`hier` is `HierCullStats::pairs_hier()` (`crates/boyko_rhi_vulkan/src/goldens.rs:3680`); `ratio` is the
selectivity the H1 test prints. Rev 4's modelled figures are kept in the last column so the size and
the SIGN of the model error stay visible:

| `N` | flat pairs | coarse | **fine (measured)** | **hier (measured)** | ratio | model cull hier | measured cull flat | Rev 4 modelled hier |
|---|---|---|---|---|---|---|---|---|
| 8    | 27 648    | 192    | **3 456**  | **3 648**  | 7.58x  | approx 14.9 us | 19.7 us  | approx 7 100 (**1.95x HIGH**) |
| 64   | 221 184   | 1 536  | **not measured** | **not measured** | — | — | 72.7 us | approx 6 500 |
| 128  | 442 368   | 3 072  | **18 719** | **21 791** | 20.30x | approx 19.9 us | 134.9 us | approx 8 300 (**2.63x LOW**) |
| 512  | 1 769 472 | 12 288 | **33 552** | **45 840** | 38.60x | approx 26.5 us | 498.1 us | approx 32 300 (**1.42x LOW**) |
| 1022 | 3 532 032 | 24 528 | **approx 54 000** | **approx 78 528** | 44.98x | approx 35.4 us | — | — |

*Provenance and its one limit.* The three exact rows reproduce the selectivity the shipped H1 test
prints to six decimals (`3648/27648 = 0.131944`, `21791/442368 = 0.049262`, `45840/1769472 = 0.025906`
— the printed table's M2 rows). The `N=1022` row is **not exact**: the test prints the ratio, not the
count, and `0.022233` pins `hier` only to `78 526–78 529`, so it is written `approx`. *(Rev 6 records,
and does not silently apply, that the count handed to this revision for that row — `78 530` — does not
reconcile with the printed selectivity; it would print `0.022234`.)* **One-line follow-up, in the same
form as Rev 5's P2-7:** H1's selectivity `eprintln!` should also print `stats.pairs_coarse` /
`stats.pairs_fine`, so this table is reproducible from the artifact rather than from a derivation. Rev
6 does not edit that test.

`froxel_total_hier(N) = 26 500 + 13 939 + 0.2736·pairs_hier(N)` against
`flat_shade(N) = 23 922 + 1 109.6·N` gave break-even `N ~ 17` (and `N = 19` at the 2x-pessimistic
`0.5472 ns/pair`) **by interpolating this table's fine column between `N = 8` and `N = 64`**.
**Rev 6: that interpolation is now UNSUPPORTED and no number replaces it.** H1's matrix is
`[0, 8, 128, 512, 1022]` (`lighting_l1_host_oracle.rs:346`) — it **dropped `N = 64` and has no sample
anywhere in 16–20**, which is exactly the interval the estimate lives in and exactly where the model
error is largest and **changes sign** (model `1.95x` HIGH at `N=8`, `2.63x` LOW at `N=128`). What can
honestly be said without a new measurement: the measured `N=8` point is **lower** than Rev 4 modelled
it (3 648 vs approx 7 100). **Rev 6 deliberately does not turn that into a direction for the crossing**
— the crossing sits at `N ~ 17`, between the only two measured points, and the model error reverses
sign across exactly that interval, so a one-sided extrapolation from `N=8` would be the same move this
document keeps catching. **Recovering a break-even number requires an `N = 64` row and at least one
sample in 16–20 in `HIER_MATRIX_N`** — a test-data edit, not a design change. Until then §2's `<= 40`
**measured** break-even gate stands unchanged and H4 remains its only arbiter; no gate is loosened on
the strength of an interpolation that no longer has endpoints.

The `N=512` conclusion is robust to a 2x model error on the *measured* count: at `0.5472 ns/pair` the
cull is `13 939 + 0.5472 x 45 840 = about 39.0 us` against a measured flat `498.1 us`.

> **Rev 5 arithmetic correction, and which way it cuts (P2).** Rev 4 printed `about 51 us` / `10x`
> here and `25–30` for the pessimistic break-even. Neither recomputes from its own inputs, and both
> were **conservative** — they made the design look *worse* than its own model says. Correcting them
> moves the prediction in the design's favour, which is a reason for **more** caution, not less:
> **no gate consumes these numbers.** §2's ship gate stays `<= 250 000 ns` and `<= 40` measured
> break-even, §10's ABORT clauses are unchanged, and H4 remains the only arbiter.

> **Rev 6 — the measurement arrived, and it cuts the OTHER way at `N >= 128`. Every gate still
> clears, and none is loosened (P1).** Rev 4/Rev 5's model was **2.63x low** at `N=128` and **1.42x
> low** at `N=512` — the design is *less* selective than its own model claimed, not more. Against §2's
> `<= 250 000 ns` ship gate, on §1.2's rate:
> * **Kronecker (bench) rig:** `13 939 + 0.2736 x 45 840 = 26.5 us` — 9.4x inside the gate.
> * **Dense in-frustum rig:** selectivity `7.45x` at `N=512` (H1, M2) ⇒ `pairs_hier ~ 237 500` ⇒
>   `13 939 + 0.2736 x 237 500 = 78.9 us` — still 3.2x inside the gate.
>
> **The honest predicted win is therefore `498.1 / 78.9 = about 6.3x`, not 15.8x.** The flat arm's
> cost is rig-independent (it tests `capacity x N` pairs wherever the lights sit), so the measured
> `498.1 us` transfers across rigs; only the hierarchical arm's cost is rig-sensitive. §2's thresholds,
> §10's ABORT clauses and H4's arbiter role are unchanged — a smaller predicted win is a reason to
> keep every gate exactly where it is.
>
> **Where the modelled `32 300` deliberately still stands, and why (so no later reader "finishes the
> job").** Two places keep it: **§7.0's Reading A** — that number is a **pre-registration**, and §7.0
> exists precisely so H4 cannot be retro-fitted, so rewriting it after the measurement would destroy
> the artifact's only function; and **§D3's `TPG` comparison table**, where the same modelled figure
> appears on **both** compared rows, so the comparison it serves is unaffected by the model error. The
> two places that **execute** the number — §8.7's re-derivation recipe and §10 ABORT clause 2 — are
> corrected to the measured **45 840**. Pre-registrations are frozen; operative literals are measured.

**Fine-column derivation — SUPERSEDED by measurement (Rev 6), kept because the size of the miss is the
point.** The paragraph below modelled `N=512` fine at `17 280 + 3 000 = about 20 000`; H1 measured
**33 552**, i.e. the concentration argument under-counted by 1.7x. It is retained, unedited, as the
record of what the model said before it was falsifiable — not as a live derivation. *(From §1.4's
collinearity result, the in-frustum lights at `N=512` are the roughly 14 % of the rig lying at
view-depth 8.7–14.4, i.e. z-slices 17–19 of 24. Those three groups therefore carry about 40 candidates
each over their 144 froxels (3 x 144 x 40 = 17 280); the remaining 21 groups carry about 0–2 (about
3 000). H1 computes this exactly, per config, on the CPU.)*

### 7.0 What this table is a bound on — pre-registered, so H4 cannot be retro-fitted

**The microsecond column above is an AGGREGATE-THROUGHPUT bound.** It multiplies total pair count by
§1.2's marginal rate, which was calibrated on a *balanced* 54-group dispatch. The hierarchical
dispatch is deliberately imbalanced: at `N=512`, **3 of 24 groups carry 17 300 of about 20 300 fine
pairs = 85.2 %**, while 21 groups idle. If the pass is latency-bound rather than throughput-bound,
wall clock is set by **one group's serialized latency**, and the aggregate bound is optimistic by
roughly the imbalance factor.

The plan therefore commits, **in advance**, to two readings and one discriminating experiment:

* **Reading A (aggregate-throughput).** `cull_ns(512) = 13 939 + 0.2736 x 32 300 = about 22.7 us`.
  This is the number §2's ship gate is written against (`<= 250 000`, a 10x margin over it).
* **Reading B (hot-group latency).** Wall clock tracks the *hottest group's* pair count
  (about 5 800 fine + 512 coarse = about 6 300) plus the fixed cost, but serialized against a machine
  that is mostly idle. Reading B predicts a *higher* number than A whenever the hot group cannot hide
  its own latency, and — critically — it predicts that `cull_ns` is **insensitive to `dim_z`** at
  fixed `N`, whereas A predicts it scales with total groups.
* **The discriminating measurement is H4's `dim_z` sweep at fixed `N`.** If `cull_ns` tracks the
  hottest group's pair count rather than the total, Reading B is confirmed and §7's microsecond column
  must be re-derived from it before any ship decision. **Neither H1 nor H1.5 can see this** — H1
  counts pairs, H1.5 varies thread count on a *balanced* dispatch.

**And the necessary/sufficient split, stated plainly.** H1's 55x selectivity is a *pair-count* result.
**55x selectivity is fully compatible with a sub-2x wall-clock win**, because the hierarchical arm
also introduces: 6144 threads instead of 3456; **43.75 %** of lanes idle in the fine phase; 3 barriers
per group; 24 groups on the **owner-stated** 28 SMs, leaving 4 SMs empty (D3 — the SM count is not
readable from this stack and no gate consumes it); a 192-B-strided `ClusterGrid` write pattern
(D3 trade-off 3); and the 85.2 % hot-group concentration above. None of those is visible to a
pair-count oracle.

### 7.1 The negative result — single-digit break-even is impossible here, and why

`froxel_shade` alone is 25–30 us and the cull's fixed cost is about 13.9 us, so **the froxel arm's
floor is about 40 us**, while `flat_shade`'s intercept is **23.9 us**. Break-even requires
`flat_shade(N) > floor`, **and that is with a cull of cost zero**:

```
N > (26 500 + 13 939 - 23 922) / 1 109.6 = 16 517 / 1 109.6 = 14.89
```

Rev 2 printed **14.4**, which came from rounding the numerator 16 517 to 16 000. **The conclusion is
unaffected and is proved rather than estimated**, because the floor is published as a **band** over
`froxel_shade`'s own stated error bar and over the choice of fit:

| variant | floor |
|---|---|
| `froxel_shade = 24 300` (−1 sigma) | `N > 12.90` |
| `froxel_shade = 26 500` (nominal) | **`N > 14.89`** |
| `froxel_shade = 28 700` (+1 sigma) | `N > 16.87` |
| consistent 128/512 re-anchoring of §1.2 (`13 871 + 945.70·N`, `25 758 + 1 106.0·N`) | `N > 13.21` |

**Every variant exceeds 12.9**, so the negative result does not depend on which fit is chosen or
where in the error bar `froxel_shade` lands. No amount of cull optimisation can push the break-even
below about 13 on this hardware and this grid. Reaching single digits would require attacking the
*fixed* costs (merge the cull into the shade dispatch to delete a barrier; eliminate the
`cmd_fill_buffer` reset via a per-FIF alloc ring; or shrink the froxel grid at low `N`). Those are
named as VB-P1g in §11 and are explicitly **out of scope** here. The stated goal "break-even collapses
toward single digits" is therefore **partially unreachable**, and the plan does not pretend otherwise:
it targets **about 17–19**, a **5.4–6.1x** improvement on the measured about 103. *(Rev 4 printed
`17–30` / `3–6x`; the upper end was the pessimistic break-even §7 has now recomputed to 19.)*

---

## 8. Rungs

Each rung is independently committable, has one gate, and states what turns that gate RED.

**Rev 4's rule for this whole section, applied to every assertion below:** *an assertion is only
listed if a concrete mutation was constructed AND that mutation was simulated (or executed) to
confirm it turns the assertion red.* Where no such mutation exists, the assertion is **deleted and
the deletion is recorded**, because an assertion that cannot fail manufactures confidence without
supplying any. §8.2 designs the three new detectors; §8.3 is the mutation table with its simulation
results; §8.4 onwards are the rungs.

### 8.1 Rung ladder

| Rung | What it establishes | Needs a GPU? |
|---|---|---|
| **HP** | §1.3's occupancy table becomes a committed pin | no |
| **H0** | Where the 13.9 us fixed cost actually goes | yes (bench) |
| **H1** | Pair-count selectivity + the map's exactly-once property, on the CPU | no |
| **H1.5** | Does `0.2736 ns/pair` transfer across dispatch shapes? | yes (bench, existing arm) |
| **H1.6** | The D10 `precise` edit in isolation + the one-time base re-pin | yes (goldens + bench) |
| **H2** | The `-D HIER=1` variant exists, is byte-gated, and conforms to §4/§5 structurally | no |
| **H3** | Arm-vs-arm equality and every memory-safety property, on device | yes |
| **H4** | Wall clock, both rigs, plus §7.0's discrimination | yes |
| **H5** | Deferred + ForwardPlus migration | yes |

### 8.2 The three detectors (design, with the arithmetic that makes each one fire)

Rev 3 had one device-side structural detector — a `0xFFFFFFFF` pre-fill of `ClusterGrid` — and read
more out of it than it can carry. Rev 4 keeps it, restates what it proves, and adds two more.

#### (A) `ClusterGrid` guard tail — catches out-of-range WRITES with no validation layer

**Why a new detector is needed at all.** Rev 3 cited three detectors for the out-of-bounds write and
all three are blind:

* the `0xFFFFFFFF` sentinel proves **at-least-once**, never **exactly-once**, and says nothing about
  cells *past* the buffer;
* assertions 2/3 (per-froxel count and sequence equality) compare only **in-range** cells;
* "with validation ON the buffer-overrun must be reported" is **unattainable on this stack** —
  `crates/boyko_rhi_vulkan/src/device.rs:2087` enables exactly one validation feature,
  `VK_VALIDATION_FEATURE_ENABLE_SYNCHRONIZATION_VALIDATION_EXT`; a repo-wide grep for `GPU_ASSISTED`
  and `debug_printf` across `crates/` returns **zero** hits; and `robustBufferAccess` is never enabled
  (`ffi.rs:2718` is a field declaration only). Sync-validation does not range-check shader accesses.
  **Rev 4 removes every citation of plain validation as an overrun detector, in §8, §9 and §10.**

**The design.** H3's cull-only driver allocates `ClusterGrid` with

```
G        = HIER_TPG * groups - capacity            // == (256*gps - dim_x*dim_y) * dim_z
alloc    = capacity + G                            // cells, 8 B each
```

pre-fills **all** `capacity + G` cells with `0xFFFFFFFF` **immediately before EACH arm's dispatch**
(honest limit 4 below — the H3 matrix is arm-vs-arm, so there are **two** dispatches per config), runs
that arm's dispatch, reads the buffer back **before** the next fill, and asserts, on that arm's own
readback:

* **(A1) totality** — no cell in `[0, capacity)` still holds the sentinel;
* **(A2) tail integrity** — **every** cell in `[capacity, capacity + G)` still holds the sentinel.

**Why `G` is exactly that expression, and why that makes A2 tight rather than lucky.** Under §4's map
`fi = s·bdz + slice` with `s < 256·gps` and `slice < bdz`. The `valid` lanes are exactly those with
`s < bdx·bdy`, and their image is `[0, capacity)`. The **invalid** lanes are exactly those with
`bdx·bdy <= s < 256·gps`, and `(s, slice) -> s·bdz + slice` is injective, so their image is exactly

```
[ bdx*bdy*bdz , 256*gps*bdz )  ==  [ capacity , capacity + G )
```

— a **bijection onto the guard tail**. Therefore dropping the `valid` guard on phase 6 writes
precisely `G` cells, all of them in the tail, and A2 goes red on all `G` of them. Simulated at
16x9x24: `G = (256 − 144)·24 = 2 688`, and the mutation produces **2 688 OOB writes with max
`fi` = 6 143**, every one inside the tail (M10). 2 688 cells x 8 B = **21 504 B**, which is the
"21 KB past the buffer" figure quoted in the status block.

**Four honest limits, stated because a detector whose limits are unstated is the failure mode this
revision exists to fix:**

1. **`G = 0` when `dim_x·dim_y` is a multiple of 256.** At E1 (16x16x24) and E2 (32x16x24) and E4
   (32x24x24) every lane is valid, so `G = 0` and A2 is vacuous — *and the mutation it detects is
   simultaneously a no-op there* (simulated: 0 OOB). **Protocol: mutation (i) must be run on a config
   with `G > 0`** — M1/M2 (16x9x24, `G = 2 688`) or E3 (16x17x24, `G = 5 760`). This is written into
   H3's mutation protocol, not left to be discovered.
2. **The tail is a detector, not a containment device.** It contains the image of §4's map under the
   pre-registered single-fault mutations (simulated: (i) max `fi` 6 143 < 6 144; (v) max `fi` 3 454 <
   5 888). A mutation that *also* changes `slice`'s range — e.g. transposing the map **and** deleting
   `slice < bdz` — can produce `fi` beyond `capacity + G`, which is genuine UB in the test process.
   **Protocol: the map mutations (vi) and the bound deletions must not be combined in one run.** For
   the map mutations the detector is A1 (coverage), which needs no containment at all.
3. **A2 cannot see a duplicate in-range write.** That is detector (B)'s job.
4. **The pre-fill is bound to the DISPATCH, not to the allocation (Rev 5 P1 fix).** Rev 4 wrote
   "allocates … pre-fills all `capacity + G` cells … runs **the** dispatch, and asserts", and §8.10
   repeated it once — but the matrix is **arm-vs-arm** ("both arms on-device"), i.e. **two dispatches
   per config against one buffer**. Sharing an un-reinitialised buffer breaks the detector in both
   directions:
   * **false GREEN.** Under mutation (vi) at E2 the hier arm writes only 6144 of 12288 cells; the
     other 6144 still hold the **base arm's correct result** from the previous dispatch, so
     assertions 2, 3, 4 **and** 5 all pass and the mutation the config exists to catch is invisible.
   * **false RED.** `alloc_total == capacity` (assertion 7) is false for whichever arm runs second,
     because `LightIndexAlloc` accumulated both arms' claims.

   **Protocol, therefore:** the driver **re-fills `ClusterGrid` with the sentinel and re-zeroes
   `LightIndexAlloc` immediately before each arm's dispatch**, and reads back **all THREE** buffers —
   `ClusterGrid`, `LightIndexAlloc` **and `LightIndexList`** — after that arm's dispatch and **before
   the next arm's dispatch**. Per-arm assertions (1, 5, 6, 7, 8) are evaluated on that arm's own
   readback; comparison assertions (2, 3, 4) compare the **two separately captured** readbacks.

   > **Rev 6 P1 fix — the enumeration omitted `LightIndexList`, and Rev 5's general rule forbade the
   > assertions that read it.** Rev 5 named two buffers here and wrote the rule as "**no assertion may
   > read a buffer that both arms wrote**". Both arms write `LightIndexList` under every protocol, so
   > that rule makes assertion 3 (per-froxel index **sequence** equality — `[P0-3]`'s whole content) and
   > assertion 8 (detector (C), the sole discharge of `[P0-4b']`) *unimplementable as written*, and an
   > implementer who follows the two-buffer enumeration instead reads `LightIndexList` **once, at the
   > end**. That is a live wrong-answer machine in both directions:
   > * **false RED on a correct pair.** The two arms hand each froxel **different** slice offsets: the
   >   offset comes from one global `InterlockedAdd` (`shaders/cluster_cull.hlsl:183`) and the 64-wide
   >   flat dispatch and the 256-wide hierarchical dispatch claim in different orders. Arm A's
   >   `ClusterGrid` offsets indexed into an arm-B-overwritten `LightIndexList` read **another froxel's
   >   slice**, and assertion 3 fails on a byte-identical pair.
   > * **false GREEN on detector (C).** Assertion 8 scans the *surviving* list. Scanned once at the end,
   >   it inspects only the second arm's writes over the second arm's allocation — the mutated arm's
   >   out-of-range indices can be gone.
   >
   > **The rule is restated in the form that is actually executable:** *every assertion is evaluated on
   > readbacks captured after **exactly one** arm's dispatch, and **no assertion may mix one arm's
   > `ClusterGrid` offsets with the other arm's `LightIndexList`.*** Comparison assertions compare two
   > such single-arm captures; they never compare a buffer to itself across arms.

#### (B) The permutation probe — exactly-once, on device, with **no shader change**

Rev 3 asserted that the sentinel proves "every froxel was written exactly once by the block
decomposition". It does not. Rev 4 obtains exactly-once two ways, neither of which requires an
instrumented shader (an instrumented shader would gate a module that is not the one that ships — the
same class of error as the rest of this section).

**(B1) Derivation.** Let `V` be the number of `valid` lanes. H1 assertion 7 pins `V == capacity` on
the CPU over the whole grid matrix. Phase 6 performs exactly one write per `valid` lane, so there are
exactly `V == capacity` writes. If any two of them targeted the same in-range cell then strictly
fewer than `capacity` distinct in-range cells were written, so either some in-range cell keeps its
sentinel (**A1 red**) or some write went out of range (**A2 red**). Hence **A1 + A2 + `V == capacity`
implies exactly-once.** Every step of that is a gate that can fail.

**(B2) Direct measurement.** A dedicated H3 configuration — the **permutation probe** — run at
**M1/M2 (16x9x24) AND at E3 (16x17x24)** (Rev 5 P2). E3 is the only `gps >= 2` config with `G > 0`, so
it is the only place where exactly-once is measured on device with the map **non-degenerate**: at
`gps = 1` the `(gid, lane) -> fi` map collapses and a probe there cannot distinguish a correct map from
`slice = gid; s = lane` (§8.3 mutation (vi) is bit-identical at M1/M2 by construction). The probe:

* one point light at the camera eye with `range = 1e6`, so **every** froxel accepts exactly one light;
* `index_list_cap = 2 * capacity`, so §6 assertion 1 holds with margin;
* assert `alloc_total == capacity` **exactly**;
* assert every `ClusterGrid[fi].count == 1` for `fi` in `[0, capacity)`;
* assert the multiset `{ ClusterGrid[fi].offset : fi in [0, capacity) }` is **exactly**
  `{0, 1, …, capacity−1}`.

**Why it fires on a duplicate.** `InterlockedAdd` hands out *distinct* offsets, one per claiming lane.
If two lanes write the same cell, one of their offsets is overwritten and never appears anywhere in
the grid, so the multiset has a gap and a repeat. This is a strictly stronger check than A1 (it fails
even in the hypothetical case where a third lane happens to fill the cell A1 would have caught), and
it costs one extra H3 configuration and zero shader bytes.

#### (C) Light-table poison tail — turns an out-of-range light READ into a detectable index

Designed in D7 and repeated here as the third detector. H3's driver always allocates `MAX_LIGHTS +
1024` light rows and fills every row at index `>= light_count` with `{kind: POINT, pos: camera eye,
range: 1e6}` — a light every froxel would accept. The driver asserts **no emitted `LightIndexList`
value is `>= light_count`**. A producer mutation that walks past `ps_n` then reads real, allocated,
*accepting* rows and emits their indices, which the assertion sees. **No UB is involved**: the rows
exist; only the header says they do not. This replaces Rev 3's mutation (iv), which could not reach
an out-of-range read at all (phase 4 sets bits only for `j < ps_n`, so forcing `ps_n = 0` in the fine
arm leaves the walk visiting the same in-range bits).

### 8.3 The mutation table — every row simulated against §4's own thread map

Simulation replicates §4/§D3 exactly: `gps = max(1, ceil(bdx·bdy/256))`, `groups = gps·bdz`
(host-derived from BOOT), `slice = gid/gps`, `s = (gid%gps)·256 + lane`, `fi = (y·bdx+x)·bdz + z`,
buffer always BOOT-sized. Columns: in-range cells written, unwritten (gaps), duplicates, OOB writes,
how many of those land in the guard tail, and `max fi`.

**Default grid 16x9x24 — `capacity` 3456, `groups` 24, `G` 2688**

| mutation | in-range cells | unwritten | dup | OOB | in tail | max `fi` | verdict |
|---|---|---|---|---|---|---|---|
| *baseline (correct)* | 3456 | 0 | 0 | 0 | 0 | 3455 | **GREEN** (correct) |
| **(i)** drop `valid` on phase 6 | 3456 | 0 | 0 | **2688** | **2688** | 6143 | **RED — A2** |
| **(vi) Rev 3's transposed form** | 144 | **3312** | 0 | 0 | 0 | 3432 | **RED — A1** *(Rev 3 pre-registered this as GREEN)* |
| **(vi) Rev 4's form** `slice=gid; s=lane` | 3456 | 0 | 0 | 0 | 0 | 3455 | **GREEN** (bit-identical — correct) |
| drop `slice < bdz` alone | 3456 | 0 | 0 | 0 | 0 | 3455 | **inert — no gate can see it** |
| drop `fi < capacity` alone | 3456 | 0 | 0 | 0 | 0 | 3455 | **inert — no gate can see it** |

**Boot 16x9x23 to live 16x9x24 — `capacity` 3312, `groups` 23, `G` 2576**

| mutation | in-range | unwritten | dup | OOB | in tail | max `fi` | verdict |
|---|---|---|---|---|---|---|---|
| **(v) as Rev 3 wrote it** (delete `fi<capacity`, dims still BOOT) | 3312 | 0 | 0 | 0 | 0 | 3311 | **inert** |
| **(v) Rev 4** (delete `fi<capacity` **and** re-source dims from LIVE) | 3174 | **138** | 0 | **138** | **138** | 3454 | **RED — A1 *and* A2** |

**E2 32x16x24 (`gps`=2 exact, `capacity` 12288, `G`=0) · E3 16x17x24 (`gps`=2 ragged, `capacity`
6528, `G`=5760) · E4 32x24x24 (`gps`=3, `capacity` 18432, `G`=0)**

| mutation | config | in-range | unwritten | OOB | max `fi` | verdict |
|---|---|---|---|---|---|---|
| *baseline* | E2 / E3 / E4 | 12288 / 6528 / 18432 | 0 | 0 | 12287 / 6527 / 18431 | GREEN |
| **(vi) Rev 3's transposed** | E2 | 1024 | **11264** | **0** | 12265 | **RED by COVERAGE** *(Rev 3 said "drives `fi` far out of range" — it does not)* |
| **(vi) Rev 4's** `slice=gid; s=lane` | E2 | 6144 | **6144** | 0 | 6143 | **RED — A1** |
| **(vi) Rev 4's** `slice=gid; s=lane` | E3 | 6144 | **384** | 0 | 6143 | **RED — A1** |
| **(vi) Rev 4's** `slice=gid; s=lane` | E4 | 6144 | **12288** | 0 | 6143 | **RED — A1** |

**What this table changes, item by item:**

* **(i) is the P0.** It writes 21 504 B past the buffer and every Rev 3 assertion stays green.
  Detector A2 turns it red on all 2 688 tail cells, with no validation layer.
* **(v) is re-specified.** Rev 3's form is arithmetically inert — which the plan itself proved two
  sections earlier ("the first two already imply `fi < bdx·bdy·bdz` algebraically"; "a live
  `ClusterConfig` edit cannot move `fi` at all") while §9 still claimed it could fail. Rev 4's form
  re-sources the dims from `load_cluster_params(LightBuf)`, which is precisely the state D11 exists to
  prevent, and reproduces D11's own measured-bounds row (`16x9x23 to 16x9x24`, max `fi` 3454 into a
  3312-cell buffer).
* **(vi) is replaced, and P1-F's discharge is re-based.** Rev 3 pre-registered its transposed form as
  "must (correctly) stay GREEN on every 16x9x24 entry — which is the demonstration that Rev 2's
  matrix was blind", and described its E2 failure as driving `fi` far out of range. **Both halves are
  false.** The genuinely `gps`-degenerate form is `slice = gid; s = lane`: at `gps = 1` it is
  *bit-identical* to the correct map (so it correctly stays green on the whole 16x9x24 matrix, which
  *is* the blindness demonstration), and at `gps >= 2` it fails by coverage. P1-F's claim — that
  Rev 2's matrix could not distinguish a wrong map — now rests on a mutation that actually has that
  property.
* **(iv) is replaced** by the producer mutation + detector (C) (D7, §8.2).
* **Two terms of `valid` have no single-fault mutation.** Simulated, and stated in D3 and §9 rather
  than papered over. They are defence in depth against a *second* fault, and (v) is that pairing.

**Mutations whose detector is not the write set** (listed here so the table is complete):

| mutation | what it breaks | detector | why it fires |
|---|---|---|---|
| **(ii)** replace the radix-16 fold with lane 0's value | Premise U + enclosure | H3 assertions 2/3/4 on the **adversarial rig** | the rig places a light exactly tangent to a **chosen froxel that is NOT lane 0's**, so the shrunken box rejects a light the fine test accepts. *The "not lane 0's" requirement is part of the rig spec, not an afterthought — with a lane-0 target the mutation is invisible.* |
| **(iii)** walk mask words descending | D4.2 order | H3 assertion 3 (sequence equality) | needs a froxel holding at least two accepted lights in **two different mask words**; the rig must run `N >= 64` with `l0a_count = 0` so bits span words 0 and 1. Stated as a rig requirement. |
| **(iv)** phase 4 loops `j < HIER_MASK_BITS` + fine clamp deleted | D7's read bound | detector (C) | poison rows are accepted and emitted; assertion "no index >= `light_count`" fires |
| **(vii)** *two-sided*: poison **all six** AABB components of the froxel `fi == 168u`, **mirrored in all three implementations** (HIER module, base module, host mirror) | §5 Case B | H3 assertions 2/3 | **RE-SPECIFIED IN REV 5** — Rev 4's form was broken in both arms. Full derivation below the table. |
| **(viii)** give `!valid` lanes the absorbing element instead of the identity | perf only | **H1 selectivity** | output-neutral, so no equality gate can see it; at 16x9x24 every group has 112 invalid lanes, so every group degrades to the flat walk and `pairs_hier/pairs_flat` goes from about 1/55 to **1.0**, missing H1's `<= 1/8` gate by 8x |
| **(ix)** an early `return` in the HIER arm | D8 | **H2(e)** top-level chain | executed (M9): RED, while `OpReturn` stays at 1 |
| **(x)** move a barrier under `if (lane < 16)` | D8 | **H2(e)** top-level chain | executed (M9): RED |
| **(xi)** drop `precise` / restore `dot()` | Premise P | **H2(e)** counts | `NoContraction` != 14 or `OpDot` != 8 |
| **(xii)** pass an argument *expression* to `sq_dist_point_aabb` | Premise P's blast radius | **H2(e)** exact count | measured (M6): `precise` back-propagates, giving 16 not 14 |

**Mutation (vii) in full — Rev 5 re-specifies it, because Rev 4's form could not go GREEN in the
GREEN arm, could not go RED for the stated reason, and did not name the same froxel in both modules.**

*Three independent falsifications of Rev 4's `inject aabb_min.x = 0.0/0.0 on lane == 7`, all verified:*

1. **The GREEN arm could not be green.** Assertions 2/3 are **HIER-vs-BASE** comparisons. A one-arm
   injection makes the poisoned froxel disagree with the un-injected base arm *regardless* of the
   mitigation. §5's own Corollary requires the froxel to compute "the same `sq_dist` from the same NaN
   AABB **in both arms**" — with a one-arm injection that premise fails by construction.
2. **`lane == 7` is not the same froxel in both modules.** The base arm is `[numthreads(64,1,1)]` and
   HIER is 256-wide, so the predicate selects **disjoint froxel sets**.
3. **The RED mechanism as stated does not follow.** `coarse_min.x` is the min over 144 froxels and
   froxel `x = 0` supplies it, so dropping froxel 7's NaN via `NMin` does not shrink the box; and with
   only `aabb_min.x` poisoned, `d.x = NMax(NMax(NaN, c.x − hi.x), 0)` still rejects on y, z and `+x`.
   Rev 4's RED arm would have needed an unstated off-screen-left rig.

*Rev 5's form:*

* **Predicate: froxel identity `fi == 168u`, not a lane index.** At the default 16x9x24 grid
  `fi = 168` is `(x, y, z) = (7, 0, 0)` — which is **HIER group 0, lane 7** (`gps = 1`, `slice = 0`,
  `s = lane`) **and** base group 2, lane 40. One predicate, the **same froxel**, in a 64-wide module, a
  256-wide module and a host mirror indexed by `fi`. That is the property `lane == 7` did not have.
* **Poison all six components:** `aabb_min = aabb_max = asfloat(0x7FC00000u).xxx` (a quiet NaN by bit
  pattern — **not** `0.0/0.0`, which is a constant expression a compiler may fold or reject).
* **Mirror the injection in the HIER module, the base module AND `golden_cluster_cull`.** All three are
  scratch/parameterised: the two modules are re-DXC'd into the temp dir (never over a committed
  artifact, exactly as H2(e) already does), and the host mirror takes an
  `inject_nan_froxel: Option<u32>` parameter defaulting to `None`, so no shipped call site changes
  behaviour.

*Why it goes GREEN when mitigated, and RED when not — this reasoning is the point of the mutation:*

* **Mitigated** (the finiteness substitution present). Froxel 168 is `valid && !finite`, so it stores
  the **absorbing** `(−FLT_MAX, +FLT_MAX)`; the group's coarse box becomes the universe; the coarse
  mask becomes the **full** punctual set; every froxel of group 0 — froxel 168 included — walks the
  flat range in ascending order under the token-identical fine test. The base arm walks that same range
  by construction. Froxel 168's own fine test computes the same all-NaN `sq_dist` in all three:
  `d_j = NMax(NMax(NaN, NaN), 0) = 0` ⇒ `F = 0 <= r·r` ⇒ it accepts every punctual light it visits, in
  both modules **and** in the host mirror (Rust's `f32::max` returns the non-NaN operand exactly as
  `NMax` does — that agreement is load-bearing and is a review item for H1's implementer). ⇒ **GREEN**,
  on all of assertions 2, 3 and 4.

  > **Rev 6 P2-1 — `NMax(NMax(NaN, NaN), 0) = 0` is asserted here as a fact, but §5.1 says the
  > both-NaN case is UNDEFINED. Close it by measuring, not by arguing.** §5.1's reading is the safe
  > one: whatever the inner `NMax` returns, the outer has the non-NaN operand `0.0`, so the *outer*
  > result is `0.0` — but "the outer `NMax` returns the non-NaN operand" is itself the spec clause
  > whose both-NaN branch is undefined *if the inner result is NaN*, and this plan's own standard is
  > measured-not-argued. **Gate:** H3's driver runs **one extra single-dispatch config** — one froxel,
  > one light, `aabb_min = aabb_max = asfloat(0x7FC00000u).xxx`, no hierarchy — and asserts the emitted
  > `sq_dist` is exactly `0.0` on this device. It costs one dispatch and it is the only thing H3
  > assertion 4's (vii) row rests on. **Failure direction is safe** (if the device returned NaN the
  > compare rejects, (vii)'s GREEN arm goes red, and the design is not wrong — the *pre-registration*
  > is), which is why this is a P2 and not a blocker.
* **Unmitigated** (the finiteness predicate deleted). Froxel 168's NaN is **dropped from the fold** by
  `NMin`/`NMax`, so the coarse box is the extremum over the other 143 froxels and the mask is the
  **coarse-accepted subset**. Froxel 168's all-NaN fine test still accepts everything it visits — but it
  now visits only that subset, while the base arm (same NaN AABB, no coarse level) emits the **whole**
  punctual range. HIER's sequence for froxel 168 is therefore a **strict subset** of the base arm's,
  and assertion 2 (per-froxel count) fails before assertion 3 is even reached. ⇒ **RED**, whenever the
  coarse level rejects at least one punctual light for group 0.
* **Rig-independence — this is why the form is better, and it is stated rather than assumed.** The
  RED condition is a property of the *coarse level's selectivity*, not of where any light sits: it
  needs only "group 0's coarse box rejects >= 1 punctual light" **plus `ps_n < max_lights_per_cluster`
  (Rev 6 P1 — the second condition is not optional; see below)**, and the first is what H1's own
  selectivity gate (`pairs_hier / pairs_flat <= 1/8`) guarantees globally. Rev 4's form, by contrast,
  needed an off-screen-left light arrangement it never specified.
* **Rig requirement (in the same form mutations (ii) and (iii) already carry), Rev 6 — all three parts
  are now stated, because each of Rev 5's readings failed.** The (vii) run must satisfy:
  1. **`ps_n < max_lights_per_cluster`. Pin the (vii) run at `N = 128`** (`max_lights_per_cluster` is
     `MAX_LIGHTS_PER_CLUSTER = 256`, `crates/boyko_render/src/light.rs:53`). *Why this is a
     requirement and not a preference:* both arms carry the same per-froxel clamp
     `if (nlocal < pc.max_lights_per_cluster && nlocal < 256u)`
     (`shaders/cluster_cull.hlsl:170`). Froxel 168 accepts **everything it visits** in both arms, so
     the base arm emits `min(ps_n, 256)` indices and the unmitigated hier arm emits `min(|S|, 256)`
     where `S` is the coarse-accepted set. At `ps_n = 512`, if `|S| >= 256` **and** the punctual
     prefix `P[0..256)` lies inside `S`, both arms emit the **identical 256-index prefix** —
     assertions 2 *and* 3 go **GREEN** while "coarse rejects >= 1" is perfectly true. The RED arm can
     then pass. At `ps_n = 128 < 256` no clamp is reachable, so `|S| < ps_n` forces different counts
     and **"coarse rejects >= 1 ⇒ assertion 2 RED" is a theorem**, which is exactly what
     rig-independence claims.
  2. **The precondition's source is the `inject_nan_froxel = None` mirror.** Measured *with* the
     injection and *with* the mitigation, group 0's coarse box is the universe and rejects **zero**
     lights — so read literally, Rev 5's precondition made every correct GREEN run report INVALID.
     The count is taken from H1 run on the same config with `inject_nan_froxel: None`. *This is sound
     in the conservative direction, and that is a derivation rather than a hope:* the un-injected box
     is the extremum over all 144 lanes, the unmitigated arm's box is the extremum over the 143
     non-poisoned lanes (the NaN lane is dropped by `NMin`/`NMax`, §5.1), so the unmitigated box is
     **contained in** the un-injected box; by §5 Step 2's monotonicity a contained box rejects at
     least as much. "The None mirror rejects >= 1" therefore **implies** "the RED arm's box rejects
     >= 1", never the reverse.
  3. **The count itself must exist.** It does not today — §8.6's H1 deliverables define seven outputs
     and `HierCullStats` (`crates/boyko_rhi_vulkan/src/goldens.rs:3657`) exposes exactly `groups`,
     `ps_n`, `valid_lanes`, `pairs_coarse`, `pairs_fine`. **Rev 6 adds a per-group coarse-accept count
     to §8.6** (deliverable 8); this bullet is its only consumer.

  On the bench rig at `N >= 64` slice 0 is a thin near-plane slab and rejects nearly every light, so
  part 1's requirement is satisfied with margin; it is asserted rather than assumed.
* **Injection point (Rev 6 — unstated in Rev 5, and only one placement works).** The poison is written
  **after phase 0's AABB build and BEFORE phase 1's finiteness test** (`bool finite = all(abs(aabb_min)
  <= 1.0e30) && ...`, §4's phase-1 HLSL). Placed *after* the finiteness test, the lane is classified
  `finite` from its pre-poison AABB, never stores the absorbing `±FLT_MAX`, and the mitigation **never
  engages** — the GREEN arm then goes red for a reason that has nothing to do with what (vii) tests.
  The same ordering applies in all three implementations (HIER module, base module, host mirror).
* **Saturation check:** froxel 168 alone claims `min(N, max_lights_per_cluster)` indices in the
  mitigated arm — `min(N, 256)`, which at the pinned `N = 128` is **128** (Rev 6 P2-4: Rev 5's "claims
  `N` indices" contradicted its own next sentence). At `N = 512` the total would be
  `~2 597 + 256 = ~2 853`, and at `N = 128` it is smaller still, against `INDEX_LIST_CAP = 16 384`, so
  §6's `alloc_total < index_list_cap` precondition (assertion 1) holds with two orders of margin and
  the run is non-saturating. Froxel 168's own count is clamped by `max_lights_per_cluster = 256`
  identically in all three implementations — which is precisely why part 1 above pins `ps_n` below it.

**Deleted assertions, recorded rather than dropped:**

| Rev 3 assertion | why it is deleted |
|---|---|
| "the `-D HIER=1` module contains **exactly one `OpReturn`**" | **vacuous** — measured (M7): the correct probe, the early-return probe and the fully-broken probe all emit exactly 1. DXC canonicalises to a single exit block |
| "every `OpControlBarrier` sits in a **merge block**" | **unsound in both directions** — measured (M8): the correct shader's first barrier is not in a merge block (false RED), and all three of the broken shader's barriers are (false GREEN) |
| "the **TWO** `OpFOrdLessThanEqual`" | **ill-posed** — §5's finiteness predicate adds two `%v3bool` compares, so a §4-conformant HIER module has **four**. Replaced by a split-by-result-type count (M11) |
| "with validation ON the buffer-overrun must be reported" (H3 mutation (v)) | **unattainable** — only `SYNCHRONIZATION_VALIDATION` is enabled (`device.rs:2087`), no GPU-assisted validation exists in the repo, `robustBufferAccess` is off. Replaced by detector (A) |
| "the `0xFFFFFFFF` probe proves every froxel was written **exactly once**" | **overclaim** — it proves at-least-once. Restated as A1; exactly-once comes from §8.2(B) |

### 8.4 HP — land §1.3's occupancy probe as a committed test (was §12's prose precondition)

Rev 3 carried this as a paragraph in §12 saying a commit "must precede approval". Rev 4 makes it a
rung with files, assertions and a RED-if, because a precondition with no gate is the same defect this
revision is about.

* **Where:** one added `#[test]` in `crates/boyko_rhi_vulkan/tests/lighting_l1_host_oracle.rs` — the
  file already exists (8 `#[test]`s today, fixture `cfg()` at `:29`), it is already H1's named target,
  and it already owns the host-oracle domain. **No new file, no new fixture, no GPU.**
  *Rejected:* committing the probe as a `scratchpad/` text file (an untested, unrun artifact that
  rots exactly like the doc-comment §1.4 refutes); a new `cluster_cull_occupancy_probe.rs`.
* **Name:** `cluster_cull_occupancy_profile_matches_the_published_table`.
* **What it drives:** `golden_cluster_cull` (`crates/boyko_rhi_vulkan/src/goldens.rs:3510`) with the
  VB-P1d camera — eye `(0, 1.1, 7.8)` looking at `(0, 0.55, 0)`, `fov_y` 52 degrees, aspect 1.0,
  512x512 — and the bench rig reproduced from
  `crates/boyko_app/tests/vb_p1d_cull_shade_bench.rs:124` (`light_position`) and `:142`
  (`light_range`), against `ClusterConfig::default()` (16x9x24, `z_near` 0.1, `z_far` 50.0).
* **What it asserts:** for `N_ps` in `{8, 14, 32, 64, 128, 256, 512, 1024}`, exactly §1.3's three
  columns — `total_indices`, `non_empty_froxels`, `max_per_froxel` — as **literal expected values**
  (`789/514/3`, `1239/543/5`, `1916/557/10`, `2063/364/15`, `1654/143/24`, `2072/115/40`,
  `2597/85/64`, `2709/55/109`), plus `total_indices < INDEX_LIST_CAP` and
  `max_per_froxel < MAX_LIGHTS_PER_CLUSTER` (which is simultaneously §6's saturation discharge).
* **Gate:** the test passes with the literals above.
* **RED if:** any of the 24 literals differs. **Concrete mutation that must turn it red:** change the
  camera's `fov_y` from 52 to 53 degrees — the froxel-to-pixel tiling moves and `non_empty_froxels`
  shifts. (Run once during review; this rung's whole point is that the table is *reproducible*, so a
  mutation that perturbs the scene must perturb the table.)
* **Why it is a rung and not a footnote:** §7's entire fine-pair column, §6's saturation discharge and
  §10's ABORT criterion rest on this table. Prose cannot re-derive a measured table. **Rev 4 may not
  be approved until this commit lands**, and §1.3 is then re-anchored on
  `lighting_l1_host_oracle::cluster_cull_occupancy_profile_matches_the_published_table` instead of on
  a scratch file.
* **Ordering:** it is provenance for the *plan*, not for the implementation, which is why it lands as
  its own commit rather than inside H1 — H1 *hardens* it into a matrix, it does not replace it.

### 8.5 H0 — Instrument the fixed cost (no behaviour change, and **no new framegraph access**)

* **Files:** `crates/boyko_rhi_vulkan/src/present/gpu_timing.rs` (+1 `VbTimedPass` slot;
  `VB_PASS_COUNT` 2 to 3), `crates/boyko_rhi_vulkan/src/present/passes/vb.rs:140-247` (split the
  `LightCull` bracket into `CullReset` = fill+barrier and `CullDispatch`),
  `crates/boyko_app/src/runner.rs` (print both).
  > **Citation staleness, stated once (Rev 5 P2).** Every `vb.rs` line number in this document
  > (`:140-219`, `:146-150`, `:157`, `:184`, `:211`, `:216-219`, and the appendix's VB-record-site row)
  > is exact against `git show HEAD:` at base commit `dc0684e` — and goes **stale the instant H0
  > commits**, because H0 inserts two timestamp brackets into that exact span. They must be
  > **re-anchored after H0 lands**, before H2 cites them again. This is a documentation obligation with
  > a named trigger, not a defect.
  > **Measurement defect under arbitration (see the Rev 5 status block):** in the landed H0,
  > `CullDispatch`'s begin is a `TOP_OF_PIPE` write placed *after* the `TRANSFER→COMPUTE` barrier. A
  > `TOP_OF_PIPE` timestamp is **not ordered** by that barrier, so it may latch before the fill retires
  > and the split can **over-count `CullDispatch` by `t_fill`**. Being measured at `N = 8` and
  > `N = 512`; **`N = 512` alone cannot detect it** (the fixed cost is 2.80 % of that cull versus
  > 70.6 % at `N = 8`). If confirmed, the fix is to move the begin write before the fill and derive
  > `CullDispatch` by subtraction, or to bracket with `BOTTOM_OF_PIPE` on the begin side; either way
  > H0's gate ("the two sub-brackets sum to `froxel_cull_ns` within 5 %") is re-run.
* **Removed from Rev 2's H0, and kept removed:** "plus `alloc_total` read back from
  `LightIndexAlloc[0]`". That readback **leaves the present path entirely** (§P1-E). Rev 2's shape
  would have appended a `TRANSFER_READ` to `light_index_alloc`, whose declared seed is
  `ResSync::seeded_writer(COMPUTE_SHADER, SHADER_WRITE)` (`graph_bridge.rs:3187-3190`); the frame-end
  state would then be `visible = TRANSFER/TRANSFER_READ, flush = 0`, which that seed no longer
  describes — the same shape as the WAR race fixed at `5e07936`. It is non-racy today only because
  `runner.rs:2069` calls `ctx.wait_idle()` on every armed frame, and an undocumented dependency on an
  incidental `wait_idle` in a different crate is a landmine, not an invariant. `alloc_total` comes
  from H3's host-visible cull-only driver and H1's CPU oracle (§6). **What H0 can no longer prove:**
  nothing about `alloc_total`; its scope is strictly the fixed-cost attribution.
* **Why first:** §1.2's "13.9 us is fill+barrier" is a *hypothesis*. §1.1 is this campaign's standing
  reminder that unmeasured hypotheses about this shader have already cost one 2x regression. If the
  fixed cost turns out to be dispatch-intrinsic rather than barrier-intrinsic, §7.1's follow-up list
  changes and the low-`N` predictions move.
* **DELETED from Rev 4's H0 (Rev 5 P1): "prints the device's SM count from
  `VkPhysicalDeviceProperties`".** It is unimplementable as written — core `VkPhysicalDeviceProperties`
  exposes **no** SM count; it requires `VK_NV_shader_sm_builtins::shaderSMCount`, which this device
  never enables, and `grep -rniE "sm_count|shaderSMCount|multiprocessor|SHADER_SM_BUILTINS" crates/`
  returns **zero** hits. Adding the extension query is its own rung and is not worth one paragraph of
  §D3. **Consequence, stated where §D3 can see it:** the 28 SMs is an **owner-stated device fact**, not
  a measurement, and §D3's occupancy prose is worded that way. No gate consumes it.
* **Follow-up (one line, NOT done here):** `crates/boyko_app/tests/vb_p1d_cull_shade_bench.rs:47`'s
  doc-comment still documents the **pre-split** print line
  (`froxel_cull_ns=.. froxel_shade_ns=.. froxel_total_ns=..`). No machine parser reads it, so nothing
  is red — it is a stale doc-comment, and it is recorded here rather than fixed silently.
* **HANG WARNING (a real defect in Rev 2's wording, re-verified for Rev 4).** `read_vb_bench_ns` uses
  `VK_QUERY_RESULT_WAIT_BIT` and will **hang forever** on any timestamp pair a code path fails to
  write. Both new pairs must therefore be written on **every armed frame**, including a flat-leg boot
  where `scene.cluster_cull` is `None` — i.e. the new `write_begin`/`write_end` calls go **outside**
  the `if let (Some(cull_pipeline), …)` gate at `passes/vb.rs:206`, exactly as the existing
  `LightCull` bracket does (`:146-150`, `:216-219`). Rev 2 placed the split inside that gate.
* **Gate:** the bench prints `cull_reset_ns + cull_dispatch_ns`, and their sum reproduces the existing
  `froxel_cull_ns` **within 5 % at `N` in {8, 512}**; every golden pin byte-identical.
* **RED if:** any pin moves (timestamp writes must not perturb rendering results); the sub-brackets do
  not sum within 5 %; the run hangs (implying an unwritten pair). **Concrete mutation:** move the new
  `write_begin` inside the `if let` gate at `:157` and boot a flat leg — the run must hang. *(This is
  the one rung whose primary failure mode is a hang rather than an assertion, which is why the
  mutation is named.)*

### 8.6 H1 — CPU oracle: the host hierarchical mirror + the permanent set/occupancy/selectivity gate

* **Files:** `crates/boyko_rhi_vulkan/src/goldens.rs` (+ `golden_cluster_cull_hier`, a
  block-decomposed mirror of `golden_cluster_cull`:3510 using D2's min/max merge),
  `crates/boyko_rhi_vulkan/tests/lighting_l1_host_oracle.rs` (+ the matrix test, beside HP's pin,
  which this rung *hardens*, not replaces).
* **What it asserts, per config in the matrix:**
  1. `golden_cluster_cull_hier == golden_cluster_cull` **exactly**, per froxel, **including order**.
  2. **Coverage as a PERMUTATION, not merely a cover** (strengthened in Rev 4, and this is the
     assertion that catches P0-1 on the CPU): the multiset of `fi` produced by all `valid` (group,
     lane) pairs is **exactly** `[0, capacity)` — no duplicate, no gap, no index `>= capacity` — and
     separately, the number of `valid` lanes equals `capacity`. *That second clause is what §8.2(B1)
     consumes to turn device totality into device exactly-once, so it is asserted explicitly rather
     than implied by the first.*
  3. `total_indices < INDEX_LIST_CAP` and `max_per_froxel < MAX_LIGHTS_PER_CLUSTER` (§6, and it pins
     §1.3's table as a regression).
  4. All AABB bounds finite on the well-formed configs. *(Defence in depth only: §5 Case B means a
     non-finite AABB is handled on device, so this assertion documents the rigs, it does not protect
     the shader. Rev 3 implied the reverse.)*
  5. **Selectivity (the perf premise):** `pairs_hier / pairs_flat <= 1/8` on the bench rig at
     `N >= 128`. This is a *pair-count gate that runs on the CPU in 0.45 s with no GPU*, and it is the
     **only** gate that can see mutation (viii) (§8.3).
  6. **Mask-capacity boundary.** A config with `l0a_count == 0` and `point_spot_count == MAX_LIGHTS`
     (1024) must be present, so **mask word 31 / bit 1023** is exercised, and the produced set must
     still equal the flat oracle. *Reason:* every other config leaves word 31 dark — `light_count` is
     clamped to 1024 by the host fold, so any directional/sky light pushes the point/spot span below
     1024. A 20 000-trial randomized simulation hit word 31 in only 196 runs.
  7. **The host-to-shader mapping pin (D11).** Replicate the shader walk (`gps`/`slice`/`s`/`x`/`y`/
     `z`/`fi` + the three-term `valid`) on the host over a dims matrix that **includes non-64-aligned
     and degenerate grids** — 16x9x23, 1x1x1, 0x0x0, 255x255x255 — and assert assertion 2's
     permutation property on each. Pure arithmetic, no GPU.
     **Explicit scope limit (P2):** this is a **Rust re-implementation of the shader's walk, not a pin
     on the HLSL.** If the shader and the mirror drift, only H3 sees it. Rev 3's §9 implied otherwise.
  8. **Per-group coarse-accept count (NEW in Rev 6 — it is a PRECONDITION SOURCE, not a diagnostic).**
     `HierCullStats` gains a per-group count of coarse-accepted punctual lights (population of the
     group's coarse mask), alongside the existing `groups` / `ps_n` / `valid_lanes` / `pairs_coarse` /
     `pairs_fine` (`crates/boyko_rhi_vulkan/src/goldens.rs:3657`). *Why it is a deliverable:* §8.10's
     mutation (vii) protocol requires the driver to assert "group 0's coarse box rejects at least one
     punctual light" **before** evaluating the arm comparison, and Rev 5 cited that count as if H1
     already produced it — it does not, so the precondition had no source. The count is read from the
     **`inject_nan_froxel: None`** run of the config (§8.3, mutation (vii), rig requirement part 2).
     *It is not a new gate:* `pairs_fine` is already the sum of these counts over valid froxels, so
     assertion 5's selectivity number is unchanged and this deliverable only exposes the per-group
     decomposition H1 already computes.
* **Matrix (six grid configs — Rev 2's two were both `gps = 1` and could not test D3 at all):**

  | entry | dims | `dim_x·dim_y` | `gps` | `G` | what it alone catches |
  |---|---|---|---|---|---|
  | M1 | 16x9x24, ORTHO 64x64 (the `l1_cluster_config` fixture, `sdf_gbuffer_hybrid.rs:5215`) | 144 | 1 | 2688 | the shipped fixture; **mutation (i)** needs `G > 0` |
  | M2 | 16x9x24, PERSPECTIVE 512x512 (the VB-P1d camera) | 144 | 1 | 2688 | the bench camera |
  | E1 | 16x16x24 | 256 | 1 | **0** | the `gps=1` boundary **from above**; a `<` vs `<=` slip in `ceil(dim_x·dim_y/256)` |
  | E2 | 32x16x24 | 512 | 2 exact | **0** | a degenerate mapping (`slice = gid; s = lane`) — provably indistinguishable from the correct one at `gps=1`, and here it misses 6144 of 12288 cells |
  | E3 | 16x17x24 | 272 | 2 ragged (16 of 256 lanes valid in the tail group) | 5760 | D8's identity-element corollary under load (240 identity lanes must not perturb MIN/MAX), `valid` gating **both** phase 6 and the fine walk, **and** the only `gps >= 2` config with `G > 0` |
  | E4 | 32x24x24 | 768 | 3 exact | **0** | an off-by-one or a hardcoded `gid >> 1` in `gid / gps`, which a `gps=2` case masks exactly |

  The `G` column is new in Rev 4 and is load-bearing: it tells the H3 mutation protocol which configs
  can exercise detector (A2) at all (§8.2 limit 1).

  Crossed with {bench Kronecker rig, corrected R3 rig, dense in-frustum rig, adversarial boundary
  rig} x `N` in {0, 1, 8, 64, 128, 512, 1024}. The **adversarial** rig places lights so that
  `sq_dist_point_aabb == r*r` exactly for a chosen froxel, and at `r ± 1 ulp`, on faces, edges and
  corners of the AABB — the boundary of the `<=` test, which is where a non-conservative coarse level
  would first fail. **Rig requirement (new, from §8.3):** the chosen froxel must **not** be the one
  lane 0 holds, or mutation (ii) is invisible; and at least one config must give a single froxel two
  accepted lights in two different mask words, or mutation (iii) is invisible.

  **Runnability, verified structurally:** nothing on the path is hardcoded to 16x9x24 —
  `runner.rs:637-642` reads the live `ClusterConfig` Resource, `gpu_scene/mod.rs:4317-4338` sizes
  every buffer from it, `passes/vb.rs:215` derives the dispatch from `scene.cluster_count`, and the
  base shader reads dims from the header. The host oracle is dim-generic (`GoldenClusterConfig`
  `goldens.rs:3398`, `golden_cluster_index(x,y,z,dim_x,dim_z)` `:3437`). The only hard limit is
  `packed_dims`' 8 bits per dim (`light.rs:763-769`), which all six configs respect. E2/E3/E4 set
  `index_list_cap = cluster_count * 8` (the `sdf_gbuffer_hybrid.rs:5230` idiom) so the cap does not
  bind — E4's 18 432 froxels are 5.3x the default.
* **Gate:** all seven assertions green over the whole matrix.
* **RED if:** any froxel's index vector differs in content or order; a light is dropped at the range
  boundary; selectivity misses 8x; assertion 2 or 7 reports a duplicate, gap or out-of-capacity index.
  **Concrete mutations that must turn it red, each executed once during review:**
  * scale the coarse extents inward (`MIN *= 1.001; MAX *= 0.999`) — a non-conservative coarse box —
    and the adversarial rig must fail assertion 1;
  * apply §8.3's mutation **(vi)** (`slice = gid; s = lane`) — assertion 2 must stay green at M1/M2
    (it is bit-identical there) and go red at E2/E3/E4 with the simulated gap counts (6144 / 384 /
    12288);
  * apply §8.3's mutation **(viii)** (absorbing element for `!valid` lanes) — assertion 5 must go
    from about 1/55 to 1.0 and miss the 1/8 gate.
* **What H1 can prove, and what it explicitly cannot.** It falsifies the **pair-count premise** — a
  *necessary* condition, and the campaign's cheap kill switch. It is **not sufficient**: it cannot
  see thread count, barrier cost, the 43.75 %-idle fine phase, hot-group serialization, or the
  `ClusterGrid` write pattern. Rev 2's one-line verdict claimed otherwise and is withdrawn (§7.0).
* **Abort point:** if selectivity on *both* the bench rig and the in-frustum rig is below 4x, the rung
  stops here at zero GPU cost and the plan is rewritten (§10).

### 8.7 H1.5 — Dispatch-shape transfer probe (no new shader, no new `.spv`, no framegraph change)

* **Files:** `crates/boyko_app/tests/vb_p1d_cull_shade_bench.rs:278` — replace
  `app.insert_resource(ClusterConfig::default())` with a swept
  `ClusterConfig { dim_x, dim_y, dim_z, ..Default::default() }`, **one app boot per config** so
  D11's boot-snapshot hazard is not exercised (`dim_x/dim_y/dim_z` are `pub` fields).
* **What:** at fixed `N_ps = 512`, measure `froxel_cull_ns` on the **existing flat arm** at grids
  8x9x24 (1 728 froxels), 16x9x24 (3 456, the anchor), 16x9x48 (6 912) and 32x18x24 (13 824), and fit
  against `froxels x N`.
* **Why:** §1.2's own model-validity caveat says `0.2736 ns/pair` is calibrated on **one** dispatch
  shape. This is the only test of whether it transfers that costs no shader bytes. Run E2's dims
  (32x16x24) through the **base** pipeline here as well, before the hier arm exists — if the base arm
  is green at `gps >= 2` dims then the config plumbing is proven independently, and any later
  `gps >= 2` failure is attributable to the hier mapping alone.
* **Gate:** the fitted rate is within **±25 %** of 0.2736 ns/pair — i.e. in
  **[0.2052, 0.3420] ns/pair** — at **every one of the four froxel counts**, not merely on the
  aggregate fit. (Rev 3 said "across the 8x froxel range", which an aggregate fit can satisfy while a
  low-froxel point sits far off the line.)
* **RED if:** any of the four points falls outside that band. In particular, if the **1 728-froxel**
  point's implied rate exceeds **0.3420 ns/pair**, a **latency floor** exists.
* **What "a latency floor is found" means, and what re-derivation follows (P2 — Rev 3's ABORT clause
  2 defined neither).** A latency floor is exactly the RED condition above. On it: refit
  `cull_ns = a + b·(froxels·N)` over the four measured grids, take `b_hi` as the largest per-point
  implied rate, and re-evaluate §7's `N=512` prediction as `a + b_hi x pairs_hier(512)` with
  **`pairs_hier(512) = 45 840`** — H1's **measured** count on config M2 / the bench Kronecker rig,
  **not** Rev 4's modelled `32 300` (Rev 6 P1: the literal was stale by 1.42x, and it is executed here
  and at §10 ABORT clause 2). That re-derived number is what §10 ABORT 2 compares against 250 000 ns.
  The refit and the re-derived number are recorded **in this document** before H2 is committed.
* **What it can no longer be claimed to prove:** it bounds *thread-count* scaling on a **balanced**
  dispatch. It says nothing about barrier cost, idle lanes or hot-group serialization; those are
  H4's (§7.0).
* **Note:** raise `index_list_cap` (or assert §6's `alloc_total < index_list_cap`) at the
  13 824-froxel point.

### 8.8 H1.6 — The D10 `precise` edit and the **one-time** base `.spv` re-pin (no HIER code)

* **Files:** `crates/boyko_rhi_vulkan/shaders/cluster_cull.hlsl` (`sq_dist_point_aabb` only, D10's
  body verbatim — the **7-decoration** form), `crates/boyko_rhi_vulkan/shaders/cluster_cull.comp.spv`
  (re-pinned once), `crates/boyko_rhi_vulkan/src/goldens.rs:3488-3490` and
  `crates/boyko_rhi_vulkan/tests/sdf_gbuffer_hybrid.rs:5187-5188, :6291-6293` (doc-comments: the
  GPU-to-host bit-exactness of the cull distance is now **structural** — matching `((dx^2+dy^2)+dz^2)`
  association, no fusion on either side — rather than incidental; the same note
  `shaders/ddgi_resolve.hlsli:136-141` already carries for DDGI).
* **Why a rung of its own:** it isolates the base-arm ULP perturbation from the hierarchical change,
  so H3's arm-vs-arm equality oracle compares two already-`precise` arms and a moved pin here has
  exactly one possible cause.
* **Gate (Rev 4 gives this the numeric form §2 has — Rev 3's was prose):**
  * **(a)** `cluster_cull_spv_sync` green under the unchanged frozen recipe; the re-pinned blob is
    **12 616 B** with **7** `NoContraction` and **8** `OpDot` (M4 — the expected values are written
    into the commit message, so a wrong `precise` placement is caught at review, not at H2).
  * **(b)** `lighting_l1_host_oracle` green (including HP's pin and H1's matrix).
  * **(c)** `sdf_gbuffer_hybrid::l1_known_light_lands_in_the_expected_clusters` and
    `l1_clustered_resolve_matches_the_brute_force_image` green.
  * **(d) Golden-move budget: ZERO.** Every image golden must be byte-identical. A moved pin is **not**
    re-pinned as a matter of course — it **stops the rung** until the mover is identified, and it may
    only be re-pinned after the ULP explanation is *demonstrated* (recompute both `sq_dist` values for
    the offending light on the host, in both associations, and show the `<=` flip), with that
    demonstration recorded in the commit message. Rev 3 allowed "moved-and-explained", which in
    practice degrades to "moved".
  * **(e) Performance, stated numerically:** `froxel_cull_ns` from `vb_p1d_cull_shade_bench.rs` at
    `N_ps` in {128, 512}, **three runs before and three runs after**, recorded in this document with
    their spread. Gate: **after-median <= before-median x 1.05** at both `N`. *(5 % is chosen because
    §1.2's own model reproduces the measured table to within 8.9 % at the worst point and 0–1.5 % at
    `N >= 128`; the three-run requirement exists because a single sample cannot distinguish a 2-op
    regression from run-to-run noise, and Rev 3's "beyond run-to-run noise" never said how noise
    would be measured.)*
* **RED if:** any of (a)–(e) fails. **On a measured regression in (e), fall back to D10's named
  alternative** — slack `r*r*(1.0 + 0x1p-20)` on the **coarse comparison only**, base arm untouched —
  and re-run this gate. **Concrete mutation for (a):** revert to plain `dot(d, d)`; the byte gate and
  the recorded census must both fail.
* **What it changes about the plan's other claims:** D5's "base `.spv` byte-frozen" does not apply
  before this commit; from this commit onward it does.

### 8.9 H2 — The `-D HIER=1` shader variant (dark infra)

* **Files:** `crates/boyko_rhi_vulkan/shaders/cluster_cull.hlsl` (the `#ifdef HIER` arm, §4),
  `crates/boyko_rhi_vulkan/shaders/cluster_cull_hier.comp.spv` (new, offline dxc 1.4.350.0),
  `crates/boyko_rhi_vulkan/src/compute.rs` (+ `cluster_cull_hier_spirv()` beside
  `cluster_cull_spirv()` at `:1610`, + the `ClusterCullHierPush` mirror per D11),
  `crates/boyko_rhi_vulkan/tests/cluster_cull_spv_sync.rs` (adopt the multi-variant idiom of
  `vb_froxel_spv_sync.rs:88-130`),
  `crates/boyko_rhi_vulkan/tests/cluster_cull_hier_dis_gate.rs` (**new**, below),
  `docs/SHADER-VARIANT-MANIFEST.md:91-97` (+ one row). Pipeline **built but never selected**.
* **Gate:**
  * **(a)** `cluster_cull_hier.comp.spv` byte-equals its re-DXC under the frozen recipe.
  * **(b)** `cluster_cull.comp.spv` byte-equals its re-DXC **with no `-D`** — i.e. the base arm is
    physically unperturbed by the seam. **Re-measured at this rung, not inherited:** Rev 3 measured
    the inertness with a *one-word* push tail, and D11 now adds a second word, so the previous
    measurement does not cover the artifact being committed. Note this is against the
    **H1.6-re-pinned** blob (12 616 B), not the original.
  * **(c)** every golden pin unchanged (nothing selects the variant).
  * **(d)** `cargo clippy --workspace --all-targets -- -D warnings`.
  * **(e) The structural tripwire** (`cluster_cull_hier_dis_gate.rs`, cloning the `spirv-dis` locator
    and skip semantics of `crates/boyko_rhi_vulkan/tests/field_probe_gate.rs:43-105`, precedent
    documented at `shaders/sdf_field.hlsli:146-148`). It re-DXCs both variants **into the temp dir** —
    never overwriting a committed artifact — disassembles, and asserts:

    | # | assertion | on | mutation that turns it RED | verified? |
    |---|---|---|---|---|
    | e1 | `OpDot == 8` (the `dot(rd, cam_forward.xyz)` in `view_z_to_t` `:87`, 4 corners x near/far — **zero** in the cull comparison) | both | restore `dot(d,d)` in `sq_dist_point_aabb` | yes (M3/M4: base+D10 has 8; committed has 9) |
    | e2 | `OpDecorate … NoContraction` count **== 14** on HIER, **== 7** on base — **exact integers, not lower bounds** | both | drop `precise`; OR pass an argument *expression* to `sq_dist_point_aabb` (M6: gives 16) | yes (M4 = 7, M6 = 14) |
    | e3 | **scalar** (`%bool`) `OpFOrdLessThanEqual` count **== 2** on HIER, **== 1** on base | both | delete one call site | yes (M11 distinguishes `%bool` from `%v3bool`) |
    | e4 | **vector** (`%v3bool`) `OpFOrdLessThanEqual` count **== 2** on HIER, **== 0** on base | both | delete §5's finiteness predicate | yes (M11: `all(abs(v) <= 1e30)` emits `%v3bool` compares) |
    | e5 | the id-normalised 14-instruction window ending at each **scalar** `OpFOrdLessThanEqual` **whose first operand is a `NoContraction`-decorated `OpFAdd`** is byte-equal to the other | HIER | make the two sites structurally different | yes (M4: the base module's scalar compare has an `OpFAdd` **first** operand and an `OpFMul` second, so the selector is well-defined). *Rev 5 P2: Rev 4 printed concrete ids here; they were mis-transcribed and are NOT re-quoted — no reviewer can re-verify an id without re-running dxc, and the selector does not depend on them. The selector itself is unchanged and still correct.* |
    | e6 | **producer assertion, scoped to the `NoContraction`-DECORATED `OpFSub` only** (i.e. the two inside `sq_dist_point_aabb`): neither of their operands is an `OpFMul` result. **The scope is mandatory** — the module carries ~24 *undecorated* `OpFSub` in ray-gen whose operands **are** `OpFMul` results, so an unscoped form false-REDs a correct module (Rev 5 P2) | both | give one site an `OpFMul`-produced operand | **this is the one an id-normalised window cannot see** — normalisation erases operand provenance, so e5 alone stays green |
    | e7 | the push block has **6** members on HIER with `Offset 16` on `cluster_dims_packed` and `Offset 20` on `cluster_capacity`; **4** members on base with last `Offset 12` | both | widen the shared struct instead of the `#ifdef` arm | yes (Rev 3 measured the 5-member form; H2 re-measures at 6) |
    | e8 | **every `OpControlBarrier` lies on the entry function's TOP-LEVEL BLOCK CHAIN** | HIER | early `return` anywhere in the arm; a barrier under divergent control flow | **yes, executed (M9)** — see below |

    **e8's definition, because it must be implementable without ambiguity.** Start at the entry
    function's first block. From a block, follow its `OpSelectionMerge`/`OpLoopMerge` **merge target**
    if it has one, else its unconditional `OpBranch` target; stop at a terminator with neither. The
    blocks visited are the top-level chain. Assert every barrier's block is in that set.
    **Measured (M9)** on three probes compiled under the frozen recipe: correct 256-lane shape
    (no early return, 3 top-level barriers) → chain of 5 blocks, **3/3 barriers on it, GREEN**;
    early `return` **alone** → chain of 2 blocks, **0/3, RED**; early `return` + a barrier under
    `if (lane < 16)` → **0/3, RED**. **This replaces the two Rev 3 assertions that were measured
    non-discriminating** (M7: all three probes emit exactly 1 `OpReturn`; M8: the merge-block test
    false-REDs the correct shader and false-GREENs the broken one).
    **Stated scope:** e8 is a **design-conformance check on §4's shape** (all three barriers at top
    level), *not* a general legality check — a barrier inside a group-uniform loop would be legal
    Vulkan and would fail e8. §4 has no such barrier, and if a future rung adds one, e8 changes with
    it rather than being silently relaxed.
  * **(f) The `#error` guards, made MECHANICAL (P2).** Rev 3 ran these "once during review" while §9
    counted them as a mechanical gate. The dis-gate test now copies `cluster_cull.hlsl` into the temp
    dir with `#define HIER_MASK_WORDS 32u` rewritten to `64u`, invokes dxc with `-D HIER=1`, and
    asserts the compile **fails**. **Mutation that turns it red:** delete either `#error` block — dxc
    then succeeds and the assertion fires.
  * **(g) Source-text pin for D8 (belt to e8's braces):** read `cluster_cull.hlsl` (the
    `shaders_dir()` + `read_to_string` idiom at `cluster_cull_spv_sync.rs:20-22`) and assert the
    region between `#ifdef HIER` and its `#endif` contains **no `return` token**. Cheap, and it names
    the defect in the language a reviewer reads rather than in SPIR-V.
  * The test's doc-comment **must state that (e) is a tripwire, not the proof** — the proof is §5
    Step 0 — because contraction is decided below the `.spv` (DXC emits zero `Fma`).
* **RED if:** the base `.spv` moves by one byte (the seam leaked into the `#else` arm); any (e) count
  differs; (f) compiles; (g) finds a `return`; the manifest row is missing (the `-D` matrix must stay
  enumerable by one grep).

### 8.10 H3 — The GPU set-level equality and memory-safety oracle `[P0-1]`, `[P0-2]`, `[P0-3]`

* **Files:** `crates/boyko_rhi_vulkan/tests/cluster_cull_hier_equiv.rs` (new) + a **cull-only**
  driver: camera UBO + light table + the three buffers + one dispatch + three readbacks. It does
  **not** go through `run_gbuffer_hybrid_lit_clustered` (`sdf_gbuffer_hybrid.rs:5276`) — no SDF, no
  resolve, about 10x faster, and it can drive a PERSPECTIVE camera trivially. The driver creates
  `LightIndexAlloc` as `MemoryLocation::HostVisibleCoherent` and reads `alloc_total` through
  `buffer_mapped_ptr` **after the fence**, following the post-fence mapped-read idiom at
  `tests/sdf_gbuffer_hybrid.rs:6202-6211` / `:6219-6228` — **no `vkCmdCopyBuffer`, no staging buffer,
  no framegraph resource** (§6, §P1-E).
* **Driver requirements introduced by §8.2's detectors** (these are part of the rung, not optional
  hardening):
  * `ClusterGrid` is allocated at **`capacity + G`** cells and **pre-filled with `0xFFFFFFFF`
    immediately before EACH arm's dispatch**, with `LightIndexAlloc` re-zeroed at the same point and
    **all THREE buffers — `ClusterGrid`, `LightIndexAlloc` and `LightIndexList` — read back after that
    arm's dispatch and before the NEXT ARM'S dispatch** (§8.2(A) limit 4 — Rev 5 P1, corrected in
    Rev 6). **The buffers are NOT shared across the two arms of a config**: a shared `ClusterGrid` lets
    mutation (vi) at E2 false-GREEN on the base arm's leftover cells, a shared `LightIndexAlloc`
    false-REDs `alloc_total == capacity` on whichever arm runs second, and a `LightIndexList` read only
    once at the end **false-REDs assertion 3 on a byte-identical pair** (the two dispatch widths claim
    different `InterlockedAdd` offsets, `shaders/cluster_cull.hlsl:183`, so arm A's offsets index arm
    B's list) **and false-GREENs assertion 8**, which is `[P0-4b']`'s only discharge;
  * the light table is allocated at **`MAX_LIGHTS + 1024`** rows with every row `>= light_count`
    filled with the **poison light** (`POINT`, at the camera eye, `range = 1e6`);
  * the **permutation probe** (§8.2(B2)) runs as its own configuration at **M1/M2 and at E3** — E3
    being the only `gps >= 2` config with `G > 0`, hence the only device measurement of exactly-once
    with a non-degenerate map;
  * a boolean **`allow_skew`** flag (default `false`), which scopes assertion 10 — see protocol item 3.
* **Why a new test rather than extending `l1_known_light_lands_in_the_expected_clusters`
  (`sdf_gbuffer_hybrid.rs:6432`):** that test is **ORTHO-only** (`CompositeCamera::Ortho`, `:6455`)
  and drives the *base* pipeline. Extended naively it would exercise the flat arm and pass green
  while testing nothing about the hierarchy — the exact failure mode `[P0-2]` names. It stays as-is
  (the flat arm's host cross-check); the hierarchy gets its own oracle that *cannot* be satisfied by
  the flat arm.
* **Asserts, per config (same matrix as H1, plus both arms on-device).** Each row names the mutation
  that turns it red; every write-set mutation was simulated in §8.3. **Evaluation discipline (Rev 5
  P1, completed in Rev 6):** assertions **1, 5, 6, 7, 8 and 10** are **per arm**, evaluated on that
  arm's own readback taken after that arm's dispatch and before the next pre-fill; assertions
  **2, 3, 4** compare the **two separately captured** readbacks; assertion **9** is structural. **Every
  one of the ten is now classified — Rev 5 left assertion 8 in no class at all, which is how an
  implementer ends up scanning `LightIndexList` once at the end.** The rule: *every assertion is
  evaluated on readbacks captured after **exactly one** arm's dispatch, and **no assertion may mix one
  arm's `ClusterGrid` offsets with the other arm's `LightIndexList`*** (§8.2(A) limit 4).

  | # | assertion | turned RED by |
  |---|---|---|
  | 1 | `alloc_total < index_list_cap` — the saturation precondition (§6). **Fails loudly**, does not silently compare clamped results | set `index_list_cap = 1` on a non-empty config |
  | 2 | Per-froxel `count` equal between arms | (ii) lane-0 fold, on the adversarial rig |
  | 3 | Per-froxel `LightIndexList[offset .. offset+count)` **equal as a sequence** (order included) | (iii) descending mask walk, on a two-word froxel |
  | 4 | Both arms equal to the host `golden_cluster_cull` set (per-froxel, as a set) | (ii); after H1.6 the host-to-GPU distance comparison is **structural** (D10), not incidental |
  | **5** | **TOTALITY (at-least-once) — detector A1:** no cell in `[0, capacity)` retains the `0xFFFFFFFF` sentinel | (v) re-specified (138 gaps); (vi) `slice=gid; s=lane` at E2/E3/E4 (6144 / 384 / 12288 gaps) |
  | **6** | **GUARD-TAIL INTEGRITY — detector A2 (NEW):** every cell in `[capacity, capacity + G)` still holds the sentinel | **(i) drop `valid` on phase 6** — 2688 tail cells cleared at M1/M2; (v) re-specified — 138 |
  | **7** | **EXACTLY-ONCE — detector B (NEW):** on the permutation-probe config, `alloc_total == capacity`, every `count == 1`, and the `offset` multiset is exactly `{0 … capacity−1}` | any duplicate in-range write (loses an offset and repeats another) |
  | **8** | **NO OUT-OF-RANGE LIGHT INDEX — detector C (NEW):** no emitted `LightIndexList` value is `>= light_count` | (iv) producer mutation: phase 4 loops `j < HIER_MASK_BITS` and the fine clamp is deleted |
  | 9 | Non-vacuity: at least one froxel non-empty, and the hier pipeline handle is asserted **distinct** from the base one | swap the hier handle for the base one — the test must fail, not silently pass |
  | 10 | **No-skew precondition (D11, D4 scope clause (b)):** the boot snapshot equals the live header dims, asserted as loudly as assertion 1 — **evaluated only when the driver's `allow_skew` flag is `false`**, which is every run except mutation (v)'s (Rev 5 P1, protocol item 3) | run a config that edits `ClusterConfig` after boot with `allow_skew == false` |

  Assertion 5 is **explicitly at-least-once**. Rev 3 wrote that it proves "every froxel was written
  exactly once"; it does not, and that overclaim is what let mutation (i) pass. Exactly-once is
  assertions 5 + 6 + H1 assertion 2 together (§8.2(B1)), and separately assertion 7.
* **Mutation protocol (new in Rev 4 — a mutation run on the wrong config proves nothing):**
  1. **(i)** must run on a config with `G > 0` — **M1, M2 or E3**. On E1/E2/E4 every lane is valid, so
     `G = 0` and the mutation is a **no-op**; a green run there is not evidence.
  2. **(vi)** must run on **E2, E3 and E4** for the RED arm *and* on **M1/M2** for the GREEN arm — the
     green arm is the demonstration that a `gps = 1`-only matrix is blind, and it is only meaningful
     because the mutation is bit-identical there.
  3. **(v)** requires **boot 16x9x23 with live 16x9x24** and *both* faults (delete `fi < capacity`
     **and** re-source the dims from `load_cluster_params`). Either fault alone is inert (§8.3).
     **It must therefore run with `allow_skew = true` (Rev 5 P1 fix).** Rev 4 made assertion 10 fire on
     "a config that edits `ClusterConfig` after boot" while the RED-if blanketed all ten assertions on
     every mutation run — so the (v) run aborted at its own precondition and never reached assertions
     **5/6**, leaving `[P0-B]`'s only discharge with **no executable gate**. With `allow_skew = true`,
     assertion 10 is skipped for this run **and this run only**, and the reviewer records the two
     expected failures **individually**: **assertion 5** (TOTALITY / detector A1) — **138 in-range
     cells still holding the `0xFFFFFFFF` sentinel** — and **assertion 6** (GUARD-TAIL / detector A2) —
     **138 cleared tail cells**. Recording them separately matters — a single "the run failed" is
     satisfied by either detector alone, and §8.3 pre-registers **both**.

     > **Rev 6 P1 fix — Rev 5 pre-registered this RED evidence against the WRONG assertion numbers.**
     > Rev 5 wrote "assertion 1's 138 gaps and assertion 2's 138 cleared tail cells" here, at §9
     > `[P0-B]` and in its own changelog, having read the detector labels **A1/A2** as table rows
     > **1/2**. The table says otherwise two screens up: the 138 gaps belong to **assertion 5** and the
     > 138 tail cells to **assertion 6**. The consequence is not cosmetic. **Assertion 1 is
     > `alloc_total < index_list_cap`** — a *precondition that is supposed to pass*, and mutation (v)
     > changes no lane's `nlocal`, so it is **unfallible on this run**: a reviewer following Rev 5
     > literally discharges `[P0-B]` off assertion 2 while **assertion 6 — the guard tail, the detector
     > that exists BECAUSE of the Rev 3 P0 — is never checked at all.**
     >
     > **CONTROL ARM (pre-registered, Rev 6).** On this config the **BASE** arm also clears tail cells,
     > and the number is **16**, not 138. Its dispatch is boot-sourced exactly as the shipped record
     > sites compute it — `scene.cluster_count.div_ceil(LIGHT_CULL_LOCAL_SIZE_X)`,
     > `crates/boyko_rhi_vulkan/src/present/passes/vb.rs:215`, with `LIGHT_CULL_LOCAL_SIZE_X = 64`
     > (`src/present/scene_types.rs:415`) — so boot `16x9x23 = 3312` gives
     > `ceil(3312/64) = 52` groups = **3328 threads**, while the base module's own guard is
     > `if (fi >= cluster_count) return;` over **LIVE** dims (`shaders/cluster_cull.hlsl:109-114`,
     > `3456`) and so admits every one of them. Threads `3312..3327` write **16** cells past the
     > boot-sized capacity, into the guard tail. **The count is 16 whether or not (v) is mirrored into
     > the base module**: at `3328 < 3456` that guard never fires, so deleting it changes nothing.
     > **This is not the mutation** — it is the base arm's
     > pre-existing live-dims read of its own bound, which is exactly the state D11 exists to prevent
     > on the hier side. **Therefore: an assertion-6 that fires 16 times is the CONTROL, and means the
     > mutation did nothing.** The (v) discharge requires the HIER arm at **138 / 138**; `16 / 0`
     > (16 tail cells, 0 gaps) is the base arm's signature and must not be recorded as `[P0-B]`'s
     > discharge.
  4. **Map mutations and bound deletions must not be combined in one run** — the combination can drive
     `fi` past `capacity + G`, which is real UB in the test process rather than a detected fault
     (§8.2 limit 2).
  5. **(vii)** is run **twice**: with the finiteness substitution (must be GREEN) and without it (must
     be RED). A one-sided run does not test the mitigation. **The injection is `fi == 168u`, all six
     AABB components, mirrored in the HIER module, the base module AND the host mirror** — Rev 4's
     one-arm `lane == 7` single-component form could not go green in the GREEN arm, and named different
     froxels in the two modules (§8.3, "Mutation (vii) in full"). **Rig requirement — three parts, all
     mandatory (Rev 6 P1; Rev 5's one-line form was unsatisfiable, insufficient AND cited a quantity
     that does not exist — see §8.3's rig-requirement bullet for the derivations):**
     * **(a) the run is pinned at `N = 128`**, i.e. `ps_n < max_lights_per_cluster = 256`
       (`crates/boyko_render/src/light.rs:53`). At `N = 512` the per-froxel clamp
       (`shaders/cluster_cull.hlsl:170`) can make both arms emit the identical 256-index prefix, and
       the RED arm passes while "coarse rejects >= 1" holds;
     * **(b) group 0's coarse box must reject at least one punctual light**, asserted **before** the
       arm comparison is evaluated, so a vacuous RED arm is reported as an invalid run — read from
       H1's **new deliverable 8** (per-group coarse-accept count, §8.6) on the **`inject_nan_froxel =
       None`** run of this config. Measured with the injection *and* the mitigation the count is the
       full punctual set (the coarse box is the universe), which would make every correct GREEN run
       report INVALID;
     * **(c) the poison is written after phase 0's AABB build and BEFORE phase 1's finiteness test.**
       After it, the lane classifies `finite`, the absorbing store never happens, and the GREEN arm
       reds for a reason unrelated to the mitigation.
  6. **(iv)** must run on a config with `light_count < ps_begin + HIER_MASK_BITS` — i.e. **NOT** H1's
     mask-capacity-boundary config (`l0a_count == 0`, `point_spot_count == MAX_LIGHTS == 1024`). There
     `ps_n == HIER_MASK_BITS`, so the mutated loop `j < HIER_MASK_BITS` is **identical to the correct
     one** and the mutation is a no-op. D7 states this; Rev 5 also states it **here**, because the
     protocol is what a reviewer executes (P2).
* **GPU matrix.** E2 (32x16x24, `gps=2` exact) and E3 (16x17x24, `gps=2` ragged) **must run on
  device** — the failure D11 names is an out-of-bounds *device* write with `robustBufferAccess` OFF,
  which a CPU mirror cannot exhibit; and E3 is the only `gps >= 2` config with `G > 0`. E4 (32x24x24,
  `gps=3`) **may stay CPU-only**, because it tests index arithmetic rather than device behaviour;
  this is stated explicitly rather than left ambiguous. Add the degenerate-header config
  (`packed_dims == 0`) as a hang / divide-by-zero probe. Keep the `N = 0` row: with no point/spot
  lights, `ps_n == 0`, the coarse loop body never runs, `gs_summary == 0`, the fine walk is empty and
  every froxel writes `uint2(0,0)` — covered by simulation as arithmetic, but the barriers on that
  path are only executed here.
* **RED if:** any of 1–10 fails, on any config, under any of the protocol's mutation runs.

### 8.11 H4 — Arm the variant for VB + the two-rig bench

* **Files:** `crates/boyko_app/src/gpu_scene/mod.rs` (pipeline choice + the
  `ClusterCullHierDispatch` write in `build_froxel_light_cull`, `:4241`, beside `:4346`, **plus the
  release `assert!` on the `<= 255` per-dim contract**, D11),
  `crates/boyko_rhi_vulkan/src/present/scene_types.rs` (the new `Option` field),
  `crates/boyko_rhi_vulkan/src/compute.rs` (the second push mirror, if not already landed at H2),
  `crates/boyko_rhi_vulkan/src/present/passes/vb.rs:215` (the one `match`; the `// SAFETY:` clause
  rewritten per D11, **including its new SCOPE sentence**),
  `crates/boyko_rhi_vulkan/tests/window_present_gbuffer.rs` (4 literals at `:2387, 3434, 8420, 9905`
  gain `cluster_cull_hier: None`), `crates/boyko_render/src/light.rs`
  (`hier_group_threads`/`hier_group_count`), `crates/boyko_app/src/runner.rs:1951` (the debug-only
  boot/live dims assert), `crates/boyko_app/tests/vb_p1d_cull_shade_bench.rs`
  (+ `BOYKO_VB_BENCH_RIG=kronecker|r3|infrustum`, default `kronecker` so the existing provenance is
  reproducible verbatim). **The framegraph is untouched** — same buffers, same passes, same accesses,
  same seeds `[P0-1]`.
* **The new rigs** (§1.4): `r3` = the plastic-constant 3-D Kronecker sequence
  `alpha = (1/p, 1/p^2, 1/p^3)`, `p = 1.220744084605760` (the root of `x^4 = x + 1`) — genuinely
  3-D equidistributed, unlike `g/g^2/g^3`; `infrustum` = stratified inside the view frustum
  (screen `(u,v)` x depth `d` in `[3,12]` mapped through the camera basis), so density *rises* with
  `N` instead of leaking out of frustum.
* **Gate:** (a) `vb_mesh_froxel` and `vb_mesh_tex_froxel` pins **byte-identical, no re-pin**;
  (b) the §2 numeric table met on the `kronecker` rig — note the `N_ps=64` row is now
  **`<= 80 023`** (measured x 1.10), matching §10 rather than contradicting it;
  (c) the full sweep published for all three rigs into `light_policy.rs`'s provenance comment **as
  additional data — `CLUSTER_LO`/`CLUSTER_HI` are NOT changed** (§1.4.3);
  (d) **the §7.0 discrimination run**: a `dim_z` sweep at fixed `N`, recorded in this document,
  deciding between Reading A and Reading B;
  (e) **the §5 finiteness substitution's cost measured** — `froxel_cull_ns` at `N` in {8, 512} with
  and without the phase-1 predicate, three runs each, recorded here. §5 states this as a model number
  (about 0.4–0.9 us); this is where it stops being a model.
* **RED if:** a froxel pin moves (implying the set or the order differs, or a cap saturated — H3's
  precondition should have caught it, so a moved pin here means the pin's scene saturates and must be
  diagnosed by reproducing its light table under H3's cull-only driver and reading `alloc_total`
  there); any §2 threshold missed; or §7's microsecond column proves to have been the wrong reading
  and the re-derived prediction misses §2.

### 8.12 H5 — (conditional on H4) migrate Deferred + ForwardPlus to the hierarchical arm

* **Files:** `present/passes/gbuffer.rs:1583`, `present/passes/forward.rs:359` (the `match` + the
  `// SAFETY:` clause, which is byte-for-byte the same text as `vb.rs`'s and must get the same
  rewrite), `gpu_scene/mod.rs`. Retire the flat arm to test-only status (it remains the equality
  oracle's reference forever).
* **Gate:** every Deferred/ForwardPlus golden byte-identical; `l1_known_light_lands...` and
  `l1_clustered_resolve_matches_the_brute_force_image` green; `forwardplus_mesh` green.
* **RED if:** any of the above moves. **Precondition:** H4 shows a win on *both* the `kronecker` and
  `infrustum` rigs; a win only on the out-of-frustum rig does not justify migrating shipped paths.

---

## 9. Validation plan (consolidated)

**The last column is the point of this table.** Rev 3's version answered "can it actually fail?" with
"yes — mutations (v), (vi)" for a row whose mutations were both mis-specified, and "n/a" for rows that
needed a detector. Rev 4 answers it with a **named mutation whose effect was simulated or executed**,
or it says **NO** and explains what carries the property instead.

| Requirement | Mechanism | Where | Can it actually fail? |
|---|---|---|---|
| `[P0-1]` framegraph seeding | **No new buffer and no new framegraph access is introduced** — the coarse mask is groupshared, and H0's `LightIndexAlloc` readback was **removed** from the present path (§P1-E). `alloc_total` comes from H3's host-visible cull-only driver and H1's CPU oracle (§6). The existing trio keeps the `add_buffer_seeded` seeds landed at `5e07936` (`graph_bridge.rs:3179-3190`; the `light_cull` pass `:3212-3242`). Any future rung wanting a *live* counter must follow §11's recipe — and must **not** construct a hybrid `ResSync` seed, because `framegraph/sync.rs:288-296` takes the flush branch first and silently discards the visible-stage WAR half. | design | **NO — and that is the claim.** Nothing new is declared, so there is nothing to fail. `framegraph_gbuffer_equiv` still covers the trio. Stated as a *design property*, not as a gate |
| `[P0-2]` tests that can fail | H3's oracle drives the **hier** pipeline explicitly on a **PERSPECTIVE** camera and asserts the hier handle != base handle (assertion 9); twelve named mutations are executed under §8.10's protocol | `tests/cluster_cull_hier_equiv.rs` | **yes** — §8.3's table, every write-set row simulated (M10) |
| `[P0-3]` set-level oracle | Per-froxel index **sequence** equality between arms + against the host oracle, not an image hash. Image pins are a secondary no-regression gate only | H1 (CPU, exhaustive) + H3 (GPU) | **yes** — mutation (iii); a single dropped marginal light fails, where an 8-bit image hash would not |
| `[P0-4a]` totality | Groupshared mask re-initialised **every dispatch** by lanes 0..31 unconditionally — no cross-frame state. `ClusterGrid` totality by the `0xFFFFFFFF` pre-fill, **stated as at-least-once** | D1, H3.5 | **yes** — mutation (vi) at E2/E3/E4 (6144 / 384 / 12288 unwritten cells, simulated) |
| **`[P0-4a']` exactly-once (NEW)** | Not implied by totality. Derived from H3.5 + H3.6 + H1.2's `V == capacity` (§8.2 B1), and separately measured by the **permutation probe** H3.7 | §8.2(B), H1.2, H3.5–7 | **yes** — any duplicate in-range write loses an `InterlockedAdd` offset, breaking the permutation |
| **`[P0-4a''] `out-of-range WRITE (NEW — this was P0-1)** | **Guard tail**: `ClusterGrid` allocated at `capacity + G`, `G = (256·gps − dim_x·dim_y)·dim_z`, all pre-filled, tail asserted intact. `G` is *exactly* the invalid-lane image, so the detector is tight, not heuristic | §8.2(A), H3.6 | **yes — mutation (i): 2 688 tail cells cleared, simulated (M10).** Under Rev 3 this same mutation left **every** assertion green. **Requires `G > 0`** — protocol §8.10 |
| `[P0-4b]` range clamp (groupshared WRITE) | **One clamp `ps_n`** bounds the coarse groupshared write; `j < ps_n <= HIER_MASK_BITS` in the same basic block, no device value in the derivation (D7) | shader (D7) | **NO — the bound is STRUCTURAL, not mutation-detectable (Rev 5 P1 fix).** Rev 4 answered "yes — mutation (iv)'s producer half makes `(j>>5)` exceed 31". **Arithmetically impossible:** (iv) loops `j < HIER_MASK_BITS`, and `HIER_MASK_BITS = HIER_MASK_WORDS * 32 = 32 * 32 = 1024`, so `j>>5 <= 31` — in range for `gs_mask[32]`. This document **proves that itself** in D7 ("`j < ps_n <= ps_room <= HIER_MASK_BITS` implies `(j>>5) <= 31`") and correctly routes (iv) to **detector (C)** in two other places, including `[P0-4b']` immediately below. What carries the property instead: `ps_n <= ps_room <= HIER_MASK_BITS == HIER_MASK_WORDS*32 == 1024`, with D6's `const _: () = assert!(MAX_LIGHTS == HIER_MASK_WORDS * 32)` **equality** pin (`MAX_LIGHTS = 1024`, `crates/boyko_render/src/light.rs:51`) and H2(f)'s mechanical `#error` compile-failure test. *(Rev 6 P2-3: the chain printed `== MAX_LIGHTS` inline on a **groupshared-WRITE** bound row. `MAX_LIGHTS` carries the device-side **READ** bound — the light table's row capacity — and appears here only because D6 pins the two numbers equal. Trimmed so a later reader does not take the device quantity to be load-bearing for a groupshared index.)* A **NO** here follows the precedent of the four other NO rows in this column: the honest answer to "can it fail?" is sometimes "no, and here is what makes it unreachable" |
| **`[P0-4b']` out-of-range light READ** | **Poison tail**: the light table is allocated with rows beyond `light_count`, filled with an always-accepted light; H3.8 asserts no emitted index reaches them. Converts UB into a detectable value | §8.2(C), D7, H3.8 | **yes** — mutation (iv). *(Rev 3's mutation here could not reach an out-of-range read at all)* |
| **`[P0-A]` same-expression premise** | **Discharged, not disclaimed:** D10 makes `sq_dist_point_aabb` a written-out `precise` sum of correctly-rounded, `NoContraction`-decorated ops, so both call sites evaluate one function `F` (§5 Step 0), with a per-node audit table covering **every** node the monotonicity chain traverses — the two `OpFSub` included (Rev 4's placement change). H2(e) is a **tripwire** — explicitly *not* the proof, because DXC emits zero `Fma` | §5 + D10 + H1.6 + H2(e) | **yes** — e1/e2 fire if `dot()` returns or `precise` is dropped; **e2's exactness** also fires on an argument *expression* (M6); e6's producer assertion covers what an id-normalised window cannot; H1.6 catches the ULP fallout |
| **`[P0-B]` total bound** | `valid = (s < bdx·bdy) && (slice < bdz) && (fi < pc.cluster_capacity)`, with `cluster_capacity` **pushed** from the same `cluster_count()` binding that sizes the buffer (D11, Rev 4). Dispatch size, allocation and write bound are three evaluations of one u32 — now *literally*, not modulo an 8-bit repack governed by a `debug_assert!` | D3/D11 + H1.7 + H3.5/6/10 | **yes, but only as a PAIR** — mutation (v) re-specified (138 OOB + 138 gaps, simulated). **Deleting `slice < bdz` alone, or `fi < capacity` alone, is INERT** (simulated) — §D3 says so, and this row no longer claims otherwise. **Rev 5 P1: that pair now has an executable gate.** Under Rev 4's protocol the (v) run aborted at assertion 10 (which fires on exactly the boot-16x9x23/live-16x9x24 skew (v) *requires*, and the RED-if blanketed all ten assertions on every mutation run), so this discharge had **no run that could reach assertions 5/6**. §8.10 item 3 now scopes assertion 10 behind an `allow_skew` driver flag and requires the reviewer to record **assertion 5**'s (TOTALITY / detector A1) **138 in-range sentinel cells** and **assertion 6**'s (GUARD-TAIL / detector A2) **138 cleared tail cells** *individually*. **Rev 6 P1:** Rev 5 wrote these as "assertion 1 / assertion 2" — the detector labels A1/A2 misread as table rows — which routes the discharge through assertion 1 (`alloc_total < index_list_cap`, a precondition (v) cannot falsify) and leaves **assertion 6 unchecked**. Rev 6 also pre-registers the **CONTROL**: the BASE arm on this config clears **16** tail cells by itself (boot-sourced 3328 threads vs its live-dims guard, §8.10 item 3), so an assertion-6 firing 16 times means the mutation did nothing |
| `[P1]` FP margin | **Deleted, not bounded**: D2 makes enclosure a monotonicity theorem (§5) with no epsilon | §5 + D8 + H1 | **yes** — mutation (ii) (lane-0 fold) on the adversarial rig, whose target froxel must not be lane 0's |
| **`[P1-D]` non-finite AABBs** | **Absorbing-element substitution `±FLT_MAX`** (D8, §5 Case B; **Rev 5 corrects Rev 4's `±1e30`**, which inverted enclosure for a finite centre with `\|c\| > 1e30`): a non-finite `valid` lane forces the coarse box to the universe, degrading the group to *exactly* the flat walk. D4 clause (c) is **deleted** as a result — in the **AABB's** finiteness only; the surviving condition on the **light centre** is named as Premise F (§5.2) rather than left implicit | §5, D8, H3 (vii) | **yes, two-sided** — mutation (vii), **re-specified in Rev 5** (`fi == 168u`, all six components, mirrored in HIER + base + host mirror), must be GREEN with the substitution and RED without. Rev 4's one-arm `lane == 7` form could not go green in the GREEN arm and named different froxels in the two modules. *(Rev 3's identity-element mitigation was a no-op in the mixed case and inverted the all-NaN case; withdrawn)* |
| `[P1-3]` cap saturation | §1.3's measured table (pinned at **HP**) + the exact `alloc_total <= cap` detector asserted as a precondition of every equality run | §6, HP, H1.3, H3.1 | **yes** — set `index_list_cap = 1`; the equality test aborts loudly instead of comparing clamped results |
| wave/subgroup coherence | No wave intrinsics used, and **no barrier elided on an assumed wave width** — `subgroup_size_control: VK_FALSE` (`device.rs:2584`), `subgroupSize` queried nowhere. Phase 5's trip count is **group-uniform** (all lanes walk the same mask), so zero loop divergence; only the append predicate diverges, exactly as today | design (D1, D9) | **NO** — nothing is assumed, so there is nothing to falsify. Listed for completeness |
| dispatch shape | One boot u32 drives the dispatch size, the allocation and the in-shader write bound; a live-header disagreement cannot move `fi`. **H1.7** pins the derivation on the CPU over six grid configs incl. degenerate ones; **H3.10** asserts no-skew on device; a `debug_assert` at `runner.rs:1951` catches an owner edit | H1.7, H3.10, H4 | **yes** — mutations (v), (vi). **Caveat:** H1.7 is a *Rust re-implementation*, not a pin on the HLSL; only H3 sees a shader/mirror drift |
| barriers | **3 total** (B1, B2, B3), stated once in D1 and footed in §4. H2 gate **(e8)** asserts each lies on the entry function's top-level block chain | D1, D9, §4, H2(e8) | **yes, executed (M9)** — RED on an isolated early `return` and on a divergent barrier, GREEN on the correct shape. *(Rev 3's "one `OpReturn`" and "barrier in a merge block" are DELETED — measured non-discriminating, M7/M8)* |
| `#error` guards | The dis-gate compiles a scratch copy with `HIER_MASK_WORDS 64u` and asserts dxc **fails** | H2(f) | **yes, and now MECHANICAL** — delete either `#error` and the compile succeeds. *(Rev 3 ran this by hand once and §9 counted it as mechanical; corrected)* |
| occupancy / groupshared | **6 276 B/group, exact by construction** (six scalar `float[256]` + 32-word mask + summary; D9). `float3` is avoided because the emitted module carries **no `ArrayStride`** for `Workgroup` storage, so its stride is driver-chosen and not derivable from the artifact. 24 groups against Ampere's 100 KB/SM — not a limiter. `local[256]` is **unchanged** (§1.1) | design | **NO** at compile time — it is arithmetic on `#define`s. Measured at H4 |
| `unsafe` discipline | The rung adds no new Rust `unsafe`; the record-site changes are inside existing `unsafe` blocks whose `// SAFETY:` comments are **rewritten** (D11) — the old "`cull_groups` covers `cluster_count` froxels" is a coverage claim, the wrong obligation, and false for the HIER map. The replacement carries an explicit **SCOPE** sentence bounding it to this dispatch's writes | H4 (`vb.rs`), H5 (`gbuffer.rs`, `forward.rs`) | clippy `-D warnings` + review |
| **ClusterGrid READ bound (frame level)** | **NOT ADDRESSED HERE.** The four consumers index with the **live** header dims behind only a non-zero test (`vb_resolve.comp.hlsl:359`, `vb_shade.comp.hlsl:527`, `deferred_pbr.hlsl:1237`, `forward_opaque.fs.hlsl:333`); a live-dims-grow skew is an out-of-range read with `robustBufferAccess` off | pre-existing; **VB-P1k** (§11) | **NO gate exists, and Rev 4 says so.** VB-P1e neither creates nor closes it. Rev 3's "the HIER arm cannot fault" framing hid this row entirely |

---

## 10. Risks and the ABORT criterion

| Risk | Mitigation |
|---|---|
| The `0.2736 ns/pair` rate does not transfer to the new dispatch shape | **H1.5** bounds froxel-count scaling on the existing flat arm with no shader work, per-point rather than on an aggregate fit; H4 measures the real thing; the abort threshold is in measured ns |
| **H1's selectivity gate is mistaken for a wall-clock predictor** | H1 is explicitly a *necessary* condition on pair count (§7.0). H1.5 bounds thread-count scaling. Barrier cost, the 43.75 % idle fine phase, hot-group serialization and the `ClusterGrid` write pattern are settled only at H4, against §7.0's **pre-registered** prediction |
| The 13.9 us fixed cost is dispatch-intrinsic, not barrier-intrinsic, so low-`N` gains vanish | H0 measures it *first*; it changes only the low-`N` prediction, not the `N >= 128` win |
| Load imbalance (3 hot groups of 24, 85.2 % of the fine work) leaves the GPU idle | The hot group's work is irreducible; imbalance is a *symptom of having removed the other 97.5 %*. §7.0 pre-registers the `dim_z`-sweep discrimination; if Reading B holds, the follow-up is a second in-group level (§11), not a redesign |
| A barrier reached under non-uniform control flow, causing a device hang | D8 is a named code-review gate item with a six-row checklist (now including the group-uniformity of the coarse box); **H2(e8)** is a mechanical gate **verified to fire** on an isolated early `return`; H2(g) pins the source text; H3 runs on-device and a hang is unmissable |
| The equality oracle is run in a saturating configuration and silently compares clamped results | H3.1's `alloc_total` precondition fails the test loudly (§6) |
| Byte-identity claim over-reached | Explicitly scoped by D4's **four** clauses: non-saturating, no boot/live skew, one shared distance function, output buffers not access patterns. *(The finite-AABB clause is deleted — §5 Case B carries it by construction, and mutation (vii) tests that two-sided.)* |
| **The D10 edit perturbs the base arm's `sq_dist` by <= 1–2 ULP and flips an exactly-tangent light in an existing golden** | Detected by H1.6's re-run of `cluster_cull_spv_sync`, `lighting_l1_host_oracle`, the two L1 GPU oracles and the image goldens, under a **zero-move golden budget**: a moved pin stops the rung until the flip is *demonstrated* on the host, and only then may be re-pinned with that demonstration recorded. The flip requires a light within about 1e-7 relative of exact tangency — measure-zero in principle, not provably zero on a procedural rig |
| **The D10 edit's ALU cost is unmeasured** (2 extra ops per pair test on a path whose wall clock tracks pair count) | H1.6(e) records `froxel_cull_ns` at `N_ps` in {128, 512}, **three runs before and after**, gate after-median <= before-median x 1.05; on a regression, fall back to the named alternative — slack `r*r*(1.0 + 0x1p-20)` on the coarse comparison only, base arm untouched |
| **§5's finiteness substitution's cost is unmeasured** | H4 gate (e) measures it with and without the phase-1 predicate. §5 labels its 0.4–0.9 us figure a model |
| **An owner edits `ClusterConfig` post-boot in a release build** | For **this dispatch's writes**, D11 makes it harmless: `fi` cannot move, and the write bound is the pushed boot capacity. `debug_assert` at `runner.rs:1951` catches it in debug; H3.10 catches it in tests. **At frame level it is NOT harmless** — the four `ClusterGrid` readers use live dims (§D11 scope correction), which is pre-existing and tracked as **VB-P1k, now filed as a safety follow-up** |
| **A gate is written that cannot fail** | The standing risk this revision exists to address. §8's rule: every assertion names a mutation and that mutation was simulated or executed. §9's last column reports the result, including three explicit **NO**s. Five Rev 3 assertions were deleted rather than reworded (§8.3) |

**ABORT (revert exactly as the two-pass attempt was reverted) if any of:**

1. **H1**: pair-count selectivity below 4x on both the `kronecker` and `infrustum` rigs at `N >= 128`.
   *(Costs zero GPU time and zero shader code — this is the cheap kill switch, and it is a
   necessary-condition test only.)*
2. **H1.5**: **a latency floor is found — defined as any of the four measured grid points implying a
   rate outside `[0.2052, 0.3420] ns/pair` (±25 % of 0.2736)** — **and** the re-derivation specified
   in §8.7 (refit over the four grids, take the largest per-point rate `b_hi`, evaluate
   **`a + b_hi x 45 840`**) exceeds §2's 250 000 ns threshold. *(Still no shader written.)* *(Rev 3's
   clause 2 defined neither "a latency floor is found" nor the re-derivation. **Rev 6 P1:** the literal
   was `32 300`, Rev 4's MODEL; H1 measured `45 840`. The clause executes the measured count — with the
   stale literal it under-states the re-derived cost by 1.42x, i.e. it fails to abort in exactly the
   band where aborting is the point.)*
3. **H3**: any per-froxel index sequence differs between arms in a non-saturating, non-skewed
   configuration; **or** any of the memory-safety assertions 5–8 fails on the unmutated shader.
4. **H4**: `froxel_cull_ns` at `N_ps=512` above 250 000 (below a 2x win), **or** any of
   `N_ps` in {8, 32, 64} regresses `froxel_total_ns` by more than **10 %**, **or** any froxel golden
   pin moves.

**The 10 % tolerance is the single reading, and §2 now matches it.** Rev 3's §2 demanded
`froxel_cull_ns @ N_ps=64 <= 72 748` — zero tolerance, one sample — while this section allowed 10 %
and its own closing paragraph said a 5 % loss at `N=8` is not an abort; H4's "any §2 threshold
missed" made the contradiction operative. §2's regression rows are now `measured x 1.10`.

A partial result — e.g. a large win at `N >= 128` and a 5 % loss at `N = 8` — is **not** an abort: the
`Auto` policy band already disarms clustering below `CLUSTER_LO`, so the low-`N` arm is not the
shipping configuration. It must, however, be reported in the provenance table rather than smoothed
over.

---

## 11. Tracked follow-ups (explicitly out of scope for VB-P1e)

* **VB-P1f — re-tune `CLUSTER_LO`/`CLUSTER_HI`.** Owner-gated. Requires H4's two-rig sweep. Until it
  lands, `Auto`-mode scenes with 64 < `N` < 128 keep the flat path and see no VB-P1e benefit
  (§1.4.3). Must also fix the false "mutually irrational" doc-comment
  (`vb_p1d_cull_shade_bench.rs:114-123`) — §1.4's proof belongs next to the code it refutes.
* **VB-P1g — attack the fixed cost** (the only route to a single-digit break-even, §7.1): delete the
  `cmd_fill_buffer` + `TRANSFER->COMPUTE` barrier by resetting `LightIndexAlloc` from within the
  previous frame's cull (or a per-FIF alloc ring), and/or fold the cull into the shade dispatch's
  prologue. Gated on H0's attribution.
* **VB-P1h — a second in-group level** (per-16-lane sub-block masks), only if H4 shows the fine phase
  or Reading B dominating. Output-neutral by D2's corollary, so it is a pure perf experiment.
* **VB-P1i — wave-intrinsic reduction** (`WaveActiveMin`/`WaveActiveBallot`). Output-neutral by D2's
  corollary. **Concrete precondition, verified:** the RHI sets `subgroup_size_control: VK_FALSE`
  (`device.rs:2584`) and queries `subgroupSize` nowhere, so VB-P1i must first add the device-feature
  query.
* **VB-P1j — give the BASE arm the same capacity bound. CLOSED.** Its total bound was
  `min(64·ceil(boot_cc/64), live_cc)`, which exceeds `boot_cc` when `boot_cc % 64 != 0` **and** the
  live dims grow: measured **16 cells (128 B) past the end of `ClusterGrid`** at boot 16x9x23 / live
  16x9x24. Two owner actions required (vs one for D3-as-written), bounded by 63 cells (vs unbounded).
  It needed its own base `.spv` re-pin, which is why it was not folded into H1.6. **Note the shared
  root with VB-P1k:** both were the base path trusting live dims against a boot-sized buffer, one on
  the write side and one on the read side — and both are closed in the SAME commit, by the same
  mechanism.
  **Closure (not by the push route this bullet assumed).** The base arm's push range is
  *unchanged* (still 16 B / 4 words); instead `cluster_cull.hlsl`'s `#else` prologue clamps
  `cluster_count` by `ClusterGrid.GetDimensions()` — SPIR-V `OpArrayLength` on the bound
  descriptor, i.e. the ALLOCATION itself rather than a host-side mirror of it. This is strictly
  stronger than the pushed boot capacity D11 gives the HIER arm (a push word can be wrong; the
  array length cannot), and it costs no host change, no pipeline-layout change and no dispatch-site
  change — two instructions (`OpArrayLength` + `OpUMin`) once per thread, outside the light loop.
  The `min` is arithmetically inert whenever boot dims == live dims, so cull output — and every
  golden pinned on it — is unchanged. Gates: `tests/cluster_grid_write_bound.rs` (skew sweep +
  the executed pre-fix mutation, which reproduces the 16-cell overrun) and
  `cluster_cull_spv_sync.rs`'s `op_array_length` census on the committed module.
* **VB-P1k — bound the `ClusterGrid` READ against the boot capacity. RE-FILED IN REV 4 AS A SAFETY
  FOLLOW-UP, not an "Owner/VALUES call".** Rev 3 filed it as a behaviour preference ("decide whether a
  detected boot/live skew should disarm the cull in release") on the strength of the claim that "the
  HIER arm cannot fault … only mis-shape the grid". That claim is true of the cull's **writes** and
  false at frame level: `vb_resolve.comp.hlsl:359`, `vb_shade.comp.hlsl:527`,
  `deferred_pbr.hlsl:1237` and `forward_opaque.fs.hlsl:333` each compute
  `cluster_linear_index(tile.x, tile.y, zsl, cp.dim_x, cp.dim_z)` from the **live** header and then
  read `ClusterGrid[cluster]` with no capacity bound, guarded only by
  `cp.dim_x * cp.dim_y * cp.dim_z != 0` (the VB-P1b-0 C1 defence, documented at
  `vb_resolve.comp.hlsl:343-348`). If the live dims grow past the boot dims, that read leaves the
  boot-sized allocation with `robustBufferAccess` off. **Pre-existing** — the repo already named the
  class at `plugins.rs:355-361` and `light_system.rs:410` — and VB-P1e changes nothing about it, but
  it is a memory-safety item, not a preference. Scope when it is taken: push the boot capacity to the
  consumers (or clamp the computed `cluster` against it) rather than only disarming the cull; the
  disarm question is then genuinely a VALUES call on top of a closed hole.
  **CLOSED, by neither of the two routes this bullet named.** Pushing the boot capacity to four
  shader families (14 committed variants, four pipeline layouts, three host push structs) was
  rejected as disproportionate; clamping the computed `cluster` was rejected because it keeps the
  walk armed against a grid that does not exist, so the pixel shades from the WRONG froxel,
  silently. What shipped: each of the four consumers reads `ClusterGrid.GetDimensions()` (SPIR-V
  `OpArrayLength` — the allocation itself, no new interface anywhere) and extends `use_clusters` to
  the three-term `clusters_enabled != 0 && cluster_count != 0 && cluster_count <= grid_capacity`. A
  skewed frame falls back to the flat light scan, which is both in-bounds and the CORRECT lighting
  for that frame, so the disarm-vs-clamp VALUES call is answered by the cheaper and more correct
  option rather than deferred. The same commit gives `deferred_pbr.hlsl` and
  `forward_opaque.fs.hlsl` the older non-zero-dims term (VB-P1b-0 C1) that they had never carried —
  the sharper of the two holes, since a zero-`dim_z` header (what `sync_cluster_light_gate`
  publishes on every non-VB-froxel boot, i.e. on exactly those two shaders' own production paths)
  makes `cluster_z_slice` return `0xFFFFFFFF`. Gate: `tests/cluster_grid_read_bound.rs`, which also
  closes a gap found while enumerating the family — the `deferred_pbr` (6 rows) and
  `forward_opaque.fs` (2 rows) families had **no `*_spv_sync` byte-identity test at all**.
* **A `safe_normalize` in `ray_gen.hlsli`** — the true root fix for the device NaN of §5's first
  source. Deliberately **not** smuggled into VB-P1e: `ray_gen.hlsli` is included by the marcher and
  the deferred PBR resolve, so the change re-DXCs and moves every dependent committed `.spv` and its
  byte pin. **Weight increased in Rev 4:** §5's on-device handling is now an *absorbing-element
  degradation to the flat walk*, i.e. it makes the output correct but silently costs the whole group
  its hierarchy. Closing the source is therefore a performance fix as well as a hygiene one.
  **NOT TAKEN — Rev 4's weight increase is WITHDRAWN, refuted by measurement; the source is closed
  on the host instead (see the `ViewUniform::from_camera` item above).** Three findings, in order
  of how much they change the item:
  1. **The absorbing-element chain does not exist.** A NaN `rd` never reaches the `finite`
     classifier: it is consumed one step earlier by `expand_aabb`'s `min`/`max`, which are
     GLSL.std.450 `NMin`/`NMax` — the very ops §5.1(2) already reasons about, and the disassembly of
     the committed module shows the AABB accumulating through a chain of exactly 8 `NMin` / 8 `NMax`
     seeded by the `(+1e30, -1e30)` constants, with **zero** `FMin`/`FMax` anywhere. Those DISCARD
     the NaN operand, so a poisoned lane keeps its "nothing yet" initializer, `all(abs(v) <= 1.0e30)`
     is **true** for it, and the lane stores its (inverted) box on the NORMAL row. The absorbing
     element is never selected, no group loses its hierarchy, and there is no performance cliff from
     this source. Reproduced host-side by
     `compute::tests::…::nan_slice_view_z_is_silently_swallowed_by_the_min_max_chain` (Rust's
     `f32::min`/`max` carry the same IEEE `minNum`/`maxNum` NaN-dropping semantics, which is also
     why `golden_froxel_aabb` mirrors the shader here without a special case).
     *What actually happens* is a correctness failure shared EQUALLY by both arms — the inverted
     sentinel box gives `sd = 1e60·3 → +inf`, so every point/spot light is rejected in every froxel
     and clustered punctual lighting goes dark. D4's byte-identity is untouched.
  2. **The blast radius is 12 sources / 27 committed `.spv`, not two.** `ray_gen.hlsli` is
     `#include`d by `cluster_cull.hlsl` (**directly** — `generate_ray` is called in BOTH the flat
     and the hierarchical froxel-AABB build), `deferred_pbr.hlsl` (6 rows), `sdf_forward_march`
     (4), `sdf_gbuffer_composite`, `forward_sky.fs`, `sdf_ssao_{low,medium,high}` (×2 with
     `VB_THIN`), `shadow_atrous`, `ssao_atrous` (3), `taa_resolve`, `vb_shadow_vis` and
     `viewt_from_depth_rz`. All 27 were re-DXC'd byte-identical under their frozen recipes as the
     baseline for this rung, so the table is validated — and it prices the item honestly.
  3. **It is a coordinated host+shader change, not a shader one-liner.** `composite_ray`
     (`compute.rs`) deliberately mirrors HLSL `normalize` with raw `sqrt`+divide and **no** zero
     guard, precisely "so this host reference predicts the GPU bit-for-bit on valid cameras. A
     degenerate (zero) `dir` yields a non-finite ray on BOTH host and shader." Guarding only the
     shader breaks that contract by construction. Whoever takes this must decide the host oracle's
     behaviour in the same commit.
  **When it would still be worth taking:** as defence in depth against the *other*, still-unclosed
  producers of a degenerate `dir` — a non-finite `tan(fovY/2)` or `aspect` reaching the camera
  block, neither of which is validated anywhere today. That is its own rung, with the 27-artifact
  re-pin, a device-oracle gate, and the `composite_ray` decision above; it is NOT this item.
* **Host-side finiteness validation of `LightElem::pos` — the closure for §5.2's Premise F (new in
  Rev 5).** §5 Case B's enclosure argument needs the light *centre* to be finite; with a `±inf`
  component the coarse level rejects while a non-finite-AABB lane's own fine test accepts, so D4's
  byte-identity is lost for that light (memory safety is untouched — `ps_n` bounds every read,
  `fi < capacity` every write, and neither involves `c`). Undischarged today and **pre-existing in
  kind**: a NaN centre already makes the BASE arm's flat test accept every froxel. Scope when taken:
  reject or clamp non-finite positions in `fold_light_table_slotted` beside the existing saturating
  `written == MAX_LIGHTS` gate, where it costs one compare on a host path that already touches every
  row. Until then, **H3's rigs must not contain a non-finite light centre** (§8.10).
  **CLOSED — by the reject-and-skip route, and the consequence it closes is bigger than this
  bullet stated.** `punctual_row_is_cullable` (`light_system.rs`) gates every point/spot row on
  "centre finite AND radius not NaN" and a failing row is DROPPED — not written, not counted, no
  hole left in the table — with a `#[cold] #[inline(never)]` one-shot log, i.e. verbatim the
  policy the SAME function already applies to its `MAX_LIGHTS` overflow. Cost: four ordered
  compares per punctual row, none on directional/sky (they carry no cull centre), the branch
  resolving the same way on every row of every well-formed frame.
  *Clamp was rejected* — there is no meaningful clamp of a NaN centre, and substituting the
  origin INVENTS lighting the author never wrote (a missing light is debuggable; an invented one
  is not). *Panic was rejected* — this is live, per-frame, gameplay-authored ECS data, not a
  build-time configuration invariant.
  **What a NaN centre actually did, traced.** `sq_dist_point_aabb`'s `max`es lower to
  GLSL.std.450 `NMax` (measured on the committed module: 18 `NMax` / 8 `NMin`, **zero**
  `FMax`/`FMin`), and `NMax(·, 0.0)` returns `0.0` when its other operand is NaN — so `d`
  collapses to `0` on the poisoned axes, `sd == 0` with all three poisoned, and the row is
  appended to **every froxel**. At the default 16×9×24 grid that is 3456 `LightIndexList`
  entries — **21 % of `INDEX_LIST_CAP`** — for ONE bad light, competing for the O2 caps, so it
  does not merely mis-light: it **evicts correct lights**. A `±inf` centre instead gives
  `sd = +inf` and is rejected everywhere; that is the shape Premise F's Case B names.
  Gates (`light_system.rs` tests, all four executed RED against a defanged predicate):
  `a_nan_positioned_point_is_dropped_and_the_finite_rows_close_up`,
  `infinite_positions_and_a_nan_range_are_dropped_on_both_punctual_kinds`,
  `a_dropped_row_does_not_spend_the_max_lights_budget`, `the_validity_gate_is_not_vacuous`, plus
  the golden-neutrality sweep `the_validity_gate_is_inert_on_every_well_formed_row` (which pins
  that an INFINITE radius stays accepted — a totally-ordered comparand both levels agree on, so
  a coherent authoring choice unlike a NaN). **§8.10's constraint on H3's rigs is now
  structural** rather than a discipline note: a non-finite centre can no longer reach the table.
* **A non-degenerate basis fallback in `ViewUniform::from_camera`** (mirroring the identity fallback
  it already gives `view` at `camera.rs:331-336`: fall back to the canonical right/up/forward when
  any of the three normalizes to ZERO), plus **release-visible `z_near > 0 && z_far > z_near`
  validation in `ClusterCullPush::new`** (`compute.rs:3473-3483`) — today the only check is a
  `debug_assert!` in a different function (`ClusterConfig::z_scale`, `light.rs:738-743`). Defence in
  depth for §5's two NaN sources; neither replaces the on-device handling, which closes the *class*
  regardless of source.
  **BOTH CLOSED. Zero `.spv` perturbed** (all 27 `ray_gen.hlsli` dependents re-DXC'd byte-identical
  before and after).
  * **Basis fallback: SHIPPED, all-or-nothing.** `ViewUniform::basis_axis_is_usable` is
    `axis.length_squared() > 0.0`, which is false for `Vec3::ZERO` **and** for the all-NaN vector
    `Vec3::normalize` returns on a non-finite input (every ordered compare against NaN is false),
    i.e. one compare classifies all three shapes `normalize` can produce. The substitution is
    **all-or-nothing, not per-axis** — this bullet's wording, and it is load-bearing: a mixed
    triple is not guaranteed to be a basis (a real `right` of `(0,0,-1)` beside the canonical
    forward is rank-deficient, so `dir` can still vanish). Golden-neutral by construction: for any
    camera with three usable axes the branch is not taken and the published bytes are bit-identical
    (pinned by `a_well_formed_camera_basis_is_bit_identical_to_the_raw_normalize`, which compares
    `to_bits`, not epsilon). Gates in `crates/boyko_scene/tests/gates_camera_degenerate_basis.rs`;
    the detector `the_degenerate_camera_no_longer_feeds_ray_gen_a_nan` runs a bit-faithful mirror of
    `ray_gen.hlsli`'s PERSPECTIVE branch (raw `sqrt`+divide, **no** guard — the same spelling
    `composite_ray` uses) over the published basis and asserts `rd` is finite; with the fallback
    removed it was executed and reports `[NaN, NaN, NaN]` at the CENTER pixel, for both the fully
    singular and the rank-2 (`diag(1,1,0)`) transform. `the_ray_gen_mirror_really_does_nan_on_a_zeroed_basis`
    keeps that detector from being vacuous.
  * **Push validation: SHIPPED, but NOT at the site this bullet named.** Putting a rejecting check
    inside `ClusterCullPush::new` as written would fire at **boot on every process**: `new(0.0, 0.0,
    0, 0)` was constructed in production as the pre-arm placeholder (`gpu_scene/mod.rs:3712`). The
    check IS in `new` — `assert!(z_near > 0.0 && z_far > z_near)`, release-visible, `const fn`-clean,
    and mirrored in `ClusterCullHierPush::new` which runs the identical `slice_view_z` — and the
    placeholder is spelled honestly as a new `ClusterCullPush::UNARMED` associated const that
    bypasses it, because an all-zero push is not a cull configuration but the "no cull exists"
    sentinel. This is a boot-path constructor (once per `build_froxel_light_cull`; the per-frame
    record site re-pushes the stored bytes), so Principle 1 is not engaged.
    **Why the check cannot be left to the device, pinned as a runtime fact rather than prose:**
    `nan_slice_view_z_is_silently_swallowed_by_the_min_max_chain` (`compute/tests.rs`) shows that
    at `z_near == 0`, `slice_view_z(0)` is finite (`0 * pow(inf, 0)`) while every `k > 0` is NaN
    (`0 * inf`) — and that the NaN is then **discarded** by the AABB's `NMin`/`NMax` chain, leaving
    the accumulator at its finite `(+1e30, -1e30)` initializer. The far corners vanish, every slice
    collapses onto its near plane, and **no device-side finiteness predicate can see the fault**.
    Six `#[should_panic]` gates (zero / negative / inverted / collapsed / NaN bound / the HIER
    push) were executed RED against a defanged assert.
* **A live `alloc_total` HUD counter** so saturation is visible in ordinary runs. Rev 2 listed this
  as a one-liner; it is not, and the recipe is written out here so it is not re-attempted cheaply:
  * a **new** graph pass `cull_alloc_readback`, declared immediately after `light_cull`, gated on the
    same 4-buffers-`Some` predicate **and** a new `Option<&BoundBuffer>` bench-staging field on
    `GBufferScene` (the `vb_gpu_timing` gating precedent verbatim, `gpu_timing.rs:232-240`: `None`
    means zero declared accesses, hence a byte-identical command stream);
  * its single access `g.buffer_access(light_index_alloc, VK_PIPELINE_STAGE_TRANSFER_BIT,
    VK_ACCESS_TRANSFER_READ_BIT)`, letting the graph derive the
    `src=(COMPUTE, SHADER_WRITE) -> dst=(TRANSFER, TRANSFER_READ)` availability barrier. **Do not
    hand-roll it** — a hand-rolled barrier leaves `ResSync` describing a state the command stream no
    longer has, and the *next* derived access is then wrong. (`present_blit.rs:335-400` is the right
    precedent for the `Option`-gating half only; the swapchain image is not a graph resource,
    `light_index_alloc` is.);
  * usage is already legal: `create_buffer` unconditionally ORs `TRANSFER_SRC | TRANSFER_DST` onto
    every `DeviceLocal` buffer (`rhi_impl/device.rs:52-58`), which is the *only* reason the copy is
    legal, since `light_index_alloc` is created with `BufferUsage::STORAGE` alone
    (`gpu_scene/mod.rs:4332-4338`);
  * the staging destination is a 4-byte `HostVisibleCoherent` buffer created **once at boot**,
    per-frame-in-flight (`[BoundBuffer; FRAMES_IN_FLIGHT]`) — never per frame (Principle 5);
  * **seed hazard, stated:** leave the declared seed as
    `ResSync::seeded_writer(COMPUTE_SHADER, SHADER_WRITE)` and pin, in a comment at the declaration
    site, why the stale seed is not a WAR race here — the bench loop calls `ctx.wait_idle()` on every
    armed frame before reading (`runner.rs:2069`), so no two frames overlap while the readback pass
    exists — with an assert next to the arm site that the `wait_idle` is present. Switching the seed
    to `seeded_readers(TRANSFER, TRANSFER_READ)` is *sound without* the `wait_idle` but changes the
    fill's in-barrier from a COMPUTE-to-TRANSFER memory dependency to a TRANSFER-to-TRANSFER
    execution-only one, i.e. it removes a compute drain from inside the `CullReset` bracket and
    **biases H0's fixed-cost number low**; if used, the alloc-readback mode and the fixed-cost
    attribution run must be declared **mutually exclusive**. A **hybrid** seed is forbidden outright:
    `transition()` (`framegraph/sync.rs:288-296`) takes the flush branch first and drops the
    visible-stage WAR half;
  * record the copy **after** the `CullDispatch` timestamp closes so neither sub-bracket is
    perturbed.
* **A specialization constant instead of D11's two push words** — output-neutral, re-openable;
  rejected here on economy and precedent (D11).

---

## 12. Approval preconditions

Rev 3 used this section to state, in prose, that a provenance commit "must precede approval". Rev 4
turns that into **rung HP** (§8.4) — a named test file, a named test function, 24 literal expected
values, a gate and a RED-if mutation. This section is now just the checklist:

1. **HP is committed.** This is the plan's own stated approval blocker, and Rev 5 keeps it. **It is
   being implemented in parallel, right now, as a real `#[test]`.** Rev 5 replaces Rev 4's prose with
   an acceptance criterion precise enough that "is it done?" is a yes/no question:
   * **File:** `crates/boyko_rhi_vulkan/tests/lighting_l1_host_oracle.rs` — an added `#[test]`, no new
     file, no new fixture, no GPU.
   * **Function:** `cluster_cull_occupancy_profile_matches_the_published_table`.
   * **What it asserts:** it drives `golden_cluster_cull` (`goldens.rs:3510`) with the VB-P1d camera
     (eye `(0, 1.1, 7.8)` → `(0, 0.55, 0)`, `fov_y` 52°, aspect 1.0, 512x512) and the bench rig
     reproduced from `vb_p1d_cull_shade_bench.rs:124` / `:142`, against `ClusterConfig::default()`, and
     asserts for `N_ps ∈ {8, 14, 32, 64, 128, 256, 512, 1024}` **all 24 of §1.3's literals** —
     `total_indices` / `non_empty_froxels` / `max_per_froxel` = `789/514/3`, `1239/543/5`,
     `1916/557/10`, `2063/364/15`, `1654/143/24`, `2072/115/40`, `2597/85/64`, `2709/55/109` — plus
     `total_indices < INDEX_LIST_CAP` and `max_per_froxel < MAX_LIGHTS_PER_CLUSTER` (which is
     simultaneously §6's saturation discharge).
   * **What makes it RED:** any one of the 24 literals differing. **The mutation that must be run once
     during review:** change `fov_y` from 52° to 53° — the froxel-to-pixel tiling moves and
     `non_empty_froxels` shifts. A test that stays green under that mutation is not pinning the table.
   * **On landing:** §1.3 and the appendix are re-anchored on
     `lighting_l1_host_oracle::cluster_cull_occupancy_profile_matches_the_published_table` instead of
     on the session-ephemeral `scratchpad/cap_probe.rs.txt`, which
     `git ls-files | grep -i 'cap_probe\|occupancy_probe'` still shows does not exist.
2. **Rev 5 has been reviewed.** **No reviewer has seen Rev 5.** The status block claims no approval,
   and Rev 5 fixed six P1s in six places where a reviewer had previously found the *opposite* claim
   plausible — which is precisely the reason a fresh pass is required rather than assumed.
3. Nothing else. Every other Rev 3/Rev 4 precondition either landed or is tracked in §11.

---

## Appendix — source anchors

Every line number below was opened and checked while writing Rev 4; the three that Rev 3 got wrong are
marked **[fixed]**.

| What | Where |
|---|---|
| The cull shader (hand-authored, no eDSL sentinels) | `crates/boyko_rhi_vulkan/shaders/cluster_cull.hlsl` — dispatch shape `:107`, early return `:112-114`, AABB build `:126-153`, `local[256]` `:159`, flat light loop `:161-175`, per-froxel cap `:170`, claim+write `:180-194`, `index_list_cap` clamp `:184-191`, `sq_dist_point_aabb` `:102-105` (D10 rewrites this), `view_z_to_t` `:85-91` (the 8 residual `OpDot`s), `slice_view_z` `:77-79` |
| Frozen dxc recipe (no `-D`, no `-O`) | `crates/boyko_rhi_vulkan/tests/cluster_cull_spv_sync.rs:45-53`; the shader's own header `cluster_cull.hlsl:25-27`; text-pin idiom `:20-22` |
| Shared ray-gen (and §5's NaN source 1) | `crates/boyko_rhi_vulkan/shaders/ray_gen.hlsli:44-75`, `normalize(dir)` `:63-67`; host side `crates/boyko_scene/src/camera.rs:325-327`, `:331-336`; `crates/boyko_math/src/vec.rs:226-233` |
| Cluster linearization / params (one source of truth) | `crates/boyko_rhi_vulkan/shaders/light_table.hlsli:313-323, 329-331`; `LightHeader` `:223`, `load_light_header` `:243-248`, `load_light` `:255-265` (no bound check), `light_kind` `:271-273` |
| Host constants + config | `crates/boyko_render/src/light.rs:42-61` — `CLUSTER_DIM_X/Y/Z` `:43,45,47`, `CLUSTER_COUNT` `:49`, `MAX_LIGHTS` `:51`, `MAX_LIGHTS_PER_CLUSTER` `:53`, **`INDEX_LIST_CAP` `:61` [fixed — Rev 3 said `:57`]**; `ClusterConfig` `:724-770` (`cluster_count()` `:728`, `z_scale` debug-assert `:738-743`, `packed_dims` `:763-769`) |
| The live-header gate (D11's divergence source) | `crates/boyko_render/src/light.rs:783-786, 792-793, 830-851` (the unarmed path writing `packed_dims = 0` at `:836-840`); the prior UB-class ruling `crates/boyko_app/src/plugins.rs:352-364` (the "out-of-bounds `ClusterGrid` index — real GPU UB" wording at `:359-360`) and `light_system.rs:410`; the Resource seed `plugins.rs:195` |
| **The four `ClusterGrid` consumers (VB-P1k's exposure)** | `crates/boyko_rhi_vulkan/shaders/vb_resolve.comp.hlsl:359` (the non-zero-dims defence at `:343-352`), `vb_shade.comp.hlsl:527` (`:512-520`), `deferred_pbr.hlsl:1237`, `forward_opaque.fs.hlsl:333` — each an unbounded `ClusterGrid[cluster]` from live dims |
| Release-clamped host fold (D6's unreachability argument) | `crates/boyko_render/src/light_system.rs:199-210, 212-300` (gates at `:263, 272, 282, 291`; the `debug_assert!` at `:300`); `LightHeaderGpu::new` debug-assert `light.rs:1082` |
| Boot snapshot (D11) | `crates/boyko_app/src/runner.rs:636-643`; `crates/boyko_app/src/gpu_scene/mod.rs:4241` (`build_froxel_light_cull`), `:4304` (spec constants), **`:4317` (the single `cluster_count` mint) / `:4318-4324` (`ClusterGrid` sizing at `cluster_count * 8`)**, `:4325-4331` (`LightIndexList` sizing), `:4332-4338` (`LightIndexAlloc`), `:4346` (`cluster_count` freeze), `:5237`, `:5307` (scene threading); light-table capacity `:205-207` |
| SSAA host-authoritative-lock precedent (D11's debug assert) | `crates/boyko_app/src/runner.rs:1919-1940`; the `scene()` call site `:1951`; the bench `wait_idle` `:2069` |
| Measured band + provenance table | `crates/boyko_render/src/light_policy.rs:40-77` (reproduces §1's six rows verbatim) |
| Host cull oracle | `crates/boyko_rhi_vulkan/src/goldens.rs:3510` (+ `GoldenClusterConfig` `:3398`, `golden_cluster_index` `:3437`, `golden_sq_dist_point_aabb` `:3491-3498`) |
| "Bit-exact" claims that D10 makes structural | `crates/boyko_rhi_vulkan/tests/sdf_gbuffer_hybrid.rs:5187-5188`, `:5199-5202`, `:6291-6293`; the DDGI precedent `crates/boyko_rhi_vulkan/shaders/ddgi_resolve.hlsli:136-143` |
| VB framegraph declaration (seeded trio, `5e07936`) | `crates/boyko_rhi_vulkan/src/present/graph_bridge.rs:3179-3190`; the `light_cull` pass `:3212-3242`; barrier derivation `crates/boyko_rhi_vulkan/src/framegraph/sync.rs:157-169` (`seeded_readers`), `:198-208` (`seeded_writer`), `:266-330` (`transition`, flush branch `:288-296`) |
| VB record site (fill, barrier, dispatch, timestamps) — **⚠ RE-ANCHOR AFTER H0 LANDS: every line number in this row is exact against `git show dc0684e:` and H0 inserts two timestamp brackets into this exact span (Rev 5 P2, §8.5)** | `crates/boyko_rhi_vulkan/src/present/passes/vb.rs:140-247` — timestamp open `:146-150`, the `if let` gate `:157`, fill `:170-174`, `record_vb_pass` `:180`, group count `:184`, `// SAFETY:` `:185-190`, dispatch `:211`, timestamp close `:216-219`. Siblings: `gbuffer.rs:1583` (`// SAFETY:` `:1587-1588`), `forward.rs:359` (`:363-364`) |
| Scene plumbing (D11) | `crates/boyko_rhi_vulkan/src/present/scene_types.rs:415` (`LIGHT_CULL_LOCAL_SIZE_X`), `:438` (`BrickActivation`, the activation idiom), `:1409-1411` (`cluster_count`, and no dims); push mirrors `crates/boyko_rhi_vulkan/src/compute.rs:3447-3496` (struct `:3453-3462`, `CLUSTER_CULL_PUSH_BYTES` `:3465`, const-asserts `:3467-3471`, `ClusterCullPush::new` `:3473-3483`), the shared COMPUTE push budget `COMPOSITE_PUSH_CONSTANT_BYTES == 80` `:2941, :2956` and `rhi_impl/mod.rs:201`, `cluster_cull_spirv()` `:1610`, camera push `:3005-3015`; test literals `crates/boyko_rhi_vulkan/tests/window_present_gbuffer.rs:2387, 3434, 8420, 9905` |
| Existing ORTHO-only cull oracle + readback idioms | `crates/boyko_rhi_vulkan/tests/sdf_gbuffer_hybrid.rs:6432` (fixture `:5215`, cap `:5230`, driver `:5276`, ORTHO `:6455`); **host zero-write BEFORE submit `:5415-5426` [fixed in Rev 5 — the `write_words(mapped, &[0u32])` is on `:5426`, so Rev 4's range stopped one line short]; the POST-FENCE mapped reads `:6202-6211` (ClusterGrid) and `:6219-6228` (LightIndexList) [fixed — Rev 3 cited `:5415-5425` for the post-fence read]** |
| Bench-armed capability whose absence is byte-identical | `crates/boyko_rhi_vulkan/src/present/gpu_timing.rs:232-240`; boot gate `crates/boyko_app/src/gpu_scene/mod.rs:3554-3571`, consumed `:5938-5949`; `Option`-gated readback precedent `crates/boyko_rhi_vulkan/src/present/passes/present_blit.rs:335-400`; `TRANSFER_SRC\|DST` OR `crates/boyko_rhi_vulkan/src/rhi_impl/device.rs:52-58` |
| **Validation features actually enabled (P0-1's basis)** | `crates/boyko_rhi_vulkan/src/device.rs:2087` — `[VK_VALIDATION_FEATURE_ENABLE_SYNCHRONIZATION_VALIDATION_EXT]`, one element, chained at `:2088-2095`. Repo-wide grep for `GPU_ASSISTED` / `debug_printf`: **zero hits**. `robustBufferAccess` bit `ffi.rs:2718` (declared, never enabled) |
| Subgroup features (VB-P1i's precondition) | `crates/boyko_rhi_vulkan/src/device.rs:2584` (`subgroup_size_control: VK_FALSE`); FFI fields `ffi.rs:2623, 2624, 2691, 2703` |
| `.spv` byte gates + `spirv-dis` gate idioms (to clone) | `crates/boyko_rhi_vulkan/tests/cluster_cull_spv_sync.rs:63-88`; multi-variant `crates/boyko_rhi_vulkan/tests/vb_froxel_spv_sync.rs:88-130`; disassembly gate `crates/boyko_rhi_vulkan/tests/field_probe_gate.rs:43-105` (precedent documented at `shaders/sdf_field.hlsli:146-148`) |
| Variant manifest | `docs/SHADER-VARIANT-MANIFEST.md:91-97` |
| The bench | `crates/boyko_app/tests/vb_p1d_cull_shade_bench.rs` — rig `:114-144` (the false "mutually irrational" claim `:122-123`, literals `:133-135` — **`0.618_033_988_75` / `0.381_966_011_25` / `0.236_067_977_5`; Rev 4 quoted the third as `0.236067977`, which is a DIFFERENT number: the printed value violates §1.4's collinearity identity 1023/1024 times, the real literal gives the claimed 0/1024** — `light_range` `:142`, and the **stale pre-split print doc-comment `:47`**, §8.5's one-line follow-up), camera `:235-254`, `ClusterConfig::default()` insertion `:278` (H1.5's hook) |
| §1.3 occupancy pin (to be landed by rung HP) | `crates/boyko_rhi_vulkan/tests/lighting_l1_host_oracle.rs` (8 `#[test]`s today; the fixture `cfg()` `:29`) |
