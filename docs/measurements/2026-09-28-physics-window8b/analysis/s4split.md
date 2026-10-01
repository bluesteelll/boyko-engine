# Window 8b: S4-AB + SPLIT (results-analyst, 2026-09-29)

VERDICTS: S4-W8 (gain >= 0.060 ms, J-T W8) = CLAIMED (span route, STRONG; the wall difference is itself CLAIMED
STRONG); R_8 <= 0.060 ms (in-block rung) = NOT CLAIMED; zone canary 30 us = CLAIMED STRONG, 60 us = CLAIMED STRONG;
no-slower W1 = holds (no resolved difference), W2 = holds (TIP faster CLAIMED), W4 = holds (TIP faster CLAIMED),
W16 = holds (no resolved difference); poses equal = holds; S4 merge decision = KEEP. No VOID.

S4 does what it was built for. At W8 on J-T, the TIP wall is 0.1705 ms faster than PARENT (-8.58 %, 2.84x the
0.060 ms bar). The `phys_solve_build` span is 0.1894 ms faster (-62 %). Both are claimed in every pass and pooled,
under r, i and s. The block's own 0.060 ms rung was not seen, so R_8 is not demonstrated in-block. The claim
therefore stands on the pre-registered span route, where both zone canaries are seen at 1.01x their injection. No
W is claimed slower. W16's wall gain (0.170 ms) is not claimed: three uniformly slow PARENT processes at W16 break
pass 2 (and pass 1 on the armed wall).

SPLIT resolves window 8's B1/B2 NO-GO:
- The first-wave (recruitment) ramp is 4.1 us per step.
- The pass-first ramps average 2.96 us each, 9.8x an ordinary wave's 0.30 us.
- The tail is 72 % imbalance and 28 % join.
- Ramp + tail are 49 % of omega(8). The other half of L_wide is inside the waves.

Window 8's own per-wave table was in TSC ticks, not ns (x3.29, post hoc 1).

Scripts and outputs: `analysis/s4split/` (`lib_s4.py`, `common_s4.py`, `sel_s4split.py` -> `sel_s4split_out.txt` +
`proc_s4split.json`, `q_s4.py` -> `q_s4.txt`, `q_s4_procs.py` -> `q_s4_procs.txt`, `q_s4_arith.py` ->
`q_s4_arith.txt`, `mk_table.py` -> `mk_table.txt`, `q_split.py` -> `q_split.txt`, `q_derive.py` -> `q_derive.txt`,
`q_checks.py` -> `q_checks.txt`, `q_posthoc.py` -> `q_posthoc.txt`). Every number below is in one of those outputs.

## Method
- **Sources.** Recomputed from `raw/<block>-p<N>_172102/*/run.csv` and each process's stdout SUMMARY. No driver
  statistic is used, except as a check: the CSV window mean equals the SUMMARY `window_mean_ns` in every process.
- **Statistics.** A cell is the median over K of the per-process means. Pooled K = 9 (3 passes x 3 rounds); per
  pass, K = 3.
  - r = (max-min)/median.
  - i = IQR (inclusive quartiles)/median.
  - s = 1.2533*SD/sqrt(K)/median.
  - A flag x is set iff |B/A-1| > 2*hypot(x_A, x_B) (window 8's lib8 definitions, verbatim).
- **Ruling 1.**
  - CLAIMED = i AND s flag, in the same direction, pooled and in each of p0, p1 and p2.
  - STRONG = r also flags, pooled and in every pass.
  - REFUTED = the opposite direction is CLAIMED.
- **Windows.** The verdict window is [0,500): the rows' window and window 8's headline convention. [100,500) is
  reported beside it.
- **Selection** (slot rule of window 8):
  - S4-AB: 155 processes (153 originals, 2 re-runs), 153 slots, 153 used, 0 dropped.
  - SPLIT: 54 processes, 54 slots, 54 used.
- **Own re-validation: 0 problems.** Checked per process:
  - exit, sha256, 500 CSV steps, void 0;
  - pose `0x30c5438bc6ad9ffa` with `expect_pose` match;
  - workers, msvc, the armed flag, drops and overflow 0, disarmed ring traffic 0;
  - TreeDiag, `kd_order_builds` 0, the canary fields;
  - the S4 setup counters by binary: PARENT 0/0 at W 8/16 in S4-AB and at W 1/8/16 in SPLIT; TIP 500/15980 at
    W8/W16.
- **Receipts.** Used processes: before and after <= 4.08 %, witness <= 1.22 %. No pass voided; no block void rule
  fired.
- **Units.** The CSV's counter columns (wave ramp, tail, join, first and pass ramps, np ramp, tail and join) are
  raw TSC ticks: the runner's `csv_text` converts only span and system columns. They are divided here by the
  SUMMARY `ticks_per_ns` (3.2935). The CSV-derived [0,500) per-wave means equal the SUMMARY `w8s` fields to
  3.3e-16 over 18 processes.

## 1. Verdict table (S4-AB; A = reference, B = test; [0,500); flags in r/i/s order, pooled then per pass)

| claim | pre-registered bar | cells, n | numbers (ms; A = reference, B = test) | flags, printed r/i/s (claim = i and s) | verdict |
|---|---|---|---|---|---|
| S4-W8 wall | gain >= 0.060 ms, TIP faster claimed; wall route needs R_8 <= 0.060 | S4-JT PARENT (A) vs TIP (B), W8, [0,500); n 9/9 (3+3+3) | A 1.9859 [1.9780-2.0244] IQR 0.0071 SE 0.0069; B 1.8154 [1.7937-1.8472] IQR 0.0230 SE 0.0070; B-A -0.1705 ms (-8.58 %); pass B-A -0.1550 / -0.1741 / -0.1810; bars pooled i 2.64 s 1.04 r 7.52 % | pooled Y/Y/Y; p0 Y/Y/Y p1 Y/Y/Y p2 Y/Y/Y | CLAIMED STRONG |
| S4-W8 span | span gain >= 0.060 ms claimed, with the 30/60 us zone canary seen | S4-JT-a PARENT vs TIP phys_solve_build, W8, [0,500); n 9/9 | A 0.3038 [0.2999-0.3119] IQR 0.0014 SE 0.0014; B 0.1144 [0.1137-0.1155] IQR 0.0006 SE 0.0002; B-A -0.1894 ms (-62.34 %); pass B-A -0.1900 / -0.1885 / -0.1891; bars pooled i 1.35 s 0.97 r 8.55 % | pooled Y/Y/Y; p0 Y/Y/Y p1 Y/Y/Y p2 Y/Y/Y | CLAIMED STRONG |
| R_8 rung | rise of the 0.060 ms rung claimed = R_8 <= 0.060 | S4-JT#tip (A) vs S4-rung#tip (B), W8, [0,500); n 9/9 | A 1.8154 [1.7937-1.8472] IQR 0.0230 SE 0.0070; B 1.8629 [1.8588-1.8876] IQR 0.0138 SE 0.0042; B-A +0.0475 ms (+2.62 %); pass B-A +0.0330 / +0.0622 / +0.0341; bars pooled i 2.94 s 0.90 r 6.65 % | pooled n/n/Y; p0 n/n/n p1 Y/Y/Y p2 n/Y/Y | NOT CLAIMED |
| zone N30000 | span rise claimed (canary seen) | S4-JT-a#tip vs S4-zone-N30000, W8, phys_solve_build, [0,500); n 9/9 | A 0.1144 [0.1137-0.1155] IQR 0.0006 SE 0.0002; B 0.1447 [0.1439-0.1473] IQR 0.0016 SE 0.0005; B-A +0.0303 ms (+26.48 %); pass B-A +0.0305 / +0.0306 / +0.0314; bars pooled i 2.38 s 0.78 r 5.67 % | pooled Y/Y/Y; p0 Y/Y/Y p1 Y/Y/Y p2 Y/Y/Y | CLAIMED STRONG |
| zone N60000 | span rise claimed (canary seen) | S4-JT-a#tip vs S4-zone-N60000, W8, phys_solve_build, [0,500); n 9/9 | A 0.1144 [0.1137-0.1155] IQR 0.0006 SE 0.0002; B 0.1749 [0.1733-0.1759] IQR 0.0015 SE 0.0004; B-A +0.0604 ms (+52.83 %); pass B-A +0.0612 / +0.0594 / +0.0608; bars pooled i 2.01 s 0.59 r 4.33 % | pooled Y/Y/Y; p0 Y/Y/Y p1 Y/Y/Y p2 Y/Y/Y | CLAIMED STRONG |
| no-slower W1 | TIP not claimed slower (W1: any W1 difference is layout) | S4-JT PARENT vs TIP, W1, [0,500); n 9/9 | A 4.4217 [4.3833-4.5336] IQR 0.0920 SE 0.0235; B 4.4500 [4.3918-4.5231] IQR 0.0349 SE 0.0166; B-A +0.0283 ms (+0.64 %); pass B-A +0.0159 / +0.0664 / -0.0413; bars pooled i 4.45 s 1.30 r 9.00 % | pooled n/n/n; p0 n/n/n p1 n/n/n p2 n/n/n | not slower (no resolved difference) |
| no-slower W2 | TIP not claimed slower | S4-JT PARENT vs TIP, W2, [0,500); n 9/9 | A 3.0807 [3.0534-3.2259] IQR 0.0369 SE 0.0234; B 2.9998 [2.9650-3.0208] IQR 0.0130 SE 0.0075; B-A -0.0808 ms (-2.62 %); pass B-A -0.0826 / -0.0808 / -0.1663; bars pooled i 2.55 s 1.60 r 11.80 % | pooled n/Y/Y; p0 n/Y/Y p1 n/Y/Y p2 n/Y/Y | not slower (TIP faster CLAIMED) |
| no-slower W4 | TIP not claimed slower | S4-JT PARENT vs TIP, W4, [0,500); n 9/9 | A 2.3529 [2.3376-2.4170] IQR 0.0234 SE 0.0111; B 2.1981 [2.1857-2.2100] IQR 0.0159 SE 0.0039; B-A -0.1549 ms (-6.58 %); pass B-A -0.1399 / -0.1558 / -0.1783; bars pooled i 2.46 s 1.01 r 7.10 % | pooled n/Y/Y; p0 Y/Y/Y p1 Y/Y/Y p2 Y/Y/Y | not slower (TIP faster CLAIMED) |
| no-slower W16 | TIP not claimed slower | S4-JT PARENT vs TIP, W16, [0,500); n 9/9 | A 2.2203 [2.2104-2.4401] IQR 0.0332 SE 0.0305; B 2.0502 [2.0357-2.0744] IQR 0.0154 SE 0.0049; B-A -0.1700 ms (-7.66 %); pass B-A -0.1696 / -0.1822 / -0.2082; bars pooled i 3.34 s 2.79 r 21.03 % | pooled n/Y/Y; p0 Y/Y/Y p1 Y/Y/Y p2 n/n/Y | not slower (no resolved difference) |
| W16 wall gain (no bar) | none pre-registered at W16 (design: -) | S4-JT PARENT vs TIP, W16, [0,500); n 9/9 | A 2.2203 [2.2104-2.4401] IQR 0.0332 SE 0.0305; B 2.0502 [2.0357-2.0744] IQR 0.0154 SE 0.0049; B-A -0.1700 ms (-7.66 %); pass B-A -0.1696 / -0.1822 / -0.2082; bars pooled i 3.34 s 2.79 r 21.03 % | pooled n/Y/Y; p0 Y/Y/Y p1 Y/Y/Y p2 n/n/Y | NOT CLAIMED |
| W16 armed wall (no bar) | none | S4-JT-a PARENT vs TIP, W16, [0,500); n 9/9 | A 2.3339 [2.3160-2.5028] IQR 0.0205 SE 0.0278; B 2.1529 [2.1367-2.1889] IQR 0.0132 SE 0.0066; B-A -0.1810 ms (-7.75 %); pass B-A -0.1776 / -0.1937 / -0.1721; bars pooled i 2.14 s 2.46 r 16.73 % | pooled n/Y/Y; p0 Y/Y/Y p1 n/Y/Y p2 n/n/Y | NOT CLAIMED |
| W16 span (no bar) | none | S4-JT-a PARENT vs TIP phys_solve_build, W16, [0,500); n 9/9 | A 0.3110 [0.3067-0.4428] IQR 0.0130 SE 0.0229; B 0.1161 [0.1157-0.1196] IQR 0.0011 SE 0.0005; B-A -0.1949 ms (-62.67 %); pass B-A -0.1929 / -0.1972 / -0.1927; bars pooled i 8.53 s 14.75 r 87.79 % | pooled n/Y/Y; p0 Y/Y/Y p1 n/Y/Y p2 n/Y/Y | CLAIMED |
| W8 armed wall | beside | S4-JT-a PARENT vs TIP, W8, [0,500); n 9/9 | A 2.0806 [2.0746-2.1475] IQR 0.0103 SE 0.0095; B 1.9025 [1.8925-1.9318] IQR 0.0171 SE 0.0058; B-A -0.1781 ms (-8.56 %); pass B-A -0.1841 / -0.1929 / -0.1747; bars pooled i 2.05 s 1.09 r 8.14 % | pooled Y/Y/Y; p0 Y/Y/Y p1 Y/Y/Y p2 Y/Y/Y | CLAIMED STRONG |

[100,500), beside (same rule):

- S4-W8 wall: B-A -0.1691 ms (-8.71 %) pooled Y/Y/Y; p0 Y/Y/Y p1 Y/Y/Y p2 Y/Y/Y -> CLAIMED STRONG
- S4-W8 span: B-A -0.1865 ms (-62.34 %) pooled Y/Y/Y; p0 Y/Y/Y p1 Y/Y/Y p2 Y/Y/Y -> CLAIMED STRONG
- rung: B-A +0.0468 ms (+2.64 %) pooled n/n/Y; p0 n/n/n p1 Y/Y/Y p2 n/Y/Y -> NOT CLAIMED
- W1: B-A +0.0165 ms (+0.39 %) pooled n/n/n; p0 n/n/n p1 n/n/n p2 n/n/n -> NOT CLAIMED
- W2: B-A -0.0837 ms (-2.83 %) pooled n/Y/Y; p0 n/n/Y p1 n/Y/Y p2 n/Y/Y -> NOT CLAIMED
- W4: B-A -0.1547 ms (-6.78 %) pooled n/Y/Y; p0 Y/Y/Y p1 Y/Y/Y p2 Y/Y/Y -> CLAIMED
- W16: B-A -0.1707 ms (-7.82 %) pooled n/Y/Y; p0 Y/Y/Y p1 Y/Y/Y p2 n/n/Y -> NOT CLAIMED
- zone30: B-A +0.0304 ms (+26.94 %) pooled Y/Y/Y; p0 Y/Y/Y p1 Y/Y/Y p2 Y/Y/Y -> CLAIMED STRONG
- zone60: B-A +0.0604 ms (+53.59 %) pooled Y/Y/Y; p0 Y/Y/Y p1 Y/Y/Y p2 Y/Y/Y -> CLAIMED STRONG

**Reading the S4 claim by its pre-registered letter** (cut §5, design §7 item 3, ruling 2; bar 0.060 ms at W8, J-T):
1. **The gain at W8 is 0.1705 ms on the wall and 0.1894 ms on the span.** Both are >= 0.060, and both still are
   after subtracting their own pooled i-bar: 0.1181 on the wall, 0.1853 on the span.
2. **The wall route needs R_8 <= 0.060 from the block's own rung, and it failed.**
   - The rise is +0.0475 ms over pooled K = 9; the pooled flags are n/n/Y and pass 0 reads n/n/n.
   - The claim therefore goes to the span route: the `phys_solve_build` gain is CLAIMED STRONG, and both zone
     canaries are seen STRONG (span rises +0.0303 and +0.0604 ms for 0.030 and 0.060 injected).
   - The span resolution is therefore <= 0.030 ms. **S4-W8: CLAIMED.**
3. **The "both min-max and SE" wording of the cut (the P1 rule) holds as well.** Wall and span are STRONG in every
   pass and pooled.
4. **No row is claimed slower at W 1/2/4/16.**
   - W1: +0.64 % (TIP slower as a point estimate), not resolved. The pooled bars are i 4.45 % and s 1.30 %, so a W1
     slowdown under ~4.45 % (0.197 ms) could not have been claimed.
   - W2 and W4: TIP faster, CLAIMED.
   - W16: TIP faster by 7.66 % as a point estimate, NOT CLAIMED (pass 2 n/n/Y).
5. **Poses are equal.** All 153 S4-AB processes and all 54 SPLIT processes read `0x30c5438bc6ad9ffa`, with the
   expectation matched.

## 2. Implied decisions
- **S4: KEEP** (already on the trunk, `16191fda`). Its window claim holds: the gain is >= 0.060 ms at W8 on J-T, no
  row is slower, and the poses are equal. The design's O2 fallback ("if window 8b reads S4 under its bar, drop the
  `plan`/`tags` writes") is not triggered.
- **The realized gain.**
  - Wall: 0.1705 ms ([100,500): 0.1691). That exceeds the design's 0.10-0.15 and falls 0.0195 ms below window 8's
    arithmetic 0.19-0.22.
  - Span: 0.1894 ms. P-c alone goes 0.2615 -> 0.0706 ms, a 3.70x speedup worth 0.1909 ms.
  - The wall realises 0.90 of the span gain.
  - Per manifold, W8, [100,500): TIP 397.0 vs PARENT 434.9 ns/manifold, on 4,467.665 manifolds in every process.
- **W16: no pre-registered bar** (the design's S4 row reads "—" at W16).
  - The wall gain is 0.1700 ms (-7.66 %): NOT CLAIMED.
  - The armed wall gain is 0.1810 ms: NOT CLAIMED.
  - The span gain is 0.1949 ms: CLAIMED, but not STRONG (the PARENT min-max is 43.8 %).
  - The failures come from PARENT's slow processes (§4), not from TIP.
- **R_8 remains undemonstrated inside S4-AB.** The pre-registered S4 claim does not need it, because it has the span
  route. Post hoc, the S7-AB ladder on the same TIP file reads R_8 = 0.060 and R_16 = 0.0525 ms (§5.2).
- **For the S1 decision (ruling 5): B1 and B2 are now measured** (§3). The first-wave recruitment is ~0.004 ms per
  step. That is the bottom of the design's 0-0.07 charge-back bracket, and it leaves nothing for the "open the region
  before P-a" mitigation to hide.
  - The S1-addressable dispatch loss (ramp + join, less one recruitment) is <= 0.083 ms per step at W8 (arith.,
    upper bound, before S1's own ω_b and claim costs).
  - 72 % of the tail and ~51 % of L_wide are not dispatch, and S1 cannot remove them.
  - Price S1 against the ω_b v2 numbers of the omega group.

## 3. SPLIT: per-wave split on PARENT (S4 off), SPLIT-J-T-a, K = 9 per W, [100,500)
At W1 no colour or np wave dispatches (lanes < 2): every counter is 0 in all 9 processes. Values below are ns per
wave, median [min-max]; n = 9 per cell.

| field | W8 | W16 |
|---|---|---|
| first_ramp_ns_mean (B1: the step's first colour wave) | 4117 [4060-4170] | 4195 [4152-4248] |
| first_tail_ns_mean | 1039 [781-1188] | 1713 [1597-1809] |
| pass_ramp_ns_mean (each pass's first wave; 12 per step) | 2960 [2933-2978] | 2872 [2834-2904] |
| other waves' ramp (derived: (ramp - pass ramp) / (waves - 12)) | 302 [298-313] | 353 [345-359] |
| helped_waves | 96.00 of 96.00 per step, every process | 96.00 of 96.00 |
| ramp_ns_mean_helped (= ramp_ns_mean, every wave helped) | 634 [630-643] | 666 [660-674] |
| tail_ns_mean | 992 [954-1022] | 1603 [1552-1671] |
| join_ns_mean (B2) | 274 [266-278] | 461 [454-472] |
| imbalance_ns_mean (B2: tail - join) | 718 [689-747] | 1142 [1089-1199] |
| np ramp / tail | 835 [795-930] / 10615 [10097-11767] | 978 [947-1060] / 11026 [10292-11477] |
| np_join_ns_mean | 2537 [2301-2850] | 2209 [2029-2359] |
| np_imbalance_ns_mean | 8100 [7745-8960] | 8794 [8184-9143] |
| np_route_worker | 400 of 400 np waves (1.000), every process | 400 of 400 |
| route_external; in-flight / lanes per wave | 0; 7.998 / 7.998 | 0; 14.51 / 15.36 |

[0,500) (the SUMMARY's own window):
- W8: first ramp 4134, first tail 1048, pass ramp 2962, ramp/helped 626, join 276, imbalance 707, np join 2652,
  np imbalance 11456, np_route_worker 500/500.
- W16: 4232, 1721, 2881, 663, 466, 1143, 2531, 11934, 500/500.

**D(W) with the W1 pair** (review N2; SPLIT-J-T = disarmed, SPLIT-J-T-a = armed; same block; medians, ms):
- [100,500): T_d W1/8/16 = 4.1755 / 1.9480 / 2.1850; T_a = 4.2179 / 2.0377 / 2.2879.
  - D(8) = [+0.0897] - [+0.0424] - dr 0.0272 - du 0.0002 = **+0.0200 ms** (0.208 us/wave). omega(8) = L_wide / waves
    = 0.3195 ms / 96 = 3.328 us; after D, 3.120 us.
  - D(16) = [+0.1029] - [+0.0424] - dr 0.0374 - du 0.0005 = **+0.0226 ms** (0.235 us/wave). omega(16) = 6.776 us;
    after D, 6.541 us.
- [0,500): D(8) = +0.0134, D(16) = +0.0134 ms.
- r and u, recomputed from the spans, equal the runner's `r_ns` / `u_ns` columns (W8: 0.02990 / 0.01099 ms).

**What the split says** (arith. on the medians above, `q_derive.txt`; no claim, because these are decompositions,
not A/B pairs):
- **B1, the first-wave ramp.** It is 4.12 us per step at W8 (4.20 at W16): 6.5x a helped wave's mean ramp.
  - That is S1's once-per-step recruitment charge-back, ~0.004 ms, at the bottom of the design's 0-0.07 ms bracket.
  - The design's mitigation, opening the region before P-a (up to 0.08-0.16 ms, arith.), has ~4 us to hide.
    **Not worth building.**
- **The pass-first ramps: the park-and-wake cascade after the serial gaps, confirmed.**
  - The 12 pass-first waves average 2.96 us, 9.8x the 0.30 us of every other wave.
  - They carry 58 % of the step's ramp: 35.5 of 60.8 us at W8.
  - The 11 after the first cost 31.4 us per step. They follow the serial warm-apply and integrate gaps in which
    helpers park.
  - **Levers: S1**, whose helpers spin through the region, **and S2/S3**, which make those gaps parallel stages.
- **B2, the tail.** It splits as imbalance 718 + join 274 ns per wave at W8 (72 % / 28 %), and 1142 + 461 at W16
  (71 % / 29 %).
  - The join, 26.3 us per step at W8, is dispatch: **S1** removes it.
  - The imbalance, 68.9 us per step at W8, is block grain, and S1 keeps it. It is close to the design's assumed
    ~0.7 us block-grain tail per wave.
- **L_wide at W8** is 319.5 us per step:
  - ramp 60.8 (19.0 %);
  - join 26.3 (8.2 %);
  - imbalance 68.9 (21.6 %);
  - **51.1 % (163.4 us) inside the waves**: neither ramp nor tail. It includes D(8) = 20 us of armed stamp inflation.
  - Ramp + tail per wave is 1.63 us, 48.9 % of omega(8) = 3.33 us.
- **S1's dispatch target at W8** is ramp + join less one recruitment: 83.0 us per step (arith., upper bound). The
  region's own ω_b × stages and block claims must come out of it.
- **At W16** L_wide is 650.5 us per step:
  - its W16 - W8 growth of +331 us is 81 % in-wave (+269 us), imbalance +40.7, join +17.9 and ramp +3.1;
  - the W16 loss is SMT in-wave slowdown, not dispatch, which is **S7**'s target (ruling 4), not S1's.
- **The np wave** (W8): ramp 0.84 us, tail 10.6 us = imbalance 8.1 us (76 %) + join 2.5 us. Every np wave joined on
  the worker route (N6: 400/400 at W8 and W16, every process).
  - The np imbalance is narrowphase chunk grain. None of S1, S2, S3 or S5 covers it; S1's region does not include
    the narrowphase.
- **Serial losses at W8** (armed PARENT, [100,500), ms), with their levers:
  - setup 0.2988 (P-c 0.2582) -> **S4**, realised: TIP's span is 0.1127;
  - warm apply 0.3079 -> **S2**;
  - bp 0.2655 (query 0.2078) -> **S5**;
  - graph 0.1195 -> S6 (not built);
  - integrate group 0.1218 -> **S3**;
  - narrow colours 0.0479 -> S1's narrow term;
  - np serial 0.0352.

## 4. Anomalies and validity notes
- **Re-runs.** Two originals were unclean by the 2 % witness rule; their re-runs (0.88 % and 0.23 %) are used:
  - S4-JT#parent@W8 p0 r1 (2.16 %; claude.exe / browser.exe / Discord.exe, 0.031 s each);
  - S4-JT-a#parent@W16 p1 r1 (3.08 %; Discord.exe 0.609 s).
- **W16 PARENT slow processes: what breaks the W16 claims.** Receipts were clean (witness <= 0.88 %), and there is
  no burst signature: 3.2-3.6 % of frames exceed 1.2x the median in the three, 1.4-4.6 % in the other PARENT W16 processes.
  - S4-JT#parent@W16 p2 r1: 2.4401 ms, median step 2.3947. The other eight read 2.2104-2.2669 and 2.1733-2.2151.
  - S4-JT-a#parent@W16 p1 r1 (the re-run): 2.4484, `phys_solve_build` 0.4266 ms.
  - S4-JT-a#parent@W16 p2 r0: 2.5028, span 0.4428, against 0.3067-0.3216 in the other seven. **The serial fill itself
    ran ~40 % slower in those processes.**
  - All 18 TIP W16 processes stay within 2.0357-2.0744 (disarmed) and 2.1367-2.1889 (armed).
  - This is the "pass-0-style uniformly slow process" of windows 7 and 8, here landing on PARENT only. Ruling 1 keeps
    them (receipts are never used to drop).
- **Pass drift (post hoc).** Pass 2 (17:39-17:44, the last pass) reads higher for PARENT at every W.
  - PARENT: W1 4.5070 vs 4.4217 / 4.3837; W2 3.1694 vs 3.0788 / 3.0807; W4 2.3869 vs 2.3403 / 2.3438; W8 2.0159 vs
    1.9812 / 1.9859; W16 2.2669 vs 2.2199 / 2.2288.
  - TIP drifts less: W8 1.8348 vs 1.8262 / 1.8118.
  - No verdict depends on it: every W8 claim holds in pass 2.
- **The rung is weaker than injected.** It rose +0.0475 of 0.060 ms (0.79x): +0.0330 / +0.0622 / +0.0341 by pass.
  - Its reference, S4-JT#tip@W8, has the widest IQR among the S4 W8 disarmed cells: 0.0230 ms (i 1.27 %), 3.2x
    PARENT's 0.0071.
  - In pass 0 the reference spans 1.7937-1.8472. p0 r2 reads 1.8472, with 9.4 % of frames > 1.2x the median.
  - The zone canaries in the same rounds read 1.01x.
- **The rows are narrower than the ruling's list.** RULINGS-2026-09-27-W8 "Window 8b contents" lists, for S4-AB, "an
  in-block ladder at W8/W16, the rung at W 1/2/4".
  - rows8b.json (cut §5's table) carries only S4-rung at W8 and the two zone canaries.
  - So "no row slower" at W1/2/4/16 is stated with the spread bars only, never with a rung. There is also no
    in-block R_16.
- **The window does not re-read S4-inline at W1.** S4-AB has no armed W1 row, so TIP's `setup_steps` = 0 at W1 is not
  read in the window; PREP's untimed gate is its only receipt. Any W1 TIP-PARENT difference is layout by
  construction; none is resolved.
- **The placement receipts do not describe the step's thread.** They are recorded and never used.
  - The top logical CPU is 10 in 206 of 207 used processes.
  - `main_share_top_est` is 1.0 or null (16 nulls: sub-tick main-thread CPU).
  - `main_cycle_frac` is 0.0046-0.0310, and 0.015-0.031 even at W1. The receipt's "main thread" (the process's
    creating thread) carries 0.5-3 % of the cycles; the step runs on the pool thread (`solve_on_dispatcher_steps` 0).
- **Cross-block agreement, same exe and row.**
  - SPLIT vs S4-AB PARENT: W8 disarmed 1.9909 vs 1.9859, armed 2.0852 vs 2.0806; W16 2.2239 vs 2.2203 and 2.3288
    vs 2.3339; W1 4.3986 vs 4.4217.
  - S7-AB's S7-JT#s7p@W8 (the TIP file) reads 1.8152, against S4-AB's TIP 1.8154.

## 5. Post-hoc observations (not pre-registered; no verdict rests on them)
1. **Window 8's per-wave table was in TSC ticks, not ns** (`q_posthoc.txt` §2). Window 8's `q3_waves.py` divided the
   raw CSV counters by 1e6 as if they were ns. Recomputed on `win8/raw` (W8S-A J-T-a, [100,500), K = 6; ticks_per_ns
   3.2934-3.2937):

   | quantity | window 8 as published | in ns |
   |---|---|---|
   | ramp / wave, W8 | 2.031 us | 0.617 us |
   | tail / wave, W8 | 3.322 us | 1.009 us |
   | ramp + tail, W8 | 5.353 us ("162 % of omega") | 1.625 us = 49.2 % of window 8's omega(8), 3.304 us recomputed (W16: 2.166 us = 33.0 % of 6.558 us) |
   | W16 ramp / tail | 2.155 / 4.978 us | 0.654 / 1.511 us |
   | np ramp / tail, W8 | 2.76 / 30.5 us | 0.837 / 9.257 us |
   | W16 growth "in the tail" | +1.66 us/wave x 96 ~ 0.16 ms | +0.503 us/wave x 96 = **0.048 ms** |

   Two statements in window 8's record rest on these unit-wrong numbers and need correcting where they are cited
   (analysis §3, and the tail-growth line):
   - "the ramp overlaps the caller's own work", offered to explain >100 % of omega;
   - "W16 growth mostly in the tail".

   8b's ns values agree with the corrected ones (PARENT W8: 0.634 / 0.992 us).
2. **The same TIP file resolves 0.060 ms at W8 in another block** (`q_posthoc.txt` §1). In S7-AB, whose s7p is
   `runner_tip_16191fda.exe`, ruling 1, [0,500):
   - W8 ladder: F0.5 +0.0337 NOT CLAIMED; F1 +0.0660 CLAIMED; F1.5 +0.0911 CLAIMED; F2 +0.1257 CLAIMED STRONG. So
     R_8 = 0.060 ms.
   - W16 ladder: all four rungs CLAIMED (+0.0545 / +0.1139 / +0.1599 / +0.2146; F1 and above STRONG). So
     R_16 = 0.0525 ms.

   Had this been S4-AB's own ladder, as the ruling listed, the wall route would have qualified as well.
3. **The armed W8 wall resolves 0.060 ms in-block.** S4-zone-N60000's wall rise is +0.0626 ms, CLAIMED (not
   STRONG). N30000's +0.0377 is NOT CLAIMED.
4. **S4 does not change the colour waves' ramps** (S4-AB armed, [100,500)). TIP vs PARENT at W8:
   - first ramp 4069 vs 4125 ns; pass ramp 2891 vs 2973; join 281.5 vs 283.0 ns/wave;
   - TIP has one more wave per step (97 vs 96): its setup wave.
5. **D(W) did not shrink at W8** with 1b's padded stamps.
   - 8b: +0.0200 ms at W8, +0.0226 at W16 ([100,500)).
   - Window 8, as published in its analysis §2: W8 +0.0037 (A) and +0.0222 (R); W16 +0.0379.
   - These are single differences of medians on different engine trees, with no spread; no claim.
6. **omega(8) ≈ window 8's**: 3.328 us (8b, PARENT at 16191fda) against window 8's 3.304 us, recomputed (226bd99e). The trees
   differ, so this is no claim.

## 6. Where the pre-registered rule's application was uncertain
1. **The cut's claim wording conflicts with ruling 1.** Cut §5 and design §7 say "both min-max and SE in every block
   and pooled" (the P1 rule); ruling 1, written later, says IQR AND SE, with r adding STRONG. I applied ruling 1, as
   the task directs. Every S4 W8 verdict is STRONG, so the answer is the same under both rules.
2. **The verdict window is [0,500).** It is the rows' window and window 8's convention; cut §5 names no window for the
   gain. [100,500) agrees everywhere except W2's "TIP faster" (CLAIMED on [0,500), NOT CLAIMED on [100,500), pass 0
   n/n/Y). No pre-registered claim needs W2 to be faster.
3. **"Gain >= 0.060 ms" was read as: the difference is CLAIMED and its median is >= 0.060.** A stricter one-sided
   reading (gain minus its own i-bar >= 0.060) gives the same answer: 0.118 on the wall, 0.185 on the span.
4. **Where R_8 must come from.** I took it from S4-AB's own rung, per ruling 2's "its own ladder, rung and zone
   canary". On that reading the wall route fails and the span route carries the claim. The S7-AB ladder's
   R_8 = 0.060 is post hoc.
5. **STRONG was taken as r flagging pooled AND in every pass.** The ruling says only "a claim that also passes it".
   Only the W16 span verdict depends on this (r fails pooled).
6. **The cut's "pass-0-style" handling cannot fire as written.** "Pass 0 and pass 1 medians differ by more than the
   pooled range" is impossible, because both medians lie inside the pooled [min, max]. Ruling 1's "every pass"
   requirement is stricter and subsumes it; all three passes are clean, so all three were required.
7. **No explicit KEEP/REVERT rule for S4 was found** in cut.md, the w8s-s4 notes, 01-DESIGN.md §6.4/§10.4 or either
   rulings file. I used the lane's stated window claim as the keep condition: gain >= 0.6 x the low end at W8 J-T,
   no row slower at W1/2/4/16, poses equal. It holds, so KEEP. REVERT would follow from a W row claimed slower, and
   none is.
8. **At W16 there is neither a bar nor a prediction** (the design's S4 row has "—"; window 8's 0.19-0.22 is W8). The
   W16 gain is reported without a verdict beyond "not slower".
9. **The W1 inline requirement has no in-window reading** (no armed TIP W1 row). It is taken from PREP's untimed gate.
