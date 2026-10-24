# PX0 — the `lit` format decision (render queue item 8)

> Render wave PX0, `plan/H3-PX0-instruments-guards.md` Scope 3 / 3b with the binding amendments
> (`04-AMENDMENTS-R6.md` rev-6.2 H3 row: the reported Punchy arm) and `px0/cut.md` §1.3 #2, #3, #6.
> **Status: decided at PX0-14.** The classes, the content model, the expected table and the
> decision table were pre-registered at the model commit (`9572cefc`). §7 classification, §8 device
> arms and §9 decision were filled at PX0-14 from GL-PX0-P's `probe.toml`, run on the probe commit
> `5eda6c78`. **Decision: `lit` = RGBA16F, by row 2. Row 4 did not fire; Row 7 was not reached.**

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

`decided_curve = "neutral"`

*Amended 2026-10-24 (render wave PXt, ruling 116; §10): PX0 decided under `"agx"`; the shipping default is now Khronos PBR Neutral, and §10 re-runs row 6 under it.*

## 6. Integrity of the pre-registration (H3 rev 5, verify O7; `px0/critique.md` W2)

- `<model>` = `9572cefcff519318e6b7903860c55fa73a9a8345`, the commit that added this record and
  `crates/boyko_rhi_vulkan/tests/lit_format/`.
- `<probe>` = `5eda6c7889366d1217fff45a6f786780713af4f1`, the commit GL-PX0-P ran on
  (`D:/tmp/render-orch/px0/gl/probe.commit`).
- `<decision>` = PX0-14, the commit that fills §§7–9. A commit cannot carry its own sha, so the
  tester reads it from `git log`.
- Printed at PX0-14 and re-run by the tester:
  - `git merge-base --is-ancestor <model> <probe>` → exit 0;
  - `git diff --exit-code <model> <decision> -- crates/boyko_rhi_vulkan/tests/lit_format crates/boyko_rhi_vulkan/tests/b10g11r11_chain_model.rs` → exit 0
    (the model's sources unchanged since the model commit — an edit of both the model and the
    pinned block after the run is red here, which the pin alone cannot be);
  - the `px0-expected` block at `<decision>` byte-equal to `git show <model>:docs/measurements/2026-10-16-px0-lit-format-decision.md`'s block.
- Results at PX0-14, printed in its commit message: the ancestry check exits 0; the freeze diff
  exits 0 (run on the tree that became the decision commit); the block is byte-equal to the
  model commit's (md5 `6c308758191c262d249d98b6d6e04512` of the bytes between the two markers,
  at both).

## 7. Classification (GL-PX0-P, probe `5eda6c78`)

- **The run:** the orchestrator's GL-PX0-P (`px0/cut.md` §5) on `5eda6c78`, on an NVIDIA GeForce
  RTX 3060 Laptop GPU (`probe.toml`'s `device`), with the validation layer off
  (`BOYKO_DISABLE_VALIDATION=1`). `gl/probe.log` shows `running 1 test` and `1 passed` (78.12 s), and
  `gl/probe_exit` reads `PROBE_EXIT=0`.
- **The earlier attempt:** the round-1 run on `af9146ee` wrote nothing; it panicked on the probe's
  push range (triage F1). The harness fixes F1, W1 and W3 landed before this run. The model did not
  change (§6's freeze diff). The probe kernels (`crates/boyko_rhi_vulkan/shaders/px0_probe_*`) have
  not changed since the commit that added them, `948975b2`.

| Path | B10G11R11 | RGBA16F |
|---|---|---|
| store | `Z` | `Z` |
| attachment | `Z` | `Z` |
| blend `ONE + ONE` | `Z` | `Unmodelled:1.000` |
| blend premultiplied over | `Z` | `Unmodelled:1.000` |
| clear | `Z` | `Z` |

- **Every path got one class or `Unmodelled`.** None was ambiguous; the probe asserts that.
- **B10G11R11 truncates on every path**, including the clear and both blends. Its single-write
  bounds are one-sided, [−1, 0] ULP (§1).
- **RGBA16F's store, attachment and clear truncate.** Neither blend matches any class of the family
  (the candidates were `N`, `Npre`, `Z` and `Zpre`, §1). The largest error is 1.000 binary16 ULP, the
  limit Vulkan allows when the result rounds to either neighbour. Bounds on those two paths are
  therefore two-sided at 1.000 ULP.
- **The constants:** `goldens::B10G11R11_ROUNDING` / `RGBA16F_ROUNDING` (order `[store, attachment,
  blend_add, blend_over, clear]`) hold these classes. Derived bounds take their class from there;
  nothing reads them before PX1a. The block below is GL-PX0-P's `[classes]` table, copied verbatim from `probe.toml`.
  `goldens::tests::rounding_constants_match_the_record` keeps the constants equal to it, in both
  directions.

<!-- px0-rounding:begin -->
```toml
[classes]
b10g11r11_store = "Z"
b10g11r11_attachment = "Z"
b10g11r11_blend_add = "Z"
b10g11r11_blend_over = "Z"
b10g11r11_clear = "Z"
rgba16f_store = "Z"
rgba16f_attachment = "Z"
rgba16f_blend_add = "Unmodelled:1.000"
rgba16f_blend_over = "Unmodelled:1.000"
rgba16f_clear = "Z"
```
<!-- px0-rounding:end -->

- **Observation, not a decision input:** `probe.toml` is byte-identical (md5
  `ad125af1fa9928e07856790f84e5b3ed`) to the triage-r1 tester's scratch diagnostic
  (`gl/probe_SCRATCH_diag.toml`, a run of an edited copy of the pre-F1 probe). So the W1 barrier fix
  moved no output on this GPU.
- **Not yet run:** W1's validation-on leg (`probe_valon`, `px0/fix_r1.md` device leg 2). Whether
  sync-validation is live on this machine is unproven (`docs/VG-R3-P2-CAPABILITY-SPLIT-PLAN.md`,
  "P2-0 RUN — the sync-validation liveness probe, EXECUTED and INCONCLUSIVE"). The decision does not
  read the two RGBA16F blend classes: rows 1–3 read B10G11R11's decisive paths, and row 4 judges the
  device arms' numbers.

## 8. Device arms, both formats

These are `probe.toml`'s `arms`, one row per arm and target. The pass rule is §2's: both numbers at
most 2 LSB (`lit_format::THRESHOLD_LSB`).

- **agreement:** the fraction of the 4096 region pixels where the device dump equals, bit for bit,
  the image the path's class simulates. It is `—` where the class is `Unmodelled`, which has no
  member to simulate.
- **model (§4) under the class:** §4's pinned row for the same arm, target and the class the
  device's path classified as, so this column sets the pre-registration beside the run. It is `—`
  where §4 has no such row (the `Unmodelled` RGBA16F blends).

**Summary**
- **B10G11R11 agreement:** 40 of the 42 decisive arms agree with their simulation on fewer than
  99.9 % of pixels. The lowest is 0.020264, on `A1-S1 k64 fire bg0.18`; the two A3 arms agree on
  1.000000.
- **B10G11R11 verdicts:** 28 of the 42 decisive arms fail on the device: A1-S1 8 of 8, A1-S2 8 of 8,
  A2 12 of 24, A3 0 of 2.
- **RGBA16F verdicts:** 0 of the 42 decisive arms fail. The largest max |Δ| is 0.049 LSB (`A3 k4
  grey bg0.18`), and the largest chroma is 0.009 LSB.
- **Model vs device:** 62 arm × target rows have a pinned model row under the device's class (every
  B10G11R11 row and RGBA16F's two A3 rows). All 62 have the same pass/fail verdict as the model. The
  largest gap in AgX max |Δ| is 0.929 LSB (`A1-S1 k64 fire bg0.18`, B10G11R11).
- **Punchy (rev-6.2 H3 row), reported and never decisive:** the host model publishes it in §4, on
  `A1-S1 k64 grey bg0.18` on B10G11R11.
  - Under `Z`, the class the device's blend `ONE + ONE` path classified as: 24.64 LSB max |Δ|,
    14.99 LSB chroma.
  - Under `N`: 2.18 / 2.75.
  - The probe does not evaluate Punchy on device images, and §4 has no RGBA16F Punchy row. So the
    format decided in §9 has no Punchy number of its own. P6's `CinematicPost` sign-off reads this
    entry.

**B10G11R11**

| arm | class | agreement | AgX max\|Δ\| | AgX chroma | ACES-Hill max\|Δ\| | ACES-Hill chroma | model (§4) under the class: AgX max\|Δ\| / chroma | verdict (AgX) |
|---|---|---|---|---|---|---|---|---|
| A1-S1 k16 grey bg0 | Z | 0.721436 | 4.313 | 2.270 | 6.513 | 3.224 | 4.44 / 2.27 (FAIL) | FAIL |
| A1-S1 k16 fire bg0 | Z | 0.487793 | 3.261 | 2.035 | 5.502 | 4.497 | 3.40 / 2.11 (FAIL) | FAIL |
| A1-S1 k64 grey bg0 | Z | 0.282959 | 16.400 | 8.664 | 24.888 | 12.378 | 16.91 / 8.57 (FAIL) | FAIL |
| A1-S1 k64 fire bg0 | Z | 0.065918 | 12.302 | 7.802 | 20.583 | 17.610 | 12.86 / 8.05 (FAIL) | FAIL |
| A1-S2 k16 grey bg0 | Z | 0.718994 | 4.120 | 2.110 | 6.201 | 2.982 | 4.25 / 2.10 (FAIL) | FAIL |
| A1-S2 k16 fire bg0 | Z | 0.509766 | 3.195 | 1.985 | 5.361 | 4.371 | 3.33 / 2.05 (FAIL) | FAIL |
| A1-S2 k64 grey bg0 | Z | 0.338867 | 14.500 | 6.886 | 22.077 | 9.824 | 14.88 / 6.76 (FAIL) | FAIL |
| A1-S2 k64 fire bg0 | Z | 0.116943 | 11.350 | 6.935 | 18.796 | 15.749 | 11.79 / 7.10 (FAIL) | FAIL |
| A2 k16 grey0.05 a0.1 bg0 | Z | 0.738037 | 3.348 | 1.714 | 5.065 | 2.428 | 3.46 / 1.72 (FAIL) | FAIL |
| A2 k16 grey0.2 a0.1 bg0 | Z | 0.749268 | 3.377 | 1.731 | 5.108 | 2.453 | 3.48 / 1.73 (FAIL) | FAIL |
| A2 k16 grey0.5 a0.1 bg0 | Z | 0.695068 | 3.120 | 1.633 | 4.699 | 2.310 | 3.22 / 1.63 (FAIL) | FAIL |
| A2 k16 grey0.05 a0.3 bg0 | Z | 0.858398 | 1.480 | 0.768 | 2.225 | 1.083 | 1.53 / 0.77 (pass) | pass |
| A2 k16 grey0.2 a0.3 bg0 | Z | 0.853516 | 1.478 | 0.765 | 2.223 | 1.080 | 1.53 / 0.77 (pass) | pass |
| A2 k16 grey0.5 a0.3 bg0 | Z | 0.856201 | 1.600 | 0.816 | 2.327 | 1.114 | 1.65 / 0.81 (pass) | pass |
| A2 k64 grey0.05 a0.1 bg0 | Z | 0.565674 | 4.556 | 2.431 | 6.879 | 3.453 | 4.72 / 2.45 (FAIL) | FAIL |
| A2 k64 grey0.2 a0.1 bg0 | Z | 0.574707 | 4.535 | 2.410 | 6.847 | 3.423 | 4.69 / 2.42 (FAIL) | FAIL |
| A2 k64 grey0.5 a0.1 bg0 | Z | 0.652344 | 3.956 | 1.851 | 5.824 | 2.559 | 4.07 / 1.84 (FAIL) | FAIL |
| A2 k64 grey0.05 a0.3 bg0 | Z | 0.840576 | 1.490 | 0.772 | 2.240 | 1.089 | 1.54 / 0.77 (pass) | pass |
| A2 k64 grey0.2 a0.3 bg0 | Z | 0.855225 | 1.490 | 0.773 | 2.240 | 1.091 | 1.54 / 0.77 (pass) | pass |
| A2 k64 grey0.5 a0.3 bg0 | Z | 0.850830 | 1.642 | 0.839 | 2.387 | 1.145 | 1.69 / 0.84 (pass) | pass |
| A3 k4 grey bg0 | Z | 1.000000 | 1.200 | 0.603 | 1.774 | 0.834 | 1.20 / 0.60 (pass) | pass |
| A3-blend k4 grey bg0 | Z | 0.935059 | 1.171 | 0.598 | 1.731 | 0.829 | 1.20 / 0.60 (pass) | Row 7 only |
| A2-flat k64 grey0.05 a0.1 bg0 | Z | 0.798340 | 5.824 | 3.032 | 8.820 | 4.317 | 5.84 / 2.95 (reported) | reported |
| A2-flat k64 grey0.2 a0.1 bg0 | Z | 0.801025 | 5.823 | 3.028 | 8.819 | 4.312 | 5.84 / 2.95 (reported) | reported |
| A2-flat k64 grey0.5 a0.1 bg0 | Z | 0.863770 | 4.468 | 2.324 | 6.559 | 3.217 | 4.52 / 2.32 (reported) | reported |
| A2-flat k64 grey0.05 a0.3 bg0 | Z | 0.625000 | 1.658 | 1.036 | 2.478 | 1.467 | 2.00 / 1.38 (reported) | reported |
| A2-flat k64 grey0.2 a0.3 bg0 | Z | 0.642578 | 1.674 | 1.052 | 2.501 | 1.490 | 2.00 / 1.38 (reported) | reported |
| A2-flat k64 grey0.5 a0.3 bg0 | Z | 1.000000 | 1.449 | 0.733 | 2.114 | 1.003 | 1.45 / 0.73 (reported) | reported |
| A1-S1 k16 grey bg0.18 | Z | 0.710938 | 4.487 | 2.373 | 6.744 | 3.358 | 4.62 / 2.37 (FAIL) | FAIL |
| A1-S1 k16 fire bg0.18 | Z | 0.433594 | 3.968 | 2.491 | 6.705 | 5.279 | 4.12 / 2.56 (FAIL) | FAIL |
| A1-S1 k64 grey bg0.18 | Z | 0.243408 | 17.626 | 9.598 | 26.707 | 13.748 | 18.20 / 9.52 (FAIL) | FAIL |
| A1-S1 k64 fire bg0.18 | Z | 0.020264 | 17.521 | 11.944 | 30.244 | 24.489 | 18.45 / 12.48 (FAIL) | FAIL |
| A1-S2 k16 grey bg0.18 | Z | 0.712402 | 4.289 | 2.200 | 6.433 | 3.101 | 4.43 / 2.20 (FAIL) | FAIL |
| A1-S2 k16 fire bg0.18 | Z | 0.453369 | 3.964 | 2.496 | 6.683 | 5.255 | 4.12 / 2.57 (FAIL) | FAIL |
| A1-S2 k64 grey bg0.18 | Z | 0.299316 | 15.348 | 7.358 | 23.342 | 10.516 | 15.75 / 7.22 (FAIL) | FAIL |
| A1-S2 k64 fire bg0.18 | Z | 0.071777 | 14.050 | 8.815 | 23.720 | 18.140 | 14.59 / 9.05 (FAIL) | FAIL |
| A2 k16 grey0.05 a0.1 bg0.18 | Z | 0.584229 | 5.514 | 2.882 | 8.176 | 3.985 | 5.71 / 2.87 (FAIL) | FAIL |
| A2 k16 grey0.2 a0.1 bg0.18 | Z | 0.647461 | 3.722 | 1.959 | 5.624 | 2.783 | 3.85 / 1.97 (FAIL) | FAIL |
| A2 k16 grey0.5 a0.1 bg0.18 | Z | 0.676758 | 3.237 | 1.703 | 4.838 | 2.394 | 3.35 / 1.71 (FAIL) | FAIL |
| A2 k16 grey0.05 a0.3 bg0.18 | Z | 0.846680 | 1.516 | 0.778 | 2.280 | 1.097 | 1.57 / 0.78 (pass) | pass |
| A2 k16 grey0.2 a0.3 bg0.18 | Z | 0.853271 | 1.490 | 0.777 | 2.240 | 1.095 | 1.54 / 0.78 (pass) | pass |
| A2 k16 grey0.5 a0.3 bg0.18 | Z | 0.855225 | 1.608 | 0.818 | 2.339 | 1.117 | 1.66 / 0.81 (pass) | pass |
| A2 k64 grey0.05 a0.1 bg0.18 | Z | 0.566650 | 4.540 | 2.404 | 6.854 | 3.414 | 4.70 / 2.41 (FAIL) | FAIL |
| A2 k64 grey0.2 a0.1 bg0.18 | Z | 0.568604 | 4.549 | 2.417 | 6.869 | 3.434 | 4.71 / 2.42 (FAIL) | FAIL |
| A2 k64 grey0.5 a0.1 bg0.18 | Z | 0.633301 | 3.964 | 1.857 | 5.835 | 2.568 | 4.08 / 1.84 (FAIL) | FAIL |
| A2 k64 grey0.05 a0.3 bg0.18 | Z | 0.854736 | 1.498 | 0.778 | 2.254 | 1.098 | 1.54 / 0.77 (pass) | pass |
| A2 k64 grey0.2 a0.3 bg0.18 | Z | 0.846924 | 1.511 | 0.790 | 2.271 | 1.115 | 1.56 / 0.79 (pass) | pass |
| A2 k64 grey0.5 a0.3 bg0.18 | Z | 0.851562 | 1.635 | 0.820 | 2.378 | 1.119 | 1.68 / 0.81 (pass) | pass |
| A3 k4 grey bg0.18 | Z | 1.000000 | 1.582 | 0.798 | 2.372 | 1.122 | 1.58 / 0.80 (pass) | pass |
| A3-blend k4 grey bg0.18 | Z | 0.863770 | 1.533 | 0.795 | 2.296 | 1.117 | 1.59 / 0.80 (pass) | Row 7 only |
| A2-flat k64 grey0.05 a0.1 bg0.18 | Z | 1.000000 | 0.321 | 0.000 | 0.507 | 0.000 | 0.32 / 0.00 (reported) | reported |
| A2-flat k64 grey0.2 a0.1 bg0.18 | Z | 0.678467 | 4.880 | 2.095 | 7.445 | 2.979 | 4.88 / 1.94 (reported) | reported |
| A2-flat k64 grey0.5 a0.1 bg0.18 | Z | 0.906006 | 4.475 | 2.318 | 6.570 | 3.209 | 4.51 / 2.32 (reported) | reported |
| A2-flat k64 grey0.05 a0.3 bg0.18 | Z | 1.000000 | 0.180 | 0.000 | 0.284 | 0.000 | 0.18 / 0.00 (reported) | reported |
| A2-flat k64 grey0.2 a0.3 bg0.18 | Z | 0.597168 | 1.631 | 1.009 | 2.440 | 1.428 | 2.00 / 1.38 (reported) | reported |
| A2-flat k64 grey0.5 a0.3 bg0.18 | Z | 1.000000 | 1.449 | 0.733 | 2.114 | 1.003 | 1.45 / 0.73 (reported) | reported |
| U-eq k64 grey blend_add bg0 | Z | 1.000000 | 20.902 | 12.545 | 31.543 | 17.874 | 20.90 / 12.55 (reported) | reported |
| U-eq k64 grey0.2 blend_over bg0 | Z | 1.000000 | 8.931 | 4.612 | 13.554 | 6.585 | 8.93 / 4.61 (reported) | reported |
| U-near k64 grey blend_add bg0 | Z | 0.968750 | 20.866 | 12.984 | 31.443 | 18.532 | 20.87 / 12.97 (reported) | reported |
| U-near k64 grey0.2 blend_over bg0 | Z | 0.843262 | 6.786 | 4.651 | 10.138 | 6.633 | 6.79 / 4.58 (reported) | reported |

**RGBA16F**

| arm | class | agreement | AgX max\|Δ\| | AgX chroma | ACES-Hill max\|Δ\| | ACES-Hill chroma | model (§4) under the class: AgX max\|Δ\| / chroma | verdict (AgX) |
|---|---|---|---|---|---|---|---|---|
| A1-S1 k16 grey bg0 | Unmodelled | — | 0.016 | 0.000 | 0.025 | 0.000 | — | pass |
| A1-S1 k16 fire bg0 | Unmodelled | — | 0.018 | 0.008 | 0.029 | 0.019 | — | pass |
| A1-S1 k64 grey bg0 | Unmodelled | — | 0.018 | 0.000 | 0.028 | 0.000 | — | pass |
| A1-S1 k64 fire bg0 | Unmodelled | — | 0.018 | 0.009 | 0.030 | 0.019 | — | pass |
| A1-S2 k16 grey bg0 | Unmodelled | — | 0.017 | 0.000 | 0.026 | 0.000 | — | pass |
| A1-S2 k16 fire bg0 | Unmodelled | — | 0.017 | 0.009 | 0.027 | 0.020 | — | pass |
| A1-S2 k64 grey bg0 | Unmodelled | — | 0.017 | 0.000 | 0.027 | 0.000 | — | pass |
| A1-S2 k64 fire bg0 | Unmodelled | — | 0.017 | 0.009 | 0.028 | 0.020 | — | pass |
| A2 k16 grey0.05 a0.1 bg0 | Unmodelled | — | 0.007 | 0.000 | 0.011 | 0.000 | — | pass |
| A2 k16 grey0.2 a0.1 bg0 | Unmodelled | — | 0.006 | 0.000 | 0.009 | 0.000 | — | pass |
| A2 k16 grey0.5 a0.1 bg0 | Unmodelled | — | 0.006 | 0.000 | 0.009 | 0.000 | — | pass |
| A2 k16 grey0.05 a0.3 bg0 | Unmodelled | — | 0.001 | 0.000 | 0.001 | 0.000 | — | pass |
| A2 k16 grey0.2 a0.3 bg0 | Unmodelled | — | 0.001 | 0.000 | 0.002 | 0.000 | — | pass |
| A2 k16 grey0.5 a0.3 bg0 | Unmodelled | — | 0.001 | 0.000 | 0.001 | 0.000 | — | pass |
| A2 k64 grey0.05 a0.1 bg0 | Unmodelled | — | 0.000 | 0.000 | 0.000 | 0.000 | — | pass |
| A2 k64 grey0.2 a0.1 bg0 | Unmodelled | — | 0.000 | 0.000 | 0.001 | 0.000 | — | pass |
| A2 k64 grey0.5 a0.1 bg0 | Unmodelled | — | 0.000 | 0.000 | 0.000 | 0.000 | — | pass |
| A2 k64 grey0.05 a0.3 bg0 | Unmodelled | — | 0.001 | 0.000 | 0.001 | 0.000 | — | pass |
| A2 k64 grey0.2 a0.3 bg0 | Unmodelled | — | 0.000 | 0.000 | 0.000 | 0.000 | — | pass |
| A2 k64 grey0.5 a0.3 bg0 | Unmodelled | — | 0.001 | 0.000 | 0.001 | 0.000 | — | pass |
| A3 k4 grey bg0 | Z | 0.999756 | 0.038 | 0.000 | 0.059 | 0.000 | 0.04 / 0.00 (pass) | pass |
| A3-blend k4 grey bg0 | Unmodelled | — | 0.014 | 0.000 | 0.021 | 0.000 | — | Row 7 only |
| A2-flat k64 grey0.05 a0.1 bg0 | Unmodelled | — | 0.005 | 0.000 | 0.009 | 0.000 | — | reported |
| A2-flat k64 grey0.2 a0.1 bg0 | Unmodelled | — | 0.005 | 0.000 | 0.008 | 0.000 | — | reported |
| A2-flat k64 grey0.5 a0.1 bg0 | Unmodelled | — | 0.000 | 0.000 | 0.001 | 0.000 | — | reported |
| A2-flat k64 grey0.05 a0.3 bg0 | Unmodelled | — | 0.002 | 0.000 | 0.003 | 0.000 | — | reported |
| A2-flat k64 grey0.2 a0.3 bg0 | Unmodelled | — | 0.002 | 0.000 | 0.003 | 0.000 | — | reported |
| A2-flat k64 grey0.5 a0.3 bg0 | Unmodelled | — | 0.000 | 0.000 | 0.000 | 0.000 | — | reported |
| A1-S1 k16 grey bg0.18 | Unmodelled | — | 0.016 | 0.000 | 0.025 | 0.000 | — | pass |
| A1-S1 k16 fire bg0.18 | Unmodelled | — | 0.016 | 0.008 | 0.026 | 0.016 | — | pass |
| A1-S1 k64 grey bg0.18 | Unmodelled | — | 0.018 | 0.000 | 0.028 | 0.000 | — | pass |
| A1-S1 k64 fire bg0.18 | Unmodelled | — | 0.016 | 0.006 | 0.025 | 0.014 | — | pass |
| A1-S2 k16 grey bg0.18 | Unmodelled | — | 0.015 | 0.000 | 0.024 | 0.000 | — | pass |
| A1-S2 k16 fire bg0.18 | Unmodelled | — | 0.016 | 0.008 | 0.026 | 0.016 | — | pass |
| A1-S2 k64 grey bg0.18 | Unmodelled | — | 0.017 | 0.000 | 0.027 | 0.000 | — | pass |
| A1-S2 k64 fire bg0.18 | Unmodelled | — | 0.017 | 0.007 | 0.027 | 0.016 | — | pass |
| A2 k16 grey0.05 a0.1 bg0.18 | Unmodelled | — | 0.010 | 0.000 | 0.016 | 0.000 | — | pass |
| A2 k16 grey0.2 a0.1 bg0.18 | Unmodelled | — | 0.000 | 0.000 | 0.000 | 0.000 | — | pass |
| A2 k16 grey0.5 a0.1 bg0.18 | Unmodelled | — | 0.004 | 0.000 | 0.005 | 0.000 | — | pass |
| A2 k16 grey0.05 a0.3 bg0.18 | Unmodelled | — | 0.001 | 0.000 | 0.001 | 0.000 | — | pass |
| A2 k16 grey0.2 a0.3 bg0.18 | Unmodelled | — | 0.000 | 0.000 | 0.000 | 0.000 | — | pass |
| A2 k16 grey0.5 a0.3 bg0.18 | Unmodelled | — | 0.001 | 0.000 | 0.001 | 0.000 | — | pass |
| A2 k64 grey0.05 a0.1 bg0.18 | Unmodelled | — | 0.001 | 0.000 | 0.001 | 0.000 | — | pass |
| A2 k64 grey0.2 a0.1 bg0.18 | Unmodelled | — | 0.001 | 0.000 | 0.001 | 0.000 | — | pass |
| A2 k64 grey0.5 a0.1 bg0.18 | Unmodelled | — | 0.001 | 0.000 | 0.001 | 0.000 | — | pass |
| A2 k64 grey0.05 a0.3 bg0.18 | Unmodelled | — | 0.000 | 0.000 | 0.000 | 0.000 | — | pass |
| A2 k64 grey0.2 a0.3 bg0.18 | Unmodelled | — | 0.001 | 0.000 | 0.001 | 0.000 | — | pass |
| A2 k64 grey0.5 a0.3 bg0.18 | Unmodelled | — | 0.001 | 0.000 | 0.001 | 0.000 | — | pass |
| A3 k4 grey bg0.18 | Z | 0.999756 | 0.049 | 0.000 | 0.077 | 0.000 | 0.05 / 0.00 (pass) | pass |
| A3-blend k4 grey bg0.18 | Unmodelled | — | 0.001 | 0.000 | 0.002 | 0.000 | — | Row 7 only |
| A2-flat k64 grey0.05 a0.1 bg0.18 | Unmodelled | — | 0.001 | 0.000 | 0.001 | 0.000 | — | reported |
| A2-flat k64 grey0.2 a0.1 bg0.18 | Unmodelled | — | 0.009 | 0.000 | 0.014 | 0.000 | — | reported |
| A2-flat k64 grey0.5 a0.1 bg0.18 | Unmodelled | — | 0.000 | 0.000 | 0.000 | 0.000 | — | reported |
| A2-flat k64 grey0.05 a0.3 bg0.18 | Unmodelled | — | 0.002 | 0.000 | 0.003 | 0.000 | — | reported |
| A2-flat k64 grey0.2 a0.3 bg0.18 | Unmodelled | — | 0.002 | 0.000 | 0.003 | 0.000 | — | reported |
| A2-flat k64 grey0.5 a0.3 bg0.18 | Unmodelled | — | 0.000 | 0.000 | 0.000 | 0.000 | — | reported |
| U-eq k64 grey blend_add bg0 | Unmodelled | — | 0.185 | 0.000 | 0.292 | 0.001 | — | reported |
| U-eq k64 grey0.2 blend_over bg0 | Unmodelled | — | 0.154 | 0.000 | 0.243 | 0.000 | — | reported |
| U-near k64 grey blend_add bg0 | Unmodelled | — | 0.002 | 0.000 | 0.003 | 0.000 | — | reported |
| U-near k64 grey0.2 blend_over bg0 | Unmodelled | — | 0.063 | 0.000 | 0.099 | 0.000 | — | reported |

## 9. Decision

This applies §5's table row by row to §§7–8. The script is `D:/tmp/render-orch/px0/px0_14/decide.py`;
it reads `probe.toml` and §4, and its output is `decision.txt` beside it. No threshold is derived
here.

1. **Classes of the decisive paths (B10G11R11):** blend `ONE + ONE` `Z`, blend premultiplied over
   `Z`, store `Z`. Every decisive path has a class, so the case is row 1 or 2, not row 3.
2. **Agreement:** 40 of the 42 decisive arms are below 99.9 % (§8). That is **row 2**: the device
   arms decide.
3. **Row 2's rule:** B10G11R11 only if every device decisive arm passes. 28 of the 42 fail (A1-S1,
   A1-S2, A2), so **`lit` = RGBA16F**.
4. **Row 7:** not reached. It applies only when RGBA16F is chosen because of A3 alone; here A1 and
   A2 fail and A3 passes (0 of 2 fail). DS-ROW7 is not named.
5. **Row 4, the escape's own check:** the outcome is RGBA16F, and 0 of the 42 RGBA16F decisive arms
   fail. **Row 4 does not fire, and the wave continues.**
6. **Rows 5 and 6:** row 5 is void (Q1 = yes). Row 6 covers a later change of the default curve.
   The decided curve in §5 stays AgX *(amended by §10, ruling 116)*.

**One reading to state.** The table's "classes of the decisive paths" are read on B10G11R11, the
format under test; RGBA16F is row 4's escape. The probe's `decide()` reads them the same way.
Counting RGBA16F's two `Unmodelled` blends in rows 1–3 as well would select row 3. Rows 2 and 3 have
the same "decided by" and `lit` cells, so the outcome is the same under either reading. The probe's
own `[decision]` (`row = 2`, `lit = "rgba16f"`, `stop_the_wave = false`) matches this application.

**Decision: `lit` = RGBA16F (`R16G16B16A16_SFLOAT`), by row 2.**

What follows from it:
- `boyko_shaderdsl::post::LIT_STORAGE_FORMAT` = `"rgba16f"` (PX0-14). Nothing emits the string
  before PX1a, so no shader, `.spv` or golden moves at PX0.
- Every budget that touches `lit` reads AD-1's escape column (`plan/01-DECISIONS.md` §3.1; `plan/00-INDEX.md`
  cross-cutting rule (x)).

## 10. Amendment, 2026-10-24: the default curve is Khronos PBR Neutral (render wave PXt, ruling 116)

Ruling 116 (the orchestrator on the owner's delegation, 2026-10-24) makes Khronos PBR Neutral the
shipping default tone curve (`boyko_render::light::Tonemapper::Neutral`, tonemap word 1), not AgX.
AgX (word 3) and ACES Hill (word 0) stay selectable. PX0 decided `lit` under AgX (§2's `D`, and
§5's `decided_curve` until this amendment), so §5's row 6 applies: the content arms and this model
re-run under the new curve before the change merges.

§§1–9 stand as written, with two exceptions: §5's `decided_curve` line now reads `"neutral"`, with
an amendment note under it, and §9's item 6 points here (*(amended by §10, ruling 116)*). The D-P4
tripwire (`the_format_decision_curve_is_the_shipping_default`) compares the `decided_curve` line
with `Tonemapper::default()`. The pre-registered `px0-expected` block is unchanged (md5
`6c308758191c262d249d98b6d6e04512` of the bytes between its markers, as in §6).

**What was re-run.** The host content model of §2 and §4 (`crates/boyko_rhi_vulkan/tests/lit_format/`)
with `D` = Khronos PBR Neutral, followed by the same OETF `pow(x, 1/2.2)`, ×255, unquantised.
Everything else is §2's: the arms, their seeds, the region, the background codes, the placements
`j ∈ [−8, 4]` and the 2-LSB rule. The curve is `post_f64::neutral_f64` (`lit_format::Curve::Neutral`),
the independent `f64` transcription of the KhronosGroup/ToneMapping README.

The device arms were not re-run. The probe keeps no `lit` images, only its per-arm numbers (§8), so
a device re-run would need the probe on the device with a Neutral column.

**How an arm is judged.**
- **RGBA16F** (the format §9 decided): an arm passes only if it passes under every modelled class
  of its path. For A3's store those are `N` and `Z`; for the blends, `N`, `Npre`, `Z` and `Zpre`.
  The device classified RGBA16F's store as `Z` and both blends as `Unmodelled:1.000` (§7), so no
  single class stands for the blends.
- **B10G11R11** under `Z`, its device class on every path: reported for the record only. It is
  not a decision input, because §9 rejected B10G11R11 on the device arms.

**The gate and the command.** `b10g11r11_chain_model::the_decided_curve_keeps_the_decided_format`
is a host test and is not ignored. It reads §5's `decided_curve`, maps it to the model's curve,
asserts that every decisive RGBA16F arm passes, and prints the table below:

```text
cargo test -p boyko_rhi_vulkan --test b10g11r11_chain_model --locked the_decided_curve_keeps_the_decided_format -- --exact --nocapture
```

**Cross-check of the gate's arithmetic.** Run against the record before this amendment
(`decided_curve = "agx"`), the same test reproduces §4's pinned AgX rows:

| arm | value | the gate under AgX | §4 |
|---|---|---|---|
| `A1-S1 k64 grey bg0` | RGBA16F `Z` max\|Δ\| | 0.531 | 0.53 |
| `A1-S1 k64 grey bg0` | B10G11R11 `Z` max\|Δ\| / chroma | 16.907 / 8.573 | 16.91 / 8.57 |
| `A2 k16 grey0.05 a0.1 bg0` | RGBA16F worst class (`Zpre`) max\|Δ\| | 0.126 | 0.13 |
| `A2 k16 grey0.05 a0.1 bg0` | B10G11R11 `Z` max\|Δ\| / chroma | 3.457 / 1.715 | 3.46 / 1.72 |

Under AgX the gate counts 0 of 42 RGBA16F failures (worst 0.656 / 0.336 LSB) and 28 of 42
B10G11R11 `Z` failures. §8 measured the same 28 of 42 on the device.

**Result under Neutral.**
- **RGBA16F:** 0 of 42 decisive arms fail under every modelled class. The worst max|Δ| is
  1.163 LSB (`A1-S1 k64 grey bg0`) and the worst chroma 0.631 LSB (`A1-S1 k64 fire bg0.18`),
  against a 2-LSB threshold. The margin is smaller than AgX's (0.656 / 0.336 LSB): Neutral is the
  identity between its toe and its shoulder, while AgX's log encoding compresses the error there.
- **B10G11R11 under `Z`** (record only): 42 of 42 fail, both A3 arms included (2.434 and 3.272 LSB
  max|Δ|). Under AgX 28 of 42 failed, so the choice against B10G11R11 is firmer under Neutral.
- **Row 6's verdict: a pass. `lit` stays RGBA16F.** This record calls for no late escape and no
  re-bless.

| arm | RGBA16F `Z`: Neutral max\|Δ\| (channel, placement) | Neutral chroma | RGBA16F, every modelled class: worst max\|Δ\| / chroma | verdict (Neutral, RGBA16F) | B10G11R11 `Z`: Neutral max\|Δ\| / chroma | B10G11R11 `Z` (record only) |
|---|---|---|---|---|---|---|
| A1-S1 k16 grey bg0 | 0.279 (r, j=-2) | 0.000 | 0.280 / 0.000 | pass | 9.823 / 5.251 | FAIL |
| A1-S1 k16 fire bg0 | 0.280 (r, j=-2) | 0.154 | 0.280 / 0.154 | pass | 5.257 / 4.096 | FAIL |
| A1-S1 k64 grey bg0 | 1.162 (r, j=-2) | 0.000 | 1.163 / 0.000 | pass | 37.242 / 20.078 | FAIL |
| A1-S1 k64 fire bg0 | 1.160 (r, j=-2) | 0.595 | 1.161 / 0.595 | pass | 19.136 / 15.959 | FAIL |
| A1-S2 k16 grey bg0 | 0.250 (r, j=-2) | 0.000 | 0.251 / 0.000 | pass | 8.915 / 4.875 | FAIL |
| A1-S2 k16 fire bg0 | 0.253 (r, j=-2) | 0.153 | 0.254 / 0.154 | pass | 5.108 / 3.974 | FAIL |
| A1-S2 k64 grey bg0 | 1.084 (r, j=-2) | 0.000 | 1.084 / 0.000 | pass | 32.450 / 15.889 | FAIL |
| A1-S2 k64 fire bg0 | 1.087 (r, j=-2) | 0.592 | 1.087 / 0.592 | pass | 17.781 / 14.363 | FAIL |
| A2 k16 grey0.05 a0.1 bg0 | 0.237 (r, j=4) | 0.000 | 0.271 / 0.000 | pass | 7.419 / 3.647 | FAIL |
| A2 k16 grey0.2 a0.1 bg0 | 0.238 (r, j=2) | 0.000 | 0.273 / 0.000 | pass | 7.454 / 4.159 | FAIL |
| A2 k16 grey0.5 a0.1 bg0 | 0.189 (r, j=1) | 0.000 | 0.219 / 0.000 | pass | 6.946 / 3.767 | FAIL |
| A2 k16 grey0.05 a0.3 bg0 | 0.091 (r, j=4) | 0.000 | 0.120 / 0.000 | pass | 3.237 / 1.762 | FAIL |
| A2 k16 grey0.2 a0.3 bg0 | 0.091 (r, j=2) | 0.000 | 0.120 / 0.000 | pass | 3.240 / 1.785 | FAIL |
| A2 k16 grey0.5 a0.3 bg0 | 0.105 (r, j=0) | 0.000 | 0.135 / 0.000 | pass | 3.173 / 1.941 | FAIL |
| A2 k64 grey0.05 a0.1 bg0 | 0.292 (r, j=4) | 0.000 | 0.326 / 0.000 | pass | 10.491 / 5.564 | FAIL |
| A2 k64 grey0.2 a0.1 bg0 | 0.290 (r, j=2) | 0.000 | 0.323 / 0.000 | pass | 10.406 / 5.602 | FAIL |
| A2 k64 grey0.5 a0.1 bg0 | 0.313 (r, j=0) | 0.000 | 0.343 / 0.000 | pass | 7.740 / 4.441 | FAIL |
| A2 k64 grey0.05 a0.3 bg0 | 0.091 (r, j=4) | 0.000 | 0.122 / 0.000 | pass | 3.268 / 1.769 | FAIL |
| A2 k64 grey0.2 a0.3 bg0 | 0.092 (r, j=2) | 0.000 | 0.121 / 0.000 | pass | 3.261 / 1.791 | FAIL |
| A2 k64 grey0.5 a0.3 bg0 | 0.107 (r, j=0) | 0.000 | 0.138 / 0.000 | pass | 3.267 / 2.011 | FAIL |
| A3 k4 grey bg0 | 0.071 (r, j=3) | 0.000 | 0.071 / 0.000 | pass | 2.434 / 1.422 | FAIL |
| A1-S1 k16 grey bg0.18 | 0.253 (r, j=-3) | 0.000 | 0.253 / 0.000 | pass | 9.792 / 5.521 | FAIL |
| A1-S1 k16 fire bg0.18 | 0.249 (r, j=-2) | 0.163 | 0.250 / 0.163 | pass | 6.371 / 4.690 | FAIL |
| A1-S1 k64 grey bg0.18 | 0.986 (r, j=-2) | 0.000 | 0.986 / 0.000 | pass | 39.790 / 22.027 | FAIL |
| A1-S1 k64 fire bg0.18 | 0.988 (r, j=-2) | 0.631 | 0.988 / 0.631 | pass | 30.538 / 22.773 | FAIL |
| A1-S2 k16 grey bg0.18 | 0.255 (r, j=-3) | 0.000 | 0.256 / 0.000 | pass | 9.038 / 5.079 | FAIL |
| A1-S2 k16 fire bg0.18 | 0.247 (r, j=-3) | 0.159 | 0.249 / 0.159 | pass | 6.344 / 4.672 | FAIL |
| A1-S2 k64 grey bg0.18 | 0.976 (r, j=-3) | 0.000 | 0.976 / 0.000 | pass | 33.903 / 16.783 | FAIL |
| A1-S2 k64 fire bg0.18 | 0.979 (r, j=-2) | 0.623 | 0.979 / 0.624 | pass | 22.110 / 16.778 | FAIL |
| A2 k16 grey0.05 a0.1 bg0.18 | 0.366 (r, j=3) | 0.000 | 0.385 / 0.000 | pass | 11.645 / 6.926 | FAIL |
| A2 k16 grey0.2 a0.1 bg0.18 | 0.248 (r, j=2) | 0.000 | 0.276 / 0.000 | pass | 8.597 / 4.575 | FAIL |
| A2 k16 grey0.5 a0.1 bg0.18 | 0.187 (r, j=0) | 0.000 | 0.214 / 0.000 | pass | 6.729 / 4.037 | FAIL |
| A2 k16 grey0.05 a0.3 bg0.18 | 0.093 (r, j=4) | 0.000 | 0.123 / 0.000 | pass | 3.318 / 1.802 | FAIL |
| A2 k16 grey0.2 a0.3 bg0.18 | 0.091 (r, j=2) | 0.000 | 0.122 / 0.000 | pass | 3.264 / 1.798 | FAIL |
| A2 k16 grey0.5 a0.3 bg0.18 | 0.104 (r, j=0) | 0.000 | 0.136 / 0.000 | pass | 3.194 / 1.958 | FAIL |
| A2 k64 grey0.05 a0.1 bg0.18 | 0.288 (r, j=4) | 0.000 | 0.321 / 0.000 | pass | 10.406 / 5.507 | FAIL |
| A2 k64 grey0.2 a0.1 bg0.18 | 0.291 (r, j=2) | 0.000 | 0.325 / 0.000 | pass | 10.454 / 5.616 | FAIL |
| A2 k64 grey0.5 a0.1 bg0.18 | 0.315 (r, j=0) | 0.000 | 0.347 / 0.000 | pass | 7.763 / 4.443 | FAIL |
| A2 k64 grey0.05 a0.3 bg0.18 | 0.092 (r, j=4) | 0.000 | 0.123 / 0.000 | pass | 3.281 / 1.774 | FAIL |
| A2 k64 grey0.2 a0.3 bg0.18 | 0.092 (r, j=2) | 0.000 | 0.122 / 0.000 | pass | 3.314 / 1.823 | FAIL |
| A2 k64 grey0.5 a0.3 bg0.18 | 0.108 (r, j=0) | 0.000 | 0.139 / 0.000 | pass | 3.247 / 1.958 | FAIL |
| A3 k4 grey bg0.18 | 0.088 (r, j=2) | 0.000 | 0.088 / 0.000 | pass | 3.272 / 1.845 | FAIL |

**The tripwire, red before the flip.** The batch ran on render wave PXt's C8b tree with
`Tonemapper`'s `#[default]` still on AgX. The logs are `D:/tmp/render-orch/pxt/neutral/logs/red_chain_*.log`.
- Against this record (`decided_curve = "neutral"`): `the_format_decision_curve_is_the_shipping_default`
  FAILED with `left: "agx"`, `right: "neutral"`.
- Against the record before this amendment (`"agx"`): the tripwire passed, and the re-aimed parse test
  `the_record_names_neutral_as_its_decided_curve` FAILED with `left: Some("agx")`, `right: Some("neutral")`.

Both pass on the commit that moves `#[default]` to `Neutral` (render wave PXt C8b).
