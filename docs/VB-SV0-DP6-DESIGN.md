# Architecture: VB-SV0 DP6 — producer consolidation (design Rev 4)

# Rev 4 delta — Open Question 1 adjudicated, and the instrument repaired first

> **This block supersedes Open Question 1 in full and amends Decisions 6 and 7, the gate
> definitions, the implementation ladder and §Metrics. Rev 3 is otherwise carried verbatim.**
>
> **Trigger:** DP6-0's baseline measurement turned OQ1 from a hypothetical into a number. The
> number says the comparator Rev 3 specified cannot be adjudicated, for two independent reasons,
> and the repair is one rung.

## R4.1 What DP6-0 measured, and what it proves

### R4.1.1 The reading

Release, 512×512, `sv0_scene`, three identical legs per arm, every arm under clause 5's 10 % bar,
certified resolution **4 608 ns**.

| zone | fused `[vb_both_sdf]` | split `[vb_both_ssao]` | ratio | what precedes its BEGIN |
|---|---|---|---|---|
| `ZONE_VB_SDF_MESH` (10) | 32 256 | 32 768 | **1.016×** | `e9` at `vb.rs:3049`, then the pass's own derived barriers at `:3141` (outside its bracket) |
| `ZONE_VB_GEO` (11) | n/a | **20 480** (arm A, SV0 armed) / **24 576** (arm B, disarmed) — two ARMS on one boot; Δ = −4 096, below resolution | the `vb_viewt` pre-tail dispatch, then the unsplit `record_hzb_poison_build` slot at `:3753-3765` |
| `ZONE_VB_SHADE` (2) | 24 576 | **112 640** | **4.58×** | fused: the SDF_MESH bracket. split: **~256 µs of unbracketed SSAO + à-trous** |

### R4.1.2 The control is already in the data

`ZONE_VB_SDF_MESH` is the same dispatch, on the same scene, under the same TOP→BOTTOM bracket, on
both boots, and agrees to **+512 ns = 0.11 × resolution**. The instrument is not broadly broken.
The one zone that disagrees by 4.58× is the one zone whose BEGIN sits downstream of a
boot-class-dependent unbracketed stretch. **Measured, not argued.**

### R4.1.3 The skew is partial, and nothing in the instrument sets the fraction

Inflation `112 640 − 24 576 = 88 064 ns` against a `~256 000 ns` predecessor stretch: **the TOP
latch absorbed ≈ 34 % of it.** At 0 % the bracket would be honest; at 100 % it would be obviously
broken. At 34 % it is **silently wrong**, and the fraction is set by front-end throttling — a
quantity no gate reads.

Third instance of a named class in this tree: `VB-P1E-HIERARCHICAL-CULL-PLAN.md:2622-2628`
(`CullDispatch` over-counting by `t_fill` for exactly this reason) and profiling rung 7c (five
green commits on silently-changed stages).

### R4.1.4 The hardware agrees, in-tree, measured, from a lane that was not looking for it

`gpu_zone.rs:470-476`, landed by the concurrent particle round:

> *"With the three compute ids on `TOP`, their medians summed against the wall span they were
> supposed to divide (`48.begin → 50.end`, `particle_lab`, three release legs) gave
> **1.083 / 1.025 / 1.077** — every leg over 1, i.e. three rows overlapping. On `BOTTOM` the same
> three legs give **1.000 / 1.002 / 1.000**, and the 1.002 is 32 ns on a 15 296 ns span: one timer
> tick of median-of-21-frames rounding, not an overlap. The ratios are quoted rather than banded
> because a band that reads "1.03–1.08" excludes one of the three legs it claims to summarise."*

Three back-to-back compute dispatches on **this box**, same recorder, same wrapper: TOP begins
overlapped on every leg; BOTTOM begins partitioned to within a tick. The ratios are quoted here for
the same reason that text refuses to band them.

### R4.1.5 "The split shade genuinely costs 4.6× more" does not survive this design's own arithmetic

Decision 3's fetch table counts `vb_shade_split` and `vb_resolve` as **one full-screen
`vb_geom_fetch` each**. The split shade's surplus over the fused resolve is the SSAO combine plus a
`thin_normal` read — the budget Decision 4 prices at **< 1 %** against 128 march iterations. **No
route from that budget reaches 4.58×.** Contribution (a) is real and small; contribution (b)
dominates.

### R4.1.6 Where the contamination actually lands — and it is not where Rev 3 supposed

`vb_both_sdf` carries no `SsaoConfig`. After DP6a it resolves `vb_sv0_split ⇒
mesh_geo_shade_split` with `pre_light == false`, so the split arm runs with `scene.ssao == None`:

- SSAO gather `vb.rs:3892`, à-trous `:3932-4022` — **skipped**
- `path_vb_ddgi()` = `path_is_vb ∧ mesh_geo_shade_split ∧ ddgi_update.is_some()`
  (`scene_types.rs:4076-4080`) — **`ddgi_update` is None** (no `DdgiConfig`), skipped
- `path_vb_hwrt_shadow()` = `path_vb_split() ∧ shadow.is_some()`, `#[cfg(feature = "hwrt")]`
  (`:4088-4092`) — feature default-OFF **and** `shadow` None, skipped

**The conjunction is stated because arming the split does not by itself keep DDGI and hwrt off —
both key on `mesh_geo_shade_split`, not on SSAO.** All three conjuncts hold on `vb_both_sdf`, so
**G-NEUTRAL's after-side has no unbracketed stretch at all.**

The 4.58× therefore lands on the **G-REDUCE** row, where the SSAO stretch is present on both sides
and is *partially* common-mode. G-NEUTRAL is contaminated by a smaller, different term: its
after-side `ZONE_VB_GEO` latches TOP while `ZONE_VB_LATE_RASTER` drains, and its before-side has no
`ZONE_VB_GEO` at all — **that skew has no counterpart to cancel against.**

### R4.1.7 Why "the skew is common-mode, do nothing" is refuted, by the numbers

DP6 deletes a dispatch re-taken at **36 864 ns** (the id-10 zone median at DP6-0b) — **35 328 ns** on
the same-fixture paired route `NET(on) − NET(off)`, the two agreeing to 1 536 ns = 3 ticks; DP6-0's
`32 768` is superseded, the instrument having changed under it — ahead of both latches, and adds
`M_geo ∈ [6 100, 14 400] ns` into `ZONE_VB_GEO` — **still a model at DP6-0b, see Decision 3's §0**:
the two `ZONE_VB_GEO` arms differ by 3 072 ns on the *same* base variant and do not bound it. The
latch is demonstrably throttled by execution progress — were it not, `ZONE_VB_SHADE` would have read
≥ 256 µs, not 112.6. So a ~36 µs change in preceding work moves the latch by an unknown amount **of
the same order as the effect** (`D + F ∈ [20 939, 29 184] ns`).
Cancellation is first-order only and the residual is bounded by nothing.

### R4.1.8 A second, independent defect in the specified comparator

Rev 3's split-pair quantity is `ZONE_VB_GEO + ZONE_VB_SHADE` — a **sum of two per-zone medians**.
`contrast.rs:477` computes `median_ns += row.median_ns`, i.e. `Σ(median)`, which
`03-STATISTICS.md:121-125` forbids: *"the sum is formed per frame, then reduced — `median_f(Σ_members)`,
never `Σ_members(median_f)`"*, an inequality that was crossed by 144–240 ns on a real reading.
`contrast.rs:447-456` **states** the precondition it would then violate (valid *"only if the zones
are the partition their ids say they are"*) and has **no caller passing a multi-zone set** — so the
defect is not a shipped miscomputation but **Rev 3's own caller-side subscription of a
non-partition**, which would have been the first such caller. Recorded alongside:
`03-STATISTICS.md:123-125` says the reducer has *"no API that adds two reduced statistics"*, and
`LegSummary::from_artifact` is one.

**Both defects are fixed by the same repair.**

## R4.2 Decisions

### R4-D1 — The restamp is the tree's own rule applied to a premise this rung changes

`gpu_zone.rs:415-419` states the rule:

> *"A `TOP_OF_PIPE` begin retires when the command is fetched, which is legal **only where the
> bracket is preceded by work that is not being attributed to it** — otherwise the stamp lands
> before its predecessor has drained and the two brackets OVERLAP."*

Ids 10 and 11 are kept at TOP on the premise that they *"sit with unbracketed work on BOTH sides"*
— and `gpu_zone.rs:440-452` already qualifies that premise for id 11 and **already names this
rung**: *"That leg's `ZONE_VB_GEO` number is contaminated by the drain ahead of it, and it is
recorded here as a known limitation rather than fixed: rung **DP6-0b** restamps ids 10 and 11 to
`BOTTOM` with a stated reason and re-baselines against it."*

**DP6-0b falsifies the premise deliberately.** After it, ids 10 and 11 are members of a
consecutive-partition run, and `ZONE_VB_PRESHADE` removes the unbracketed stretch on id 11's far
side. Under the rule as written, they must then bottom. **This is not a reversal of the concurrent
round; it is that round's rule evaluated against a changed premise, at the rung that round
nominated.**

`ZONE_PARTICLE_DRAW` is untouched (premise unchanged). `ZONE_VB_SHADE` (id 2) is untouched — the
split producer's cost is derived (R4-D3), so VB-P1d's published break-even keeps its meaning and
`tops(ZONE_VB_SHADE)` stands.

**Spelling.** Four **names**, never a range:
`|| matches!(zone, ZONE_VB_SDF_MESH | ZONE_VB_GEO | ZONE_VB_PRODUCE_RUN | ZONE_VB_PRESHADE)`
— the tree's own argument at `gpu_zone.rs:485-488` (*"Names cost three tokens and cannot do
that"*). A range `10..=13` would sit flush against `3..=9` and re-open rung 7c's slide. **The
assertion message at `gpu_zone.rs:539-542` ("the three isolated single-dispatch ids open at
`TOP_OF_PIPE`") goes false in this same edit and is rewritten to name `ZONE_PARTICLE_DRAW` alone,
in the same commit.**

### R4-D2 — One comparator: `ZONE_VB_PRODUCE_RUN`, opened and closed under one hoisted predicate

**Span.** BOTTOM→BOTTOM, begin immediately after `ts.end(ZONE_VB_RUN)` at `vb.rs:3049`, end
immediately after the lit-producer chain closes at `vb.rs:4494`. It is the smallest interval
containing every site DP6 can move work into or out of: `ZONE_VB_SDF_MESH` `[3167,3206]`, the
unsplit hzb slot `[3753,3765]`, `vb_viewt` `:3776`, `ZONE_VB_GEO` `[3798,3889]`, the
SSAO/à-trous/hwrt/DDGI stretch `[3892,4307]`, and all three `ZONE_VB_SHADE` arms. **Its definition
never mentions which producer ran, so it is identical on both sides of the fused/split
discontinuity by construction** — which neither `ZONE_VB_SHADE` nor `ZONE_VB_GEO + ZONE_VB_SHADE`
is.

**Scope, verified.** `if scene.resolved_render_path.mesh_leg {` opens at `:1529` and closes at
`:3735`; `:3050` is inside it, `:4495` is outside it; there is **no `return` anywhere in
`[3049, 4494]`**. The bracket **cannot** be moved to one nesting level without losing its subject —
id 10 and the fused/classified shade arms are inside the `mesh_leg` block, and opening after
`:3735` would exclude exactly what DP6 deletes. So:

```rust
let produce_run_armed = scene.resolved_render_path.mesh_leg;   // hoisted ONCE, above :1529
```

read at the begin (`:3050`, where scope also implies it) and at the end (`:4495`,
`if produce_run_armed { … }`). One binding, two consumers — invariant 9's discipline.

**`mesh_leg` is the correct arming predicate, not a workaround.** `mesh_geo_shade_split ⇒ mesh_leg`
by definition, so `path_vb_split ⇒ mesh_leg`, and every lit producer plus the dedicated SV0 pass is
inside that block. **On a mesh-less leg there is no producer run**, and `NotBracketed` is the honest
label. `debug_assert!(!scene.path_vb_split() || produce_run_armed)` at the close makes the
containment structural.

**Bonus the sum could not have.** `ZONE_VB_SDF_MESH`'s `record_vb_pass` barriers at `:3141` are
**outside** its bracket while ids 11 and 2 include theirs — a real attribution asymmetry. The run
bracket is **immune** to it. That immunity is the whole argument for gating on a run rather than on
members; the asymmetry is fixed at DP6-0b for attribution's sake and the gate does not depend on
the fix.

### R4-D3 — `ZONE_VB_PRESHADE`, and the split producer's cost by derivation

BOTTOM→BOTTOM, begin `vb.rs:3890` (after `ts.end(ZONE_VB_GEO)`, before `if scene.path_vb_ssao()` at
`:3892`), end `vb.rs:4327` (after the DDGI block closes at `:4307`, before `ts.begin(ZONE_VB_SHADE)`
at `:4328`). Both inside `if scene.path_vb_split()`.

It completes the partition, so `shade_derived = PRODUCE_RUN.end − PRESHADE.end` — two BOTTOM stamps
this rung owns — and **id 2 is never restamped.**

It also ends the state in which ~256 µs of shipped GPU work sits outside every bracket. It does
**not** make `gpu_zone.rs:238`'s claim (*"the family's ONLY unbracketed dispatch"*) true:
`sdf_forward_march` (`vb.rs:4503-4608`, unbracketed compute on **both** legs of every VB×Both frame)
and `vb_viewt` (unbracketed at **both** sites) remain outside every bracket. **The `:238` edit
narrows a falsehood to an enumerated residual and names what remains** — the doc-rot discipline, not
a claim of repair.

### R4-D4 — The derived per-frame row is PRIMARY; the wide bracket is the total

On `[vb_both_ssao]`, `PRESHADE ≈ 256 µs` is **~78 %** of a `PRODUCE_RUN ≈ 330 µs`, so it dominates
that bracket's variance as well as its magnitude. A control set at `R_preshade` cannot bound an
effect whose **entire low end is 20 939 ns**. And migration-immunity does not discriminate: **DP6's
migration is entirely upstream of `b13`**, so `PRODUCE_RUN − PRESHADE` is equally migration-immune
*for this rung*, on a 3–4× smaller base, with the largest jitter term cancelled **per frame** rather
than tolerated by a threshold.

Applying one consistent 8 % scaling to both candidates:

| comparator | base | effect `D+F` | R at 8 % | effect / R | verdict |
|---|---|---|---|---|---|
| `PRODUCE_RUN` (wide) | ~330 µs | 20.9–29.2 µs | ~26 µs | **0.8–1.1×** | below its own resolution |
| **`PRODUCE_RUN − PRESHADE`** | ~74 µs | 20.9–29.2 µs | ~5.9 µs | **3.5–4.9×** | **resolvable** |

**Decision: `ZONE_VB_PRODUCE_NET = PRODUCE_RUN − PRESHADE`, formed per frame, is the primary
comparator on both rows.** `PRODUCE_RUN` is reported beside it as the total and as a per-frame
cross-check (`NET + PRESHADE ≡ PRODUCE_RUN`).

**On the fused row `PRESHADE` is absent-Forbidden before and `≈ 0` after, so `NET ≡ PRODUCE_RUN`
there — the two rows use ONE comparator, not two.**

**The control is demoted, not deleted.** `Δ median(PRESHADE)` becomes a **reported anomaly requiring
a stated cause**: DP6 emits no SSAO, à-trous, DDGI or hwrt command, so a movement beyond
`R_preshade` means the workload changed and the run is re-taken. It no longer has to bound the
effect, because the effect is no longer inside it.

### R4-D5 — What is retained from the "restrict the claim" option, and why it is not the answer

Restricting G-REDUCE to boots split on both sides, and handing the fused→split transition to
Decision 6's same-boot paired-delta, is **rejected as primary**: that transition **is** what
G-NEUTRAL exists to price — Decision 3 says so verbatim (*"Gated by G-NEUTRAL, which is the only
reason this trade is acceptable"*) — and arms A/B/C are all same-boot, so none of them crosses the
boundary. It would leave Decision 3 unpriced and foreclose OQ1's own fallback.

**Retained:** the observation is correct that the same-boot arms were never contaminated, and after
the restamp they get strictly better. `Δ_AB` and `Δ_BC` move onto the BOTTOM-stamped `ZONE_VB_GEO`,
where they become exact differences of prefix-completion times on one stream. Decision 6's
instrument is **strengthened**, not replaced.

## R4.3 Mechanism specifications

### R4.3.1 Zone ids and headroom

| id | name | stamped? | stage |
|---|---|---|---|
| `ZONE_BASE_VB + 12` | `ZONE_VB_PRODUCE_RUN` | yes | BOTTOM |
| `ZONE_BASE_VB + 13` | `ZONE_VB_PRESHADE` | yes | BOTTOM |
| `ZONE_BASE_VB + 14` | `ZONE_VB_PRODUCE_NET` | **never** — derived | n/a |

`VB_ZONE_COUNT` **12 → 15**, under the `≤ 16` const assert at `gpu_zone.rs:292-295`. **This rung
consumes three of the family's four remaining ids; ONE slot remains, and the next VB zone after that
is the last the `u16` witness masks can carry.** Stated as a cost, not discovered later.

`ZONE_VB_PRODUCE_NET` sits in the VB family because the artifact's `[[zone]]` rows are keyed by
`u16` zone id and every consumer finds by `z.zone == want`; a second id space would be a second
vocabulary. **`TsWitness` never stamps it: `slot_of` and the leg table are untouched, `pair_of[14]`
stays `NO_PAIR`, mask bit 14 is never set, and the expectation table declares it `Forbidden` as a
stamped row on every leg** — a release-live check, not a `debug_assert`. `pair_of` and the masks
grow by one entry automatically from `VB_ZONE_COUNT`; no call site changes.

### R4.3.2 Record sites (all verified)

| what | site | scope |
|---|---|---|
| `PRODUCE_RUN` begin | immediately after `vb.rs:3049` | inside `mesh_leg` `[1529,3735]` |
| `PRODUCE_RUN` end | immediately after `vb.rs:4494`, under `produce_run_armed` | outside; predicate hoisted above `:1529` |
| `PRESHADE` begin | `vb.rs:3890` | inside `path_vb_split()` `[3771,4494]` |
| `PRESHADE` end | `vb.rs:4327` | same |
| `ZONE_VB_SDF_MESH` begin | moves **above** its `record_vb_pass` at `vb.rs:3141` | attribution symmetry; the gate does not depend on it |

### R4.3.3 The per-frame channel — a deliverable inside `WindowReducer::observe_frame`

Artifact rows carry window **medians** of `begin_off_ns`/`end_off_ns` (`reduce.rs:199-213`);
per-frame samples live in private `ZoneAccum` vectors and die at `finish()`. A containment check
written as a test over the artifact would compare medians — a different statement, and
`vg_occ_split_timing.rs:669-672` records that exact composition reporting an inequality **backwards
by 144 ns**. A TOP-stamped member violates the chain *non-deterministically*, which is precisely
what a median averages away.

`observe_frame(&mut self, pairs: &[PairResult])` (`reduce.rs:105-157`) is the **only** place in the
tree holding one frame's `PairResult` slice. The check goes there, and **only its verdict is
reduced**:

1. **`WindowReducer::new` gains a declared `chain: &'static [u16]` and
   `derived: &'static [DerivedSpec]`.** Declarations, so a family shipping an undeclared chain is a
   compile-site omission, not a silent pass.
2. **Per frame, over that frame's raw `begin_ticks`/`dur_ticks`:** for each consecutive `(a,b)` in
   `chain` present-and-`Measured` this frame, require `begin(a) ≤ begin(b)`; and for each member
   `m`, `begin(run) ≤ begin(m)` and `begin(m)+dur(m) ≤ begin(run)+dur(run)`. Ties are legal (`≤`) —
   equal-tick BOTTOM stamps give equal offsets, and this box's empty-bracket floor is 96–128 ticks.
3. **The output is a COUNT, not a statistic:**
   `OrderCensus { frames_checked, frames_skipped, violations, worst_ns }`. A median can average a
   violation away; a count cannot. Quantitatively: with per-frame violation probability `p` over a
   ~100-frame window, `P(0 violations) = (1-p)^100`; at the 2.5–8.3 % overlap the particle lane
   measured, that is `< 10⁻³`.
4. **`frames_checked` is published,** so `violations == 0` over `frames_checked == 0` is
   distinguishable from a pass.
5. **First violation raises a sticky `boyko_diag` flag,** so a red reaches a reader who never opens
   the artifact.
6. **The derived row is formed AT THE FRAME** — `PRODUCE_RUN.dur − PRESHADE.dur` from the frame's
   own slice, pushed as **one sample** into an ordinary `ZoneAccum`. **Nothing is ever zipped**, so
   positional misalignment across `Lost`/`NotBracketed` frames cannot arise; and this is
   `median_f(Σ)`, which is what R4-D7's statistics rule demands.
7. **Artifact.** The `[order]` block dispatches at the **match-`k` arm set at `artifact.rs:848+`,
   with its own builder** (`:830-845` is the `[[zone]]` arm set, and `:842`/`:1026` are zone-row
   sites — those are affected by the derived ROW, a separate item).

### R4.3.4 The absence policy keys on ABSENCE, resolved against the leg's expectation

A never-opened zone has **no `PairResult` in the slice at all** — `alloc_pair` is called only from
`begin`, and `label_slot` iterates `used_pairs`. It is **absent**, not `NotBracketed`. Keying the
policy on the label would key on something the per-frame path does not deliver for the structural
case.

Worse, the two absences differ in what they mean:

| absence | cause | correct contribution |
|---|---|---|
| **structural** | `PRESHADE` on a fused leg — `path_vb_split()` false, the bracket is not in the stream | **0.0** |
| **runtime** | `alloc_pair` returned `None` on a full ring (`gpu_zone.rs:761-777`, raising `GpuPairBudgetExhausted`) — **`PRESHADE`'s 256 µs EXECUTED** | **skip the frame** |

Contributing `0.0` in the runtime case yields a **4.5× inflated `NET` sample**, and the `n` floor
cannot catch it because the sample was pushed, not skipped.

**`DerivedSpec`'s policy therefore keys on member-absent-from-slice, resolved against the leg's
declared expectation:**
- expectation `Forbidden` ⇒ structural ⇒ contribute **0.0**
- expectation `Required` but absent ⇒ **skip the frame**, count in `frames_skipped`
- expectation `Optional` ⇒ the spec must declare which; an undeclared `Optional` member is a
  compile-site omission

**Floor:** if the derived row's `n < 0.9 × frames_checked`, the row is **INCONCLUSIVE** — a derived
row over a different subset of frames than its terms is not comparable to them.

### R4.3.5 The unmatched-END detector, RELEASE-LIVE

`TsWitness::end` (`vb.rs:411-424`) returns silently when `pair_of[slot] == NO_PAIR` (`:238`,
`u16::MAX`): no command, no `Torn`, no counter — `mark_end` is *after* the return. `Torn` is
`begun ∧ ¬ended` and catches only the opposite direction.

**`TsWitness::writes` cannot be the detector.** It is `#[cfg(debug_assertions)]` (`vb.rs:232-233`)
and documented *"Dev-profile only"*, while the gates run **release** — a detector there is dead in
exactly the runs clause 5(2) reads it in, and red mutation (c) would show green. The type's own doc
settled this class at `vb.rs:184-186`: *"[`Self::finish`] is the per-frame invariant, and it is
RELEASE-LIVE: a `debug_assert!` cannot substitute, because the timing worker inherits the driver's
profile and a release bench run has `debug_assertions` OFF."*

**Decision — two mechanisms, each with its stated liveness, and `writes` keeps its own name.** The
two invariants are different: `writes` counts a `VUID-vkCmdWriteTimestamp` double-write, a
driver-correctness question no gate reads; the begin/end pairing is a **gate input**. Merging them
would put two liveness requirements on one array, and making `writes` release-live would falsify its
own doc.

- **`writes` stays `#[cfg(debug_assertions)]`, and `finish` READS it under the same `cfg`.** That
  closes the dead datum on its own terms: `vb.rs:5353` and `:2540` both assert *"the dev-profile
  double-write counter in `TsWitness::finish` reds by slot name"* — a safety net cited as
  load-bearing for `record_hzb_poison_build`'s mutual-exclusion invariant, incremented at
  `:333`/`:355` and **read nowhere**. After this edit those two comments are true in the profile they
  name.
- **Two NEW unconditional `u16` masks — `begin_called`, `end_called`** — set at the **top** of
  `begin` and `end`, **before** the `NO_PAIR` / alloc-failure early returns. Four bytes on a
  per-frame stack struct. `finish` (already release-live) does three mask compares, total over both
  directions:

| condition | meaning | outcome |
|---|---|---|
| `begin_called & !begun` | `alloc_pair` returned `None` | already flagged `GpuPairBudgetExhausted` |
| `end_called & !begin_called` | **unmatched END** — the direction that was invisible | raise `GpuZoneUnmatchedEnd` |
| `begin_called & !end_called` | unmatched BEGIN | `Torn`, now visible even when alloc failed |

Sticky flags via `boyko_diag::loss::raise`, the idiom `alloc_pair` uses at `gpu_zone.rs:775` and for
the same structural reason: this crate cannot reach the `92xx` emitter.

**DP6d's leg shapes gain a fifth — a non-mesh-leg `VB × Sdf` leg.** Without it the four originally
listed could never exercise the direction this detector exists to catch.

### R4.3.6 What the span contains that is not its subject

**`ZONE_VB_HZB_BUILD` (id 6).** `record_hzb_poison_build` stamps it unconditionally at its own first
and last statements (`vb.rs:5376`, `:5576`). Its **unsplit** call site `:3753-3765`, gated only on
`!occlusion_split`, runs on every leg and sits textually inside `[3050, 4495]`; its split-armed
sibling `:2541-2542` is inside `ZONE_VB_RUN`, i.e. **outside** `PRODUCE_RUN`. `vb.rs:3749-3752`
states the governing rule: *"every aggregate that spans both legs excludes it by name."* Three
obligations:
1. `PRODUCE_RUN`'s doc **names id 6** and states the scope: *comparable only within one
   occlusion-split arming.*
2. id 6 enters the chain **leg-conditionally**, driven by the expectation table. It is already
   `bottoms(...)`, so it composes natively.
3. **The gate asserts occlusion-split leg-field equality between the two sides** before comparing.
   DP6 does not touch `HzbConfig`, so this makes a common-mode assumption verified rather than
   assumed.

**`vb_viewt` — and the authoritative predicate is the two-arm expression, not "no `TaaConfig`".**
`gpu_scene/mod.rs:6550-6553`:

```
viewt_from_vb_depth = VB ∧ mesh_leg ∧ ( (¬sdf_leg ∧ aa_mode == Taa)                      [arm a]
                                      ∨ (mesh_geo_shade_split ∧ ssao_variant.is_some()) ) [arm b]
```

Derived **per side**:

| fixture | side | arm (a) | arm (b) | result |
|---|---|---|---|---|
| `[vb_both_ssao]` | before | dead (`sdf_leg`) | **fires** (split ∧ SSAO) | **runs**, at site A `:3776`, **inside** `PRODUCE_RUN` |
| `[vb_both_ssao]` | after | dead | **fires** | **runs**, same site — **common-mode inside the run ✓** |
| `[vb_both_sdf]` | before | dead (`sdf_leg`) | dead (no split) | None |
| `[vb_both_sdf]` | after | dead | dead (**no SSAO**, though the split is now armed) | None — **common-mode ✓** |

**So `vb_viewt` DOES run on `[vb_both_ssao]`, and `gpu_zone.rs:450-452` quantifies it: the 5 248 ns
gap between id 6's END and id 11's BEGIN IS that dispatch.** §R4.1.1's GEO predecessor row is stated
accordingly. The conclusion (common-mode on each row) survives by the corrected route, and the
reason `[vb_both_sdf]`'s after-side stays clean is **arm (b)'s second conjunct**, not the absence of
`TaaConfig`.

**It gets a per-side cell in the expectation table** — `Forbidden` on `[vb_both_sdf]` both sides,
`Required` on `[vb_both_ssao]` both sides — **checked mechanically without minting a zone**: the
`[e6 → b11]` gap must be ≈ 5 248 ns where `Required` and ≈ 0 where `Forbidden`. That turns an
unbracketed dispatch into a checked one.

> **⚠️ Correction, landed with the implementation: the `Forbidden` side is not live at DP6-0b.**
> On `[vb_both_sdf]` there is no `b11` — id 11 stamps only inside `if scene.path_vb_split()` and
> that fixture is fused — so the gap has no second end and the arithmetic has no subject. What
> ships: the fused driver asserts the gap is **absent** (`None`), which is "the check does not
> apply here" rather than "the check passed", and the `Forbidden` arm is exercised by unit-tested
> arithmetic. It becomes live on a real leg at **DP6a**, where `[vb_both_sdf]` gains the split
> (id 11 appears) while still carrying no `SsaoConfig` (arm (b) stays dead) — the first boot on
> which "id 11 exists and `vb_viewt` must not have run" is a statement about a frame.

**The residual hazard is named in `PRODUCE_RUN`'s doc beside id 6:** site A `vb.rs:3773-3779` is
inside the span and site B `:4622-4628` (gated `ssao.is_none()`) is outside it, so a config that
flips which arm fires **moves a dispatch across `e12`**. A TAA-armed VB×Mesh boot takes arm (a) with
`ssao.is_none()` ⇒ site B ⇒ outside. The first such fixture would otherwise move it unnoticed.

### R4.3.7 Outcome space for the diagnosis, made total

The repair rung is a red-capable test of §R4.1's own diagnosis. Let
`Δ_host = shade_derived − shade_fused|DP6-0b` — **the RE-TAKEN fused-shade cell from DP6-0b, never
DP6-0's voided 24 576.**

| branch | verdict | consequence |
|---|---|---|
| `\|Δ_host\| ≤ 2 × R_neutral` | §R4.1 **confirmed** — the 112 640 was latch skew | proceed |
| `2 × R_neutral < Δ_host ≤ 29 184` | **mixed** — both contributions material | `Δ_host` is recorded as `E_split_host`, the split tail's hosting surcharge. **DP6a does not land** until Decision 3's fused row is re-derived with it, because G-NEUTRAL's after-side pays it and its before-side does not |
| `Δ_host > 29 184` | §R4.1 **refuted** | `vb_shade_split` costs more than one extra full-screen fetch+dispatch over `vb_resolve` — no work-budget explanation remains. **DP6's cost model re-opens**, and Decision 3's consolidate-into-the-split premise is in question |

**The `29 184` boundary is derived, not chosen:** it is this rung's own published upper bound on
`D + F`, one full-screen dispatch plus one `vb_geom_fetch`.

> **MEASURED at DP6-0b: `Δ_host = 11 264 ns` ⇒ the MIDDLE row. `E_split_host = 11 264 ns`, and
> DP6a is BLOCKED** until Decision 3's fused row absorbs it. The margin to the first row is
> **272 ns — half a timer tick, i.e. unresolvable** — so branch 1 is not excluded by the
> measurement; what makes the verdict usable is that all nine leg pairings land in this row
> (`E_split_host ∈ [10 512, 19 392] ns`), so no pairing reaches branch 1 or branch 3. Full cells,
> spreads and the instrument-skew decomposition are in the DP6-0b RESULT block of §The ladder.
>
> **BLOCK DISCHARGED at Rev 4.4.** Decision 3's *"Trade-off, RE-DERIVED at DP6-0b"* sub-block
> absorbs `E_split_host` into the fused row — `Δ NET ∈ [−5 404, +14 848] ns`, point estimate
> `+1 034`. **DP6a may land.** The blocking sentence above is kept rather than rewritten, because
> it is the measurement that imposed the block.

## R4.4 `vg_occ_split_timing` — the premise, corrected

Its worker is one `VB × Mesh` boot (`vg_occ_split_timing.rs:629-636`; no
`SsaoConfig`/`DdgiConfig`/`TaaConfig`; `:644` `assert_no_split_producer`). On it, ids 10/11/13
**never stamp** — no `alloc_pair`, no `PairResult`, no `ZoneAccum`, **no row**. They are not
written-and-unread; they do not exist there. And `:708-713` **panics** via `unwrap_or_else` on a
missing row, so widening `PASS_COUNT` 10→14 would be an **unconditional panic on all four legs**.
**That edit is withdrawn** — it was never in a shipped Rev, and it is recorded here so no reader
re-derives it.

**The blindness is real and doubled:** blind **by fixture** (its boot structurally cannot stamp
10/11/13) **and by loop bound** (`PASS_COUNT = 10` would ignore them if it could). Widening the
bound fixes neither.

`vg_occ_split_timing` keeps `PASS_COUNT = 10` and gains **exactly one row** (id 12 — `mesh_leg` is
true on VB×Mesh), which its bounded loop ignores. No panic. Its `begin_off` base is unmoved: the
frame's earliest measured begin is `ZONE_VB_CULL_RESET` at `vb.rs:1066`, far ahead of `:3050`.

**The `280/560` pin does not exist, and it is not being minted.** The literals appear nowhere in
`vb_bench_query_validation.rs`, which asserts `bench_ok == control_ok`, `measured > 0`, and
validation-message-set equality. The only occurrences are doc comments at `gpu_zone.rs:232` and
`:241-242` citing a number the named test does not contain. **A pair-count pin is refused with a
reason:** it is leg-dependent, would red on every legitimate zone addition, and has already failed
to red on two. The expectation table asserts the invariant those comments were reaching for — *which
zones stamp on which leg* — and is red-capable.

## R4.5 Costs

| cost | size | disposition |
|---|---|---|
| One rung before DP6a | no behaviour change | required: DP6a moves armed-leg timing |
| DP6-0's four cells void | 4 numbers | re-taken; kept as the evidence for the repair |
| **Zone budget 12 → 15** | three of four free ids | **ONE remains**; the next VB zone after it is the last the `u16` masks carry |
| Reducer gains a per-frame predicate | one pass over one frame's slice, off-frame host code | the module already allocates per-zone `Vec`s here (`reduce.rs:32-37`) |
| Artifact gains `[order]` | one match-`k` arm set + builder | `artifact.rs:848+` |
| Two unconditional `u16` masks + 3 compares in `finish` | 4 bytes, per frame, off the GPU path | the price of a release-live gate input |
| DP6b's `vb_geo_aux_layout` 3→5 widening | 2 descriptor writes at boot, **0 per-frame commands** — `cmd_bind_descriptor_sets` at `vb.rs:3850` binds the set regardless of width | priced, negligible |
| New harness | 1 file | it is the real content of the blindness finding, not overhead |
| **Wide-bracket dilution** | — | **no longer a cost**: R4-D4 makes the derived row primary. `PRODUCE_RUN`'s fused magnitude is **not a measured quantity anywhere** — it is the `[3050,4495]` interval including the id-6 slot and the inter-bracket gaps, not a sum of two brackets — and becomes a DP6-0b cell. No comparison is made against 4 608, since inheriting it is forbidden |

## R4.6 Residuals and the dead-datum ledger

**`zone_begin_stage`'s lost gate is NARROWED, not closed.** `docs/OPEN-QUESTIONS.md:471-475` is
about the table as a whole. After DP6-0b the VB ids stand in three tiers:

| tier | ids | check |
|---|---|---|
| per-frame chain (this rung) | **10, 11, 12, 13** | `OrderCensus`, per frame, counted |
| median-level chain (today) | 3, 4, 5, 7, 8, 9 | `vg_occ_split_timing`'s monotone clause — weaker, and `:669-672`'s own 144 ns finding says why |
| none | 0, 1, 2, **6**, 14, and the gbuffer / SV0 / particle families | — |

> **⚠️ Correction, landed with the implementation (this table's first form put id 6 in tier 1).**
> The chain is exactly the quartet `zone_begin_stage` restamps to `BOTTOM_OF_PIPE`, and id 6 is not
> in it. The reason is stronger than "its position is leg-dependent":
> **`path_vb_occlusion_split()` is not boot-frozen at all.** It conjoins
> `scene.vb_occlusion.is_some()` — recomputed EVERY FRAME in `boyko_app::runner` from a live
> `OcclusionConfig` resource, since rung P4-4 turned that regime from a boot env read into a live
> resource — and `scene.vb_occlusion_instances > 0`, this frame's count of marker-carrying
> instances. So id 6 can take its `ZONE_VB_RUN`-side slot on one frame of a window and its
> post-producer slot on the next, while `chain` is one `&'static [u16]` chosen once at reducer
> construction. A declaration naming id 6 would be right on some frames of a single window and
> wrong on others — and the wrong ones would be counted as violations of an order the recorder
> never promised. Its containment is still asserted the way the design intends: the gate requires
> the two sides of a comparison to agree in occlusion-split arming.
>
> **Id 2 is in tier "none" and stays there**, which the implementation now enforces by name: it
> keeps `TOP_OF_PIPE`, a `TOP` begin retires at command FETCH rather than at prefix completion, so
> ordering it against `BOTTOM` begins would manufacture non-deterministic violations that describe
> only the stage difference. **Id 14** is derived and never stamped, so it has no begin to order.

**Offered as a follow-up, not claimed as done:** once the per-frame channel exists, upgrading tier 2
to it is a one-line change to that harness's chain declaration.

**Dead-datum ledger** (a list, because the count was wrong and a count is the wrong shape):
- **`gpu_zone.rs:238`** — *"the family's ONLY unbracketed dispatch"*, false while SSAO, à-trous,
  DDGI, hwrt, `sdf_forward_march` and both `vb_viewt` sites are unbracketed. → narrowed to an
  enumerated residual.
- **`gpu_zone.rs:232` / `:241-242`** — cite a `280/560` pin the named test does not contain. →
  corrected to point at the expectation table.
- **`TsWitness::writes`** — incremented at `vb.rs:333`/`:355`, read nowhere, while `:5353` and
  `:2540` assert a `finish` counter that does not exist. → `finish` reads it under its own `cfg`.
- *(stale, dropped: "id 10 has no stage pin" — the concurrent round landed `tops(ZONE_VB_SDF_MESH)`
  at `gpu_zone.rs:539-542`.)*

**Note beyond this rung's scope:** `TsWitness::writes` being unread means
`record_hzb_poison_build`'s documented mutual-exclusion safety net has never existed in any profile.
DP6-0b repairs it as a side effect; if DP6 is deferred, that repair should be lifted out and landed
on its own, since it guards an invariant unrelated to this rung.

## R4.7 Revision trail

| rev | change |
|---|---|
| 4 | OQ1 adjudicated: A+B composed, C rejected as primary. Repair rung, run bracket, containment clause. |
| 4.1 | 4 P0 + 8 P1: containment moved into `observe_frame`; `PASS_COUNT` edit withdrawn and the blindness premise corrected; `PRODUCE_RUN`'s predicate hoisted + unmatched-END detector; **primary/fallback inverted with a number**; id 6 and `vb_viewt` named; DDGI/hwrt mechanism corrected; DP6a's timing claim corrected; the `280/560` pin refuted; `Δ_host`'s third branch; OQ narrowed. |
| **4.3 (post-implementation, measured)** | Written back from the rung that ran. **RESULT recorded**: four re-taken cells, `E_split_host = 11 264 ns`, **branch 2 MIXED**, DP6a BLOCKED, 87.8 % of DP6-0's inflation shown to be instrument skew (§The ladder's DP6-0b RESULT block, §R4.3.7, OQ1). **Mutation (a)'s predicted red CORRECTED against measurement** — `OrderCensus` does not fire on it (0 violations / 241 frames); the `const` stage pin's build failure is the real gate and the `[e6 → b11]` gap the runtime carrier; §R4.3.3's nondeterminism argument is scoped to members whose predecessor is a tick away, not tens of µs. **Mutation (c)'s direction corrected.** **§R4.6's tier table corrected**: id 6 → tier "none" because occlusion-split arming is per-frame and not boot-frozen; ids 2 and 14 placed with their reasons. **§R4.3.6**: the `vb_viewt` `Forbidden` cell is not live until DP6a. |
| **4.4 (Decision 3 re-derived; arithmetic only, no design change)** | **The headline edit is a WITHDRAWAL:** §Goal's *"fused boots: cost-neutral **by construction** (2/2 → 2/2)"* is struck — the counts are still 2/2 → 2/2, but the boot changes **leg class** and pays `E_split_host`, so neutrality is a **measured question, not a construction**, and it is claimed at DP6d or not at all. Decision 3's trade-off paragraph loses its second sentence and gains a **RE-DERIVED sub-block** (§R4.3.7's block, discharged): the four non-cancelling terms — deletion **−35 328** (measured, paired; id-10 median 36 864 agrees to 3 ticks), `GEO_base` **+13 312…+16 384** (measured, transferable post-restamp), `M_geo` **+6 100…+14 400** (**MODEL**, unmeasured — the two `ZONE_VB_GEO` arms run the *same* base variant and their 3 072 ns Δ does not bound it), `E_split_host` **+10 512…+19 392** (measured) — giving **`Δ NET_fused ∈ [−5 404, +14 848] ns`, point `+1 034`** against a `+5 120` bar, cross-checked against a DISJOINT-INPUT route through the split row (`Δ NET_fused = Δ NET_split + GEO_base + E_split_host` = `[−6 940, +13 312]`, point `−502`; the 1 536 ns gap IS the deletion term measured two ways on two fixtures — the original "independent absolute reconstruction" was struck at the DP6a review as the same calculation rearranged), with a three-row `M_geo` ceiling table showing the red region is confined to the joint upper corner (**99.2 %** of the model band clears at median inputs). Decision 6's cost table gains the measured before-side cells (**62 976** fused / **97 280** split). **G-MARGINAL: `Δ_AB` is reported BEFORE G-NEUTRAL is interpreted** — `M_geo` is 41 % of the predicted width and the term that decides the row. **The `PRESHADE` anomaly clause extends from across-rung to between-arm on the same side**, forced by a finding this re-derivation was not looking for: the split row's arithmetic misses by **16 384 ns** (3.2 × `R`) while the fused row's closes to 3 ticks, and `NET = PRODUCE_RUN − PRESHADE` with `PRESHADE ≈ 304 µs` makes a 5.4 % between-arm drift worth ±16 µs. **DP6a's precondition is discharged** (gate content unchanged); OQ1's fallback gains its quantified trigger region (`GEO_base + M_geo + E_split_host > 40 448 ns`); §R4.1.7's deletion is re-pointed to **36 864 / 35 328** with the `M_geo` model marked as still a model. |
| **4.5 (DP6a review dispositions; two resolver changes, three doc corrections)** | **W3 — the cap conjunct TAKEN:** `vb_sv0_split` gains `&& vb_sdf_mesh_storage_ok` (hoisted, one spelling, read twice), on that binding ALONE and never on `sdf_soft_march`; the deciding number is +26 112 ns/frame (NET 27 648 → 53 760, **+94.4 %, 5.1 × `R_neutral`**) boot-frozen for a term that is never produced. Invariants **11** and **12** added; Decision 5's degrade chain gains `!rg8 ⇒ !vb_sv0_split ⇒ !split` at its head; red mutation **(7)** added and demonstrated; `sv0_arm_matrix` gains the matched-pair row *"VB x Both + term wanted, no RG8 storage"*. **O5 — the NORMAL union's antecedent SWITCHED** from `|| vb_sv0_split` to `|| mesh_geo_shade_split`: the obligation's own antecedent is the tightest discharging term, and the old spelling armed a producer-less channel on `VB × Sdf` (the 09600 class). `mesh_geo_shade_split ⇒ NORMAL` becomes by-construction and gains its CONVERSE as a shipped property. Composed, **`vb_sv0_split` ends with exactly one consumer.** **W2 — the cross-check's independence claim STRUCK**: the absolute reconstruction was the same calculation rearranged (the `24 576` cancels identically), replaced by the disjoint-input route through the split row, `[−6 940, +13 312]` point `−502`, whose **1 536 ns** disagreement with the primary is the deletion term measured two ways on two fixtures. **W4 — the boot-freeze boundary corrected at all three sites**: the deadline is `run_windowed`'s `:536`, which PRECEDES `app.finish()` at `:627`, so **a startup system is already too late**; "frame 100" understated it by the whole startup phase. **W5 — N3's `matches!` instruction STRUCK** as the inverse of its own stated purpose (adding `"host"` would set the request bits and destroy arm B) and its site count corrected from two to four. |
| **4.2 (as landed here)** | **P1-A:** `vb_viewt` DOES run on `[vb_both_ssao]` — the authoritative two-arm predicate is `gpu_scene/mod.rs:6550-6553`, arm (b) fires without `TaaConfig`; §R4.1.1's GEO row restored and quantified at 5 248 ns from `gpu_zone.rs:450-452`; the precondition is restated per side and gets a checked expectation cell. **P1-B:** the detector is release-live — `writes` is `#[cfg(debug_assertions)]` and stays so under its own name; two unconditional `u16` masks + `finish`'s three compares are the gate input. **P1-C:** the absence policy keys on absent-from-slice resolved against the leg's expectation, so a full-ring runtime absence skips instead of injecting a 4.5× inflated sample; `GpuPairBudgetExhausted` added to clause 5(2). **P2:** `Δ_host` baselines on DP6-0b's re-taken cell; `ZONE_VB_PRODUCE_NET` gets id 14, `VB_ZONE_COUNT` 15, one slot left, never stamped; the three particle ratios quoted with the tree's refusal of the band; `[order]` dispatches at `artifact.rs:848+`; the `atrous_levels` assertion closes the DP6-0b→DP6c window. |

---

> **Rev 3 delta** (closing the verify pass's N1..N6; everything else is Rev 2 verbatim):
> **N1 (P0), narrowed by the verify pass's N7** — `vb_sv0_host` gets its EXPRESSION:
> **`vb_sv0_host ≡ vb_sdf_mesh_armable() ∧ sdf_mesh_term_wanted`** — boot-frozen, carried on
> `ResolvedRenderPathGpu` as a second mirrored bool beside the existing `vb_sdf_mesh_armable`
> (`scene_types.rs:1514`, set at `gpu_scene/mod.rs:750` — the stated precedent; `sdf_mesh_term_wanted`
> is a `RenderPathConsumers` INPUT and needs this carrier to reach the declarator/recorder).
> Armable ALONE is too wide (N7): a VB×Both+SSAO boot with no SDF_MESH request resolves armable=true,
> and `host ≡ armable` would bind the +10 128 B sv0 variant AND declare a skipped `sdf_term` write on
> every such production frame — the Decision-1 dark tax paid unconditionally. With the conjunct:
> pure env host ⇒ term_wanted ⇒ vb_sv0_split ⇒ split ⇒ armable ⇒ host true, mode 0 (**arm B buildable**);
> production armed ⇒ term_wanted ∧ armable ⇒ host, mode ≠ 0 (**invariant 10 by construction, non-vacuous**);
> SSAO-only ⇒ term_wanted false ⇒ host false ⇒ `base` bound, no declared write (**invariant 6 restored**);
> `!rg8 ⇒ !armable ⇒ !host` (**Decision 5's degrade chain closes**). `armable`'s own definition is NOT
> narrowed (the conjunct lives in `host`, not in `mesh_geo_shade_split`), so `sv0_arm_matrix.rs BOOTS[1]`
> keeps `armable: true` — the P1-1 fixture table stands as written. The declared-write-but-skipped case is
> once again EXACTLY the measurement arm. One consequence folded in: `scene_types.rs:1512`'s claim that the
> armable mirror "emits no Vulkan command and cannot move a rendered byte" becomes false at DP6c (it is now
> a dispatch input) — corrected in the same commit.
> **N2 (P1)** — the `[vb_both_ssao]` FIXTURE is created at **DP6-0** (unpinned), so all four baseline cells
> are measurable before the producer moves; DP6c byte-pins it as planned.
> **N3** — seam corrected: `RenderPathConsumers` is built at `runner.rs:467` and resolved at `:509`; the env
> `host` read is a NEW site there (not "beside an existing read").
> ~~The only in-tree BOYKO_SDF_MESH reads are the two test/example hosts, whose `matches!` arms each gain
> `"host"` to keep the REQUEST bits false.~~
> **⚠️ STRUCK at the DP6a review (W5) — that instruction is the INVERSE of what it asks for, and it was
> implemented as its own negation.** `matches!(sdf_mesh.as_str(), "on" | "shadow")` already yields `false`
> for `"host"`; ADDING `"host"` to those arms would SET the request bits, which arms the term, resolves
> `mode != 0`, and destroys measurement arm B (whose entire definition is "host bound, mode 0"). The count
> is also wrong: there are **four** in-tree readers, not two —
> `crates/boyko_app/examples/vb_lab.rs`, `crates/boyko_app/tests/vb_both_sdf.rs`,
> `crates/boyko_app/tests/vb_both_ssao.rs`, `crates/boyko_app/tests/vb_sv0_produce_run_timing.rs`.
> **What DP6a actually landed, and what a future implementer must do:** leave every `matches!` pattern
> untouched, and add a comment at each of the four naming `"host"` as a fourth accepted value that
> deliberately arms neither request bit. The request bits stay false because the patterns do NOT list it.
> **N4** — red mutation (6) respecified: drop the `sdf_term` write access while **arm A** (mode ≠ 0) runs;
> the detector is `graph.rs:643-690`'s debug authoring guard on the tails' read of a non-seeded transient —
> NOT sync validation, which this box's record says cannot see it.
> **N5** — invariant 6 restated where it lives: structural absence holds on frames that are **disarmed and
> not host**; the host arm is the one deliberate exception (declared write, in-shader skip) and says so.
> **N6** — the DP7-alone estimate shown with its derivation: `D + (F+M_ded)/4 = 8 832 + 0.75·D ≈ 9.6 µs`
> (band 8.8–10.4), not "10–12"; and it OMITS `U` — a depth/normal-aware upsample in EVERY tail, plus the
> fact that a bilinear term read needs a sampler in `vb_layout0`, the layout shared by the whole VB family
> ⇒ a family-wide `.spv` re-bless. DP6d.5's deliverables now include pricing `U` and the sampler change.
> Omitting `U` made DP7 look cheaper, which STRENGTHENS the anti-reordering conclusion — error direction
> recorded. **Δ_AB gains a stated trigger**: if `Δ_AB > 2 × 6 144 ns` (the retired clause's own line), that
> is the numeric argument that "half-res is wanted" — DP6d.5's disposition is then owner-eval PLUS this
> number, not owner-eval alone.

> Supersedes Rev 2 in full. Companion research: [VB-SV0-DP6-RESEARCH.md](VB-SV0-DP6-RESEARCH.md).

> Supersedes Rev 1 in full. Companion research: [VB-SV0-DP6-RESEARCH.md](VB-SV0-DP6-RESEARCH.md).

# Answer to the critic's question 1, first, because it decides the rung

**P0-1 is correct, and my Rev 1 contradicted itself.** Decision 3's own table said "≈0 to −8 µs" on the measured boot; the Goal and the metrics table headlined "35 328 → ≤ 12 288 ns". Both cannot be true. The table was right.

The fixture DP4 measured (`vb_both_sdf`, verified: `insert_resource` at `:95` and `:102` only — no `SsaoConfig`, no `DdgiConfig`) arms no pre-light consumer, so `mesh_geo_shade_split == false` and DP4's 35 328 ns is a **fused-boot** number. Counting full-screen `vb_geom_fetch` walks and dispatches per armed frame:

| boot class | today | after DP6 | Δ fetches | Δ dispatches |
|---|---|---|---|---|
| **fused** (`vb_both_sdf` — the measured one) | `vb_resolve` + `sdf_mesh_shadow` = **2 / 2** | `vb_geo` + `vb_shade_split` = **2 / 2** | **0** | **0** |
| **already-split** (VB×Both + SSAO/DDGI/temporal) | `vb_geo` + `sdf_mesh_shadow` + `vb_shade_split` = **3 / 3** | `vb_geo` + `vb_shade_split` = **2 / 2** | **−1** | **−1** |

On the measured boot DP6 **relocates** the fetch into a dispatch that boot newly runs. The `D + F ∈ [20 939, 29 184] ns` deletion is real **only on already-split boots**. Rev 1's headline was DP4's category error re-run in the opposite direction, and it is withdrawn.

**Decision: option (a), reframed as PRODUCER CONSOLIDATION, plus the P0-4 remedy from option (c) as a gate rather than a reordering.**

Why not (b) — keep the amortization frame, restrict claim + DP6e to split boots: that leaves **two producers shipped forever** (dedicated pass for fused, geo half for split), which is the two-code-paths/two-zone-stories/two-cost-models outcome the rung exists to avoid, and it makes the feature's cost a function of an unrelated consumer's arming. Rejected.

Why not (c) — measure DP7 (half-res) first and let it decide: the numbers do not separate them. In the **dedicated pass** half-res quarters *both* F and M (one thread per term texel), so DP7-alone ≈ `D + (F + M_ded)/4` ≈ **10–12 µs**. DP6-alone leaves SV0's marginal cost at `M_geo ∈ [6.1, 14.4] µs`. **Same band.** A full reordering is not justified by a tie. Two things break it: DP6 additionally buys consolidation (−1 shader, −1 `.spv`, −1 pipeline, −1 layout, −1 descriptor ring, −1 zone id, −1 `spv_sync` test, −75 lines of `record_vb`) which DP7 buys none of; and DP7 carries a quality cost (half-res soft shadow + contact AO crawl on silhouettes without an edge-aware upsample) which DP6 does not.

But **P0-4 stands and is not answered by that tie.** DP6 and DP7 are *partially antagonistic*: `vb_geo` is one thread per full-res `vb_id` pixel (`vb_geo.comp.hlsl:214-236`), so a half-res term in the geo host needs a second dispatch — which reintroduces `D` and converges back on DP7-alone. Retiring the dedicated pass therefore removes the only host shape DP7 can use. **Remedy: a DP7 FEASIBILITY PROBE (rung DP6d.5) runs in the dedicated pass while it still exists, and DP6e's gate carries its explicit disposition.** Probe it; do not build it. That converts the one-way door into a door with a measurement in front of it, at a fraction of a reordering's cost.

**Consequence for the gates: the 2× cost clause is NOT this rung's justification and is dropped as such.** A consolidation is justified by maintenance surface plus the split-boot win. It is replaced by the gate text below — **Rev 4, superseding Rev 3's three bullets in full** (Rev 3 stated G-NEUTRAL/G-REDUCE over `T(armed)` with the split side read as `ZONE_VB_GEO + ZONE_VB_SHADE`; §R4.1.8 shows that sum is a `Σ(median_f)` the reducer forbids, and §R4.1.3 shows the TOP latch makes it unadjudicable either way):

> **All gates read ONE quantity: `ZONE_VB_PRODUCE_NET = ZONE_VB_PRODUCE_RUN − ZONE_VB_PRESHADE`**, formed inside `WindowReducer::observe_frame` from each frame's own pairs and reduced afterwards — `median_f(Σ)`, never `Σ(median_f)`. `ZONE_VB_PRODUCE_RUN` is reported beside it as the total. Protocol: release, 512×512, `sv0_scene`, **3 identical legs per side**, warmup discarded.
>
> **G-NEUTRAL (fused boots).** `[vb_both_sdf]`, `BOYKO_SDF_MESH=on` (arm A), fixture otherwise unchanged:
> `median_f(NET)|after DP6c ≤ median_f(NET)|at DP6-0b + R_neutral`.
> `PRESHADE` is absent-`Forbidden` before and `≈ 0` after, so `NET ≡ PRODUCE_RUN` on this row. **The two sides differ in leg class deliberately** — the fused→split flip is exactly the trade Decision 3 makes and the sole thing G-NEUTRAL prices.
>
> **G-REDUCE (split boots).** `[vb_both_ssao]`, same arm:
> `median_f(NET)|after DP6c < median_f(NET)|at DP6-0b − R_reduce`.
>
> **`R` is certified per row, per fixture, per session, and inherited from nothing** — not DP6-0's 4 608 ns (void: the instrument changed under it), not DP2's 24 576, not DP4's 1 024.
>
> **Reported, not gating:** `Δ median(PRESHADE)`, **in two directions — across the rung AND between arms on the same side.** DP6 emits no SSAO, à-trous, DDGI or hwrt command; a movement beyond `R_preshade` means the workload changed and the run is re-taken with the cause stated, and a **between-arm** movement beyond `R_preshade` is a stated-cause anomaly exactly like the across-rung one.
>
> **Why the between-arm direction was added (Rev 4.4), with the finding that forced it.** The split row's arithmetic does not close: `NET(on) − NET(off) = 23 552` on `[vb_both_ssao]` against `36 864` (id 10) `+ (16 384 − 13 312)` (GEO) `= 39 936` of measured per-zone armed change — a **16 384 ns** shortfall, 32 ticks, **3.2 × `R`, so not rounding** — while the fused row's closes to 1 536 ns (3 ticks, below `R`). The most likely carrier is structural and was unpriced: `NET = PRODUCE_RUN − PRESHADE` and `PRESHADE ≈ 304 µs`, so a **5.4 % drift between arms injects ±16 µs into `NET`** — inside `PRESHADE`'s own spread and invisible to a clause that only compares it across the rung. This does **not** change G-REDUCE's bar; it makes the subtraction's sensitivity visible. It does not touch the fused row, where `PRESHADE ≈ 0` on both sides — which is incidentally why the fused arithmetic closed and the split one did not.
>
> **G-MARGINAL (informational).** `Δ_AB` on `ZONE_VB_GEO`, now BOTTOM-stamped and therefore an exact difference of prefix-completion times. Reported against 6 144 ns with its ratio; **not adjudicated** — the rung no longer claims it. **`Δ_AB` stays informational and non-adjudicated, but DP6d reports it BEFORE interpreting G-NEUTRAL** — `M_geo` is 41 % of the fused row's predicted width and the term that decides it, so a red read without it will be attributed to "consolidation costs" when the decomposition may say "the march costs, and it would have cost in either host."
>
> **Clause 5, four clauses. A row failing any is INCONCLUSIVE — never PASS, never FAIL:**
> 1. either side's 3-leg relative spread of the gated median > 10 %;
> 2. any gated zone not `Measured` on any leg; or `lost != 0` / `torn != 0`; or the **`GpuZoneUnmatchedEnd`** flag raised; or the **`GpuPairBudgetExhausted`** flag raised;
> 3. `OrderCensus.violations != 0`, **or `OrderCensus.frames_checked == 0`** (a zero over zero is not a pass);
> 4. the derived row's `n < 0.9 × frames_checked`; **or** the two sides differ in occlusion-split arming; **or** the per-leg expectation table does not match the artifact exactly.
>
> **Lattice-snap rule.** A threshold derived as a fraction of a measured bound is **snapped up to the next multiple of the device timer step (512 ns on this box)**, with the arithmetic shown: half of `20 939` is `10 469.5` → **`10 752 = 21 × 512`**.

---

# Changelog — Rev 1 → Rev 2

| Finding | Closure |
|---|---|
| **P0-1** baseline / relocation | Rung reframed to consolidation. Per-boot-class cost table added (above and §Metrics). Headline "35 328 → ≤ 12 288" **withdrawn**. 2× clause demoted from gate to informational. Gates replaced by G-NEUTRAL / G-REDUCE. |
| **P0-2** bracket spans nothing | **Verified: `ZONE_VB_RUN` ends `vb.rs:3016`; `vb_geo` records `:3752`; split shade `:4277`.** Comparator respecified: split pair = **`ZONE_VB_GEO + ZONE_VB_SHADE` (a SUM of two disjoint intervals, not one span)**; fused comparator = **`ZONE_VB_SHADE` alone** (it is *defined* to bracket whichever of the three producers runs — `vb.rs:3388-3398`), plus `ZONE_VB_SDF_MESH` on today's armed frames. New rung **DP6-0** mints the zone **before** the producer moves, so baselines are paired, not remembered. **⚠️ Rev 4: the two comparators in this row are WITHDRAWN** (§R4.1.8, §R4.1.3) — read §R4-D2/§R4-D4 for the one that replaced them. The DP6-0 rung and the paired-baseline principle stand. |
| **P0-3** arm B unbuildable | ONE predicate `GBufferScene::vb_sv0_host` drives **both** the pipeline pick and the `sdf_term` access declaration. The shader's store is **moved inside `if (sv0_mode != 0u)`** — wave-uniform, behaviourally identical on every mode≠0 frame. Arm B's write is declared (safe over-declaration) *and* skipped in-shader. Both halves stated in §Decision 6. |
| **P0-4** one-way door | New rung **DP6d.5 — DP7 feasibility probe** in the dedicated pass. **DP6e's gate requires an explicit recorded DP7 disposition.** Antagonism (`vb_geo` = one thread per full-res pixel) stated as a first-class constraint. |
| **P1-1** truth tables turned | Enumerated in §Integration: `render_path_config.rs::sv0_never_arms_under_hwrt` (:2755) and `sv0_armable_only_on_vb_with_both_legs`; `sv0_arm_matrix.rs` `BOOTS[0]` (`:96-105`, `armable:true` → `false`, `why` rewritten) and `BOOTS[1]`'s `why` (its stated reason "armable exactly like the fused one" becomes false); `BOOTS` gains a fused+term-wanted row. |
| **P1-2** Rev-5 erratum | §Decision 4 records it verbatim and argues the W4 class cannot re-open (SV0 is VB-only, consumes no thin-aux channel, and *produces* rather than consumes). |
| **P1-3 / Q3** boot-committed + config surface | **`LightingConfig::vb_sdf_mesh_host` DELETED from the design.** The host arm is env-only (`BOYKO_SDF_MESH=host`) at the boot seam, like every sibling knob. Boot-frozen contract + frame-100 behaviour + `runner.rs` named + `vb_sdf_mesh_armable`'s "SINGLE capability predicate" doc correction, all in §Decision 4. |
| **P1-4** `register(t0)` | **Verified: `BUF_T0_DECL = "StructuredBuffer<uint> Buf : register(t0)"` (`:642`) and `find_decl_line` PANICS (`:567-574`).** `register(t12)` withdrawn — the span uses `register(t0)`, matching `sdf_mesh_shadow.comp.hlsl:96`. Only the **vk::binding** assertion is re-pointed. |
| **P1-5** `-P` gate is new work | Owned: new file `crates/boyko_rhi_vulkan/tests/vb_geo_preprocess_sync.rs`, landing at DP6b. Hash is **recomputed via `git show`**, not committed — rationale in §Validation. |
| **P1-6** @3 storage degrade | **Partially refuted.** `sdf_mesh_shadow_set0` **already carries the cap gate** — `targets.rs:4271-4279` conjoins `ctx.device_caps().rg8_unorm_storage_ok` with the VUID named in a comment. **No shipped hole.** What *is* real is doc-rot at `:639-643` ("built on every VB boot" omits the conjunct) — flagged for repair, not a fix. My own @3 hole is real and closed: placeholder-bind to `thin_normal[i]` on `!rg8_ok`, the shipped R9d idiom. |
| **P2-1** missing include | `#include "light_table.hlsli"` added, ordered before `sdf_field.hlsli` as in the shipped consumer. |
| **P2-2** pick is cfg-split | **Verified `vb.rs:3761-3774`** — two `cfg`-split bindings. Snippet corrected to extend both arms. |
| **P2-3** wrong doc-rot target | Corrected: the array doc (`:698-709`) is **current**; the stale text is the test-fn doc `:523` and the assertion message `:545-548`. |
| **P2-4** leg table | New split+SV0 row **and** the "armed" ambiguity resolved (occlusion-split vs geo/shade split get distinct words) in the same edit. |
| **Q4** | Answered: the dedicated pass becomes **unreachable at DP6c**, not at DP6e. Revert story restated accordingly. |
| **Q5** | Answered constructively: no pin covers split+SV0 today (`[vb_mesh_ssao]` is VB×**Mesh** — no SDF leg — so it can never arm SV0). DP6c **adds** `[vb_both_ssao]` (split, SV0 disarmed) and seeds `[vb_both_ssao_sv0]` PENDING. |
| **Preserve list** | All ten items byte-stable. Fetch arithmetic, zone story (11→12, hole at 10, auto-TopOfPipe via `zone_begin_stage`'s range exclusion — **⚠️ Rev 4 supersedes the stage half: 12→15, and ids 10/11 restamp to BOTTOM by name, §R4-D1**), `gSdfTerm` double-role non-hazard, 2+1 tail reads, Decision 2's emptiness proof, span fidelity, `sdf_field.hlsli`'s Buf-only need, and Decision 6's paired-delta instrument all carried unchanged except where a P0 forced a stated edit. |

---

# Architecture: VB-SV0 DP6 — producer consolidation into the split path's geometry half

**Status:** DESIGN Rev 4 (Rev 3 was critic-converged: 'fix N7's one conjunct and this is APPROVED' — the conjunct is fixed above; Rev 4 adjudicates OQ1 on DP6-0's measurement and inserts the DP6-0b repair rung, architect↔critic converged over three rounds).
**Parent:** `docs/VB-SV0-SDF-SHADOW-PLAN.md` Rev 10, DP4 adjudication block.

## Goal

Collapse SV0's **two** possible producers into **one**, hosted in `vb_geo.comp.hlsl` — the split path's geometry half, which already performs the per-covered-pixel `vb_geom_fetch`.

- **Maintenance:** −1 shader source, −1 committed `.spv`, −1 pipeline, −1 Set-0 layout, −1 per-FIF descriptor ring, −1 zone id, −1 `spv_sync` test, −75 lines of `record_vb`, −1 declared pass. One producer, one code path, one zone story, one cost model.
- **Performance, stated per boot class** (never as one headline):
  - already-split boots: **−1 fetch, −1 dispatch** = `D + F ∈ [20 939, 29 184] ns` deleted.
  - fused boots: ~~**cost-neutral by construction** (2/2 → 2/2), gated as such.~~ — **WITHDRAWN at
    Rev 4.4: a construction claim refuted by measurement.** The fetch/dispatch counts are still
    2/2 → 2/2, but the boot changes **leg class** and pays `E_split_host` (measured 11 264 ns at
    DP6-0b), so neutrality is a **measured question, not a construction**. What replaces it:
    **predicted `Δ NET ∈ [−5 404, +14 848] ns`, point estimate `+1 034`**, against a `+5 120` bar
    (`R_neutral` as certified at DP6-0b; DP6d certifies its own) — see Decision 3's re-derivation.
    **Neutrality is claimed by measurement at DP6d, not by counting.**
- **Not a goal:** reducing SV0's marginal armed cost below the inherited 2× threshold. That is DP7's job, and DP6 must not foreclose it.

## Context and constraints

Invariants 1–7 from Rev 1 are carried unchanged, plus:

8. **`vb_geo`'s thread↔pixel mapping stays one-per-full-res-`vb_id`-pixel.** DP7's antagonism follows from it; any rung that changes it re-opens DP6e's disposition.
9. **`vb_sv0_host ⇒` (pipeline pick == sv0) `∧` (`sdf_term` write declared).** One predicate, two consumers — the O1 discipline.
10. **`vb_sdf_mesh_mode != 0 ⇒ vb_sv0_host`.** Host is the weaker predicate.
11. **`vb_sv0_split ⇒ vb_sdf_mesh_storage_ok`** (DP6a review, W3). The term's request cannot arm the split on a device that cannot host the `sdf_term` ring — otherwise the boot pays the consolidation's debits with its credit structurally zero.
12. **`vb_sv0_split ∧ mesh_leg ⇒ vb_sdf_mesh_armable()`** (DP6a review, W3). The converse of 11 on the rows that matter: once the split is armed BY the term on a mesh-carrying leg, every remaining conjunct of `armable` is already implied, so the rung can never arm the split for a term it then refuses to deliver.

## Key decisions

Decisions 1, 2, 5, 7, 8 are carried from Rev 1 with the edits noted below. Decisions 3, 4, 6 are rewritten.

### Decision 1 (carried) — `-D VB_SV0_TERM=1` variant, not an unconditional runtime-gated span
Unchanged and unchallenged. `+10 128 B` on a `15 888 B` kernel is **+64 %** instruction footprint on the smallest kernel in the family, measured on this exact march at `13f1c9a3` (+75 % on `vb_resolve`). **P2-1 edit:** the guarded span now includes `#include "light_table.hlsli"` before `sdf_field.hlsli`. **P1-4 edit:** `Buf` keeps `register(t0)`.

### Decision 2 (carried) — exactly ONE new variant; `MOTION × VB_SV0_TERM` is provably empty
Unchanged; on the critic's preserve list. **P2-2 edit:** the record-site `debug_assert!` lands in the `#[cfg(feature = "hwrt")]` arm only (the `not(hwrt)` arm has no `vb_geo_mv_active()` to assert against, and the cross is vacuous there).

### Decision 3 (REWRITTEN) — consolidation: SV0 arming requires the split, and the dedicated pass is retired **at DP6c**, deleted at DP6e

**What.** `vb_sdf_mesh_armable()` gains `&& self.mesh_geo_shade_split`. From DP6c the dedicated pass is **unreachable** (an SV0-armed boot *is* split, so `mesh_leg && mode != 0` can no longer coexist with `path_vb_fused()`); DP6e deletes the now-dead code.

**Why (answering Q4 and the revert story).** Rev 1 claimed the two producers "coexist for exactly one rung". They do not — they are mutually exclusive from the moment the conjunct lands. So the honest revert story is: **DP6c is the semantic point of no return; DP6e is bookkeeping.** The revert target for a DP6d failure is therefore `DP6c^`, not `DP6e^`. DP6e's separation from DP6c buys one thing only: the dead code remains *in the tree* while DP6d.5's probe runs, so the probe has a host. That is its whole justification and it is now stated as such.

**Why consolidation and not two producers.** The surviving second producer would be the one that **failed its inherited cost clause at 5.75× and does not claim it**. Shipping it as the fused-boot path means the feature's cost jumps 2–3× depending on whether an unrelated consumer (SSAO) happens to be armed — a cliff the owner cannot predict from the config. Principle 10.

**Trade-off, priced.** A VB×Both boot wanting SV0 and nothing else now allocates `thin_normal`, runs `vb_geo`, and shades through `vb_shade_split`. **Priced at Rev 4.4 with `E_split_host` measured:** the four terms that do not cancel are tabulated once, in the RE-DERIVED sub-block immediately below, and they give **`Δ NET_fused ∈ [−5 404, +14 848] ns`, point estimate `+1 034 ns`**, against G-NEUTRAL's `+5 120` bar. **G-NEUTRAL remains the only reason this trade is acceptable — and the prediction does not exclude a red: the interval straddles the bar in the upper corner where `E_split_host` and `GEO_base` are both pessimistic.**

#### Trade-off, RE-DERIVED at DP6-0b (§R4.3.7's block, discharged)

**Release condition, stated so the block reads as discharged rather than forgotten.** §R4.3.7's
middle branch blocked DP6a until this row absorbed `E_split_host`. This sub-block **is** that
absorption: the block is lifted and DP6a may land. It is an arithmetic update to one decision; no
design changes.

**§0 — a premise correction first, because it changes which terms are measured.**

> **`ZONE_VB_GEO`'s two DP6-0b arms (13 312 off / 16 384 on) do NOT bound `M_geo`.**

At DP6-0b the producer has **not moved** — the march is still in the dedicated pass. `vb_geo` runs
the **same base variant on both arms**, so the two cells differ only by whatever arming perturbs
around it. Their Δ is **3 072 ns, below `R_neutral` = 5 120**, i.e. indistinguishable from zero — the
same shape as DP6-0's GEO Δ of −4 096 below its 4 608. **`M_geo` is therefore unmeasured and stays
the design's model `[6 100, 14 400]`.** It is first measured at DP6d, as `Δ_AB` on the
now-BOTTOM-stamped `ZONE_VB_GEO` — which is exactly G-MARGINAL's own quantity, whence the
gate-ordering consequence recorded in G-MARGINAL's clause.

What the two GEO arms *do* give is **`GEO_base` — the cost of `vb_geo` itself, `[13 312, 16 384]`** —
and after the restamp that number is **transferable across boots**, because a BOTTOM begin no longer
admits the predecessor drain that made DP6-0's GEO cell fixture-specific. That transferability is
what the restamp bought, and this re-derivation is its first consumer.

**The four terms.** Before-side, MEASURED, no modelling: `NET([vb_both_sdf], arm A) = 62 976 ns` at
DP6-0b. Four terms do not cancel; everything else does. `id 6`'s unsplit hzb slot runs on both sides
(same fixture, same occlusion arming); `vb_viewt` is `Forbidden` on both sides of this fixture
(§R4.3.6's two-arm derivation); `PRESHADE` is absent-`Forbidden` before and `≈ 0` after.

| # | term | value | provenance |
|---|---|---|---|
| 1 | dedicated pass deleted | **−35 328** | **MEASURED**, same-fixture paired: `NET(on) − NET(off) = 62 976 − 27 648`. Corroborated by the id-10 zone median (36 864) to **1 536 ns = 3 ticks**, and by DP4's independently published pass median of **35 328** |
| 2 | `vb_geo` newly runs (`GEO_base`) | **+13 312 … +16 384** | **MEASURED** on `[vb_both_ssao]`, transferable because id 11 is now BOTTOM-stamped. Includes the `thin_normal` write and the NORMAL union |
| 3 | march moves into `vb_geo` (`M_geo`) | **+6 100 … +14 400** | **MODEL** — the design's own band. **Unmeasured; §0 above says why the DP6-0b cells cannot bound it** |
| 4 | shade fused→split (`E_split_host`) | **+10 512 … +19 392**, median **11 264** | **MEASURED** cross-fixture (`shade_derived` 35 840 on the split fixture − re-taken fused shade 24 576), band from the 3×3 leg cross-product |

**Prediction:** `Δ NET_fused ∈ [−5 404, +14 848] ns`, **point estimate `+1 034 ns`**, taken at term
medians (`GEO_base` 14 848, `M_geo` 10 250, `E_split_host` 11 264). **Where the width comes from:**
`E_split_host` **43.8 %** (8 880), `M_geo` **41.0 %** (8 300), `GEO_base` **15.2 %** (3 072).

**Cross-check — corrected at the DP6a review (W2).**
~~Reconstructing the after-side absolutely — `3 072` (common frame, = 27 648 − 24 576) `+ GEO_base +
M_geo + 24 576 + E_split_host` — reproduces the same interval to the nanosecond, so the two routes
are not one calculation written twice.~~ **WITHDRAWN: that route is the same calculation
rearranged.** The `24 576` enters once positively and once inside the `3 072`, so it cancels
identically; a slip in any input reproduces itself down both routes, and "agrees to the nanosecond"
is what an algebraic identity does, not what a corroboration does.

**Replaced by a route with DISJOINT inputs.** The fused row can be reached from the *split* row,
which shares no measured cell with the primary decomposition except by prediction:

`Δ NET_fused = Δ NET_split + GEO_base + E_split_host`

— because the split side already runs `vb_geo` and already hosts the tail in `vb_shade_split`, so
those two terms are exactly what the fused side additionally pays. Feeding it the split row's own
`Δ NET_split = −36 864 + M_geo` (whose deletion term is the **id-10 zone median on
`[vb_both_ssao]`**, a different fixture and a different statistic from the primary route's
same-fixture paired `35 328`) gives **`[−6 940, +13 312]`, point estimate `−502`** against the
primary's `[−5 404, +14 848]`, point `+1 034`. **The two disagree by 1 536 ns — 3 ticks — which is
the deletion term measured two ways on two fixtures**, and that gap is the corroboration: it is a
measurement difference, not an identity.

Two caveats carry: the route is only as good as `M_geo`, which is still a model on both sides; and
**§3.3's 16 384 ns split-row shortfall must be dispositioned at DP6d before this check is read**,
since it is the split row's own arithmetic that the second route consumes.

**Verdict: the point estimate clears the bar with 4 086 ns of margin; the interval straddles it.**
Bar `Δ ≤ +R_neutral = +5 120` (certified at DP6-0b; DP6d certifies its own and inherits nothing).
Point estimate **+1 034** clears. Upper end **+14 848** exceeds by 9 728. Lower end **−5 404** is a
net *improvement* on a fused boot, which the rung never claimed.

**What has to happen for it to red, stated as a region.** Red requires
`GEO_base + M_geo + E_split_host > 40 448`.

| holding the other two at | `M_geo` ceiling | share of the model band that clears |
|---|---|---|
| medians (14 848 / 11 264) | **14 336** | **99.2 %** — only the top 64 ns of `[6 100, 14 400]` fails |
| pessimistic (16 384 / 19 392) | 4 672 | 0 % — reds across the whole band |
| optimistic (13 312 / 10 512) | 16 624 | 100 % |

**So the red region is real but confined to the upper corner of the joint space:** it requires
`E_split_host` and `GEO_base` to land at their pessimistic ends **together**, and `M_geo` near its
model top. At median inputs the bar sits essentially exactly at the top of the `M_geo` model band — a
coincidence worth naming, because it means the fused row's verdict is decided almost entirely by
term 3.

**Disposition — what DP6d's measurement decides.** **`M_geo`, and it is the one term that is a model
rather than a measurement.** DP6d measures it directly as `Δ_AB` on `ZONE_VB_GEO` — arm A minus arm B
on one boot, one stream, both BOTTOM-stamped, so it is an exact difference of prefix-completion
times. It also re-measures `E_split_host` on the after-side (where `shade_derived` is read on
`[vb_both_sdf]` itself rather than transferred), collapsing term 4's cross-fixture provenance.

> **Predicted disposition:** G-NEUTRAL passes unless `M_geo` lands above
> `40 448 − GEO_base − E_split_host` as those two are then measured. If it reds, OQ1's pre-agreed
> fallback applies unchanged — restrict DP6e to split boots, keep the dedicated pass for fused — and
> it now has a **quantified** trigger region instead of a bare contingency.

**One distinction, recorded because both quantities are now live and both in nanoseconds on the same
row.** The inherited 2× reference (6 144 ns) is about **the term's marginal cost inside an
already-fetching host** — that is `M_geo`, i.e. `Δ_AB`. **`E_split_host` is a host-change cost, a
different quantity, and must not be folded into `Δ_AB`.**

### Decision 4 (REWRITTEN) — env-only host arm; boot-frozen contract stated; Rev-5 erratum recorded

**What.**
```rust
// RenderPathConsumers — the ONLY new field. DEFAULT false.
pub sdf_mesh_term_wanted: bool,

// resolve_rules — SDF_SOFT_MARCH hoisted so it has exactly ONE spelling
let sdf_soft_march = sdf_leg && consumers.sdf_shadows_wanted && !consumers.hwrt_denoise_or_vis_on;
let vb_sv0_split   = matches!(path, RenderPath::VisibilityBuffer)
                  && consumers.sdf_mesh_term_wanted && sdf_soft_march;
let mesh_geo_shade_split = matches!(path, RenderPath::VisibilityBuffer)
                        && mesh_leg && (pre_light || vb_sv0_split);
// NORMAL union gains `|| mesh_geo_shade_split`  (see the O5 amendment below)
// later, unchanged in effect: if sdf_soft_march { shadow.insert(SDF_SOFT_MARCH) }
```

> **Amendment, DP6a review (W3) — the cap conjunct.** `vb_sv0_split` gains
> `&& vb_sdf_mesh_storage_ok`, hoisted as `let vb_sdf_mesh_storage_ok = caps.rg8_unorm_storage;`
> so the RG8 fact has ONE spelling and the `vb_sdf_mesh_storage_ok` FIELD reads the same binding:
> ```rust
> let vb_sdf_mesh_storage_ok = caps.rg8_unorm_storage;
> let vb_sv0_split = matches!(path, RenderPath::VisibilityBuffer)
>                 && consumers.sdf_mesh_term_wanted && sdf_soft_march && vb_sdf_mesh_storage_ok;
> ```
> **SCOPE WARNING: the conjunct goes on `vb_sv0_split` ALONE, never on `sdf_soft_march`.** The
> inline soft march writes no storage image and needs no RG8; moving the cap up one line would
> disarm SDF soft shadows on every non-RG8 device.
>
> **Deciding argument.** On a non-RG8 device there is no dedicated pass to delete, so the
> consolidation's credit is **zero** and its debits stand alone: `GEO_base + E_split_host` =
> **+26 112 ns/frame** at medians, taking `NET` from 27 648 to **53 760 — +94.4 %, 5.1 ×
> `R_neutral`** — boot-frozen for the process, in exchange for a term that is never produced.
>
> **Amendment, DP6a review (O5) — the NORMAL union's antecedent.** The union reads
> `|| mesh_geo_shade_split`, **not** `|| vb_sv0_split`. The obligation is `mesh_geo_shade_split ⇒
> NORMAL`, so the tightest discharging term is the antecedent itself and the implication becomes
> true by construction. `vb_sv0_split` carries no `mesh_leg` conjunct, so it is true on a
> `VB × Sdf` boot where the split is false — arming a NORMAL channel on a leg that runs no
> `vb_geo` to write it, which is the bound-but-never-written 09600 class the MOTION arm refuses by
> name. Composed with the W3 amendment, **`vb_sv0_split` ends with exactly one consumer.**
`sdf_mesh_term_wanted` is set at the **`boyko_app::gpu_scene` boot seam** from `LightingConfig::vb_sdf_mesh_shadow || ::vb_sdf_mesh_ao`, OR'd with an **env-only** host flag (`BOYKO_SDF_MESH == "host"`) read at that same seam. **No new `LightingConfig` field.**

**Q3 answered.** Rev 1 put a measurement knob on a production `Resource`; every sibling knob (`BOYKO_VB_ZONE`, `BOYKO_SDF_MESH=on|shadow|ao`, `BOYKO_AA`, `BOYKO_SSAO`, `BOYKO_CSM_OFF`) is env-gated. Nothing kept a shipped title from setting the field. Now nothing *can*: there is no field. The env read lives beside the existing `BOYKO_SDF_MESH` read, so it adds one match arm, not a plumbing hop.

**P1-3 — the boot-frozen contract, stated.** `resolve_render_path` runs **once**, at boot (`render_path_config.rs:1374-1376`); `LightingConfig` requests are re-asserted **every frame** (`light.rs:2231-2233`). Therefore:
> **Contract.** `sdf_mesh_term_wanted` is a **boot snapshot of the request**. A boot that does not carry `vb_sdf_mesh_shadow`/`_ao` at resolve time gets `mesh_geo_shade_split == false`, `vb_sdf_mesh_armable() == false`, and `sync_sv0_light_gate` clamps the request to 0 **for the process lifetime**, reported once by the cold latch. To arm SV0 the request must be present **before the first `resolve_render_path` call**.
>
> ⚠️ **Where that line actually falls, corrected at the DP6a review (W4).** This clause used to illustrate with "a world that arms the request at frame 100", which reads as a warning about late dynamic toggling and understates the deadline by the entire startup phase. **`run_windowed` takes the snapshot at `runner.rs:536` and calls `app.finish()` at `:627`** — no user system of any kind has run at `:536`. **A STARTUP SYSTEM that sets the request is therefore ALREADY TOO LATE**, and its only symptom is one `eprintln!` from the clamp's cold latch. The request must be `insert_resource`d onto the `App` **before `run_windowed` is entered**. `clusters_wanted` sits on the same boundary and carries the same trap.

This is the **same** contract `ssao_on` already carries (`SsaoConfig::enabled` is a boot read; a late SSAO enable is a no-op under VB), so it introduces no new class — but it was previously true only of *capabilities*, and it is now true of a *request*. Two consequences, both owned:
- **`runner.rs` is an affected file** (it publishes the request into the boot seam) and gains the contract comment.
- **`vb_sdf_mesh_armable()`'s doc is factually wrong after this rung.** "The SINGLE capability predicate every SV0 consumer reads" must become: *"the single ARMABILITY predicate — a capability conjunction plus, through `mesh_geo_shade_split`, a boot-frozen snapshot of the owner's request. It is no longer purely a statement about the device and the path."* Load-bearing for the arm matrix; corrected in the same commit.

**P1-2 — the Rev-5 erratum, recorded.**
> **Erratum (DP6) to Rev 5's MANDATORY single-predicate rule** (`render_path_config.rs:74-79`, `:985-987`, `:1009-1010`). Rev 5 says one union `pre_light` is the **sole** trigger for three flags. After DP6 that holds for **two** of them — `needs_depth_prepass` (Forward) and `sdf_geo_shade_split` (the SDF leg) still read `pre_light` alone. The **VB** flag reads `pre_light ∨ vb_sv0_split`. The rule is restated as: *one predicate for the Forward and SDF flags; the VB flag is that predicate OR the VB-only SV0 term.*

**Why the W4 hole class cannot re-open.** W4 was: *a MOTION-only pre-light consumer under Forward reads frame-stale motion because the prepass was not armed* — a **consumer** left without its **producer**. `vb_sv0_split` is the opposite shape: it conjoins `path == VisibilityBuffer` (so it can never reach `needs_depth_prepass`), it consumes **no** thin-aux channel, and it exists precisely to arm a **producer** (`vb_geo_sv0`) for an image it writes itself. There is no consumer it can leave unfed. The NORMAL union's new term is likewise producer-side: it keeps `split ⇒ NORMAL` true so `vb_geo`'s unconditional `thin_normal` write and the mask agree (R9b's own stated reason: "the mask must stay the single truth"). ~~That term is `|| vb_sv0_split`.~~ — **corrected at the DP6a review (O5): it is `|| mesh_geo_shade_split`**, the obligation's own antecedent; the `vb_sv0_split` spelling armed the channel on `VB × Sdf`, where the split is false and no `vb_geo` runs.

**`split ⇒ NORMAL` cost, unchanged from Rev 1:** one `oct_encode` (~10 ALU) + two already-warm loads + one RGBA8 store, against 128 march iterations × N-edit field walks plus 5 AO taps. **<1 %.** Buying that back would cost an invariant, a 4th variant, and a test.

### Decision 5 (carried, P1-6 edit) — bindings, with the storage degrade closed

`vb_geo_aux_layout` 3 → 5 bindings, declared unconditionally (one layout object for all three pipelines):

| slot | reg | resource | base | `sv0` |
|---|---|---|---|---|
| @0 | u8 | `gThinNormal` RGBA8 | WRITE | WRITE |
| @1 | u9 | `gMotion` RG16F | `#if MOTION` | unread |
| @2 | b10 | `MotionCam` UBO | `#if MOTION` | unread |
| **@3** | **u11** | **`gSdfTerm` RWTexture2D\<float2\> rg8** | unread | **WRITE** |
| **@4** | **t0** | **`Buf`** (edit list) — **`register(t0)`, P1-4** | unread | READ |

**P1-6 degrade, stated.** `vb_geo_aux_set` is built on **every split boot** (`targets.rs:347-351`), including SSAO boots on devices without `rg8_unorm_storage_ok` — where the `sdf_term` ring is created **SAMPLED-only** (`:1144-1155`). A `StorageImage` entry over it would violate `VUID-VkWriteDescriptorSet-descriptorType-00339` at update time.
> **Degrade: placeholder-bind @3 to `thin_normal[i]` when `!ctx.device_caps().rg8_unorm_storage_ok`.** Same descriptor type (`STORAGE_IMAGE`), `thin_normal` always carries STORAGE usage, and this is the shipped R9d idiom verbatim (`vb_geo_aux_set`'s @1 motion slot is already placeholder-bound to `thin_normal[i]`, `targets.rs:347-351`). Provably inert, and the chain gained a new HEAD at the DP6a review (W3): **`!rg8_ok ⇒ !vb_sv0_split ⇒ !mesh_geo_shade_split`** (unless some other pre-light consumer arms it) `⇒ !vb_sdf_mesh_armable ⇒ mode 0 ⇒ !vb_sv0_host ⇒ `sv0` module never bound ⇒ @3 never referenced by any executing module`. The new head is what keeps a non-RG8 boot from paying `vb_geo` + `vb_shade_split` for a term it can never produce. @4 needs no degrade — `edit_list` is always a valid `StorageBuffer`.

**Refutation of the critic's parenthetical, with the citation.** The shipped `sdf_mesh_shadow_set0` does **not** carry this hole. `targets.rs:4271-4279` gates its construction on `ctx.device_caps().rg8_unorm_storage_ok`, with a comment naming the exact hazard:

> ```rust
> let sdf_mesh_shadow_set0: Option<[VulkanBindGroup; FRAMES_IN_FLIGHT]> = if let (true, true, Some(layout)) = (
>     scene.path_is_vb(),
>     // The set's @6 is a STORAGE_IMAGE descriptor over the `sdf_term` ring — on a device
>     // without RG8 storage the ring was created SAMPLED-only ... and a storage descriptor
>     // over it would violate the update-time VUID; SV0 is unarmable there ...
>     ctx.device_caps().rg8_unorm_storage_ok,
> ```
> What is real is **doc-rot**: the field doc at `:639-643` says "built on every VB boot", omitting the cap conjunct. Filed for repair at DP6e (where the field is deleted anyway, so the repair is the deletion) — **not widened, and not a defect.**

### Decision 6 (REWRITTEN) — ONE predicate, the store gated in-shader, and the measurement respecified

**P0-3, both halves.**
```rust
// The ONE carrier: a second boot-frozen bool on `ResolvedRenderPathGpu`, beside the existing
// `vb_sdf_mesh_armable` mirror (`scene_types.rs:1514`, set at `gpu_scene/mod.rs:750` — the precedent):
//   vb_sv0_host ≡ vb_sdf_mesh_armable() ∧ sdf_mesh_term_wanted        (N7's narrowing)
// NOT armable alone — that is satisfied by every VB×Both+SSAO boot with no SDF_MESH request and
// would bind the +64 %-footprint sv0 variant + declare a skipped `sdf_term` write on ordinary
// production frames (the Decision-1 dark tax). With the conjunct: env host ⇒ term_wanted ⇒ split ⇒
// armable ⇒ host, mode 0 (arm B buildable); production armed ⇒ host ∧ mode ≠ 0 (invariant 10 by
// construction); SSAO-only ⇒ host false ⇒ `base`, no declared write; !rg8 ⇒ !armable ⇒ !host.
// Read at BOTH `declare_vb_graph` (the conditional `sdf_term` write access) and `record_vb` (the
// pipeline pick) — the O1 discipline, one predicate, two consumers. `armable`'s own definition is
// untouched, so the P1-1 fixture table (incl. BOOTS[1] armable:true) stands as written.
```
- **pick** (both `cfg` arms, P2-2): `if mv_active { mv } else if scene.vb_sv0_host { sv0 } else { base }`.
- **declaration**: `if scene.vb_sv0_host { g.image_access(sdf_term, COMPUTE, WRITE, GENERAL, COLOR); ... }`.
- **shader**: the store moves **inside** the mode gate —
  ```hlsl
  if (sv0_mode != 0u) { gSdfTerm[uint2(px, py)] = float2(vis, ao); }
  ```

Three properties this buys, all needed:
1. **Arm B is buildable.** `vb_sv0_host && mode == 0`: the `sv0` module is bound, the write is *declared* (safe over-declaration — a barrier for a write that does not occur is correctness-neutral; the converse is UB), and the shader skips it. A and B then differ in **exactly one taken branch**, which is what a control arm must mean.
2. **Semantic fidelity preserved** (critic's preserve list). `sdf_mesh_shadow.comp.hlsl:183` stores unconditionally, but is only *recorded* when `mode != 0` — so its store is unconditional-given-mode-nonzero. Gating in-shader is behaviourally identical on every `mode != 0` frame. The branch is on a **wave-uniform** scalar header read: one scalar compare, zero divergence.
3. **Invariant 6 intact.** On a production disarmed boot `vb_sv0_host == false` ⇒ no access, no barrier, no command.

**P0-2 — the brackets, named against verified line numbers.**
`ZONE_VB_RUN` ends at `vb.rs:3016`; `vb_geo` records at `:3752`; `ZONE_VB_SHADE` brackets the split shade at `:4277`/`:4442`, the classified producer at `:3416`/`:3585`, the fused one at `:3591`/`:3700`. So:

- **New `ZONE_VB_GEO = ZONE_BASE_VB + 11`** brackets `vb_geo`'s barriers→bind→dispatch. Rev 3 called it "the family's only unbracketed dispatch"; §R4-D3 shows that claim was false when written (`sdf_forward_march` and both `vb_viewt` sites remain unbracketed after DP6-0b too) and narrows it to an enumerated residual.
- ~~Split-pair quantity = `ZONE_VB_GEO + ZONE_VB_SHADE`~~ — **WITHDRAWN at Rev 4.** Two independent defects (§R4.1.8: it is a `Σ(median_f)` the reducer's own rule forbids; §R4.1.3: `ZONE_VB_SHADE`'s TOP latch absorbed ≈ 34 % of a ~256 µs unbracketed predecessor stretch, measured at 4.58× the fused row). Superseded by `ZONE_VB_PRODUCE_NET` (§R4-D2/§R4-D4). The disjointness observation itself stands and is why a sum was reached for: `vb_viewt`, the SSAO gather and the à-trous chain sit between the two brackets, so the pair was never a span.
- ~~Fused-side comparator = `ZONE_VB_SHADE` alone~~ — **WITHDRAWN at Rev 4** for the same latch reason. `ZONE_VB_SHADE` remains a reported row and keeps `tops(...)` (id 2 is never restamped, so VB-P1d's published break-even keeps its meaning); the split producer's cost is obtained by derivation, `shade_derived = PRODUCE_RUN.end − PRESHADE.end`.

**The cost table's four cells, restated on the Rev 4 comparator** (one quantity on every row, and it does not name which producer ran):

| | today | today, MEASURED at DP6-0b | after DP6 |
|---|---|---|---|
| fused, SV0 armed | `ZONE_VB_PRODUCE_NET` (`≡ PRODUCE_RUN`; `PRESHADE` absent-`Forbidden`) | **62 976 ns** (`[vb_both_sdf]`, arm A) | `ZONE_VB_PRODUCE_NET` |
| split, SV0 armed | `ZONE_VB_PRODUCE_NET` | **97 280 ns** (`[vb_both_ssao]`, arm A) | `ZONE_VB_PRODUCE_NET` |

The middle column is a number where one exists — the two armed cells of the DP6-0b RESULT block, on
the repaired instrument. The after-DP6 column stays symbolic because it is not measured until DP6d;
its **prediction** for the fused row is Decision 3's re-derived sub-block.

Both rows still need their brackets to exist **before** the producer moves — hence **rung DP6-0**, which minted `ZONE_VB_GEO` alone, and **rung DP6-0b**, which mints ids 12/13/14 and re-takes DP6-0's four cells on the repaired instrument. Baselines are then a *paired* before/after on one instrument, not a comparison against a remembered number — and DP6-0's own four cells are **void as baselines** (the instrument changed under them) while being **kept as the evidence for the repair**.

**Arms** (on `vb_both_sdf` for fused, on the new `[vb_both_ssao]` boot for split):
| arm | `BOYKO_SDF_MESH` | split | variant | mode | `vb_sv0_host` |
|---|---|---|---|---|---|
| A | `on` | armed | `sv0` | 3 | true |
| B | `host` | armed | `sv0` | **0** | **true** |
| C | (SSAO boot, no SV0) | armed | `base` | 0 | false |

`Δ_AB` = the term's marginal cost inside an already-fetching host — **the same shape as the 6 144 ns reference**, reported with its ratio but **not gating** (the rung no longer claims it). `Δ_BC` = the compiled-in-but-closed variant tax, budgeted at the null-certified resolution; it is the number that would *refute* Decision 1. Arm C's SSAO-chain confound is second-order (`ZONE_VB_GEO` brackets only the geo dispatch) and named.

**Clause 5 gates every row.** An uncertified row is INCONCLUSIVE — never PASS, never FAIL.

### Decision 7 (carried, P2-4 edit) — `ZONE_VB_GEO` minted, `ZONE_VB_SDF_MESH` retired in place

`VB_ZONE_COUNT` 11 → 12; slot 10 a permanent hole (one unused pair, zero commands — `NotBracketed`); id 11 auto-`TopOfPipe` via the `matches!(zone, LATE_UPLOAD..=RUN)` exclusion in `zone_begin_stage`. **On the preserve list, carried verbatim.**

**Rev 4 amendment — the zone count and the stages both move at DP6-0b.** `VB_ZONE_COUNT` **12 → 15**:

| id | name | stamped? | stage |
|---|---|---|---|
| `ZONE_BASE_VB + 12` | `ZONE_VB_PRODUCE_RUN` | yes | BOTTOM |
| `ZONE_BASE_VB + 13` | `ZONE_VB_PRESHADE` | yes | BOTTOM |
| `ZONE_BASE_VB + 14` | `ZONE_VB_PRODUCE_NET` | **never** — derived from the two above, per frame | n/a |

Ids **10 and 11 restamp TOP → BOTTOM** (§R4-D1: the tree's own rule at `gpu_zone.rs:415-419` applied to a premise this rung deliberately falsifies, at the rung `:448` itself nominates), spelled as **four names** in `zone_begin_stage` and never as a range. `tops(ZONE_VB_SHADE)` and `tops(ZONE_PARTICLE_DRAW)` are retained; the `gpu_zone.rs:539-542` assertion message is rewritten to name `ZONE_PARTICLE_DRAW` alone, because it goes false in the same edit. Under the `≤ 16` const assert at `:292-295` this rung consumes **three of the family's four remaining ids: ONE slot remains, and the next VB zone after that is the last the `u16` witness masks can carry** — a stated cost, not a later discovery.

**P2-4:** the leg table (`gpu_zone.rs:227-230`) currently has two rows labelled by *occlusion*-split arming and no SV0 row, so "armed" already means two things. The same edit (a) renames the axis to `occlusion split armed` / `occlusion split off`, (b) adds a `geo/shade split` dimension, (c) adds the SV0 row that `ZONE_VB_SDF_MESH`'s own doc describes but the table never had. Resolving the pre-existing ambiguity is in scope because the new row inherits it.

### Decision 8 (carried, P1-4 + P2-3 edits) — the shadow-leaves pins

```rust
const SHADOW_LEAVES_MIN_CONSUMERS: [&str; 2] = ["deferred_pbr.hlsl", "vb_geo.comp.hlsl"];
/// Each derived consumer's own expected vk::binding spelling. A consumer ABSENT from this
/// table is a RED: a new marcher host must state where it put the edit list.
const BUF_BINDING_BY_CONSUMER: [(&str, &str); 2] = [
    ("deferred_pbr.hlsl", "[[vk::binding(10)]]"),
    ("vb_geo.comp.hlsl",  "[[vk::binding(4, 1)]]"),
];
```
**`BUF_T0_DECL` is untouched** (P1-4) — the span writes `[[vk::binding(4, 1)]] StructuredBuffer<uint> Buf : register(t0);`, matching `sdf_mesh_shadow.comp.hlsl:96`'s own `vk::binding(10,0)` + `register(t0)` pairing. Only the **slot** assertion moves; the register pin, the ordering assertions and the const-block assertions all pass unchanged.

**P2-3 — the right doc-rot targets.** The array's own doc (`:698-709`) is **current** and needs only the third-turn update. The stale text is the **test-fn doc `:523`** ("`SHADOW_LEAVES_MIN_CONSUMERS` only asserts that the four known ones did not vanish" — the array holds two) and the **assertion message `:545-548`** ("The three VB lit-producer tails plus the deferred resolve are the whole reason the shared header exists"). All three are edited together; per the doc-rot lesson the new text names commits, not adjectives.

## Data structures

```rust
// boyko_render::render_path_config — ONE new field, default false.
pub struct RenderPathConsumers {
    /// **VB-SV0 DP6.** BOOT SNAPSHOT of "the owner asked for the SDF-on-mesh term (either
    /// half), or the env-only measurement host arm". Set at the `gpu_scene` boot seam.
    ///
    /// DEFAULT `false` — the mechanism by which zero goldens turn: no existing boot's
    /// resolution moves by one field.
    ///
    /// Gates `mesh_geo_shade_split` (VB only) because the term's PRODUCER is the split's
    /// geometry half. BOOT-FROZEN: a request first raised at frame 100 is clamped forever
    /// (see `vb_sdf_mesh_armable`'s corrected doc).
    pub sdf_mesh_term_wanted: bool,
}
// LightingConfig: NO new field. bits 5..6, both `_armed` fields, `shadow_gate_word` UNCHANGED.

// GBufferScene — the ONE predicate (invariants 9, 10).
pub vb_sv0_host: bool,
```

```hlsl
// vb_geo.comp.hlsl — every addition inside the guard; APPEND-ONLY; no new local outside it
// (the R9b hoisted-load lesson). `n` and `geo` are reused, not re-derived.
#ifdef VB_SV0_TERM
#define VB_SV0
#endif
// ... existing includes/bindings/oct_encode/push constant UNTOUCHED ...
#ifdef VB_SV0_TERM
[[vk::binding(3, 1)]] [[vk::image_format("rg8")]] RWTexture2D<float2> gSdfTerm : register(u11);
#include "light_table.hlsli"                              // P2-1: mode bits + light loads, FIRST
[[vk::binding(4, 1)]] StructuredBuffer<uint> Buf : register(t0);   // P1-4: register(t0) KEPT
#include "sdf_field.hlsli"
static const float EPS = 0.001;  static const float T_MAX = 10.0;
static const uint  MAX_IT = 128u; static const float SHADOW_K = 8.0;
static const float SHADOW_MINT = 16.0 * GRAD_H;
static const float SHADOW_MINT_STEP = 16.0 * GRAD_H;
static const float SHADOW_HIT_EPS = 2.0 * EPS;
static const float SHADOW_NDOTL_EPS = 0.0;
static const float SHADOW_NORMAL_BIAS = 0.02;
#include "sdf_shadow_leaves.hlsli"
#endif

void main(...) {
    // ... sentinel early-out, vb_geom_fetch, n, #if MOTION, materials, gThinNormal — UNTOUCHED
#ifdef VB_SV0_TERM
    uint sv0_mode = load_vb_sdf_mesh_mode(LightBuf);   // wave-uniform, hoisted once
    float vis = 1.0;
    if ((sv0_mode & VB_SDF_MESH_SHADOW_BIT) != 0u) { /* primary directional; ranged march,
        origin lifted along vb_sv0_face_normal(geo) — §4.2 fidelity preserved */ }
    float ao = 1.0;
    if ((sv0_mode & VB_SDF_MESH_AO_BIT) != 0u) { ao = sdf_ao(geo.world_pos, n); }
    // P0-3: the store is GATED, so arm B (host, mode 0) writes nothing while binding this
    // module. Wave-uniform branch; identical behaviour on every mode != 0 frame.
    if (sv0_mode != 0u) { gSdfTerm[uint2(px, py)] = float2(vis, ao); }
#endif
}
```

## Integration

**`declare_vb_graph`:** delete the `sv0_pass` block (`:4607-4628`); inside the `if split` arm, after `thin_normal`, add under `if scene.vb_sv0_host` the `sdf_term` WRITE and the `light_table` READ (`vb_id`/`vb_instance_ring` already declared by `vb_geo`). The three tail-side term reads (`:4747`, `:4820`, `:5269` — 2 fused + 1 split, preserve list) become `if scene.vb_sdf_mesh_mode != 0`, plus `debug_assert!(!scene.path_vb_fused() || scene.vb_sdf_mesh_mode == 0)`. `VB_IMAGE_COUNT` and every ResId **unchanged** (`sdf_term` keeps index 15 / 21).

**`record_vb`:** delete `:3098-3174`; extend **both** `cfg` arms of the pick (`:3761-3774`); wrap `:3752`→dispatch in `ZONE_VB_GEO`.

**P1-1 — every truth-table fixture the conjunct turns:**

| fixture | today | after | edit |
|---|---|---|---|
| `render_path_config.rs::sv0_never_arms_under_hwrt` `:2755` | `assert!(armable.vb_sdf_mesh_armable())` on VB×Both, no consumers | **reds** | `sv0_consumers()` gains `sdf_mesh_term_wanted: true` |
| `render_path_config.rs::sv0_never_arms_under_hwrt` `:2775`, `:2796` | `!hwrt.vb_sdf_mesh_armable()` | still passes | + `assert!(hwrt.mesh_geo_shade_split)` — the split *is* armed, SV0 still is not (Decision 2's proof) |
| `render_path_config.rs::sv0_armable_only_on_vb_with_both_legs` | 4 negative rows | all still `false` | `sv0_consumers()` change propagates; add a positive-control row |
| `sv0_arm_matrix.rs` `BOOTS[0]` `:96-105` "VB x Both (fused)" | `armable: true`, *why*: "the SDF soft march is the shadow source and there are mesh pixels to shade" | **`armable: false`** | *why* rewritten: "no split ⇒ no `vb_geo` ⇒ no producer for the term" |
| `sv0_arm_matrix.rs` `BOOTS[1]` `:106-115` "+ SSAO (split tail)" | `armable: true`, *why*: "…armable exactly like the fused one" | still `true`, **reason now false** | *why* rewritten: SSAO arms the split, which IS the producer |
| `sv0_arm_matrix.rs` `BOOTS` | 9 rows | **10** | new row: VB×Both + term-wanted, no SSAO → `armable: true` (SV0 arms its own split) |
| `sv0_arm_matrix.rs` `:92-94` header doc | "the armable rows are the configuration the `vb_both_sdf`/`_tex` fixtures boot under" | false | rewritten with the split requirement |
| `sv0_mode_nonzero_implies_the_mesh_leg` `:397` | — | — | replaced by `..._implies_the_split` (strictly stronger) |

**Also affected:** `runner.rs` (boot-freeze contract comment + the `host` env arm), `gpu_scene/mod.rs` (the boot seam sets `sdf_mesh_term_wanted` and `vb_sv0_host`), `docs/RENDER-PARITY-PLAN.md` §3.2 (erratum: Option B superseded **under VB**; its overdraw-invariance rationale is *preserved* — `vb_geo` is also exactly one march per covered pixel), `docs/SHADER-VARIANT-MANIFEST.md`, `docs/OPEN-QUESTIONS.md`.

**Also affected, Rev 4 (DP6-0b only — no render behaviour, no shader, no pipeline):**
`crates/boyko_rhi_vulkan/src/present/gpu_zone.rs` (ids 12/13/14, `VB_ZONE_COUNT` 12→15, the four names in `zone_begin_stage`, the const stage pins, the `:539-542` message, the leg table's rows for 12/13 and id 6's two positions, `ZONE_VB_SHADE`'s skew warning, and the two dead-datum doc repairs at `:238` and `:232`/`:241-242`);
`crates/boyko_rhi_vulkan/src/present/passes/vb.rs` (the hoisted `produce_run_armed`, the two new brackets, id 10's begin moved above its `record_vb_pass`, the `begin_called`/`end_called` masks and `finish`'s compares including the `cfg`-gated `writes` read);
`crates/boyko_app/src/profiling/reduce.rs` (`chain` + `derived` declarations, the per-frame predicate, `OrderCensus`, frame-formed derived rows);
`crates/boyko_app/src/profiling/artifact.rs` (the `[order]` block and its builder, the derived `[[zone]]` row);
`crates/boyko_diag` (the `GpuZoneUnmatchedEnd` flag beside `GpuPairBudgetExhausted`);
new `crates/boyko_app/tests/vb_sv0_produce_run_timing.rs`;
`crates/boyko_app/tests/vg_occ_split_timing.rs` (**comment only** — id 12 now appears on its VB×Mesh boot and its bounded loop ignores it; `PASS_COUNT` stays 10).

## Implementation plan

Revert-red at every rung. **Semantic point of no return is DP6c** (Q4).

**Ladder after Rev 4's insertion:** DP6-0 → **DP6-0b** → DP6a → DP6b → DP6c *(semantic point of no return)* → DP6d → DP6d.5 → DP6e.

- **DP6-0 — the instrument, alone.** `ZONE_VB_GEO`; `VB_ZONE_COUNT` 12; leg-table edit (P2-4). No producer change. *Gate:* all goldens byte-identical; `vb_bench_query_validation` still `measured > 0`; **the four baseline cells recorded** on the unmodified producer.
- **DP6-0b — instrument repair and re-baseline. No render behaviour change.** (Rev 4; inserted between DP6-0, shipped and not re-opened, and DP6a.)

  **Why it precedes DP6a.** *"Nothing in DP6a/DP6b perturbs GPU timing"* is true only of **golden, SV0-disarmed legs**. On an **armed** leg DP6a's entire effect **is** the fused→split flip — which is what G-NEUTRAL prices. That is the argument for the ordering: the flip must be measured with the instrument already repaired.

  *`gpu_zone.rs`:* four names added to `zone_begin_stage` (never a range); ids 10/11 to BOTTOM with §R4-D1's premise-change reasoning written at the function, discharging `:448`'s own nomination of this rung; `:539-542`'s message rewritten to name `ZONE_PARTICLE_DRAW` alone; `VB_ZONE_COUNT` 12→15; the leg table gains rows for 12/13 and id 6's two positions; `ZONE_VB_SHADE`'s doc gains the measured skew warning (112 640 split vs 24 576 fused, 4.58×, no fetch arithmetic produces it) pointing at `shade_derived`; `:238`'s falsehood narrowed to an enumerated residual naming `sdf_forward_march` and both `vb_viewt` sites; `:232`/`:241-242`'s phantom `280/560` citation corrected to point at the expectation table.

  *`passes/vb.rs`:* `produce_run_armed` hoisted above `:1529`; `PRODUCE_RUN` `[3050, 4495]`; `PRESHADE` `[3890, 4327]`; id 10's begin moved above `:3141`; the two unconditional `begin_called`/`end_called` masks + `finish`'s three compares; `finish` reads `writes` under `#[cfg(debug_assertions)]`, making `:5353` and `:2540` true.

  *`boyko_app/src/profiling/`:* `reduce.rs` gains `chain` + `derived` declarations, the per-frame predicate, `OrderCensus`, and frame-formed derived rows; `artifact.rs` gains the `[order]` block at the `:848+` match-`k` arm set with its own builder, plus the derived row through the `[[zone]]` sites.

  *Tests:* new `crates/boyko_app/tests/vb_sv0_produce_run_timing.rs` — boots `[vb_both_sdf]` and `[vb_both_ssao]`, drives the **per-leg Required/Forbidden/Optional expectation table** (both directions red), the `[e6 → b11]` gap check, and **one assertion on `vb_both_ssao.rs:121`'s `atrous_levels: 3` literal**, which closes the DP6-0b→DP6c window in which `PRESHADE Required` asserts stamping but not magnitude and the byte pin does not yet exist. `vg_occ_split_timing` **unchanged** but for a comment recording that id 12 now appears and why its bounded loop ignores it.

  *Gate:* all goldens byte-identical; `torn == 0` and no diag flag on **five** leg shapes; `OrderCensus.violations == 0` with `frames_checked > 0`; **the four DP6-0 cells re-taken and published beside the old ones**, with §R4.1's diagnosis landed explicitly on one of §R4.3.7's three branches.

  ### DP6-0b RESULT — measured, and the rung's verdict

  > **The gate passed and the diagnosis landed on branch 2. `DP6a IS BLOCKED.`**
  >
  > **The four cells, re-taken on the repaired instrument** (release, 512×512, `sv0_scene`, 3 legs per arm; every arm's 3-leg relative spread in **0–7.89 %**, all under clause 5's 10 % bar; `R` certified this session and inherited from nothing):
  >
  > | fixture | `BOYKO_SDF_MESH` | `ZONE_VB_PRODUCE_NET` |
  > |---|---|---|
  > | `[vb_both_sdf]` (fused) | off | **27 648 ns** |
  > | `[vb_both_sdf]` (fused) | on | **62 976 ns** |
  > | `[vb_both_ssao]` (split) | off | **73 728 ns** |
  > | `[vb_both_ssao]` (split) | on | **97 280 ns** |
  >
  > `PRESHADE ≈ 304 µs` on the split fixture, confirming §R4-D4's premise that it dominates the wide bracket — which is why `NET` and not `PRODUCE_RUN` is the comparator.
  >
  > **The instrument is confirmed repaired, two ways.** `shade_derived` (`PRODUCE_RUN.end − PRESHADE.end`) agrees with the direct `ZONE_VB_SHADE` reading **to one timer tick**, so the derivation D3 introduced measures what it claims. And DP6-0's `112 640` split-shade reading is now decomposed: **87.8 % of its 88 064 ns inflation was instrument skew, 12.2 % was real.** §R4.1.3's "≈ 34 % absorbed" estimate was the right shape and the wrong fraction; the measured split is recorded here in its place.
  >
  > **The branch.** `Δ_host = shade_derived (35 840) − re-taken fused shade (24 576) = 11 264 ns`, which lands in §R4.3.7's **middle band** (`2 × R_neutral < Δ_host ≤ 29 184`) ⇒ **branch 2, MIXED**: both contributions are material, and `Δ_host` is recorded as **`E_split_host` = 11 264 ns**, the split tail's hosting surcharge.
  >
  > ⚠️ **The margin to branch 1 is 272 ns — half a timer tick — on the worst leg pairing.** That is *below this box's resolution*, so the branch-1/branch-2 boundary is **not resolvable** by this measurement: the verdict is branch 2, and the honest statement is that branch 1 cannot be excluded by 272 ns. What makes the verdict actionable anyway is its **robustness across the 3×3 leg cross-product: `E_split_host ∈ [10 512, 19 392] ns`** — every pairing is inside the middle band, so no pairing produces branch 1 or branch 3.
  >
  > **Consequence, per §R4.3.7's own text: `DP6a does not land` until Decision 3's fused row is re-derived with `E_split_host`.** G-NEUTRAL's after-side pays this surcharge and its before-side does not, so the fused cost table is understated by 10.5–19.4 µs until it is carried explicitly.
  >
  > **DISCHARGED at Rev 4.4** — the re-derivation is Decision 3's *"Trade-off, RE-DERIVED at DP6-0b"* sub-block, which carries `E_split_host` explicitly as term 4 of four. **DP6a is unblocked.**

  *Red mutations:* **(a) id 11 back to TOP ⇒ BUILD FAILURE (`E0080`) at the `const` stage pin, and — at runtime — the `[e6 → b11]` gap check.** *(Corrected against measurement; see the block below.)* (b) `PRODUCE_RUN` closed inside the split block ⇒ `Torn` on the fused leg; **(c) the `if produce_run_armed` guard dropped from `PRODUCE_RUN`'s CLOSE while its open stays inside `mesh_leg` ⇒ `GpuZoneUnmatchedEnd` on the VB×Sdf leg, in a RELEASE run** *(direction corrected: an open moved outside `mesh_leg` with a gated close is open-without-close, i.e. `Torn`, not an unmatched END)*; (d) declare `ZONE_VB_GEO` `Forbidden` on the split leg ⇒ expectation table reds.

  > **⚠️ Mutation (a)'s predicted red was WRONG, and the measurement says so.**
  >
  > This entry predicted `OrderCensus.violations > 0` on the split fixture. **Measured: 0 violations over 241 frames.** Restamping id 11 to TOP moves `b11` about **7 µs** earlier, but `b10` sits roughly **38 µs** ahead of it, so `prev_begin ≤ begin(m)` is never crossed — the displacement is an order of magnitude smaller than the slack it would have to consume.
  >
  > §R4.3.3's nondeterminism argument (`P(0 violations) = (1-p)^100 < 10⁻³`) **does not generalise to a member whose predecessor sits tens of µs ahead of it.** It was derived from the particle lane, where three dispatches are back-to-back with *zero* recorded commands between them and the slack is a timer tick. Stated here so the claim does not outlive the case it was measured on.
  >
  > The mutation IS caught, twice, and the stronger of the two is the one this design did not credit:
  > 1. **The `const` stage pin — a BUILD FAILURE (`E0080`), at compile time.** `bottoms(ZONE_VB_GEO)` is asserted in `gpu_zone.rs`'s `const` block, so the mutation cannot produce a binary at all. A gate that fails before a frame is rendered is strictly stronger than one that counts frames, and the ladder should have named it first.
  > 2. **The `[e6 → b11]` gap check**, as the runtime carrier: measured **544 ns** against the required ~5 248 ns, because the TOP latch swallows the `vb_viewt` dispatch that the gap exists to see.
  >
  > `OrderCensus` keeps its place for the direction it *did* prove — the per-frame containment and ordering of ids 10/11/12/13 — and its `frames_checked` remains the number that makes `violations == 0` readable. It is simply not the detector for this mutation on this box.

- **DP6a — resolver.** The consumer bit, the hoist, `mesh_geo_shade_split`, the NORMAL union, the `armable` conjunct, the env `host` arm, the doc corrections, the Rev-5 erratum. *Gate:* the eight fixtures above green after their stated edits; `sdf_mesh_term_wanted == false ⇒ every ResolvedRenderPath field bit-identical to pre-DP6` (tested, not argued); **all goldens byte-identical**. **Gate content unchanged by Rev 4.4** — DP6a is resolver-only, gated on fixture truth tables and byte-identity, none of which the re-derivation touches. **Precondition discharged:** §R4.3.7's block is lifted by Decision 3's re-derivation (`E_split_host = 11 264 ns`, band `[10 512, 19 392]`); **DP6a may land.**
- **DP6b — the dark variant.** Guarded span; `vb_geo_aux_layout` @3/@4 + the `!rg8_ok` placeholder; `vb_geo_sv0.comp.spv`; `embed_spirv!`; boot pipeline. Selected by nothing. *Gate:* `vb_geo.comp.spv`/`vb_geo_mv.comp.spv` byte-identical; new `spv_sync` row 7; **the new two-sided `-P` gate**; `spirv-val`; `sdf_field_edsl_sync` re-pointed; manifest row; all goldens.
- **DP6c — select, declare, record.** `vb_sv0_host`; the graph diff; the pick; `ZONE_VB_GEO` recording. Dedicated pass becomes **unreachable**. *Gate:* **new pin `[vb_both_ssao]`** (VB×Both + SSAO, SV0 disarmed — the boot class DP6 changes most, unpinned today) byte-identical across the rung; `[vb_mesh_ssao]` byte-identical; **live pixel-signature armed**: `SV0_MIN_SHADOWED_PIXELS`/`SV0_MIN_AO_PIXELS` with **(ii-a) shadow alone and (ii-b) AO alone each moving pixels on its own**; `sv0_arm_matrix.ps1` re-pointed; declare↔record parity asserts.
- **DP6d — measure.** Arms A/B/C on both boot classes. *Gates (Rev 4 — every row reads `ZONE_VB_PRODUCE_NET`, see the gate text above):* **what DP6d does FIRST, before any arm is read:**
  1. **Assert the instrument identity** — `const` block: `bottoms(ZONE_VB_SDF_MESH) && bottoms(ZONE_VB_GEO) && bottoms(ZONE_VB_PRODUCE_RUN) && bottoms(ZONE_VB_PRESHADE)`; `tops(ZONE_VB_SHADE) && tops(ZONE_PARTICLE_DRAW)` retained. A build failure, not a test.
  2. **Certify `R`** per gated row, per fixture, this session. Inherit nothing.
  3. **Read the structural verdicts** — `OrderCensus` (violations **and** `frames_checked`), both diag flags, the expectation table, and `torn == 0` on **five** leg shapes: VB×Mesh; VB×Both fused; VB×Both split; VB×Both split+SV0; **and VB×Sdf (non-mesh-leg)**.
  4. **Confirm** the derived row's `n` floor and the occlusion-split leg-field equality between sides.

  Only then are arms A/B/C read — **G-NEUTRAL** (fused: `median_f(NET)` Δ ≤ `+R_neutral`) and **G-REDUCE** (split: Δ < `−R_reduce`); `Δ_BC` ≤ resolution; `Δ_AB` on the now-BOTTOM-stamped `ZONE_VB_GEO`, reported with its 2× ratio, **informational**; `Δ median(PRESHADE)` reported as an anomaly requiring a stated cause. **A DP6d that reads an arm before step 4 has chosen its comparator after seeing its data, and its verdict is void.**
- **DP6d.5 — DP7 feasibility probe (P0-4).** In the dedicated pass, still present: half-res dispatch grid + half-extent term + a bilinear read at the tail. *Deliverables:* the quarter-cost number, and an owner-eval visual on silhouette crawl. **Not shipped, not gated on** — a probe. Reverted after measurement.
- **DP6e — retire.** Delete the shader, `.spv`, pipeline, layout, `sdf_mesh_shadow_set0` (and with it the `:639-643` doc-rot), `VbPlan::sv0_pass`, `ZONE_VB_SDF_MESH`'s live use, `sdf_mesh_shadow_spv_sync.rs`; `SHADOW_LEAVES_MIN_CONSUMERS` → 2 entries; the three P2-3 prose sites. *Gate:* `--workspace --no-fail-fast` green; all goldens; **the DP6c live-pixel proof re-run**; and — **blocking** — **an explicit recorded DP7 disposition** from DP6d.5: either *"half-res is refused on quality, the door may close"* or *"half-res is wanted; here is the host shape it will use after `vb_geo` retires the dedicated pass"*. **No disposition ⇒ DP6e does not land.**

## Metrics and validation

**Per-boot-class table** (§Decision 6) is the headline. No single-number claim.

**Rev 4 additions — the timing channel's own deliverables.** Every one of these is a gate input, not a report:

- **`ZONE_VB_PRODUCE_NET` is the one gated quantity** on both rows, formed per frame inside `WindowReducer::observe_frame` and reduced afterwards (`median_f(Σ)`). `ZONE_VB_PRODUCE_RUN` is published beside it as the total and as a per-frame cross-check (`NET + PRESHADE ≡ PRODUCE_RUN`). `Δ median(PRESHADE)` is reported, not gating, and a movement beyond `R_preshade` re-takes the run with a stated cause.
- **`OrderCensus { frames_checked, frames_skipped, violations, worst_ns }`** — a COUNT of per-frame chain violations, published in the artifact's `[order]` block. `violations == 0` **with `frames_checked > 0`**; a zero over zero is not a pass. The first violation additionally raises a sticky `boyko_diag` flag, so a red reaches a reader who never opens the artifact.
- **The per-leg expectation table** (`Required` / `Forbidden` / `Optional` per zone per fixture) must match the artifact exactly. It is the red-capable form of the invariant `gpu_zone.rs:232`/`:241-242`'s phantom `280/560` citation was reaching for; a pair-count pin is refused with a reason (leg-dependent, reds on every legitimate zone addition, has already failed to red on two).
- **`ZONE_VB_PRODUCE_NET` (id 14) is `Forbidden` as a stamped row on every leg** — a release-live check that the derived id never reaches `TsWitness`.
- **The `[e6 → b11]` gap** checks the unbracketed `vb_viewt` dispatch without minting a zone: ≈ 5 248 ns where the expectation table says `Required`, ≈ 0 where `Forbidden`.
- **`R` is certified per row, per fixture, per session and inherited from nothing** — including from DP6-0's own 4 608 ns, which is void because the instrument changed under it.
- **Two diag flags gate every row:** `GpuZoneUnmatchedEnd` (new at DP6-0b, release-live) and `GpuPairBudgetExhausted`.
- **Five leg shapes** carry `torn == 0` and no diag flag: VB×Mesh; VB×Both fused; VB×Both split; VB×Both split+SV0; VB×Sdf (non-mesh-leg). The fifth exists because the four originally listed could never exercise the unmatched-END direction.

**Byte-identity:** all goldens at every rung; `vb_geo.comp.spv` / `vb_geo_mv.comp.spv`; all ten lit-producer `.spv`; the two-sided `-P` gate. **DP6-0b moves no pixel** — it adds timestamp brackets only, so its own gate is the full golden set byte-identical, proved rather than argued.

**P1-5 — the `-P` gate is NEW work.** No `.rs`/`.ps1` in-tree invokes `dxc -P`; only plan prose cites it (the recurring dead-datum shape — five instances on record). **Owning file: `crates/boyko_rhi_vulkan/tests/vb_geo_preprocess_sync.rs`, landing at DP6b**, cloning `find_dxc()`/temp-dir discipline from `cluster_cull_spv_sync.rs`. Two assertions: (i) `dxc -P vb_geo.comp.hlsl` (no defines) is **character-identical** to the pre-DP6b file's `-P`; (ii) with `-D VB_SV0_TERM=1` it **differs**. **The pre-DP6b hash is RECOMPUTED via `git show <DP6b^>:crates/.../vb_geo.comp.hlsl`, not committed** — a committed literal is a datum nobody re-derives and the first "fix" is to re-bless it; `git show` makes staleness impossible. Skips (with `eprintln!`) when no pinned `dxc` resolves, per house idiom.

**Property-based (quantified, not sampled):** `vb_sdf_mesh_armable() ⇒ mesh_geo_shade_split`; `mesh_geo_shade_split ⇒ thin_aux.NORMAL` — **BY CONSTRUCTION after the O5 amendment**, since the union's new disjunct IS the antecedent, so this one can no longer fail independently; `vb_geo_mv_active() ⇒ !vb_sv0_host` (the variant-count proof); `vb_sdf_mesh_mode != 0 ⇒ vb_sv0_host` (invariant 10).

**The CONVERSE companion, added at the DP6a review (O5) because it is the property that would have caught the defect:** **`thin_aux.NORMAL ∧ path == VisibilityBuffer ⇒ mesh_geo_shade_split ∨ another pre-light consumer`.** `mesh_geo_shade_split ⇒ NORMAL` says the channel is armed wherever `vb_geo` writes it and says nothing about the other direction — and the other direction is where the first spelling failed: `|| vb_sv0_split` armed NORMAL on a `VB × Sdf` boot that runs no `vb_geo` at all. Shipped as `under_vb_the_normal_channel_is_never_armed_without_a_writer_or_a_reader`, quantified over the same path × legs × caps × 2^12-consumer-mask space as the exclusion sweep. It is deliberately a statement about the union's permitted MEMBERSHIP rather than its formula, so it reds exactly when a disjunct that is neither the writer nor one of the five readers is added.

**Q5 answered.** No committed pin covers split+SV0-armed. `[vb_mesh_ssao]` is VB×**Mesh** — no SDF leg — so `SDF_SOFT_MARCH` never arms and it can *never* pin SV0; `[vb_both_sdf]` is fused. With the new bit defaulting false, none would arise by itself. **This is not left as intended-but-unpinned:** DP6c adds `[vb_both_ssao]` (split, SV0 **disarmed**) as a real byte pin on the boot class the rung most changes, and seeds `[vb_both_ssao_sv0]` **PENDING** for the owner-eval packet — the `[vb_both_sdf]` precedent. The armed combination stays proven by adequacy floors until the owner blesses the frame.

**Red mutations to DEMONSTRATE:** (1) move a statement outside `#ifdef VB_SV0_TERM` → `-P` gate reds; (2) drop the `mesh_geo_shade_split` conjunct → `sv0_armable_requires_the_split` reds; (3) default the consumer bit `true` → goldens red; (4) move `Buf`'s vk::binding without the table → `sdf_field_edsl_sync` reds; (5) drive the pick from `mode != 0` instead of `vb_sv0_host` → arm B binds `base` and `Δ_AB` collapses to the whole march (the P0-3 mutation); (6) drop the `sdf_term` access while keeping the pick → validation/sync red on arm B. **(7) [DP6a review, W3] drop the `&& vb_sdf_mesh_storage_ok` conjunct from `vb_sv0_split` → `sv0_arm_matrix`'s row *"VB x Both + term wanted, no RG8 storage"* flips (`expect_split` false → true, and the matrix reds on the split assertion before it reaches armability).**

## Open questions

1. **ADJUDICATED at Rev 4 — REPLACED in full by §R4.** The question was *"can G-NEUTRAL fail on fused boots, and what then?"*. DP6-0's measurement turned it from a hypothetical into a number, and the number says Rev 3's comparator could not have adjudicated it either way: `ZONE_VB_SHADE` read **4.58×** higher on the split boot than the fused one, of which the fetch arithmetic explains **< 1 %** and a TOP-latch absorbing ≈ 34 % of a ~256 µs unbracketed predecessor stretch explains the rest (§R4.1.3, §R4.1.5); and the split-side sum `ZONE_VB_GEO + ZONE_VB_SHADE` is a `Σ(median_f)` the reducer's own rule forbids (§R4.1.8). **Disposition: repair the instrument first (rung DP6-0b), then gate on `ZONE_VB_PRODUCE_NET`** — one quantity whose definition never mentions which producer ran, so it is identical on both sides of the fused/split discontinuity by construction (§R4-D2, §R4-D4). The Rev 4 gate text above replaces Rev 3's three bullets.

   **The old disposition is retained as the FALLBACK CHAIN, not deleted:** `vb_shade_split` is still not bit-for-bit `vb_resolve` (different Set 1, SSAO combine, DDGI sampling — runtime-gated off but compiled in), and the split still adds the `thin_normal` write, so the fused row can still red *on its merits* once the instrument no longer skews it. Two escalation steps, in order:
   - ~~**If** DP6-0b's `Δ_host` lands in §R4.3.7's middle branch~~ — **IT DID. MEASURED: `Δ_host = 11 264 ns`, branch 2, `E_split_host` real and material** (see the DP6-0b RESULT block in the ladder for the cells, the 272 ns unresolvable margin to branch 1, and the `[10 512, 19 392]` robustness band). ~~**DP6a does not land** until Decision 3's fused row is re-derived with it.~~ — **DISCHARGED at Rev 4.4**: the re-derivation is Decision 3's *"Trade-off, RE-DERIVED at DP6-0b"* sub-block, and DP6a may land. The third branch (`Δ_host > 29 184`) — which would have re-opened DP6's whole cost model — is **excluded** on every leg pairing.
   - **If G-NEUTRAL then reds on the repaired instrument, or if `R_net > 10 752` makes G-REDUCE INCONCLUSIVE** (§R4.6's open residual 1), the disposition is the one Rev 3 pre-agreed and Rev 4 keeps: **restrict DP6e to split boots and keep the dedicated pass for fused** — the critic's option (b) *with a measurement behind it* rather than as a premise. Pre-agreed here so it is not improvised under a red. **Rev 4.4 — the pre-agreed fallback now has a measured region rather than a bare contingency: it applies iff `GEO_base + M_geo + E_split_host > 40 448 ns` as DP6d measures them**, which at term medians means `M_geo` above **14 336 ns**, i.e. only the top 64 ns of its `[6 100, 14 400]` model band (Decision 3's three-row ceiling table).
2. **DP6d.5 may say half-res is wanted.** Then DP6e must name the post-retirement host shape for it, and the honest answer may be "a new minimal half-res marcher pass" — which partially un-does the consolidation. Recorded as a real risk of Decision 3, not hidden.
3. **DP2's and DP4's null resolutions disagree** (~24 576 ns vs 1 024 ns, same date, different fixtures) and both are load-bearing for their PASS verdicts. DP6-0 must re-certify on its own fixtures and **inherit neither**. → `docs/OPEN-QUESTIONS.md`.

   **Rev 4 extends the do-not-inherit list by one, and the new entry is this design's own:** **DP6-0's 4 608 ns is void as a baseline** — the instrument changed under it when DP6-0b restamped ids 10/11 and added ids 12/13. Every `R` is certified **per row, per fixture, per session**: not DP6-0's 4 608, not DP2's 24 576, not DP4's 1 024. DP6-0's four cells are kept as *evidence for the repair* and republished beside the re-taken ones at DP6-0b; they are never a side of a comparison. The reason the list keeps growing is the same each time — a resolution is a property of one instrument on one fixture in one session, and a number that outlives any of the three is a remembered number, not a measured one.
4. **`docs/RENDER-PARITY-PLAN.md` §3.2B does not exist** — repo-wide grep for `3.2B` returns zero; §3.2 is the A/B/C options list, next heading §3.3. The lever's entire prior written form is one sentence in DP4's disposition. DP6a adds the erratum rather than pretending the subsection existed. **LANDED at DP6a**: the erratum sits as a blockquote after §3.2's A/B/C list — Option B superseded **under VB only**, its overdraw-invariance rationale preserved (`vb_geo` is also one march per covered pixel), and the withdrawn "cost-neutral by construction" named so a §3.2 reader does not inherit it. The `3.2B` grep still returns zero for a subsection; the two new hits are the erratum's own citation of this line.
5. **External corroboration still pending.** A `researcher` sweep on published `vb_geom_fetch` cost shares, dedicated-vs-inline practice and its occupancy/VGPR rationale, half-res march savings, and async-compute overlap has not returned to me. Two findings could still move Rev 2: a documented occupancy cliff for merging a long march into a geometry pass would strengthen G-NEUTRAL's risk (open question 1); a large published half-res saving would raise DP6d.5 from probe to blocking rung. **Neither can change the fetch/dispatch counting table**, which is measured on this box — which is why the rung's justification now rests on that table and on consolidation, not on external practice.