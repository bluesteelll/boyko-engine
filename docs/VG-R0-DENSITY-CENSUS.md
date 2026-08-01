# VG-R0 density census — MACHINE-WRITTEN by `vg_r0d_census_gate`

Every number below was produced by the run that wrote this file; nothing here is hand-entered. The census renders at FULL detail (`[k1].measured_at`), so the densities are the CEILING of the mechanism available to any LOD scheme.

`assets/vg_corpus/CORPUS.toml` sha256: `5999af9567849b87467303691ed2a3ab4f4fa120db96e2c2d684602d7ce8e2f7`

## Rows — one per (committed camera path, ladder rung) pair

| path | rung | extent | covered px | **covered %** | visible tris | mode | submitted | vis/covered | sub/covered | readback sha256 |
|---|---|---|---|---|---|---|---|---|---|---|
| orbit_mid | 0 | 512×512 | 164746 | **62.8 %** | 83373 | 0 | 13709880 | 0.506070 | 83.218288 | `ceccf5fe7a006c75` |
| orbit_mid | 1 | 1920×1080 | 1110569 | **53.6 %** | 367946 | 0 | 13709880 | 0.331313 | 12.344915 | `da4e9869f823bf26` |
| orbit_mid | 2 | 2560×1440 | 1974355 | **53.6 %** | 532341 | 0 | 13709880 | 0.269628 | 6.943979 | `f77a1c7b460f55e7` |
| orbit_mid | 3 | 3840×2160 | 4442145 | **53.6 %** | 824651 | 0 | 13709880 | 0.185643 | 3.086320 | `8b5de0b20b2836d1` |
| approach_close | 0 | 512×512 | 211369 | **80.6 %** | 33630 | 0 | 13709880 | 0.159106 | 64.862302 | `1e42045fd51ecaf0` |
| approach_close | 1 | 1920×1080 | 1605538 | **77.4 %** | 249183 | 0 | 13709880 | 0.155202 | 8.539119 | `b658ee97aa87b246` |
| approach_close | 2 | 2560×1440 | 2854287 | **77.4 %** | 343241 | 0 | 13709880 | 0.120255 | 4.803259 | `289b1fe46a7399b0` |
| approach_close | 3 | 3840×2160 | 6422334 | **77.4 %** | 454260 | 1 | 13709880 | 0.070731 | 2.134719 | `e2c489eb3396c85e` |

The **covered %** column is what rung R0b′ exists for. No floor is frozen for it here either — `[k1_instrument].representativeness_floor_status` still records that axis UNSOLVED — but the frame now looks like a frame, and the number is on the page per row instead of being absent.

### ⚠️ The framing effect, kept on the page because it is the largest single lever found

R0's ORIGINAL arrangement was one flat layer of seven assets in a void, with no inter-asset occlusion at all. R0b′ recomposed the SAME seven assets — same manifest, same hashes, same decoded triangle total, so R0b(b)'s equality is untouched — into three staggered depth layers framed to fill the view. Only the composition and the two camera poses changed; the CONTENT did not.

| | covered % | `D_est` |
|---|---|---|
| `orbit_mid`, flat layer in a void | 8.1 % | **1.0527** |
| `orbit_mid`, filled frame | see above | see above |
| `approach_close`, flat layer in a void | 22.2 % | **0.5090** |
| `approach_close`, filled frame | see above | see above |

**Filling the frame LOWERS the measured density, and the reason is geometric rather than methodological:** the same assets magnified to cover more screen have larger triangles, so fewer triangles per covered pixel. The 8 %-covered reading was therefore an OVERSTATEMENT of density produced by framing the corpus small — the direction that flatters the campaign. Both readings are of the same content; the filled-frame one is the one a rendered frame resembles.

The poses were set from the arrangement's geometry and from the stated goal of filling the frame, **before** any density was read, and the number moved AGAINST the campaign. That is the only guarantee available on this axis: §9.1 records that re-aiming a committed path is invisible to every R0 gate part, so what constrains it is commit ordering and the fact that both readings are published, not a check.

## D_est — the decisive statistic

`D_est(p) = visible_tris(p, top rung) / covered_pixels(p, decision rung)`. It is a LOWER bound (sub-pixel triangles that win no sample are absent from `visible_tris`), so it can CONFIRM density and never deny it — which is why R0 can only refute K1. Its ceiling on this ladder is 4.0 exactly, the top-rung/decision-rung area ratio.

| path | D_est | vs d_est_min |
|---|---|---|
| orbit_mid | 0.7425 | < |
| approach_close | 0.2829 | < |

**MIN over committed paths = 0.2829** against `[k1].d_est_min` = 1 ⇒ **UNDECIDED, escalate**.

MIN rather than MAX because refutation is the campaign-FAVOURABLE outcome: a favourable verdict must clear the bar on the WEAKEST committed framing, not the strongest.

⚠️ **`visible_tris` has NOT converged on orbit_mid, approach_close** (residual above `[k1_instrument].ladder_convergence_margin` = 0.05; see the table below). `D_est` is therefore an UNDERESTIMATE of unknown size — it is still rising with resolution. Per `[k1_instrument].on_not_converged_refute_direction` that would not weaken a REFUTATION, since an understatement already at or above the floor still proves density; it does mean this UNDECIDED verdict is a statement about the INSTRUMENT's reach, not a finding that the content is sparse. The disposition is to extend the ladder upward in a new plan revision, NOT to adjudicate on an underestimate.

## Cross-process reproduction — R0c(e)'s measurement, R0d(a)'s gate

| path | rung | identical | digests |
|---|---|---|---|
| orbit_mid | 3 | true | `8b5de0b20b2836d1`, `8b5de0b20b2836d1`, `8b5de0b20b2836d1` |
| approach_close | 3 | true | `e2c489eb3396c85e`, `e2c489eb3396c85e`, `e2c489eb3396c85e` |

## Convergence residual — REPORTED, not gated

Firing K1 is unreachable at R0 (no non-saturating upper bound exists), and convergence is a precondition for FIRING, never for REFUTING — so this residual gates nothing.

| path | visible_tris(top-1) | visible_tris(top) | residual | margin |
|---|---|---|---|---|
| orbit_mid | 532341 | 824651 | 0.3545 | 0.05 |
| approach_close | 343241 | 454260 | 0.2444 | 0.05 |

## Modal-bucket shift — MEASURED AND RECORDED, deliberately NOT a gate

Against the per-pair `log2` of the actual area ratio. The measured shift is a difference of INTEGER bucket indices while the targets are irrational, so a tolerance around them admits exactly one integer and would assert an integer while claiming not to. Worse, near the one-pixel censoring floor — the micro-polygon regime the census exists for — every newly visible triangle enters at bucket 0 and pushes the mode the wrong way, so the check would red hardest exactly where the premise is most strongly confirmed.

Rung [0] excluded: 512² is 1:1 while the other rungs are 16:9, so the projection is a DIFFERENT FRUSTUM and the "area scales with pixel count" premise does not hold across that step at all.

| path | pair | area ratio | target (log2) | measured | residual | tolerance |
|---|---|---|---|---|---|---|
| orbit_mid | 1→2 | 1.7778 | 0.830075 | +0 | 0.830075 | 0.35 |
| orbit_mid | 2→3 | 2.2500 | 1.169925 | +0 | 1.169925 | 0.35 |
| approach_close | 1→2 | 1.7778 | 0.830075 | +0 | 0.830075 | 0.35 |
| approach_close | 2→3 | 2.2500 | 1.169925 | +1 | 0.169925 | 0.35 |

## What this census does NOT decide

* **K1 cannot be FIRED here.** `D_est` is a lower bound; firing needs an upper bound on VISIBLE density that is demonstrably non-saturating, which is an unsolved design problem (`[k1].k1_fire_instrument_status`).
* **There is no representativeness floor.** The non-degeneracy floors are EMPTY-FRAME guards; `D_est` is scale-free, so no floor on that axis can carry representativeness. It needs a floor on covered FRACTION, and R0 does not have one.
* **Camera-path DEFINITIONS are hashed by nothing here.** Membership and cardinality are gated; re-aiming a committed path is neither, and no R0 gate part sees it.
* **Corpus placement is normalised.** Each asset is scaled to a unit cube, which equalises screen share — a choice about what the corpus represents, recorded rather than claimed away.
