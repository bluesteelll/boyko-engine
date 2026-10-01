"""C4-CGU, POST HOC only (nothing here decides the pre-registered claim):
 1. paired within-round ratios cgu1/cgu16 (same pass, round, W), all used pairs and adjacent-originals only, sign counts
 2. order position: cgu1/cgu16 when cgu1 ran first (p0, p2) vs second (p1)
 3. re-run bias: used re-runs against the originals of their cell; the claims on originals-only slots
 4. the sorted process values at W8 / W16 (spread shape)
 5. cross-block: this block's tip (cgu16) cells against C4-AB's C4-JD#tip cells (run 191354, same exe, same args)
 6. all-W pooled summary of the cgu1/cgu16 ratio"""
import math
import os
import statistics
from collections import defaultdict

import cgu_lib as C

L = C.L
o = C.Out()
recs = C.all_records()
procs, used, dropped = C.select(C.BLOCK, recs, C.RUN_CGU, row=C.ROW)
P, B = C.PRIMARY, C.BESIDE


def binom_two_sided(k, n):
    if n == 0:
        return 1.0
    k = min(k, n - k)
    return min(1.0, 2 * sum(math.comb(n, i) for i in range(k + 1)) / 2 ** n)


# 1. paired
o('# C4-CGU post hoc (labelled POST HOC throughout; decides nothing)')
o('\n## 1. Paired within-round ratios cgu1/cgu16 (same pass, round, W)')
pairs = defaultdict(dict)
for p in used:
    pairs[(p['pass'], p['rec']['round'], p['W'])][p['binary']] = p
for win in (P, B):
    o(f'window {win}:')
    allr = []
    for W in C.WS:
        rs, ra = [], []
        for (ps, rd, w), d in sorted(pairs.items()):
            if w != W or len(d) != 2:
                continue
            r = d[C.B_KEY]['v'][win] / d[C.A_KEY]['v'][win]
            rs.append(r)
            if all(x['attempt'] == 'original' for x in d.values()):
                ra.append(r)
        allr += rs
        nf = sum(1 for r in rs if r < 1)
        nfa = sum(1 for r in ra if r < 1)
        o(f'  W{W}: all pairs n={len(rs)} median {statistics.median(rs):.4f} ({100*(statistics.median(rs)-1):+.2f} %), '
          f'cgu1 faster in {nf}/{len(rs)} (sign test p={binom_two_sided(nf, len(rs)):.2f}); adjacent originals n={len(ra)} '
          f'median {statistics.median(ra):.4f}, cgu1 faster {nfa}/{len(ra)}')
    nf = sum(1 for r in allr if r < 1)
    gm = math.exp(sum(math.log(r) for r in allr) / len(allr))
    o(f'  all W: n={len(allr)} median {statistics.median(allr):.4f} geomean {gm:.4f} ({100*(gm-1):+.2f} %); cgu1 faster {nf}/{len(allr)} '
      f'(sign test p={binom_two_sided(nf, len(allr)):.2f})')

# 2. order position
o('\n## 2. Order position (adjacent original pairs only): median cgu1/cgu16 by which exe ran first')
for win in (P, B):
    first_b = [d[C.B_KEY]['v'][win] / d[C.A_KEY]['v'][win] for (ps, rd, w), d in pairs.items()
               if len(d) == 2 and all(x['attempt'] == 'original' for x in d.values()) and ps in (0, 2)]
    first_a = [d[C.B_KEY]['v'][win] / d[C.A_KEY]['v'][win] for (ps, rd, w), d in pairs.items()
               if len(d) == 2 and all(x['attempt'] == 'original' for x in d.values()) and ps == 1]
    o(f'  {win}: cgu1 first (p0,p2) n={len(first_b)} median {statistics.median(first_b):.4f}; cgu16 first (p1) n={len(first_a)} '
      f'median {statistics.median(first_a):.4f}; second-minus-first position effect estimate '
      f'{100*(statistics.median(first_b)/statistics.median(first_a)-1)/-2:+.2f} % per position (sign: + = the second runner reads slower)')

# 3. re-run bias
o('\n## 3. Re-run bias: each used re-run against the median of the ORIGINAL clean used processes of its cell')
for win in (P, B):
    rb = []
    for p in used:
        if p['attempt'] != 'rerun':
            continue
        og = [q['v'][win] for q in used if q['attempt'] == 'original' and q['binary'] == p['binary'] and q['W'] == p['W']]
        rb.append((p['binary'], p['W'], p['v'][win] / statistics.median(og)))
    o(f'  {win}: n={len(rb)} median {statistics.median(r for _, _, r in rb):.4f}; above 1: {sum(1 for *_, r in rb if r > 1)}/{len(rb)}; '
      f'by key: cgu16 {statistics.median(r for k, _, r in rb if k == C.A_KEY):.4f} (n={sum(1 for k, *_ in rb if k == C.A_KEY)}), '
      f'cgu1 {statistics.median(r for k, _, r in rb if k == C.B_KEY):.4f} (n={sum(1 for k, *_ in rb if k == C.B_KEY)})')
o('  claims on clean ORIGINAL slots only (re-run slots dropped; a pass-cell with K < 3 sets no flag = LETTER):')
orig_only = [p for p in used if p['attempt'] == 'original']
for win in (P, B):
    for W in C.WS:
        j = L.judge(C.vals(orig_only, C.A_KEY, W, win), C.vals(orig_only, C.B_KEY, W, win), 'LETTER')
        jg = L.judge(C.vals(orig_only, C.A_KEY, W, win), C.vals(orig_only, C.B_KEY, W, win), 'GATING-ONLY')
        o(f'    {win} W{W}: nA {j["a"]["K"]} nB {j["b"]["K"]} B/A {j["pooled"]["ratio"]:.4f} pooled {L.yn(j["pooled"])} '
          f'LETTER {L.label(j)}; GATING-ONLY gates {jg["gates"]} {L.label(jg)} [{jg["dir"]}]')

# 4. spread shape
o('\n## 4. Sorted used process values, PRIMARY window (ms), pass in brackets')
for W in (8, 16):
    for k in (C.A_KEY, C.B_KEY):
        xs = sorted((p['v'][P], p['pass'], p['attempt'][0]) for p in used if p['binary'] == k and p['W'] == W)
        o(f'  W{W} {k:8s}: ' + ' '.join(f'{v:.4f}(p{ps}{a})' for v, ps, a in xs))

# 5. cross-block
o('\n## 5. Cross-block (POST HOC): C4-CGU tip (cgu16) vs C4-AB C4-JD#tip, same exe 8d6e7d41, same args; ~7.5 h apart')
ab_procs, ab_used, ab_drop = C.select('C4-AB', recs, C.RUN_AB, row='C4-JD', binary='tip')
o(f'C4-AB C4-JD#tip: processes {len(ab_procs)}, used {len(ab_used)}, dropped {len(ab_drop)}')
for win in (P, B):
    for W in C.WS:
        va = C.vals(ab_used, 'tip', W, win, row='C4-JD')
        vb = C.vals(used, C.A_KEY, W, win)
        jl = L.judge(va, vb, 'LETTER')
        jg = L.judge(va, vb, 'GATING-ONLY')
        ns = '/'.join(str(len(va.get(k, []))) for k in L.PASSES)
        o(f'  {win} W{W}: AB {jl["a"]["median"]:.4f} (n {jl["a"]["K"]}: {ns}) -> CGU {jl["b"]["median"]:.4f} (n {jl["b"]["K"]}) '
          f'CGU/AB {jl["pooled"]["ratio"]:.4f} ({100*(jl["pooled"]["ratio"]-1):+.2f} %) pooled {L.yn(jl["pooled"])}'
          f'{" sep" if jl["pooled"]["minmax_sep"] else ""}; LETTER {L.label(jl)}; GATING-ONLY {jg["gates"]} {L.label(jg)} [{jg["dir"]}]')
o('  and C4-CGU cgu1 against C4-AB C4-JD#tip (both readings):')
for win in (P,):
    for W in C.WS:
        va = C.vals(ab_used, 'tip', W, win, row='C4-JD')
        vb = C.vals(used, C.B_KEY, W, win)
        jg = L.judge(va, vb, 'GATING-ONLY')
        o(f'  {win} W{W}: CGU-cgu1/AB-tip {jg["pooled"]["ratio"]:.4f} ({100*(jg["pooled"]["ratio"]-1):+.2f} %) pooled {L.yn(jg["pooled"])}; GATING-ONLY {L.label(jg)} [{jg["dir"]}]')

# 6. exe sizes (informational)
o('\n## 6. Exe sizes (informational)')
for k in (C.A_KEY, C.B_KEY):
    o(f'  {k}: {os.path.getsize(os.path.join(C.W9A, L.BINS[k]["exe"])):,} bytes')
o.save('c2_posthoc.txt')
