"""Composes analysis/s7.md from s7_results.json, s7_posthoc.json, s7_select.txt and s7_engagement.txt. Every number in
the tables is formatted from those files (computed by s7_select / s7_claims / s7_posthoc / s7_engagement)."""
import json
import os

H = os.path.dirname(os.path.abspath(__file__))
R = json.load(open(os.path.join(H, 's7_results.json'), encoding='utf-8'))
X = json.load(open(os.path.join(H, 's7_posthoc.json'), encoding='utf-8'))
MD = []


def w(s=''):
    MD.append(s)


def yn(c):
    return ('Y' if c['cl_r'] else 'n') + '/' + ('Y' if c['cl_i'] else 'n') + '/' + ('Y' if c['cl_s'] else 'n')


def cellf(c):
    return (f"{c['median']:.4f} [{c['min']:.4f}-{c['max']:.4f}], IQR {c['iqr_abs']:.4f} ({100 * c['i']:.2f} %), "
            f"SE {c['se_abs']:.4f} ({100 * c['s']:.2f} %)")


def flags(j):
    return (f"pooled {yn(j['pooled'])} (bars r {100 * j['pooled']['bar_r']:.2f} / i {100 * j['pooled']['bar_i']:.2f} / "
            f"s {100 * j['pooled']['bar_s']:.2f} %); p0 {yn(j['per']['0'])}, p1 {yn(j['per']['1'])}, p2 {yn(j['per']['2'])}")


def diff(j):
    p = j['pooled']
    return (f"B/A {p['ratio']:.4f} ({100 * (p['ratio'] - 1):+.2f} %), B-A {p['delta']:+.4f} ms; per pass "
            f"{j['per']['0']['ratio']:.4f} / {j['per']['1']['ratio']:.4f} / {j['per']['2']['ratio']:.4f}")


def st(j):
    return ('CLAIMED' + (' STRONG' if j['strong_all'] else '')) if j['claimed'] else 'not claimed'


C = R['claims']['0..500']
L = R['ladders']['0..500']
R8, R16 = L['ladder8']['R_ms'], L['ladder16']['R_ms']
c3 = C['C3-JT']
VERD = {
    'C1-JT': 'FAILS (REFUTED: T(16) slower CLAIMED, STRONG)' if C['C1-JT']['judge']['claimed'] and C['C1-JT']['judge']['pooled']['ratio'] > 1 else 'HOLDS',
    'C1-JA': 'FAILS (REFUTED: T(16) slower CLAIMED, STRONG)' if C['C1-JA']['judge']['claimed'] and C['C1-JA']['judge']['pooled']['ratio'] > 1 else 'HOLDS',
    'C2-JT': 'HOLDS (T(8) slower NOT CLAIMED)' if not (C['C2-JT']['judge']['claimed'] and C['C2-JT']['judge']['pooled']['ratio'] > 1) else 'FAILS',
    'C2-JA': 'HOLDS (T(8) slower NOT CLAIMED)' if not (C['C2-JA']['judge']['claimed'] and C['C2-JA']['judge']['pooled']['ratio'] > 1) else 'FAILS',
    'C3-JT': c3['status'].split(' (')[0] + (' (T(16) SLOWER than P(16), CLAIMED STRONG)' if c3['status'].startswith('REFUTED') else ''),
}
first = '; '.join(f'{k}={v.split(" (")[0]}' for k, v in VERD.items())
w(f'VERDICTS: {first}; R_8={R8:.4f} ms; R_16={R16:.4f} ms; Outcome branch = "C1 fails without C3" -> S7 does NOT merge')
w()
w('# Window 8b - block S7-AB: C1, C2, C3 (results-analyst, 2026-09-29)')
w()
g = c3['gain_ms']
w(f"The cap engages as designed (pre-flight receipt PASS) and leaves W8 unchanged (C2 holds on J-T and J-A), but it makes "
  f"W16 slower, not faster: P(16) - T(16) = {g:+.4f} ms on J-T against a predicted +0.166..+0.175 ms, CLAIMED STRONG in "
  f"the opposite direction in every pass and in all three windows. T(16) is CLAIMED slower than T(8) on both rows "
  f"(J-T {100 * (C['C1-JT']['judge']['pooled']['ratio'] - 1):+.2f} %, J-A {100 * (C['C1-JA']['judge']['pooled']['ratio'] - 1):+.2f} %), "
  f"so C1 fails on both. The resolution is not the limit: R_16 = {R16:.4f} ms and R_8 = {R8:.4f} ms were demonstrated "
  f"in the same block. By the cut's Outcome paragraph, read literally, this is \"C1 fails without C3\": report, do not merge.")
w()
w('## Method')
w('Recomputed from `raw/runs.jsonl` and every process\'s `run.csv` / `stdout.txt` (parsers: `tools/lib/driver.py` '
  '`load_csv`, `parse_summary`; engagement: `tools/s7pre8b.py` `lanes_bound`, `engagement_why`); nothing is taken from '
  'the driver\'s statistics (its `mean_ms` is cross-checked, 171/171 equal). Metric: the process mean of `wall_ns` over '
  '[0,500), with [0,100) and [100,500) beside. **Cell = median over K of the process means**, ms per step. '
  'Spreads relative to the median, as window 8 (`win8/analysis/lib8.py`): r = (max-min), i = IQR (inclusive quartiles), '
  's = 1.2533 SD / sqrt(K). B against A: a flag x is set iff |B/A - 1| > 2 hypot(A_x, B_x). **Ruling 1:** CLAIMED iff '
  'i AND s are set pooled (K = 9) AND in each of the three passes (K = 3 per cell), with one sign; STRONG iff r is also '
  'set pooled and in every pass. A rung is SEEN iff its rise over the plain P row at the same W is CLAIMED (rung slower); '
  'R_W = the smallest rung seen with every larger rung seen. "Binding threshold" = the largest of the i and s bars over '
  'the pooled and per-pass comparisons, i.e. the smallest effect this rule could have claimed on that pair. Slot rule '
  '(window 7/8): original if valid and clean, else its clean re-run. P = s7p (`runner_tip_16191fda.exe`, no S7), '
  'T = s7t (`runner_s7_tip_a3adc827.exe`, S7 partial). n = 9 in every cell (3 per pass); 171 processes used, 0 dropped.')
w()
w('## 1. Verdict table, window [0,500) (A = the first cell, B = the second; flags r/i/s)')
w()
w('| claim | pre-registered bar | cells (n) | A: median [min-max], IQR, SE (ms) | B: median [min-max], IQR, SE (ms) | '
  'difference | flags r/i/s | verdict |')
w('|---|---|---|---|---|---|---|---|')
BAR = {
    'C1-JT': 'T(16) not claimed slower than T(8); with R_16, R_8',
    'C1-JA': 'T(16) not claimed slower than T(8); with R_16, R_8',
    'C2-JT': 'T(8) not claimed slower than P(8); with R_8',
    'C2-JA': 'T(8) not claimed slower than P(8); with R_8',
    'C3-JT': 'P(16) - T(16), predicted +0.166..+0.175 ms, design bar 0.105 ms; claimed or reported with resolution',
}
CELLS = {
    'C1-JT': 'A T(8) S7-JT#s7t@W8 (9); B T(16) S7-JT#s7t@W16 (9)',
    'C1-JA': 'A T(8) S7-JA#s7t@W8 (9); B T(16) S7-JA#s7t@W16 (9)',
    'C2-JT': 'A P(8) S7-JT#s7p@W8 (9); B T(8) S7-JT#s7t@W8 (9)',
    'C2-JA': 'A P(8) S7-JA#s7p@W8 (9); B T(8) S7-JA#s7t@W8 (9)',
    'C3-JT': 'A P(16) S7-JT#s7p@W16 (9); B T(16) S7-JT#s7t@W16 (9)',
}
for cid in ('C1-JT', 'C1-JA', 'C2-JT', 'C2-JA', 'C3-JT'):
    j = C[cid]['judge']
    extra = ''
    if cid.startswith('C3'):
        extra = f"; P(16)-T(16) = {C[cid]['gain_ms']:+.4f} ms; bar 0.105 not met (sign reversed); R_16 {R16:.4f} ms"
    elif cid.startswith('C1'):
        extra = f"; R_16 {R16:.4f}, R_8 {R8:.4f} ms"
    else:
        extra = f"; R_8 {R8:.4f} ms"
    w(f"| {cid} | {BAR[cid]} | {CELLS[cid]} | {cellf(j['A_cell'])} | {cellf(j['B_cell'])} | {diff(j)}; binding "
      f"threshold {100 * j['rule_threshold_rel']:.2f} % = {j['rule_threshold_ms']:.4f} ms{extra} | {flags(j)} | "
      f"**{VERD[cid]}** |")
for name, lab in (('ladder8', 'R_8 (J-T ladder, P, W8, rungs 0.030/0.060/0.090/0.120 ms)'),
                  ('ladder16', 'R_16 (J-T ladder, P, W16, rungs 0.0525/0.105/0.1575/0.210 ms)'),
                  ('JA-ladder16', 'J-A rung, P, W16, 0.105 ms (recommended row)')):
    lad = L[name]
    parts = []
    for f, r in lad['rungs'].items():
        j = r['judge']
        parts.append(f"F{f}: rise {r['rise_ms']:+.4f} ms (inj {r['injected_ms']:.4f}; x{r['rise_ms'] / r['injected_ms']:.2f}) "
                     f"pooled {yn(j['pooled'])}, p0 {yn(j['per']['0'])}, p1 {yn(j['per']['1'])}, p2 {yn(j['per']['2'])} -> "
                     f"{'SEEN' if r['seen'] else 'not seen'}")
    ref = lad['rungs'][list(lad['rungs'])[0]]['judge']['A_cell']
    w(f"| {lab} | smallest rung seen with every larger rung seen | ref (9) + each rung (9) | ref {cellf(ref)} | - | "
      f"{'; '.join(parts)} | - | **{'R = %.4f ms' % lad['R_ms'] if lad['R_ms'] else 'NOT DEMONSTRATED'}** |")
w()
w('## 1b. The same claims in [0,100) and [100,500) (reported beside; the pre-registered metric is [0,500))')
w()
w('| claim | [0,500) | [0,100) | [100,500) |')
w('|---|---|---|---|')
for cid in ('C1-JT', 'C1-JA', 'C2-JT', 'C2-JA', 'C3-JT'):
    row = [cid]
    for win in ('0..500', '0..100', '100..500'):
        j = R['claims'][win][cid]['judge']
        p = j['pooled']
        row.append(f"A {j['A_cell']['median']:.4f}, B {j['B_cell']['median']:.4f}, B/A {p['ratio']:.4f} "
                   f"({p['delta']:+.4f} ms), pooled {yn(p)}, passes {yn(j['per']['0'])} {yn(j['per']['1'])} "
                   f"{yn(j['per']['2'])}: {st(j)}")
    w('| ' + ' | '.join(row) + ' |')
w()
w('Ladders by window: ' + '; '.join(
    f"[{win}) R_8 {('%.4f' % R['ladders'][win]['ladder8']['R_ms']) if R['ladders'][win]['ladder8']['R_ms'] else 'not demonstrated'}, "
    f"R_16 {('%.4f' % R['ladders'][win]['ladder16']['R_ms']) if R['ladders'][win]['ladder16']['R_ms'] else 'not demonstrated'}, "
    f"J-A rung {'seen' if R['ladders'][win]['JA-ladder16']['rungs']['1']['seen'] else 'not seen'}"
    for win in ('0..500', '0..100', '100..500')) + ' (ms).')
w()
w('## 2. Implied decisions (cut.md §5 "Outcome", applied literally)')
w()
w('- "C1 and C2 on both rows -> S7 merges": **not met** - C2 holds on J-T and J-A, C1 fails on J-T and on J-A.')
w('- "C2 fails -> the W <= cores path moved": **not triggered** (C2 holds on both rows; T(8)/P(8) '
  f"{C['C2-JT']['judge']['pooled']['ratio']:.4f} J-T, {C['C2-JA']['judge']['pooled']['ratio']:.4f} J-A, n/n/n pooled).")
w('- "C1 fails with C3 claimed -> SMT co-scheduling or the uncapped np/setup waves -> PC-S7-3/4": **not triggered** - C3 '
  'is not claimed; its opposite is (T(16) slower than P(16)).')
w('- **"C1 fails without C3 -> the participant cap does not reach the W16 loss; report, do not merge on C1": this branch '
  'applies. Decision implied: S7 (u/phys-s7 commit (2), a3adc827) does NOT merge.**')
w(f"- Beyond the letter: the branch's wording (\"does not reach\") presumes a null; the data are a STRONG regression at "
  f"W16: P(16) - T(16) = {g:+.4f} ms per step on J-T and {-X['ratios']['0..500']['T16/P16 J-A']['delta']:+.4f} ms on "
  f"J-A (post hoc, no pre-registered claim). The design's own gate (01-DESIGN.md §6.9, \"no row may "
  f"be claimed slower at W 1/2/4/16\") would also fail on both rows. Because `LaneCap::PhysicalCores` is the default and "
  f"engages wherever W > physical cores, merging would ship that regression to every such host.")
w()
w('## 3. Anomalies and validity')
w()
SEL = open(os.path.join(H, 's7_select.txt'), encoding='utf-8').read().splitlines()
ENG = open(os.path.join(H, 's7_engagement.txt'), encoding='utf-8').read().splitlines()
w('- **Voids: none fired.** The block\'s void rules (cut.md §5: pose mismatch, failed engagement receipt, '
  'placement-receipt-free process) - from `s7_select.txt` / `s7_engagement.txt`:')
for s in SEL:
    if s.startswith(('validity problems', "driver 'invalid'", 'processes without a placement', 'processes with main_share',
                     'J-T:', 'J-A:', 'per-process', 'slots', 'cells', 'K per')):
        w(f'  - {s}')
for s in ENG:
    if 'W16' in s and ('lanes sum' in s or 'structural' in s):
        w(f'  - {s.strip()}')
import re  # noqa: E402
WL = open(os.path.join(os.path.dirname(os.path.dirname(H)), 'raw', 'window_log.txt'), encoding='utf-8').read()
idle = re.findall(r'T(\d\d:\d\d:\d\d)\.\d+\+03:00 S7-AB-p(\d): idle reached', WL)
opn = re.findall(r'S7-AB-p(\d): opening receipt ([\d.]+)%', WL)
vpass = len(re.findall(r'S7-AB-p\d: PASS VOIDED', WL))
w(f"- Passes: {len(idle)} of 3 reached the idle rule and completed; passes voided: {vpass}; idle reached at "
  f"{', '.join(f'p{q} {t}' for t, q in idle)}; opening receipts {', '.join(f'p{q} {v} %' for q, v in opn)} "
  f"(raw/window_log.txt). Every pass counts as clean for ruling 1.")
w('- Hot originals re-run once at the end of their pass (the standing rule), the re-runs used by the slot rule:')
for s in SEL:
    if s.strip().startswith(('p1 r0 seq', 'p2 r0 seq')):
        w(f'  - {s.strip()}')
w('  Post hoc sensitivity (the hot originals used instead, `s7_posthoc.txt`): C2-JA stays not claimed, C1-JA stays '
  'CLAIMED STRONG, the W1 pair stays not claimed - no verdict depends on the choice.')
w('- Placement receipts: recorded for all 171 processes and used for nothing. The main thread is the parked dispatcher '
  '(main-thread cycle fraction 0.002-0.016 by cell), so `main_share_top_est` says little at W >= 8; the top logical CPU '
  'is the busiest worker. 11 processes lie > 1.5 % from their cell median (listed in `s7_posthoc.txt` with receipts, '
  'largest +3.16 %, all slow-side, none with a receipt or witness above the gates); every claim above holds in each '
  'pass separately, so no single process decides a verdict.')
w('- Per manifold: the per-step manifold sequence is identical in all 171 processes (J-T and J-A; pose '
  '0x30c5438bc6ad9ffa everywhere), so per-manifold ratios equal the wall ratios; ns/manifold [100,500) in `s7_posthoc.txt`.')
w('- Ladder calibration: every seen rung rises 1.01-1.10x its injection on [0,500) (`s7_claims.txt`), i.e. the canary '
  'adds ~1:1 to the wall as the design assumes.')
w('- [0,100) is noisier: R_8 is not demonstrated there (the F2 rung fails pass 1, whose cell has a 7.58 % range); all five claim verdicts are '
  'the same in the three windows.')
w()
w('## 4. Post hoc observations (none pre-registered; ruling-1 flags are descriptive)')
w()
w('| reading | window | A | B | B/A (B-A ms) | pooled r/i/s; passes | reading under ruling 1 |')
w('|---|---|---|---|---|---|---|')
for tag in X['ratios']['0..500']:
    for win in ('0..500', '0..100', '100..500'):
        x = X['ratios'][win][tag]
        w(f"| {tag} | [{win}) | {x['A']:.4f} | {x['B']:.4f} | {x['ratio']:.4f} ({x['delta']:+.4f}) | {x['flags']}; "
          f"{x['per_flags']['0']} {x['per_flags']['1']} {x['per_flags']['2']} | {x['status']} ({x['direction']}; "
          f"threshold {100 * x['thr_rel']:.2f} %) |")
w()
PH = open(os.path.join(H, 's7_posthoc.txt'), encoding='utf-8').read().splitlines()
w('- Paired per round (P and T launched adjacently; the launch order flips between passes, p0/p2 T first, p1 P first):')
for s in PH:
    if s.startswith('  S7-') and 'T/P median' in s:
        w(f'  - {s.strip()}')
w('- Scaling from the W1 pair row, [0,500):')
for s in PH:
    if s.strip().startswith(('s7p: T(1)', 's7t: T(1)')):
        w(f'  - {s.strip()}')
w('- Size per colour wave (arithmetic on the window deltas and the pre-flight colour-scope count):')
for s in ENG:
    if 'per colour wave' in s and not s.startswith('#'):
        w(f'  - {s.strip()}')
w('- Reading (post hoc, not a finding of this block): the participant cap removes W16\'s extra lanes (pre-flight armed '
  'lanes_mean ~12 -> ~8, `s7_engagement.txt`) and the wall rises instead of falling, and more on J-A (scalar colours, '
  '~30 tasks per wave) than on J-T (~23). Plausible causes, none measured here: the 8 capped participants are not '
  '8 distinct physical cores under P-none (no affinity - the cap bounds threads, not cores); the CAS claim loop and '
  'the caller-runs-inline shape change the per-wave ramp/tail; the other 8 pool workers still spin in backoff on the '
  'SMT siblings. The armed pre-flight rows (n = 1 each, taken under other lanes\' load, T(8) armed slower than P(8) by '
  'more than any window cell shows) cannot separate these. PC-S7-3\'s core-id sample is the instrument that can.')
w('- P alone: P(16) is CLAIMED slower than P(8) on J-T (STRONG), and not claimed on J-A (P(16) faster in every pass, '
  'pass 0 unresolved). S7 turns J-A\'s W16 from "not slower than W8" into CLAIMED STRONG slower.')
w('- The optional W1 pair (Q10): T(1) vs P(1) not claimed in any window; the per-step decision costs nothing this block '
  'resolves (binding threshold in the table).')
w()
w('## 5. Where the pre-registered rule\'s application needed a reading')
w()
w('1. **"every clean block"** is read as every clean PASS (the task\'s wording; K = 3 per pass-cell). With K = 3 the IQR '
  '(inclusive quartiles) is half the range, so a pass-level i test is a range test at half weight.')
w('2. **Sign.** A claim also requires the pooled and per-pass ratios to share one sign (implicit in "a difference is '
  'claimed"). It changes no verdict here (every claimed pair has one sign in all passes).')
w('3. **STRONG** is read as r set pooled AND in every pass; the pooled-only reading is also computed '
  '(`s7_claims.txt`) and gives the same STRONG labels for C1-JT, C1-JA, C3.')
w('4. **C1/C2 are "not claimed slower" claims.** Their verdict is HOLDS when the slower direction is not claimed (no '
  'resolved difference, or the faster direction claimed) and FAILS/REFUTED when it is claimed. "Reported with R" is '
  'read as a report, not a condition: the Outcome paragraph does not condition on R. R_8 and R_16 are J-T ladders; J-A '
  'has no W8 ladder, and its one W16 rung (0.105 ms) is NOT seen (pass 0 fails), so C2-JA\'s "not claimed slower" '
  'carries only the rule\'s own binding threshold on that pair (see table), not a demonstrated J-A resolution. '
  'Likewise the C2-JT pair threshold (the smallest slowdown this rule could have claimed on that pair) '
  'is slightly above R_8; both are in the table. The measured T(8) - P(8) is -0.0040 ms on both rows.')
w('5. **C3 "against the design bar".** Read with the design\'s gate wording (01-DESIGN.md §6.9: the realised gain must be '
  '>= the bar AND claimed): C3 would need P(16) - T(16) >= 0.105 ms and T faster CLAIMED. The measured difference has the '
  'opposite sign and is CLAIMED STRONG, so C3 is REFUTED under any reading of the bar (point, or bar-shifted test).')
w('6. **The Outcome paragraph did not foresee C3 refuted.** Its branches are "C1 fails with C3 claimed" and "C1 fails '
  'without C3". C3 is not claimed, so the second applies literally ("report, do not merge on C1"); its gloss "the cap '
  'does not reach the W16 loss" understates what was measured (the cap adds a W16 loss).')
w('7. **P is 16191fda, not the cut\'s 54a7714f.** PREP.md records `git diff --stat 54a7714f 16191fda` = CLAUDE.md only '
  '(no crates/ file), so the P exe is the cut\'s P by content; not re-verified here.')
w('8. **Ladder reference = the plain P row at the same W in the same block** (window 8\'s q4 convention). The ladders '
  'resolve P\'s cells; T\'s W16 cell has a spread of the same order (table), so R_16 is taken to transfer to C1/C3.')
w()
w('## Files')
w()
w('Scripts and outputs in `analysis/s7/`: `s7lib.py` (library), `s7_select.py` -> `s7_select.txt`, `s7_procs.json`; '
  '`s7_claims.py` -> `s7_claims.txt`, `s7_results.json`; `s7_posthoc.py` -> `s7_posthoc.txt`, `s7_posthoc.json`; '
  '`s7_engagement.py` -> `s7_engagement.txt`; `s7_report.py` -> this file. Run order: select, claims, posthoc, '
  'engagement, report.')
open(os.path.join(os.path.dirname(H), 's7.md'), 'w', encoding='utf-8').write('\n'.join(MD) + '\n')
print('\n'.join(MD[:3]))
