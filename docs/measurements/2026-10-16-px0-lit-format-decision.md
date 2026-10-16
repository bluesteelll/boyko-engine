# PX0 — the `lit` format decision (render queue item 8)

> Render wave PX0, `plan/H3-PX0-instruments-guards.md` Scope 3 / 3b with the binding amendments
> (`04-AMENDMENTS-R6.md` rev-6.2 H3 row: the reported Punchy arm) and `px0/cut.md` §1.3 #2, #3, #6.
> **Status: the classes, the content model, the expected table and the decision table are
> pre-registered here at the model commit; §7 classification, §8 device arms and §9 decision are
> pending GL-PX0-P** (the round-1 tester's device leg; PX0-14 fills them from its `probe.toml`).

## 1. Classes (OPEN-ITEMS #2; `critique_verify_r5.md` I1)

The model family (`boyko_shaderdsl::pixfmt::Member`) and the equivalence classes a device path can
classify as (`pixfmt::PathClass`, `pixfmt::classes_for`). B10G11R11's channels are a sub-grid of
binary16 (same 5-bit exponent and bias, fewer mantissa bits), so truncating through `f16` equals
truncating directly (`Z16 ≡ Z`, every path); on an RGBA16F target rounding to `f16` first is one
rounding twice (`N16 ≡ N`).

| Format | Path | Classes (merged members) |
|---|---|---|
| B10G11R11 | store, attachment, clear | `N` · `N16` · `Z`(≡`Z16`) |
| B10G11R11 | blend `ONE+ONE`, blend premultiplied over | `N` · `N16` · `Npre` · `Z`(≡`Z16`) · `Zpre` |
| RGBA16F | store, attachment, clear | `N`(≡`N16`) · `Z`(≡`Z16`) |
| RGBA16F | both blends | `N`(≡`N16`) · `Npre` · `Z`(≡`Z16`) · `Zpre` |

`Zpre ≢ Z` — witness on the blue channel: `dst = 2⁻¹¹`, `src = 1 + 1/32 − 2⁻¹²` ⇒ `Z` stores
`1 + 1/32`, `Zpre` stores `1` (`pixfmt` test `zpre_differs_from_z_witness`).

A path's class is the unique class whose prediction equals every device output on the path's input
set; zero matches ⇒ `Unmodelled:<max error in ULP>`; more than one match is impossible by
construction (`b10g11r11_chain_model::probe_inputs_separate_classes`) and is a RED of the input set
at run time, never a device verdict.

**Single-write bound per class** (AD-19 (a); `PathClass::store_bound_ulp`): `N` ±½ ULP; `N16`
±(½ + 2^(m−11)) ULP (0.516 UFLOAT10, 0.531 UFLOAT11); `Z` [−1, 0] ULP; `Unmodelled` ± its recorded
maximum; RGBA16F `N` ±½, `Z` [−1, 0] binary16 ULP.

## 2. Content model (H3 3b; D-P2)

Every per-pixel, per-layer value is drawn on the host from a seeded SplitMix64 (per arm: FNV-1a of
the arm's label; per pixel: `seed ^ px · 0xD1B54A32D192ED03`) and uploaded; the device computes
nothing but A3's `lit · t + s`. Region: 64 × 64 = 4096 independent stacks per arm. Backgrounds: black
and 0.18 grey, uploaded as the nearest-even code (identical bits for every class).

| Arm | Path | k | Per-layer value | Decisive |
|---|---|---|---|---|
| A1-S1 | blend `ONE + ONE` | 16, 64 | `c · m · u`, `u ∼ U[0.5, 1.5]`; `c` ∈ {grey (1, 1, 1), fire (1, 0.45, 0.12)}; `m = 3.2 / k` | yes |
| A1-S2 | blend `ONE + ONE` | 16, 64 | as A1-S1, `u ∼ U[0, 2]` | yes |
| A2 | premultiplied over | 16, 64 | alpha `ā · u_a`, colour `c̄ · u_c`, `u_a, u_c ∼ U[0.5, 1.5]`; `ā` ∈ {0.1, 0.3}, `c̄` ∈ {0.05, 0.2, 0.5} grey | yes |
| A3 | compute storage RMW | 4 | `lit ← lit · t + s`, `t ∼ U[0.8, 1]`, `s ∼ U[0, 0.05]` per pixel and pass | yes |
| A3-blend | premultiplied over | 4 | A3's `t`, `s` as `src = (s, 1 − t)` — exact-math identical to `lit · t + s` | Row 7 only |
| A2-flat | premultiplied over | 64 | colour fixed at `c̄`, alpha jittered as A2 | no — K3 |
| U-eq / U-near | both blends | 64 | equal layers / ±5 % | no — K3 |

**The metric (pre-registered).** `D` = AgX (`crates/boyko_rhi_vulkan/tests/lit_format/agx_f64.rs`,
three.js `AgXToneMapping`) followed by the OETF `pow(x, 1/2.2)`, ×255, unquantised. Per arm and
channel `c`, `Δ_c(j)` = the region mean of `D(2^j · lit_device) − D(2^j · lit_exact)`, `lit_exact`
the `f64` composite of the same uploaded values from the decoded background; `chroma(j) =
max_{c,c'} |Δ_c(j) − Δ_c'(j)|`. The arm's numbers are `max_{j,c} |Δ_c(j)|` and `max_j chroma(j)` over
`j ∈ [−8, 4]` — each maximised over the placement. **An arm passes iff both are ≤ 2 LSB.** Hill ACES
(`boyko_shaderdsl/tests/post_f64`) is reported beside AgX for every row (K9 reads it).

**Punchy (rev-6.2 H3 row) — reported, never decisive:** Filament's `agxLook` PUNCHY (slope 1,
offset 0, power 1.35, saturation 1.4 about the Rec.709 luma of the pre-CDL value) between the
sigmoid and the outset, on the tightest decisive arm's content (the decisive arm on B10G11R11 under
`N` whose AgX metric is largest), under every class of its path. P6's `CinematicPost` look sign-off
reads it.

**The placement argument** holds over the normal range (`b10g11r11_chain_model::placement_invariance`,
`j ∈ [−5, 4]` on the A1-S1 arms): below the smallest normal (2⁻¹⁴) power-of-two scaling is not exact
(`px0/critique.md` O3); A1-S2's `u ∼ U[0, 2]` puts subnormal first layers at every placement, which the
metric evaluates as they are.

## 3. Classification input sets (`lit_format::class_inputs`)

Per channel, per binade `2^e` (`e ∈ [−14, 15]`), up to 64 intervals between consecutive
representable points (every interval when the binade has no more), and per interval: the exact
midpoint, the midpoint ± ½ binary16 ULP (the inputs a double rounding moves), and the three-quarter
point. The interval ending at the overflow point is excluded, so **every input is finite and at or
below the channel's largest finite value** (65024 UFLOAT11, 64512 UFLOAT10, 65504 binary16) and a
device's overflow rule (clamp or Inf) never enters classification. The host model's own overflow
rule, stated: a finite value past the largest finite code encodes to Inf under nearest-even and to
the largest finite code under truncation; NaN stays NaN; a negative value encodes to 0 on the
unsigned encodings.

- **Blend `ONE + ONE`:** `dst = trunc(value / 4)` (uploaded bits), `src = value − dst`.
- **Blend premultiplied over:** `dst = trunc(value)`, `alpha = ½`, `src = value − dst · ½` — dyadic
  alpha, so every product and sum is exact in f32 and no FMA or factor precision moves an input
  (`px0/critique.md` O4).
- **Clear** (`px0/critique.md` open question 1): one attachment clear per value — a dynamic-rendering
  pass with `loadOp = CLEAR` over a one-texel render area at the value's texel of the 256-wide
  classification image (`b10g11r11_rounding_probe.rs`, the clear branch of `Rig::run`), the route
  the engine's own `lit` clears take (`passes/forward.rs`: the `lit` colour attachment's
  `VK_ATTACHMENT_LOAD_OP_CLEAR` with `FORWARD_LIT_CLEAR` as its clear value); no
  `vkCmdClearColorImage` is recorded. The clear set is the subset at binades
  `{−14, −10, −6, −2, −1, 0, 1, 2, 6, 10, 14, 15}` with 8 intervals each;
  `probe_inputs_separate_classes` runs on exactly that set.
  *Erratum (PX0 triage r1 W6):* this paragraph read "one `vkCmdClearColorImage` per value, one array
  layer per value (≤ 2048 layers)" at the model commit, a route the probe never ran. The frozen
  model's own comment on `lit_format::clear_binades` (`crates/boyko_rhi_vulkan/tests/lit_format/mod.rs`)
  still says so: it stays unedited, because §6's freeze check diffs the model's sources from the
  model commit to the decision commit, and the subset it returns is the one the probe clears.

## 4. Expected (pinned — `b10g11r11_chain_model::expected_table_is_pinned`)

The model's numbers, not H3's [E] figures, are the pre-registration. Verdict = AgX under the row's
class (`reported` for the non-decisive families).

<!-- px0-expected:begin -->
| arm | target | class | AgX max\|Δ\| (channel, placement) | AgX chroma | ACES-Hill max\|Δ\| | ACES-Hill chroma | verdict (AgX) |
|---|---|---|---|---|---|---|---|
| A1-S1 k16 grey bg0 | b10g11r11 | N | 0.06 (b, j=-4) | 0.08 | 0.08 | 0.12 | pass |
| A1-S1 k16 grey bg0 | b10g11r11 | N16 | 0.06 (b, j=-4) | 0.08 | 0.08 | 0.11 | pass |
| A1-S1 k16 grey bg0 | b10g11r11 | Npre | 0.05 (b, j=-4) | 0.07 | 0.07 | 0.10 | pass |
| A1-S1 k16 grey bg0 | b10g11r11 | Z | 4.44 (b, j=-4) | 2.27 | 6.72 | 3.22 | FAIL |
| A1-S1 k16 grey bg0 | b10g11r11 | Zpre | 4.45 (b, j=-4) | 2.27 | 6.73 | 3.22 | FAIL |
| A1-S1 k16 grey bg0 | rgba16f | N | 0.00 (r, j=-4) | 0.00 | 0.00 | 0.00 | pass |
| A1-S1 k16 grey bg0 | rgba16f | Npre | 0.00 (r, j=-4) | 0.00 | 0.00 | 0.00 | pass |
| A1-S1 k16 grey bg0 | rgba16f | Z | 0.14 (r, j=-4) | 0.00 | 0.22 | 0.00 | pass |
| A1-S1 k16 grey bg0 | rgba16f | Zpre | 0.14 (r, j=-4) | 0.00 | 0.22 | 0.00 | pass |
| A1-S1 k16 fire bg0 | b10g11r11 | N | 0.03 (g, j=-3) | 0.03 | 0.06 | 0.05 | pass |
| A1-S1 k16 fire bg0 | b10g11r11 | N16 | 0.03 (r, j=-4) | 0.03 | 0.05 | 0.06 | pass |
| A1-S1 k16 fire bg0 | b10g11r11 | Npre | 0.04 (g, j=-3) | 0.03 | 0.07 | 0.06 | pass |
| A1-S1 k16 fire bg0 | b10g11r11 | Z | 3.40 (b, j=-2) | 2.11 | 5.72 | 4.66 | FAIL |
| A1-S1 k16 fire bg0 | b10g11r11 | Zpre | 3.41 (b, j=-2) | 2.11 | 5.73 | 4.67 | FAIL |
| A1-S1 k16 fire bg0 | rgba16f | N | 0.00 (r, j=-4) | 0.00 | 0.00 | 0.00 | pass |
| A1-S1 k16 fire bg0 | rgba16f | Npre | 0.00 (r, j=-3) | 0.00 | 0.00 | 0.00 | pass |
| A1-S1 k16 fire bg0 | rgba16f | Z | 0.14 (b, j=-1) | 0.08 | 0.23 | 0.17 | pass |
| A1-S1 k16 fire bg0 | rgba16f | Zpre | 0.14 (b, j=-1) | 0.08 | 0.23 | 0.17 | pass |
| A1-S1 k64 grey bg0 | b10g11r11 | N | 1.21 (b, j=-4) | 1.54 | 1.68 | 2.16 | pass |
| A1-S1 k64 grey bg0 | b10g11r11 | N16 | 1.17 (b, j=-4) | 1.51 | 1.62 | 2.12 | pass |
| A1-S1 k64 grey bg0 | b10g11r11 | Npre | 1.14 (b, j=-4) | 1.48 | 1.59 | 2.08 | pass |
| A1-S1 k64 grey bg0 | b10g11r11 | Z | 16.91 (b, j=-4) | 8.57 | 25.70 | 12.24 | FAIL |
| A1-S1 k64 grey bg0 | b10g11r11 | Zpre | 16.91 (b, j=-4) | 8.57 | 25.70 | 12.24 | FAIL |
| A1-S1 k64 grey bg0 | rgba16f | N | 0.00 (r, j=-4) | 0.00 | 0.00 | 0.00 | pass |
| A1-S1 k64 grey bg0 | rgba16f | Npre | 0.00 (r, j=-4) | 0.00 | 0.00 | 0.00 | pass |
| A1-S1 k64 grey bg0 | rgba16f | Z | 0.53 (r, j=-4) | 0.00 | 0.84 | 0.00 | pass |
| A1-S1 k64 grey bg0 | rgba16f | Zpre | 0.53 (r, j=-4) | 0.00 | 0.84 | 0.00 | pass |
| A1-S1 k64 fire bg0 | b10g11r11 | N | 0.72 (b, j=-2) | 1.10 | 1.48 | 1.87 | pass |
| A1-S1 k64 fire bg0 | b10g11r11 | N16 | 0.71 (b, j=-2) | 1.09 | 1.46 | 1.85 | pass |
| A1-S1 k64 fire bg0 | b10g11r11 | Npre | 0.68 (b, j=-2) | 1.07 | 1.41 | 1.81 | pass |
| A1-S1 k64 fire bg0 | b10g11r11 | Z | 12.86 (b, j=-2) | 8.05 | 21.40 | 18.25 | FAIL |
| A1-S1 k64 fire bg0 | b10g11r11 | Zpre | 12.86 (b, j=-2) | 8.05 | 21.40 | 18.25 | FAIL |
| A1-S1 k64 fire bg0 | rgba16f | N | 0.00 (r, j=-3) | 0.00 | 0.00 | 0.00 | pass |
| A1-S1 k64 fire bg0 | rgba16f | Npre | 0.00 (r, j=-3) | 0.00 | 0.00 | 0.00 | pass |
| A1-S1 k64 fire bg0 | rgba16f | Z | 0.56 (b, j=-1) | 0.29 | 0.90 | 0.65 | pass |
| A1-S1 k64 fire bg0 | rgba16f | Zpre | 0.56 (b, j=-1) | 0.29 | 0.90 | 0.65 | pass |
| A1-S2 k16 grey bg0 | b10g11r11 | N | 0.08 (b, j=-4) | 0.07 | 0.12 | 0.10 | pass |
| A1-S2 k16 grey bg0 | b10g11r11 | N16 | 0.08 (b, j=-4) | 0.07 | 0.12 | 0.10 | pass |
| A1-S2 k16 grey bg0 | b10g11r11 | Npre | 0.09 (b, j=-4) | 0.08 | 0.13 | 0.12 | pass |
| A1-S2 k16 grey bg0 | b10g11r11 | Z | 4.25 (b, j=-4) | 2.10 | 6.40 | 2.97 | FAIL |
| A1-S2 k16 grey bg0 | b10g11r11 | Zpre | 4.26 (b, j=-4) | 2.11 | 6.43 | 2.99 | FAIL |
| A1-S2 k16 grey bg0 | rgba16f | N | 0.00 (r, j=-4) | 0.00 | 0.00 | 0.00 | pass |
| A1-S2 k16 grey bg0 | rgba16f | Npre | 0.00 (r, j=-4) | 0.00 | 0.00 | 0.00 | pass |
| A1-S2 k16 grey bg0 | rgba16f | Z | 0.14 (r, j=-4) | 0.00 | 0.21 | 0.00 | pass |
| A1-S2 k16 grey bg0 | rgba16f | Zpre | 0.14 (r, j=-4) | 0.00 | 0.21 | 0.00 | pass |
| A1-S2 k16 fire bg0 | b10g11r11 | N | 0.03 (b, j=-1) | 0.02 | 0.05 | 0.05 | pass |
| A1-S2 k16 fire bg0 | b10g11r11 | N16 | 0.03 (b, j=-2) | 0.03 | 0.06 | 0.06 | pass |
| A1-S2 k16 fire bg0 | b10g11r11 | Npre | 0.04 (b, j=-2) | 0.03 | 0.07 | 0.07 | pass |
| A1-S2 k16 fire bg0 | b10g11r11 | Z | 3.33 (b, j=-2) | 2.05 | 5.57 | 4.53 | FAIL |
| A1-S2 k16 fire bg0 | b10g11r11 | Zpre | 3.34 (b, j=-2) | 2.06 | 5.59 | 4.54 | FAIL |
| A1-S2 k16 fire bg0 | rgba16f | N | 0.00 (r, j=-4) | 0.00 | 0.00 | 0.00 | pass |
| A1-S2 k16 fire bg0 | rgba16f | Npre | 0.00 (r, j=-3) | 0.00 | 0.00 | 0.00 | pass |
| A1-S2 k16 fire bg0 | rgba16f | Z | 0.14 (r, j=-4) | 0.07 | 0.23 | 0.16 | pass |
| A1-S2 k16 fire bg0 | rgba16f | Zpre | 0.14 (r, j=-4) | 0.07 | 0.23 | 0.16 | pass |
| A1-S2 k64 grey bg0 | b10g11r11 | N | 1.16 (b, j=-4) | 1.04 | 1.69 | 1.47 | pass |
| A1-S2 k64 grey bg0 | b10g11r11 | N16 | 1.19 (b, j=-4) | 1.06 | 1.73 | 1.50 | pass |
| A1-S2 k64 grey bg0 | b10g11r11 | Npre | 1.21 (b, j=-4) | 1.08 | 1.76 | 1.52 | pass |
| A1-S2 k64 grey bg0 | b10g11r11 | Z | 14.88 (b, j=-4) | 6.76 | 22.68 | 9.64 | FAIL |
| A1-S2 k64 grey bg0 | b10g11r11 | Zpre | 14.88 (b, j=-4) | 6.76 | 22.69 | 9.64 | FAIL |
| A1-S2 k64 grey bg0 | rgba16f | N | 0.00 (r, j=-4) | 0.00 | 0.00 | 0.00 | pass |
| A1-S2 k64 grey bg0 | rgba16f | Npre | 0.00 (r, j=-4) | 0.00 | 0.00 | 0.00 | pass |
| A1-S2 k64 grey bg0 | rgba16f | Z | 0.53 (r, j=-4) | 0.00 | 0.83 | 0.00 | pass |
| A1-S2 k64 grey bg0 | rgba16f | Zpre | 0.53 (r, j=-4) | 0.00 | 0.83 | 0.00 | pass |
| A1-S2 k64 fire bg0 | b10g11r11 | N | 0.81 (b, j=-2) | 0.71 | 1.51 | 1.42 | pass |
| A1-S2 k64 fire bg0 | b10g11r11 | N16 | 0.82 (b, j=-2) | 0.71 | 1.52 | 1.43 | pass |
| A1-S2 k64 fire bg0 | b10g11r11 | Npre | 0.84 (b, j=-2) | 0.73 | 1.56 | 1.46 | pass |
| A1-S2 k64 fire bg0 | b10g11r11 | Z | 11.79 (b, j=-1) | 7.10 | 19.43 | 16.21 | FAIL |
| A1-S2 k64 fire bg0 | b10g11r11 | Zpre | 11.79 (b, j=-1) | 7.10 | 19.44 | 16.21 | FAIL |
| A1-S2 k64 fire bg0 | rgba16f | N | 0.00 (g, j=-3) | 0.00 | 0.00 | 0.00 | pass |
| A1-S2 k64 fire bg0 | rgba16f | Npre | 0.00 (g, j=-2) | 0.00 | 0.00 | 0.00 | pass |
| A1-S2 k64 fire bg0 | rgba16f | Z | 0.55 (b, j=-1) | 0.29 | 0.90 | 0.64 | pass |
| A1-S2 k64 fire bg0 | rgba16f | Zpre | 0.55 (b, j=-1) | 0.29 | 0.90 | 0.64 | pass |
| A2 k16 grey0.05 a0.1 bg0 | b10g11r11 | N | 0.02 (b, j=3) | 0.01 | 0.03 | 0.02 | pass |
| A2 k16 grey0.05 a0.1 bg0 | b10g11r11 | N16 | 0.02 (b, j=3) | 0.02 | 0.04 | 0.02 | pass |
| A2 k16 grey0.05 a0.1 bg0 | b10g11r11 | Npre | 0.01 (b, j=3) | 0.00 | 0.02 | 0.01 | pass |
| A2 k16 grey0.05 a0.1 bg0 | b10g11r11 | Z | 3.46 (b, j=2) | 1.72 | 5.24 | 2.43 | FAIL |
| A2 k16 grey0.05 a0.1 bg0 | b10g11r11 | Zpre | 3.95 (b, j=2) | 1.96 | 5.98 | 2.78 | FAIL |
| A2 k16 grey0.05 a0.1 bg0 | rgba16f | N | 0.00 (r, j=1) | 0.00 | 0.00 | 0.00 | pass |
| A2 k16 grey0.05 a0.1 bg0 | rgba16f | Npre | 0.00 (r, j=-2) | 0.00 | 0.00 | 0.00 | pass |
| A2 k16 grey0.05 a0.1 bg0 | rgba16f | Z | 0.11 (r, j=2) | 0.00 | 0.17 | 0.00 | pass |
| A2 k16 grey0.05 a0.1 bg0 | rgba16f | Zpre | 0.13 (r, j=2) | 0.00 | 0.20 | 0.00 | pass |
| A2 k16 grey0.2 a0.1 bg0 | b10g11r11 | N | 0.02 (b, j=1) | 0.02 | 0.03 | 0.02 | pass |
| A2 k16 grey0.2 a0.1 bg0 | b10g11r11 | N16 | 0.03 (b, j=1) | 0.02 | 0.04 | 0.03 | pass |
| A2 k16 grey0.2 a0.1 bg0 | b10g11r11 | Npre | 0.02 (b, j=1) | 0.02 | 0.04 | 0.03 | pass |
| A2 k16 grey0.2 a0.1 bg0 | b10g11r11 | Z | 3.48 (b, j=0) | 1.73 | 5.26 | 2.45 | FAIL |
| A2 k16 grey0.2 a0.1 bg0 | b10g11r11 | Zpre | 3.98 (b, j=0) | 1.98 | 6.02 | 2.80 | FAIL |
| A2 k16 grey0.2 a0.1 bg0 | rgba16f | N | 0.00 (r, j=0) | 0.00 | 0.00 | 0.00 | pass |
| A2 k16 grey0.2 a0.1 bg0 | rgba16f | Npre | 0.00 (r, j=0) | 0.00 | 0.00 | 0.00 | pass |
| A2 k16 grey0.2 a0.1 bg0 | rgba16f | Z | 0.11 (r, j=0) | 0.00 | 0.18 | 0.00 | pass |
| A2 k16 grey0.2 a0.1 bg0 | rgba16f | Zpre | 0.13 (r, j=0) | 0.00 | 0.20 | 0.00 | pass |
| A2 k16 grey0.5 a0.1 bg0 | b10g11r11 | N | 0.01 (r, j=-1) | 0.00 | 0.02 | 0.01 | pass |
| A2 k16 grey0.5 a0.1 bg0 | b10g11r11 | N16 | 0.01 (r, j=-1) | 0.00 | 0.02 | 0.00 | pass |
| A2 k16 grey0.5 a0.1 bg0 | b10g11r11 | Npre | 0.01 (r, j=-1) | 0.00 | 0.02 | 0.01 | pass |
| A2 k16 grey0.5 a0.1 bg0 | b10g11r11 | Z | 3.22 (b, j=-1) | 1.63 | 4.86 | 2.31 | FAIL |
| A2 k16 grey0.5 a0.1 bg0 | b10g11r11 | Zpre | 3.75 (b, j=-1) | 1.90 | 5.65 | 2.69 | FAIL |
| A2 k16 grey0.5 a0.1 bg0 | rgba16f | N | 0.00 (r, j=-1) | 0.00 | 0.00 | 0.00 | pass |
| A2 k16 grey0.5 a0.1 bg0 | rgba16f | Npre | 0.00 (r, j=-3) | 0.00 | 0.00 | 0.00 | pass |
| A2 k16 grey0.5 a0.1 bg0 | rgba16f | Z | 0.10 (r, j=-1) | 0.00 | 0.15 | 0.00 | pass |
| A2 k16 grey0.5 a0.1 bg0 | rgba16f | Zpre | 0.11 (r, j=-1) | 0.00 | 0.18 | 0.00 | pass |
| A2 k16 grey0.05 a0.3 bg0 | b10g11r11 | N | 0.00 (r, j=2) | 0.00 | 0.00 | 0.00 | pass |
| A2 k16 grey0.05 a0.3 bg0 | b10g11r11 | N16 | 0.00 (b, j=2) | 0.00 | 0.00 | 0.00 | pass |
| A2 k16 grey0.05 a0.3 bg0 | b10g11r11 | Npre | 0.01 (b, j=2) | 0.01 | 0.01 | 0.02 | pass |
| A2 k16 grey0.05 a0.3 bg0 | b10g11r11 | Z | 1.53 (b, j=2) | 0.77 | 2.30 | 1.08 | pass |
| A2 k16 grey0.05 a0.3 bg0 | b10g11r11 | Zpre | 2.04 (b, j=2) | 1.03 | 3.08 | 1.45 | FAIL |
| A2 k16 grey0.05 a0.3 bg0 | rgba16f | N | 0.00 (r, j=3) | 0.00 | 0.00 | 0.00 | pass |
| A2 k16 grey0.05 a0.3 bg0 | rgba16f | Npre | 0.00 (r, j=2) | 0.00 | 0.00 | 0.00 | pass |
| A2 k16 grey0.05 a0.3 bg0 | rgba16f | Z | 0.05 (r, j=2) | 0.00 | 0.08 | 0.00 | pass |
| A2 k16 grey0.05 a0.3 bg0 | rgba16f | Zpre | 0.06 (r, j=2) | 0.00 | 0.10 | 0.00 | pass |
| A2 k16 grey0.2 a0.3 bg0 | b10g11r11 | N | 0.01 (b, j=0) | 0.00 | 0.01 | 0.01 | pass |
| A2 k16 grey0.2 a0.3 bg0 | b10g11r11 | N16 | 0.00 (b, j=0) | 0.01 | 0.01 | 0.01 | pass |
| A2 k16 grey0.2 a0.3 bg0 | b10g11r11 | Npre | 0.01 (r, j=0) | 0.00 | 0.01 | 0.00 | pass |
| A2 k16 grey0.2 a0.3 bg0 | b10g11r11 | Z | 1.53 (b, j=0) | 0.77 | 2.30 | 1.09 | pass |
| A2 k16 grey0.2 a0.3 bg0 | b10g11r11 | Zpre | 2.05 (b, j=0) | 1.04 | 3.08 | 1.46 | FAIL |
| A2 k16 grey0.2 a0.3 bg0 | rgba16f | N | 0.00 (r, j=0) | 0.00 | 0.00 | 0.00 | pass |
| A2 k16 grey0.2 a0.3 bg0 | rgba16f | Npre | 0.00 (r, j=-1) | 0.00 | 0.00 | 0.00 | pass |
| A2 k16 grey0.2 a0.3 bg0 | rgba16f | Z | 0.05 (r, j=0) | 0.00 | 0.08 | 0.00 | pass |
| A2 k16 grey0.2 a0.3 bg0 | rgba16f | Zpre | 0.06 (r, j=0) | 0.00 | 0.10 | 0.00 | pass |
| A2 k16 grey0.5 a0.3 bg0 | b10g11r11 | N | 0.01 (b, j=-1) | 0.00 | 0.01 | 0.01 | pass |
| A2 k16 grey0.5 a0.3 bg0 | b10g11r11 | N16 | 0.01 (b, j=-1) | 0.01 | 0.02 | 0.01 | pass |
| A2 k16 grey0.5 a0.3 bg0 | b10g11r11 | Npre | 0.02 (b, j=-1) | 0.02 | 0.04 | 0.03 | pass |
| A2 k16 grey0.5 a0.3 bg0 | b10g11r11 | Z | 1.65 (b, j=-1) | 0.81 | 2.40 | 1.10 | pass |
| A2 k16 grey0.5 a0.3 bg0 | b10g11r11 | Zpre | 2.15 (b, j=-1) | 1.07 | 3.13 | 1.46 | FAIL |
| A2 k16 grey0.5 a0.3 bg0 | rgba16f | N | 0.00 (r, j=-2) | 0.00 | 0.00 | 0.00 | pass |
| A2 k16 grey0.5 a0.3 bg0 | rgba16f | Npre | 0.00 (r, j=-1) | 0.00 | 0.00 | 0.00 | pass |
| A2 k16 grey0.5 a0.3 bg0 | rgba16f | Z | 0.05 (r, j=-1) | 0.00 | 0.08 | 0.00 | pass |
| A2 k16 grey0.5 a0.3 bg0 | rgba16f | Zpre | 0.07 (r, j=-1) | 0.00 | 0.11 | 0.00 | pass |
| A2 k64 grey0.05 a0.1 bg0 | b10g11r11 | N | 0.01 (b, j=2) | 0.00 | 0.01 | 0.00 | pass |
| A2 k64 grey0.05 a0.1 bg0 | b10g11r11 | N16 | 0.01 (r, j=2) | 0.01 | 0.02 | 0.01 | pass |
| A2 k64 grey0.05 a0.1 bg0 | b10g11r11 | Npre | 0.00 (r, j=2) | 0.00 | 0.01 | 0.01 | pass |
| A2 k64 grey0.05 a0.1 bg0 | b10g11r11 | Z | 4.72 (b, j=2) | 2.45 | 7.14 | 3.47 | FAIL |
| A2 k64 grey0.05 a0.1 bg0 | b10g11r11 | Zpre | 5.28 (b, j=2) | 2.73 | 7.99 | 3.88 | FAIL |
| A2 k64 grey0.05 a0.1 bg0 | rgba16f | N | 0.00 (r, j=3) | 0.00 | 0.00 | 0.00 | pass |
| A2 k64 grey0.05 a0.1 bg0 | rgba16f | Npre | 0.00 (r, j=2) | 0.00 | 0.00 | 0.00 | pass |
| A2 k64 grey0.05 a0.1 bg0 | rgba16f | Z | 0.14 (r, j=2) | 0.00 | 0.22 | 0.00 | pass |
| A2 k64 grey0.05 a0.1 bg0 | rgba16f | Zpre | 0.16 (r, j=2) | 0.00 | 0.25 | 0.00 | pass |
| A2 k64 grey0.2 a0.1 bg0 | b10g11r11 | N | 0.00 (b, j=1) | 0.00 | 0.01 | 0.00 | pass |
| A2 k64 grey0.2 a0.1 bg0 | b10g11r11 | N16 | 0.01 (b, j=0) | 0.01 | 0.01 | 0.01 | pass |
| A2 k64 grey0.2 a0.1 bg0 | b10g11r11 | Npre | 0.00 (r, j=0) | 0.01 | 0.01 | 0.01 | pass |
| A2 k64 grey0.2 a0.1 bg0 | b10g11r11 | Z | 4.69 (b, j=0) | 2.42 | 7.10 | 3.43 | FAIL |
| A2 k64 grey0.2 a0.1 bg0 | b10g11r11 | Zpre | 5.25 (b, j=0) | 2.70 | 7.94 | 3.84 | FAIL |
| A2 k64 grey0.2 a0.1 bg0 | rgba16f | N | 0.00 (r, j=0) | 0.00 | 0.00 | 0.00 | pass |
| A2 k64 grey0.2 a0.1 bg0 | rgba16f | Npre | 0.00 (r, j=1) | 0.00 | 0.00 | 0.00 | pass |
| A2 k64 grey0.2 a0.1 bg0 | rgba16f | Z | 0.14 (r, j=0) | 0.00 | 0.22 | 0.00 | pass |
| A2 k64 grey0.2 a0.1 bg0 | rgba16f | Zpre | 0.16 (r, j=0) | 0.00 | 0.25 | 0.00 | pass |
| A2 k64 grey0.5 a0.1 bg0 | b10g11r11 | N | 0.05 (b, j=-1) | 0.02 | 0.08 | 0.03 | pass |
| A2 k64 grey0.5 a0.1 bg0 | b10g11r11 | N16 | 0.06 (b, j=-1) | 0.02 | 0.09 | 0.03 | pass |
| A2 k64 grey0.5 a0.1 bg0 | b10g11r11 | Npre | 0.05 (b, j=-1) | 0.02 | 0.07 | 0.03 | pass |
| A2 k64 grey0.5 a0.1 bg0 | b10g11r11 | Z | 4.07 (b, j=-1) | 1.84 | 6.00 | 2.55 | FAIL |
| A2 k64 grey0.5 a0.1 bg0 | b10g11r11 | Zpre | 4.56 (b, j=-1) | 2.11 | 6.73 | 2.92 | FAIL |
| A2 k64 grey0.5 a0.1 bg0 | rgba16f | N | 0.00 (r, j=-1) | 0.00 | 0.00 | 0.00 | pass |
| A2 k64 grey0.5 a0.1 bg0 | rgba16f | Npre | 0.00 (r, j=-1) | 0.00 | 0.00 | 0.00 | pass |
| A2 k64 grey0.5 a0.1 bg0 | rgba16f | Z | 0.16 (r, j=-1) | 0.00 | 0.25 | 0.00 | pass |
| A2 k64 grey0.5 a0.1 bg0 | rgba16f | Zpre | 0.18 (r, j=-1) | 0.00 | 0.27 | 0.00 | pass |
| A2 k64 grey0.05 a0.3 bg0 | b10g11r11 | N | 0.01 (r, j=2) | 0.01 | 0.01 | 0.02 | pass |
| A2 k64 grey0.05 a0.3 bg0 | b10g11r11 | N16 | 0.00 (r, j=2) | 0.01 | 0.01 | 0.01 | pass |
| A2 k64 grey0.05 a0.3 bg0 | b10g11r11 | Npre | 0.00 (r, j=2) | 0.01 | 0.00 | 0.01 | pass |
| A2 k64 grey0.05 a0.3 bg0 | b10g11r11 | Z | 1.54 (b, j=2) | 0.77 | 2.32 | 1.09 | pass |
| A2 k64 grey0.05 a0.3 bg0 | b10g11r11 | Zpre | 2.06 (b, j=2) | 1.04 | 3.10 | 1.46 | FAIL |
| A2 k64 grey0.05 a0.3 bg0 | rgba16f | N | 0.00 (r, j=2) | 0.00 | 0.00 | 0.00 | pass |
| A2 k64 grey0.05 a0.3 bg0 | rgba16f | Npre | 0.00 (r, j=2) | 0.00 | 0.00 | 0.00 | pass |
| A2 k64 grey0.05 a0.3 bg0 | rgba16f | Z | 0.05 (r, j=2) | 0.00 | 0.08 | 0.00 | pass |
| A2 k64 grey0.05 a0.3 bg0 | rgba16f | Zpre | 0.06 (r, j=2) | 0.00 | 0.10 | 0.00 | pass |
| A2 k64 grey0.2 a0.3 bg0 | b10g11r11 | N | 0.01 (r, j=0) | 0.00 | 0.01 | 0.01 | pass |
| A2 k64 grey0.2 a0.3 bg0 | b10g11r11 | N16 | 0.01 (r, j=0) | 0.01 | 0.01 | 0.01 | pass |
| A2 k64 grey0.2 a0.3 bg0 | b10g11r11 | Npre | 0.02 (b, j=0) | 0.02 | 0.02 | 0.03 | pass |
| A2 k64 grey0.2 a0.3 bg0 | b10g11r11 | Z | 1.54 (b, j=0) | 0.77 | 2.31 | 1.09 | pass |
| A2 k64 grey0.2 a0.3 bg0 | b10g11r11 | Zpre | 2.05 (b, j=0) | 1.03 | 3.09 | 1.46 | FAIL |
| A2 k64 grey0.2 a0.3 bg0 | rgba16f | N | 0.00 (r, j=-1) | 0.00 | 0.00 | 0.00 | pass |
| A2 k64 grey0.2 a0.3 bg0 | rgba16f | Npre | 0.00 (r, j=1) | 0.00 | 0.00 | 0.00 | pass |
| A2 k64 grey0.2 a0.3 bg0 | rgba16f | Z | 0.05 (r, j=0) | 0.00 | 0.08 | 0.00 | pass |
| A2 k64 grey0.2 a0.3 bg0 | rgba16f | Zpre | 0.06 (r, j=0) | 0.00 | 0.10 | 0.00 | pass |
| A2 k64 grey0.5 a0.3 bg0 | b10g11r11 | N | 0.01 (b, j=-1) | 0.00 | 0.01 | 0.00 | pass |
| A2 k64 grey0.5 a0.3 bg0 | b10g11r11 | N16 | 0.01 (b, j=-1) | 0.00 | 0.01 | 0.01 | pass |
| A2 k64 grey0.5 a0.3 bg0 | b10g11r11 | Npre | 0.02 (b, j=-1) | 0.02 | 0.02 | 0.02 | pass |
| A2 k64 grey0.5 a0.3 bg0 | b10g11r11 | Z | 1.69 (b, j=-1) | 0.84 | 2.47 | 1.14 | pass |
| A2 k64 grey0.5 a0.3 bg0 | b10g11r11 | Zpre | 2.19 (b, j=-1) | 1.08 | 3.19 | 1.48 | FAIL |
| A2 k64 grey0.5 a0.3 bg0 | rgba16f | N | 0.00 (r, j=-1) | 0.00 | 0.00 | 0.00 | pass |
| A2 k64 grey0.5 a0.3 bg0 | rgba16f | Npre | 0.00 (r, j=0) | 0.00 | 0.00 | 0.00 | pass |
| A2 k64 grey0.5 a0.3 bg0 | rgba16f | Z | 0.06 (r, j=-1) | 0.00 | 0.08 | 0.00 | pass |
| A2 k64 grey0.5 a0.3 bg0 | rgba16f | Zpre | 0.07 (r, j=-1) | 0.00 | 0.11 | 0.00 | pass |
| A3 k4 grey bg0 | b10g11r11 | N | 0.00 (b, j=4) | 0.00 | 0.00 | 0.00 | pass |
| A3 k4 grey bg0 | b10g11r11 | N16 | 0.00 (b, j=3) | 0.00 | 0.01 | 0.01 | pass |
| A3 k4 grey bg0 | b10g11r11 | Z | 1.20 (b, j=1) | 0.60 | 1.77 | 0.83 | pass |
| A3 k4 grey bg0 | rgba16f | N | 0.00 (r, j=-1) | 0.00 | 0.00 | 0.00 | pass |
| A3 k4 grey bg0 | rgba16f | Z | 0.04 (r, j=1) | 0.00 | 0.06 | 0.00 | pass |
| A3-blend k4 grey bg0 | b10g11r11 | N | 0.01 (b, j=2) | 0.02 | 0.02 | 0.02 | pass |
| A3-blend k4 grey bg0 | b10g11r11 | N16 | 0.01 (b, j=2) | 0.01 | 0.01 | 0.01 | pass |
| A3-blend k4 grey bg0 | b10g11r11 | Npre | 0.01 (b, j=2) | 0.01 | 0.02 | 0.01 | pass |
| A3-blend k4 grey bg0 | b10g11r11 | Z | 1.20 (b, j=1) | 0.60 | 1.77 | 0.83 | pass |
| A3-blend k4 grey bg0 | b10g11r11 | Zpre | 1.61 (b, j=1) | 0.82 | 2.38 | 1.14 | pass |
| A3-blend k4 grey bg0 | rgba16f | N | 0.00 (r, j=2) | 0.00 | 0.00 | 0.00 | pass |
| A3-blend k4 grey bg0 | rgba16f | Npre | 0.00 (r, j=2) | 0.00 | 0.00 | 0.00 | pass |
| A3-blend k4 grey bg0 | rgba16f | Z | 0.04 (r, j=1) | 0.00 | 0.06 | 0.00 | pass |
| A3-blend k4 grey bg0 | rgba16f | Zpre | 0.05 (r, j=1) | 0.00 | 0.08 | 0.00 | pass |
| A2-flat k64 grey0.05 a0.1 bg0 | b10g11r11 | N | 2.89 (b, j=2) | 1.69 | 4.33 | 2.40 | reported |
| A2-flat k64 grey0.05 a0.1 bg0 | b10g11r11 | N16 | 2.88 (b, j=2) | 1.86 | 4.31 | 2.63 | reported |
| A2-flat k64 grey0.05 a0.1 bg0 | b10g11r11 | Npre | 2.38 (b, j=2) | 1.30 | 3.59 | 1.85 | reported |
| A2-flat k64 grey0.05 a0.1 bg0 | b10g11r11 | Z | 5.84 (b, j=2) | 2.95 | 8.85 | 4.20 | reported |
| A2-flat k64 grey0.05 a0.1 bg0 | b10g11r11 | Zpre | 6.11 (b, j=2) | 3.15 | 9.26 | 4.49 | reported |
| A2-flat k64 grey0.05 a0.1 bg0 | rgba16f | N | 0.04 (r, j=2) | 0.00 | 0.06 | 0.00 | reported |
| A2-flat k64 grey0.05 a0.1 bg0 | rgba16f | Npre | 0.03 (r, j=2) | 0.00 | 0.05 | 0.00 | reported |
| A2-flat k64 grey0.05 a0.1 bg0 | rgba16f | Z | 0.15 (r, j=2) | 0.00 | 0.24 | 0.00 | reported |
| A2-flat k64 grey0.05 a0.1 bg0 | rgba16f | Zpre | 0.17 (r, j=2) | 0.00 | 0.26 | 0.00 | reported |
| A2-flat k64 grey0.2 a0.1 bg0 | b10g11r11 | N | 2.89 (b, j=0) | 1.69 | 4.33 | 2.40 | reported |
| A2-flat k64 grey0.2 a0.1 bg0 | b10g11r11 | N16 | 2.88 (b, j=0) | 1.86 | 4.31 | 2.64 | reported |
| A2-flat k64 grey0.2 a0.1 bg0 | b10g11r11 | Npre | 2.40 (b, j=0) | 1.32 | 3.61 | 1.87 | reported |
| A2-flat k64 grey0.2 a0.1 bg0 | b10g11r11 | Z | 5.84 (b, j=0) | 2.95 | 8.85 | 4.21 | reported |
| A2-flat k64 grey0.2 a0.1 bg0 | b10g11r11 | Zpre | 6.11 (b, j=0) | 3.15 | 9.26 | 4.49 | reported |
| A2-flat k64 grey0.2 a0.1 bg0 | rgba16f | N | 0.04 (r, j=0) | 0.00 | 0.06 | 0.00 | reported |
| A2-flat k64 grey0.2 a0.1 bg0 | rgba16f | Npre | 0.03 (r, j=0) | 0.00 | 0.05 | 0.00 | reported |
| A2-flat k64 grey0.2 a0.1 bg0 | rgba16f | Z | 0.15 (r, j=0) | 0.00 | 0.24 | 0.00 | reported |
| A2-flat k64 grey0.2 a0.1 bg0 | rgba16f | Zpre | 0.17 (r, j=0) | 0.00 | 0.26 | 0.00 | reported |
| A2-flat k64 grey0.5 a0.1 bg0 | b10g11r11 | N | 2.14 (b, j=-1) | 1.11 | 3.12 | 1.53 | reported |
| A2-flat k64 grey0.5 a0.1 bg0 | b10g11r11 | N16 | 2.14 (b, j=-1) | 1.11 | 3.12 | 1.53 | reported |
| A2-flat k64 grey0.5 a0.1 bg0 | b10g11r11 | Npre | 1.62 (b, j=-1) | 0.84 | 2.37 | 1.15 | reported |
| A2-flat k64 grey0.5 a0.1 bg0 | b10g11r11 | Z | 4.52 (b, j=-1) | 2.32 | 6.63 | 3.22 | reported |
| A2-flat k64 grey0.5 a0.1 bg0 | b10g11r11 | Zpre | 5.10 (b, j=-1) | 2.62 | 7.51 | 3.63 | reported |
| A2-flat k64 grey0.5 a0.1 bg0 | rgba16f | N | 0.02 (r, j=-1) | 0.00 | 0.03 | 0.00 | reported |
| A2-flat k64 grey0.5 a0.1 bg0 | rgba16f | Npre | 0.02 (r, j=-1) | 0.00 | 0.03 | 0.00 | reported |
| A2-flat k64 grey0.5 a0.1 bg0 | rgba16f | Z | 0.12 (r, j=-1) | 0.00 | 0.18 | 0.00 | reported |
| A2-flat k64 grey0.5 a0.1 bg0 | rgba16f | Zpre | 0.14 (r, j=-1) | 0.00 | 0.21 | 0.00 | reported |
| A2-flat k64 grey0.05 a0.3 bg0 | b10g11r11 | N | 0.18 (b, j=2) | 0.00 | 0.29 | 0.00 | reported |
| A2-flat k64 grey0.05 a0.3 bg0 | b10g11r11 | N16 | 0.20 (b, j=2) | 0.02 | 0.31 | 0.03 | reported |
| A2-flat k64 grey0.05 a0.3 bg0 | b10g11r11 | Npre | 0.18 (r, j=2) | 0.00 | 0.28 | 0.00 | reported |
| A2-flat k64 grey0.05 a0.3 bg0 | b10g11r11 | Z | 2.00 (b, j=2) | 1.38 | 2.97 | 1.95 | reported |
| A2-flat k64 grey0.05 a0.3 bg0 | b10g11r11 | Zpre | 2.00 (b, j=2) | 1.06 | 3.02 | 1.51 | reported |
| A2-flat k64 grey0.05 a0.3 bg0 | rgba16f | N | 0.01 (r, j=2) | 0.00 | 0.02 | 0.00 | reported |
| A2-flat k64 grey0.05 a0.3 bg0 | rgba16f | Npre | 0.01 (r, j=2) | 0.00 | 0.02 | 0.00 | reported |
| A2-flat k64 grey0.05 a0.3 bg0 | rgba16f | Z | 0.04 (r, j=2) | 0.00 | 0.06 | 0.00 | reported |
| A2-flat k64 grey0.05 a0.3 bg0 | rgba16f | Zpre | 0.06 (r, j=2) | 0.00 | 0.10 | 0.00 | reported |
| A2-flat k64 grey0.2 a0.3 bg0 | b10g11r11 | N | 0.18 (b, j=0) | 0.00 | 0.29 | 0.00 | reported |
| A2-flat k64 grey0.2 a0.3 bg0 | b10g11r11 | N16 | 0.20 (b, j=0) | 0.02 | 0.31 | 0.03 | reported |
| A2-flat k64 grey0.2 a0.3 bg0 | b10g11r11 | Npre | 0.18 (r, j=0) | 0.00 | 0.28 | 0.00 | reported |
| A2-flat k64 grey0.2 a0.3 bg0 | b10g11r11 | Z | 2.00 (b, j=0) | 1.38 | 2.97 | 1.95 | reported |
| A2-flat k64 grey0.2 a0.3 bg0 | b10g11r11 | Zpre | 2.00 (b, j=0) | 1.06 | 3.02 | 1.50 | reported |
| A2-flat k64 grey0.2 a0.3 bg0 | rgba16f | N | 0.01 (r, j=0) | 0.00 | 0.02 | 0.00 | reported |
| A2-flat k64 grey0.2 a0.3 bg0 | rgba16f | Npre | 0.01 (r, j=0) | 0.00 | 0.02 | 0.00 | reported |
| A2-flat k64 grey0.2 a0.3 bg0 | rgba16f | Z | 0.04 (r, j=0) | 0.00 | 0.06 | 0.00 | reported |
| A2-flat k64 grey0.2 a0.3 bg0 | rgba16f | Zpre | 0.06 (r, j=0) | 0.00 | 0.10 | 0.00 | reported |
| A2-flat k64 grey0.5 a0.3 bg0 | b10g11r11 | N | 0.72 (b, j=-1) | 0.36 | 1.05 | 0.49 | reported |
| A2-flat k64 grey0.5 a0.3 bg0 | b10g11r11 | N16 | 0.72 (b, j=-1) | 0.36 | 1.05 | 0.49 | reported |
| A2-flat k64 grey0.5 a0.3 bg0 | b10g11r11 | Npre | 0.00 (b, j=-1) | 0.00 | 0.00 | 0.00 | reported |
| A2-flat k64 grey0.5 a0.3 bg0 | b10g11r11 | Z | 1.45 (b, j=-1) | 0.73 | 2.11 | 1.00 | reported |
| A2-flat k64 grey0.5 a0.3 bg0 | b10g11r11 | Zpre | 1.45 (b, j=-1) | 0.73 | 2.11 | 1.00 | reported |
| A2-flat k64 grey0.5 a0.3 bg0 | rgba16f | N | 0.02 (r, j=-1) | 0.00 | 0.03 | 0.00 | reported |
| A2-flat k64 grey0.5 a0.3 bg0 | rgba16f | Npre | 0.00 (r, j=-1) | 0.00 | 0.00 | 0.00 | reported |
| A2-flat k64 grey0.5 a0.3 bg0 | rgba16f | Z | 0.04 (r, j=-1) | 0.00 | 0.07 | 0.00 | reported |
| A2-flat k64 grey0.5 a0.3 bg0 | rgba16f | Zpre | 0.04 (r, j=-1) | 0.00 | 0.07 | 0.00 | reported |
| A1-S1 k16 grey bg0.18 | b10g11r11 | N | 0.08 (b, j=-4) | 0.11 | 0.11 | 0.15 | pass |
| A1-S1 k16 grey bg0.18 | b10g11r11 | N16 | 0.08 (b, j=-4) | 0.10 | 0.10 | 0.14 | pass |
| A1-S1 k16 grey bg0.18 | b10g11r11 | Npre | 0.07 (b, j=-4) | 0.10 | 0.10 | 0.13 | pass |
| A1-S1 k16 grey bg0.18 | b10g11r11 | Z | 4.62 (b, j=-4) | 2.37 | 6.95 | 3.36 | FAIL |
| A1-S1 k16 grey bg0.18 | b10g11r11 | Zpre | 4.62 (b, j=-4) | 2.37 | 6.95 | 3.36 | FAIL |
| A1-S1 k16 grey bg0.18 | rgba16f | N | 0.00 (r, j=-4) | 0.00 | 0.00 | 0.00 | pass |
| A1-S1 k16 grey bg0.18 | rgba16f | Npre | 0.00 (r, j=-4) | 0.00 | 0.00 | 0.00 | pass |
| A1-S1 k16 grey bg0.18 | rgba16f | Z | 0.14 (r, j=-4) | 0.00 | 0.22 | 0.00 | pass |
| A1-S1 k16 grey bg0.18 | rgba16f | Zpre | 0.14 (r, j=-4) | 0.00 | 0.22 | 0.00 | pass |
| A1-S1 k16 fire bg0.18 | b10g11r11 | N | 0.03 (g, j=-3) | 0.04 | 0.05 | 0.06 | pass |
| A1-S1 k16 fire bg0.18 | b10g11r11 | N16 | 0.03 (g, j=-3) | 0.03 | 0.05 | 0.05 | pass |
| A1-S1 k16 fire bg0.18 | b10g11r11 | Npre | 0.03 (g, j=-3) | 0.02 | 0.05 | 0.03 | pass |
| A1-S1 k16 fire bg0.18 | b10g11r11 | Z | 4.12 (b, j=-2) | 2.56 | 6.95 | 5.41 | FAIL |
| A1-S1 k16 fire bg0.18 | b10g11r11 | Zpre | 4.12 (b, j=-2) | 2.56 | 6.95 | 5.41 | FAIL |
| A1-S1 k16 fire bg0.18 | rgba16f | N | 0.00 (r, j=-3) | 0.00 | 0.00 | 0.00 | pass |
| A1-S1 k16 fire bg0.18 | rgba16f | Npre | 0.00 (b, j=-2) | 0.00 | 0.00 | 0.00 | pass |
| A1-S1 k16 fire bg0.18 | rgba16f | Z | 0.17 (b, j=-2) | 0.09 | 0.27 | 0.18 | pass |
| A1-S1 k16 fire bg0.18 | rgba16f | Zpre | 0.17 (b, j=-2) | 0.09 | 0.27 | 0.18 | pass |
| A1-S1 k64 grey bg0.18 | b10g11r11 | N | 1.37 (b, j=-4) | 1.76 | 1.89 | 2.46 | pass |
| A1-S1 k64 grey bg0.18 | b10g11r11 | N16 | 1.33 (b, j=-4) | 1.73 | 1.84 | 2.42 | pass |
| A1-S1 k64 grey bg0.18 | b10g11r11 | Npre | 1.28 (b, j=-4) | 1.68 | 1.76 | 2.34 | pass |
| A1-S1 k64 grey bg0.18 | b10g11r11 | Z | 18.20 (b, j=-4) | 9.52 | 27.62 | 13.63 | FAIL |
| A1-S1 k64 grey bg0.18 | b10g11r11 | Zpre | 18.20 (b, j=-4) | 9.52 | 27.62 | 13.63 | FAIL |
| A1-S1 k64 grey bg0.18 | rgba16f | N | 0.00 (r, j=-4) | 0.00 | 0.00 | 0.00 | pass |
| A1-S1 k64 grey bg0.18 | rgba16f | Npre | 0.00 (r, j=-4) | 0.00 | 0.00 | 0.00 | pass |
| A1-S1 k64 grey bg0.18 | rgba16f | Z | 0.55 (r, j=-4) | 0.00 | 0.86 | 0.00 | pass |
| A1-S1 k64 grey bg0.18 | rgba16f | Zpre | 0.55 (r, j=-4) | 0.00 | 0.86 | 0.00 | pass |
| A1-S1 k64 fire bg0.18 | b10g11r11 | N | 0.95 (b, j=-2) | 1.08 | 1.63 | 1.86 | pass |
| A1-S1 k64 fire bg0.18 | b10g11r11 | N16 | 1.11 (b, j=-2) | 1.24 | 1.92 | 2.16 | pass |
| A1-S1 k64 fire bg0.18 | b10g11r11 | Npre | 1.07 (b, j=-2) | 1.19 | 1.84 | 2.08 | pass |
| A1-S1 k64 fire bg0.18 | b10g11r11 | Z | 18.45 (b, j=-2) | 12.48 | 31.78 | 25.67 | FAIL |
| A1-S1 k64 fire bg0.18 | b10g11r11 | Zpre | 18.45 (b, j=-2) | 12.48 | 31.78 | 25.67 | FAIL |
| A1-S1 k64 fire bg0.18 | rgba16f | N | 0.00 (b, j=-3) | 0.00 | 0.00 | 0.00 | pass |
| A1-S1 k64 fire bg0.18 | rgba16f | Npre | 0.00 (b, j=-3) | 0.00 | 0.00 | 0.00 | pass |
| A1-S1 k64 fire bg0.18 | rgba16f | Z | 0.66 (b, j=-2) | 0.34 | 1.06 | 0.70 | pass |
| A1-S1 k64 fire bg0.18 | rgba16f | Zpre | 0.66 (b, j=-2) | 0.34 | 1.06 | 0.70 | pass |
| A1-S2 k16 grey bg0.18 | b10g11r11 | N | 0.08 (b, j=-4) | 0.06 | 0.12 | 0.08 | pass |
| A1-S2 k16 grey bg0.18 | b10g11r11 | N16 | 0.08 (b, j=-4) | 0.06 | 0.12 | 0.08 | pass |
| A1-S2 k16 grey bg0.18 | b10g11r11 | Npre | 0.10 (b, j=-4) | 0.08 | 0.15 | 0.11 | pass |
| A1-S2 k16 grey bg0.18 | b10g11r11 | Z | 4.43 (b, j=-4) | 2.20 | 6.65 | 3.10 | FAIL |
| A1-S2 k16 grey bg0.18 | b10g11r11 | Zpre | 4.43 (b, j=-4) | 2.20 | 6.65 | 3.11 | FAIL |
| A1-S2 k16 grey bg0.18 | rgba16f | N | 0.00 (r, j=2) | 0.00 | 0.00 | 0.00 | pass |
| A1-S2 k16 grey bg0.18 | rgba16f | Npre | 0.00 (r, j=-2) | 0.00 | 0.00 | 0.00 | pass |
| A1-S2 k16 grey bg0.18 | rgba16f | Z | 0.14 (r, j=-4) | 0.00 | 0.22 | 0.00 | pass |
| A1-S2 k16 grey bg0.18 | rgba16f | Zpre | 0.14 (r, j=-4) | 0.00 | 0.22 | 0.00 | pass |
| A1-S2 k16 fire bg0.18 | b10g11r11 | N | 0.06 (b, j=-2) | 0.05 | 0.10 | 0.09 | pass |
| A1-S2 k16 fire bg0.18 | b10g11r11 | N16 | 0.06 (b, j=-2) | 0.05 | 0.11 | 0.10 | pass |
| A1-S2 k16 fire bg0.18 | b10g11r11 | Npre | 0.07 (b, j=-2) | 0.06 | 0.12 | 0.11 | pass |
| A1-S2 k16 fire bg0.18 | b10g11r11 | Z | 4.12 (b, j=-2) | 2.57 | 6.93 | 5.40 | FAIL |
| A1-S2 k16 fire bg0.18 | b10g11r11 | Zpre | 4.12 (b, j=-2) | 2.57 | 6.93 | 5.40 | FAIL |
| A1-S2 k16 fire bg0.18 | rgba16f | N | 0.00 (g, j=-3) | 0.00 | 0.00 | 0.00 | pass |
| A1-S2 k16 fire bg0.18 | rgba16f | Npre | 0.00 (g, j=-3) | 0.00 | 0.00 | 0.00 | pass |
| A1-S2 k16 fire bg0.18 | rgba16f | Z | 0.17 (b, j=-2) | 0.09 | 0.27 | 0.18 | pass |
| A1-S2 k16 fire bg0.18 | rgba16f | Zpre | 0.17 (b, j=-2) | 0.09 | 0.27 | 0.18 | pass |
| A1-S2 k64 grey bg0.18 | b10g11r11 | N | 1.29 (b, j=-4) | 1.17 | 1.86 | 1.65 | pass |
| A1-S2 k64 grey bg0.18 | b10g11r11 | N16 | 1.31 (b, j=-4) | 1.18 | 1.90 | 1.66 | pass |
| A1-S2 k64 grey bg0.18 | b10g11r11 | Npre | 1.33 (b, j=-4) | 1.22 | 1.93 | 1.72 | pass |
| A1-S2 k64 grey bg0.18 | b10g11r11 | Z | 15.75 (b, j=-4) | 7.22 | 24.00 | 10.32 | FAIL |
| A1-S2 k64 grey bg0.18 | b10g11r11 | Zpre | 15.75 (b, j=-4) | 7.22 | 24.00 | 10.32 | FAIL |
| A1-S2 k64 grey bg0.18 | rgba16f | N | 0.00 (r, j=-4) | 0.00 | 0.00 | 0.00 | pass |
| A1-S2 k64 grey bg0.18 | rgba16f | Npre | 0.00 (r, j=-4) | 0.00 | 0.00 | 0.00 | pass |
| A1-S2 k64 grey bg0.18 | rgba16f | Z | 0.54 (r, j=-4) | 0.00 | 0.85 | 0.00 | pass |
| A1-S2 k64 grey bg0.18 | rgba16f | Zpre | 0.54 (r, j=-4) | 0.00 | 0.85 | 0.00 | pass |
| A1-S2 k64 fire bg0.18 | b10g11r11 | N | 1.45 (b, j=-2) | 1.37 | 2.60 | 2.55 | pass |
| A1-S2 k64 fire bg0.18 | b10g11r11 | N16 | 1.50 (b, j=-2) | 1.40 | 2.68 | 2.61 | pass |
| A1-S2 k64 fire bg0.18 | b10g11r11 | Npre | 1.51 (b, j=-2) | 1.42 | 2.70 | 2.64 | pass |
| A1-S2 k64 fire bg0.18 | b10g11r11 | Z | 14.59 (b, j=-2) | 9.05 | 24.56 | 18.72 | FAIL |
| A1-S2 k64 fire bg0.18 | b10g11r11 | Zpre | 14.59 (b, j=-2) | 9.05 | 24.56 | 18.72 | FAIL |
| A1-S2 k64 fire bg0.18 | rgba16f | N | 0.00 (b, j=-2) | 0.00 | 0.00 | 0.00 | pass |
| A1-S2 k64 fire bg0.18 | rgba16f | Npre | 0.00 (b, j=-2) | 0.00 | 0.00 | 0.00 | pass |
| A1-S2 k64 fire bg0.18 | rgba16f | Z | 0.65 (b, j=-2) | 0.33 | 1.05 | 0.69 | pass |
| A1-S2 k64 fire bg0.18 | rgba16f | Zpre | 0.65 (b, j=-2) | 0.33 | 1.05 | 0.69 | pass |
| A2 k16 grey0.05 a0.1 bg0.18 | b10g11r11 | N | 0.00 (r, j=2) | 0.01 | 0.01 | 0.01 | pass |
| A2 k16 grey0.05 a0.1 bg0.18 | b10g11r11 | N16 | 0.01 (r, j=2) | 0.00 | 0.01 | 0.01 | pass |
| A2 k16 grey0.05 a0.1 bg0.18 | b10g11r11 | Npre | 0.01 (r, j=2) | 0.01 | 0.01 | 0.02 | pass |
| A2 k16 grey0.05 a0.1 bg0.18 | b10g11r11 | Z | 5.71 (b, j=2) | 2.87 | 8.48 | 3.97 | FAIL |
| A2 k16 grey0.05 a0.1 bg0.18 | b10g11r11 | Zpre | 6.00 (b, j=2) | 3.02 | 8.91 | 4.17 | FAIL |
| A2 k16 grey0.05 a0.1 bg0.18 | rgba16f | N | 0.00 (r, j=-2) | 0.00 | 0.00 | 0.00 | pass |
| A2 k16 grey0.05 a0.1 bg0.18 | rgba16f | Npre | 0.00 (r, j=1) | 0.00 | 0.00 | 0.00 | pass |
| A2 k16 grey0.05 a0.1 bg0.18 | rgba16f | Z | 0.18 (r, j=2) | 0.00 | 0.28 | 0.00 | pass |
| A2 k16 grey0.05 a0.1 bg0.18 | rgba16f | Zpre | 0.19 (r, j=2) | 0.00 | 0.29 | 0.00 | pass |
| A2 k16 grey0.2 a0.1 bg0.18 | b10g11r11 | N | 0.01 (r, j=0) | 0.01 | 0.01 | 0.01 | pass |
| A2 k16 grey0.2 a0.1 bg0.18 | b10g11r11 | N16 | 0.01 (b, j=0) | 0.01 | 0.01 | 0.01 | pass |
| A2 k16 grey0.2 a0.1 bg0.18 | b10g11r11 | Npre | 0.01 (r, j=0) | 0.01 | 0.01 | 0.01 | pass |
| A2 k16 grey0.2 a0.1 bg0.18 | b10g11r11 | Z | 3.85 (b, j=0) | 1.97 | 5.83 | 2.80 | FAIL |
| A2 k16 grey0.2 a0.1 bg0.18 | b10g11r11 | Zpre | 4.31 (b, j=0) | 2.20 | 6.52 | 3.13 | FAIL |
| A2 k16 grey0.2 a0.1 bg0.18 | rgba16f | N | 0.00 (r, j=1) | 0.00 | 0.00 | 0.00 | pass |
| A2 k16 grey0.2 a0.1 bg0.18 | rgba16f | Npre | 0.00 (r, j=0) | 0.00 | 0.00 | 0.00 | pass |
| A2 k16 grey0.2 a0.1 bg0.18 | rgba16f | Z | 0.12 (r, j=0) | 0.00 | 0.18 | 0.00 | pass |
| A2 k16 grey0.2 a0.1 bg0.18 | rgba16f | Zpre | 0.13 (r, j=0) | 0.00 | 0.21 | 0.00 | pass |
| A2 k16 grey0.5 a0.1 bg0.18 | b10g11r11 | N | 0.01 (b, j=-1) | 0.01 | 0.01 | 0.01 | pass |
| A2 k16 grey0.5 a0.1 bg0.18 | b10g11r11 | N16 | 0.01 (b, j=-1) | 0.00 | 0.01 | 0.01 | pass |
| A2 k16 grey0.5 a0.1 bg0.18 | b10g11r11 | Npre | 0.01 (r, j=-1) | 0.00 | 0.01 | 0.00 | pass |
| A2 k16 grey0.5 a0.1 bg0.18 | b10g11r11 | Z | 3.35 (b, j=-1) | 1.71 | 5.01 | 2.41 | FAIL |
| A2 k16 grey0.5 a0.1 bg0.18 | b10g11r11 | Zpre | 3.83 (b, j=-1) | 1.95 | 5.74 | 2.75 | FAIL |
| A2 k16 grey0.5 a0.1 bg0.18 | rgba16f | N | 0.00 (r, j=-1) | 0.00 | 0.00 | 0.00 | pass |
| A2 k16 grey0.5 a0.1 bg0.18 | rgba16f | Npre | 0.00 (r, j=-1) | 0.00 | 0.00 | 0.00 | pass |
| A2 k16 grey0.5 a0.1 bg0.18 | rgba16f | Z | 0.10 (r, j=-1) | 0.00 | 0.16 | 0.00 | pass |
| A2 k16 grey0.5 a0.1 bg0.18 | rgba16f | Zpre | 0.12 (r, j=-1) | 0.00 | 0.18 | 0.00 | pass |
| A2 k16 grey0.05 a0.3 bg0.18 | b10g11r11 | N | 0.00 (r, j=2) | 0.00 | 0.00 | 0.01 | pass |
| A2 k16 grey0.05 a0.3 bg0.18 | b10g11r11 | N16 | 0.01 (b, j=2) | 0.00 | 0.01 | 0.01 | pass |
| A2 k16 grey0.05 a0.3 bg0.18 | b10g11r11 | Npre | 0.01 (b, j=2) | 0.01 | 0.01 | 0.01 | pass |
| A2 k16 grey0.05 a0.3 bg0.18 | b10g11r11 | Z | 1.57 (b, j=2) | 0.78 | 2.37 | 1.11 | pass |
| A2 k16 grey0.05 a0.3 bg0.18 | b10g11r11 | Zpre | 2.08 (b, j=2) | 1.04 | 3.14 | 1.47 | FAIL |
| A2 k16 grey0.05 a0.3 bg0.18 | rgba16f | N | 0.00 (r, j=1) | 0.00 | 0.00 | 0.00 | pass |
| A2 k16 grey0.05 a0.3 bg0.18 | rgba16f | Npre | 0.00 (r, j=2) | 0.00 | 0.00 | 0.00 | pass |
| A2 k16 grey0.05 a0.3 bg0.18 | rgba16f | Z | 0.05 (r, j=2) | 0.00 | 0.08 | 0.00 | pass |
| A2 k16 grey0.05 a0.3 bg0.18 | rgba16f | Zpre | 0.07 (r, j=2) | 0.00 | 0.10 | 0.00 | pass |
| A2 k16 grey0.2 a0.3 bg0.18 | b10g11r11 | N | 0.00 (r, j=0) | 0.00 | 0.00 | 0.00 | pass |
| A2 k16 grey0.2 a0.3 bg0.18 | b10g11r11 | N16 | 0.00 (b, j=0) | 0.00 | 0.00 | 0.01 | pass |
| A2 k16 grey0.2 a0.3 bg0.18 | b10g11r11 | Npre | 0.00 (b, j=0) | 0.00 | 0.01 | 0.00 | pass |
| A2 k16 grey0.2 a0.3 bg0.18 | b10g11r11 | Z | 1.54 (b, j=0) | 0.78 | 2.31 | 1.09 | pass |
| A2 k16 grey0.2 a0.3 bg0.18 | b10g11r11 | Zpre | 2.05 (b, j=0) | 1.03 | 3.08 | 1.45 | FAIL |
| A2 k16 grey0.2 a0.3 bg0.18 | rgba16f | N | 0.00 (r, j=0) | 0.00 | 0.00 | 0.00 | pass |
| A2 k16 grey0.2 a0.3 bg0.18 | rgba16f | Npre | 0.00 (r, j=0) | 0.00 | 0.00 | 0.00 | pass |
| A2 k16 grey0.2 a0.3 bg0.18 | rgba16f | Z | 0.05 (r, j=0) | 0.00 | 0.08 | 0.00 | pass |
| A2 k16 grey0.2 a0.3 bg0.18 | rgba16f | Zpre | 0.06 (r, j=0) | 0.00 | 0.10 | 0.00 | pass |
| A2 k16 grey0.5 a0.3 bg0.18 | b10g11r11 | N | 0.02 (b, j=-1) | 0.03 | 0.03 | 0.03 | pass |
| A2 k16 grey0.5 a0.3 bg0.18 | b10g11r11 | N16 | 0.02 (b, j=-1) | 0.02 | 0.03 | 0.03 | pass |
| A2 k16 grey0.5 a0.3 bg0.18 | b10g11r11 | Npre | 0.02 (b, j=-1) | 0.02 | 0.04 | 0.03 | pass |
| A2 k16 grey0.5 a0.3 bg0.18 | b10g11r11 | Z | 1.66 (b, j=-1) | 0.81 | 2.42 | 1.11 | pass |
| A2 k16 grey0.5 a0.3 bg0.18 | b10g11r11 | Zpre | 2.16 (b, j=-1) | 1.06 | 3.15 | 1.45 | FAIL |
| A2 k16 grey0.5 a0.3 bg0.18 | rgba16f | N | 0.00 (r, j=-2) | 0.00 | 0.00 | 0.00 | pass |
| A2 k16 grey0.5 a0.3 bg0.18 | rgba16f | Npre | 0.00 (r, j=0) | 0.00 | 0.00 | 0.00 | pass |
| A2 k16 grey0.5 a0.3 bg0.18 | rgba16f | Z | 0.05 (r, j=-1) | 0.00 | 0.08 | 0.00 | pass |
| A2 k16 grey0.5 a0.3 bg0.18 | rgba16f | Zpre | 0.07 (r, j=-1) | 0.00 | 0.11 | 0.00 | pass |
| A2 k64 grey0.05 a0.1 bg0.18 | b10g11r11 | N | 0.01 (b, j=3) | 0.00 | 0.01 | 0.01 | pass |
| A2 k64 grey0.05 a0.1 bg0.18 | b10g11r11 | N16 | 0.01 (b, j=2) | 0.00 | 0.01 | 0.01 | pass |
| A2 k64 grey0.05 a0.1 bg0.18 | b10g11r11 | Npre | 0.01 (b, j=2) | 0.01 | 0.02 | 0.01 | pass |
| A2 k64 grey0.05 a0.1 bg0.18 | b10g11r11 | Z | 4.70 (b, j=2) | 2.41 | 7.11 | 3.43 | FAIL |
| A2 k64 grey0.05 a0.1 bg0.18 | b10g11r11 | Zpre | 5.26 (b, j=2) | 2.70 | 7.95 | 3.83 | FAIL |
| A2 k64 grey0.05 a0.1 bg0.18 | rgba16f | N | 0.00 (r, j=2) | 0.00 | 0.00 | 0.00 | pass |
| A2 k64 grey0.05 a0.1 bg0.18 | rgba16f | Npre | 0.00 (r, j=2) | 0.00 | 0.00 | 0.00 | pass |
| A2 k64 grey0.05 a0.1 bg0.18 | rgba16f | Z | 0.14 (r, j=2) | 0.00 | 0.22 | 0.00 | pass |
| A2 k64 grey0.05 a0.1 bg0.18 | rgba16f | Zpre | 0.16 (r, j=2) | 0.00 | 0.25 | 0.00 | pass |
| A2 k64 grey0.2 a0.1 bg0.18 | b10g11r11 | N | 0.00 (r, j=1) | 0.00 | 0.01 | 0.01 | pass |
| A2 k64 grey0.2 a0.1 bg0.18 | b10g11r11 | N16 | 0.00 (r, j=1) | 0.00 | 0.01 | 0.01 | pass |
| A2 k64 grey0.2 a0.1 bg0.18 | b10g11r11 | Npre | 0.01 (b, j=0) | 0.01 | 0.01 | 0.01 | pass |
| A2 k64 grey0.2 a0.1 bg0.18 | b10g11r11 | Z | 4.71 (b, j=0) | 2.42 | 7.12 | 3.44 | FAIL |
| A2 k64 grey0.2 a0.1 bg0.18 | b10g11r11 | Zpre | 5.29 (b, j=0) | 2.73 | 8.00 | 3.88 | FAIL |
| A2 k64 grey0.2 a0.1 bg0.18 | rgba16f | N | 0.00 (r, j=0) | 0.00 | 0.00 | 0.00 | pass |
| A2 k64 grey0.2 a0.1 bg0.18 | rgba16f | Npre | 0.00 (r, j=0) | 0.00 | 0.00 | 0.00 | pass |
| A2 k64 grey0.2 a0.1 bg0.18 | rgba16f | Z | 0.14 (r, j=0) | 0.00 | 0.22 | 0.00 | pass |
| A2 k64 grey0.2 a0.1 bg0.18 | rgba16f | Zpre | 0.16 (r, j=0) | 0.00 | 0.25 | 0.00 | pass |
| A2 k64 grey0.5 a0.1 bg0.18 | b10g11r11 | N | 0.06 (b, j=-1) | 0.05 | 0.09 | 0.06 | pass |
| A2 k64 grey0.5 a0.1 bg0.18 | b10g11r11 | N16 | 0.07 (b, j=-1) | 0.05 | 0.10 | 0.07 | pass |
| A2 k64 grey0.5 a0.1 bg0.18 | b10g11r11 | Npre | 0.06 (b, j=-1) | 0.04 | 0.09 | 0.06 | pass |
| A2 k64 grey0.5 a0.1 bg0.18 | b10g11r11 | Z | 4.08 (b, j=-1) | 1.84 | 6.01 | 2.55 | FAIL |
| A2 k64 grey0.5 a0.1 bg0.18 | b10g11r11 | Zpre | 4.56 (b, j=-1) | 2.09 | 6.72 | 2.90 | FAIL |
| A2 k64 grey0.5 a0.1 bg0.18 | rgba16f | N | 0.00 (r, j=-1) | 0.00 | 0.00 | 0.00 | pass |
| A2 k64 grey0.5 a0.1 bg0.18 | rgba16f | Npre | 0.00 (r, j=-1) | 0.00 | 0.00 | 0.00 | pass |
| A2 k64 grey0.5 a0.1 bg0.18 | rgba16f | Z | 0.16 (r, j=-1) | 0.00 | 0.25 | 0.00 | pass |
| A2 k64 grey0.5 a0.1 bg0.18 | rgba16f | Zpre | 0.18 (r, j=-1) | 0.00 | 0.27 | 0.00 | pass |
| A2 k64 grey0.05 a0.3 bg0.18 | b10g11r11 | N | 0.00 (r, j=2) | 0.00 | 0.01 | 0.01 | pass |
| A2 k64 grey0.05 a0.3 bg0.18 | b10g11r11 | N16 | 0.00 (b, j=2) | 0.00 | 0.00 | 0.01 | pass |
| A2 k64 grey0.05 a0.3 bg0.18 | b10g11r11 | Npre | 0.00 (b, j=2) | 0.00 | 0.00 | 0.01 | pass |
| A2 k64 grey0.05 a0.3 bg0.18 | b10g11r11 | Z | 1.54 (b, j=2) | 0.77 | 2.33 | 1.09 | pass |
| A2 k64 grey0.05 a0.3 bg0.18 | b10g11r11 | Zpre | 2.07 (b, j=2) | 1.04 | 3.11 | 1.47 | FAIL |
| A2 k64 grey0.05 a0.3 bg0.18 | rgba16f | N | 0.00 (r, j=2) | 0.00 | 0.00 | 0.00 | pass |
| A2 k64 grey0.05 a0.3 bg0.18 | rgba16f | Npre | 0.00 (r, j=2) | 0.00 | 0.00 | 0.00 | pass |
| A2 k64 grey0.05 a0.3 bg0.18 | rgba16f | Z | 0.05 (r, j=2) | 0.00 | 0.08 | 0.00 | pass |
| A2 k64 grey0.05 a0.3 bg0.18 | rgba16f | Zpre | 0.06 (r, j=2) | 0.00 | 0.10 | 0.00 | pass |
| A2 k64 grey0.2 a0.3 bg0.18 | b10g11r11 | N | 0.01 (b, j=0) | 0.01 | 0.02 | 0.02 | pass |
| A2 k64 grey0.2 a0.3 bg0.18 | b10g11r11 | N16 | 0.01 (b, j=0) | 0.01 | 0.01 | 0.02 | pass |
| A2 k64 grey0.2 a0.3 bg0.18 | b10g11r11 | Npre | 0.00 (b, j=-1) | 0.00 | 0.00 | 0.00 | pass |
| A2 k64 grey0.2 a0.3 bg0.18 | b10g11r11 | Z | 1.56 (b, j=0) | 0.79 | 2.35 | 1.11 | pass |
| A2 k64 grey0.2 a0.3 bg0.18 | b10g11r11 | Zpre | 2.07 (b, j=0) | 1.05 | 3.12 | 1.48 | FAIL |
| A2 k64 grey0.2 a0.3 bg0.18 | rgba16f | N | 0.00 (r, j=4) | 0.00 | 0.00 | 0.00 | pass |
| A2 k64 grey0.2 a0.3 bg0.18 | rgba16f | Npre | 0.00 (r, j=0) | 0.00 | 0.00 | 0.00 | pass |
| A2 k64 grey0.2 a0.3 bg0.18 | rgba16f | Z | 0.05 (r, j=0) | 0.00 | 0.08 | 0.00 | pass |
| A2 k64 grey0.2 a0.3 bg0.18 | rgba16f | Zpre | 0.06 (r, j=0) | 0.00 | 0.10 | 0.00 | pass |
| A2 k64 grey0.5 a0.3 bg0.18 | b10g11r11 | N | 0.02 (b, j=-1) | 0.01 | 0.03 | 0.01 | pass |
| A2 k64 grey0.5 a0.3 bg0.18 | b10g11r11 | N16 | 0.02 (b, j=-1) | 0.01 | 0.03 | 0.02 | pass |
| A2 k64 grey0.5 a0.3 bg0.18 | b10g11r11 | Npre | 0.03 (b, j=-1) | 0.02 | 0.04 | 0.03 | pass |
| A2 k64 grey0.5 a0.3 bg0.18 | b10g11r11 | Z | 1.68 (b, j=-1) | 0.81 | 2.45 | 1.11 | pass |
| A2 k64 grey0.5 a0.3 bg0.18 | b10g11r11 | Zpre | 2.19 (b, j=-1) | 1.07 | 3.19 | 1.47 | FAIL |
| A2 k64 grey0.5 a0.3 bg0.18 | rgba16f | N | 0.00 (r, j=-2) | 0.00 | 0.00 | 0.00 | pass |
| A2 k64 grey0.5 a0.3 bg0.18 | rgba16f | Npre | 0.00 (r, j=-1) | 0.00 | 0.00 | 0.00 | pass |
| A2 k64 grey0.5 a0.3 bg0.18 | rgba16f | Z | 0.06 (r, j=-1) | 0.00 | 0.08 | 0.00 | pass |
| A2 k64 grey0.5 a0.3 bg0.18 | rgba16f | Zpre | 0.07 (r, j=-1) | 0.00 | 0.11 | 0.00 | pass |
| A3 k4 grey bg0.18 | b10g11r11 | N | 0.01 (r, j=0) | 0.00 | 0.01 | 0.01 | pass |
| A3 k4 grey bg0.18 | b10g11r11 | N16 | 0.01 (r, j=0) | 0.00 | 0.01 | 0.00 | pass |
| A3 k4 grey bg0.18 | b10g11r11 | Z | 1.58 (b, j=0) | 0.80 | 2.37 | 1.12 | pass |
| A3 k4 grey bg0.18 | rgba16f | N | 0.00 (r, j=-3) | 0.00 | 0.00 | 0.00 | pass |
| A3 k4 grey bg0.18 | rgba16f | Z | 0.05 (r, j=0) | 0.00 | 0.08 | 0.00 | pass |
| A3-blend k4 grey bg0.18 | b10g11r11 | N | 0.01 (b, j=0) | 0.01 | 0.01 | 0.02 | pass |
| A3-blend k4 grey bg0.18 | b10g11r11 | N16 | 0.01 (b, j=0) | 0.01 | 0.01 | 0.02 | pass |
| A3-blend k4 grey bg0.18 | b10g11r11 | Npre | 0.01 (r, j=0) | 0.01 | 0.01 | 0.02 | pass |
| A3-blend k4 grey bg0.18 | b10g11r11 | Z | 1.59 (b, j=0) | 0.80 | 2.38 | 1.12 | pass |
| A3-blend k4 grey bg0.18 | b10g11r11 | Zpre | 1.80 (b, j=0) | 0.91 | 2.70 | 1.28 | pass |
| A3-blend k4 grey bg0.18 | rgba16f | N | 0.00 (r, j=0) | 0.00 | 0.00 | 0.00 | pass |
| A3-blend k4 grey bg0.18 | rgba16f | Npre | 0.00 (r, j=0) | 0.00 | 0.00 | 0.00 | pass |
| A3-blend k4 grey bg0.18 | rgba16f | Z | 0.05 (r, j=0) | 0.00 | 0.08 | 0.00 | pass |
| A3-blend k4 grey bg0.18 | rgba16f | Zpre | 0.06 (r, j=0) | 0.00 | 0.09 | 0.00 | pass |
| A2-flat k64 grey0.05 a0.1 bg0.18 | b10g11r11 | N | 2.30 (b, j=2) | 1.27 | 3.44 | 1.79 | reported |
| A2-flat k64 grey0.05 a0.1 bg0.18 | b10g11r11 | N16 | 2.30 (b, j=2) | 1.06 | 3.47 | 1.48 | reported |
| A2-flat k64 grey0.05 a0.1 bg0.18 | b10g11r11 | Npre | 2.30 (b, j=2) | 1.28 | 3.44 | 1.79 | reported |
| A2-flat k64 grey0.05 a0.1 bg0.18 | b10g11r11 | Z | 0.32 (r, j=2) | 0.00 | 0.51 | 0.00 | reported |
| A2-flat k64 grey0.05 a0.1 bg0.18 | b10g11r11 | Zpre | 1.85 (b, j=2) | 1.01 | 2.78 | 1.43 | reported |
| A2-flat k64 grey0.05 a0.1 bg0.18 | rgba16f | N | 0.00 (r, j=2) | 0.00 | 0.01 | 0.00 | reported |
| A2-flat k64 grey0.05 a0.1 bg0.18 | rgba16f | Npre | 0.00 (r, j=2) | 0.00 | 0.01 | 0.00 | reported |
| A2-flat k64 grey0.05 a0.1 bg0.18 | rgba16f | Z | 0.14 (r, j=2) | 0.00 | 0.22 | 0.00 | reported |
| A2-flat k64 grey0.05 a0.1 bg0.18 | rgba16f | Zpre | 0.16 (r, j=2) | 0.00 | 0.26 | 0.00 | reported |
| A2-flat k64 grey0.2 a0.1 bg0.18 | b10g11r11 | N | 2.93 (b, j=0) | 1.77 | 4.40 | 2.52 | reported |
| A2-flat k64 grey0.2 a0.1 bg0.18 | b10g11r11 | N16 | 2.93 (b, j=0) | 1.87 | 4.38 | 2.65 | reported |
| A2-flat k64 grey0.2 a0.1 bg0.18 | b10g11r11 | Npre | 2.26 (b, j=0) | 1.18 | 3.42 | 1.67 | reported |
| A2-flat k64 grey0.2 a0.1 bg0.18 | b10g11r11 | Z | 4.88 (b, j=0) | 1.94 | 7.47 | 2.76 | reported |
| A2-flat k64 grey0.2 a0.1 bg0.18 | b10g11r11 | Zpre | 4.88 (b, j=0) | 1.93 | 7.47 | 2.75 | reported |
| A2-flat k64 grey0.2 a0.1 bg0.18 | rgba16f | N | 0.07 (r, j=0) | 0.00 | 0.12 | 0.00 | reported |
| A2-flat k64 grey0.2 a0.1 bg0.18 | rgba16f | Npre | 0.07 (r, j=0) | 0.00 | 0.11 | 0.00 | reported |
| A2-flat k64 grey0.2 a0.1 bg0.18 | rgba16f | Z | 0.18 (r, j=0) | 0.00 | 0.28 | 0.00 | reported |
| A2-flat k64 grey0.2 a0.1 bg0.18 | rgba16f | Zpre | 0.18 (r, j=0) | 0.00 | 0.29 | 0.00 | reported |
| A2-flat k64 grey0.5 a0.1 bg0.18 | b10g11r11 | N | 2.16 (b, j=-1) | 1.11 | 3.15 | 1.53 | reported |
| A2-flat k64 grey0.5 a0.1 bg0.18 | b10g11r11 | N16 | 2.16 (b, j=-1) | 1.11 | 3.15 | 1.53 | reported |
| A2-flat k64 grey0.5 a0.1 bg0.18 | b10g11r11 | Npre | 1.62 (b, j=-1) | 0.83 | 2.36 | 1.13 | reported |
| A2-flat k64 grey0.5 a0.1 bg0.18 | b10g11r11 | Z | 4.51 (b, j=-1) | 2.32 | 6.62 | 3.21 | reported |
| A2-flat k64 grey0.5 a0.1 bg0.18 | b10g11r11 | Zpre | 5.08 (b, j=-1) | 2.59 | 7.47 | 3.59 | reported |
| A2-flat k64 grey0.5 a0.1 bg0.18 | rgba16f | N | 0.03 (r, j=-1) | 0.00 | 0.05 | 0.00 | reported |
| A2-flat k64 grey0.5 a0.1 bg0.18 | rgba16f | Npre | 0.03 (r, j=-1) | 0.00 | 0.05 | 0.00 | reported |
| A2-flat k64 grey0.5 a0.1 bg0.18 | rgba16f | Z | 0.12 (r, j=-1) | 0.00 | 0.19 | 0.00 | reported |
| A2-flat k64 grey0.5 a0.1 bg0.18 | rgba16f | Zpre | 0.14 (r, j=-1) | 0.00 | 0.21 | 0.00 | reported |
| A2-flat k64 grey0.05 a0.3 bg0.18 | b10g11r11 | N | 0.71 (b, j=2) | 0.44 | 1.06 | 0.63 | reported |
| A2-flat k64 grey0.05 a0.3 bg0.18 | b10g11r11 | N16 | 0.71 (b, j=2) | 0.44 | 1.06 | 0.63 | reported |
| A2-flat k64 grey0.05 a0.3 bg0.18 | b10g11r11 | Npre | 0.23 (r, j=2) | 0.33 | 0.35 | 0.47 | reported |
| A2-flat k64 grey0.05 a0.3 bg0.18 | b10g11r11 | Z | 0.18 (r, j=2) | 0.00 | 0.28 | 0.00 | reported |
| A2-flat k64 grey0.05 a0.3 bg0.18 | b10g11r11 | Zpre | 1.97 (b, j=2) | 1.17 | 2.95 | 1.66 | reported |
| A2-flat k64 grey0.05 a0.3 bg0.18 | rgba16f | N | 0.02 (r, j=2) | 0.00 | 0.03 | 0.00 | reported |
| A2-flat k64 grey0.05 a0.3 bg0.18 | rgba16f | Npre | 0.01 (r, j=2) | 0.00 | 0.02 | 0.00 | reported |
| A2-flat k64 grey0.05 a0.3 bg0.18 | rgba16f | Z | 0.01 (r, j=2) | 0.00 | 0.02 | 0.00 | reported |
| A2-flat k64 grey0.05 a0.3 bg0.18 | rgba16f | Zpre | 0.05 (r, j=2) | 0.00 | 0.08 | 0.00 | reported |
| A2-flat k64 grey0.2 a0.3 bg0.18 | b10g11r11 | N | 0.18 (b, j=0) | 0.00 | 0.28 | 0.00 | reported |
| A2-flat k64 grey0.2 a0.3 bg0.18 | b10g11r11 | N16 | 0.19 (b, j=0) | 0.01 | 0.30 | 0.02 | reported |
| A2-flat k64 grey0.2 a0.3 bg0.18 | b10g11r11 | Npre | 0.18 (r, j=0) | 0.00 | 0.28 | 0.00 | reported |
| A2-flat k64 grey0.2 a0.3 bg0.18 | b10g11r11 | Z | 2.00 (b, j=0) | 1.38 | 2.97 | 1.95 | reported |
| A2-flat k64 grey0.2 a0.3 bg0.18 | b10g11r11 | Zpre | 2.00 (b, j=0) | 1.07 | 3.02 | 1.52 | reported |
| A2-flat k64 grey0.2 a0.3 bg0.18 | rgba16f | N | 0.01 (r, j=0) | 0.00 | 0.02 | 0.00 | reported |
| A2-flat k64 grey0.2 a0.3 bg0.18 | rgba16f | Npre | 0.01 (r, j=0) | 0.00 | 0.02 | 0.00 | reported |
| A2-flat k64 grey0.2 a0.3 bg0.18 | rgba16f | Z | 0.04 (r, j=0) | 0.00 | 0.06 | 0.00 | reported |
| A2-flat k64 grey0.2 a0.3 bg0.18 | rgba16f | Zpre | 0.06 (r, j=0) | 0.00 | 0.10 | 0.00 | reported |
| A2-flat k64 grey0.5 a0.3 bg0.18 | b10g11r11 | N | 0.72 (b, j=-1) | 0.36 | 1.05 | 0.49 | reported |
| A2-flat k64 grey0.5 a0.3 bg0.18 | b10g11r11 | N16 | 0.72 (b, j=-1) | 0.36 | 1.05 | 0.49 | reported |
| A2-flat k64 grey0.5 a0.3 bg0.18 | b10g11r11 | Npre | 0.00 (r, j=-1) | 0.00 | 0.00 | 0.00 | reported |
| A2-flat k64 grey0.5 a0.3 bg0.18 | b10g11r11 | Z | 1.45 (b, j=-1) | 0.73 | 2.11 | 1.00 | reported |
| A2-flat k64 grey0.5 a0.3 bg0.18 | b10g11r11 | Zpre | 1.45 (b, j=-1) | 0.73 | 2.11 | 1.00 | reported |
| A2-flat k64 grey0.5 a0.3 bg0.18 | rgba16f | N | 0.02 (r, j=-1) | 0.00 | 0.03 | 0.00 | reported |
| A2-flat k64 grey0.5 a0.3 bg0.18 | rgba16f | Npre | 0.00 (r, j=-1) | 0.00 | 0.00 | 0.00 | reported |
| A2-flat k64 grey0.5 a0.3 bg0.18 | rgba16f | Z | 0.04 (r, j=-1) | 0.00 | 0.07 | 0.00 | reported |
| A2-flat k64 grey0.5 a0.3 bg0.18 | rgba16f | Zpre | 0.04 (r, j=-1) | 0.00 | 0.07 | 0.00 | reported |
| U-eq k64 grey blend_add bg0 | b10g11r11 | N | 6.51 (b, j=-4) | 4.10 | 9.61 | 5.71 | reported |
| U-eq k64 grey blend_add bg0 | b10g11r11 | N16 | 6.51 (b, j=-4) | 4.10 | 9.61 | 5.71 | reported |
| U-eq k64 grey blend_add bg0 | b10g11r11 | Npre | 6.51 (b, j=-4) | 4.10 | 9.61 | 5.71 | reported |
| U-eq k64 grey blend_add bg0 | b10g11r11 | Z | 20.90 (b, j=-4) | 12.55 | 31.54 | 17.87 | reported |
| U-eq k64 grey blend_add bg0 | b10g11r11 | Zpre | 20.90 (b, j=-4) | 12.55 | 31.54 | 17.87 | reported |
| U-eq k64 grey blend_add bg0 | rgba16f | N | 0.18 (r, j=-4) | 0.00 | 0.29 | 0.00 | reported |
| U-eq k64 grey blend_add bg0 | rgba16f | Npre | 0.18 (r, j=-4) | 0.00 | 0.29 | 0.00 | reported |
| U-eq k64 grey blend_add bg0 | rgba16f | Z | 0.52 (r, j=-4) | 0.00 | 0.82 | 0.00 | reported |
| U-eq k64 grey blend_add bg0 | rgba16f | Zpre | 0.52 (r, j=-4) | 0.00 | 0.82 | 0.00 | reported |
| U-eq k64 grey0.2 blend_over bg0 | b10g11r11 | N | 3.85 (b, j=0) | 1.90 | 5.83 | 2.70 | reported |
| U-eq k64 grey0.2 blend_over bg0 | b10g11r11 | N16 | 3.85 (b, j=0) | 1.90 | 5.83 | 2.70 | reported |
| U-eq k64 grey0.2 blend_over bg0 | b10g11r11 | Npre | 3.85 (b, j=0) | 1.90 | 5.83 | 2.70 | reported |
| U-eq k64 grey0.2 blend_over bg0 | b10g11r11 | Z | 8.93 (b, j=0) | 4.61 | 13.55 | 6.59 | reported |
| U-eq k64 grey0.2 blend_over bg0 | b10g11r11 | Zpre | 10.01 (b, j=0) | 5.20 | 15.20 | 7.44 | reported |
| U-eq k64 grey0.2 blend_over bg0 | rgba16f | N | 0.07 (r, j=0) | 0.00 | 0.11 | 0.00 | reported |
| U-eq k64 grey0.2 blend_over bg0 | rgba16f | Npre | 0.07 (r, j=0) | 0.00 | 0.11 | 0.00 | reported |
| U-eq k64 grey0.2 blend_over bg0 | rgba16f | Z | 0.21 (r, j=0) | 0.00 | 0.33 | 0.00 | reported |
| U-eq k64 grey0.2 blend_over bg0 | rgba16f | Zpre | 0.24 (r, j=0) | 0.00 | 0.38 | 0.00 | reported |
| U-near k64 grey blend_add bg0 | b10g11r11 | N | 6.51 (b, j=-4) | 3.61 | 9.68 | 5.02 | reported |
| U-near k64 grey blend_add bg0 | b10g11r11 | N16 | 6.52 (b, j=-4) | 3.91 | 9.64 | 5.44 | reported |
| U-near k64 grey blend_add bg0 | b10g11r11 | Npre | 6.51 (b, j=-4) | 3.63 | 9.68 | 5.04 | reported |
| U-near k64 grey blend_add bg0 | b10g11r11 | Z | 20.87 (b, j=-4) | 12.97 | 31.45 | 18.51 | reported |
| U-near k64 grey blend_add bg0 | b10g11r11 | Zpre | 20.87 (b, j=-4) | 12.97 | 31.45 | 18.51 | reported |
| U-near k64 grey blend_add bg0 | rgba16f | N | 0.01 (r, j=-4) | 0.00 | 0.02 | 0.00 | reported |
| U-near k64 grey blend_add bg0 | rgba16f | Npre | 0.01 (r, j=-4) | 0.00 | 0.02 | 0.00 | reported |
| U-near k64 grey blend_add bg0 | rgba16f | Z | 0.55 (r, j=-4) | 0.00 | 0.86 | 0.00 | reported |
| U-near k64 grey blend_add bg0 | rgba16f | Zpre | 0.55 (r, j=-4) | 0.00 | 0.86 | 0.00 | reported |
| U-near k64 grey0.2 blend_over bg0 | b10g11r11 | N | 1.92 (b, j=0) | 1.98 | 2.76 | 2.80 | reported |
| U-near k64 grey0.2 blend_over bg0 | b10g11r11 | N16 | 1.93 (b, j=0) | 1.89 | 2.79 | 2.68 | reported |
| U-near k64 grey0.2 blend_over bg0 | b10g11r11 | Npre | 1.92 (b, j=0) | 1.99 | 2.77 | 2.82 | reported |
| U-near k64 grey0.2 blend_over bg0 | b10g11r11 | Z | 6.79 (b, j=0) | 4.58 | 10.15 | 6.53 | reported |
| U-near k64 grey0.2 blend_over bg0 | b10g11r11 | Zpre | 7.74 (b, j=0) | 5.29 | 11.57 | 7.55 | reported |
| U-near k64 grey0.2 blend_over bg0 | rgba16f | N | 0.00 (r, j=0) | 0.00 | 0.00 | 0.00 | reported |
| U-near k64 grey0.2 blend_over bg0 | rgba16f | Npre | 0.00 (r, j=0) | 0.00 | 0.00 | 0.00 | reported |
| U-near k64 grey0.2 blend_over bg0 | rgba16f | Z | 0.14 (r, j=0) | 0.00 | 0.22 | 0.00 | reported |
| U-near k64 grey0.2 blend_over bg0 | rgba16f | Zpre | 0.15 (r, j=0) | 0.00 | 0.24 | 0.00 | reported |

Punchy (rev-6.2 H3 row) on the tightest decisive arm, `A1-S1 k64 grey bg0.18`:

| arm | target | class | AgX-Punchy max\|Δ\| | AgX-Punchy chroma | status |
|---|---|---|---|---|---|
| A1-S1 k64 grey bg0.18 | b10g11r11 | N | 2.18 | 2.75 | reported (never decisive) |
| A1-S1 k64 grey bg0.18 | b10g11r11 | N16 | 2.12 | 2.70 | reported (never decisive) |
| A1-S1 k64 grey bg0.18 | b10g11r11 | Npre | 2.04 | 2.61 | reported (never decisive) |
| A1-S1 k64 grey bg0.18 | b10g11r11 | Z | 24.64 | 14.99 | reported (never decisive) |
| A1-S1 k64 grey bg0.18 | b10g11r11 | Zpre | 24.64 | 14.99 | reported (never decisive) |
<!-- px0-expected:end -->

## 5. Decision table — every branch written before the run

The decisive paths are blend `ONE + ONE` (A1), blend premultiplied over (A2) and storage RMW (A3).
Each arm is judged under the class its own path classified as. The content arms run on the device
in every case, on both formats.

| # | Classes of the decisive paths | Model ↔ device agreement on the arm images | Decided by | `lit` |
|---|---|---|---|---|
| 1 | every decisive path a class | every arm's simulated image equals the device dump on ≥ 99.9 % of region pixels | the model (≡ the device) | **B10G11R11** iff every decisive arm passes under its path's class; else **RGBA16F** |
| 2 | every decisive path a class | < 99.9 % on some arm | the device arms | **B10G11R11** iff every device decisive arm passes; else **RGBA16F** |
| 3 | any decisive path `Unmodelled` | — | the device arms | **B10G11R11** iff every device decisive arm passes; else **RGBA16F** |
| 4 | — (the escape's own check) | — | the same arms on an RGBA16F target | the outcome is RGBA16F and any RGBA16F decisive arm fails ⇒ **stop the wave**, escalate to the owner |
| 5 | — (the curve) | — | **void** — Q1 = yes (AgX ships; no substitute curve) | — |
| 6 | — (a later default-curve change) | — | the content arms and this model re-run under the new curve before the change merges (`the_format_decision_curve_is_the_shipping_default` is the tripwire) | a pass keeps the format; a fail ⇒ the late escape with AD-1's re-bless |
| 7 | rows 1–3 would choose RGBA16F **because of A3 alone** (A1 and A2 pass) | — | **A3-blend** (A3's sequences through the premultiplied-over blend), judged under the blend-over path's class | A3-blend passes ⇒ **B10G11R11**, and V2 `fog_apply` / R7 `oit_composite` are bound to the premultiplied-over raster route (**DS-ROW7**, for the orchestrator); A3-blend fails ⇒ **RGBA16F** |

`decided_curve = "agx"`

## 6. Integrity of the pre-registration (H3 rev 5, verify O7; `px0/critique.md` W2)

- `<model>` = the commit that added this record and `crates/boyko_rhi_vulkan/tests/lit_format/` —
  pending, filled at PX0-14.
- `<probe>` = the commit GL-PX0-P ran on (`D:/tmp/render-orch/px0/gl/probe.commit`) — pending.
- `<decision>` = PX0-14.
- Printed at PX0-14 and re-run by the tester:
  - `git merge-base --is-ancestor <model> <probe>` → exit 0;
  - `git diff --exit-code <model> <decision> -- crates/boyko_rhi_vulkan/tests/lit_format crates/boyko_rhi_vulkan/tests/b10g11r11_chain_model.rs` → exit 0
    (the model's sources unchanged since the model commit — an edit of both the model and the
    pinned block after the run is red here, which the pin alone cannot be);
  - the `px0-expected` block at `<decision>` byte-equal to `git show <model>:docs/measurements/2026-10-16-px0-lit-format-decision.md`'s block.

## 7. Classification (pending GL-PX0-P)

## 8. Device arms, both formats (pending GL-PX0-P)

## 9. Decision (pending GL-PX0-P)
