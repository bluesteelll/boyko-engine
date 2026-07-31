# VG-R0 density census — MACHINE-WRITTEN by `vg_r0d_census_gate`

Every number below was produced by the run that wrote this file; nothing here is hand-entered. The census renders at FULL detail (`[k1].measured_at`), so the densities are the CEILING of the mechanism available to any LOD scheme.

`assets/vg_corpus/CORPUS.toml` sha256: `5999af9567849b87467303691ed2a3ab4f4fa120db96e2c2d684602d7ce8e2f7`

## Rows — one per (committed camera path, ladder rung) pair

| path | rung | extent | covered px | visible tris | mode | submitted | vis/covered | sub/covered | readback sha256 |
|---|---|---|---|---|---|---|---|---|---|
| orbit_mid | 0 | 512×512 | 37643 | 20973 | 0 | 2279237 | 0.557155 | 60.548761 | `8e295b729252ca63` |
| orbit_mid | 1 | 1920×1080 | 167573 | 69517 | 0 | 2279237 | 0.414846 | 13.601457 | `e1f3ab010ba34c9a` |
| orbit_mid | 2 | 2560×1440 | 297835 | 103819 | 0 | 2279237 | 0.348579 | 7.652684 | `3987e8910487668d` |
| orbit_mid | 3 | 3840×2160 | 670160 | 176397 | 0 | 2279237 | 0.263216 | 3.401034 | `02c8ef0b1922dd73` |
| approach_close | 0 | 512×512 | 49364 | 23269 | 0 | 2279237 | 0.471376 | 46.172048 | `f01d80e12faa6cbf` |
| approach_close | 1 | 1920×1080 | 459878 | 146965 | 0 | 2279237 | 0.319574 | 4.956178 | `6bb6990005f974f9` |
| approach_close | 2 | 2560×1440 | 817526 | 185065 | 0 | 2279237 | 0.226372 | 2.787969 | `7b961f56539f5f44` |
| approach_close | 3 | 3840×2160 | 1839315 | 234094 | 0 | 2279237 | 0.127272 | 1.239177 | `77dcdf283800d5b7` |

## D_est — the decisive statistic

`D_est(p) = visible_tris(p, top rung) / covered_pixels(p, decision rung)`. It is a LOWER bound (sub-pixel triangles that win no sample are absent from `visible_tris`), so it can CONFIRM density and never deny it — which is why R0 can only refute K1. Its ceiling on this ladder is 4.0 exactly, the top-rung/decision-rung area ratio.

| path | D_est | vs d_est_min |
|---|---|---|
| orbit_mid | 1.0527 | ≥ |
| approach_close | 0.5090 | < |

**MIN over committed paths = 0.5090** against `[k1].d_est_min` = 1 ⇒ **UNDECIDED, escalate**.

MIN rather than MAX because refutation is the campaign-FAVOURABLE outcome: a favourable verdict must clear the bar on the WEAKEST committed framing, not the strongest.

⚠️ **`visible_tris` has NOT converged on orbit_mid, approach_close** (residual above `[k1_instrument].ladder_convergence_margin` = 0.05; see the table below). `D_est` is therefore an UNDERESTIMATE of unknown size — it is still rising with resolution. Per `[k1_instrument].on_not_converged_refute_direction` that would not weaken a REFUTATION, since an understatement already at or above the floor still proves density; it does mean this UNDECIDED verdict is a statement about the INSTRUMENT's reach, not a finding that the content is sparse. The disposition is to extend the ladder upward in a new plan revision, NOT to adjudicate on an underestimate.

## Cross-process reproduction — R0c(e)'s measurement, R0d(a)'s gate

| path | rung | identical | digests |
|---|---|---|---|
| orbit_mid | 3 | true | `02c8ef0b1922dd73`, `02c8ef0b1922dd73`, `02c8ef0b1922dd73` |
| approach_close | 3 | true | `77dcdf283800d5b7`, `77dcdf283800d5b7`, `77dcdf283800d5b7` |

## Convergence residual — REPORTED, not gated

Firing K1 is unreachable at R0 (no non-saturating upper bound exists), and convergence is a precondition for FIRING, never for REFUTING — so this residual gates nothing.

| path | visible_tris(top-1) | visible_tris(top) | residual | margin |
|---|---|---|---|---|
| orbit_mid | 103819 | 176397 | 0.4114 | 0.05 |
| approach_close | 185065 | 234094 | 0.2094 | 0.05 |

## Modal-bucket shift — MEASURED AND RECORDED, deliberately NOT a gate

Against the per-pair `log2` of the actual area ratio. The measured shift is a difference of INTEGER bucket indices while the targets are irrational, so a tolerance around them admits exactly one integer and would assert an integer while claiming not to. Worse, near the one-pixel censoring floor — the micro-polygon regime the census exists for — every newly visible triangle enters at bucket 0 and pushes the mode the wrong way, so the check would red hardest exactly where the premise is most strongly confirmed.

Rung [0] excluded: 512² is 1:1 while the other rungs are 16:9, so the projection is a DIFFERENT FRUSTUM and the "area scales with pixel count" premise does not hold across that step at all.

| path | pair | area ratio | target (log2) | measured | residual | tolerance |
|---|---|---|---|---|---|---|
| orbit_mid | 1→2 | 1.7778 | 0.830075 | +0 | 0.830075 | 0.35 |
| orbit_mid | 2→3 | 2.2500 | 1.169925 | +0 | 1.169925 | 0.35 |
| approach_close | 1→2 | 1.7778 | 0.830075 | +0 | 0.830075 | 0.35 |
| approach_close | 2→3 | 2.2500 | 1.169925 | +0 | 1.169925 | 0.35 |

## What this census does NOT decide

* **K1 cannot be FIRED here.** `D_est` is a lower bound; firing needs an upper bound on VISIBLE density that is demonstrably non-saturating, which is an unsolved design problem (`[k1].k1_fire_instrument_status`).
* **There is no representativeness floor.** The non-degeneracy floors are EMPTY-FRAME guards; `D_est` is scale-free, so no floor on that axis can carry representativeness. It needs a floor on covered FRACTION, and R0 does not have one.
* **Camera-path DEFINITIONS are hashed by nothing here.** Membership and cardinality are gated; re-aiming a committed path is neither, and no R0 gate part sees it.
* **Corpus placement is normalised.** Each asset is scaled to a unit cube, which equalises screen share — a choice about what the corpus represents, recorded rather than claimed away.
